use egui::{Context, Id, Margin, Panel, Stroke, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, Style as DockStyle, TabViewer};
use std::path::PathBuf;

use crate::audio_player::AudioPlayer;
use crate::inspector::InspectorState;
use crate::media_page::MediaPageState;
use crate::media_pool::MediaPoolState;
use crate::preferences::UserPreferences;
use crate::theme::ResolveColors;
use crate::timeline::{TimelineState, Tool};
use crate::timeline_view::TimelineView;
use crate::video_decoder::VideoPlayer;
use crate::viewer::ViewerState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Media,
    Cut,
    Edit,
    Color,
    Deliver,
}

use serde::{Deserialize, Serialize};
use crate::waveform::WaveformManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    MediaPool,
    Viewers,
    Inspector,
    Timeline,
}

struct ResolveTabViewer<'a> {
    media_pool: &'a mut MediaPoolState,
    viewer: &'a mut ViewerState,
    inspector: &'a mut InspectorState,
    timeline: &'a mut TimelineState,
    video_player: &'a mut VideoPlayer,
    audio_player: &'a mut AudioPlayer,
    waveform_mgr: &'a WaveformManager,
    render_cache: &'a mut crate::render_cache::RenderCacheManager,
}

impl<'a> TabViewer for ResolveTabViewer<'a> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Self::Tab) -> Id {
        Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> WidgetText {
        match tab {
            Tab::MediaPool => "📁 Media Pool".into(),
            Tab::Viewers => "🖥 Viewers".into(),
            Tab::Inspector => "🔍 Inspector".into(),
            Tab::Timeline => "🎬 Timeline".into(),
        }
    }

    // Запрет закрытия критически важных панелей без явного сброса
    fn is_closeable(&self, _tab: &Self::Tab) -> bool {
        false
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        let ctx = ui.ctx().clone();
        match tab {
            Tab::MediaPool => {
                self.media_pool.render(ui, self.timeline);
            }
            Tab::Viewers => {
                let vu = self.audio_player.get_vu_levels();
                self.viewer.render_dual_viewers(ui, self.timeline, self.video_player, self.render_cache, vu, &ctx);
            }
            Tab::Inspector => {
                self.inspector.render(ui, self.timeline);
            }
            Tab::Timeline => {
                TimelineView::render(ui, self.timeline, self.waveform_mgr, self.render_cache);
            }
        }
    }
}

pub struct NleApp {
    pub current_page: Page,
    pub dock_state: DockState<Tab>,
    pub media_pool: MediaPoolState,
    pub media_page: MediaPageState,
    pub viewer: ViewerState,
    pub inspector: InspectorState,
    pub timeline: TimelineState,
    pub video_player: VideoPlayer,
    pub audio_player: AudioPlayer,
    pub waveform_mgr: WaveformManager,
    pub render_cache: crate::render_cache::RenderCacheManager,
    pub preferences: UserPreferences,
    pub preferences_modal: crate::preferences::PreferencesModalState,
    pub show_preferences_window: bool,
    pub last_frame_time: std::time::Instant,
    pub notification: Option<(String, std::time::Instant)>,
    /// Самопроверка реального приложения (включается переменной окружения VERIZON_AUTOPILOT)
    pub autopilot: Option<crate::autopilot::Autopilot>,
}

