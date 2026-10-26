use egui::Color32;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Selection, // Клавиша 'A' (Стрелка для перемещения и выделения)
    Blade,     // Клавиша 'B' (Лезвие для нарезки клипов)
    Trim,      // Клавиша 'T' (Подрезка краев)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    Start,
    End,
}

#[derive(Debug, Clone, Copy)]
pub struct TrimSession {
    pub clip_id: u64,
    pub edge: TrimEdge,
    pub mouse_start_x: f32,
    pub init_start_frame: i64,
    pub init_duration_frames: i64,
    pub init_source_in_frame: i64,
    pub init_source_in_secs: f64,
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
    pub source_audio_stream: usize,

    // Управление скоростью и Retime Control в стиле DaVinci Resolve
    pub speed: f64,                 // 1.0 = 100%, 0.5 = 50%, 2.0 = 200%, -1.0 = Reverse
    pub retime_controls_open: bool, // Отображение панели Retime Controls на клипе (Ctrl+R)
    pub is_disabled: bool,          // Отключение / mute клипа ('D' в DaVinci Resolve)
}

impl Default for Clip {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            track_index: 0,
            start_frame: 0,
            duration_frames: 0,
            is_audio: false,
            color: Color32::from_rgb(0x2a, 0x64, 0x96),
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
            source_audio_stream: 0,
            speed: 1.0,
            retime_controls_open: false,
            is_disabled: false,
        }
    }
}

impl Clip {
    pub fn end_frame(&self) -> i64 {
        self.start_frame + self.duration_frames
    }

    pub fn contains_frame(&self, frame: i64) -> bool {
        frame >= self.start_frame && frame < self.end_frame()
    }

    /// Перевод глобального кадра таймлайна в локальный номер кадра исходного медиафайла с учетом скорости (Retime)
    pub fn source_frame_from_timeline(&self, timeline_frame: i64, timeline_fps: u32) -> i64 {
        let offset = timeline_frame - self.start_frame;
        let time_secs = if self.speed < 0.0 {
            let rev_offset = (self.duration_frames - 1 - offset).max(0);
            (rev_offset as f64 / timeline_fps.max(1) as f64) * self.speed.abs()
        } else {
            (offset as f64 / timeline_fps.max(1) as f64) * self.speed
        };
        let total_source_secs = self.source_in_secs + time_secs;
        let fps = if self.source_fps > 0.0 { self.source_fps } else { timeline_fps as f64 };
        (total_source_secs * fps).round() as i64
    }

    /// Обратный перевод номера кадра медиафайла в глобальный кадр таймлайна
    pub fn timeline_frame_from_source(&self, source_frame: i64, timeline_fps: u32) -> i64 {
        let fps = if self.source_fps > 0.0 { self.source_fps } else { timeline_fps as f64 };
        let total_source_secs = source_frame as f64 / fps.max(1.0);
        let time_secs = total_source_secs - self.source_in_secs;
        if self.speed < 0.0 {
            let rev_offset = (time_secs / self.speed.abs() * timeline_fps as f64).round() as i64;
            self.start_frame + (self.duration_frames - 1 - rev_offset).max(0)
        } else {
            let eff_speed = if self.speed.abs() > 0.001 { self.speed } else { 1.0 };
            self.start_frame + ((time_secs / eff_speed) * timeline_fps as f64).round() as i64
        }
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub is_video: bool,
    pub is_muted: bool,
    pub is_locked: bool,
    pub is_solo: bool,
    pub height: f32, // Высота дорожки в пикселях
}

#[derive(Debug, Clone)]
pub struct DraggedMedia {
    pub name: String,
    pub path: Option<PathBuf>,
    pub is_audio: bool,
    pub has_video: bool,
    pub has_audio: bool,
    pub audio_tracks_count: usize,
    pub duration_frames: i64,
    pub duration_secs: f64,
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
    pub scroll_y: f32,

    pub in_point: Option<i64>,
    pub out_point: Option<i64>,
    pub loop_enabled: bool, // Флаг зацикливания воспроизведения

