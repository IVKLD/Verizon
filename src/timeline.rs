use egui::Color32;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Selection, // Клавиша 'A' (Стрелка для перемещения и выделения)
    Blade,     // Клавиша 'B' (Лезвие для нарезки клипов)
    #[allow(dead_code)]
    Trim,      // Клавиша 'T' (Подрезка краев)
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub id: u64,
    pub name: String,
    pub track_index: usize,
    pub start_frame: i64,
    pub duration_frames: i64,
    pub is_audio: bool,
    pub color: Color32,

    // Ссылка на реальный медиафайл
    pub source_path: Option<PathBuf>,
    pub source_in_frame: i64,
    pub source_fps: f64,

    // Свойства инспектора (Transform / Composite / Audio)
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub position_x: f32,
    pub position_y: f32,
    pub rotation: f32,
    pub opacity: f32,
    pub volume_db: f32,
    pub pan: f32,
}

impl Clip {
    pub fn end_frame(&self) -> i64 {
        self.start_frame + self.duration_frames
    }

    pub fn contains_frame(&self, frame: i64) -> bool {
        frame >= self.start_frame && frame < self.end_frame()
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub is_video: bool,
    #[allow(dead_code)]
    pub is_muted: bool,
    #[allow(dead_code)]
    pub is_locked: bool,
    #[allow(dead_code)]
    pub is_solo: bool,
}

#[derive(Debug, Clone)]
pub struct DraggedMedia {
    pub name: String,
    pub path: Option<PathBuf>,
    pub is_audio: bool,
    pub duration_frames: i64,
    pub source_fps: f64,
}

#[derive(Debug, Clone)]
pub struct TimelineState {
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub next_clip_id: u64,

    pub playhead_frame: i64,
    pub fps: u32,
    pub zoom_factor: f32, // Пикселей на кадр
    pub scroll_x: f32,

    pub in_point: Option<i64>,
    pub out_point: Option<i64>,
    pub loop_enabled: bool, // Флаг зацикливания воспроизведения

    pub active_tool: Tool,
    pub snapping_enabled: bool,
    pub selected_clip_id: Option<u64>,
    pub is_playing: bool,
    pub playback_speed: f64, // 1.0 = нормальная, 2.0 = 2x (L), -1.0 = реверс (J)
    pub frame_accumulator: f64,

    // Активно перетаскиваемый клип (Drag & Drop)
    pub dragged_media: Option<DraggedMedia>,

    // Очередь импорта медиа
    pub pending_imports: Vec<PathBuf>,

