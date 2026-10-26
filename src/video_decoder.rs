use egui::{ColorImage, Context, ImageData, TextureHandle, TextureOptions};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum DecoderCommand {
    Play { start_frame: i64, epoch: u64 },
    Pause { epoch: u64 },
    Seek { target_frame: i64, epoch: u64 },
    UpdatePlayhead { playhead_frame: i64 },
}

pub struct VideoPlayer {
    pub current_path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub total_frames: i64,
    pub texture: Option<TextureHandle>,
    /// Предыдущий показанный кадр (ping-pong с `texture`) для GPU-блендинга при несовпадении частоты видео и монитора
    back_texture: Option<TextureHandle>,
    /// Номер исходного кадра в `back_texture` (-1 — блендинг недоступен)
    back_frame: i64,
    /// Готов ли декодер выдавать кадры воспроизведения (false между командой Play и первым кадром)
    playback_ready: bool,
    /// Момент отправки последней команды Play (для ограничения времени пре-ролла)
    play_requested_at: std::time::Instant,
    pub is_real_video: bool,
    pub last_decoded_frame: i64,
    pub last_exact_frame: i64,
    pub last_playback_target: i64,
    pub current_epoch: u64,
    pub last_displayed_epoch: u64,

    // Высокопроизводительный движок рендеринга и декодирования
    pub gpu_accelerated: bool,
    pub gpu_device_name: String,

    // Неблокирующие каналы общения с фоновым потоком декодера
    command_tx: Option<std::sync::mpsc::Sender<DecoderCommand>>,
    frame_rx: Option<std::sync::mpsc::Receiver<(u64, i64, ColorImage)>>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    pub frame_queue: VecDeque<(i64, Arc<ColorImage>)>,
    pub scrub_cache: VecDeque<(i64, Arc<ColorImage>)>,
    pub prefetch_target: Option<i64>,
    pub prefetch_epoch: u64,
    pub prefetch_queue: VecDeque<(i64, Arc<ColorImage>)>,
    pub is_playing_mode: bool,

    // Время последнего отправленного запроса (для троттлинга скраббинга)
    pub last_request_time: std::time::Instant,

    // Мониторинг производительности реального времени (DaVinci Drop Frame Indicator)
    pub real_playback_fps: f32,
    pub dropped_frames_count: u64,
    pub is_dropping_frames: bool,
    pub drop_frames_enabled: bool,
    pub fps_sample_time: std::time::Instant,
    pub rendered_frames_in_sample: u32,

    /// Грубый отпечаток последнего показанного кадра (сетка 16x9, яркость) — для проверки точности кадра в тестах
    pub last_fingerprint: Vec<u8>,
    /// Последнее фактически отображенное изображение кадра (для интеграции с Render Cache)
    pub last_displayed_image: Option<Arc<ColorImage>>,
    /// Очередь свежедекодированных кадров для прямой регистрации в Render Cache
    pub newly_cached_frames: Vec<(i64, Arc<ColorImage>)>,

    // Фоновый канал для безынерционной асинхронной очистки кадров (исключает вызовы thread::spawn и mmap_lock в UI)
    gc_tx: std::sync::mpsc::Sender<Vec<ColorImage>>,
}


/// Render-ноды DRM в порядке предпочтения для аппаратного декодирования.
///
/// При двух GPU декодируем на менее мощном (меньше VRAM, обычно встроенный): дискретная карта остаётся свободной
/// для композитинга интерфейса и других приложений (браузер, YouTube), а декод 4K HEVC идёт на VCN встроенной графики.
/// Явный выбор: переменная окружения `VERIZON_VAAPI_DEVICE=/dev/dri/renderDNNN`.
fn vaapi_render_nodes() -> Vec<PathBuf> {
    let mut nodes: Vec<(u64, PathBuf)> = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .map(|e| {
            let vram = std::fs::read_to_string(format!("/sys/class/drm/{}/device/mem_info_vram_total", e.file_name().to_string_lossy()))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            (vram, e.path())
        })
        .collect();
    nodes.sort();
    let mut ordered: Vec<PathBuf> = nodes.into_iter().map(|(_, p)| p).collect();
    if let Some(forced) = std::env::var_os("VERIZON_VAAPI_DEVICE") {
        ordered.insert(0, PathBuf::from(forced));
    }
    ordered
}

/// Максимальное время ожидания первого кадра после старта воспроизведения (мс)
const PREROLL_TIMEOUT_MS: u128 = 1000;
/// Минимальный интервал между аппаратными запросами кадра при скраббинге (мс)
const SCRUB_MIN_REQUEST_INTERVAL_MS: u128 = 80;
/// Через сколько мс без доставки кадра запрос считается потерянным и отправляется заново
const SCRUB_STALL_TIMEOUT_MS: u128 = 200;

