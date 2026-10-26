use egui::ColorImage;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use crate::timeline::TimelineState;

/// Режим работы Render Cache (аналог DaVinci Resolve Smart / User Cache)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RenderCacheMode {
    /// Кэширование отключено
    None,
    /// Smart Cache: автоматическое упреждающее кэширование кадров таймлайна
    Smart,
    /// User Cache: кэширование по запросу
    User,
}

impl Default for RenderCacheMode {
    fn default() -> Self {
        Self::Smart
    }
}

/// Кэшированный кадр таймлайна с сигнатурой валидности
#[derive(Clone)]
pub struct CachedRenderFrame {
    pub frame: i64,
    pub image: Arc<ColorImage>,
    /// Хэш параметров клипов под этим кадром (для защиты от рассинхронизации при правках)
    pub signature: u64,
}

/// Статистика Render Cache для отображения в UI
#[derive(Debug, Clone, Copy)]
pub struct RenderCacheStats {
    pub cached_frames: usize,
    pub max_frames: usize,
    pub mode: RenderCacheMode,
    pub memory_mb: f32,
}

/// Менеджер Render Cache: thread-safe, lock-free проверка наличия, LRU-вытеснение
#[derive(Clone)]
pub struct RenderCacheManager {
    pub mode: RenderCacheMode,
    cache: Arc<Mutex<HashMap<i64, CachedRenderFrame>>>,
    lru_order: Arc<Mutex<VecDeque<i64>>>,
    cached_set: Arc<RwLock<HashSet<i64>>>,
    cached_intervals: Arc<RwLock<(u64, Vec<(i64, i64)>)>>,
    pub max_frames: usize,
    pub generation: Arc<AtomicU64>,
}

impl Default for RenderCacheManager {
    fn default() -> Self {
        Self::new(RenderCacheMode::Smart, 360) // 360 кадров = 6 секунд 60 FPS в RAM (~1.3 ГБ)
    }
}

