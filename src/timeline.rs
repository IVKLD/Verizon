use crate::theme::ResolveColors;
use egui::Color32;

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
    pub is_audio: bool,
    pub duration_frames: i64,
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

    // История для Undo
    pub history: Vec<Vec<Clip>>,
}

impl TimelineState {
    pub fn new_demo() -> Self {
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

        let clips = vec![
            // V1 Клипы (Основная камера A-Roll)
            Clip {
                id: 1,
                name: "A001_C001_0422.MOV".to_string(),
                track_index: 1, // V1
                start_frame: 0,
                duration_frames: 120, // 5 секунд при 24fps
                is_audio: false,
                color: ResolveColors::VIDEO_CLIP,
                zoom_x: 1.0,
                zoom_y: 1.0,
                position_x: 0.0,
                position_y: 0.0,
                rotation: 0.0,
                opacity: 100.0,
                volume_db: 0.0,
                pan: 0.0,
            },
            Clip {
                id: 2,
                name: "A001_C002_0422.MOV".to_string(),
                track_index: 1, // V1
                start_frame: 120,
                duration_frames: 168, // 7 секунд
                is_audio: false,
                color: ResolveColors::VIDEO_CLIP,
                zoom_x: 1.0,
                zoom_y: 1.0,
                position_x: 0.0,
                position_y: 0.0,
                rotation: 0.0,
                opacity: 100.0,
                volume_db: 0.0,
                pan: 0.0,
            },
            Clip {
                id: 3,
                name: "A001_C005_0422.MOV".to_string(),
                track_index: 1, // V1
                start_frame: 288,
                duration_frames: 192, // 8 секунд
                is_audio: false,
                color: ResolveColors::VIDEO_CLIP,
                zoom_x: 1.0,
                zoom_y: 1.0,
                position_x: 0.0,
                position_y: 0.0,
                rotation: 0.0,
                opacity: 100.0,
                volume_db: 0.0,
                pan: 0.0,
            },
            // V2 Клип (B-Roll перебивка)
            Clip {
                id: 4,
                name: "B_ROLL_DRONE_4K.MP4".to_string(),
                track_index: 0, // V2
                start_frame: 72,
                duration_frames: 96,
                is_audio: false,
                color: Color32::from_rgb(0x3a, 0x6e, 0x9f),
                zoom_x: 1.05,
                zoom_y: 1.05,
                position_x: 0.0,
                position_y: 0.0,
                rotation: 0.0,
                opacity: 100.0,
                volume_db: 0.0,
                pan: 0.0,
            },
            // A1 Клипы (Голос / Диалог)
            Clip {
                id: 5,
                name: "VOICE_MIC_01.WAV".to_string(),
                track_index: 2, // A1
                start_frame: 0,
                duration_frames: 288,
                is_audio: true,
                color: ResolveColors::AUDIO_CLIP,
                zoom_x: 1.0,
                zoom_y: 1.0,
                position_x: 0.0,
                position_y: 0.0,
                rotation: 0.0,
                opacity: 100.0,
                volume_db: -1.5,
                pan: 0.0,
            },
            // A2 Клип (Фоновый саундтрек)
            Clip {
                id: 6,
                name: "CINEMATIC_OST_AMBIENT.WAV".to_string(),
                track_index: 3, // A2
                start_frame: 0,
                duration_frames: 480, // 20 секунд
                is_audio: true,
                color: Color32::from_rgb(0x27, 0x75, 0x50),
                zoom_x: 1.0,
                zoom_y: 1.0,
                position_x: 0.0,
                position_y: 0.0,
                rotation: 0.0,
                opacity: 100.0,
                volume_db: -12.0,
                pan: 0.0,
            },
        ];

        Self {
            tracks,
            clips,
            next_clip_id: 10,
            playhead_frame: 48, // 2 секунды
            fps: 24,
            zoom_factor: 2.0, // 1 кадр = 2px
            scroll_x: 0.0,
            in_point: None,
            out_point: None,
            loop_enabled: false,
            active_tool: Tool::Selection,
            snapping_enabled: true,
            selected_clip_id: Some(1),
            is_playing: false,
            playback_speed: 1.0,
            frame_accumulator: 0.0,
            dragged_media: None,
            history: Vec::new(),
        }
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

    /// Шаг плейхеда при воспроизведении с точной привязкой к реальному времени
    pub fn step_playback(&mut self, delta_seconds: f64) {
        if self.is_playing {
            let dt = delta_seconds.min(0.1);
            self.frame_accumulator += dt * self.fps as f64 * self.playback_speed;

            if self.playback_speed >= 0.0 {
                if self.frame_accumulator >= 1.0 {
                    let frames = self.frame_accumulator.floor() as i64;
                    self.frame_accumulator -= frames as f64;
                    let mut next = self.playhead_frame + frames;

                    // Зацикливание ТОЛЬКО если включен флаг loop_enabled и задан Out-point
                    if self.loop_enabled {
                        if let Some(out_p) = self.out_point {
                            if next >= out_p {
                                next = self.in_point.unwrap_or(0);
                            }
                        }
                    }

                    self.playhead_frame = next.max(0);
                }
            } else if self.frame_accumulator <= -1.0 {
                let frames = self.frame_accumulator.ceil() as i64;
                self.frame_accumulator -= frames as f64;
                self.playhead_frame = (self.playhead_frame + frames).max(0);
            }
        }
    }
}
