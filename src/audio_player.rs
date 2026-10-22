use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub struct AudioSharedState {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub current_sample_pos: usize,
    pub is_playing: bool,
    pub volume: f32,
    pub current_vu_l: f32,
    pub current_vu_r: f32,
}

pub struct AudioPlayer {
    pub current_path: Option<PathBuf>,
    pub shared_state: Arc<Mutex<AudioSharedState>>,
    pub is_loading: Arc<AtomicBool>,
    load_counter: Arc<AtomicU64>,
    _stream: Option<cpal::Stream>,
}

impl AudioPlayer {
    pub fn new() -> Self {
        let shared_state = Arc::new(Mutex::new(AudioSharedState {
            samples: Vec::new(),
            sample_rate: 48000,
            channels: 2,
            current_sample_pos: 0,
            is_playing: false,
            volume: 1.0,
            current_vu_l: 0.0,
            current_vu_r: 0.0,
        }));

        let stream = Self::init_cpal_stream(shared_state.clone());

        Self {
            current_path: None,
            shared_state,
            is_loading: Arc::new(AtomicBool::new(false)),
            load_counter: Arc::new(AtomicU64::new(0)),
            _stream: stream,
        }
    }

    fn init_cpal_stream(shared: Arc<Mutex<AudioSharedState>>) -> Option<cpal::Stream> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;

        let sample_format = config.sample_format();
        let err_fn = |err| eprintln!("Audio stream error: {}", err);

        let stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let shared_clone = shared.clone();
                device.build_output_stream(
                    config.into(),
                    move |data: &mut [f32], _| {
                        Self::write_audio_data(data, &shared_clone);
                    },
                    err_fn,
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let shared_clone = shared.clone();
                device.build_output_stream(
                    config.into(),
                    move |data: &mut [i16], _| {
                        let mut temp = vec![0.0f32; data.len()];
                        Self::write_audio_data(&mut temp, &shared_clone);
                        for (dst, src) in data.iter_mut().zip(temp) {
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

    fn write_audio_data(output: &mut [f32], shared: &Arc<Mutex<AudioSharedState>>) {
        if let Ok(mut state) = shared.lock() {
            if !state.is_playing || state.samples.is_empty() {
                output.fill(0.0);
                state.current_vu_l = 0.0;
                state.current_vu_r = 0.0;
                return;
            }

            let volume = state.volume;
            let total = state.samples.len();
            let mut peak_l: f32 = 0.0;
            let mut peak_r: f32 = 0.0;

            for frame in output.chunks_mut(2) {
                if state.current_sample_pos < total {
                    let s_l = state.samples[state.current_sample_pos] * volume;
                    frame[0] = s_l;
                    if s_l.abs() > peak_l {
                        peak_l = s_l.abs();
                    }
                    state.current_sample_pos += 1;
                } else {
                    frame[0] = 0.0;
                }

                if frame.len() > 1 {
                    if state.current_sample_pos < total {
                        let s_r = state.samples[state.current_sample_pos] * volume;
                        frame[1] = s_r;
                        if s_r.abs() > peak_r {
                            peak_r = s_r.abs();
                        }
                        state.current_sample_pos += 1;
                    } else {
                        frame[1] = 0.0;
                    }
                }
            }

            state.current_vu_l = peak_l;
            state.current_vu_r = peak_r;
        } else {
            output.fill(0.0);
        }
    }

    /// Фоновый декодер аудиофайла через FFmpeg
    pub fn decode_audio_worker(path: &Path) -> Result<(Vec<f32>, u32), String> {
        let mut ictx = ffmpeg_next::format::input(path).map_err(|e| e.to_string())?;

        let audio_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Audio)
            .ok_or_else(|| "Аудиопоток не найден".to_string())?;

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

    /// Загрузить аудио из медиафайла асинхронно через фоновый поток без блокировки UI
    pub fn load_audio_from_file(&mut self, path: &Path) -> Result<(), String> {
        if self.current_path.as_deref() == Some(path) {
            return Ok(());
        }

        self.current_path = Some(path.to_path_buf());
        self.is_loading.store(true, Ordering::SeqCst);

        // Сбрасываем старые семплы перед асинхронной загрузкой нового файла
        if let Ok(mut state) = self.shared_state.lock() {
            state.samples.clear();
            state.current_sample_pos = 0;
        }

        let path_buf = path.to_path_buf();
        let shared = self.shared_state.clone();
        let is_loading = self.is_loading.clone();
        let load_id = self.load_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let load_counter = self.load_counter.clone();

        std::thread::spawn(move || {
            match Self::decode_audio_worker(&path_buf) {
                Ok((samples, out_rate)) => {
                    if load_counter.load(Ordering::SeqCst) == load_id {
                        if let Ok(mut state) = shared.lock() {
                            state.samples = samples;
                            state.sample_rate = out_rate;
                            state.channels = 2;
                            state.current_sample_pos = 0;
                        }
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

    /// Синхронизация текущей позиции звука с плейхедом таймлайна без щелчков и заиканий
    pub fn sync_playhead(&self, current_time_seconds: Option<f64>, is_playing: bool) {
        if self.is_loading.load(Ordering::SeqCst) {
            return;
        }

        if let Ok(mut state) = self.shared_state.lock() {
            let was_playing = state.is_playing;
            state.is_playing = is_playing;

            if let Some(time_sec) = current_time_seconds {
                let sample_rate = state.sample_rate as f64;
                let target_sample = (time_sec * sample_rate * state.channels as f64).max(0.0) as usize;

                if !was_playing && is_playing {
                    // Момент запуска воспроизведения: жестко встаем на позицию плейхеда
                    state.current_sample_pos = target_sample.min(state.samples.len());
                } else if !is_playing {
                    // Пауза / скраббинг / перемотка вручную: всегда точно следуем за плейхедом
                    state.current_sample_pos = target_sample.min(state.samples.len());
                } else {
                    // При активном непрерывном воспроизведении НЕ дергаем семплы каждые 7-16мс!
                    // Аудиопоток течет плавно без заиканий и щелчков (anti-crackling buffer).
                    // Корректируем ТОЛЬКО при реальном прыжке/клике по таймлайну (>200 мс).
                    let diff = (state.current_sample_pos as isize - target_sample as isize).abs();
                    let threshold = (sample_rate * 0.20 * state.channels as f64) as isize;
                    if diff > threshold {
                        state.current_sample_pos = target_sample.min(state.samples.len());
                    }
                }
            } else {
                state.current_sample_pos = state.samples.len();
            }
        }
    }

    /// Получить текущие уровни VU-метра для анимации
    pub fn get_vu_levels(&self) -> (f32, f32) {
        if let Ok(state) = self.shared_state.lock() {
            (state.current_vu_l, state.current_vu_r)
        } else {
            (0.0, 0.0)
        }
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
}