    pub active_tool: Tool,
    pub snapping_enabled: bool,
    pub snapping_threshold_px: f32,
    pub stop_at_timeline_end: bool,
    pub linked_selection: bool, // Связность клипов (Linked Selection Ctrl+Shift+L)
    pub selected_clip_id: Option<u64>,
    pub is_playing: bool,
    pub is_scrubbing: bool,
    pub playback_speed: f64, // 1.0 = нормальная, 2.0 = 2x (L), -1.0 = реверс (J)
    pub frame_accumulator: f64,

    // Активно перетаскиваемый клип (Drag & Drop)
    pub dragged_media: Option<DraggedMedia>,

    // Текущая сессия перемещения клипа на таймлайне: (clip_id, mouse_start_x, mouse_start_y, clip_initial_start_frame, initial_track_index)
    pub moving_clip: Option<(u64, f32, f32, i64, usize)>,

    // Текущая сессия подрезки краев клипа (DaVinci Resolve Edge Trimming)
    pub trim_session: Option<TrimSession>,

    // Очередь импорта медиа
    pub pending_imports: Vec<PathBuf>,

    // История для Undo
    pub history: Vec<Vec<Clip>>,
    /// Стек повтора (Redo)
    pub redo_stack: Vec<Vec<Clip>>,

    // Флаг необходимости пересчета аудиомикса
    pub audio_dirty: bool,
    // Флаг изменения структуры таймлайна (разрез, удаление, перемещение, подрезка, скорость)
    pub structure_dirty: bool,

    // Высота дорожек и параметры отображения таймлайна (Timeline View Options)
    pub track_height: f32, // Глобальная высота дорожки по умолчанию (52.0 px)
    pub resizing_track: Option<(usize, f32, f32)>, // (индекс дорожки, стартовый Y курсора, начальная высота)
    pub retime_session: Option<(u64, f32, i64, f64)>, // (clip_id, mouse_start_x, initial_duration, initial_speed)
    pub show_view_options: bool, // Меню View Options на тулбаре
}

impl TimelineState {
    /// Получить активный видеоклип под кадром с учетом приоритета дорожек V2 > V1, отключения и выделения
    #[inline]
    pub fn active_video_clip_at(&self, frame: i64) -> Option<&Clip> {
        self.clips
            .iter()
            .filter(|c| !c.is_audio && !c.is_disabled && c.contains_frame(frame))
            .min_by_key(|c| (if self.selected_clip_id == Some(c.id) { 0usize } else { 1usize }, c.track_index))
    }

    pub fn track_height(&self, track_idx: usize) -> f32 {
        self.tracks.get(track_idx).map(|t| t.height).unwrap_or(self.track_height)
    }

    pub fn track_top_y(&self, track_idx: usize) -> f32 {
        let mut y = 0.0;
        for i in 0..track_idx.min(self.tracks.len()) {
            y += self.track_height(i);
        }
        y
    }

    pub fn total_tracks_height(&self) -> f32 {
        let mut y = 0.0;
        for i in 0..self.tracks.len() {
            y += self.track_height(i);
        }
        y
    }

    pub fn track_at_y(&self, rel_y: f32) -> usize {
        let mut accumulated = 0.0;
        for (i, t) in self.tracks.iter().enumerate() {
            accumulated += t.height;
            if rel_y < accumulated {
                return i;
            }
        }
        self.tracks.len().saturating_sub(1)
    }

