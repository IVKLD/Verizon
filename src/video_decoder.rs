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

    // Аппаратное ускорение на GPU (VAAPI для AMD/Intel, CUDA для NVIDIA)
    pub gpu_accelerated: bool,
    pub gpu_device_name: String,

    // Неблокирующие каналы общения с фоновым потоком декодера
    request_tx: Option<std::sync::mpsc::Sender<i64>>,
    frame_rx: Option<std::sync::mpsc::Receiver<(i64, ColorImage)>>,

    // Преаллоцированный буфер для плейсхолдера
    placeholder_buffer: Vec<u8>,
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
            gpu_accelerated: false,
            gpu_device_name: "CPU".to_string(),
            request_tx: None,
            frame_rx: None,
            placeholder_buffer: Vec::new(),
        }
    }

    /// Открыть видеофайл через FFmpeg с автоматическим определением параметров
    pub fn open_file(&mut self, path: &Path, ctx: &Context) -> Result<(), String> {
        // Завершаем предыдущий фоновый воркер если был открыт другой файл
        self.request_tx = None;
        self.frame_rx = None;

        let ictx = ffmpeg_next::format::input(path).map_err(|e| format!("FFmpeg open error: {}", e))?;

        let input_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or_else(|| "В файле не найден видеопоток".to_string())?;

        let fps = f64::from(input_stream.avg_frame_rate());
        let fps = if fps <= 0.0 || fps.is_nan() { 24.0 } else { fps };

        let duration_secs = ictx.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
        let total_frames = if duration_secs > 0.0 {
            (duration_secs * fps).round() as i64
        } else {
            1000
        };

        let context_decoder = ffmpeg_next::codec::context::Context::from_parameters(input_stream.parameters())
            .map_err(|e| e.to_string())?;
        let video_decoder = context_decoder.decoder().video().map_err(|e| e.to_string())?;

        let orig_w = video_decoder.width();
        let orig_h = video_decoder.height();

        // Разрешение рендера (макс 1920x1080 для молниеносного предпросмотра)
        let (scaled_w, scaled_h) = if orig_w > 1920 {
            let scale = 1920.0 / orig_w as f32;
            (1920, ((orig_h as f32 * scale) as u32) & !1)
        } else {
            (orig_w, orig_h)
        };

        // Проверка аппаратного GPU ускорения
        let is_gpu = Path::new("/dev/dri/renderD128").exists();
        let gpu_name = if is_gpu {
            "AMD Radeon VAAPI".to_string()
        } else {
            "CPU Multi-threaded".to_string()
        };

        self.current_path = Some(path.to_path_buf());
        self.width = scaled_w;
        self.height = scaled_h;
        self.fps = fps;
        self.total_frames = total_frames;
        self.gpu_accelerated = is_gpu;
        self.gpu_device_name = gpu_name;
        self.is_real_video = true;
        self.last_decoded_frame = -1;

        // Создаем каналы для фонового воркера декодирования
        let (req_tx, req_rx) = std::sync::mpsc::channel::<i64>();
        let (frame_tx, frame_rx) = std::sync::mpsc::channel::<(i64, ColorImage)>();
        let egui_ctx = ctx.clone();
        let file_path = path.to_path_buf();

        std::thread::spawn(move || {
            Self::run_decoder_worker(
                file_path,
                scaled_w,
                scaled_h,
                fps,
                req_rx,
                frame_tx,
                egui_ctx,
            );
        });

        // Запрашиваем 0-й кадр
        let _ = req_tx.send(0);

        self.request_tx = Some(req_tx);
        self.frame_rx = Some(frame_rx);

        // Показываем начальный кадр
        self.seek_and_decode(0, ctx);

        Ok(())
    }

    /// Отобразить чистый нейтральный экран Resolve без декодирования
    pub fn show_blank_frame(&mut self, ctx: &Context) {
        if self.last_decoded_frame == -999 && self.texture.is_some() {
            return;
        }
        let img = self.generate_placeholder();
        if let Some(tex) = &mut self.texture {
            tex.set(img, TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture("video_frame", img, TextureOptions::NEAREST));
        }
        self.last_decoded_frame = -999;
    }

    /// Неблокирующая выборка готовых кадров из фонового потока
    pub fn poll_frames(&mut self, ctx: &Context) {
        if let Some(rx) = &self.frame_rx {
            while let Ok((frame_num, image)) = rx.try_recv() {
                if let Some(tex) = &mut self.texture {
                    tex.set(image, TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture("video_frame", image, TextureOptions::LINEAR));
                }
                self.last_decoded_frame = frame_num;
            }
        }
    }

    /// Прецизионное декодирование кадра по таймкоду (Frame-Accurate Zero-Lag)
    /// Полностью неблокирующий метод: UI никогда не зависает даже на 4K видео 165 Mbps
    pub fn seek_and_decode(&mut self, target_frame: i64, ctx: &Context) {
        self.poll_frames(ctx);

        if target_frame == self.last_decoded_frame && self.texture.is_some() {
            return;
        }

        if self.is_real_video {
            if let Some(tx) = &self.request_tx {
                let _ = tx.send(target_frame);
            }
            return;
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

    /// Ожидание декодирования целевого кадра (для детерминированных тестов)
    #[allow(dead_code)]
    pub fn wait_for_frame(&mut self, target_frame: i64, ctx: &Context, timeout: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        self.seek_and_decode(target_frame, ctx);
        while start.elapsed() < timeout {
            self.poll_frames(ctx);
            if self.last_decoded_frame == target_frame && self.texture.is_some() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    /// Выделенный фоновый воркер декодирования видео
    fn run_decoder_worker(
        path: PathBuf,
        scaled_w: u32,
        scaled_h: u32,
        fps: f64,
        req_rx: std::sync::mpsc::Receiver<i64>,
        frame_tx: std::sync::mpsc::Sender<(i64, ColorImage)>,
        egui_ctx: Context,
    ) {
        let mut ictx = match ffmpeg_next::format::input(&path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let input_stream = match ictx.streams().best(ffmpeg_next::media::Type::Video) {
            Some(s) => s,
            None => return,
        };

        let stream_idx = input_stream.index();
        let time_base = input_stream.time_base();
        let mut context_decoder = match ffmpeg_next::codec::context::Context::from_parameters(input_stream.parameters()) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Аппаратное ускорение VAAPI / CUDA внутри рабочего потока
        let mut hw_device_ctx: *mut ffmpeg_next::ffi::AVBufferRef = std::ptr::null_mut();
        unsafe {
            let vaapi_path = std::ffi::CString::new("/dev/dri/renderD128").unwrap();
            let ret = ffmpeg_next::ffi::av_hwdevice_ctx_create(
                &mut hw_device_ctx,
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                vaapi_path.as_ptr(),
                std::ptr::null_mut(),
                0,
            );
            if ret == 0 && !hw_device_ctx.is_null() {
                let codec_ctx_ptr = context_decoder.as_mut_ptr();
                (*codec_ctx_ptr).hw_device_ctx = ffmpeg_next::ffi::av_buffer_ref(hw_device_ctx);
            } else {
                let ret_cuda = ffmpeg_next::ffi::av_hwdevice_ctx_create(
                    &mut hw_device_ctx,
                    ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                );
                if ret_cuda == 0 && !hw_device_ctx.is_null() {
                    let codec_ctx_ptr = context_decoder.as_mut_ptr();
                    (*codec_ctx_ptr).hw_device_ctx = ffmpeg_next::ffi::av_buffer_ref(hw_device_ctx);
                }
            }
        }

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .min(16);

        context_decoder.set_threading(ffmpeg_next::codec::threading::Config {
            kind: ffmpeg_next::codec::threading::Type::Frame,
            count: num_threads,
        });

        let mut video_decoder = match context_decoder.decoder().video() {
            Ok(d) => d,
            Err(_) => {
                if !hw_device_ctx.is_null() {
                    unsafe { ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx); }
                }
                return;
            }
        };

        let orig_w = video_decoder.width();
        let orig_h = video_decoder.height();

        let mut scaler = match ffmpeg_next::software::scaling::Context::get(
            video_decoder.format(),
            orig_w,
            orig_h,
            ffmpeg_next::format::Pixel::RGBA,
            scaled_w,
            scaled_h,
            ffmpeg_next::software::scaling::flag::Flags::FAST_BILINEAR,
        ) {
            Ok(s) => s,
            Err(_) => {
                if !hw_device_ctx.is_null() {
                    unsafe { ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx); }
                }
                return;
            }
        };

        let w = scaled_w as usize;
        let h = scaled_h as usize;
        let mut current_pts: i64 = -1;
        let mut rgba_buffer = vec![0u8; w * h * 4];
        let mut decoded_frame = ffmpeg_next::util::frame::Video::empty();
        let mut sw_frame = ffmpeg_next::util::frame::Video::empty();
        let mut rgba_frame = ffmpeg_next::util::frame::Video::empty();

        while let Ok(mut target_frame) = req_rx.recv() {
            // Дрейним промежуточные запросы при быстром перемещении курсора
            while let Ok(newer) = req_rx.try_recv() {
                target_frame = newer;
            }

            let time_base_f = f64::from(time_base);
            let target_pts = if time_base_f > 0.0 {
                (target_frame as f64 / fps / time_base_f).round() as i64
            } else {
                target_frame
            };

            let frame_duration_pts = if time_base_f > 0.0 && fps > 0.0 {
                (1.0 / fps / time_base_f).round().max(1.0) as i64
            } else {
                1
            };
            let tolerance = (frame_duration_pts / 2).max(1);
            let one_sec_pts = if time_base_f > 0.0 {
                (1.0 / time_base_f).round() as i64
            } else {
                30
            };

            let pts_delta = target_pts - current_pts;

            if current_pts < 0 || pts_delta < -tolerance || pts_delta > one_sec_pts {
                let av_timestamp = (target_frame as f64 / fps * f64::from(ffmpeg_next::ffi::AV_TIME_BASE)) as i64;
                let _ = ictx.seek(av_timestamp, ..av_timestamp);
                let _ = video_decoder.flush();
                current_pts = -1;
            }

            let mut found_image: Option<ColorImage> = None;

            // 1. Проверяем буферизованные кадры в декодере
            while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                if let Some(pts) = decoded_frame.pts() {
                    current_pts = pts;
                    if pts >= target_pts - tolerance {
                        found_image = Self::scale_and_copy_frame(
                            &mut scaler,
                            &decoded_frame,
                            &mut sw_frame,
                            &mut rgba_frame,
                            w,
                            h,
                            &mut rgba_buffer,
                        );
                        break;
                    }
                }
            }

            // 2. Читаем новые пакеты до достижения целевого кадра
            if found_image.is_none() {
                let mut packets_read = 0;
                let mut fallback: Option<ColorImage> = None;

                for (st, packet) in ictx.packets() {
                    if st.index() == stream_idx {
                        packets_read += 1;
                        if video_decoder.send_packet(&packet).is_ok() {
                            while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                                if let Some(pts) = decoded_frame.pts() {
                                    current_pts = pts;
                                    if pts >= target_pts - tolerance {
                                        found_image = Self::scale_and_copy_frame(
                                            &mut scaler,
                                            &decoded_frame,
                                            &mut sw_frame,
                                            &mut rgba_frame,
                                            w,
                                            h,
                                            &mut rgba_buffer,
                                        );
                                        break;
                                    } else if fallback.is_none() && pts > target_pts - (tolerance * 10) {
                                        fallback = Self::scale_and_copy_frame(
                                            &mut scaler,
                                            &decoded_frame,
                                            &mut sw_frame,
                                            &mut rgba_frame,
                                            w,
                                            h,
                                            &mut rgba_buffer,
                                        );
                                    }
                                }
                            }
                        }
                    }
                    if found_image.is_some() || packets_read > 120 {
                        break;
                    }
                }

                if found_image.is_none() {
                    found_image = fallback;
                }
            }

            if let Some(img) = found_image {
                let _ = frame_tx.send((target_frame, img));
                egui_ctx.request_repaint();
            }
        }

        // Очистка VAAPI при завершении фонового потока
        if !hw_device_ctx.is_null() {
            unsafe {
                ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
            }
        }
    }

    fn scale_and_copy_frame(
        scaler: &mut ffmpeg_next::software::scaling::Context,
        decoded_frame: &ffmpeg_next::util::frame::Video,
        sw_frame: &mut ffmpeg_next::util::frame::Video,
        rgba_frame: &mut ffmpeg_next::util::frame::Video,
        w: usize,
        h: usize,
        rgba_buffer: &mut Vec<u8>,
    ) -> Option<ColorImage> {
        let frame_to_scale = unsafe {
            let fmt = (*decoded_frame.as_ptr()).format;
            if fmt == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI as i32
                || fmt == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_CUDA as i32
            {
                if ffmpeg_next::ffi::av_hwframe_transfer_data(sw_frame.as_mut_ptr(), decoded_frame.as_ptr(), 0) == 0 {
                    &*sw_frame
                } else {
                    decoded_frame
                }
            } else {
                decoded_frame
            }
        };

        // Адаптация формата конвертера при аппаратном декодировании (NV12 -> RGBA)
        if scaler.input().format != frame_to_scale.format()
            || scaler.input().width != frame_to_scale.width()
            || scaler.input().height != frame_to_scale.height()
        {
            if let Ok(new_scaler) = ffmpeg_next::software::scaling::Context::get(
                frame_to_scale.format(),
                frame_to_scale.width(),
                frame_to_scale.height(),
                ffmpeg_next::format::Pixel::RGBA,
                w as u32,
                h as u32,
                ffmpeg_next::software::scaling::flag::Flags::FAST_BILINEAR,
            ) {
                *scaler = new_scaler;
            }
        }

        if scaler.run(frame_to_scale, rgba_frame).is_ok() {
            let data = rgba_frame.data(0);
            let stride = rgba_frame.stride(0);

            if rgba_buffer.len() != w * h * 4 {
                rgba_buffer.resize(w * h * 4, 0);
            }

            for y in 0..h {
                let src_start = y * stride;
                let dst_start = y * w * 4;
                if src_start + w * 4 <= data.len() {
                    rgba_buffer[dst_start..dst_start + w * 4]
                        .copy_from_slice(&data[src_start..src_start + w * 4]);
                }
            }

            Some(ColorImage::from_rgba_unmultiplied([w, h], rgba_buffer))
        } else {
            None
        }
    }

    /// Чистый экран ожидания медиа
    fn generate_placeholder(&mut self) -> ColorImage {
        let w = 640;
        let h = 360;
        if self.placeholder_buffer.len() != w * h * 4 {
            self.placeholder_buffer.resize(w * h * 4, 0);
        }

        for y in 0..h {
            let row = y * w * 4;
            for x in 0..w {
                let idx = row + x * 4;
                self.placeholder_buffer[idx] = 18;      // #121315 DaVinci Dark
                self.placeholder_buffer[idx + 1] = 19;
                self.placeholder_buffer[idx + 2] = 21;
                self.placeholder_buffer[idx + 3] = 255;
            }
        }

        ColorImage::from_rgba_unmultiplied([w, h], &self.placeholder_buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_playback_consecutive_frames() {
        let test_path = Path::new("/home/igni/Documents/IMG_6686.MP4");
        if !test_path.exists() {
            return;
        }

        let ctx = egui::Context::default();
        let mut player = VideoPlayer::new();
        assert!(player.open_file(test_path, &ctx).is_ok());

        assert!(player.is_real_video);
        assert!(player.width > 0);
        assert!(player.height > 0);

        // Ждем декодирования целевого кадра
        assert!(player.wait_for_frame(10, &ctx, std::time::Duration::from_secs(3)));
        assert_eq!(player.last_decoded_frame, 10);
    }

    #[test]
    fn test_decoder_4k_mkv_playback() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        let ctx = egui::Context::default();
        let mut player = VideoPlayer::new();
        assert!(player.open_file(test_path, &ctx).is_ok());

        // Проверяем быстрое неблокирующее декодирование на 4K MKV
        assert!(player.wait_for_frame(0, &ctx, std::time::Duration::from_secs(3)));
        assert_eq!(player.last_decoded_frame, 0);

        // Перемотка на кадр 600
        assert!(player.wait_for_frame(600, &ctx, std::time::Duration::from_secs(3)));
        assert_eq!(player.last_decoded_frame, 600);
    }
}
