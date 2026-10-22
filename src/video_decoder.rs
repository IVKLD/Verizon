use egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::path::{Path, PathBuf};

pub struct VideoPlayer {
    pub current_path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub total_frames: i64,
    pub texture: Option<TextureHandle>,
    pub is_real_video: bool,
    pub last_decoded_frame: i64,

    // FFmpeg структуры
    ictx: Option<ffmpeg_next::format::context::Input>,
    decoder: Option<ffmpeg_next::codec::decoder::Video>,
    scaler: Option<ffmpeg_next::software::scaling::Context>,
    video_stream_idx: usize,
    time_base: ffmpeg_next::Rational,
    current_pts: i64,

    // Преаллоцированный RGBA буфер
    rgba_buffer: Vec<u8>,
}

impl VideoPlayer {
    pub fn new() -> Self {
        let _ = ffmpeg_next::init();
        Self {
            current_path: None,
            width: 1280,
            height: 720,
            fps: 24.0,
            total_frames: 0,
            texture: None,
            is_real_video: false,
            last_decoded_frame: -1,
            ictx: None,
            decoder: None,
            scaler: None,
            video_stream_idx: 0,
            time_base: ffmpeg_next::Rational(1, 1000),
            current_pts: -1,
            rgba_buffer: Vec::new(),
        }
    }

    /// Открыть видеофайл через FFmpeg с автоматическим определением параметров
    pub fn open_file(&mut self, path: &Path, ctx: &Context) -> Result<(), String> {
        let ictx = ffmpeg_next::format::input(path).map_err(|e| format!("FFmpeg open error: {}", e))?;

        let input_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or_else(|| "В файле не найден видеопоток".to_string())?;

        let stream_idx = input_stream.index();
        let time_base = input_stream.time_base();
        let mut context_decoder = ffmpeg_next::codec::context::Context::from_parameters(input_stream.parameters())
            .map_err(|e| e.to_string())?;

        // Включение многопоточного декодирования (Multi-threaded 4K Decoding) на всех ядрах CPU
        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .min(16);

        context_decoder.set_threading(ffmpeg_next::codec::threading::Config {
            kind: ffmpeg_next::codec::threading::Type::Frame,
            count: num_threads,
        });

        let video_decoder = context_decoder.decoder().video().map_err(|e| e.to_string())?;

        let width = video_decoder.width();
        let height = video_decoder.height();
        let fps = f64::from(input_stream.avg_frame_rate());
        let fps = if fps <= 0.0 || fps.is_nan() { 24.0 } else { fps };

        let duration_secs = ictx.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
        let total_frames = if duration_secs > 0.0 {
            (duration_secs * fps).round() as i64
        } else {
            1000
        };

        // Разрешение рендера (макс 1920x1080 для качественного предпросмотра)
        let (scaled_w, scaled_h) = if width > 1920 {
            let scale = 1920.0 / width as f32;
            (1920, ((height as f32 * scale) as u32) & !1)
        } else {
            (width, height)
        };

        let scaler = ffmpeg_next::software::scaling::Context::get(
            video_decoder.format(),
            width,
            height,
            ffmpeg_next::format::Pixel::RGBA,
            scaled_w,
            scaled_h,
            ffmpeg_next::software::scaling::flag::Flags::FAST_BILINEAR,
        )
        .map_err(|e| e.to_string())?;

        self.current_path = Some(path.to_path_buf());
        self.width = scaled_w;
        self.height = scaled_h;
        self.fps = fps;
        self.total_frames = total_frames;
        self.ictx = Some(ictx);
        self.decoder = Some(video_decoder);
        self.scaler = Some(scaler);
        self.video_stream_idx = stream_idx;
        self.time_base = time_base;
        self.current_pts = -1;
        self.is_real_video = true;
        self.last_decoded_frame = -1;
        self.rgba_buffer = vec![0u8; (scaled_w * scaled_h * 4) as usize];

        // Декодировать первый кадр
        self.seek_and_decode(0, ctx);

        Ok(())
    }