    pub fn new_empty() -> Self {
        let default_h = 52.0;
        let tracks = vec![
            Track {
                name: "V2".to_string(),
                is_video: true,
                is_muted: false,
                is_locked: false,
                is_solo: false,
                height: default_h,
            },
            Track {
                name: "V1".to_string(),
                is_video: true,
                is_muted: false,
                is_locked: false,
                is_solo: false,
                height: default_h,
            },
            Track {
                name: "A1".to_string(),
                is_video: false,
                is_muted: false,
                is_locked: false,
                is_solo: false,
                height: default_h,
            },
            Track {
                name: "A2".to_string(),
                is_video: false,
                is_muted: false,
                is_locked: false,
                is_solo: false,
                height: default_h,
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
            scroll_y: 0.0,
            in_point: None,
            out_point: None,
            loop_enabled: false,
            active_tool: Tool::Selection,
            snapping_enabled: true,
            snapping_threshold_px: 10.0,
            stop_at_timeline_end: true,
            linked_selection: true, // По умолчанию связность включена как в Resolve
            selected_clip_id: None,
            is_playing: false,
            is_scrubbing: false,
            playback_speed: 1.0,
            frame_accumulator: 0.0,
            dragged_media: None,
            moving_clip: None,
            trim_session: None,
            pending_imports: Vec::new(),
            history: Vec::new(),
            redo_stack: Vec::new(),
            audio_dirty: true,
            structure_dirty: false,
            track_height: default_h,
            resizing_track: None,
            retime_session: None,
            show_view_options: false,
        }
    }

    /// Динамически гарантировать существование аудиодорожек на таймлайне (A1, A2, A3...)
    pub fn ensure_audio_track(&mut self, track_index: usize) {
        while self.tracks.len() <= track_index {
            let next_audio_num = self.tracks.iter().filter(|t| !t.is_video).count() + 1;
            self.tracks.push(Track {
                name: format!("A{}", next_audio_num),
                is_video: false,
                is_muted: false,
                is_locked: false,
                is_solo: false,
                height: self.track_height,
            });
        }
    }

    /// Добавить новую видеодорожку (V3, V4...)
    pub fn add_video_track(&mut self) {
        let video_count = self.tracks.iter().filter(|t| t.is_video).count();
        let new_track = Track {
            name: format!("V{}", video_count + 1),
            is_video: true,
            is_muted: false,
            is_locked: false,
            is_solo: false,
            height: self.track_height,
        };
        let first_audio = self.tracks.iter().position(|t| !t.is_video).unwrap_or(self.tracks.len());
        self.tracks.insert(first_audio, new_track);
        for c in &mut self.clips {
            if c.track_index >= first_audio {
                c.track_index += 1;
            }
        }
        self.audio_dirty = true;
    }

    /// Добавить новую аудиодорожку (A2, A3...)
    pub fn add_audio_track(&mut self) {
        let audio_count = self.tracks.iter().filter(|t| !t.is_video).count();
        self.tracks.push(Track {
            name: format!("A{}", audio_count + 1),
            is_video: false,
            is_muted: false,
            is_locked: false,
            is_solo: false,
            height: self.track_height,
        });
        self.audio_dirty = true;
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
        self.redo_stack.clear();
    }

    /// Отменить последнее действие (Ctrl+Z)
    pub fn undo(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.redo_stack.push(std::mem::replace(&mut self.clips, prev));
            self.audio_dirty = true;
        }
    }