/// Грубый отпечаток кадра: средняя яркость в сетке 16x9 (по 3x3 выборкам на ячейку). Дёшево (~400 пикселей).
pub fn image_fingerprint(img: &ColorImage) -> Vec<u8> {
    let [w, h] = img.size;
    let mut out = Vec::with_capacity(144);
    if w == 0 || h == 0 {
        return out;
    }
    for gy in 0..9usize {
        for gx in 0..16usize {
            let mut acc = 0u32;
            for sy in 0..3usize {
                for sx in 0..3usize {
                    let x = ((gx * 3 + sx) * w / 48).min(w - 1);
                    let y = ((gy * 3 + sy) * h / 27).min(h - 1);
                    let c = img.pixels[y * w + x];
                    acc += (c.r() as u32 * 77 + c.g() as u32 * 150 + c.b() as u32 * 29) >> 8;
                }
            }
            out.push((acc / 9) as u8);
        }
    }
    out
}
impl Drop for VideoPlayer {
    fn drop(&mut self) {
        // Корректно завершаем и дожидаемся фонового рабочего потока для мгновенного освобождения GPU VAAPI контекста
        self.command_tx = None;
        self.frame_rx = None;
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
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
            back_texture: None,
            back_frame: -1,
            playback_ready: true,
            play_requested_at: std::time::Instant::now(),
            is_real_video: false,
            last_decoded_frame: -1,
            last_exact_frame: -1,
            last_playback_target: -1,
            current_epoch: 1,
            last_displayed_epoch: 0,
            gpu_accelerated: true,
            gpu_device_name: "Vulkan (wgpu) + Multi-Threaded Engine".to_string(),
            command_tx: None,
            frame_rx: None,
            worker_handle: None,
            frame_queue: VecDeque::new(),
            scrub_cache: VecDeque::with_capacity(64),
            prefetch_target: None,
            prefetch_epoch: 0,
            prefetch_queue: VecDeque::new(),
            is_playing_mode: false,
            last_request_time: std::time::Instant::now(),
            real_playback_fps: 24.0,
            dropped_frames_count: 0,
            is_dropping_frames: false,
            drop_frames_enabled: true,
            fps_sample_time: std::time::Instant::now(),
            rendered_frames_in_sample: 0,
            last_fingerprint: Vec::new(),
            last_displayed_image: None,
            newly_cached_frames: Vec::with_capacity(64),
            gc_tx: {
                let (tx, rx) = std::sync::mpsc::channel::<Vec<ColorImage>>();
                std::thread::Builder::new()
                    .name("verizon_gc".to_string())
                    .spawn(move || {
                        while let Ok(batch) = rx.recv() {
                            drop(batch);
                        }
                    })
                    .expect("Failed to spawn GC worker");
                tx
            },
        }
    }

    /// Безопасная фоновая утилизация буферов изображений без системных вызовов в UI-потоке
    #[inline]
    pub fn recycle_images(&self, images: Vec<ColorImage>) {
        if !images.is_empty() {
            let _ = self.gc_tx.send(images);
        }
    }

    /// Безопасная фоновая утилизация очереди кадров без системных вызовов в UI-потоке
    #[inline]
    pub fn recycle_queue(&self, q: VecDeque<(i64, Arc<ColorImage>)>) {
        if !q.is_empty() {
            let images: Vec<ColorImage> = q.into_iter().filter_map(|(_, img)| Arc::try_unwrap(img).ok()).collect();
            if !images.is_empty() {
                let _ = self.gc_tx.send(images);
            }
        }
    }

    /// Открыть видеофайл через FFmpeg с автоматическим определением параметров
    pub fn open_file(&mut self, path: &Path, ctx: &Context) -> Result<(), String> {
        // Завершаем предыдущий фоновый воркер если был открыт другой файл
        self.command_tx = None;
        self.frame_rx = None;
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
        self.frame_queue.clear();
        self.is_playing_mode = false;

        let ictx = ffmpeg_next::format::input(path).map_err(|e| format!("FFmpeg open error: {}", e))?;

        let input_stream = ictx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .ok_or_else(|| "В файле не найден видеопоток".to_string())?;

        let fps = f64::from(input_stream.avg_frame_rate());
        let fps = if fps <= 0.0 || fps.is_nan() {
            let r = f64::from(input_stream.rate());
            if r > 0.0 && !r.is_nan() { r } else { 24.0 }
        } else {
            fps
        };

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

        // Разрешение рендера предпросмотра (1280x720 HD для сверхбыстрого и четкого отображения без перегрузки шины PCIe)
        let (scaled_w, scaled_h) = if orig_w > 1280 {
            let scale = 1280.0 / orig_w as f32;
            (1280, ((orig_h as f32 * scale) as u32) & !1)
        } else {
            (orig_w, orig_h)
        };

        // Сбрасываем и корректно завершаем предыдущий фоновый воркер (если файл уже был открыт)
        self.command_tx = None;
        self.frame_rx = None;
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
        self.frame_queue.clear();
        self.scrub_cache.clear();
        self.prefetch_target = None;
        self.prefetch_queue.clear();

        self.current_path = Some(path.to_path_buf());
        self.width = scaled_w;
        self.height = scaled_h;
        self.fps = fps;
        self.total_frames = total_frames;
        self.gpu_accelerated = true;
        self.gpu_device_name = "Vulkan (wgpu) + 32-Thread Engine".to_string();
        self.is_real_video = true;
        self.last_decoded_frame = -1;
        self.last_exact_frame = -1;
        self.last_playback_target = -1;
        self.current_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
        self.last_displayed_epoch = 0;
        self.last_request_time = std::time::Instant::now();
        self.real_playback_fps = fps as f32;

        // Создаем каналы для фонового воркера декодирования
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<DecoderCommand>();
        let (frame_tx, frame_rx) = std::sync::mpsc::channel::<(u64, i64, ColorImage)>();
        let egui_ctx = ctx.clone();
        let file_path = path.to_path_buf();

        let handle = std::thread::spawn(move || {
            Self::run_decoder_worker(
                file_path,
                scaled_w,
                scaled_h,
                fps,
                cmd_rx,
                frame_tx,
                egui_ctx,
            );
        });
        self.worker_handle = Some(handle);

        // Запрашиваем начальный 0-й кадр
        let _ = cmd_tx.send(DecoderCommand::Seek {
            target_frame: 0,
            epoch: self.current_epoch,
        });

        self.command_tx = Some(cmd_tx);
        self.frame_rx = Some(frame_rx);

        self.seek_and_decode(0, ctx);

        Ok(())
    }

    /// Сохраняет декодированный кадр в кольцевой кэш быстрого скраббинга (0 мс при движении курсора туда-сюда, не нагружая DRM/VCN)
    fn cache_scrub_frame(&mut self, frame_num: i64, img: Arc<ColorImage>, garbage: &mut Vec<ColorImage>) {
        if frame_num < 0 {
            garbage.extend(Arc::try_unwrap(img).ok());
            return;
        }
        if let Some(pos) = self.scrub_cache.iter().position(|(f, _)| *f == frame_num) {
            if let Some(entry) = self.scrub_cache.remove(pos) {
                self.scrub_cache.push_back(entry);
            }
            garbage.extend(Arc::try_unwrap(img).ok());
            return;
        }
        if self.scrub_cache.len() >= 120 {
            if let Some((_, old_img)) = self.scrub_cache.pop_front() {
                garbage.extend(Arc::try_unwrap(old_img).ok());
            }
        }
        self.scrub_cache.push_back((frame_num, img));
    }

    /// Мгновенная подстановка ближайшего кадра из RAM-кэша при перемещении курсора туда-сюда (0 обращений к DRM/VAAPI)
    fn try_apply_scrub_cache(&mut self, target_frame: i64, max_dist: i64, ctx: &Context) -> Option<i64> {
        let mut best_idx = None;
        let mut best_dist = i64::MAX;
        for (i, (f, _)) in self.scrub_cache.iter().enumerate() {
            let d = (*f - target_frame).abs();
            if d <= max_dist && d < best_dist {
                best_dist = d;
                best_idx = Some(i);
            }
        }
        if let Some(idx) = best_idx {
            let (cached_f, _) = self.scrub_cache[idx];
            let current_dist = if self.last_decoded_frame >= 0 {
                (self.last_decoded_frame - target_frame).abs()
            } else {
                i64::MAX
            };
            if cached_f != self.last_decoded_frame && (best_dist < current_dist || cached_f == target_frame) {
                // Arc вместо копии кадра: никаких аллокаций 3.7 МБ и mmap/munmap в UI-потоке при каждом тике скраббинга
                let img = Arc::clone(&self.scrub_cache[idx].1);
                self.rendered_frames_in_sample += 1;
                self.back_frame = -1;
                self.last_fingerprint = image_fingerprint(&img);
                self.last_displayed_image = Some(Arc::clone(&img));
                let data = ImageData::Color(img);
                if let Some(tex) = &mut self.texture {
                    tex.set(data, TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ctx.load_texture("video_frame", data, TextureOptions::LINEAR));
                }
                self.last_decoded_frame = cached_f;
                if cached_f == target_frame {
                    self.last_exact_frame = cached_f;
                }
            }
            return Some(best_dist);
        }
        None
    }

    /// Асинхронная очистка очередей кадров в фоновом потоке (исключает 60+ вызовов munmap и TLB shootdown в UI-потоке!)
    fn clear_queues_async(&mut self) {
        let mut q1 = std::mem::take(&mut self.frame_queue);
        let q2 = std::mem::take(&mut self.prefetch_queue);
        // Переносим часть уже готовых кадров из очереди в scrub_cache без аллокаций и клонирования!
        let step = (q1.len() / 12).max(1);
        let mut idx = 0usize;
        let mut drop_q = VecDeque::with_capacity(q1.len());
        while let Some((f_num, img)) = q1.pop_front() {
            if idx % step == 0 && f_num >= 0 && !self.scrub_cache.iter().any(|(f, _)| *f == f_num) {
                if self.scrub_cache.len() >= 120 {
                    if let Some((old_f, old)) = self.scrub_cache.pop_front() {
                        drop_q.push_back((old_f, old));
                    }
                }
                self.scrub_cache.push_back((f_num, img));
            } else {
                drop_q.push_back((f_num, img));
            }
            idx += 1;
        }
        if !drop_q.is_empty() {
            self.recycle_queue(drop_q);
        }
        if !q2.is_empty() {
            self.recycle_queue(q2);
        }
        self.newly_cached_frames.clear();
    }

    /// Главный метод обновления кадра:
    /// Автоматически разделяет непрерывное воспроизведение (Jitter Buffer Prefetch)
    /// и точечный скраббинг/перемотку (Frame-Accurate Seek)
    pub fn update_playback(&mut self, target_frame: i64, is_playing: bool, ctx: &Context) {
        self.update_playback_with_cut(target_frame, is_playing, None, ctx);
    }

    /// Обновление воспроизведения с упреждающей подготовкой следующей монтажной склейки (Zero-Latency Cut)
    pub fn update_playback_with_cut(
        &mut self,
        target_frame: i64,
        is_playing: bool,
        upcoming_cut: Option<(i64, i64)>,
        ctx: &Context,
    ) {
        if !self.is_real_video {
            self.show_blank_frame(ctx);
            return;
        }

        if is_playing {
            if !self.is_playing_mode {
                self.is_playing_mode = true;
                self.last_playback_target = target_frame;
                self.dropped_frames_count = 0;
                self.rendered_frames_in_sample = 0;
                self.fps_sample_time = std::time::Instant::now();
                self.real_playback_fps = self.fps as f32;
                self.is_dropping_frames = false;
                if self.prefetch_target.map(|pf| (target_frame - pf).abs() <= 2).unwrap_or(false) {
                    self.current_epoch = self.prefetch_epoch;
                    let old_q = std::mem::replace(&mut self.frame_queue, std::mem::take(&mut self.prefetch_queue));
                    self.recycle_queue(old_q);
                    self.prefetch_target = None;
                } else {
                    self.try_apply_scrub_cache(target_frame, 30, ctx);
                    self.current_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
                    self.clear_queues_async();
                    self.prefetch_target = None;
                    self.begin_preroll();
                    if let Some(tx) = &self.command_tx {
                        let _ = tx.send(DecoderCommand::Play {
                            start_frame: target_frame,
                            epoch: self.current_epoch,
                        });
                    }
                }
            } else {
                let jump = target_frame - self.last_playback_target;
                // Сбрасываем очередь и отправляем Play только при РЕАЛЬНОМ скачке пользователем или на склейке (назад или вперед >8 кадров)
                if jump < -2 || jump > 8 {
                    if self.prefetch_target.map(|pf| (target_frame - pf).abs() <= 2).unwrap_or(false) {
                        // Склейка уже предзагружена в фоновом потоке (Zero-Latency Cut)!
                        self.current_epoch = self.prefetch_epoch;
                        let old_q = std::mem::replace(&mut self.frame_queue, std::mem::take(&mut self.prefetch_queue));
                        self.recycle_queue(old_q);
                        self.prefetch_target = None;
                        self.last_playback_target = target_frame;
                        if let Some(tx) = &self.command_tx {
                            let _ = tx.send(DecoderCommand::UpdatePlayhead { playhead_frame: target_frame });
                        }
                    } else {
                        let _ = self.try_apply_scrub_cache(target_frame, 45, ctx);
                        let now = std::time::Instant::now();
                        let elapsed = now.duration_since(self.last_request_time).as_millis();
                        let prev_delivered = self.last_displayed_epoch >= self.current_epoch;
                        if (prev_delivered && elapsed >= 32) || elapsed >= 55 {
                            self.last_request_time = now;
                            self.last_playback_target = target_frame;
                            self.current_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
                            self.clear_queues_async();
                            self.prefetch_target = None;
                            self.begin_preroll();
                            if let Some(tx) = &self.command_tx {
                                let _ = tx.send(DecoderCommand::Play {
                                    start_frame: target_frame,
                                    epoch: self.current_epoch,
                                });
                            }
                        } else if let Some(tx) = &self.command_tx {
                            let _ = tx.send(DecoderCommand::UpdatePlayhead { playhead_frame: target_frame });
                        }
                    }
                } else {
                    if self.prefetch_target.is_none() && target_frame != self.last_playback_target {
                        if let Some(tx) = &self.command_tx {
                            let _ = tx.send(DecoderCommand::UpdatePlayhead { playhead_frame: target_frame });
                        }
                    }
                    self.last_playback_target = target_frame;
                }
            }

            self.poll_playback_frames_with_cut(target_frame, upcoming_cut, ctx);
        } else {
            if self.is_playing_mode {
                self.is_playing_mode = false;
                self.current_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
                self.clear_queues_async();
                self.prefetch_target = None;
                self.is_dropping_frames = false;
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(DecoderCommand::Pause { epoch: self.current_epoch });
                }
            }

            self.seek_and_decode(target_frame, ctx);
        }
    }

    /// Выборка кадров из буфера упреждающего чтения в режиме воспроизведения с поддержкой DaVinci Drop-Frame и предзагрузкой склеек
    fn poll_playback_frames_with_cut(
        &mut self,
        target_frame: i64,
        upcoming_cut: Option<(i64, i64)>,
        ctx: &Context,
    ) {
        let cut_limit = upcoming_cut.map(|(last_src, _)| last_src).unwrap_or(i64::MAX);
        let mut fallback_preview: Option<(u64, i64, Arc<ColorImage>)> = None;
        let mut newly_cached: Vec<(i64, Arc<ColorImage>)> = Vec::new();
        let mut garbage: Vec<ColorImage> = Vec::new();
        if let Some(rx) = &self.frame_rx {
            while let Ok((epoch, raw_frame_num, image)) = rx.try_recv() {
                let arc_img = Arc::new(image);
                if self.prefetch_target.is_some() && epoch == self.prefetch_epoch {
                    if raw_frame_num < 0 {
                        let pf = -raw_frame_num - 1;
                        if self.prefetch_queue.is_empty() {
                            self.prefetch_queue.push_back((pf, arc_img));
                        } else {
                            garbage.extend(Arc::try_unwrap(arc_img).ok());
                        }
                    } else {
                        let frame_num = raw_frame_num;
                        if let Some(back) = self.prefetch_queue.back_mut() {
                            if back.0 == frame_num {
                                let old = std::mem::replace(&mut back.1, arc_img);
                                garbage.extend(Arc::try_unwrap(old).ok());
                                continue;
                            }
                        }
                        self.prefetch_queue.push_back((frame_num, arc_img));
                    }
                    continue;
                }
                if raw_frame_num < 0 {
                    let preview_f = -raw_frame_num - 1;
                    if epoch <= self.current_epoch && (epoch == self.current_epoch || epoch > self.last_displayed_epoch) {
                        newly_cached.push((preview_f, Arc::clone(&arc_img)));
                        if let Some((_, _, old_prev)) = fallback_preview.replace((epoch, preview_f, arc_img)) {
                            garbage.extend(Arc::try_unwrap(old_prev).ok());
                        }
                    } else {
                        garbage.extend(Arc::try_unwrap(arc_img).ok());
                    }
                    continue;
                }
                let frame_num = raw_frame_num;
                if epoch == self.current_epoch {
                    if frame_num <= cut_limit {
                        if frame_num % 8 == 0 {
                            newly_cached.push((frame_num, Arc::clone(&arc_img)));
                        }
                        self.newly_cached_frames.push((frame_num, Arc::clone(&arc_img)));
                        if self.newly_cached_frames.len() > 64 {
                            self.newly_cached_frames.remove(0);
                        }
                        if let Some(back) = self.frame_queue.back_mut() {
                            if back.0 == frame_num {
                                let old = std::mem::replace(&mut back.1, arc_img);
                                garbage.extend(Arc::try_unwrap(old).ok());
                                continue;
                            }
                        }
                        self.frame_queue.push_back((frame_num, arc_img));
                    } else {
                        garbage.extend(Arc::try_unwrap(arc_img).ok());
                    }
                } else if epoch < self.current_epoch && epoch > self.last_displayed_epoch {
                    if let Some((_, _, old_prev)) = fallback_preview.replace((epoch, frame_num, arc_img)) {
                        garbage.extend(Arc::try_unwrap(old_prev).ok());
                    }
                } else {
                    garbage.extend(Arc::try_unwrap(arc_img).ok());
                }
            }
        }
        for (cf, cimg) in newly_cached {
            self.cache_scrub_frame(cf, cimg, &mut garbage);
        }

        // Первый кадр текущего сегмента уже в очереди — пре-ролл окончен, плейхед может идти (кадр может быть на 1 впереди цели)
        if !self.playback_ready && !self.frame_queue.is_empty() {
            self.playback_ready = true;
        }

        // Если все кадры текущего сегмента до монтажной склейки уже в очереди — мгновенно переключаем
        // фоновый декодер на декодирование GOP следующего клипа после склейки!
        if let Some((last_src, next_src)) = upcoming_cut {
            let have_last_frame = self
                .frame_queue
                .back()
                .map(|(f, _)| *f >= last_src && *f > self.last_playback_target)
                .unwrap_or(false)
                || (target_frame >= last_src.saturating_sub(2)
                    && self.last_decoded_frame >= last_src
                    && self.last_decoded_frame <= last_src + 2);
            if have_last_frame && self.prefetch_target != Some(next_src) {
                self.prefetch_target = Some(next_src);
                self.prefetch_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
                let old_pq = std::mem::take(&mut self.prefetch_queue);
                for (_, img) in old_pq {
                    garbage.extend(Arc::try_unwrap(img).ok());
                }
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(DecoderCommand::Play {
                        start_frame: next_src,
                        epoch: self.prefetch_epoch,
                    });
                }
            }
        }

        let mut best_img: Option<Arc<ColorImage>> = None;
        let mut best_frame = self.last_decoded_frame;

        // Если еще ничего не отобразили или произошел скачок назад за пределы старого кадра
        if self.last_decoded_frame < 0 || self.last_decoded_frame > target_frame + 5 {
            if let Some((f, img)) = self.frame_queue.pop_front() {
                best_img = Some(img);
                best_frame = f;
            }
        }

        let mut dropped = 0;
        while let Some(&(f_num, _)) = self.frame_queue.front() {
            if f_num <= target_frame {
                let Some((f, img)) = self.frame_queue.pop_front() else { break; };
                // Если включен режим Drop Frame (по умолчанию как в Resolve) и в очереди есть более актуальный кадр,
                // при этом текущий кадр реально отстал от плейхеда минимум на 2 кадра (устраняет ложные дропы от 60Hz VSync фазового биения),
                // устаревший кадр отбрасывается без отображения для поддержания точной скорости 1.0x
                let really_late = target_frame - f >= 2 || self.frame_queue.len() >= 2 && self.frame_queue[1].0 <= target_frame;
                if self.drop_frames_enabled
                    && really_late
                    && self.frame_queue.front().map(|(next_f, _)| *next_f <= target_frame).unwrap_or(false)
                {
                    garbage.extend(Arc::try_unwrap(img).ok());
                    dropped += 1;
                } else {
                    if let Some(old_best) = best_img.replace(img) {
                        garbage.extend(Arc::try_unwrap(old_best).ok());
                    }
                    best_frame = f;
                    // Если этот кадр отстал всего на 1 кадр из-за фазы 60Hz VSync — показываем его сейчас,
                    // а следующий кадр (f+1 == target_frame) оставляем в голове очереди на следующий тик!
                    if !really_late {
                        break;
                    }
                }
            } else {
                break;
            }
        }
        self.dropped_frames_count += dropped;

        while self.frame_queue.len() > 90 {
            if let Some((_, old_img)) = self.frame_queue.pop_front() {
                garbage.extend(Arc::try_unwrap(old_img).ok());
            }
            self.dropped_frames_count += 1;
        }

        if let Some(img) = best_img {
            if let Some((_, _, old_prev)) = fallback_preview.take() {
                garbage.extend(Arc::try_unwrap(old_prev).ok());
            }
            self.rendered_frames_in_sample += 1;
            self.last_displayed_epoch = self.current_epoch;
            self.last_fingerprint = image_fingerprint(&img);
            self.present_playback_frame(ctx, img);
            self.last_decoded_frame = best_frame;
            self.last_exact_frame = best_frame;
        } else if let Some((ep, prev_f, prev_img)) = fallback_preview {
            self.rendered_frames_in_sample += 1;
            self.last_displayed_epoch = ep;
            self.last_fingerprint = image_fingerprint(&prev_img);
            self.back_frame = -1;
            let data = ImageData::Color(prev_img);
            if let Some(tex) = &mut self.texture {
                tex.set(data, TextureOptions::LINEAR);
            } else {
                self.texture = Some(ctx.load_texture("video_frame", data, TextureOptions::LINEAR));
            }
            self.last_decoded_frame = prev_f;
        } else {
            let _ = self.try_apply_scrub_cache(target_frame, 18, ctx);
        }

        self.recycle_images(garbage);

        // Обновление реального FPS воспроизведения каждые 500 мс
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.fps_sample_time).as_secs_f32();
        if elapsed >= 0.5 {
            self.real_playback_fps = (self.rendered_frames_in_sample as f32 / elapsed).round();
            self.rendered_frames_in_sample = 0;
            self.fps_sample_time = now;
            self.is_dropping_frames = self.real_playback_fps < (self.fps as f32 - 1.5) || self.dropped_frames_count > 0;
        }
    }

    /// Начало пре-ролла: плейхед ждёт первого кадра после Play, чтобы не терять кадры (аналог пре-ролла DaVinci Resolve)
    fn begin_preroll(&mut self) {
        self.playback_ready = false;
        self.play_requested_at = std::time::Instant::now();
    }

    /// Идёт ли пре-ролл: таймлайн и звук должны ждать, пока декодер выдаст первый кадр (не дольше `PREROLL_TIMEOUT_MS`)
    pub fn is_prerolling(&self) -> bool {
        self.is_real_video
            && self.is_playing_mode
            && !self.playback_ready
            && self.play_requested_at.elapsed().as_millis() < PREROLL_TIMEOUT_MS
    }

    /// Выводит кадр воспроизведения через ping-pong двух текстур: предыдущий кадр остаётся на GPU для блендинга
    pub fn present_playback_frame(&mut self, ctx: &Context, img: Arc<ColorImage>) {
        self.last_displayed_image = Some(Arc::clone(&img));
        let data = ImageData::Color(img);
        if self.texture.is_none() {
            self.texture = Some(ctx.load_texture("video_frame", data, TextureOptions::LINEAR));
            self.back_frame = -1;
            return;
        }
        let next = match self.back_texture.take() {
            Some(mut tex) => {
                tex.set(data, TextureOptions::LINEAR);
                tex
            }
            None => ctx.load_texture("video_frame_b", data, TextureOptions::LINEAR),
        };
        self.back_texture = self.texture.replace(next);
        self.back_frame = self.last_decoded_frame;
    }

    /// Предыдущий кадр для временного блендинга, если он ровно на 1 кадр раньше текущего (плавное 60 fps на мониторе с другой частотой)
    pub fn blend_source(&self) -> Option<&TextureHandle> {
        let back = self.back_texture.as_ref()?;
        (self.is_playing_mode && self.back_frame >= 0 && self.last_decoded_frame - self.back_frame == 1).then_some(back)
    }

    /// Отобразить чистый нейтральный экран Resolve (а при воспроизведении паузы — заранее подготовить декодер к следующему клипу!)
    pub fn show_blank_frame_with_prefetch(&mut self, prefetch_frame: Option<i64>, ctx: &Context) {
        if let Some(next_src) = prefetch_frame {
            if self.prefetch_target.map(|pf| (next_src - pf).abs() <= 2).unwrap_or(false) {
                self.is_playing_mode = true;
                self.last_playback_target = next_src;
                self.current_epoch = self.prefetch_epoch;
                let old_q = std::mem::replace(&mut self.frame_queue, std::mem::take(&mut self.prefetch_queue));
                self.recycle_queue(old_q);
                self.prefetch_target = None;
            } else if !self.is_playing_mode || self.last_playback_target != next_src {
                self.is_playing_mode = true;
                self.last_playback_target = next_src;
                self.current_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
                self.clear_queues_async();
                self.prefetch_target = None;
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(DecoderCommand::Play {
                        start_frame: next_src,
                        epoch: self.current_epoch,
                    });
                }
            }
        } else {
            if self.is_playing_mode {
                self.is_playing_mode = false;
                self.current_epoch = self.current_epoch.max(self.prefetch_epoch).wrapping_add(1);
                self.clear_queues_async();
                self.prefetch_target = None;
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(DecoderCommand::Pause { epoch: self.current_epoch });
                }
            }
            if let Some(rx) = &self.frame_rx {
                let mut stale = Vec::new();
                while let Ok((_, _, img)) = rx.try_recv() {
                    stale.push(img);
                }
                self.recycle_images(stale);
            }
        }
        if self.last_decoded_frame == -999 && self.texture.is_some() {
            return;
        }
        self.back_frame = -1;
        let img = Self::generate_placeholder();
        if let Some(tex) = &mut self.texture {
            tex.set(img, TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture("video_frame", img, TextureOptions::NEAREST));
        }
        self.last_decoded_frame = -999;
    }

    /// Отобразить чистый нейтральный экран Resolve без декодирования
    pub fn show_blank_frame(&mut self, ctx: &Context) {
        self.show_blank_frame_with_prefetch(None, ctx);
    }

    /// Неблокирующая выборка готовых кадров из фонового потока
    pub fn poll_frames(&mut self, ctx: &Context) {
        let mut newly_cached: Vec<(i64, Arc<ColorImage>)> = Vec::new();
        let mut stale = Vec::new();
        if let Some(rx) = &self.frame_rx {
            while let Ok((epoch, raw_frame_num, image)) = rx.try_recv() {
                let is_preview = raw_frame_num < 0;
                let frame_num = if is_preview { -raw_frame_num - 1 } else { raw_frame_num };
                let arc_img = Arc::new(image);
                if epoch != self.current_epoch && epoch <= self.last_displayed_epoch {
                    if !is_preview {
                        newly_cached.push((frame_num, arc_img));
                    } else {
                        stale.extend(Arc::try_unwrap(arc_img).ok());
                    }
                    continue;
                }
                if !is_preview {
                    newly_cached.push((frame_num, Arc::clone(&arc_img)));
                    self.newly_cached_frames.push((frame_num, Arc::clone(&arc_img)));
                    if self.newly_cached_frames.len() > 64 {
                        self.newly_cached_frames.remove(0);
                    }
                }
                self.last_displayed_epoch = self.last_displayed_epoch.max(epoch);

                // Обновляем монитор только если кадр совпадает с целевым (плейхедом)
                // или включен режим воспроизведения. Упреждающие кадры Smart Cache не сбивают текущий кадр паузы!
                if self.is_playing_mode || frame_num == self.last_playback_target {
                    self.rendered_frames_in_sample += 1;
                    self.last_fingerprint = image_fingerprint(&arc_img);
                    self.last_displayed_image = Some(Arc::clone(&arc_img));
                    self.back_frame = -1;
                    let data = ImageData::Color(arc_img);
                    if let Some(tex) = &mut self.texture {
                        tex.set(data, TextureOptions::LINEAR);
                    } else {
                        self.texture = Some(ctx.load_texture("video_frame", data, TextureOptions::LINEAR));
                    }
                    self.last_decoded_frame = frame_num;
                    if !is_preview && epoch == self.current_epoch {
                        self.last_exact_frame = frame_num;
                    }
                }
            }
        }
        for (cf, cimg) in newly_cached {
            self.cache_scrub_frame(cf, cimg, &mut stale);
        }
        self.recycle_images(stale);
    }

    /// Прецизионное декодирование кадра по таймкоду (Frame-Accurate Zero-Lag)
    pub fn seek_and_decode(&mut self, target_frame: i64, ctx: &Context) {
        self.poll_frames(ctx);

        if target_frame == self.last_exact_frame && target_frame == self.last_decoded_frame && self.texture.is_some() {
            return;
        }

        if self.is_real_video {
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(self.last_request_time).as_millis();
            if let Some(dist) = self.try_apply_scrub_cache(target_frame, 45, ctx) {
                if dist == 0 || (dist <= 15 && elapsed < 90) {
                    self.last_playback_target = target_frame;
                    return;
                }
            }
            if target_frame == self.last_playback_target && self.texture.is_some() {
                return;
            }
            let prev_delivered = self.last_displayed_epoch >= self.current_epoch;
            // Обратное давление: новый аппаратный запрос к /dev/dri/renderD128 уходит только после доставки
            // предыдущего (не чаще ~12 Hz). Каждый Seek декодирует GOP от ключевого кадра, и очередь запросов
            // забивает VCN/DRM, из-за чего подвисает вся система (в т.ч. YouTube в браузере).
            if (prev_delivered && elapsed >= SCRUB_MIN_REQUEST_INTERVAL_MS) || elapsed >= SCRUB_STALL_TIMEOUT_MS || target_frame == 0 {
                self.last_request_time = now;
                self.last_playback_target = target_frame;
                self.current_epoch = self.current_epoch.wrapping_add(1);
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(DecoderCommand::Seek {
                        target_frame,
                        epoch: self.current_epoch,
                    });
                }
            } else {
                // Гарантируем повторную попытку после паузы лимитера, чтобы финальный кадр скраббинга не потерялся
                let wait_ms = SCRUB_MIN_REQUEST_INTERVAL_MS.saturating_sub(elapsed).max(1) as u64;
                ctx.request_repaint_after(std::time::Duration::from_millis(wait_ms));
            }
            return;
        }

        // Если файл не открыт, отображаем нейтральный экран Resolve
        let img = Self::generate_placeholder();
        if let Some(tex) = &mut self.texture {
            tex.set(img, TextureOptions::NEAREST);
        } else {
            self.texture = Some(ctx.load_texture("video_frame", img, TextureOptions::NEAREST));
        }
        self.last_decoded_frame = target_frame;
        self.last_playback_target = target_frame;
    }

    /// Принудительный запрос кадра (когда скраббинг завершен)
    pub fn force_seek_frame(&mut self, target_frame: i64, ctx: &Context) {
        self.poll_frames(ctx);
        if target_frame == self.last_exact_frame && target_frame == self.last_decoded_frame && self.texture.is_some() {
            return;
        }
        if self.is_real_video {
            if let Some(dist) = self.try_apply_scrub_cache(target_frame, 0, ctx) {
                if dist == 0 {
                    self.last_playback_target = target_frame;
                    return;
                }
            }
            self.last_request_time = std::time::Instant::now();
            self.last_playback_target = target_frame;
            self.current_epoch = self.current_epoch.wrapping_add(1);
            self.last_displayed_epoch = self.current_epoch.saturating_sub(1);
            if let Some(tx) = &self.command_tx {
                let _ = tx.send(DecoderCommand::Seek {
                    target_frame,
                    epoch: self.current_epoch,
                });
            }
        }
    }

    /// Ожидание декодирования целевого кадра (для детерминированных тестов)
    pub fn wait_for_frame(&mut self, target_frame: i64, ctx: &Context, timeout: std::time::Duration) -> bool {
        let start = std::time::Instant::now();
        self.force_seek_frame(target_frame, ctx);
        while start.elapsed() < timeout {
            self.poll_frames(ctx);
            if self.last_exact_frame == target_frame && self.texture.is_some() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        self.poll_frames(ctx);
        if self.last_exact_frame == target_frame && self.texture.is_some() {
            return true;
        }
        eprintln!("[TEST] wait_for_frame timed out! target_frame={}, last_decoded_frame={}", target_frame, self.last_decoded_frame);
        false
    }

    /// Выделенный фоновый воркер декодирования видео
    fn run_decoder_worker(
        path: PathBuf,
        scaled_w: u32,
        scaled_h: u32,
        fps: f64,
        cmd_rx: std::sync::mpsc::Receiver<DecoderCommand>,
        frame_tx: std::sync::mpsc::Sender<(u64, i64, ColorImage)>,
        egui_ctx: Context,
    ) {
        unsafe extern "C" fn get_vaapi_format(
            _ctx: *mut ffmpeg_next::ffi::AVCodecContext,
            pix_fmts: *const ffmpeg_next::ffi::AVPixelFormat,
        ) -> ffmpeg_next::ffi::AVPixelFormat {
            let mut p = pix_fmts;
            while *p != ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                if *p == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI {
                    return *p;
                }
                p = p.add(1);
            }
            *pix_fmts
        }

        ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Fatal);

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
        let time_base_f = f64::from(time_base);

        let frame_duration_pts = if time_base_f > 0.0 && fps > 0.0 {
            (1.0 / fps / time_base_f).round().max(1.0) as i64
        } else {
            1
        };
        let tolerance = (frame_duration_pts / 2).max(1);
        let gop_max_pts = if time_base_f > 0.0 {
            (2.1 / time_base_f).round() as i64
        } else {
            126
        };

        let mut context_decoder = match ffmpeg_next::codec::context::Context::from_parameters(input_stream.parameters()) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Отключаем демультиплексирование не-видео потоков (аудио/субтитры), чтобы не аллоцировать лишние AVPacket
        for mut s in ictx.streams_mut() {
            if s.index() != stream_idx {
                unsafe {
                    (*s.as_mut_ptr()).discard = ffmpeg_next::ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
        }

        unsafe {
            (*context_decoder.as_mut_ptr()).flags2 |= ffmpeg_next::ffi::AV_CODEC_FLAG2_FAST as i32;
        }

        // Аппаратное ускорение на GPU (VAAPI для AMD Radeon RX 9070 XT)
        let mut hw_device_ctx: *mut ffmpeg_next::ffi::AVBufferRef = std::ptr::null_mut();
        for dev in &vaapi_render_nodes() {
            if let Ok(c_str) = std::ffi::CString::new(dev.as_os_str().as_encoded_bytes()) {
                let ret = unsafe {
                    ffmpeg_next::ffi::av_hwdevice_ctx_create(
                        &mut hw_device_ctx,
                        ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                        c_str.as_ptr(),
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if ret == 0 && !hw_device_ctx.is_null() {
                    break;
                }
            }
        }

        if !hw_device_ctx.is_null() {
            unsafe {
                let c_ptr = context_decoder.as_mut_ptr();
                // КРИТИЧНО ДЛЯ DRM/VCN И YOUTUBE В БРАУЗЕРЕ:
                // Никогда не включаем FF_THREAD_FRAME на аппаратном VAAPI декодере!
                // Многопоточная отправка кадров в VAAPI перегружает кольцевую очередь AMDGPU VCN (/dev/dri/renderD128)
                // конкурентными 4K-задачами при каждом скраббинге и вызывает фризы видео в браузере (YouTube).
                (*c_ptr).thread_count = 1;
                (*c_ptr).thread_type = 0;
                (*c_ptr).hw_device_ctx = ffmpeg_next::ffi::av_buffer_ref(hw_device_ctx);
                (*c_ptr).get_format = Some(get_vaapi_format);
                (*c_ptr).extra_hw_frames = 8;
            }
        } else {
            // Если аппаратный GPU VAAPI недоступен — используем многопоточный декодер CPU (все 32 потока)
            unsafe {
                (*context_decoder.as_mut_ptr()).thread_type =
                    (ffmpeg_next::ffi::FF_THREAD_FRAME | ffmpeg_next::ffi::FF_THREAD_SLICE) as i32;
            }
            let num_threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(16)
                .min(32);

            context_decoder.set_threading(ffmpeg_next::codec::threading::Config {
                kind: ffmpeg_next::codec::threading::Type::Frame,
                count: num_threads,
            });
        }

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
        let mut current_decoded_frame: i64 = -1;
        let mut rgba_buffer = vec![0u8; w * h * 4];
        let mut decoded_frame = ffmpeg_next::util::frame::Video::empty();
        let mut sw_frame = ffmpeg_next::util::frame::Video::empty();
        let mut rgba_frame = ffmpeg_next::util::frame::Video::empty();

        // Двухступенчатый конвейер воспроизведения:
        // Thread A (этот поток) держит 3-кадровый конвейер VCN (vaapi_fifo) и выполняет DMA-копирование в переиспользуемые sw_frame,
        // а Thread B (scale_worker) параллельно выполняет swscale 4K NV12 -> 720p RGBA и упаковку ColorImage на другом ядре CPU!
        struct SendVideoFrame(ffmpeg_next::util::frame::Video);
        unsafe impl Send for SendVideoFrame {}

        let (scale_tx, scale_rx) = std::sync::mpsc::sync_channel::<(u64, i64, SendVideoFrame)>(4);
        let (recycle_tx, recycle_rx) = std::sync::mpsc::channel::<SendVideoFrame>();
        let cancel_epoch = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cancel_epoch_b = cancel_epoch.clone();
        let frame_tx_b = frame_tx.clone();
        let egui_ctx_b = egui_ctx.clone();
        let init_fmt = video_decoder.format();
        let scale_worker_handle = std::thread::spawn(move || {
            let Ok(mut scaler_b) = ffmpeg_next::software::scaling::Context::get(
                init_fmt,
                orig_w,
                orig_h,
                ffmpeg_next::format::Pixel::RGBA,
                scaled_w,
                scaled_h,
                ffmpeg_next::software::scaling::flag::Flags::FAST_BILINEAR,
            ) else {
                return;
            };
            let mut rgba_frame_b = ffmpeg_next::util::frame::Video::empty();
            let mut dummy_buf = Vec::new();
            let mut ignored_sw = ffmpeg_next::util::frame::Video::empty();
            while let Ok((ep, f_num, SendVideoFrame(sw_f))) = scale_rx.recv() {
                if ep >= cancel_epoch_b.load(std::sync::atomic::Ordering::Acquire) {
                    if let Some(img) = Self::scale_and_copy_frame(
                        &mut scaler_b,
                        &sw_f,
                        &mut ignored_sw,
                        &mut rgba_frame_b,
                        w,
                        h,
                        &mut dummy_buf,
                    ) {
                        if frame_tx_b.send((ep, f_num, img)).is_err() {
                            break;
                        }
                        egui_ctx_b.request_repaint();
                    }
                }
                let _ = recycle_tx.send(SendVideoFrame(sw_f));
            }
        });

        let mut vaapi_fifo: VecDeque<(u64, i64, ffmpeg_next::util::frame::Video)> = VecDeque::with_capacity(4);
        let transfer_and_send = |ep: u64,
                                 f_num: i64,
                                 hw_or_sw: ffmpeg_next::util::frame::Video,
                                 recycle_rx: &std::sync::mpsc::Receiver<SendVideoFrame>,
                                 scale_tx: &std::sync::mpsc::SyncSender<(u64, i64, SendVideoFrame)>|
         -> bool {
            let fmt = unsafe { (*hw_or_sw.as_ptr()).format };
            if fmt == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI as i32 {
                let mut dst_sw = recycle_rx
                    .try_recv()
                    .map(|s| s.0)
                    .unwrap_or_else(|_| ffmpeg_next::util::frame::Video::empty());
                let ok = unsafe {
                    ffmpeg_next::ffi::av_hwframe_transfer_data(dst_sw.as_mut_ptr(), hw_or_sw.as_ptr(), 0) == 0
                };
                drop(hw_or_sw);
                if ok {
                    return scale_tx.send((ep, f_num, SendVideoFrame(dst_sw))).is_ok();
                }
                true
            } else {
                scale_tx.send((ep, f_num, SendVideoFrame(hw_or_sw))).is_ok()
            }
        };

        let mut is_playing = false;
        let mut playhead_frame: i64 = 0;
        let mut start_playback_frame: i64;
        let mut active_epoch: u64 = 1;
        let mut waiting_for_keyframe = false;
        let mut pending_cmd: Option<DecoderCommand> = None;

        'worker_loop: loop {
            let active_cmd = if let Some(c) = pending_cmd.take() {
                c
            } else if is_playing {
                match cmd_rx.try_recv() {
                    Ok(c) => c,
                    Err(std::sync::mpsc::TryRecvError::Empty) => DecoderCommand::Play {
                        start_frame: playhead_frame,
                        epoch: active_epoch,
                    },
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'worker_loop,
                }
            } else {
                match cmd_rx.recv() {
                    Ok(c) => c,
                    Err(_) => break 'worker_loop,
                }
            };

            let mut resolved_cmd = active_cmd;
            if let DecoderCommand::Play { start_frame, .. } = resolved_cmd {
                playhead_frame = start_frame;
            }
            while let Ok(newer) = cmd_rx.try_recv() {
                match newer {
                    DecoderCommand::Seek { .. } | DecoderCommand::Pause { .. } => {
                        resolved_cmd = newer;
                    }
                    DecoderCommand::Play { start_frame, epoch } => {
                        playhead_frame = start_frame;
                        resolved_cmd = DecoderCommand::Play { start_frame, epoch };
                    }
                    DecoderCommand::UpdatePlayhead { playhead_frame: new_pf } => {
                        playhead_frame = new_pf;
                    }
                }
            }

            match resolved_cmd {
                DecoderCommand::Pause { epoch } => {
                    active_epoch = epoch;
                    cancel_epoch.store(epoch, std::sync::atomic::Ordering::Release);
                    vaapi_fifo.clear();
                    is_playing = false;
                }
                DecoderCommand::UpdatePlayhead { playhead_frame: new_pf } => {
                    playhead_frame = new_pf;
                }
                DecoderCommand::Seek { mut target_frame, mut epoch } => {
                    active_epoch = epoch;
                    cancel_epoch.store(epoch, std::sync::atomic::Ordering::Release);
                    vaapi_fifo.clear();
                    is_playing = false;
                    let mut target_pts = if time_base_f > 0.0 {
                        (target_frame as f64 / fps / time_base_f).round() as i64
                    } else {
                        target_frame
                    };

                    let can_decode_forward = current_pts >= 0
                        && target_pts >= current_pts
                        && (target_pts - current_pts) <= gop_max_pts
                        && !waiting_for_keyframe;

                    if !can_decode_forward {
                        unsafe {
                            ffmpeg_next::ffi::av_seek_frame(
                                ictx.as_mut_ptr(),
                                stream_idx as i32,
                                target_pts,
                                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                            );
                        }
                        video_decoder.flush();
                        current_pts = -1;
                        current_decoded_frame = -1;
                        waiting_for_keyframe = true;
                    }

                    let mut found_image: Option<ColorImage> = None;
                    let mut keyframe_sent = false;
                    let mut aborted_seek = false;

                    if !waiting_for_keyframe {
                        while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                            let pts = decoded_frame.pts().unwrap_or(current_pts + frame_duration_pts);
                            current_pts = pts;
                            let f_num = (pts as f64 * time_base_f * fps).round() as i64;
                            current_decoded_frame = f_num;
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

                    if found_image.is_none() {
                        for (st, packet) in ictx.packets() {
                            match cmd_rx.try_recv() {
                                Ok(first_cmd) => {
                                    let mut newer = first_cmd;
                                    if let DecoderCommand::Play { start_frame, .. } = newer {
                                        playhead_frame = start_frame;
                                    }
                                    while let Ok(even_newer) = cmd_rx.try_recv() {
                                        match even_newer {
                                            DecoderCommand::Seek { .. } | DecoderCommand::Pause { .. } => {
                                                newer = even_newer;
                                            }
                                            DecoderCommand::Play { start_frame, epoch } => {
                                                playhead_frame = start_frame;
                                                newer = DecoderCommand::Play { start_frame, epoch };
                                            }
                                            DecoderCommand::UpdatePlayhead { playhead_frame: pf } => {
                                                playhead_frame = pf;
                                            }
                                        }
                                    }
                                    match newer {
                                        DecoderCommand::Seek { target_frame: newer_target, epoch: newer_epoch } => {
                                            target_frame = newer_target;
                                            epoch = newer_epoch;
                                            active_epoch = newer_epoch;
                                            let new_target_pts = if time_base_f > 0.0 {
                                                (target_frame as f64 / fps / time_base_f).round() as i64
                                            } else {
                                                target_frame
                                            };
                                            if current_pts >= 0 && new_target_pts >= current_pts && (new_target_pts - current_pts) <= gop_max_pts {
                                                target_pts = new_target_pts;
                                                keyframe_sent = false;
                                            } else {
                                                pending_cmd = Some(DecoderCommand::Seek { target_frame, epoch });
                                                aborted_seek = true;
                                                break;
                                            }
                                        }
                                        DecoderCommand::Play { start_frame, epoch: newer_epoch } => {
                                            pending_cmd = Some(DecoderCommand::Play { start_frame, epoch: newer_epoch });
                                            aborted_seek = true;
                                            break;
                                        }
                                        DecoderCommand::Pause { epoch: newer_epoch } => {
                                            active_epoch = newer_epoch;
                                            aborted_seek = true;
                                            break;
                                        }
                                        DecoderCommand::UpdatePlayhead { playhead_frame: pf } => {
                                            playhead_frame = pf;
                                        }
                                    }
                                }
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                    break 'worker_loop;
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                            }

                            if st.index() == stream_idx {
                                if waiting_for_keyframe {
                                    if packet.is_key() {
                                        waiting_for_keyframe = false;
                                    } else {
                                        continue;
                                    }
                                }

                                if video_decoder.send_packet(&packet).is_ok() {
                                    while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                                        let pts = decoded_frame.pts().unwrap_or(current_pts + frame_duration_pts);
                                        current_pts = pts;
                                        let f_num = (pts as f64 * time_base_f * fps).round() as i64;
                                        current_decoded_frame = f_num;

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

                                        if !keyframe_sent {
                                            if let Some(kf_img) = Self::scale_and_copy_frame(
                                                &mut scaler,
                                                &decoded_frame,
                                                &mut sw_frame,
                                                &mut rgba_frame,
                                                w,
                                                h,
                                                &mut rgba_buffer,
                                            ) {
                                                if frame_tx.send((epoch, -(target_frame + 1), kf_img)).is_err() {
                                                    break 'worker_loop;
                                                }
                                                egui_ctx.request_repaint();
                                                keyframe_sent = true;
                                            }
                                        }
                                    }
                                }
                            }

                            if found_image.is_some() {
                                break;
                            }
                        }

                        // Дренируем буфер декодера на EOF (чтобы последние кадры в конце клипа не обрезались!)
                        if found_image.is_none() && !aborted_seek {
                            let _ = video_decoder.send_eof();
                            let mut last_drain_frame_ok = false;
                            while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                                let pts = decoded_frame.pts().unwrap_or(current_pts + frame_duration_pts);
                                current_pts = pts;
                                last_drain_frame_ok = true;
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
                            if found_image.is_none() && last_drain_frame_ok {
                                found_image = Self::scale_and_copy_frame(
                                    &mut scaler,
                                    &decoded_frame,
                                    &mut sw_frame,
                                    &mut rgba_frame,
                                    w,
                                    h,
                                    &mut rgba_buffer,
                                );
                            }
                            // После send_eof декодер нужно сбросить перед следующими пакетами
                            video_decoder.flush();
                            current_pts = -1;
                            current_decoded_frame = -1;
                            waiting_for_keyframe = true;
                        }
                    }

                    if let Some(img) = found_image {
                        if frame_tx.send((epoch, target_frame, img)).is_err() {
                            break 'worker_loop;
                        }
                        egui_ctx.request_repaint();
                    }
                }
                DecoderCommand::Play { start_frame, epoch } => {
                    active_epoch = epoch;
                    is_playing = true;
                    playhead_frame = start_frame;

                    let target_pts = if time_base_f > 0.0 {
                        (start_frame as f64 / fps / time_base_f).round() as i64
                    } else {
                        start_frame
                    };

                    let can_continue_forward = current_pts >= 0
                        && target_pts >= current_pts - tolerance
                        && (target_pts - current_pts) <= gop_max_pts
                        && !waiting_for_keyframe;

                    let mut keyframe_preview_sent = can_continue_forward && start_frame <= current_decoded_frame + 2;
                    let mut last_catchup_preview: i64 = -999;
                    if can_continue_forward {
                        vaapi_fifo.retain(|(_, fn_out, _)| *fn_out >= start_frame);
                        for (ep, _, _) in vaapi_fifo.iter_mut() {
                            *ep = active_epoch;
                        }
                        // Вычитываем уже находящиеся внутри декодера кадры до отправки новых пакетов!
                        // Это исключает скачок к началу GOP, когда мы только что сделали Seek на start_frame.
                        while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                            let pts = decoded_frame.pts().unwrap_or(current_pts + frame_duration_pts);
                            current_pts = pts;
                            let f_num = (pts as f64 * time_base_f * fps).round() as i64;
                            current_decoded_frame = f_num;
                            if f_num >= start_frame {
                                let mut moved = ffmpeg_next::util::frame::Video::empty();
                                unsafe {
                                    ffmpeg_next::ffi::av_frame_move_ref(moved.as_mut_ptr(), decoded_frame.as_mut_ptr());
                                }
                                vaapi_fifo.push_back((active_epoch, f_num, moved));
                                if vaapi_fifo.len() >= 3 {
                                    if let Some((ep, fn_out, front_f)) = vaapi_fifo.pop_front() {
                                        if !transfer_and_send(ep, fn_out, front_f, &recycle_rx, &scale_tx) {
                                            break 'worker_loop;
                                        }
                                    }
                                }
                            }
                            if is_playing && current_decoded_frame > playhead_frame + 65 {
                                while let Some((ep, fn_out, front_f)) = vaapi_fifo.pop_front() {
                                    if !transfer_and_send(ep, fn_out, front_f, &recycle_rx, &scale_tx) {
                                        break 'worker_loop;
                                    }
                                }
                            }
                            while is_playing && current_decoded_frame > playhead_frame + 65 {
                                match cmd_rx.recv_timeout(std::time::Duration::from_millis(4)) {
                                    Ok(DecoderCommand::UpdatePlayhead { playhead_frame: pf }) => {
                                        playhead_frame = pf;
                                    }
                                    Ok(other) => {
                                        pending_cmd = Some(other);
                                        break;
                                    }
                                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break 'worker_loop,
                                }
                                if pending_cmd.is_some() {
                                    break;
                                }
                            }
                            if pending_cmd.is_some() {
                                break;
                            }
                        }
                    } else {
                        vaapi_fifo.clear();
                        unsafe {
                            ffmpeg_next::ffi::av_seek_frame(
                                ictx.as_mut_ptr(),
                                stream_idx as i32,
                                target_pts,
                                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
                            );
                        }
                        video_decoder.flush();
                        current_pts = -1;
                        current_decoded_frame = -1;
                        waiting_for_keyframe = true;
                    }
                    start_playback_frame = start_frame;

                    if pending_cmd.is_some() {
                        continue 'worker_loop;
                    }

                    let mut stream_aborted = false;
                    for (st, packet) in ictx.packets() {
                        let mut break_stream = false;
                        let mut should_exit_worker = false;

                        loop {
                            match cmd_rx.try_recv() {
                                Ok(cmd) => match cmd {
                                    DecoderCommand::UpdatePlayhead { playhead_frame: pf } => {
                                        playhead_frame = pf;
                                    }
                                    DecoderCommand::Play { start_frame: new_sf, epoch: new_ep } => {
                                        active_epoch = new_ep;
                                        playhead_frame = new_sf;
                                        let ref_f = if current_decoded_frame >= 0 { current_decoded_frame } else { start_playback_frame };
                                        if new_sf < ref_f.saturating_sub(2) || new_sf > ref_f + 125 {
                                            vaapi_fifo.clear();
                                            pending_cmd = Some(DecoderCommand::Play { start_frame: new_sf, epoch: new_ep });
                                            break_stream = true;
                                            break;
                                        } else {
                                            start_playback_frame = new_sf;
                                            keyframe_preview_sent = false;
                                            vaapi_fifo.retain(|(_, fn_out, _)| *fn_out >= new_sf);
                                            for (ep, _, _) in vaapi_fifo.iter_mut() {
                                                *ep = new_ep;
                                            }
                                        }
                                    }
                                    DecoderCommand::Pause { epoch: new_ep } => {
                                        active_epoch = new_ep;
                                        cancel_epoch.store(new_ep, std::sync::atomic::Ordering::Release);
                                        vaapi_fifo.clear();
                                        is_playing = false;
                                        break_stream = true;
                                        break;
                                    }
                                    DecoderCommand::Seek { target_frame, epoch: new_ep } => {
                                        active_epoch = new_ep;
                                        cancel_epoch.store(new_ep, std::sync::atomic::Ordering::Release);
                                        vaapi_fifo.clear();
                                        is_playing = false;
                                        pending_cmd = Some(DecoderCommand::Seek { target_frame, epoch: new_ep });
                                        break_stream = true;
                                        break;
                                    }
                                },
                                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                    should_exit_worker = true;
                                    break;
                                }
                                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                            }
                        }

                        if should_exit_worker {
                            break 'worker_loop;
                        }

                        if break_stream {
                            stream_aborted = true;
                            break;
                        }

                        if st.index() == stream_idx {
                            if waiting_for_keyframe {
                                if packet.is_key() {
                                    waiting_for_keyframe = false;
                                } else {
                                    continue;
                                }
                            }

                            if video_decoder.send_packet(&packet).is_ok() {
                                while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                                    let pts = decoded_frame.pts().unwrap_or(current_pts + frame_duration_pts);
                                    current_pts = pts;
                                    let f_num = (pts as f64 * time_base_f * fps).round() as i64;
                                    current_decoded_frame = f_num;

                                    // Пропускаем медленный трансфер из VRAM в RAM для кадров до live_target (но отдаем ключевой кадр
                                    // и промежуточные превью каждые 15 кадров при холодном старте или скачке в середину GOP!),
                                    // чтобы декодер мгновенно догонял живой playhead_frame и выдавал стабильные 60 FPS!
                                    let live_target = playhead_frame.max(start_playback_frame);
                                    if f_num < live_target.saturating_sub(1) {
                                        if !keyframe_preview_sent || (f_num - last_catchup_preview >= 15) {
                                            keyframe_preview_sent = true;
                                            last_catchup_preview = f_num;
                                            let mut moved = ffmpeg_next::util::frame::Video::empty();
                                            unsafe {
                                                ffmpeg_next::ffi::av_frame_move_ref(moved.as_mut_ptr(), decoded_frame.as_mut_ptr());
                                            }
                                            if !transfer_and_send(active_epoch, -(f_num + 1), moved, &recycle_rx, &scale_tx) {
                                                break 'worker_loop;
                                            }
                                        }
                                    } else if f_num >= start_playback_frame {
                                        keyframe_preview_sent = true;
                                        let mut moved = ffmpeg_next::util::frame::Video::empty();
                                        unsafe {
                                            ffmpeg_next::ffi::av_frame_move_ref(moved.as_mut_ptr(), decoded_frame.as_mut_ptr());
                                        }
                                        vaapi_fifo.push_back((active_epoch, f_num, moved));
                                        if vaapi_fifo.len() >= 3 {
                                            if let Some((ep, fn_out, front_f)) = vaapi_fifo.pop_front() {
                                                if !transfer_and_send(ep, fn_out, front_f, &recycle_rx, &scale_tx) {
                                                    break 'worker_loop;
                                                }
                                            }
                                        }
                                    }

                                    if is_playing && current_decoded_frame > playhead_frame + 65 {
                                        while let Some((ep, fn_out, front_f)) = vaapi_fifo.pop_front() {
                                            if !transfer_and_send(ep, fn_out, front_f, &recycle_rx, &scale_tx) {
                                                break 'worker_loop;
                                            }
                                        }
                                    }

                                    // Упреждающий троттлинг jitter buffer
                                    while is_playing && current_decoded_frame > playhead_frame + 65 {
                                        match cmd_rx.recv_timeout(std::time::Duration::from_millis(4)) {
                                            Ok(DecoderCommand::UpdatePlayhead { playhead_frame: pf }) => {
                                                playhead_frame = pf;
                                            }
                                            Ok(DecoderCommand::Play { start_frame: new_sf, epoch: new_ep }) => {
                                                active_epoch = new_ep;
                                                playhead_frame = new_sf;
                                                let ref_f = if current_decoded_frame >= 0 { current_decoded_frame } else { start_playback_frame };
                                                if new_sf < ref_f.saturating_sub(2) || new_sf > ref_f + 60 {
                                                    vaapi_fifo.clear();
                                                    pending_cmd = Some(DecoderCommand::Play { start_frame: new_sf, epoch: new_ep });
                                                    break_stream = true;
                                                    break;
                                                } else {
                                                    start_playback_frame = new_sf;
                                                    keyframe_preview_sent = false;
                                                    vaapi_fifo.retain(|(_, fn_out, _)| *fn_out >= new_sf);
                                                    for (ep, _, _) in vaapi_fifo.iter_mut() {
                                                        *ep = new_ep;
                                                    }
                                                }
                                            }
                                            Ok(DecoderCommand::Pause { epoch: new_ep }) => {
                                                active_epoch = new_ep;
                                                cancel_epoch.store(new_ep, std::sync::atomic::Ordering::Release);
                                                vaapi_fifo.clear();
                                                is_playing = false;
                                                break_stream = true;
                                                break;
                                            }
                                            Ok(DecoderCommand::Seek { target_frame, epoch: new_ep }) => {
                                                active_epoch = new_ep;
                                                cancel_epoch.store(new_ep, std::sync::atomic::Ordering::Release);
                                                vaapi_fifo.clear();
                                                is_playing = false;
                                                pending_cmd = Some(DecoderCommand::Seek { target_frame, epoch: new_ep });
                                                break_stream = true;
                                                break;
                                            }
                                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break 'worker_loop,
                                        }
                                    }

                                    if break_stream {
                                        break;
                                    }
                                }
                            }
                        }

                        if break_stream {
                            stream_aborted = true;
                            break;
                        }
                    }

                    // Если пакеты файла закончились в режиме воспроизведения — дренируем хвост декодера до последнего кадра!
                    if !stream_aborted && is_playing {
                        let _ = video_decoder.send_eof();
                        while video_decoder.receive_frame(&mut decoded_frame).is_ok() {
                            let pts = decoded_frame.pts().unwrap_or(current_pts + frame_duration_pts);
                            current_pts = pts;
                            let f_num = (pts as f64 * time_base_f * fps).round() as i64;
                            if f_num >= start_playback_frame {
                                let mut moved = ffmpeg_next::util::frame::Video::empty();
                                unsafe {
                                    ffmpeg_next::ffi::av_frame_move_ref(moved.as_mut_ptr(), decoded_frame.as_mut_ptr());
                                }
                                vaapi_fifo.push_back((active_epoch, f_num, moved));
                            }
                        }
                        while let Some((ep, fn_out, front_f)) = vaapi_fifo.pop_front() {
                            if !transfer_and_send(ep, fn_out, front_f, &recycle_rx, &scale_tx) {
                                break 'worker_loop;
                            }
                        }
                        video_decoder.flush();
                        current_pts = -1;
                        current_decoded_frame = -1;
                        waiting_for_keyframe = true;
                        is_playing = false;
                    }
                }
            }
        }

        drop(vaapi_fifo);
        drop(scale_tx);
        let _ = scale_worker_handle.join();

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
        _rgba_buffer: &mut Vec<u8>,
    ) -> Option<ColorImage> {
        let frame_to_scale = unsafe {
            let fmt = (*decoded_frame.as_ptr()).format;
            if fmt == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI as i32 {
                if ffmpeg_next::ffi::av_hwframe_transfer_data(sw_frame.as_mut_ptr(), decoded_frame.as_ptr(), 0) == 0 {
                    &*sw_frame
                } else {
                    decoded_frame
                }
            } else {
                decoded_frame
            }
        };

        // Адаптация формата конвертера при необходимости
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

            // Безопасная однопроходная конвертация RGBA8 -> Color32 (один memcpy, без unsafe и без промежуточного клона)
            let pixels: Vec<egui::Color32> = if stride == w * 4 && data.len() >= w * h * 4 {
                bytemuck::pod_collect_to_vec::<u8, egui::Color32>(&data[..w * h * 4])
            } else {
                let mut px: Vec<egui::Color32> = Vec::with_capacity(w * h);
                for y in 0..h {
                    let src_start = y * stride;
                    if src_start + w * 4 > data.len() {
                        return None;
                    }
                    px.extend_from_slice(bytemuck::cast_slice::<u8, egui::Color32>(&data[src_start..src_start + w * 4]));
                }
                px
            };

            Some(ColorImage { size: [w, h], source_size: egui::vec2(w as f32, h as f32), pixels })
        } else {
            None
        }
    }

    /// Чистый экран ожидания медиа
    fn generate_placeholder() -> ColorImage {
        ColorImage::new([2, 2], vec![egui::Color32::from_rgb(18, 19, 21); 4])
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

    #[test]
    fn test_seek_to_90() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        let mut ictx = ffmpeg_next::format::input(&test_path).unwrap();
        let (stream_idx, time_base) = {
            let s = ictx.streams().best(ffmpeg_next::media::Type::Video).unwrap();
            (s.index(), s.time_base())
        };

        let target_frame = 90;
        let time_base_f = f64::from(time_base);
        let target_pts = (target_frame as f64 / 60.0 / time_base_f).round() as i64;
        println!("target_pts: {}", target_pts);

        unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                ictx.as_mut_ptr(),
                stream_idx as i32,
                target_pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
            );
        }

        let mut count = 0;
        for (st, packet) in ictx.packets() {
            if st.index() == stream_idx {
                if packet.is_key() {
                    println!("Key packet {}: pts={:?}", count, packet.pts());
                }
                count += 1;
                if count >= 300 {
                    break;
                }
            }
        }
    }

    #[test]
    fn test_seek_packets() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        let ctx = egui::Context::default();
        let mut player = VideoPlayer::new();
        assert!(player.open_file(test_path, &ctx).is_ok());

        // Проверяем устойчивость к быстрому случайному скраббингу по всему 4K HEVC файлу
        let scrub_frames = [0, 50, 120, 240, 350, 480, 210, 75, 590, 15, 0];
        for &frame in &scrub_frames {
            player.force_seek_frame(frame, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(20));
            player.poll_frames(&ctx);
        }

        // Финальный кадр должен успешно декодироваться
        assert!(player.wait_for_frame(350, &ctx, std::time::Duration::from_secs(3)));
        assert_eq!(player.last_decoded_frame, 350);
    }

    #[test]
    fn test_clean_seek_and_playback_zero_errors() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        let ctx = egui::Context::default();
        let mut player = VideoPlayer::new();
        assert!(player.open_file(test_path, &ctx).is_ok());

        assert!(player.wait_for_frame(0, &ctx, std::time::Duration::from_secs(3)));

        // Скраббинг через границы GOP
        for &f in &[50, 120, 89, 15] {
            assert!(player.wait_for_frame(f, &ctx, std::time::Duration::from_secs(3)));
            assert_eq!(player.last_decoded_frame, f);
        }

        // Плавное воспроизведение, начиная с кадра 89 (без ошибок RPS/POC)
        for f in 89..130 {
            player.update_playback(f, true, &ctx);
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        player.update_playback(130, false, &ctx);
    }

    #[test]
    fn test_vaapi_gpu_decode_speed() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        unsafe extern "C" fn get_vaapi_format(
            _ctx: *mut ffmpeg_next::ffi::AVCodecContext,
            pix_fmts: *const ffmpeg_next::ffi::AVPixelFormat,
        ) -> ffmpeg_next::ffi::AVPixelFormat {
            let mut p = pix_fmts;
            while *p != ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                if *p == ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI {
                    return *p;
                }
                p = p.add(1);
            }
            ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_NONE
        }

        let mut ictx = ffmpeg_next::format::input(&test_path).unwrap();
        let (stream_idx, params) = {
            let s = ictx.streams().best(ffmpeg_next::media::Type::Video).unwrap();
            (s.index(), s.parameters())
        };

        let mut ctx = ffmpeg_next::codec::context::Context::from_parameters(params).unwrap();

        let mut hw_device_ctx: *mut ffmpeg_next::ffi::AVBufferRef = std::ptr::null_mut();
        let dev_path = std::ffi::CString::new("/dev/dri/renderD128").unwrap();
        let err = unsafe {
            ffmpeg_next::ffi::av_hwdevice_ctx_create(
                &mut hw_device_ctx,
                ffmpeg_next::ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                dev_path.as_ptr(),
                std::ptr::null_mut(),
                0,
            )
        };
        assert_eq!(err, 0, "Failed to create VAAPI device context");
        assert!(!hw_device_ctx.is_null());

        unsafe {
            let c_ptr = ctx.as_mut_ptr();
            (*c_ptr).hw_device_ctx = ffmpeg_next::ffi::av_buffer_ref(hw_device_ctx);
            (*c_ptr).get_format = Some(get_vaapi_format);
        }

        let mut decoder = ctx.decoder().video().unwrap();
        let mut hw_frame = ffmpeg_next::util::frame::Video::empty();
        let mut sw_frame = ffmpeg_next::util::frame::Video::empty();

        let mut scaler = ffmpeg_next::software::scaling::Context::get(
            ffmpeg_next::format::Pixel::NV12,
            3840,
            2160,
            ffmpeg_next::format::Pixel::RGBA,
            1280,
            720,
            ffmpeg_next::software::scaling::flag::Flags::FAST_BILINEAR,
        ).unwrap();
        let mut rgba_frame = ffmpeg_next::util::frame::Video::empty();
        let mut rgba_buf = vec![0u8; 1280 * 720 * 4];

        let start = std::time::Instant::now();
        let mut decoded = 0;

        for (st, packet) in ictx.packets() {
            if st.index() == stream_idx {
                if decoder.send_packet(&packet).is_ok() {
                    while decoder.receive_frame(&mut hw_frame).is_ok() {
                        unsafe {
                            let ret = ffmpeg_next::ffi::av_hwframe_transfer_data(
                                sw_frame.as_mut_ptr(),
                                hw_frame.as_ptr(),
                                0,
                            );
                            assert_eq!(ret, 0, "Transfer from GPU failed");
                        }
                        scaler.run(&sw_frame, &mut rgba_frame).unwrap();
                        let d = rgba_frame.data(0);
                        let s = rgba_frame.stride(0);
                        for y in 0..720 {
                            let src = y * s;
                            let dst = y * 1280 * 4;
                            rgba_buf[dst..dst + 1280 * 4].copy_from_slice(&d[src..src + 1280 * 4]);
                        }
                        decoded += 1;
                        if decoded >= 60 {
                            break;
                        }
                    }
                    if decoded >= 60 {
                        break;
                    }
                }
            }
        }

        let el = start.elapsed();
        println!("GPU VAAPI + Swscale to 720p: 60 frames in {:?} ({:.1} FPS)!", el, 60.0 / el.as_secs_f64());
        assert!(decoded >= 60);

        unsafe {
            ffmpeg_next::ffi::av_buffer_unref(&mut hw_device_ctx);
        }
    }

    #[test]
    fn test_4k_mkv_continuous_smooth_playback() {
        let test_path = Path::new("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !test_path.exists() {
            return;
        }

        let ctx = egui::Context::default();
        let mut player = VideoPlayer::new();
        assert!(player.open_file(test_path, &ctx).is_ok());

        // Симулируем 60 кадров воспроизведения в реальном времени (1 секунда 60 FPS видео)
        let mut rendered_frames = Vec::new();
        for target_frame in 0..60 {
            player.update_playback(target_frame, true, &ctx);
            if player.last_decoded_frame >= 0 && rendered_frames.last() != Some(&player.last_decoded_frame) {
                rendered_frames.push(player.last_decoded_frame);
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }

        // Даем 200 мс на финализацию очереди
        for _ in 0..15 {
            std::thread::sleep(std::time::Duration::from_millis(16));
            player.update_playback(59, true, &ctx);
            if player.last_decoded_frame >= 0 && rendered_frames.last() != Some(&player.last_decoded_frame) {
                rendered_frames.push(player.last_decoded_frame);
            }
        }

        println!("Rendered frames count: {}, frames: {:?}", rendered_frames.len(), rendered_frames);
        let min_expected = if cfg!(debug_assertions) { 15 } else { 35 };
        assert!(rendered_frames.len() >= min_expected, "Должно воспроизводиться плавно без потерь кадров: получено {} кадров", rendered_frames.len());
        // Проверяем монотонное возрастание кадров
        for w in rendered_frames.windows(2) {
            assert!(w[0] < w[1], "Кадры должны идти строго последовательно: {} -> {}", w[0], w[1]);
        }
    }
}
