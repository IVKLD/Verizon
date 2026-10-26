use egui::ColorImage;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
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
pub struct RenderCacheManager {
    pub mode: RenderCacheMode,
    cache: Arc<Mutex<HashMap<i64, CachedRenderFrame>>>,
    lru_order: Arc<Mutex<VecDeque<i64>>>,
    cached_set: Arc<RwLock<HashSet<i64>>>,
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
            max_frames: max_frames.max(1),
            generation: Arc::new(AtomicU64::new(0)),
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
