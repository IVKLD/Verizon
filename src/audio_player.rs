use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use crate::timeline::TimelineState;

/// Прогрессивный потоковый буфер аудиопотока (48 кГц Stereo Interleaved F32).
/// Создается мгновенно (0 мс) при добавлении клипа на таймлайн и заполняется в фоне с нулевой секунды:
/// real-time коллбэк CPAL читает уже декодированную часть lock-free через `decoded_len` (Acquire/Release).
pub struct AudioStreamBuffer {
    pub samples: Box<[AtomicU32]>,
    pub decoded_len: AtomicUsize,
    pub finished: AtomicBool,
    pub peaks: Mutex<Option<Arc<Vec<f32>>>>,
}

impl AudioStreamBuffer {
    pub fn with_capacity(cap_samples: usize) -> Arc<Self> {
        let cap = cap_samples.max(2) & !1;
        let mut vec = Vec::with_capacity(cap);
        vec.resize_with(cap, || AtomicU32::new(0));
        Arc::new(Self {
            samples: vec.into_boxed_slice(),
            decoded_len: AtomicUsize::new(0),
            finished: AtomicBool::new(false),
            peaks: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub fn from_vec(v: Vec<f32>) -> Arc<Self> {
        let len = v.len() & !1;
        let boxed: Box<[AtomicU32]> = v
            .into_iter()
            .take(len)
            .map(|f| AtomicU32::new(f.to_bits()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Arc::new(Self {
            samples: boxed,
            decoded_len: AtomicUsize::new(len),
            finished: AtomicBool::new(true),
            peaks: Mutex::new(None),
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.decoded_len.load(Ordering::Acquire)
    }

    /// Дописать порцию декодированных сэмплов и опубликовать новую длину (Release)
    pub fn append_slice(&self, cursor: &mut usize, data: &[f32]) {
        let cap = self.samples.len();
        if *cursor >= cap {
            return;
        }
        let avail = (cap - *cursor).min(data.len());
        for (i, &s) in data[..avail].iter().enumerate() {
            self.samples[*cursor + i].store(s.to_bits(), Ordering::Relaxed);
        }
        *cursor += avail;
        self.decoded_len.store(*cursor & !1, Ordering::Release);
    }
}

/// Один звуковой сегмент на таймлайне (клип A-дорожки) в координатах interleaved-сэмплов (L,R,L,R...)
pub struct MixSegment {
    pub samples: Arc<AudioStreamBuffer>,
    /// Позиция начала сегмента на таймлайне (в interleaved-сэмплах, чётная)
    pub start: usize,
    /// Смещение внутри исходного PCM-потока (в interleaved-сэмплах, чётное)
    pub in_off: usize,
    /// Длина сегмента (в interleaved-сэмплах)
    pub len: usize,
    pub gain_l: f32,
    pub gain_r: f32,
}

/// Иммутабельный снимок аудио-таймлайна. Публикуется через ArcSwap (lock-free, wait-free чтение).
/// Пересборка стоит O(число клипов) — микросекунды, без копирования PCM.
#[derive(Default)]
pub struct MixSnapshot {
    pub segments: Vec<MixSegment>,
    pub total_len: usize,
}

impl MixSnapshot {
    /// Микширует окно [pos, pos + out.len()) прямо в `out` (real-time safe: без аллокаций и блокировок)
    pub fn mix_block(&self, pos: usize, out: &mut [f32]) {
        out.fill(0.0);
        let end = pos + out.len();
        for seg in &self.segments {
            let seg_end = seg.start + seg.len;
            let lo = pos.max(seg.start);
            let hi = end.min(seg_end);
            if lo >= hi {
                continue;
            }
            let src_base = seg.in_off + (lo - seg.start);
            let dst = &mut out[lo - pos..hi - pos];
            let pcm = &seg.samples;
            let avail_len = pcm.decoded_len.load(Ordering::Acquire).min(pcm.samples.len());
            for (k, d) in dst.iter_mut().enumerate() {
                let src = src_base + k;
                if src >= avail_len {
                    break;
                }
                // Чётность относительно начала таймлайна: чётный индекс = L, нечётный = R
                let gain = if (lo + k) & 1 == 0 { seg.gain_l } else { seg.gain_r };
                let s = f32::from_bits(pcm.samples[src].load(Ordering::Relaxed));
                *d += s * gain;
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Состояние аудио-движка, разделяемое между GUI-потоком и real-time коллбэком CPAL.
/// Всё lock-free: ArcSwap для снимка таймлайна и атомики для транспорта.
pub struct AudioEngineState {
    pub snapshot: ArcSwap<MixSnapshot>,
    pub pos: AtomicUsize,
    pub is_playing: AtomicBool,
    pub volume_bits: AtomicU32,
}

pub const AUDIO_RATE: u32 = 48000;

pub struct AudioPlayer {
    pub current_path: Option<PathBuf>,
    pub engine: Arc<AudioEngineState>,
    pub vu_l_bits: Arc<AtomicU32>,
    pub vu_r_bits: Arc<AtomicU32>,
    pub is_loading: Arc<AtomicBool>,
    load_counter: Arc<AtomicU64>,
    _stream: Option<cpal::Stream>,
    // Кэш прогрессивных аудиопотоков: (файл, индекс_аудиопотока) -> AudioStreamBuffer (PCM F32 Stereo 48kHz)
    pub audio_cache: Arc<Mutex<HashMap<(PathBuf, usize), Arc<AudioStreamBuffer>>>>,
    pub pending_decodes: Arc<Mutex<HashSet<(PathBuf, usize)>>>,
    pub cache_generation: Arc<AtomicU64>,
    pub last_seen_generation: u64,
}

impl AudioPlayer {
    pub fn new() -> Self {
        let engine = Arc::new(AudioEngineState {
            snapshot: ArcSwap::from_pointee(MixSnapshot::default()),
            pos: AtomicUsize::new(0),
            is_playing: AtomicBool::new(false),
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
        });

        let vu_l_bits = Arc::new(AtomicU32::new(0));
        let vu_r_bits = Arc::new(AtomicU32::new(0));

        let stream = Self::init_cpal_stream(engine.clone(), vu_l_bits.clone(), vu_r_bits.clone());

        Self {
            current_path: None,
            engine,
            vu_l_bits,
            vu_r_bits,
            is_loading: Arc::new(AtomicBool::new(false)),
            load_counter: Arc::new(AtomicU64::new(0)),
            _stream: stream,
            audio_cache: Arc::new(Mutex::new(HashMap::new())),
            pending_decodes: Arc::new(Mutex::new(HashSet::new())),
            cache_generation: Arc::new(AtomicU64::new(0)),
            last_seen_generation: 0,
        }
    }

    fn init_cpal_stream(
        engine: Arc<AudioEngineState>,
        vu_l: Arc<AtomicU32>,
        vu_r: Arc<AtomicU32>,
    ) -> Option<cpal::Stream> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;

        let sample_format = config.sample_format();
        let err_fn = |err: cpal::Error| {
            let msg = err.to_string();
            // Underrun/overrun в ALSA/PipeWire являются нефатальными предупреждениями тайминга буфера
            if !msg.to_lowercase().contains("underrun") && !msg.to_lowercase().contains("overrun") {
                eprintln!("Audio stream error: {}", msg);
            }
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let engine_clone = engine.clone();
                let vu_l_clone = vu_l.clone();
                let vu_r_clone = vu_r.clone();
                device.build_output_stream(
                    config.into(),
                    move |data: &mut [f32], _| {
                        Self::write_audio_data(data, &engine_clone, &vu_l_clone, &vu_r_clone);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let engine_clone = engine.clone();
                let vu_l_clone = vu_l.clone();
                let vu_r_clone = vu_r.clone();
                // Буфер преобразования переиспользуется между вызовами коллбэка (без аллокаций в real-time)
                let mut temp: Vec<f32> = Vec::new();
                device.build_output_stream(
                    config.into(),
                    move |data: &mut [i16], _| {
                        if temp.len() != data.len() {
                            temp.resize(data.len(), 0.0);
                        }
                        Self::write_audio_data(&mut temp, &engine_clone, &vu_l_clone, &vu_r_clone);
                        for (dst, src) in data.iter_mut().zip(temp.iter()) {
                            *dst = (src * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16;
                        }
                    },
                    err_fn,
                    None,
                )
            }
            _ => return None,
        }
        .ok()?;

        let _ = stream.play();
        Some(stream)
    }

    /// Real-time коллбэк: полностью lock-free (ArcSwap::load + атомики). Нет мьютексов, нет аллокаций.
    fn write_audio_data(
        output: &mut [f32],
        engine: &Arc<AudioEngineState>,
        vu_l: &Arc<AtomicU32>,
        vu_r: &Arc<AtomicU32>,
    ) {
        if !engine.is_playing.load(Ordering::Acquire) {
            output.fill(0.0);
            vu_l.store(0, Ordering::Relaxed);
            vu_r.store(0, Ordering::Relaxed);
            return;
        }

        let snapshot = engine.snapshot.load();
        let pos_raw = engine.pos.load(Ordering::Acquire);
        let pos = pos_raw & !1;
        snapshot.mix_block(pos, output);

        let volume = f32::from_bits(engine.volume_bits.load(Ordering::Relaxed));
        let mut peak_l: f32 = 0.0;
        let mut peak_r: f32 = 0.0;
        for (i, s) in output.iter_mut().enumerate() {
            *s *= volume;
            let a = s.abs();
            if i & 1 == 0 {
                if a > peak_l {
                    peak_l = a;
                }
            } else if a > peak_r {
                peak_r = a;
            }
        }

        // CAS: если GUI-поток перемотал плейхед во время микширования — не затираем его seek
        let _ = engine.pos.compare_exchange(
            pos_raw,
            pos + output.len(),
            Ordering::AcqRel,
            Ordering::Relaxed,
        );

        vu_l.store(peak_l.to_bits(), Ordering::Relaxed);
        vu_r.store(peak_r.to_bits(), Ordering::Relaxed);
    }

    /// Фоновый декодер первого аудиопотока через FFmpeg (обратная совместимость)
    #[cfg(test)]
    pub fn decode_audio_worker(path: &Path) -> Result<(Vec<f32>, u32), String> {
        Self::decode_audio_stream_worker(path, 0)
    }

    /// Прецизионный декодер конкретной аудиодорожки (потока) из медиафайла
    #[cfg(test)]
    pub fn decode_audio_stream_worker(path: &Path, audio_track_index: usize) -> Result<(Vec<f32>, u32), String> {
        let mut ictx = ffmpeg_next::format::input(path).map_err(|e| e.to_string())?;

        let audio_streams: Vec<_> = ictx
            .streams()
            .filter(|s| s.parameters().medium() == ffmpeg_next::media::Type::Audio)
            .collect();

        if audio_streams.is_empty() {
            return Err("Аудиопотоки не найдены в медиафайле".to_string());
        }

        let audio_stream = audio_streams
            .get(audio_track_index)
            .or_else(|| audio_streams.first())
            .unwrap();

        let stream_idx = audio_stream.index();
        let context_decoder = ffmpeg_next::codec::context::Context::from_parameters(audio_stream.parameters())
            .map_err(|e| e.to_string())?;
        let mut audio_decoder = context_decoder.decoder().audio().map_err(|e| e.to_string())?;

        // Отключаем чтение видеопотоков (AVDISCARD_ALL), чтобы не аллоцировать гигабайты видеопакетов при декодировании звука
        for mut s in ictx.streams_mut() {
            if s.index() != stream_idx {
                unsafe {
                    (*s.as_mut_ptr()).discard = ffmpeg_next::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }

        let in_rate = audio_decoder.rate();
        let in_format = audio_decoder.format();
        let in_channel_layout = if audio_decoder.channel_layout().is_empty() {
            if audio_decoder.channels() == 1 {
                ffmpeg_next::channel_layout::ChannelLayout::MONO
            } else {
                ffmpeg_next::channel_layout::ChannelLayout::STEREO
            }
        } else {
            audio_decoder.channel_layout()
        };

        let out_rate = 48000;
        let out_channel_layout = ffmpeg_next::channel_layout::ChannelLayout::STEREO;
        let out_format = ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed);

        let mut resampler = ffmpeg_next::software::resampling::Context::get(
            in_format,
            in_channel_layout,
            in_rate,
            out_format,
            out_channel_layout,
            out_rate,
        )
        .map_err(|e| e.to_string())?;

        let mut all_samples = Vec::new();
        let mut decoded_frame = ffmpeg_next::util::frame::Audio::empty();
        let mut resampled_frame = ffmpeg_next::util::frame::Audio::empty();

        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_idx {
                if audio_decoder.send_packet(&packet).is_ok() {
                    while audio_decoder.receive_frame(&mut decoded_frame).is_ok() {
                        if decoded_frame.channel_layout().is_empty() {
                            decoded_frame.set_channel_layout(in_channel_layout);
                        }
                        if resampler.run(&decoded_frame, &mut resampled_frame).is_ok() {
                            let samples_count = resampled_frame.samples() * 2; // Stereo
                            let data: &[f32] = unsafe {
                                std::slice::from_raw_parts(
                                    resampled_frame.data(0).as_ptr() as *const f32,
                                    samples_count,
                                )
                            };
                            all_samples.extend_from_slice(data);
                        }
                    }
                }
            }
        }

        if audio_decoder.send_eof().is_ok() {
            while audio_decoder.receive_frame(&mut decoded_frame).is_ok() {
                if decoded_frame.channel_layout().is_empty() {
                    decoded_frame.set_channel_layout(in_channel_layout);
                }
                if resampler.run(&decoded_frame, &mut resampled_frame).is_ok() {
                    let samples_count = resampled_frame.samples() * 2;
                    let data: &[f32] = unsafe {
                        std::slice::from_raw_parts(
                            resampled_frame.data(0).as_ptr() as *const f32,
                            samples_count,
                        )
                    };
                    all_samples.extend_from_slice(data);
                }
            }
        }

        Ok((all_samples, out_rate))
    }

    /// Проверить, появились ли новые расшифрованные аудиопотоки
    pub fn has_new_decoded_audio(&mut self) -> bool {
        let gen = self.cache_generation.load(Ordering::Acquire);
        if gen != self.last_seen_generation {
            self.last_seen_generation = gen;
            return true;
        }
        false
    }

    /// Передать вычисленные во время декодирования волновые пики в WaveformManager (без повторного чтения файла с диска!)
    pub fn export_waveforms_to(&self, waveform_mgr: &crate::waveform::WaveformManager) {
        if let Ok(cache) = self.audio_cache.lock() {
            for ((path, idx), buf) in cache.iter() {
                if waveform_mgr.get_waveform_stream(path, *idx).is_none() {
                    if let Ok(pk_guard) = buf.peaks.lock() {
                        if let Some(ref arc_peaks) = *pk_guard {
                            waveform_mgr.insert_waveform_arc(path, *idx, arc_peaks.clone());
                        }
                    }
                }
            }
        }
    }

    /// Синхронное гарантированное получение полностью декодированного аудиопотока (для тестов)
    #[cfg(test)]
    pub fn ensure_audio_stream_cached_sync(&self, path: &Path, audio_track_index: usize) -> Option<Arc<AudioStreamBuffer>> {
        let key = (path.to_path_buf(), audio_track_index);
        if let Ok(cache) = self.audio_cache.lock() {
            if let Some(buf) = cache.get(&key) {
                if buf.finished.load(Ordering::Acquire) {
                    return Some(buf.clone());
                }
            }
        }
        if let Ok((samples, _)) = Self::decode_audio_stream_worker(path, audio_track_index) {
            let arc_buf = AudioStreamBuffer::from_vec(samples);
            if let Ok(mut cache) = self.audio_cache.lock() {
                cache.insert(key, arc_buf.clone());
            }
            self.cache_generation.fetch_add(1, Ordering::Release);
            Some(arc_buf)
        } else {
            None
        }
    }

    /// Однопроходный прогрессивный декодер ВСЕХ аудиопотоков файла одновременно.
    /// Заполняет все буферы потоков с 0-й секунды параллельно чтению пакетов (звук играет мгновенно!).
    fn decode_all_streams_progressive(
        path: &Path,
        buffers: Vec<(usize, Arc<AudioStreamBuffer>)>,
    ) {
        let Ok(mut ictx) = ffmpeg_next::format::input(path) else {
            for (_, b) in &buffers {
                b.finished.store(true, Ordering::Release);
            }
            return;
        };

        let audio_stream_indices: Vec<(usize, usize, ffmpeg_next::codec::Parameters)> = ictx
            .streams()
            .filter(|s| s.parameters().medium() == ffmpeg_next::media::Type::Audio)
            .enumerate()
            .map(|(a_idx, s)| (a_idx, s.index(), s.parameters()))
            .collect();

        // Отключаем чтение видеопотоков (AVDISCARD_ALL), чтобы не аллоцировать 10+ ГБ 4K60 видеопакетов при прогрессивном декодировании звука
        for mut s in ictx.streams_mut() {
            if s.parameters().medium() != ffmpeg_next::media::Type::Audio {
                unsafe {
                    (*s.as_mut_ptr()).discard = ffmpeg_next::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }

        struct StreamDecoder {
            stream_idx: usize,
            decoder: ffmpeg_next::decoder::Audio,
            resampler: ffmpeg_next::software::resampling::Context,
            in_channel_layout: ffmpeg_next::channel_layout::ChannelLayout,
            decoded_frame: ffmpeg_next::util::frame::Audio,
            resampled_frame: ffmpeg_next::util::frame::Audio,
            buf: Arc<AudioStreamBuffer>,
            cursor: usize,
            peaks: Vec<f32>,
            current_max: f32,
            count_in_bucket: usize,
        }

        let samples_per_bucket = (AUDIO_RATE as usize * 2) / 100; // 100 пиков в секунду (каждые 10 мс = 960 сэмплов)
        let mut decoders: Vec<StreamDecoder> = Vec::new();
        for (a_idx, buf) in &buffers {
            let Some((_, stream_idx, ref params)) = audio_stream_indices
                .iter()
                .find(|(idx, _, _)| idx == a_idx)
                .or_else(|| audio_stream_indices.first())
            else {
                buf.finished.store(true, Ordering::Release);
                continue;
            };

            let Ok(ctx_dec) = ffmpeg_next::codec::context::Context::from_parameters(params.clone()) else {
                buf.finished.store(true, Ordering::Release);
                continue;
            };
            let Ok(audio_decoder) = ctx_dec.decoder().audio() else {
                buf.finished.store(true, Ordering::Release);
                continue;
            };

            let in_rate = audio_decoder.rate();
            let in_format = audio_decoder.format();
            let in_channel_layout = if audio_decoder.channel_layout().is_empty() {
                if audio_decoder.channels() == 1 {
                    ffmpeg_next::channel_layout::ChannelLayout::MONO
                } else {
                    ffmpeg_next::channel_layout::ChannelLayout::STEREO
                }
            } else {
                audio_decoder.channel_layout()
            };

            let out_rate = AUDIO_RATE;
            let out_channel_layout = ffmpeg_next::channel_layout::ChannelLayout::STEREO;
            let out_format = ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed);

            let Ok(resampler) = ffmpeg_next::software::resampling::Context::get(
                in_format,
                in_channel_layout,
                in_rate,
                out_format,
                out_channel_layout,
                out_rate,
            ) else {
                buf.finished.store(true, Ordering::Release);
                continue;
            };

            let est_buckets = (buf.samples.len() / samples_per_bucket).max(256);
            decoders.push(StreamDecoder {
                stream_idx: *stream_idx,
                decoder: audio_decoder,
                resampler,
                in_channel_layout,
                decoded_frame: ffmpeg_next::util::frame::Audio::empty(),
                resampled_frame: ffmpeg_next::util::frame::Audio::empty(),
                buf: buf.clone(),
                cursor: 0,
                peaks: Vec::with_capacity(est_buckets),
                current_max: 0.0,
                count_in_bucket: 0,
            });
        }

        if decoders.is_empty() {
            return;
        }

        for (stream, packet) in ictx.packets() {
            let s_idx = stream.index();
            for dec in &mut decoders {
                if dec.stream_idx == s_idx && dec.decoder.send_packet(&packet).is_ok() {
                    while dec.decoder.receive_frame(&mut dec.decoded_frame).is_ok() {
                        if dec.decoded_frame.channel_layout().is_empty() {
                            dec.decoded_frame.set_channel_layout(dec.in_channel_layout);
                        }
                        if dec.resampler.run(&dec.decoded_frame, &mut dec.resampled_frame).is_ok() {
                            let samples_count = dec.resampled_frame.samples() * 2;
                            let data: &[f32] = unsafe {
                                std::slice::from_raw_parts(
                                    dec.resampled_frame.data(0).as_ptr() as *const f32,
                                    samples_count,
                                )
                            };
                            dec.buf.append_slice(&mut dec.cursor, data);
                            for &sample in data {
                                let abs = sample.abs();
                                if abs > dec.current_max {
                                    dec.current_max = abs;
                                }
                                dec.count_in_bucket += 1;
                                if dec.count_in_bucket >= samples_per_bucket {
                                    dec.peaks.push(dec.current_max.clamp(0.0, 1.0));
                                    dec.current_max = 0.0;
                                    dec.count_in_bucket = 0;
                                }
                            }
                        }
                    }
                }
            }
        }

        for dec in &mut decoders {
            if dec.decoder.send_eof().is_ok() {
                while dec.decoder.receive_frame(&mut dec.decoded_frame).is_ok() {
                    if dec.decoded_frame.channel_layout().is_empty() {
                        dec.decoded_frame.set_channel_layout(dec.in_channel_layout);
                    }
                    if dec.resampler.run(&dec.decoded_frame, &mut dec.resampled_frame).is_ok() {
                        let samples_count = dec.resampled_frame.samples() * 2;
                        let data: &[f32] = unsafe {
                            std::slice::from_raw_parts(
                                dec.resampled_frame.data(0).as_ptr() as *const f32,
                                samples_count,
                            )
                        };
                        dec.buf.append_slice(&mut dec.cursor, data);
                        for &sample in data {
                            let abs = sample.abs();
                            if abs > dec.current_max {
                                dec.current_max = abs;
                            }
                            dec.count_in_bucket += 1;
                            if dec.count_in_bucket >= samples_per_bucket {
                                dec.peaks.push(dec.current_max.clamp(0.0, 1.0));
                                dec.current_max = 0.0;
                                dec.count_in_bucket = 0;
                            }
                        }
                    }
                }
            }
            if dec.count_in_bucket > 0 {
                dec.peaks.push(dec.current_max.clamp(0.0, 1.0));
            }
            let max_peak = dec.peaks.iter().copied().fold(0.0_f32, f32::max);
            if max_peak > 0.001 && max_peak < 0.8 {
                let gain = (0.85 / max_peak).min(6.0);
                for p in &mut dec.peaks {
                    *p = (*p * gain).clamp(0.0, 1.0);
                }
            }
            if let Ok(mut pk_guard) = dec.buf.peaks.lock() {
                *pk_guard = Some(Arc::new(std::mem::take(&mut dec.peaks)));
            }
            dec.buf.finished.store(true, Ordering::Release);
        }
    }

    /// Неблокирующее получение прогрессивного аудиопотока из кэша.
    /// Возвращает `Some(Arc<AudioStreamBuffer>)` МГНОВЕННО (0 ms) и запускает однопроходное фоновое
    /// декодирование всех аудиодорожек файла, чтобы звук играл с первой же миллисекунды!
    pub fn get_or_decode_audio_stream(&self, path: &Path, audio_track_index: usize) -> Option<Arc<AudioStreamBuffer>> {
        let key = (path.to_path_buf(), audio_track_index);
        if let Ok(cache) = self.audio_cache.lock() {
            if let Some(buf) = cache.get(&key) {
                return Some(buf.clone());
            }
        }

        // Определяем число аудиопотоков и длительность файла за ~0.2 мс (чтение заголовка контейнера)
        let (num_streams, duration_secs) = if let Ok(ictx) = ffmpeg_next::format::input(path) {
            let n = ictx
                .streams()
                .filter(|s| s.parameters().medium() == ffmpeg_next::media::Type::Audio)
                .count();
            let d = (ictx.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE)).max(1.0);
            (n, d)
        } else {
            return None;
        };

        if num_streams == 0 {
            return None;
        }

        // Выделяем прогрессивные буферы сразу для всех аудиодорожек файла (с запасом +2 сек)
        let cap_samples = ((duration_secs + 2.0) * AUDIO_RATE as f64).ceil() as usize * 2;
        let mut to_decode: Vec<(usize, Arc<AudioStreamBuffer>)> = Vec::new();
        let mut result_buf: Option<Arc<AudioStreamBuffer>> = None;

        if let (Ok(mut cache), Ok(mut pending)) = (self.audio_cache.lock(), self.pending_decodes.lock()) {
            let max_idx = num_streams.max(audio_track_index + 1);
            for idx in 0..max_idx {
                let k = (path.to_path_buf(), idx);
                let buf = cache
                    .entry(k.clone())
                    .or_insert_with(|| AudioStreamBuffer::with_capacity(cap_samples))
                    .clone();
                if idx == audio_track_index {
                    result_buf = Some(buf.clone());
                }
                if !pending.contains(&k) && !buf.finished.load(Ordering::Acquire) {
                    pending.insert(k);
                    to_decode.push((idx, buf));
                }
            }
        }

        if !to_decode.is_empty() {
            let pending_clone = self.pending_decodes.clone();
            let gen_clone = self.cache_generation.clone();
            let is_loading = self.is_loading.clone();
            is_loading.store(true, Ordering::Release);
            let p = path.to_path_buf();
            std::thread::spawn(move || {
                let indices: Vec<usize> = to_decode.iter().map(|(i, _)| *i).collect();
                Self::decode_all_streams_progressive(&p, to_decode);
                if let Ok(mut pend) = pending_clone.lock() {
                    for idx in indices {
                        pend.remove(&(p.clone(), idx));
                    }
                }
                gen_clone.fetch_add(1, Ordering::Release);
                is_loading.store(false, Ordering::Release);
            });
        }

        result_buf
    }

    /// Мгновенная сборка снимка аудио-таймлайна (Multi-Track Mixer): O(число клипов), без копирования PCM.
    /// Само микширование выполняется в real-time коллбэке только для текущего окна ~10 мс.
    /// Поддерживает неограниченное число аудиодорожек (A1, A2, A3...), Mute/Solo, громкость (volume_db), панораму (pan)
    pub fn rebuild_timeline_audio(&mut self, timeline: &TimelineState) {
        let fps = timeline.fps.max(1) as f64;
        let has_solo = timeline.tracks.iter().any(|t| !t.is_video && t.is_solo);
        let mut segments: Vec<MixSegment> = Vec::with_capacity(timeline.clips.len());
        let mut total_len = 0usize;

        for c in &timeline.clips {
            if !c.is_audio {
                continue;
            }

            // Проверка Mute / Solo для дорожки клипа
            if let Some(t) = timeline.tracks.get(c.track_index) {
                if t.is_muted {
                    continue;
                }
                if has_solo && !t.is_solo {
                    continue;
                }
            }

            let path = match &c.source_path {
                Some(p) => p,
                None => continue,
            };

            let clip_samples = match self.get_or_decode_audio_stream(path, c.source_audio_stream) {
                Some(s) => s,
                None => continue,
            };

            let vol_linear = 10.0f32.powf(c.volume_db / 20.0);
            let pan_l = ((1.0 - c.pan) / 2.0).clamp(0.0, 1.0);
            let pan_r = ((1.0 + c.pan) / 2.0).clamp(0.0, 1.0);

            let start = ((c.start_frame.max(0) as f64 / fps) * AUDIO_RATE as f64).round() as usize * 2;
            let in_off = (c.source_in_secs.max(0.0) * AUDIO_RATE as f64).round() as usize * 2;
            let len = ((c.duration_frames.max(0) as f64 / fps) * AUDIO_RATE as f64).round() as usize * 2;

            total_len = total_len.max(start + len);
            segments.push(MixSegment {
                samples: clip_samples,
                start,
                in_off,
                len,
                gain_l: vol_linear * pan_l,
                gain_r: vol_linear * pan_r,
            });
        }

        self.engine.snapshot.store(Arc::new(MixSnapshot { segments, total_len }));
    }

    /// Офлайн-рендер окна мастер-микса (для тестов и экспорта): [start_sample, start_sample + len) interleaved
    #[cfg(test)]
    pub fn render_mix_window(&self, start_sample: usize, len: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; len];
        self.engine.snapshot.load().mix_block(start_sample & !1, &mut out);
        out
    }

    /// Загрузить аудио из медиафайла асинхронно через единый прогрессивный декодер (без дублирования потоков).
    pub fn load_audio_from_file(&mut self, path: &Path) -> Result<(), String> {
        let _ = self.load_counter;
        self.current_path = Some(path.to_path_buf());
        let _ = self.get_or_decode_audio_stream(path, 0);
        Ok(())
    }

    /// Синхронизация текущей позиции звука с плейхедом таймлайна без щелчков и заиканий (lock-free)
    pub fn sync_playhead(&self, current_time_seconds: Option<f64>, is_playing: bool) {
        let engine = &self.engine;
        let was_playing = engine.is_playing.swap(is_playing, Ordering::AcqRel);

        match current_time_seconds {
            Some(time_sec) => {
                let rate = AUDIO_RATE as f64;
                let target_sample = ((time_sec.max(0.0) * rate).round() as usize) * 2;
                if !was_playing || !is_playing {
                    engine.pos.store(target_sample, Ordering::Release);
                } else {
                    let cur = engine.pos.load(Ordering::Acquire);
                    let diff = (cur as isize - target_sample as isize).abs();
                    let threshold = (rate * 0.10 * 2.0) as isize;
                    if diff > threshold {
                        engine.pos.store(target_sample, Ordering::Release);
                    }
                }
            }
            None => {
                let total = engine.snapshot.load().total_len;
                engine.pos.store(total, Ordering::Release);
            }
        }
    }

    /// Синхронизация таймлайна по номеру кадра (для прямого многодорожечного микса)
    pub fn sync_playhead_frame(&self, playhead_frame: i64, fps: u32, is_playing: bool) {
        let time_sec = playhead_frame as f64 / fps.max(1) as f64;
        self.sync_playhead(Some(time_sec), is_playing);
    }

    /// Получить текущие уровни VU-метра для анимации (lock-free)
    pub fn get_vu_levels(&self) -> (f32, f32) {
        let l = f32::from_bits(self.vu_l_bits.load(Ordering::Relaxed));
        let r = f32::from_bits(self.vu_r_bits.load(Ordering::Relaxed));
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_decode_worker_replay_mkv() {
        let path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !path.exists() {
            return;
        }
        let res = AudioPlayer::decode_audio_worker(path);
        assert!(res.is_ok());
        let (samples, rate) = res.unwrap();
        assert_eq!(rate, 48000);
        assert!(!samples.is_empty(), "Samples should not be empty for MKV with PCM audio");
    }

    #[test]
    fn test_multitrack_mkv_decodes_track1_and_track2() {
        let path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !path.exists() {
            return;
        }
        // Декодируем дорожку 1 (Stream 0:1)
        let res1 = AudioPlayer::decode_audio_stream_worker(path, 0);
        assert!(res1.is_ok(), "Не удалось декодировать дорожку 1");
        let (samples1, rate1) = res1.unwrap();
        assert_eq!(rate1, 48000);
        assert!(!samples1.is_empty());

        // Декодируем дорожку 2 (Stream 0:2)
        let res2 = AudioPlayer::decode_audio_stream_worker(path, 1);
        assert!(res2.is_ok(), "Не удалось декодировать дорожку 2");
        let (samples2, rate2) = res2.unwrap();
        assert_eq!(rate2, 48000);
        assert!(!samples2.is_empty());
    }
}
