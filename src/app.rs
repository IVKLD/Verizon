use egui::{Context, Id, Margin, Panel, Stroke, Ui, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, Style as DockStyle, TabViewer};

use crate::audio_player::AudioPlayer;
use crate::inspector::InspectorState;
use crate::media_page::MediaPageState;
use crate::media_pool::MediaPoolState;
use crate::preferences::UserPreferences;
use crate::theme::{apply_resolve_theme, ResolveColors};
use crate::timeline::{TimelineState, Tool};
use crate::timeline_view::TimelineView;
use crate::video_decoder::VideoPlayer;
use crate::viewer::ViewerState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
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
                self.viewer.render_dual_viewers(ui, self.timeline, self.video_player, vu, &ctx);
            }
            Tab::Inspector => {
                self.inspector.render(ui, self.timeline);
            }
            Tab::Timeline => {
                TimelineView::render(ui, self.timeline, self.waveform_mgr);
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
    pub preferences: UserPreferences,
    pub show_preferences_window: bool,
    pub last_frame_time: std::time::Instant,
    pub notification: Option<(String, std::time::Instant)>,
}

impl NleApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_resolve_theme(&cc.egui_ctx);

        let prefs = UserPreferences::load();
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
            preferences: prefs.clone(),
            show_preferences_window: false,
            last_frame_time: std::time::Instant::now(),
            notification: None,
        };

        app.timeline.snapping_enabled = prefs.snapping_enabled;
        app.timeline.linked_selection = prefs.linked_selection;
        app.timeline.loop_enabled = prefs.loop_enabled;

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

    fn handle_drag_and_drop(&mut self, ctx: &Context) {
        // 1. Прямой сброс файлов из файлового менеджера Nautilus (Drag & Drop)
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped_files.is_empty() {
            let mut total_imported = 0;
            for file in dropped_files {
                let path = file.path();
                let count = self.media_pool.import_path(path);
                total_imported += count;

                // СРАЗУ ПОМЕЩАЕМ СБРОШЕННЫЙ ФАЙЛ НА ДОРОЖКУ ТАЙМЛАЙНА!
                self.media_pool.insert_selected_to_timeline(&mut self.timeline);

                if MediaPoolState::is_media_file(path) {
                    let _ = self.video_player.open_file(path, ctx);
                    let _ = self.audio_player.load_audio_from_file(path);
                }
            }
            if total_imported > 0 {
                self.notification = Some((
                    format!("✅ Файл из Nautilus помещен на дорожку таймлайна (импортировано: {})", total_imported),
                    std::time::Instant::now(),
                ));
            }
        }

        // 2. Вставка из буфера обмена (Ctrl+V) файлов, скопированных в Nautilus
        for event in &ctx.input(|i| i.events.clone()) {
            if let egui::Event::Paste(text) = event {
                for line in text.lines() {
                    let clean = line.trim().trim_start_matches("file://");
                    let path = std::path::Path::new(clean);
                    if path.exists() {
                        let count = self.media_pool.import_path(path);
                        if count > 0 {
                            self.media_pool.insert_selected_to_timeline(&mut self.timeline);
                            if MediaPoolState::is_media_file(path) {
                                let _ = self.video_player.open_file(path, ctx);
                                let _ = self.audio_player.load_audio_from_file(path);
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
                    self.timeline.zoom_factor = (self.timeline.zoom_factor * 1.25).clamp(0.05, 50.0);
                } else if i.key_pressed(egui::Key::Minus) {
                    self.timeline.zoom_factor = (self.timeline.zoom_factor * 0.80).clamp(0.05, 50.0);
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

            // J / K / L Shuttle
            if i.key_pressed(egui::Key::J) {
                self.timeline.playback_speed = -2.0;
                self.timeline.is_playing = true;
            }
            if i.key_pressed(egui::Key::K) {
                self.timeline.is_playing = false;
            }
            if i.key_pressed(egui::Key::L) {
                self.timeline.playback_speed = 2.0;
                self.timeline.is_playing = true;
            }

            // Инструменты: A (Arrow), B (Blade)
            if i.key_pressed(egui::Key::A) {
                self.timeline.active_tool = Tool::Selection;
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

            // Delete selected clip
            if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
                self.timeline.delete_selected_clip();
            }

            // Undo: Ctrl + Z
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Z) {
                self.timeline.undo();
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

    fn render_preferences_window(&mut self, ctx: &Context) {
        if !self.show_preferences_window {
            return;
        }

        let mut open = self.show_preferences_window;
        let mut close_clicked = false;

        egui::Window::new("⚙ Настройки (Preferences)")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.heading(
                    egui::RichText::new("Масштабирование интерфейса (HiDPI / 4K)")
                        .color(ResolveColors::ACCENT_ORANGE)
                        .size(15.0),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Выберите подходящий масштаб под разрешение вашего экрана:")
                        .size(12.0)
                        .color(ResolveColors::TEXT_PRIMARY),
                );
                ui.add_space(8.0);

                let current_scale = self.preferences.ui_scale;
                let mut chosen_scale = current_scale;

                // Быстрые пресеты масштабирования для 1080p, 2K, 4K
                ui.horizontal_wrapped(|ui| {
                    let presets = [
                        (1.0, "100% (FHD 1080p)"),
                        (1.25, "125% (2K)"),
                        (1.5, "150% (QHD)"),
                        (1.75, "175% (3K)"),
                        (2.0, "200% (4K UHD)"),
                        (2.5, "250% (4K Ultra)"),
                    ];

                    for (scale, label) in presets {
                        let is_active = (current_scale - scale).abs() < 0.02;
                        let btn = egui::Button::new(label).fill(if is_active {
                            ResolveColors::ACCENT_ORANGE
                        } else {
                            ResolveColors::BG_HEADER
                        });
                        if ui.add(btn).clicked() {
                            chosen_scale = scale;
                        }
                    }
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // Плавный ползунок точной настройки
                ui.horizontal(|ui| {
                    ui.label("Точный масштаб:");
                    ui.add(
                        egui::Slider::new(&mut chosen_scale, 0.75..=3.5)
                            .step_by(0.05)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });

                if (chosen_scale - current_scale).abs() > 0.001 {
                    self.set_ui_scale(chosen_scale, ctx);
                }

                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("💡 Горячие клавиши масштабирования: Ctrl + / Ctrl - / Ctrl 0")
                        .size(11.0)
                        .color(ResolveColors::TEXT_MUTED),
                );

                ui.add_space(12.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Сбросить на 100%").clicked() {
                        self.set_ui_scale(1.0, ctx);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("✔ Закрыть").clicked() {
                            close_clicked = true;
                        }
                    });
                });
            });

        if close_clicked {
            open = false;
        }
        self.show_preferences_window = open;
    }
}

impl eframe::App for NleApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 0. Обработка очереди импорта из таймлайна
        let pending = std::mem::take(&mut self.timeline.pending_imports);
        for f in pending {
            self.media_pool.import_path(&f);
            self.media_pool.insert_selected_to_timeline(&mut self.timeline);
            let _ = self.video_player.open_file(&f, &ctx);
            let _ = self.audio_player.load_audio_from_file(&f);
        }

        // 1. Шаг воспроизведения по таймеру
        let now = std::time::Instant::now();
        let delta = now.duration_since(self.last_frame_time).as_secs_f64();
        self.last_frame_time = now;

        if self.timeline.is_playing {
            self.timeline.step_playback(delta);
            ctx.request_repaint(); // 144+ FPS перерисовка при активном воспроизведении
        }

        // 2. Синхронизация звука с видеоплейбэком активного аудиоклипа
        let active_audio_clip = self
            .timeline
            .clips
            .iter()
            .find(|c| c.is_audio && c.contains_frame(self.timeline.playhead_frame));

        if let Some(clip) = active_audio_clip {
            if let Some(path) = &clip.source_path {
                if self.audio_player.current_path.as_deref() != Some(path.as_path()) {
                    let _ = self.audio_player.load_audio_from_file(path);
                }
                let offset = self.timeline.playhead_frame - clip.start_frame;
                let time_in_clip = offset as f64 / self.timeline.fps as f64;
                let total_source_secs = clip.source_in_secs + time_in_clip;
                self.audio_player.sync_playhead(Some(total_source_secs), self.timeline.is_playing);
            } else {
                self.audio_player.sync_playhead(None, self.timeline.is_playing);
            }
        } else {
            self.audio_player.sync_playhead(None, self.timeline.is_playing);
        }

        // 3. Обработка Drag & Drop файлов и буфера обмена (Ctrl+V)
        self.handle_drag_and_drop(&ctx);

        // 4. Обработка хоткеев DaVinci
        self.handle_keyboard_shortcuts(&ctx);

        // 5. Верхний бар и Меню
        self.render_top_bar(ui);

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
                };

                DockArea::new(&mut self.dock_state)
                    .style(dock_style)
                    .draggable_tabs(true)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_inside(ui, &mut tab_viewer);
            }
        }

        // 8. Модальное окно настроек (Масштабирование UI для 4K)
        self.render_preferences_window(&ctx);
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_preferences();
        self.save_dock_layout();
    }
}
