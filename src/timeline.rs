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

    // Ссылка на реальный медиафайл и смещение
    pub source_path: Option<PathBuf>,
    pub source_in_frame: i64,
    pub source_in_secs: f64,
    pub source_fps: f64,

    // Связанность клипов (Linked Clips: Video + Audio)
    pub link_group: Option<u64>,

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
    pub has_video: bool,
    pub has_audio: bool,
    pub duration_frames: i64,
    pub source_fps: f64,
    pub link_group: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct TimelineState {
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub next_clip_id: u64,
    pub next_link_id: u64,

    pub playhead_frame: i64,
    pub fps: u32,
    pub zoom_factor: f32, // Пикселей на кадр
    pub scroll_x: f32,

    pub in_point: Option<i64>,
    pub out_point: Option<i64>,
    pub loop_enabled: bool, // Флаг зацикливания воспроизведения

    pub active_tool: Tool,
    pub snapping_enabled: bool,
    pub linked_selection: bool, // Связность клипов (Linked Selection Ctrl+Shift+L)
    pub selected_clip_id: Option<u64>,
    pub is_playing: bool,
    pub playback_speed: f64, // 1.0 = нормальная, 2.0 = 2x (L), -1.0 = реверс (J)
    pub frame_accumulator: f64,

    // Активно перетаскиваемый клип (Drag & Drop)
    pub dragged_media: Option<DraggedMedia>,

    // Текущая сессия перемещения клипа на таймлайне: (clip_id, mouse_start_x, clip_initial_start_frame)
    pub moving_clip: Option<(u64, f32, i64)>,

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
            next_link_id: 1,
            playhead_frame: 0,
            fps: 24,
            zoom_factor: 2.0,
            scroll_x: 0.0,
            in_point: None,
            out_point: None,
            loop_enabled: false,
            active_tool: Tool::Selection,
            snapping_enabled: true,
            linked_selection: true, // По умолчанию связность включена как в Resolve
            selected_clip_id: None,
            is_playing: false,
            playback_speed: 1.0,
            frame_accumulator: 0.0,
            dragged_media: None,
            moving_clip: None,
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

    /// Разрезать клип лезвием (Blade Cut) в указанном кадре со связностью
    pub fn blade_cut_at(&mut self, clip_id: u64, cut_frame: i64) -> bool {
        let clip_opt = self.clips.iter().find(|c| c.id == clip_id).cloned();
        let clip = match clip_opt {
            Some(c) => c,
            None => return false,
        };

        // Если включена связность клипов, находим все клипы в группе (Video + Audio)
        let target_ids: Vec<u64> = if self.linked_selection && clip.link_group.is_some() {
            self.clips
                .iter()
                .filter(|c| c.link_group == clip.link_group && cut_frame > c.start_frame && cut_frame < c.end_frame())
                .map(|c| c.id)
                .collect()
        } else if cut_frame > clip.start_frame && cut_frame < clip.end_frame() {
            vec![clip.id]
        } else {
            Vec::new()
        };

        if target_ids.is_empty() {
            return false;
        }

        self.push_history();
        let new_link_group = if clip.link_group.is_some() {
            let id = self.next_link_id;
            self.next_link_id += 1;
            Some(id)
        } else {
            None
        };

        for id in target_ids {
            if let Some(idx) = self.clips.iter().position(|c| c.id == id) {
                let start = self.clips[idx].start_frame;
                let duration = self.clips[idx].duration_frames;
                let left_duration = cut_frame - start;
                let right_duration = duration - left_duration;

                let mut right_clip = self.clips[idx].clone();
                right_clip.id = self.next_clip_id;
                self.next_clip_id += 1;
                right_clip.start_frame = cut_frame;
                right_clip.duration_frames = right_duration;
                right_clip.link_group = new_link_group;

                let frames_offset = if self.fps > 0 {
                    (left_duration as f64 * (right_clip.source_fps / self.fps as f64)).round() as i64
                } else {
                    left_duration
                };
                right_clip.source_in_frame += frames_offset;

                let secs_offset = if self.fps > 0 {
                    left_duration as f64 / self.fps as f64
                } else {
                    0.0
                };
                right_clip.source_in_secs += secs_offset;

                self.clips[idx].duration_frames = left_duration;
                self.clips.insert(idx + 1, right_clip);
            }
        }
        true
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

    /// Удалить выделенный клип (Delete / Backspace) с учетом связности
    pub fn delete_selected_clip(&mut self) {
        if let Some(selected_id) = self.selected_clip_id {
            self.push_history();
            let link_grp = self.clips.iter().find(|c| c.id == selected_id).and_then(|c| c.link_group);
            if self.linked_selection && link_grp.is_some() {
                self.clips.retain(|c| c.link_group != link_grp);
            } else {
                self.clips.retain(|c| c.id != selected_id);
            }
            self.selected_clip_id = None;
        }
    }

    /// Перемещение клипа с учетом магнитного прилипания и связности клипов (Linked Clips)
    pub fn move_clip(&mut self, clip_id: u64, raw_start: i64) {
        let clip_idx = match self.clips.iter().position(|c| c.id == clip_id) {
            Some(idx) => idx,
            None => return,
        };

        let old_start = self.clips[clip_idx].start_frame;
        let mut target_frame = raw_start.max(0);

        if self.snapping_enabled {
            let snap_threshold = (10.0 / self.zoom_factor).max(2.0) as i64;

            // Прилипание к плейхеду
            if (target_frame - self.playhead_frame).abs() <= snap_threshold {
                target_frame = self.playhead_frame;
            }

            // Прилипание к границам других несвязанных клипов
            let current_link = self.clips[clip_idx].link_group;
            for other in &self.clips {
                if other.id == clip_id || (current_link.is_some() && other.link_group == current_link) {
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

        let delta_frames = target_frame - old_start;
        if delta_frames == 0 {
            return;
        }

        let link_grp = self.clips[clip_idx].link_group;
        if self.linked_selection && link_grp.is_some() {
            // Перемещаем ВСЕ связанные клипы (Видео + Аудио) синхронно!
            for clip in &mut self.clips {
                if clip.link_group == link_grp {
                    clip.start_frame = (clip.start_frame + delta_frames).max(0);
                }
            }
        } else {
            self.clips[clip_idx].start_frame = target_frame;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blade_cut_preserves_audio_continuity() {
        let mut tl = TimelineState::new_empty();
        tl.fps = 24;

        // Создаем видео и аудио клип (Linked group 1) от кадра 0 до 240 (10 секунд)
        tl.clips.push(Clip {
            id: 1,
            name: "Video".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 240,
            is_audio: false,
            color: Color32::BLUE,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 24.0,
            link_group: Some(1),
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
        });

        tl.clips.push(Clip {
            id: 2,
            name: "Audio".to_string(),
            track_index: 2,
            start_frame: 0,
            duration_frames: 240,
            is_audio: true,
            color: Color32::GREEN,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 24.0,
            link_group: Some(1),
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
        });

        // Режем на кадре 96 (ровно 4.0 секунды)
        assert!(tl.blade_cut_at(1, 96));

        // Должно стать 4 клипа: 2 левых и 2 правых
        assert_eq!(tl.clips.len(), 4);

        let left_audio = tl.clips.iter().find(|c| c.is_audio && c.start_frame == 0).unwrap();
        assert_eq!(left_audio.duration_frames, 96);
        assert_eq!(left_audio.source_in_secs, 0.0);

        let right_audio = tl.clips.iter().find(|c| c.is_audio && c.start_frame == 96).unwrap();
        assert_eq!(right_audio.duration_frames, 144);
        assert!((right_audio.source_in_secs - 4.0).abs() < 1e-6);

        // Проверяем непрерывность звука при переходе через точку реза
        // В кадре 95 (перед резом):
        let time_at_95 = left_audio.source_in_secs + (95 - left_audio.start_frame) as f64 / tl.fps as f64;
        // В кадре 96 (первый кадр правого клипа):
        let time_at_96 = right_audio.source_in_secs + (96 - right_audio.start_frame) as f64 / tl.fps as f64;

        let delta = time_at_96 - time_at_95;
        let expected_frame_time = 1.0 / 24.0;
        assert!((delta - expected_frame_time).abs() < 1e-6, "Audio must not restart at cut point!");
    }

    #[test]
    fn test_playback_stops_at_timeline_end() {
        let mut tl = TimelineState::new_empty();
        tl.fps = 24;
        tl.clips.push(Clip {
            id: 1,
            name: "Clip".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 48, // 2 секунды
            is_audio: false,
            color: Color32::BLUE,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 24.0,
            link_group: None,
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
        });

        tl.is_playing = true;
        tl.playhead_frame = 40;

        // Несколько шагов воспроизведения по 50мс до конца таймлайна
        for _ in 0..10 {
            tl.step_playback(0.05);
        }

        // Должно остановиться ровно на конце (48 кадрах), воспроизведение выключено
        assert_eq!(tl.playhead_frame, 48);
        assert!(!tl.is_playing);
    }

    #[test]
    fn test_linked_clip_movement() {
        let mut tl = TimelineState::new_empty();
        tl.fps = 24;
        tl.linked_selection = true;
        tl.snapping_enabled = false;

        tl.clips.push(Clip {
            id: 1,
            name: "Video".to_string(),
            track_index: 1,
            start_frame: 10,
            duration_frames: 50,
            is_audio: false,
            color: Color32::BLUE,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 24.0,
            link_group: Some(42),
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
        });

        tl.clips.push(Clip {
            id: 2,
            name: "Audio".to_string(),
            track_index: 2,
            start_frame: 10,
            duration_frames: 50,
            is_audio: true,
            color: Color32::GREEN,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 24.0,
            link_group: Some(42),
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
        });

        // Перемещаем только видеоклип на 30 кадр
        tl.move_clip(1, 30);

        // Оба связанных клипа должны переместиться на кадр 30
        assert_eq!(tl.clips[0].start_frame, 30);
        assert_eq!(tl.clips[1].start_frame, 30);
    }

    #[test]
    fn test_blade_cut_does_not_modify_playhead() {
        let mut tl = TimelineState::new_empty();
        tl.fps = 24;
        tl.playhead_frame = 50;

        tl.clips.push(Clip {
            id: 1,
            name: "Clip".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 100,
            is_audio: false,
            color: Color32::BLUE,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 24.0,
            link_group: None,
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
        });

        // Режем на кадре 25
        assert!(tl.blade_cut_at(1, 25));

        // Плейхед должен остаться строго на кадре 50!
        assert_eq!(tl.playhead_frame, 50);
        assert_eq!(tl.clips.len(), 2);
    }
}