    // История для Undo
    pub history: Vec<Vec<Clip>>,
}

impl TimelineState {
    pub fn new_empty() -> Self {
        let tracks = vec![
            Track {
                name: "V2".to_string(),
                is_video: true,
                is_muted: false,
                is_locked: false,
                is_solo: false,
            },
            Track {
                name: "V1".to_string(),
                is_video: true,
                is_muted: false,
                is_locked: false,
                is_solo: false,
            },
            Track {
                name: "A1".to_string(),
                is_video: false,
                is_muted: false,
                is_locked: false,
                is_solo: false,
            },
            Track {
                name: "A2".to_string(),
                is_video: false,
                is_muted: false,
                is_locked: false,
                is_solo: false,
            },
        ];

        Self {
            tracks,
            clips: Vec::new(),
            next_clip_id: 1,
            playhead_frame: 0,
            fps: 24,
            zoom_factor: 2.0,
            scroll_x: 0.0,
            in_point: None,
            out_point: None,
            loop_enabled: false,
            active_tool: Tool::Selection,
            snapping_enabled: true,
            selected_clip_id: None,
            is_playing: false,
            playback_speed: 1.0,
            frame_accumulator: 0.0,
            dragged_media: None,
            pending_imports: Vec::new(),
            history: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn new_demo() -> Self {
        Self::new_empty()
    }

    /// Максимальный конечный кадр всех клипов на таймлайне
    pub fn timeline_end_frame(&self) -> i64 {
        self.clips.iter().map(|c| c.end_frame()).max().unwrap_or(0)
    }

    /// Сохранить текущее состояние клипов для Undo
    pub fn push_history(&mut self) {
        if self.history.len() > 30 {
            self.history.remove(0);
        }
        self.history.push(self.clips.clone());
    }

    /// Отменить последнее действие (Ctrl+Z)
    pub fn undo(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.clips = prev;
        }
    }

    /// Разрезать клип лезвием (Blade Cut) в указанном кадре
    pub fn blade_cut_at(&mut self, clip_id: u64, cut_frame: i64) -> bool {
        if let Some(idx) = self.clips.iter().position(|c| c.id == clip_id) {
            let start = self.clips[idx].start_frame;
            let duration = self.clips[idx].duration_frames;
            let end = start + duration;

            if cut_frame > start && cut_frame < end {
                self.push_history();

                let left_duration = cut_frame - start;
                let right_duration = duration - left_duration;

                let mut right_clip = self.clips[idx].clone();
                right_clip.id = self.next_clip_id;
                self.next_clip_id += 1;
                right_clip.start_frame = cut_frame;
                right_clip.duration_frames = right_duration;
                // Сдвиг смещения в исходном файле
                let frames_offset_in_source = (left_duration as f64 * (right_clip.source_fps / self.fps as f64)).round() as i64;
                right_clip.source_in_frame += frames_offset_in_source;

                self.clips[idx].duration_frames = left_duration;
                self.clips.insert(idx + 1, right_clip);
                return true;
            }
        }
        false
    }

    /// Разрезать все клипы под плейхедом (Ctrl+B)
    pub fn blade_all_at_playhead(&mut self) {
        let playhead = self.playhead_frame;
        let intersecting_ids: Vec<u64> = self
            .clips
            .iter()
            .filter(|c| c.contains_frame(playhead))
            .map(|c| c.id)
            .collect();

        if !intersecting_ids.is_empty() {
            for id in intersecting_ids {
                self.blade_cut_at(id, playhead);
            }
        }
    }

    /// Удалить выделенный клип (Delete / Backspace)
    pub fn delete_selected_clip(&mut self) {
        if let Some(selected_id) = self.selected_clip_id {
            self.push_history();
            self.clips.retain(|c| c.id != selected_id);
            self.selected_clip_id = None;
        }
    }

    /// Перемещение клипа с учетом магнитного прилипания (Snapping)
    pub fn move_clip(&mut self, clip_id: u64, raw_start: i64) {
        let mut target_frame = raw_start.max(0);

        if self.snapping_enabled {
            let snap_threshold = (10.0 / self.zoom_factor) as i64;

            // Прилипание к плейхеду
            if (target_frame - self.playhead_frame).abs() <= snap_threshold {
                target_frame = self.playhead_frame;
            }

            // Прилипание к точкам других клипов
            for other in &self.clips {
                if other.id == clip_id {
                    continue;
                }
                if (target_frame - other.start_frame).abs() <= snap_threshold {
                    target_frame = other.start_frame;
                    break;
                }
                if (target_frame - other.end_frame()).abs() <= snap_threshold {
                    target_frame = other.end_frame();
                    break;
                }
            }
        }

        if let Some(clip) = self.clips.iter_mut().find(|c| c.id == clip_id) {
            clip.start_frame = target_frame;
        }
    }

    /// Шаг плейхеда при воспроизведении с точной остановкой в конце таймлайна
    pub fn step_playback(&mut self, delta_seconds: f64) {
        if self.is_playing {
            let dt = delta_seconds.min(0.1);
            self.frame_accumulator += dt * self.fps as f64 * self.playback_speed;

            let end_frame = self.out_point.unwrap_or_else(|| self.timeline_end_frame());

            if self.playback_speed >= 0.0 {
                if self.frame_accumulator >= 1.0 {
                    let frames = self.frame_accumulator.floor() as i64;
                    self.frame_accumulator -= frames as f64;
                    let next = self.playhead_frame + frames;

                    // Если дошли до конца клипов или Out-point
                    if end_frame > 0 && next >= end_frame {
                        if self.loop_enabled {
                            self.playhead_frame = self.in_point.unwrap_or(0);
                        } else {
                            // ОСТАНОВКА ВОСПРОИЗВЕДЕНИЯ В КОНЦЕ ТАЙМЛАЙНА
                            self.playhead_frame = end_frame;
                            self.is_playing = false;
                        }
                    } else {
                        self.playhead_frame = next.max(0);
                    }
                }
            } else if self.frame_accumulator <= -1.0 {
                let frames = self.frame_accumulator.ceil() as i64;
                self.frame_accumulator -= frames as f64;
                let start_bound = self.in_point.unwrap_or(0);
                let next = self.playhead_frame + frames;
                if next <= start_bound {
                    if self.loop_enabled && end_frame > 0 {
                        self.playhead_frame = end_frame;
                    } else {
                        self.playhead_frame = start_bound;
                        self.is_playing = false;
                    }
                } else {
                    self.playhead_frame = next.max(0);
                }
            }
        }
    }
}
