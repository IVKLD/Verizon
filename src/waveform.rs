use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct WaveformManager {
    cache: Arc<Mutex<HashMap<(PathBuf, usize), Arc<Vec<f32>>>>>,
    pending: Arc<Mutex<HashSet<(PathBuf, usize)>>>,
}

impl WaveformManager {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            pending: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Получить кэшированную волновую форму для конкретного аудиопотока
    /// Возвращает Arc: отрисовка клипа каждый кадр не копирует массив пиков (zero-copy)
    pub fn get_waveform_stream(&self, path: &Path, audio_track_index: usize) -> Option<Arc<Vec<f32>>> {
        let key = (path.to_path_buf(), audio_track_index);
        self.cache.lock().ok()?.get(&key).cloned()
    }

    /// Принудительно положить пики в кэш (тесты / предзагрузка)
    #[allow(dead_code)]
    pub fn insert_waveform(&self, path: &Path, audio_track_index: usize, peaks: Vec<f32>) {
        if let Ok(mut c) = self.cache.lock() {
            c.insert((path.to_path_buf(), audio_track_index), Arc::new(peaks));
        }
    }

    /// Получить кэшированную волновую форму (первая дорожка по умолчанию)
    #[allow(dead_code)]
    pub fn get_waveform(&self, path: &Path) -> Option<Arc<Vec<f32>>> {
        self.get_waveform_stream(path, 0)
    }

    /// Асинхронно запросить расшифровку аудиопотока в фоне через Tokio / Thread pool
    pub fn request_waveform_stream(&self, path: &Path, audio_track_index: usize, ctx: egui::Context) {
        let key = (path.to_path_buf(), audio_track_index);

        {
            let mut pending = match self.pending.lock() {
                Ok(p) => p,
                Err(_) => return,
            };
            if pending.contains(&key) {
                return;
            }
            if let Ok(cache) = self.cache.lock() {
                if cache.contains_key(&key) {
                    return;
                }
            }
            pending.insert(key.clone());
        }

        let cache = self.cache.clone();
        let pending = self.pending.clone();
        let path_buf = path.to_path_buf();

        tokio::spawn(async move {
            let path_for_blocking = path_buf.clone();
            let result = tokio::task::spawn_blocking(move || {
                Self::extract_audio_peaks_stream(&path_for_blocking, audio_track_index)
            })
            .await;

            if let Ok(Ok(peaks)) = result {
                if let Ok(mut c) = cache.lock() {
                    c.insert(key.clone(), Arc::new(peaks));
                }
                ctx.request_repaint();
            }

            if let Ok(mut p) = pending.lock() {
                p.remove(&key);
            }
        });
    }

    #[allow(dead_code)]
    pub fn request_waveform(&self, path: &Path, ctx: egui::Context) {
        self.request_waveform_stream(path, 0, ctx);
    }

    /// Быстрая расшифровка амплитуд конкретного аудиопотока (100 замеров в секунду) через FFmpeg
    pub fn extract_audio_peaks_stream(path: &Path, audio_track_index: usize) -> Result<Vec<f32>, String> {
        let mut ictx = ffmpeg_next::format::input(path).map_err(|e| e.to_string())?;

        let audio_streams: Vec<_> = ictx
            .streams()
            .filter(|s| s.parameters().medium() == ffmpeg_next::media::Type::Audio)
            .collect();

        if audio_streams.is_empty() {
            return Err("Аудиопотоки не найдены".to_string());
        }

        let audio_stream = audio_streams
            .get(audio_track_index)
            .or_else(|| audio_streams.first())
            .unwrap();

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

                            for &s in data {
                                let abs_s = s.abs();
                                if abs_s > current_max {
                                    current_max = abs_s;
                                }
                                count_in_bucket += 1;
                                if count_in_bucket >= samples_per_point {
                                    peaks.push(current_max);
                                    current_max = 0.0;
                                    count_in_bucket = 0;
                                }
                            }
                        }
                    }
                }
            }
        }

        if count_in_bucket > 0 {
            peaks.push(current_max);
        }

        Ok(peaks)
    }

    #[allow(dead_code)]
    pub fn extract_audio_peaks(path: &Path) -> Result<Vec<f32>, String> {
        Self::extract_audio_peaks_stream(path, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_audio_peaks() {
        let path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !path.exists() {
            return;
        }

        let start = std::time::Instant::now();
        let res = WaveformManager::extract_audio_peaks(path);
        let elapsed = start.elapsed();
        assert!(res.is_ok());
        let peaks = res.unwrap();
        println!("Waveform extraction took: {:?}, extracted {} points", elapsed, peaks.len());
        assert!(!peaks.is_empty());
    }

    #[test]
    fn test_extract_multitrack_peaks() {
        let path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !path.exists() {
            return;
        }

        let peaks1 = WaveformManager::extract_audio_peaks_stream(path, 0).unwrap();
        let peaks2 = WaveformManager::extract_audio_peaks_stream(path, 1).unwrap();
        assert!(!peaks1.is_empty());
        assert!(!peaks2.is_empty());
    }
}
