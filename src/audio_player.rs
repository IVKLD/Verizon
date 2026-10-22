use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use crate::timeline::TimelineState;

/// Один звуковой сегмент на таймлайне (клип A-дорожки) в координатах interleaved-сэмплов (L,R,L,R...)
pub struct MixSegment {
    pub samples: Arc<Vec<f32>>,
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
            for (k, d) in dst.iter_mut().enumerate() {
                let src = src_base + k;
                // Чётность относительно начала таймлайна: чётный индекс = L, нечётный = R
                let gain = if (lo + k) & 1 == 0 { seg.gain_l } else { seg.gain_r };
                if let Some(s) = pcm.get(src) {
                    *d += *s * gain;
                }
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
    // Кэш декодированных аудиопотоков: (файл, индекс_аудиопотока) -> PCM F32 Stereo 48kHz
    pub audio_cache: Arc<Mutex<HashMap<(PathBuf, usize), Arc<Vec<f32>>>>>,
    pub pending_decodes: Arc<Mutex<std::collections::HashSet<(PathBuf, usize)>>>,
    pub last_cache_len: usize,
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
            pending_decodes: Arc::new(Mutex::new(std::collections::HashSet::new())),
            last_cache_len: 0,
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
    #[allow(dead_code)]
    pub fn decode_audio_worker(path: &Path) -> Result<(Vec<f32>, u32), String> {
        Self::decode_audio_stream_worker(path, 0)
    }

    /// Прецизионный декодер конкретной аудиодорожки (потока) из медиафайла
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

    /// Проверить, завершилась ли асинхронная расшифровка новых аудиодорожек
    pub fn has_new_decoded_audio(&mut self) -> bool {
        if let Ok(cache) = self.audio_cache.lock() {
            let current_len = cache.len();
            if current_len != self.last_cache_len {
                self.last_cache_len = current_len;
                return true;
            }
        }
        false
    }

    /// Синхронное гарантированное получение аудиопотока (для тестов)
    #[allow(dead_code)]
    pub fn ensure_audio_stream_cached_sync(&self, path: &Path, audio_track_index: usize) -> Option<Arc<Vec<f32>>> {
        let key = (path.to_path_buf(), audio_track_index);
        if let Ok(cache) = self.audio_cache.lock() {
            if let Some(samples) = cache.get(&key) {
                return Some(samples.clone());
            }
        }
        if let Ok((samples, _)) = Self::decode_audio_stream_worker(path, audio_track_index) {
            let arc_samples = Arc::new(samples);
            if let Ok(mut cache) = self.audio_cache.lock() {
                cache.insert(key, arc_samples.clone());
            }
            Some(arc_samples)
        } else {
            None
        }
    }

    /// Неблокирующее получение аудиопотока из кэша.
    /// Если аудио ещё не расшифровано, запускает фоновый воркер без блокировки интерфейса (0 ms UI lag)!
    pub fn get_or_decode_audio_stream(&self, path: &Path, audio_track_index: usize) -> Option<Arc<Vec<f32>>> {
        let key = (path.to_path_buf(), audio_track_index);
        if let Ok(cache) = self.audio_cache.lock() {
            if let Some(samples) = cache.get(&key) {
                return Some(samples.clone());
            }
        }

        // Запуск неблокирующего фонового декодирования (гарантирует 0 ms зависания GUI при импорте сложных видео)
        if let Ok(mut pending) = self.pending_decodes.lock() {
            if !pending.contains(&key) {
                pending.insert(key.clone());
                let cache_clone = self.audio_cache.clone();
                let pending_clone = self.pending_decodes.clone();
                let p = path.to_path_buf();
                std::thread::spawn(move || {
                    if let Ok((samples, _)) = Self::decode_audio_stream_worker(&p, audio_track_index) {
                        let arc_samples = Arc::new(samples);
                        if let Ok(mut c) = cache_clone.lock() {
                            c.insert((p.clone(), audio_track_index), arc_samples);
                        }
                    }
                    if let Ok(mut pend) = pending_clone.lock() {
                        pend.remove(&(p, audio_track_index));
                    }
                });
            }
        }

        None
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
    #[allow(dead_code)]
    pub fn render_mix_window(&self, start_sample: usize, len: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; len];
        self.engine.snapshot.load().mix_block(start_sample & !1, &mut out);
        out
    }

    /// Загрузить аудио из медиафайла асинхронно через фоновый поток (прогрев кэша PCM).
    /// Сведение выполняется снимком таймлайна, как только поток расшифрован (см. has_new_decoded_audio).
    pub fn load_audio_from_file(&mut self, path: &Path) -> Result<(), String> {
        if self.current_path.as_deref() == Some(path) {
            return Ok(());
        }

        self.current_path = Some(path.to_path_buf());
        self.is_loading.store(true, Ordering::SeqCst);

        let path_buf = path.to_path_buf();
        let is_loading = self.is_loading.clone();
        let load_id = self.load_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let load_counter = self.load_counter.clone();
        let cache = self.audio_cache.clone();

        std::thread::spawn(move || {
            match Self::decode_audio_stream_worker(&path_buf, 0) {
                Ok((samples, _out_rate)) => {
                    if let Ok(mut c) = cache.lock() {
                        c.insert((path_buf.clone(), 0), Arc::new(samples));
                    }
                }
                Err(e) => {
                    eprintln!("Background audio decode error: {}", e);
                }
            }
            if load_counter.load(Ordering::SeqCst) == load_id {
                is_loading.store(false, Ordering::SeqCst);
            }
        });

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