impl NleApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs = UserPreferences::load();
        crate::theme::apply_resolve_theme_with_accent(&cc.egui_ctx, prefs.theme_accent_idx);
        cc.egui_ctx.set_zoom_factor(prefs.ui_scale);

        let mut app = Self {
            current_page: Page::Edit,
            dock_state: DockState::new(vec![Tab::Viewers]),
            media_pool: MediaPoolState::new_demo(),
            media_page: MediaPageState::new(),
            viewer: ViewerState::new(),
            inspector: InspectorState::new(),
            timeline: TimelineState::new_empty(),
            video_player: VideoPlayer::new(),
            audio_player: AudioPlayer::new(),
            waveform_mgr: WaveformManager::new(),
            render_cache: crate::render_cache::RenderCacheManager::default(),
            preferences: prefs.clone(),
            preferences_modal: crate::preferences::PreferencesModalState::new(),
            show_preferences_window: false,
            last_frame_time: std::time::Instant::now(),
            notification: None,
            autopilot: crate::autopilot::Autopilot::from_env(),
        };

        app.timeline.snapping_enabled = prefs.snapping_enabled;
        app.timeline.snapping_threshold_px = prefs.snapping_threshold_px;
        app.timeline.linked_selection = prefs.linked_selection;
        app.timeline.loop_enabled = prefs.loop_enabled;
        app.timeline.stop_at_timeline_end = prefs.stop_at_timeline_end;

        app.render_cache.mode = match prefs.render_cache_mode {
            0 => crate::render_cache::RenderCacheMode::None,
            1 => crate::render_cache::RenderCacheMode::Smart,
            _ => crate::render_cache::RenderCacheMode::User,
        };
        app.render_cache.set_max_memory_mb(prefs.render_cache_ram_mb);
        app.video_player.drop_frames_enabled = prefs.drop_frames_enabled;
        app.audio_player.set_master_volume(prefs.master_volume);

        // Восстановление сохраненного расположения окон (Dock Layout)
        let dock_path = UserPreferences::dock_layout_path();
        let loaded_dock = if let Ok(json) = std::fs::read_to_string(&dock_path) {
            serde_json::from_str::<DockState<Tab>>(&json).ok()
        } else {
            None
        };

        if let Some(dock) = loaded_dock {
            app.dock_state = dock;
        } else {
            app.reset_dock_layout();
        }

        app
    }

    pub fn save_preferences(&mut self) {
        self.preferences.snapping_enabled = self.timeline.snapping_enabled;
        self.preferences.linked_selection = self.timeline.linked_selection;
        self.preferences.loop_enabled = self.timeline.loop_enabled;
        self.preferences.save();
    }

    pub fn save_dock_layout(&self) {
        let path = UserPreferences::dock_layout_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.dock_state) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn set_ui_scale(&mut self, new_scale: f32, ctx: &Context) {
        let clamped = new_scale.clamp(0.5, 4.0);
        self.preferences.ui_scale = (clamped * 100.0).round() / 100.0;
        ctx.set_zoom_factor(self.preferences.ui_scale);
        self.save_preferences();
    }

    pub fn prompt_import_media(&mut self, ctx: &Context) {
        if let Some(files) = rfd::FileDialog::new()
            .add_filter("Media Files", &["mp4", "mov", "mkv", "avi", "webm", "wav", "mp3", "flac"])
            .pick_files()
        {
            let mut count = 0;
            for f in files {
                let imported = self.media_pool.import_path(&f);
                if imported > 0 {
                    self.media_pool.insert_selected_to_timeline(&mut self.timeline);
                    self.waveform_mgr.mark_pending_streams(&f, 8);
                    let _ = self.video_player.open_file(&f, ctx);
                    let _ = self.audio_player.load_audio_from_file(&f);
                    count += 1;
                }
            }
            if count > 0 {
                self.notification = Some((
                    format!("✅ Импортировано клипов на таймлайн: {}", count),
                    std::time::Instant::now(),
                ));
            }
        }
    }

    /// Сброс расположения окон к дефолтному стандарту DaVinci Resolve:
    /// Media слева, Viewers по центру, Inspector справа от предпросмотра, Timeline снизу во всю ширину
    pub fn reset_dock_layout(&mut self) {
        let mut dock = DockState::new(vec![Tab::Viewers]);

        // 1. Снизу во всю ширину: Timeline
        let [top, _bottom] = dock.main_surface_mut().split_below(
            NodeIndex::root(),
            0.55,
            vec![Tab::Timeline],
        );

        // 2. Слева сверху: Media Pool (Media слева)
        let [_left, center] = dock.main_surface_mut().split_left(
            top,
            0.22,
            vec![Tab::MediaPool],
        );

        // 3. Справа сверху: Inspector (Inspector строго справа от Viewers/предпросмотра)
        let [_center, _right] = dock.main_surface_mut().split_right(
            center,
            0.70,
            vec![Tab::Inspector],
        );

        self.dock_state = dock;
        self.save_dock_layout();
    }

    fn decode_uri_path(raw: &str) -> Option<PathBuf> {
        let clean = raw.trim().trim_start_matches("file://");
        let decoded = Self::percent_decode(clean);
        let p_dec = PathBuf::from(&decoded);
        if p_dec.exists() {
            return Some(p_dec);
        }
        let p_raw = PathBuf::from(clean);
        if p_raw.exists() {
            return Some(p_raw);
        }
        None
    }

    fn percent_decode(input: &str) -> String {
        let mut bytes = Vec::new();
        let mut chars = input.bytes();
        while let Some(b) = chars.next() {
            if b == b'%' {
                if let (Some(h1), Some(h2)) = (chars.next(), chars.next()) {
                    if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&[h1, h2]).unwrap_or(""), 16) {
                        bytes.push(val);
                        continue;
                    }
                }
            }
            bytes.push(b);
        }
        String::from_utf8(bytes).unwrap_or_else(|_| input.to_string())
    }

    fn handle_drag_and_drop(&mut self, ctx: &Context) {
        // 1. Прямой сброс файлов из файлового менеджера Nautilus (Drag & Drop)
        let has_dropped = ctx.input(|i| !i.raw.dropped_files.is_empty());
        if has_dropped {
            let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
            let mut total_imported = 0;
            for file in dropped_files {
                let p = file.path();
                let maybe_path = if !p.as_os_str().is_empty() {
                    if p.exists() {
                        Some(p.to_path_buf())
                    } else if let Some(decoded) = Self::decode_uri_path(&p.to_string_lossy()) {
                        Some(decoded)
                    } else {
                        None
                    }
                } else if let Ok(bytes) = file.bytes() {
                    if let Ok(text) = std::str::from_utf8(&bytes) {
                        Self::decode_uri_path(text)
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(path) = maybe_path {
                    let count = self.media_pool.import_path(&path);
                    total_imported += count;

                    // СРАЗУ ПОМЕЩАЕМ СБРОШЕННЫЙ ФАЙЛ НА ДОРОЖКУ ТАЙМЛАЙНА!
                    self.media_pool.insert_selected_to_timeline(&mut self.timeline);

                    if MediaPoolState::is_media_file(&path) {
                        self.waveform_mgr.mark_pending_streams(&path, 8);
                        let _ = self.video_player.open_file(&path, ctx);
                        let _ = self.audio_player.load_audio_from_file(&path);
                    }
                }
            }
            if total_imported > 0 {
                self.notification = Some((
                    format!("✅ Файл из Nautilus помещен на дорожку таймлайна (импортировано: {})", total_imported),
                    std::time::Instant::now(),
                ));
            }
        }

        // 2. Вставка из буфера обмена (Ctrl+V) файлов, скопированных в Nautilus / Dolphin
        let paste_text = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Paste(text) = e {
                    Some(text.clone())
                } else {
                    None
                }
            })
        });
        if let Some(text) = paste_text {
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }
                if let Some(path) = Self::decode_uri_path(line) {
                    let count = self.media_pool.import_path(&path);
                    if count > 0 {
                        self.media_pool.insert_selected_to_timeline(&mut self.timeline);
                        if MediaPoolState::is_media_file(&path) {
                            self.waveform_mgr.mark_pending_streams(&path, 8);
                            let _ = self.video_player.open_file(&path, ctx);
                            let _ = self.audio_player.load_audio_from_file(&path);
                        }
                        self.notification = Some((
                            format!("✅ Вставлен клип на дорожку (Ctrl+V): {}", path.file_name().and_then(|n| n.to_str()).unwrap_or("")),
                            std::time::Instant::now(),
                        ));
                    }
                }
            }
        }
    }

    fn handle_keyboard_shortcuts(&mut self, ctx: &Context) {
        let mut import_requested = false;
        let mut scale_change: Option<f32> = None;
        let mut toggle_prefs = false;

        ctx.input(|i| {
            // Space: Play / Pause
            if i.key_pressed(egui::Key::Space) {
                self.timeline.is_playing = !self.timeline.is_playing;
                self.timeline.playback_speed = 1.0;
                if self.timeline.is_playing {
                    self.timeline.frame_accumulator = 0.0;
                    self.last_frame_time = std::time::Instant::now();
                } else {
                    self.video_player.pause();
                }
            }

            // Ctrl+I: Import Media
            if (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::I) {
                import_requested = true;
            }

            // Ctrl + , (Preferences / Настройки)
            if (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::Comma) {
                toggle_prefs = true;
            }

            // Масштабирование интерфейса для 4K: Ctrl + Alt + / Ctrl + Alt -
            if (i.modifiers.ctrl || i.modifiers.command) && i.modifiers.alt {
                if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                    scale_change = Some(self.preferences.ui_scale + 0.1);
                } else if i.key_pressed(egui::Key::Minus) {
                    scale_change = Some(self.preferences.ui_scale - 0.1);
                } else if i.key_pressed(egui::Key::Num0) {
                    scale_change = Some(1.0);
                }
            } else if (i.modifiers.ctrl || i.modifiers.command) && !i.modifiers.shift {
                // Ctrl + / Ctrl - в DaVinci Resolve: Зум таймлайна по плейхеду
                if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                    self.timeline.zoom_factor = (self.timeline.zoom_factor * 1.25).clamp(0.0005, 50.0);
                } else if i.key_pressed(egui::Key::Minus) {
                    self.timeline.zoom_factor = (self.timeline.zoom_factor * 0.80).clamp(0.0005, 50.0);
                }
            }

            // Shift + Z: Zoom to Fit (Показать весь проект целиком во всю ширину окна, как в DaVinci Resolve)
            if i.modifiers.shift && !i.modifiers.ctrl && i.key_pressed(egui::Key::Z) {
                let w = i.raw.screen_rect.map(|r| r.width()).unwrap_or(1280.0) * 0.70;
                self.timeline.zoom_to_fit(w);
            }

            // Навигация по таймлайну (Edit Points и Покадровый переход)
            if i.key_pressed(egui::Key::ArrowUp) {
                self.timeline.jump_to_prev_edit_point();
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                self.timeline.jump_to_next_edit_point();
            }
            if i.key_pressed(egui::Key::ArrowLeft) {
                let step = if i.modifiers.shift { self.timeline.fps as i64 } else { 1 };
                self.timeline.playhead_frame = (self.timeline.playhead_frame - step).max(0);
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                let step = if i.modifiers.shift { self.timeline.fps as i64 } else { 1 };
                self.timeline.playhead_frame = self.timeline.playhead_frame + step;
            }
            if i.key_pressed(egui::Key::Home) {
                self.timeline.playhead_frame = 0;
            }
            if i.key_pressed(egui::Key::End) {
                self.timeline.playhead_frame = self.timeline.timeline_end_frame();
            }

            // Alt + X: Сброс In / Out точек
            if i.modifiers.alt && i.key_pressed(egui::Key::X) {
                self.timeline.in_point = None;
                self.timeline.out_point = None;
            }

            // J / K / L Shuttle (Прогрессивный Shuttle в стиле DaVinci Resolve)
            if i.key_pressed(egui::Key::J) {
                if !self.timeline.is_playing {
                    self.timeline.is_playing = true;
                    self.timeline.playback_speed = -1.0;
                    self.timeline.frame_accumulator = 0.0;
                    self.last_frame_time = std::time::Instant::now();
                } else if self.timeline.playback_speed > 0.0 {
                    self.timeline.playback_speed = -1.0;
                } else if self.timeline.playback_speed == -1.0 {
                    self.timeline.playback_speed = -2.0;
                } else if self.timeline.playback_speed == -2.0 {
                    self.timeline.playback_speed = -4.0;
                } else if self.timeline.playback_speed <= -4.0 {
                    self.timeline.playback_speed = -8.0;
                }
            }
            if i.key_pressed(egui::Key::K) {
                self.timeline.is_playing = false;
                self.timeline.playback_speed = 1.0;
                self.video_player.pause();
            }
            if i.key_pressed(egui::Key::L) {
                if !self.timeline.is_playing {
                    self.timeline.is_playing = true;
                    self.timeline.playback_speed = 1.0;
                    self.timeline.frame_accumulator = 0.0;
                    self.last_frame_time = std::time::Instant::now();
                } else if self.timeline.playback_speed < 0.0 {
                    self.timeline.playback_speed = 1.0;
                } else if self.timeline.playback_speed == 1.0 {
                    self.timeline.playback_speed = 2.0;
                } else if self.timeline.playback_speed == 2.0 {
                    self.timeline.playback_speed = 4.0;
                } else if self.timeline.playback_speed >= 4.0 {
                    self.timeline.playback_speed = 8.0;
                }
            }

            // Инструменты: A (Arrow), T (Trim), B (Blade)
            if i.key_pressed(egui::Key::A) {
                self.timeline.active_tool = Tool::Selection;
            }
            if i.key_pressed(egui::Key::T) {
                self.timeline.active_tool = Tool::Trim;
            }
            if i.key_pressed(egui::Key::B) {
                if i.modifiers.ctrl {
                    self.timeline.blade_all_at_playhead();
                } else {
                    self.timeline.active_tool = Tool::Blade;
                }
            }

            // Enter: вставить выбранный клип из пула на таймлайн
            if i.key_pressed(egui::Key::Enter) {
                self.media_pool.insert_selected_to_timeline(&mut self.timeline);
            }

            // Snapping: N
            if i.key_pressed(egui::Key::N) {
                self.timeline.snapping_enabled = !self.timeline.snapping_enabled;
            }

            // Ctrl+Shift+L: Связность клипов (Linked Selection)
            if (i.modifiers.ctrl || i.modifiers.command) && i.modifiers.shift && i.key_pressed(egui::Key::L) {
                self.timeline.linked_selection = !self.timeline.linked_selection;
            }

            // In / Out: I / O (без модификаторов)
            if !i.modifiers.ctrl && !i.modifiers.command {
                if i.key_pressed(egui::Key::I) {
                    self.timeline.in_point = Some(self.timeline.playhead_frame);
                }
                if i.key_pressed(egui::Key::O) {
                    self.timeline.out_point = Some(self.timeline.playhead_frame);
                }
            }

            // Delete: Shift+Delete/Shift+Backspace = Ripple Delete (схлопывание пустоты)
            // Обычный Delete/Backspace = Lift (удаление с сохранением зазора)
            if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
                if i.modifiers.shift {
                    self.timeline.ripple_delete_selected_clip();
                } else {
                    self.timeline.delete_selected_clip();
                }
            }

            // Undo: Ctrl + Z
            if i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z) {
                self.timeline.undo();
                self.timeline.structure_dirty = true;
            }

            // Redo: Ctrl + Shift + Z или Ctrl + Y
            if (i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Z))
                || (i.modifiers.ctrl && i.key_pressed(egui::Key::Y))
            {
                self.timeline.redo();
                self.timeline.structure_dirty = true;
            }

            // Ctrl + R: Retime Controls (DaVinci Resolve Управление скоростью клипа)
            if (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::R) {
                if let Some(sel_id) = self.timeline.selected_clip_id {
                    if let Some(c) = self.timeline.clips.iter_mut().find(|c| c.id == sel_id) {
                        c.retime_controls_open = !c.retime_controls_open;
                    }
                }
            }

            // D: Отключить / Включить клип (DaVinci Resolve Disable/Enable Clip)
            if !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.command && i.key_pressed(egui::Key::D) {
                if let Some(sel_id) = self.timeline.selected_clip_id {
                    if let Some(c) = self.timeline.clips.iter_mut().find(|c| c.id == sel_id) {
                        c.is_disabled = !c.is_disabled;
                        self.timeline.structure_dirty = true;
                        self.timeline.audio_dirty = true;
                    }
                }
            }

            // Shift + V: Выделить клип под плейхедом
            if i.modifiers.shift && !i.modifiers.ctrl && i.key_pressed(egui::Key::V) {
                let playhead = self.timeline.playhead_frame;
                if let Some(c) = self.timeline.active_video_clip_at(playhead) {
                    self.timeline.selected_clip_id = Some(c.id);
                }
            }

            // Shift + Plus / Shift + =: Увеличить высоту дорожек клипов (как в DaVinci Resolve)
            if i.modifiers.shift && !i.modifiers.ctrl {
                if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                    let new_h = (self.timeline.track_height + 8.0).clamp(24.0, 160.0);
                    self.timeline.track_height = new_h;
                    for t in &mut self.timeline.tracks {
                        t.height = new_h;
                    }
                } else if i.key_pressed(egui::Key::Minus) {
                    let new_h = (self.timeline.track_height - 8.0).clamp(24.0, 160.0);
                    self.timeline.track_height = new_h;
                    for t in &mut self.timeline.tracks {
                        t.height = new_h;
                    }
                }
            }

            // Ctrl + /: Зациклить воспроизведение (Loop Playback)
            if (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::Slash) {
                self.timeline.loop_enabled = !self.timeline.loop_enabled;
            }

            // Ctrl + \: Разрезать все дорожки под плейхедом (Blade All)
            if (i.modifiers.ctrl || i.modifiers.command) && i.key_pressed(egui::Key::Backslash) {
                self.timeline.blade_all_at_playhead();
            }
        });

        if import_requested {
            self.prompt_import_media(ctx);
        }

        if toggle_prefs {
            self.show_preferences_window = !self.show_preferences_window;
        }

        if let Some(new_scale) = scale_change {
            self.set_ui_scale(new_scale, ctx);
        }
    }

    fn render_top_bar(&mut self, ui: &mut Ui) {
        Panel::top("top_header_panel")
            .frame(
                egui::Frame::new()
                    .fill(ResolveColors::BG_HEADER)
                    .stroke(Stroke::new(1.0, ResolveColors::BORDER))
                    .inner_margin(Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Логотип
                    ui.label(
                        egui::RichText::new("⚡ VERIZON NLE")
                            .size(13.0)
                            .strong()
                            .color(ResolveColors::ACCENT_ORANGE),
                    );

                    ui.separator();

                    // Верхнее главное меню DaVinci Resolve
                    ui.menu_button("File", |ui| {
                        if ui.button("📂 Import Media... [Ctrl+I]").clicked() {
                            self.prompt_import_media(ui.ctx());
                            ui.close();
                        }
                        if ui.button("📁 Import Folder...").clicked() {
                            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                                self.media_pool.import_path(&folder);
                            }
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("⚙ Preferences... [Ctrl+,]").clicked() {
                            self.show_preferences_window = true;
                            ui.close();
                        }
                    });

                    ui.menu_button("Edit", |ui| {
                        if ui.button("⮌ Undo [Ctrl+Z]").clicked() {
                            self.timeline.undo();
                            ui.close();
                        }
                        if ui.button("✂ Blade Cut at Playhead [Ctrl+B]").clicked() {
                            self.timeline.blade_all_at_playhead();
                            ui.close();
                        }
                        if ui.button("🗑 Delete Selected [Del]").clicked() {
                            self.timeline.delete_selected_clip();
                            ui.close();
                        }
                    });

                    ui.menu_button("Timeline", |ui| {
                        let play_str = if self.timeline.is_playing { "⏸ Pause [Space]" } else { "▶ Play [Space]" };
                        if ui.button(play_str).clicked() {
                            self.timeline.is_playing = !self.timeline.is_playing;
                            if self.timeline.is_playing {
                                self.timeline.frame_accumulator = 0.0;
                                self.last_frame_time = std::time::Instant::now();
                            } else {
                                self.video_player.pause();
                            }
                            ui.close();
                        }
                        if ui.button("[ I ] Mark In").clicked() {
                            self.timeline.in_point = Some(self.timeline.playhead_frame);
                            ui.close();
                        }
                        if ui.button("[ O ] Mark Out").clicked() {
                            self.timeline.out_point = Some(self.timeline.playhead_frame);
                            ui.close();
                        }
                        ui.separator();
                        let link_label = if self.timeline.linked_selection { "🔗 Linked Selection: ON [Ctrl+Shift+L]" } else { "🔗 Linked Selection: OFF [Ctrl+Shift+L]" };
                        if ui.button(link_label).clicked() {
                            self.timeline.linked_selection = !self.timeline.linked_selection;
                            ui.close();
                        }
                    });

                    ui.menu_button("Workspace", |ui| {
                        if ui.button("⟲ Сбросить расположение панелей к дефолту").clicked() {
                            self.reset_dock_layout();
                            self.notification = Some(("✅ Расположение окон сброшено к дефолту".to_string(), std::time::Instant::now()));
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("🔍 Масштаб интерфейса (4K / HiDPI)... [Ctrl+,]").clicked() {
                            self.show_preferences_window = true;
                            ui.close();
                        }
                    });

                    ui.separator();

                    // Кнопка быстрого сброса окон прямо на панели
                    if ui.button(egui::RichText::new("⟲ Сбросить окна").size(10.0)).on_hover_text("Вернуть исходное расположение всех панелей (Media, Viewers, Inspector, Timeline)").clicked() {
                        self.reset_dock_layout();
                    }

                    if let Some((msg, time)) = &self.notification {
                        if time.elapsed().as_secs() < 5 {
                            ui.separator();
                            ui.label(egui::RichText::new(msg).size(11.0).color(ResolveColors::ACCENT_ORANGE).strong());
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // 1. Кнопка открытия настроек
                        if ui.button("⚙ Настройки [Ctrl+,]").on_hover_text("Настройки масштабирования интерфейса и программы").clicked() {
                            self.show_preferences_window = true;
                        }

                        // 2. Быстрые кнопки масштабирования для 4K
                        if ui.button("➕").on_hover_text("Увеличить масштаб интерфейса [Ctrl +]").clicked() {
                            let s = self.preferences.ui_scale + 0.1;
                            self.set_ui_scale(s, ui.ctx());
                        }
                        ui.label(
                            egui::RichText::new(format!("{:.0}%", self.preferences.ui_scale * 100.0))
                                .strong()
                                .color(ResolveColors::ACCENT_ORANGE),
                        );
                        if ui.button("➖").on_hover_text("Уменьшить масштаб интерфейса [Ctrl -]").clicked() {
                            let s = self.preferences.ui_scale - 0.1;
                            self.set_ui_scale(s, ui.ctx());
                        }
                        ui.label(egui::RichText::new("🔍 UI:").size(11.0).color(ResolveColors::TEXT_MUTED));
                    });
                });
            });
    }

    fn render_bottom_page_bar(&mut self, ui: &mut Ui) {
        Panel::bottom("resolve_bottom_page_bar")
            .frame(
                egui::Frame::new()
                    .fill(ResolveColors::BG_MAIN)
                    .stroke(Stroke::new(1.0, ResolveColors::BORDER))
                    .inner_margin(Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Статус Wayland слева
                    ui.label(
                        egui::RichText::new("WAYLAND NATIVE")
                            .size(9.0)
                            .monospace()
                            .color(ResolveColors::TEXT_MUTED),
                    );

                    ui.separator();

                    // Центрированные кнопки рабочих пространств DaVinci Resolve 21
                    ui.add_space((ui.available_width() * 0.28).max(0.0));

                    let pages = [
                        (Page::Media, "📁 Media"),
                        (Page::Cut, "✂ Cut"),
                        (Page::Edit, "🎞 Edit"),
                        (Page::Color, "🎨 Color"),
                        (Page::Deliver, "🚀 Deliver"),
                    ];

                    for (p, label) in pages {
                        let is_active = self.current_page == p;
                        let text = egui::RichText::new(label)
                            .size(11.0)
                            .color(if is_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_MUTED });
                        let btn = egui::Button::new(text)
                            .fill(if is_active { ResolveColors::BG_HEADER } else { egui::Color32::TRANSPARENT })
                            .stroke(if is_active { Stroke::new(1.0, ResolveColors::ACCENT_ORANGE.gamma_multiply(0.5)) } else { Stroke::NONE });
                        if ui.add(btn).clicked() {
                            self.current_page = p;
                        }
                    }

                    // Правая часть нижнего бара: таймкод проекта и кнопка настроек ⚙ как в DaVinci Resolve
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(egui::RichText::new("⚙").size(12.0))
                            .on_hover_text("Preferences / Настройки DaVinci Resolve [Ctrl+,]")
                            .clicked()
                        {
                            self.show_preferences_window = true;
                        }
                        ui.separator();
                        let tc = crate::timecode::Timecode::new(self.timeline.playhead_frame, self.timeline.fps).to_smpte_str();
                        ui.label(
                            egui::RichText::new(&*tc)
                                .size(10.0)
                                .monospace()
                                .color(ResolveColors::TEXT_PRIMARY),
                        );
                    });
                });
            });
    }

    /// Применить новые настройки ко всем подсистемам приложения и сохранить на диск
    pub fn apply_preferences(&mut self, prefs: UserPreferences, ctx: &Context) {
        self.preferences = prefs.clone();
        self.preferences.save();

        crate::theme::apply_resolve_theme_with_accent(ctx, self.preferences.theme_accent_idx);
        ctx.set_zoom_factor(self.preferences.ui_scale);

        self.timeline.snapping_enabled = self.preferences.snapping_enabled;
        self.timeline.snapping_threshold_px = self.preferences.snapping_threshold_px;
        self.timeline.linked_selection = self.preferences.linked_selection;
        self.timeline.loop_enabled = self.preferences.loop_enabled;
        self.timeline.stop_at_timeline_end = self.preferences.stop_at_timeline_end;

        self.render_cache.mode = match self.preferences.render_cache_mode {
            0 => crate::render_cache::RenderCacheMode::None,
            1 => crate::render_cache::RenderCacheMode::Smart,
            _ => crate::render_cache::RenderCacheMode::User,
        };
        self.render_cache.set_max_memory_mb(self.preferences.render_cache_ram_mb);

        self.video_player.drop_frames_enabled = self.preferences.drop_frames_enabled;
        self.audio_player.set_master_volume(self.preferences.master_volume);

        self.notification = Some((
            "✔ Настройки DaVinci Resolve 21 успешно сохранены и применены".to_string(),
            std::time::Instant::now(),
        ));
    }

    fn render_preferences_window(&mut self, ctx: &Context) {
        if !self.show_preferences_window {
            self.preferences_modal.initialized = false;
            return;
        }

        if !self.preferences_modal.initialized {
            self.preferences_modal.sync_from_saved(&self.preferences);
        }

        if let Some(applied) = self.preferences_modal.render(ctx, &mut self.show_preferences_window) {
            self.apply_preferences(applied, ctx);
        }
    }
}

