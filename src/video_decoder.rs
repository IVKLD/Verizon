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
            total_frames: 2400,
            texture: None,
            is_real_video: false,
            last_decoded_frame: -1,
            ictx: None,
            decoder: None,
            scaler: None,
            video_stream_idx: 0,
            rgba_buffer: Vec::new(),
        }
    }

    /// Открыть видеофайл через FFmpeg
    pub fn open_file(&mut self, path: &Path, ctx: &Context) -> Result<(), String> {
        let ictx = ffmpeg_next::format::input(path).map_err(|e| format!("FFmpeg open error: {}", e))?;

        let input_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or_else(|| "В файле не найден видеопоток".to_string())?;

        let stream_idx = input_stream.index();
        let context_decoder = ffmpeg_next::codec::context::Context::from_parameters(input_stream.parameters())
            .map_err(|e| e.to_string())?;
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

        // Масштабируем до удобного превью-разрешения (макс 1280x720 для мгновенного рендера)
        let (scaled_w, scaled_h) = if width > 1280 {
            let scale = 1280.0 / width as f32;
            (1280, ((height as f32 * scale) as u32) & !1)
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
        self.is_real_video = true;
        self.last_decoded_frame = -1;
        self.rgba_buffer = vec![0u8; (scaled_w * scaled_h * 4) as usize];

        // Декодировать первый кадр
        self.seek_and_decode(0, ctx);

        Ok(())
    }

    /// Быстрое декодирование нужного кадра (Zero-Lag Playback)
    pub fn seek_and_decode(&mut self, target_frame: i64, ctx: &Context) {
        // Если кадр уже декодирован, ничего не пересчитываем!
        if target_frame == self.last_decoded_frame && self.texture.is_some() {
            return;
        }

        if self.is_real_video && self.ictx.is_some() {
            if let Some(image) = self.decode_frame_at(target_frame) {
                if let Some(tex) = &mut self.texture {
                    tex.set(image, TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture("video_frame", image, TextureOptions::LINEAR));
                }
                self.last_decoded_frame = target_frame;
                return;
            }
        }

        // Если файл не открыт, генерируем легкий превью-кадр
        let img = self.generate_fast_frame(target_frame);
        if let Some(tex) = &mut self.texture {
            tex.set(img, TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture("video_frame", img, TextureOptions::NEAREST));
        }
        self.last_decoded_frame = target_frame;
    }

    fn decode_frame_at(&mut self, target_frame: i64) -> Option<ColorImage> {
        let ictx = self.ictx.as_mut()?;
        let decoder = self.decoder.as_mut()?;
        let scaler = self.scaler.as_mut()?;
        let stream_idx = self.video_stream_idx;

        let delta = target_frame - self.last_decoded_frame;

        // КРИТИЧЕСКАЯ ОПТИМИЗАЦИЯ: Делаем Seek ТОЛЬКО при прыжках/перемотках!
        // При последовательном воспроизведении (delta == 1 или 2) мы просто читаем следующий пакет!
        if delta < 0 || delta > 6 {
            let timestamp = (target_frame as f64 / self.fps * f64::from(ffmpeg_next::ffi::AV_TIME_BASE)) as i64;
            let _ = ictx.seek(timestamp, ..timestamp);
            let _ = decoder.flush();
        }

        let mut decoded_frame = ffmpeg_next::util::frame::Video::empty();
        let mut rgba_frame = ffmpeg_next::util::frame::Video::empty();

        let w = self.width as usize;
        let h = self.height as usize;

        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_idx {
                if decoder.send_packet(&packet).is_ok() {
                    while decoder.receive_frame(&mut decoded_frame).is_ok() {
                        if scaler.run(&decoded_frame, &mut rgba_frame).is_ok() {
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

                            return Some(ColorImage::from_rgba_unmultiplied(
                                [w, h],
                                &self.rgba_buffer,
                            ));
                        }
                    }
                }
            }
        }
        None
    }

    /// Сверхбыстрый генератор тестового видеокадра
    fn generate_fast_frame(&mut self, frame_num: i64) -> ColorImage {
        let w = 480;
        let h = 270;
        if self.rgba_buffer.len() != w * h * 4 {
            self.rgba_buffer.resize(w * h * 4, 255);
        }

        let scan_x = ((frame_num * 6) % (w as i64)) as usize;
        let phase = ((frame_num as f32 * 0.1).sin() * 40.0) as i32;

        for y in 0..h {
            let row_offset = y * w * 4;
            let r_base = ((y * 180) / h) as u8;
            for x in 0..w {
                let idx = row_offset + x * 4;
                if (x as isize - scan_x as isize).abs() < 3 {
                    self.rgba_buffer[idx] = 232;     // Resolve Orange
                    self.rgba_buffer[idx + 1] = 124;
                    self.rgba_buffer[idx + 2] = 36;
                } else {
                    let g_val = (((x as i32 + phase) * 120) / w as i32).clamp(20, 160) as u8;
                    self.rgba_buffer[idx] = r_base / 2 + 20;
                    self.rgba_buffer[idx + 1] = g_val / 2 + 30;
                    self.rgba_buffer[idx + 2] = 60;
                }
            }
        }

        ColorImage::from_rgba_unmultiplied([w, h], &self.rgba_buffer)
    }
}