    /// Повторить отменённое действие (Ctrl+Shift+Z / Ctrl+Y)
    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.history.push(std::mem::replace(&mut self.clips, next));
            self.audio_dirty = true;
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
        self.audio_dirty = true;
        self.structure_dirty = true;
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
            self.audio_dirty = true;
            self.structure_dirty = true;
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
            self.audio_dirty = true;
            self.structure_dirty = true;
        }
    }

    /// Удаление со сдвигом последующих клипов (DaVinci Resolve Ripple Delete [Shift+Delete / Shift+Backspace])
    pub fn ripple_delete_selected_clip(&mut self) {
        if let Some(selected_id) = self.selected_clip_id {
            self.push_history();

            // Определяем все удаляемые клипы (с учетом связи A/V)
            let link_grp = self.clips.iter().find(|c| c.id == selected_id).and_then(|c| c.link_group);
            let to_delete: Vec<Clip> = if self.linked_selection && link_grp.is_some() {
                self.clips.iter().filter(|c| c.link_group == link_grp).cloned().collect()
            } else {
                self.clips.iter().filter(|c| c.id == selected_id).cloned().collect()
            };

            if to_delete.is_empty() {
                return;
            }

            let min_start = to_delete.iter().map(|c| c.start_frame).min().unwrap_or(0);
            let max_end = to_delete.iter().map(|c| c.end_frame()).max().unwrap_or(0);
            let gap_duration = max_end - min_start;

            if gap_duration > 0 {
                // Удаляем клипы
                if self.linked_selection && link_grp.is_some() {
                    self.clips.retain(|c| c.link_group != link_grp);
                } else {
                    self.clips.retain(|c| c.id != selected_id);
                }

                // Сдвигаем все последующие клипы влево на ширину удаленного интервала
                for clip in &mut self.clips {
                    if clip.start_frame >= max_end {
                        clip.start_frame -= gap_duration;
                    }
                }

                // Корректируем плейхед
                if self.playhead_frame >= max_end {
                    self.playhead_frame = (self.playhead_frame - gap_duration).max(0);
                } else if self.playhead_frame > min_start {
                    self.playhead_frame = min_start;
                }
            }

            self.selected_clip_id = None;
            self.audio_dirty = true;
            self.structure_dirty = true;
        }
    }

    /// Перемещение клипа с учетом магнитного прилипания и связности клипов (Linked Clips)
    #[cfg(test)]
    pub fn move_clip(&mut self, clip_id: u64, raw_start: i64) {
        self.move_clip_to_track(clip_id, raw_start, None);
    }

    /// Перемещение клипа по времени и между дорожками (Tracks)
    pub fn move_clip_to_track(&mut self, clip_id: u64, raw_start: i64, new_track: Option<usize>) {
        let clip_idx = match self.clips.iter().position(|c| c.id == clip_id) {
            Some(idx) => idx,
            None => return,
        };

        let old_start = self.clips[clip_idx].start_frame;
        let mut target_frame = raw_start.max(0);

        if self.snapping_enabled {
            let snap_threshold = (self.snapping_threshold_px / self.zoom_factor).max(2.0) as i64;

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
        let link_grp = self.clips[clip_idx].link_group;

        // Перемещение по дорожкам (Track Index) с проверкой типа (видео на видео, аудио на аудио)
        if let Some(target_t) = new_track {
            if target_t < self.tracks.len() {
                let is_vid = !self.clips[clip_idx].is_audio;
                let track_is_vid = self.tracks[target_t].is_video;
                if is_vid == track_is_vid {
                    self.clips[clip_idx].track_index = target_t;
                }
            }
        }

        if delta_frames != 0 {
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
        self.audio_dirty = true;
        self.structure_dirty = true;
    }

    /// Подрезка начала клипа (Trim In) с привязкой к начальному состоянию сессии
    pub fn trim_clip_start_session(&mut self, session: &TrimSession, target_start: i64) {
        let clip_idx = match self.clips.iter().position(|c| c.id == session.clip_id) {
            Some(idx) => idx,
            None => return,
        };

        let link_grp = self.clips[clip_idx].link_group;
        // Нельзя подрезать левее начала исходного файла медиа
        let min_start = (session.init_start_frame - session.init_source_in_frame).max(0);
        let max_start = session.init_start_frame + session.init_duration_frames - 1;
        let clamped_start = target_start.clamp(min_start, max_start);
        let delta_frames = clamped_start - session.init_start_frame;

        let apply_trim = |clip: &mut Clip| {
            let fps = clip.source_fps.max(1.0);
            clip.start_frame = clamped_start;
            clip.duration_frames = session.init_duration_frames - delta_frames;
            clip.source_in_frame = session.init_source_in_frame + delta_frames;
            clip.source_in_secs = session.init_source_in_secs + (delta_frames as f64 / fps);
        };

        if self.linked_selection && link_grp.is_some() {
            for clip in &mut self.clips {
                if clip.link_group == link_grp {
                    apply_trim(clip);
                }
            }
        } else {
            apply_trim(&mut self.clips[clip_idx]);
        }

        self.audio_dirty = true;
        self.structure_dirty = true;
    }

    /// Подрезка конца клипа (Trim Out) с привязкой к начальному состоянию сессии
    pub fn trim_clip_end_session(&mut self, session: &TrimSession, target_end: i64) {
        let clip_idx = match self.clips.iter().position(|c| c.id == session.clip_id) {
            Some(idx) => idx,
            None => return,
        };

        let link_grp = self.clips[clip_idx].link_group;
        let min_end = session.init_start_frame + 1;
        let clamped_end = target_end.max(min_end);
        let new_duration = clamped_end - session.init_start_frame;

        if self.linked_selection && link_grp.is_some() {
            for clip in &mut self.clips {
                if clip.link_group == link_grp {
                    clip.duration_frames = new_duration;
                }
            }
        } else {
            self.clips[clip_idx].duration_frames = new_duration;
        }

        self.audio_dirty = true;
        self.structure_dirty = true;
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
                        } else if self.stop_at_timeline_end {
                            // ОСТАНОВКА ВОСПРОИЗВЕДЕНИЯ В КОНЦЕ ТАЙМЛАЙНА
                            self.playhead_frame = end_frame;
                            self.is_playing = false;
                        } else {
                            self.playhead_frame = next.max(0);
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

    /// Перейти к предыдущему стыку/резу клипов (Клавиша Up в Resolve)
    pub fn jump_to_prev_edit_point(&mut self) {
        let current = self.playhead_frame;
        let mut candidates = vec![0];
        for clip in &self.clips {
            if clip.start_frame < current {
                candidates.push(clip.start_frame);
            }
            if clip.end_frame() < current {
                candidates.push(clip.end_frame());
            }
        }
        if let Some(in_pt) = self.in_point {
            if in_pt < current {
                candidates.push(in_pt);
            }
        }
        if let Some(&prev) = candidates.iter().max() {
            self.playhead_frame = prev;
        }
    }

    /// Перейти к следующему стыку/резу клипов (Клавиша Down в Resolve)
    pub fn jump_to_next_edit_point(&mut self) {
        let current = self.playhead_frame;
        let end = self.timeline_end_frame();
        let mut candidates = Vec::new();
        for clip in &self.clips {
            if clip.start_frame > current {
                candidates.push(clip.start_frame);
            }
            if clip.end_frame() > current {
                candidates.push(clip.end_frame());
            }
        }
        if let Some(out_pt) = self.out_point {
            if out_pt > current {
                candidates.push(out_pt);
            }
        }
        if end > current {
            candidates.push(end);
        }
        if let Some(&next) = candidates.iter().min() {
            self.playhead_frame = next;
        }
    }

    /// Показать весь таймлайн целиком (Shift + Z в Resolve)
    pub fn zoom_to_fit(&mut self, available_width: f32) {
        let total_frames = self.timeline_end_frame().max(self.fps as i64 * 10);
        let optimal_zoom = (available_width / total_frames as f32).clamp(0.0005, 50.0);
        self.zoom_factor = optimal_zoom;
        self.scroll_x = 0.0;
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
            source_audio_stream: 0,
            ..Default::default()
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
            source_audio_stream: 0,
            ..Default::default()
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
            source_audio_stream: 0,
            ..Default::default()
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
            source_audio_stream: 0,
            ..Default::default()
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
            source_audio_stream: 0,
            ..Default::default()
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
            source_audio_stream: 0,
            ..Default::default()
        });

        // Режем на кадре 25
        assert!(tl.blade_cut_at(1, 25));

        // Плейхед должен остаться строго на кадре 50!
        assert_eq!(tl.playhead_frame, 50);
        assert_eq!(tl.clips.len(), 2);
    }

    #[test]
    fn test_ripple_delete_closes_gap_and_shifts_following_clips() {
        let mut tl = TimelineState::new_empty();
        tl.fps = 24;

        // Clip 1: [0..50)
        tl.clips.push(Clip {
            id: 1,
            name: "Clip 1".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 50,
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
            source_audio_stream: 0,
            ..Default::default()
        });

        // Clip 2: [50..100) (Будет удален через Ripple Delete)
        tl.clips.push(Clip {
            id: 2,
            name: "Clip 2".to_string(),
            track_index: 1,
            start_frame: 50,
            duration_frames: 50,
            is_audio: false,
            color: Color32::RED,
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
            source_audio_stream: 0,
            ..Default::default()
        });

        // Clip 3: [100..180)
        tl.clips.push(Clip {
            id: 3,
            name: "Clip 3".to_string(),
            track_index: 1,
            start_frame: 100,
            duration_frames: 80,
            is_audio: false,
            color: Color32::GREEN,
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
            source_audio_stream: 0,
            ..Default::default()
        });

        // Выделяем Clip 2 и делаем Ripple Delete
        tl.selected_clip_id = Some(2);
        tl.playhead_frame = 120;
        tl.ripple_delete_selected_clip();

        assert_eq!(tl.clips.len(), 2);
        // Clip 1 остался на месте [0..50)
        assert_eq!(tl.clips[0].id, 1);
        assert_eq!(tl.clips[0].start_frame, 0);

        // Clip 3 сдвинулся влево на 50 кадров: теперь начинается на кадре 50 вместо 100!
        assert_eq!(tl.clips[1].id, 3);
        assert_eq!(tl.clips[1].start_frame, 50);
        assert_eq!(tl.clips[1].duration_frames, 80);

        // Плейхед тоже сдвинулся со 120 на 70 (120 - 50)
        assert_eq!(tl.playhead_frame, 70);
    }

    #[test]
    fn test_trim_session_preserves_audio_sync_and_clamping() {
        let mut tl = TimelineState::new_empty();
        tl.fps = 24;
        tl.linked_selection = true;

        // Создаем связанную пару Video + Audio:
        // исходный файл уже отрезан на 24 кадра (1 сек)
        tl.clips.push(Clip {
            id: 10,
            name: "Video".to_string(),
            track_index: 1,
            start_frame: 100,
            duration_frames: 50,
            is_audio: false,
            color: Color32::BLUE,
            source_path: None,
            source_in_frame: 24,
            source_in_secs: 1.0,
            source_fps: 24.0,
            link_group: Some(77),
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
            source_audio_stream: 0,
            ..Default::default()
        });

        tl.clips.push(Clip {
            id: 11,
            name: "Audio".to_string(),
            track_index: 2,
            start_frame: 100,
            duration_frames: 50,
            is_audio: true,
            color: Color32::GREEN,
            source_path: None,
            source_in_frame: 24,
            source_in_secs: 1.0,
            source_fps: 24.0,
            link_group: Some(77),
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
            source_audio_stream: 0,
            ..Default::default()
        });

        let session = TrimSession {
            clip_id: 10,
            edge: TrimEdge::Start,
            mouse_start_x: 500.0,
            init_start_frame: 100,
            init_duration_frames: 50,
            init_source_in_frame: 24,
            init_source_in_secs: 1.0,
        };

        // Подрезаем на 10 кадров вперед (In-point = 110)
        tl.trim_clip_start_session(&session, 110);
        assert_eq!(tl.clips[0].start_frame, 110);
        assert_eq!(tl.clips[0].duration_frames, 40);
        assert_eq!(tl.clips[0].source_in_frame, 34);
        // Связанное аудио синхронно подрезалось:
        assert_eq!(tl.clips[1].start_frame, 110);
        assert_eq!(tl.clips[1].duration_frames, 40);
        assert_eq!(tl.clips[1].source_in_frame, 34);

        // Пытаемся подрезать левее начала исходного файла (In-point = 50, когда source_in_frame всего 24)
        tl.trim_clip_start_session(&session, 50);
        // Не может быть левее 100 - 24 = 76!
        assert_eq!(tl.clips[0].start_frame, 76);
        assert_eq!(tl.clips[0].source_in_frame, 0);
        assert_eq!(tl.clips[1].start_frame, 76);
        assert_eq!(tl.clips[1].source_in_frame, 0);
    }

    #[test]
    fn test_clip_bidirectional_frame_conversion() {
        let clip = Clip {
            id: 1,
            name: "TestClip".to_string(),
            track_index: 0,
            start_frame: 100,
            duration_frames: 300,
            is_audio: false,
            color: Color32::RED,
            source_path: None,
            source_in_frame: 120,
            source_in_secs: 2.0,
            source_fps: 60.0,
            link_group: None,
            zoom_x: 1.0,
            zoom_y: 1.0,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            opacity: 100.0,
            volume_db: 0.0,
            pan: 0.0,
            source_audio_stream: 0,
            ..Default::default()
        };

        let tl_fps = 60;
        // Кадр 160 на таймлайне (+1 сек от старта клипа на 100)
        let src_f = clip.source_frame_from_timeline(160, tl_fps);
        // source_in_secs (2.0) + 1.0 sec = 3.0 sec * 60 fps = 180
        assert_eq!(src_f, 180);

        // Обратный перевод: исходный кадр 180 -> таймлайн 160
        let back_tl_f = clip.timeline_frame_from_source(src_f, tl_fps);
        assert_eq!(back_tl_f, 160);
    }

    #[test]
    fn test_retime_controls_speed_and_reverse_conversion() {
        let mut clip = Clip {
            id: 1,
            name: "RetimeClip".to_string(),
            track_index: 0,
            start_frame: 100,
            duration_frames: 200,
            is_audio: false,
            color: Color32::BLUE,
            source_path: None,
            source_in_frame: 0,
            source_in_secs: 0.0,
            source_fps: 60.0,
            speed: 1.0,
            ..Default::default()
        };

        let tl_fps = 60;
        // 1. Нормальная скорость (100%): кадр 150 (+50 кадров) -> source 50
        assert_eq!(clip.source_frame_from_timeline(150, tl_fps), 50);

        // 2. Ускорение 2x (200%): кадр 150 (+50 кадров на таймлайне) -> source 100
        clip.speed = 2.0;
        assert_eq!(clip.source_frame_from_timeline(150, tl_fps), 100);

        // 3. Замедление 0.5x (50%): кадр 150 (+50 кадров на таймлайне) -> source 25
        clip.speed = 0.5;
        assert_eq!(clip.source_frame_from_timeline(150, tl_fps), 25);

        // 4. Реверс -1.0x (Reverse Speed): в начале клипа (кадр 100, смещение 0) -> последний кадр источника (кадр 199)
        clip.speed = -1.0;
        assert_eq!(clip.source_frame_from_timeline(100, tl_fps), 199);
        // В конце клипа (кадр 299, смещение 199) -> первый кадр источника (кадр 0)
        assert_eq!(clip.source_frame_from_timeline(299, tl_fps), 0);
    }

    #[test]
    fn test_variable_track_heights_and_y_lookup() {
        let mut tl = TimelineState::new_empty();
        tl.tracks.clear();
        tl.tracks.push(Track { name: "V1".to_string(), is_video: true, is_muted: false, is_locked: false, is_solo: false, height: 40.0 });
        tl.tracks.push(Track { name: "V2".to_string(), is_video: true, is_muted: false, is_locked: false, is_solo: false, height: 80.0 });
        tl.tracks.push(Track { name: "A1".to_string(), is_video: false, is_muted: false, is_locked: false, is_solo: false, height: 50.0 });

        assert_eq!(tl.track_height(0), 40.0);
        assert_eq!(tl.track_height(1), 80.0);
        assert_eq!(tl.track_height(2), 50.0);
        assert_eq!(tl.total_tracks_height(), 170.0);

        assert_eq!(tl.track_top_y(0), 0.0);
        assert_eq!(tl.track_top_y(1), 40.0);
        assert_eq!(tl.track_top_y(2), 120.0);

        // Поиск трека по Y координате
        assert_eq!(tl.track_at_y(10.0), 0);
        assert_eq!(tl.track_at_y(39.0), 0);
        assert_eq!(tl.track_at_y(40.0), 1);
        assert_eq!(tl.track_at_y(119.0), 1);
        assert_eq!(tl.track_at_y(120.0), 2);
        assert_eq!(tl.track_at_y(300.0), 2); // clamped to last track
    }

    #[test]
    fn test_disabled_clip_not_selected_for_video_playback() {
        let mut tl = TimelineState::new_empty();
        tl.tracks.push(Track { name: "V1".to_string(), is_video: true, is_muted: false, is_locked: false, is_solo: false, height: 52.0 });
        tl.tracks.push(Track { name: "V2".to_string(), is_video: true, is_muted: false, is_locked: false, is_solo: false, height: 52.0 });

        // Верхний клип на V2 (индекс 1)
        let clip_top = Clip {
            id: 10,
            name: "TopDisabled".to_string(),
            track_index: 0,
            start_frame: 0,
            duration_frames: 100,
            is_audio: false,
            color: Color32::RED,
            is_disabled: true, // ВЫКЛЮЧЕН (клавиша 'D')
            ..Default::default()
        };

        // Нижний клип на V1 (индекс 0)
        let clip_bottom = Clip {
            id: 20,
            name: "BottomActive".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 100,
            is_audio: false,
            color: Color32::BLUE,
            is_disabled: false,
            ..Default::default()
        };

        tl.clips.push(clip_top);
        tl.clips.push(clip_bottom);

        // Так как верхний клип отключен, активным видеоклипом должен стать нижний (id 20)
        let active = tl.active_video_clip_at(50).expect("should find active bottom clip");
        assert_eq!(active.id, 20);
    }
}