impl eframe::App for NleApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Проверка изменения структуры таймлайна (разрезы, перемещения, удаление, Retime)
        if self.timeline.structure_dirty {
            self.render_cache.invalidate_all();
            self.video_player.flush_queues_on_cut();
            self.timeline.structure_dirty = false;
        }

        // 0. Обработка очереди импорта из таймлайна
        let pending = std::mem::take(&mut self.timeline.pending_imports);
        for f in pending {
            self.media_pool.import_path(&f);
            self.media_pool.insert_selected_to_timeline(&mut self.timeline);
            self.waveform_mgr.mark_pending_streams(&f, 8);
            let _ = self.video_player.open_file(&f, &ctx);
            let _ = self.audio_player.load_audio_from_file(&f);
        }

        // 1. Шаг воспроизведения по таймеру
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.last_frame_time).as_secs_f64();
        self.last_frame_time = now;

        if self.autopilot.is_none() && !ctx.input(|i| i.pointer.primary_down()) {
            self.timeline.is_scrubbing = false;
        }

        let is_prerolling = self.video_player.is_prerolling();
        if self.timeline.is_playing && is_prerolling {
            // Пре-ролл: держим плейхед, пока декодер не выдаст первый кадр, иначе кадры теряются сразу после старта
            self.timeline.frame_accumulator = 0.0;
            ctx.request_repaint();
        } else if self.timeline.is_playing && !self.timeline.is_scrubbing {
            self.timeline.step_playback(delta);
            ctx.request_repaint(); // 144+ FPS перерисовка при активном воспроизведении
        } else if self.timeline.is_scrubbing {
            self.timeline.frame_accumulator = 0.0;
            ctx.request_repaint();
        }

        // 2. Верхний бар меню и Нижний бар переключения страниц DaVinci Resolve
        self.render_top_bar(ui);
        self.render_bottom_page_bar(ui);

        // 6. Отображение активной страницы (Media / Edit)
        match self.current_page {
            Page::Media => {
                // Полноценная страница DaVinci Resolve 21 Media Page
                self.media_page.render(
                    ui,
                    &mut self.media_pool,
                    &mut self.video_player,
                    &ctx,
                );
            }
            Page::Edit | Page::Cut | Page::Color | Page::Deliver => {
                // Страница монтажа Edit Page
                let mut dock_style = DockStyle::from_egui(ui.style());
                dock_style.dock_area_padding = None;

                let mut tab_viewer = ResolveTabViewer {
                    media_pool: &mut self.media_pool,
                    viewer: &mut self.viewer,
                    inspector: &mut self.inspector,
                    timeline: &mut self.timeline,
                    video_player: &mut self.video_player,
                    audio_player: &mut self.audio_player,
                    waveform_mgr: &self.waveform_mgr,
                    render_cache: &mut self.render_cache,
                };

                DockArea::new(&mut self.dock_state)
                    .style(dock_style)
                    .draggable_tabs(true)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_inside(ui, &mut tab_viewer);
            }
        }

        // 7. Синхронизация Audio Mixer ПОСЛЕ обработки всех событий ввода, кликов и изменений таймлайна в текущем кадре
        if self.audio_player.has_new_decoded_audio() {
            self.audio_player.export_waveforms_to(&self.waveform_mgr);
            self.timeline.audio_dirty = true;
            ctx.request_repaint();
        }
        if self.timeline.audio_dirty {
            self.audio_player.rebuild_timeline_audio(&self.timeline);
            self.render_cache.invalidate_all();
            self.timeline.audio_dirty = false;
        }
        self.audio_player.sync_playhead_frame(
            self.timeline.playhead_frame,
            self.timeline.fps,
            self.timeline.is_playing && !self.timeline.is_scrubbing,
        );

        // 8. Автопилот самопроверки ПОСЛЕ рендера Viewers (чтобы замерять актуальный декодированный кадр текущего тика)
        let is_autopilot = self.autopilot.is_some();
        if let Some(mut ap) = self.autopilot.take() {
            ap.tick(self, &ctx);
            self.autopilot = Some(ap);
            if self.timeline.audio_dirty {
                self.audio_player.rebuild_timeline_audio(&self.timeline);
                self.render_cache.invalidate_all();
                self.timeline.audio_dirty = false;
            }
            self.audio_player.sync_playhead_frame(
                self.timeline.playhead_frame,
                self.timeline.fps,
                self.timeline.is_playing && !self.timeline.is_scrubbing,
            );
        }

        // 3. Обработка Drag & Drop и хоткеев (во время VERIZON_AUTOPILOT игнорируем внешние события клавиатуры/мыши пользователя)
        if !is_autopilot {
            self.handle_drag_and_drop(&ctx);
            self.handle_keyboard_shortcuts(&ctx);
            if self.timeline.structure_dirty {
                self.render_cache.invalidate_all();
                self.video_player.flush_queues_on_cut();
                self.timeline.structure_dirty = false;
            }
        }

        // 9. Модальное окно настроек (Масштабирование UI для 4K)
        self.render_preferences_window(&ctx);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_preferences();
        self.save_dock_layout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_uri_path_with_spaces() {
        let uri = "file:///mnt/kdenlive-cache/epic-movie/Replay%202026-10-13%2021-12-56.mkv";
        let path = NleApp::decode_uri_path(uri);
        assert!(path.is_some(), "URI with %20 must decode to valid path");
        let p = path.unwrap();
        assert_eq!(p.to_str().unwrap(), "/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        assert!(p.exists(), "Target file on disk must exist");
    }
}