impl RenderCacheManager {
    pub fn new(mode: RenderCacheMode, max_frames: usize) -> Self {
        Self {
            mode,
            cache: Arc::new(Mutex::new(HashMap::with_capacity(max_frames))),
            lru_order: Arc::new(Mutex::new(VecDeque::with_capacity(max_frames))),
            cached_set: Arc::new(RwLock::new(HashSet::with_capacity(max_frames))),
            cached_intervals: Arc::new(RwLock::new((0, Vec::new()))),
            max_frames: max_frames.max(1),
            generation: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Вычислить сигнатуру валидности для кадра таймлайна
    pub fn compute_frame_signature(timeline: &TimelineState, frame: i64) -> u64 {
        let mut hasher = DefaultHasher::new();
        // Включаем активные видеоклипы на таймлайне под этим кадром
        for c in &timeline.clips {
            if !c.is_audio && c.contains_frame(frame) {
                c.id.hash(&mut hasher);
                c.track_index.hash(&mut hasher);
                c.start_frame.hash(&mut hasher);
                c.duration_frames.hash(&mut hasher);
                c.source_in_secs.to_bits().hash(&mut hasher);
                c.source_fps.to_bits().hash(&mut hasher);
                c.zoom_x.to_bits().hash(&mut hasher);
                c.zoom_y.to_bits().hash(&mut hasher);
                c.position_x.to_bits().hash(&mut hasher);
                c.position_y.to_bits().hash(&mut hasher);
                c.rotation.to_bits().hash(&mut hasher);
                c.opacity.to_bits().hash(&mut hasher);
                c.speed.to_bits().hash(&mut hasher);
                c.is_disabled.hash(&mut hasher);
                if let Some(ref path) = c.source_path {
                    path.hash(&mut hasher);
                }
            }
        }
        hasher.finish()
    }

    /// Быстрая O(1) проверка наличия кадра в кэше (без Mutex на хранилище, для быстрой отрисовки полосы Ruler)
    #[inline]
    pub fn is_cached(&self, frame: i64) -> bool {
        if self.mode == RenderCacheMode::None {
            return false;
        }
        if let Ok(set) = self.cached_set.read() {
            set.contains(&frame)
        } else {
            false
        }
    }

    /// Захват read guard для пакетных проверок без повторного взятия RwLock на каждый кадр
    #[inline]
    pub fn read_cached_set(&self) -> Option<std::sync::RwLockReadGuard<'_, HashSet<i64>>> {
        if self.mode == RenderCacheMode::None {
            return None;
        }
        self.cached_set.read().ok()
    }

    /// Получить отсортированный список непрерывных интервалов кэшированных кадров (с O(1) переиспользованием по поколению)
    pub fn cached_intervals(&self) -> Vec<(i64, i64)> {
        if self.mode == RenderCacheMode::None {
            return Vec::new();
        }
        let cur_gen = self.generation.load(Ordering::Acquire);
        if let Ok(guard) = self.cached_intervals.read() {
            if guard.0 == cur_gen {
                return guard.1.clone();
            }
        }

        let Ok(set) = self.cached_set.read() else {
            return Vec::new();
        };
        let mut frames: Vec<i64> = set.iter().copied().collect();
        frames.sort_unstable();

        let mut intervals: Vec<(i64, i64)> = Vec::new();
        for f in frames {
            if let Some(last) = intervals.last_mut() {
                if f == last.1 + 1 {
                    last.1 = f;
                    continue;
                }
            }
            intervals.push((f, f));
        }

        if let Ok(mut guard) = self.cached_intervals.write() {
            *guard = (cur_gen, intervals.clone());
        }
        intervals
    }

    /// Получить готовый кадр из Render Cache (0 мс доступ к RAM без декодирования)
    pub fn get(&self, frame: i64, expected_sig: u64) -> Option<Arc<ColorImage>> {
        if self.mode == RenderCacheMode::None {
            return None;
        }

        // Предварительная проверка через легкий RwLock
        {
            let set = self.cached_set.read().ok()?;
            if !set.contains(&frame) {
                return None;
            }
        }

        let mut cache = self.cache.lock().ok()?;
        let entry = cache.get(&frame)?;
        if entry.signature != expected_sig {
            // Клип был изменен (сигнатура не совпала) — мгновенная инвалидация
            cache.remove(&frame);
            if let Ok(mut set) = self.cached_set.write() {
                set.remove(&frame);
            }
            return None;
        }

        let img = Arc::clone(&entry.image);

        // Обновляем LRU-позицию
        if let Ok(mut lru) = self.lru_order.lock() {
            if let Some(pos) = lru.iter().position(|&f| f == frame) {
                lru.remove(pos);
                lru.push_back(frame);
            }
        }

        Some(img)
    }

    /// Сохранить отрендеренный кадр в Render Cache с LRU-вытеснением
    pub fn insert(&self, frame: i64, image: Arc<ColorImage>, signature: u64) {
        if self.mode == RenderCacheMode::None {
            return;
        }

        let Ok(mut cache) = self.cache.lock() else { return; };
        let Ok(mut lru) = self.lru_order.lock() else { return; };
        let Ok(mut set) = self.cached_set.write() else { return; };

        if cache.contains_key(&frame) {
            cache.insert(frame, CachedRenderFrame { frame, image, signature });
            if let Some(pos) = lru.iter().position(|&f| f == frame) {
                lru.remove(pos);
            }
            lru.push_back(frame);
            set.insert(frame);
            self.generation.fetch_add(1, Ordering::Release);
            return;
        }

        // LRU-вытеснение старых кадров при превышении квоты
        while cache.len() >= self.max_frames {
            if let Some(oldest) = lru.pop_front() {
                cache.remove(&oldest);
                set.remove(&oldest);
            } else {
                break;
            }
        }

        cache.insert(frame, CachedRenderFrame { frame, image, signature });
        lru.push_back(frame);
        set.insert(frame);
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Инвалидировать диапазон кадров (например, при разрезе или сдвиге клипа)
    pub fn invalidate_range(&self, start_frame: i64, end_frame: i64) {
        let Ok(mut cache) = self.cache.lock() else { return; };
        let Ok(mut lru) = self.lru_order.lock() else { return; };
        let Ok(mut set) = self.cached_set.write() else { return; };

        for f in start_frame..=end_frame {
            cache.remove(&f);
            set.remove(&f);
        }
        lru.retain(|f| *f < start_frame || *f > end_frame);
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Полный сброс всего кэша
    pub fn invalidate_all(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
        }
        if let Ok(mut lru) = self.lru_order.lock() {
            lru.clear();
        }
        if let Ok(mut set) = self.cached_set.write() {
            set.clear();
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// Установить лимит оперативной памяти для кэша рендера (в мегабайтах)
    pub fn set_max_memory_mb(&mut self, mb: usize) {
        // Кадр 720p/1080p ~ 3.68 МБ
        let frames = (mb as f32 / 3.68).round() as usize;
        self.max_frames = frames.clamp(30, 10000);
    }

    /// Получить статистику для GUI
    pub fn stats(&self) -> RenderCacheStats {
        let count = self.cached_set.read().map(|s| s.len()).unwrap_or(0);
        // Приблизительный объем памяти (720p RGBA = 3.68 МБ на кадр)
        let memory_mb = count as f32 * 3.68;
        RenderCacheStats {
            cached_frames: count,
            max_frames: self.max_frames,
            mode: self.mode,
            memory_mb,
        }
    }
}

/// Фоновый авто-рендерер Render Cache для незакэшированных красных участков таймлайна в режиме ожидания (Idle)
pub struct BackgroundCacheEngine {
    is_busy: Arc<AtomicBool>,
    cancel_flag: Arc<AtomicBool>,
}

impl Default for BackgroundCacheEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundCacheEngine {
    pub fn new() -> Self {
        Self {
            is_busy: Arc::new(AtomicBool::new(false)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_busy(&self) -> bool {
        self.is_busy.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Release);
    }

    /// Запросить фоновое кэширование пачки кадров без влияния на UI-поток
    pub fn request_cache_batch(
        &self,
        path: PathBuf,
        frames: Vec<(i64, i64, u64)>, // (src_frame, timeline_frame, signature)
        source_fps: f64,
        cache: RenderCacheManager,
        egui_ctx: egui::Context,
    ) {
        if frames.is_empty() {
            return;
        }
        if self.is_busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() {
            return;
        }
        self.cancel_flag.store(false, Ordering::Release);

        let is_busy = Arc::clone(&self.is_busy);
        let cancel_flag = Arc::clone(&self.cancel_flag);

        std::thread::spawn(move || {
            Self::run_cache_worker(path, frames, source_fps, cache, egui_ctx, cancel_flag);
            is_busy.store(false, Ordering::Release);
        });
    }

    fn run_cache_worker(
        path: PathBuf,
        frames: Vec<(i64, i64, u64)>,
        source_fps: f64,
        cache: RenderCacheManager,
        egui_ctx: egui::Context,
        cancel_flag: Arc<AtomicBool>,
    ) {
        let Ok(mut ictx) = ffmpeg_next::format::input(&path) else { return; };
        let Some(video_stream) = ictx.streams().best(ffmpeg_next::media::Type::Video) else { return; };
        let stream_idx = video_stream.index();
        let time_base = video_stream.time_base();
        let Ok(context_decoder) = ffmpeg_next::codec::context::Context::from_parameters(video_stream.parameters()) else { return; };
        let Ok(mut decoder) = context_decoder.decoder().video() else { return; };

        let first_src = frames[0].0;
        let time_base_f = time_base.numerator() as f64 / time_base.denominator() as f64;
        let target_pts = if time_base_f > 0.0 {
            (first_src as f64 / source_fps.max(1.0) / time_base_f).round() as i64
        } else {
            first_src
        };
        unsafe {
            ffmpeg_next::ffi::av_seek_frame(
                ictx.as_mut_ptr(),
                stream_idx as i32,
                target_pts,
                ffmpeg_next::ffi::AVSEEK_FLAG_BACKWARD,
            );
        }
        decoder.flush();

        let orig_w = decoder.width();
        let orig_h = decoder.height();
        let scaled_w = 1280.min(orig_w);
        let scaled_h = (scaled_w as f32 * (orig_h as f32 / orig_w.max(1) as f32)).round() as u32;

        let Ok(mut scaler) = ffmpeg_next::software::scaling::Context::get(
            decoder.format(),
            orig_w,
            orig_h,
            ffmpeg_next::format::Pixel::RGBA,
            scaled_w,
            scaled_h,
            ffmpeg_next::software::scaling::flag::Flags::FAST_BILINEAR,
        ) else { return; };

        let mut decoded_frame = ffmpeg_next::util::frame::Video::empty();
        let mut rgb_frame = ffmpeg_next::util::frame::Video::empty();

        let mut target_map: HashMap<i64, Vec<(i64, u64)>> = HashMap::with_capacity(frames.len());
        for (src_f, tl_f, sig) in frames {
            target_map.entry(src_f).or_default().push((tl_f, sig));
        }

        let mut cached_count = 0;
        let mut last_valid_img: Option<Arc<ColorImage>> = None;
        for (stream, packet) in ictx.packets() {
            if cancel_flag.load(Ordering::Acquire) {
                break;
            }
            if stream.index() != stream_idx {
                continue;
            }

            if decoder.send_packet(&packet).is_ok() {
                while decoder.receive_frame(&mut decoded_frame).is_ok() {
                    if cancel_flag.load(Ordering::Acquire) {
                        if cached_count > 0 {
                            egui_ctx.request_repaint();
                        }
                        return;
                    }
                    let pts = decoded_frame.pts().unwrap_or(0);
                    let cur_src_f = if time_base_f > 0.0 {
                        (pts as f64 * time_base_f * source_fps).round() as i64
                    } else {
                        pts
                    };

                    let matched_key = if target_map.contains_key(&cur_src_f) {
                        Some(cur_src_f)
                    } else if target_map.contains_key(&(cur_src_f - 1)) {
                        Some(cur_src_f - 1)
                    } else if target_map.contains_key(&(cur_src_f + 1)) {
                        Some(cur_src_f + 1)
                    } else {
                        None
                    };

                    if let Some(key) = matched_key {
                        let tl_targets = target_map.remove(&key).unwrap();
                        if scaler.run(&decoded_frame, &mut rgb_frame).is_ok() {
                            let data = rgb_frame.data(0);
                            let stride = rgb_frame.stride(0);
                            let mut pixels = Vec::with_capacity((scaled_w * scaled_h * 4) as usize);
                            for row in 0..scaled_h as usize {
                                let start = row * stride;
                                let end = start + (scaled_w as usize * 4);
                                if end <= data.len() {
                                    pixels.extend_from_slice(&data[start..end]);
                                }
                            }
                            let color_img = ColorImage::from_rgba_unmultiplied(
                                [scaled_w as usize, scaled_h as usize],
                                &pixels,
                            );
                            let arc_img = Arc::new(color_img);
                            last_valid_img = Some(Arc::clone(&arc_img));
                            for (tl_f, sig) in tl_targets {
                                cache.insert(tl_f, Arc::clone(&arc_img), sig);
                                cached_count += 1;
                            }
                            if cached_count % 2 == 0 || target_map.is_empty() {
                                egui_ctx.request_repaint();
                            }
                        }
                    }

                    if target_map.is_empty() {
                        egui_ctx.request_repaint();
                        return;
                    }
                }
            }
        }

        // Дренируем буфер декодера до EOF, чтобы не терялись хвостовые кадры
        if !target_map.is_empty() && !cancel_flag.load(Ordering::Acquire) {
            let _ = decoder.send_eof();
            while decoder.receive_frame(&mut decoded_frame).is_ok() {
                if cancel_flag.load(Ordering::Acquire) {
                    if cached_count > 0 {
                        egui_ctx.request_repaint();
                    }
                    return;
                }
                let pts = decoded_frame.pts().unwrap_or(0);
                let cur_src_f = if time_base_f > 0.0 {
                    (pts as f64 * time_base_f * source_fps).round() as i64
                } else {
                    pts
                };

                let matched_key = if target_map.contains_key(&cur_src_f) {
                    Some(cur_src_f)
                } else if target_map.contains_key(&(cur_src_f - 1)) {
                    Some(cur_src_f - 1)
                } else if target_map.contains_key(&(cur_src_f + 1)) {
                    Some(cur_src_f + 1)
                } else {
                    None
                };

                if let Some(key) = matched_key {
                    let tl_targets = target_map.remove(&key).unwrap();
                    if scaler.run(&decoded_frame, &mut rgb_frame).is_ok() {
                        let data = rgb_frame.data(0);
                        let stride = rgb_frame.stride(0);
                        let mut pixels = Vec::with_capacity((scaled_w * scaled_h * 4) as usize);
                        for row in 0..scaled_h as usize {
                            let start = row * stride;
                            let end = start + (scaled_w as usize * 4);
                            if end <= data.len() {
                                pixels.extend_from_slice(&data[start..end]);
                            }
                        }
                        let color_img = ColorImage::from_rgba_unmultiplied(
                            [scaled_w as usize, scaled_h as usize],
                            &pixels,
                        );
                        let arc_img = Arc::new(color_img);
                        last_valid_img = Some(Arc::clone(&arc_img));
                        for (tl_f, sig) in tl_targets {
                            cache.insert(tl_f, Arc::clone(&arc_img), sig);
                            cached_count += 1;
                        }
                    }
                }

                if target_map.is_empty() {
                    break;
                }
            }

            // Если декодер закончил отдавать кадры (EOF), а в target_map еще остались хвостовые кадры клипа
            // (например, при расхождении длительностей аудио и видео в контейнере):
            // заполняем их последним валидным кадром (Hold Frame), полностью устраняя красную зону!
            if !target_map.is_empty() && !cancel_flag.load(Ordering::Acquire) {
                if let Some(ref last_img) = last_valid_img {
                    for (_, tl_targets) in target_map.drain() {
                        for (tl_f, sig) in tl_targets {
                            cache.insert(tl_f, Arc::clone(last_img), sig);
                            cached_count += 1;
                        }
                    }
                }
            }
        }
        if cached_count > 0 {
            egui_ctx.request_repaint();
        }
    }
}