    /// Прецизионное декодирование кадра по таймкоду (Frame-Accurate Zero-Lag)
    pub fn seek_and_decode(&mut self, target_frame: i64, ctx: &Context) {
        if target_frame == self.last_decoded_frame && self.texture.is_some() {
            return;
        }

        if self.is_real_video && self.ictx.is_some() {
            if let Some(image) = self.decode_frame_pts(target_frame) {
                if let Some(tex) = &mut self.texture {
                    tex.set(image, TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture("video_frame", image, TextureOptions::LINEAR));
                }
                self.last_decoded_frame = target_frame;
                return;
            } else if self.texture.is_some() {
                // Если достигнут конец или кадр временно недоступен, удерживаем последний кадр
                self.last_decoded_frame = target_frame;
                return;
            }
        }

        // Если файл не открыт, отображаем нейтральный экран Resolve
        let img = self.generate_placeholder();
        if let Some(tex) = &mut self.texture {
            tex.set(img, TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture("video_frame", img, TextureOptions::NEAREST));
        }
        self.last_decoded_frame = target_frame;
    }

    fn decode_frame_pts(&mut self, target_frame: i64) -> Option<ColorImage> {
        let ictx = self.ictx.as_mut()?;
        let decoder = self.decoder.as_mut()?;
        let scaler = self.scaler.as_mut()?;
        let stream_idx = self.video_stream_idx;
        let time_base_f = f64::from(self.time_base);

        let target_pts = if time_base_f > 0.0 {
            (target_frame as f64 / self.fps / time_base_f).round() as i64
        } else {
            target_frame
        };

        let one_sec_pts = if time_base_f > 0.0 {
            (1.0 / time_base_f).round() as i64
        } else {
            30
        };

        let pts_delta = target_pts - self.current_pts;

        // Перемотка (Seek): если кадр позади текущего или дальше чем на 1 секунду вперед
        if self.current_pts < 0 || pts_delta < 0 || pts_delta > one_sec_pts {
            let av_timestamp = (target_frame as f64 / self.fps * f64::from(ffmpeg_next::ffi::AV_TIME_BASE)) as i64;
            let _ = ictx.seek(av_timestamp, ..av_timestamp);
            let _ = decoder.flush();
            self.current_pts = -1;
        }

        let mut decoded_frame = ffmpeg_next::util::frame::Video::empty();
        let mut rgba_frame = ffmpeg_next::util::frame::Video::empty();
        let w = self.width as usize;
        let h = self.height as usize;

        // 1. Проверяем буферизованные кадры в очереди декодера
        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            if let Some(pts) = decoded_frame.pts() {
                self.current_pts = pts;
                if pts >= target_pts {
                    if scaler.run(&decoded_frame, &mut rgba_frame).is_ok() {
                        return self.copy_frame_to_image(&rgba_frame, w, h);
                    }
                }
            }
        }

        // 2. Читаем новые пакеты до достижения целевого PTS
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_idx {
                if decoder.send_packet(&packet).is_ok() {
                    while decoder.receive_frame(&mut decoded_frame).is_ok() {
                        if let Some(pts) = decoded_frame.pts() {
                            self.current_pts = pts;
                            if pts >= target_pts {
                                if scaler.run(&decoded_frame, &mut rgba_frame).is_ok() {
                                    return self.copy_frame_to_image(&rgba_frame, w, h);
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    fn copy_frame_to_image(&mut self, rgba_frame: &ffmpeg_next::util::frame::Video, w: usize, h: usize) -> Option<ColorImage> {
        let data = rgba_frame.data(0);
        let stride = rgba_frame.stride(0);

        if self.rgba_buffer.len() != w * h * 4 {
            self.rgba_buffer.resize(w * h * 4, 0);
        }

        for y in 0..h {
            let src_start = y * stride;
            let dst_start = y * w * 4;
            if src_start + w * 4 <= data.len() {
                self.rgba_buffer[dst_start..dst_start + w * 4]
                    .copy_from_slice(&data[src_start..src_start + w * 4]);
            }
        }

        Some(ColorImage::from_rgba_unmultiplied([w, h], &self.rgba_buffer))
    }

    /// Чистый экран ожидания медиа
    fn generate_placeholder(&mut self) -> ColorImage {
        let w = 640;
        let h = 360;
        if self.rgba_buffer.len() != w * h * 4 {
            self.rgba_buffer.resize(w * h * 4, 0);
        }

        for y in 0..h {
            let row = y * w * 4;
            for x in 0..w {
                let idx = row + x * 4;
                self.rgba_buffer[idx] = 18;      // #121315 DaVinci Dark
                self.rgba_buffer[idx + 1] = 19;
                self.rgba_buffer[idx + 2] = 21;
                self.rgba_buffer[idx + 3] = 255;
            }
        }

        ColorImage::from_rgba_unmultiplied([w, h], &self.rgba_buffer)
    }
}
