use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct AudioSharedState {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub current_sample_pos: usize,
    pub is_playing: bool,
    pub volume: f32,
}

pub struct AudioPlayer {
    pub shared_state: Arc<Mutex<AudioSharedState>>,
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
        }));

        let stream = Self::init_cpal_stream(shared_state.clone());

        Self {
            shared_state,
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
                return;
            }

            let volume = state.volume;
            let total = state.samples.len();

            for sample in output.iter_mut() {
                if state.current_sample_pos < total {
                    *sample = state.samples[state.current_sample_pos] * volume;
                    state.current_sample_pos += 1;
                } else {
                    *sample = 0.0;
                }
            }
        } else {
            output.fill(0.0);
        }
    }

    /// Загрузить аудио из медиафайла через FFmpeg
    pub fn load_audio_from_file(&mut self, path: &Path) -> Result<(), String> {
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
        let in_channel_layout = audio_decoder.channel_layout();

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

        if let Ok(mut state) = self.shared_state.lock() {
            state.samples = all_samples;
            state.sample_rate = out_rate;
            state.channels = 2;
            state.current_sample_pos = 0;
        }

        Ok(())
    }

    /// Синхронизация текущей позиции звука с плейхедом видео
    pub fn sync_playhead(&self, frame: i64, fps: f64, is_playing: bool) {
        if let Ok(mut state) = self.shared_state.lock() {
            state.is_playing = is_playing;
            let sample_rate = state.sample_rate as f64;
            let target_sample = ((frame as f64 / fps) * sample_rate * state.channels as f64).max(0.0) as usize;
            // Если расхождение больше 0.1 сек, корректируем позицию звука
            let diff = (state.current_sample_pos as isize - target_sample as isize).abs();
            if diff > (sample_rate * 0.1 * state.channels as f64) as isize || !is_playing {
                state.current_sample_pos = target_sample.min(state.samples.len());
            }
        }
    }
}
