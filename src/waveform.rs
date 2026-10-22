use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct WaveformManager {
    cache: Arc<Mutex<HashMap<PathBuf, Vec<f32>>>>,
    pending: Arc<Mutex<HashSet<PathBuf>>>,
}

impl WaveformManager {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Получить кэшированную волновую форму (100 точек на секунду)
    pub fn get_waveform(&self, path: &Path) -> Option<Vec<f32>> {
        self.cache.lock().ok()?.get(path).cloned()
    }

    /// Асинхронно запросить расшифровку аудио части в фоне через Tokio / Thread pool
    pub fn request_waveform(&self, path: &Path, ctx: egui::Context) {
        let path_buf = path.to_path_buf();

        {
            let mut pending = match self.pending.lock() {
                Ok(p) => p,
                Err(_) => return,
            };
            if pending.contains(&path_buf) {
                return;
            }
            if let Ok(cache) = self.cache.lock() {
                if cache.contains_key(&path_buf) {
                    return;
                }
            }
            pending.insert(path_buf.clone());
        }

        let cache = self.cache.clone();
        let pending = self.pending.clone();

        tokio::spawn(async move {
            let path_for_blocking = path_buf.clone();
            let result = tokio::task::spawn_blocking(move || {
                Self::extract_audio_peaks(&path_for_blocking)
            })
            .await;

            if let Ok(Ok(peaks)) = result {
                if let Ok(mut c) = cache.lock() {
                    c.insert(path_buf.clone(), peaks);
                }
            }

            if let Ok(mut p) = pending.lock() {
                p.remove(&path_buf);
            }

            ctx.request_repaint();
        });
    }

    /// Быстрая расшифровка амплитуд аудио (100 замеров в секунду) через FFmpeg
    fn extract_audio_peaks(path: &Path) -> Result<Vec<f32>, String> {
        let mut ictx = ffmpeg_next::format::input(path).map_err(|e| e.to_string())?;

        let audio_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Audio)
            .ok_or_else(|| "Аудиопоток не найден".to_string())?;

        let stream_idx = audio_stream.index();
        let context_decoder = ffmpeg_next::codec::context::Context::from_parameters(audio_stream.parameters())
            .map_err(|e| e.to_string())?;
        let mut decoder = context_decoder.decoder().audio().map_err(|e| e.to_string())?;

        let in_rate = decoder.rate();
        let in_format = decoder.format();
        let in_channel_layout = if decoder.channel_layout().is_empty() {
            if decoder.channels() == 1 {
                ffmpeg_next::channel_layout::ChannelLayout::MONO
            } else {
                ffmpeg_next::channel_layout::ChannelLayout::STEREO
            }
        } else {
            decoder.channel_layout()
        };

        // 8000 Гц моно — молниеносная скорость конвертации и идеальная точность для отображения волноформы
        let out_rate = 8000;
        let out_channel_layout = ffmpeg_next::channel_layout::ChannelLayout::MONO;
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

        let mut decoded_frame = ffmpeg_next::util::frame::Audio::empty();
        let mut resampled_frame = ffmpeg_next::util::frame::Audio::empty();

        // 100 точек на секунду: при 8000 Гц один бакет равен 80 семплам (10 мс)
        let samples_per_point = 80;
        let mut peaks = Vec::new();
        let mut current_max: f32 = 0.0;
        let mut count_in_bucket = 0;

        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_idx {
                if decoder.send_packet(&packet).is_ok() {
                    while decoder.receive_frame(&mut decoded_frame).is_ok() {
                        if decoded_frame.channel_layout().is_empty() {
                            decoded_frame.set_channel_layout(in_channel_layout);
                        }
                        if resampler.run(&decoded_frame, &mut resampled_frame).is_ok() {
                            let samples_count = resampled_frame.samples();
                            let data: &[f32] = unsafe {
                                std::slice::from_raw_parts(
                                    resampled_frame.data(0).as_ptr() as *const f32,
                                    samples_count,
                                )
                            };
                            for &sample in data {
                                let abs = sample.abs();
                                if abs > current_max {
                                    current_max = abs;
                                }
                                count_in_bucket += 1;
                                if count_in_bucket >= samples_per_point {
                                    // Нормализация с шумоподавлением: тишина (<0.005) становится 0.0
                                    let normalized = if current_max < 0.005 {
                                        0.0
                                    } else {
                                        current_max.min(1.0)
                                    };
                                    peaks.push(normalized);
                                    current_max = 0.0;
                                    count_in_bucket = 0;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Завершающий бакет
        if count_in_bucket > 0 {
            let normalized = if current_max < 0.005 { 0.0 } else { current_max.min(1.0) };
            peaks.push(normalized);
        }

        Ok(peaks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_audio_peaks() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        let t0 = std::time::Instant::now();
        let peaks = WaveformManager::extract_audio_peaks(test_path).expect("Failed to extract peaks");
        println!("Waveform extraction took: {:?}, extracted {} points", t0.elapsed(), peaks.len());
        assert!(!peaks.is_empty(), "Peaks should not be empty");
        let has_sound = peaks.iter().any(|&p| p > 0.05);
        assert!(has_sound, "Audio must contain audible sound");
    }
}
