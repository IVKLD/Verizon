use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use egui::{Color32, Context, CornerRadius, Rect, Stroke, Ui, Vec2};
use crate::theme::get_accent_colors;

/// Вкладки верхнего уровня (System / User) как в DaVinci Resolve 21
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefTab {
    System,
    User,
}

/// Категории настроек в стиле DaVinci Resolve 21 Preferences
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefSection {
    // SYSTEM PREFERENCES
    SystemMemoryGpu,
    SystemMediaStorage,
    SystemAudio,
    SystemHardware,

    // USER PREFERENCES
    UserAppearance,
    UserEditing,
    UserPlaybackCache,
    UserColorScopes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserPreferences {
    // 1. UI & Appearance
    pub ui_scale: f32,
    pub timecode_display_frames: bool, // false: SMPTE, true: Frames
    pub theme_accent_idx: usize,       // 0: DaVinci Orange, 1: Resolve Blue, 2: Studio Gold
    pub hide_titlebar: bool,           // Скрыть заголовок окна (Borderless)

    // 2. Editing & Timeline
    pub default_fps: u32,
    pub default_width: u32,
    pub default_height: u32,
    pub snapping_enabled: bool,
    pub snapping_threshold_px: f32,
    pub linked_selection: bool,
    pub loop_enabled: bool,
    pub stop_at_timeline_end: bool,
    pub zoom_step_percent: f32,        // Шаг зума таймлайна в процентах (по умолчанию 8%)

    // 3. Playback & Render Cache
    pub render_cache_mode: usize, // 0: None, 1: Smart (Resolve Auto), 2: User
    pub render_cache_ram_mb: usize,
    pub drop_frames_enabled: bool,
    pub gpu_blending_enabled: bool,
    pub auto_cache_idle_secs: f32,
    pub proxy_resolution: usize, // 0: Full, 1: Half, 2: Quarter, 3: Eighth
    pub proxy_codec: usize,      // 0: ProRes 422 Proxy, 1: DNxHR LB, 2: H.264

    // 4. Memory & GPU
    pub gpu_mode: usize, // 0: Auto, 1: VAAPI Hardware (AMD/Intel), 2: CPU Software
    pub cache_directory: PathBuf,

    // 5. Audio I/O
    pub audio_sample_rate: u32,
    pub audio_buffer_size: usize,
    pub master_volume: f32,
    pub mute_on_scrub: bool,

    // 6. Color & Scopes
    pub color_science: usize, // 0: DaVinci YRGB, 1: DaVinci YRGB Color Managed, 2: ACEScc
    pub timeline_color_space: usize, // 0: Rec.709 Gamma 2.4, 1: Rec.709 (Scene), 2: Rec.2020, 3: DCI-P3
    pub scopes_quality_high: bool,
    pub scopes_brightness: f32,

    // 7. General & Media
    pub hardware_decoding_enabled: bool,
    pub auto_cleanup_proxies: bool,
}

impl Default for UserPreferences {
    fn default() -> Self {
        let cache_dir = if let Ok(cache) = std::env::var("XDG_CACHE_HOME") {
            PathBuf::from(cache).join("verizon_nle")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".cache/verizon_nle")
        } else {
            PathBuf::from(".cache/verizon_nle")
        };

        Self {
            ui_scale: 1.5,
            timecode_display_frames: false,
            theme_accent_idx: 0, // DaVinci Orange
            hide_titlebar: false,

            default_fps: 60,
            default_width: 1920,
            default_height: 1080,
            snapping_enabled: true,
            snapping_threshold_px: 10.0,
            linked_selection: true,
            loop_enabled: false,
            stop_at_timeline_end: true,
            zoom_step_percent: 8.0,

            render_cache_mode: 1, // Smart
            render_cache_ram_mb: 2048,
            drop_frames_enabled: true,
            gpu_blending_enabled: true,
            auto_cache_idle_secs: 0.2,
            proxy_resolution: 1, // Half (1/2)
            proxy_codec: 0,      // ProRes 422 Proxy

            gpu_mode: 0, // Auto
            cache_directory: cache_dir,

            audio_sample_rate: 48000,
            audio_buffer_size: 512,
            master_volume: 1.0,
            mute_on_scrub: false,

            color_science: 0,
            timeline_color_space: 0,
            scopes_quality_high: true,
            scopes_brightness: 0.75,
            hardware_decoding_enabled: true,
            auto_cleanup_proxies: false,
        }
    }
}

impl UserPreferences {
    pub fn config_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config/verizon_nle")
        } else {
            PathBuf::from(".config/verizon_nle")
        }
    }

    pub fn preferences_path() -> PathBuf {
        Self::config_dir().join("preferences.json")
    }

    pub fn dock_layout_path() -> PathBuf {
        Self::config_dir().join("dock_layout.json")
    }

    pub fn scene_path() -> PathBuf {
        Self::config_dir().join("autosave_scene.json")
    }

    pub fn load() -> Self {
        let path = Self::preferences_path();
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(prefs) = serde_json::from_str::<UserPreferences>(&content) {
                return prefs;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let dir = Self::config_dir();
        let _ = fs::create_dir_all(&dir);
        let path = Self::preferences_path();
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

/// Состояние интерактивного модального окна настроек DaVinci Resolve 21
pub struct PreferencesModalState {
    pub current_tab: PrefTab,
    pub current_section: PrefSection,
    pub draft: UserPreferences,
    pub initialized: bool,
    pub is_dirty: bool,
    #[cfg(test)]
    pub last_close_btn_rect: Option<Rect>,
}

impl PreferencesModalState {
    pub fn new() -> Self {
        Self {
            current_tab: PrefTab::System,
            current_section: PrefSection::SystemMemoryGpu,
            draft: UserPreferences::default(),
            initialized: false,
            is_dirty: false,
            #[cfg(test)]
            last_close_btn_rect: None,
        }
    }

    pub fn sync_from_saved(&mut self, saved: &UserPreferences) {
        self.draft = saved.clone();
        self.initialized = true;
        self.is_dirty = false;
    }

    /// Отобразить интерактивное модальное окно настроек DaVinci Resolve 21
    /// Возвращает `Some(draft)`, если пользователь нажал кнопку «Сохранить и применить»
    pub fn render(&mut self, ctx: &Context, open: &mut bool) -> Option<UserPreferences> {
        let mut apply_result = None;
        let mut should_close = false;

        let screen = ctx.content_rect();
        let win_width = if screen.width() > 100.0 {
            (screen.width() - 24.0).clamp(520.0, 920.0)
        } else {
            920.0
        };
        let win_height = if screen.height() > 100.0 {
            (screen.height() - 24.0).clamp(380.0, 620.0)
        } else {
            620.0
        };
        let win_pos = egui::pos2(
            screen.min.x + ((screen.width() - win_width) * 0.5).max(0.0),
            screen.min.y + ((screen.height() - win_height) * 0.5).max(0.0),
        );

        let header_height = 46.0;
        let footer_height = 50.0;
        let body_height = (win_height - header_height - footer_height - 2.0).max(200.0);
        let sidebar_width = (win_width * 0.21).clamp(150.0, 190.0);
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        egui::Window::new("preferences_modal_window")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_pos(win_pos)
            .fixed_size([win_width, win_height])
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x13, 0x14, 0x17))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(0x2d, 0x30, 0x3a)))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::ZERO),
            )
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.spacing_mut().item_spacing = Vec2::ZERO;

                // 1. ШАПКА ОКНА (Header): фиксированный прямоугольник с 3 независимыми зонами (Заголовок | Вкладки | Кнопка ✕)
                let (header_outer, _) = ui.allocate_exact_size(Vec2::new(win_width, header_height), egui::Sense::hover());
                ui.painter().rect_filled(
                    header_outer,
                    CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 },
                    Color32::from_rgb(0x18, 0x1a, 0x20),
                );
                ui.painter().line_segment(
                    [header_outer.left_bottom(), header_outer.right_bottom()],
                    Stroke::new(1.0, Color32::from_rgb(0x26, 0x29, 0x32)),
                );

                let header_inner = header_outer.shrink2(Vec2::new(16.0, 7.0));

                // 1a. Левая зона шапки: заголовок «⚙ Preferences»
                let left_rect = Rect::from_min_max(
                    header_inner.min,
                    egui::pos2(header_inner.center().x - 90.0, header_inner.max.y),
                );
                let mut left_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_hdr_left")
                        .max_rect(left_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                left_ui.label(
                    egui::RichText::new("⚙ Preferences")
                        .size(13.5)
                        .strong()
                        .color(Color32::WHITE),
                );

                // 1b. Центральная зона шапки: переключатель вкладок [ System | User ]
                let pill_rect = Rect::from_center_size(header_outer.center(), Vec2::new(168.0, 28.0));
                ui.painter().rect_filled(pill_rect, CornerRadius::same(4), Color32::from_rgb(0x12, 0x13, 0x17));
                ui.painter().rect_stroke(
                    pill_rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, Color32::from_rgb(0x28, 0x2b, 0x34)),
                    egui::StrokeKind::Inside,
                );
                let pill_inner = pill_rect.shrink2(Vec2::new(3.0, 2.0));
                let half_pill_w = (pill_inner.width() - 4.0) * 0.5;
                let sys_rect = Rect::from_min_size(pill_inner.min, Vec2::new(half_pill_w, pill_inner.height()));
                let usr_rect = Rect::from_min_size(
                    egui::pos2(pill_inner.min.x + half_pill_w + 4.0, pill_inner.min.y),
                    Vec2::new(half_pill_w, pill_inner.height()),
                );

                let sys_active = self.current_tab == PrefTab::System;
                let user_active = self.current_tab == PrefTab::User;

                let mut sys_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_hdr_sys")
                        .max_rect(sys_rect)
                        .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
                );
                let sys_btn = egui::Button::new(
                    egui::RichText::new("System")
                        .size(11.5)
                        .strong()
                        .color(if sys_active { Color32::WHITE } else { Color32::from_rgb(0x92, 0x97, 0xa5) }),
                )
                .fill(if sys_active { accent.gamma_multiply(0.35) } else { Color32::TRANSPARENT })
                .stroke(if sys_active { Stroke::new(1.0, accent) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(3));
                if sys_ui.add(sys_btn).clicked() {
                    self.current_tab = PrefTab::System;
                    self.current_section = PrefSection::SystemMemoryGpu;
                }

                let mut usr_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_hdr_usr")
                        .max_rect(usr_rect)
                        .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
                );
                let user_btn = egui::Button::new(
                    egui::RichText::new("User")
                        .size(11.5)
                        .strong()
                        .color(if user_active { Color32::WHITE } else { Color32::from_rgb(0x92, 0x97, 0xa5) }),
                )
                .fill(if user_active { accent.gamma_multiply(0.35) } else { Color32::TRANSPARENT })
                .stroke(if user_active { Stroke::new(1.0, accent) } else { Stroke::NONE })
                .corner_radius(CornerRadius::same(3));
                if usr_ui.add(user_btn).clicked() {
                    self.current_tab = PrefTab::User;
                    self.current_section = PrefSection::UserAppearance;
                }

                // 1c. Правая зона шапки: кнопка закрытия ✕ (жестко закреплена у правого края шапки)
                let close_rect = Rect::from_min_max(
                    egui::pos2(header_inner.max.x - 30.0, header_outer.center().y - 13.0),
                    egui::pos2(header_inner.max.x, header_outer.center().y + 13.0),
                );
                let mut right_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_hdr_close")
                        .max_rect(close_rect)
                        .layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)),
                );
                let close_btn = egui::Button::new(
                    egui::RichText::new("✕")
                        .size(14.0)
                        .strong()
                        .color(Color32::from_rgb(0xe2, 0xe6, 0xf0)),
                )
                .fill(Color32::from_rgb(0x25, 0x28, 0x32))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x36, 0x3a, 0x46)))
                .corner_radius(CornerRadius::same(4));

                let close_resp = right_ui.add(close_btn).on_hover_text("Закрыть [Esc]");
                #[cfg(test)]
                {
                    self.last_close_btn_rect = Some(close_resp.rect);
                }
                if close_resp.clicked() {
                    should_close = true;
                }

                // 2. ТЕЛО ОКНА (Body): фиксированный прямоугольник, разделенный на Сайдбар слева и ScrollArea справа
                let (body_outer, _) = ui.allocate_exact_size(Vec2::new(win_width, body_height), egui::Sense::hover());
                let body_inner = body_outer.shrink2(Vec2::new(14.0, 10.0));

                let sidebar_rect = Rect::from_min_size(body_inner.min, Vec2::new(sidebar_width, body_inner.height()));
                ui.painter().rect_filled(sidebar_rect, CornerRadius::same(4), Color32::from_rgb(0x16, 0x17, 0x1c));
                ui.painter().rect_stroke(
                    sidebar_rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, Color32::from_rgb(0x23, 0x25, 0x2c)),
                    egui::StrokeKind::Inside,
                );

                let mut sidebar_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_body_sidebar")
                        .max_rect(sidebar_rect.shrink(6.0))
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                sidebar_ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
                self.render_sidebar(&mut sidebar_ui);

                let content_rect = Rect::from_min_max(
                    egui::pos2(sidebar_rect.max.x + 12.0, body_inner.min.y),
                    body_inner.max,
                );
                let mut content_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_body_content")
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                content_ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                egui::ScrollArea::vertical()
                    .id_salt("preferences_body_scroll")
                    .auto_shrink([false, false])
                    .max_height(content_rect.height())
                    .show(&mut content_ui, |ui| {
                        ui.set_width((content_rect.width() - 12.0).max(180.0));
                        self.render_content(ui);
                    });

                // 3. ПОДВАЛ ОКНА (Footer): фиксированный прямоугольник (Reset слева | Cancel + Save справа)
                let (footer_outer, _) = ui.allocate_exact_size(Vec2::new(win_width, footer_height), egui::Sense::hover());
                ui.painter().rect_filled(
                    footer_outer,
                    CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 },
                    Color32::from_rgb(0x17, 0x18, 0x1e),
                );
                ui.painter().line_segment(
                    [footer_outer.left_top(), footer_outer.right_top()],
                    Stroke::new(1.0, Color32::from_rgb(0x26, 0x29, 0x32)),
                );

                let footer_inner = footer_outer.shrink2(Vec2::new(16.0, 10.0));
                let mut ftr_left_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_ftr_left")
                        .max_rect(footer_inner)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let reset_btn = egui::Button::new(
                    egui::RichText::new("⟲  Reset to Defaults")
                        .size(11.0)
                        .color(Color32::from_rgb(0xa2, 0xa8, 0xb6)),
                )
                .fill(Color32::from_rgb(0x1e, 0x20, 0x26))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x2d, 0x30, 0x39)))
                .corner_radius(CornerRadius::same(3))
                .min_size(Vec2::new(134.0, 28.0));

                if ftr_left_ui.add(reset_btn).clicked() {
                    self.draft = UserPreferences::default();
                }

                let mut ftr_right_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("pref_ftr_right")
                        .max_rect(footer_inner)
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                ftr_right_ui.spacing_mut().item_spacing.x = 8.0;

                let save_btn = egui::Button::new(
                    egui::RichText::new("  Save  ")
                        .size(12.0)
                        .color(Color32::WHITE)
                        .strong(),
                )
                .fill(accent)
                .stroke(Stroke::new(1.0, accent))
                .corner_radius(CornerRadius::same(3))
                .min_size(Vec2::new(84.0, 28.0));

                if ftr_right_ui.add(save_btn).clicked() {
                    apply_result = Some(self.draft.clone());
                    should_close = true;
                }

                let cancel_btn = egui::Button::new(
                    egui::RichText::new("  Cancel  ")
                        .size(11.5)
                        .color(Color32::from_rgb(0xd5, 0xda, 0xe5)),
                )
                .fill(Color32::from_rgb(0x23, 0x25, 0x2c))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x30, 0x33, 0x3d)))
                .corner_radius(CornerRadius::same(3))
                .min_size(Vec2::new(76.0, 28.0));

                if ftr_right_ui.add(cancel_btn).clicked() {
                    should_close = true;
                }

                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    should_close = true;
                }
            });

        if should_close {
            *open = false;
        }

        apply_result
    }
}

/// Фирменная карточка группы настроек DaVinci Resolve
fn pref_card<R>(ui: &mut Ui, title: &str, subtitle: Option<&str>, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = egui::Frame::new()
        .fill(Color32::from_rgb(0x18, 0x19, 0x1e))
        .stroke(Stroke::new(1.0, Color32::from_rgb(0x27, 0x2a, 0x32)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(14, 10));

    frame.show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(title)
                        .size(11.5)
                        .strong()
                        .color(Color32::from_rgb(0xec, 0xf0, 0xf8)),
                )
                .wrap(),
            );
            if let Some(sub) = subtitle {
                ui.add_space(2.0);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(sub)
                            .size(10.0)
                            .color(Color32::from_rgb(0x84, 0x8a, 0x99)),
                    )
                    .wrap(),
                );
            }
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            add_contents(ui)
        }).inner
    }).inner
}

/// Структурированная 2-колоночная сетка DaVinci Resolve
fn pref_grid<R>(ui: &mut Ui, id_salt: &str, add_rows: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id_salt)
        .num_columns(2)
        .spacing([14.0, 10.0])
        .min_col_width(170.0)
        .max_col_width(210.0)
        .show(ui, add_rows)
        .inner
}

/// Строка сетки: левая колонка - название параметра с автопереносом, правая - виджет
fn pref_grid_row<R>(ui: &mut Ui, label: &str, add_widget: impl FnOnce(&mut Ui) -> R) -> R {
    ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(11.5)
                .color(Color32::from_rgb(0x9d, 0xa3, 0xb4)),
        )
        .wrap(),
    );
    let res = add_widget(ui);
    ui.end_row();
    res
}

/// Чекбокс с гарантированным переносом строк и кликабельным текстом
fn pref_checkbox(ui: &mut Ui, value: &mut bool, text: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let mut cb_resp = ui.checkbox(value, "");
        let lbl_resp = ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .size(11.5)
                    .color(Color32::from_rgb(0xcc, 0xd0, 0xdc)),
            )
            .wrap(),
        );
        if lbl_resp.clicked() {
            *value = !*value;
            cb_resp.mark_changed();
        }
        cb_resp | lbl_resp
    }).inner
}

/// Сегментированные кнопки-пилюли DaVinci Resolve
fn pref_segmented_pills(ui: &mut Ui, current: &mut usize, options: &[(usize, &str)], accent: Color32) {
    ui.horizontal(|ui| {
        egui::Frame::new()
            .fill(Color32::from_rgb(0x1a, 0x1c, 0x22))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2d, 0x38)))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(egui::Margin::symmetric(2, 2))
            .show(ui, |ui| {
                for (val, title) in options {
                    let is_active = *current == *val;
                    let btn = egui::Button::new(
                        egui::RichText::new(*title)
                            .size(11.0)
                            .color(if is_active { Color32::WHITE } else { Color32::from_rgb(0x9a, 0xa0, 0xb0) }),
                    )
                    .fill(if is_active { accent.gamma_multiply(0.35) } else { Color32::TRANSPARENT })
                    .stroke(if is_active { Stroke::new(1.0, accent) } else { Stroke::NONE })
                    .corner_radius(CornerRadius::same(2));

                    if ui.add(btn).clicked() {
                        *current = *val;
                    }
                }
            });
    });
}

impl PreferencesModalState {
    fn render_sidebar(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        let sections = match self.current_tab {
            PrefTab::System => vec![
                (PrefSection::SystemMemoryGpu, "🧠", "Memory and GPU"),
                (PrefSection::SystemMediaStorage, "📁", "Media Storage"),
                (PrefSection::SystemAudio, "🔊", "Audio I/O"),
                (PrefSection::SystemHardware, "⚡", "Hardware Decode"),
            ],
            PrefTab::User => vec![
                (PrefSection::UserAppearance, "🖥", "UI & Appearance"),
                (PrefSection::UserEditing, "🎞", "Editing & Timeline"),
                (PrefSection::UserPlaybackCache, "⚡", "Playback & Cache"),
                (PrefSection::UserColorScopes, "🎨", "Color & Scopes"),
            ],
        };

        for (sec, icon, title) in sections {
            let is_selected = self.current_section == sec;
            let btn_text = format!(" {}  {}", icon, title);
            let btn = egui::Button::new(
                egui::RichText::new(btn_text)
                    .size(11.5)
                    .color(if is_selected { Color32::WHITE } else { Color32::from_rgb(0xa4, 0xaa, 0xb8) }),
            )
            .fill(if is_selected { Color32::from_rgb(0x23, 0x25, 0x2c) } else { Color32::TRANSPARENT })
            .stroke(if is_selected { Stroke::new(1.0, accent.gamma_multiply(0.6)) } else { Stroke::NONE })
            .corner_radius(CornerRadius::same(3))
            .min_size(Vec2::new(ui.available_width(), 32.0));

            if ui.add(btn).clicked() {
                self.current_section = sec;
            }
            ui.add_space(2.0);
        }
    }

    fn render_content(&mut self, ui: &mut Ui) {
        match self.current_section {
            PrefSection::SystemMemoryGpu => self.render_system_memory_gpu(ui),
            PrefSection::SystemMediaStorage => self.render_system_media_storage(ui),
            PrefSection::SystemAudio => self.render_system_audio(ui),
            PrefSection::SystemHardware => self.render_system_hardware(ui),
            PrefSection::UserAppearance => self.render_user_appearance(ui),
            PrefSection::UserEditing => self.render_user_editing(ui),
            PrefSection::UserPlaybackCache => self.render_user_playback_cache(ui),
            PrefSection::UserColorScopes => self.render_color_scopes(ui),
        }
    }

    fn render_system_memory_gpu(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        pref_card(ui, "SYSTEM MEMORY CONFIGURATION", Some("Выделение оперативной памяти для фонового рендера и воспроизведения"), |ui| {
            pref_grid(ui, "grid_sys_mem", |ui| {
                pref_grid_row(ui, "Общий объем памяти:", |ui| {
                    ui.label(egui::RichText::new("32 768 MB (32.0 GB DDR5)").strong().color(Color32::WHITE));
                });

                pref_grid_row(ui, "Лимит для Verizon NLE:", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.render_cache_ram_mb, 512..=16384)
                            .step_by(256.0)
                            .suffix(" MB"),
                    );
                });
            });

            ui.add_space(8.0);

            // Градусник памяти в стиле DaVinci Resolve
            let total_ram_mb = 32768.0;
            let nle_ram_mb = self.draft.render_cache_ram_mb as f32;
            let ratio = (nle_ram_mb / total_ram_mb).clamp(0.02, 0.95);

            let bar_height = 22.0;
            let bar_width = (ui.available_width() - 4.0).max(120.0);
            let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(bar_width, bar_height), egui::Sense::hover());
            let painter = ui.painter();

            painter.rect_filled(bar_rect, CornerRadius::same(3), Color32::from_rgb(0x23, 0x25, 0x2d));

            let filled_w = bar_rect.width() * ratio;
            let nle_rect = Rect::from_min_size(bar_rect.min, Vec2::new(filled_w, bar_height));
            painter.rect_filled(nle_rect, CornerRadius::same(3), accent.gamma_multiply(0.85));

            let text_nle = format!("Verizon NLE: {} MB ({:.1}%)", self.draft.render_cache_ram_mb, ratio * 100.0);
            painter.text(
                nle_rect.center(),
                egui::Align2::CENTER_CENTER,
                text_nle,
                egui::FontId::proportional(10.5),
                Color32::WHITE,
            );

            let os_rect = Rect::from_min_max(nle_rect.right_top(), bar_rect.max);
            if os_rect.width() > 100.0 {
                let text_os = format!("OS & Other: {:.0} MB", total_ram_mb - nle_ram_mb);
                painter.text(
                    os_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text_os,
                    egui::FontId::proportional(10.0),
                    Color32::from_rgb(0x9a, 0xa0, 0xb0),
                );
            }

            ui.add_space(6.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("💡 Выделение 2048-4096 МБ обеспечивает стабильный реалтайм 60 FPS предпросмотр без задержек.")
                        .size(10.0)
                        .color(Color32::from_rgb(0x82, 0x88, 0x96)),
                )
                .wrap(),
            );
        });

        ui.add_space(10.0);

        pref_card(ui, "GPU PROCESSING CONFIGURATION", Some("Графические ускорители и параметры аппаратного рендеринга"), |ui| {
            pref_grid(ui, "grid_gpu_proc", |ui| {
                pref_grid_row(ui, "Режим работы GPU:", |ui| {
                    pref_segmented_pills(ui, &mut self.draft.gpu_mode, &[
                        (0, "Auto"),
                        (1, "VAAPI Hardware"),
                        (2, "CPU Software"),
                    ], accent);
                });

                pref_grid_row(ui, "Аппаратный блендинг:", |ui| {
                    pref_checkbox(ui, &mut self.draft.gpu_blending_enabled, "GPU Blending (Аппаратное альфа-смешивание)");
                });
            });

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Обнаруженные видеоустройства в системе:").size(11.0).strong().color(Color32::from_rgb(0xec, 0xf0, 0xf8)));
            ui.add_space(4.0);

            // Таблица обнаруженных GPU в структурированном Grid
            egui::Frame::new()
                .fill(Color32::from_rgb(0x14, 0x15, 0x1a))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x27, 0x2a, 0x34)))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    egui::Grid::new("sys_gpu_grid")
                        .num_columns(5)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("ID").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Модель устройства").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("VRAM").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Шина / DRM").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Статус").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.end_row();

                            // GPU 0
                            ui.label("GPU 0");
                            ui.label("AMD Radeon RX 9070 XT");
                            ui.label("16 GB");
                            ui.monospace("renderD129");
                            ui.label(egui::RichText::new("● Active (Primary)").color(Color32::from_rgb(0x40, 0xd0, 0x70)));
                            ui.end_row();

                            // GPU 1
                            ui.label("GPU 1");
                            ui.label("AMD Radeon 780M Graphics");
                            ui.label("2 GB");
                            ui.monospace("renderD128");
                            ui.label(egui::RichText::new("○ Available (iGPU)").color(Color32::from_rgb(0x82, 0x88, 0x96)));
                            ui.end_row();
                        });
                });
        });
    }

    fn render_system_media_storage(&mut self, ui: &mut Ui) {
        pref_card(ui, "MOUNTED MEDIA STORAGE & SCRATCH DISKS", Some("Дисковые накопители для кэширования, прокси и исходников"), |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgb(0x14, 0x15, 0x1a))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x27, 0x2a, 0x34)))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    egui::Grid::new("sys_storage_grid")
                        .num_columns(4)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Точка монтирования / Путь").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Роль тома").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Свободно").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Действия").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.end_row();

                            // Строка 1: Кэш таймлайна
                            let path_display = self.draft.cache_directory.to_string_lossy().to_string();
                            let short_path = if path_display.len() > 30 {
                                format!("...{}", &path_display[path_display.len() - 27..])
                            } else {
                                path_display.clone()
                            };
                            ui.add(egui::Label::new(egui::RichText::new(short_path).monospace().size(11.0)))
                                .on_hover_text(&path_display);
                            ui.label(egui::RichText::new("Scratch & Cache").color(Color32::from_rgb(0x56, 0xad, 0xee)));
                            ui.label("428.5 GB");

                            if ui.button("🗑 Purge").clicked() {
                                if let Ok(entries) = std::fs::read_dir(&self.draft.cache_directory) {
                                    for e in entries.flatten() {
                                        let _ = std::fs::remove_file(e.path());
                                    }
                                }
                            }
                            ui.end_row();

                            // Строка 2: Проектные диски
                            ui.label(egui::RichText::new("/mnt/projects-and-games").monospace().size(11.0));
                            ui.label("Media Storage");
                            ui.label("1.8 TB");
                            ui.label(egui::RichText::new("● Mounted").color(Color32::from_rgb(0x40, 0xd0, 0x70)));
                            ui.end_row();
                        });
                });

            ui.add_space(10.0);

            let mut path_str = self.draft.cache_directory.to_string_lossy().to_string();
            pref_grid(ui, "grid_cache_disk", |ui| {
                pref_grid_row(ui, "Каталог кэша рендера:", |ui| {
                    ui.horizontal(|ui| {
                        let text_w = (ui.available_width() - 80.0).max(180.0);
                        if ui.add(egui::TextEdit::singleline(&mut path_str).desired_width(text_w)).changed() {
                            self.draft.cache_directory = PathBuf::from(path_str);
                        }
                        if ui.button("⟲ Сброс").clicked() {
                            if let Ok(home) = std::env::var("HOME") {
                                self.draft.cache_directory = PathBuf::from(home).join(".cache/verizon_nle");
                            }
                        }
                    });
                });

                pref_grid_row(ui, "Автоматическая очистка:", |ui| {
                    pref_checkbox(ui, &mut self.draft.auto_cleanup_proxies, "Очищать временные прокси и дисковый кэш при закрытии");
                });
            });
        });
    }

    fn render_system_audio(&mut self, ui: &mut Ui) {
        pref_card(ui, "AUDIO ENGINE & I/O CONFIGURATION", Some("Низколатентный аудиодрайвер PipeWire / ALSA и частота дискретизации"), |ui| {
            pref_grid(ui, "grid_audio_io", |ui| {
                pref_grid_row(ui, "Аудиоподсистема:", |ui| {
                    ui.label(egui::RichText::new("PipeWire Audio Server (Native)").strong().color(Color32::WHITE));
                });

                pref_grid_row(ui, "Устройство вывода:", |ui| {
                    ui.label(egui::RichText::new("● PipeWire ALSA / Studio Monitor").color(Color32::from_rgb(0x40, 0xd0, 0x70)));
                });

                pref_grid_row(ui, "Частота дискретизации:", |ui| {
                    egui::ComboBox::from_id_salt("audio_sr_combo")
                        .selected_text(match self.draft.audio_sample_rate {
                            44100 => "44.1 kHz (CD Audio)",
                            96000 => "96.0 kHz (Hi-Res Studio)",
                            _ => "48.0 kHz (Cinema Standard)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.audio_sample_rate, 44100, "44.1 kHz (CD Audio)");
                            ui.selectable_value(&mut self.draft.audio_sample_rate, 48000, "48.0 kHz (Cinema Standard)");
                            ui.selectable_value(&mut self.draft.audio_sample_rate, 96000, "96.0 kHz (Hi-Res Studio)");
                        });
                });

                pref_grid_row(ui, "Размер буфера (Latency):", |ui| {
                    egui::ComboBox::from_id_salt("audio_buf_combo")
                        .selected_text(match self.draft.audio_buffer_size {
                            128 => "128 (2.7 ms - Low Latency)",
                            256 => "256 (5.3 ms)",
                            1024 => "1024 (21.3 ms - Safe)",
                            _ => "512 (10.7 ms - Рекомендуется)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.audio_buffer_size, 128, "128 (2.7 ms - Low Latency)");
                            ui.selectable_value(&mut self.draft.audio_buffer_size, 256, "256 (5.3 ms)");
                            ui.selectable_value(&mut self.draft.audio_buffer_size, 512, "512 (10.7 ms - Рекомендуется)");
                            ui.selectable_value(&mut self.draft.audio_buffer_size, 1024, "1024 (21.3 ms - Safe)");
                        });
                });
            });
        });

        ui.add_space(10.0);

        pref_card(ui, "SPEAKER SETUP & CHANNEL ROUTING", Some("Назначение каналов мониторинга Fairlight"), |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgb(0x14, 0x15, 0x1a))
                .stroke(Stroke::new(1.0, Color32::from_rgb(0x27, 0x2a, 0x34)))
                .corner_radius(CornerRadius::same(3))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    egui::Grid::new("audio_spk_grid")
                        .num_columns(4)
                        .spacing([14.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new("Канал микшера").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Динамик монитора").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Порт PipeWire").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.label(egui::RichText::new("Статус").strong().color(Color32::from_rgb(0x8a, 0x90, 0xa0)));
                            ui.end_row();

                            ui.label("Channel 1 (Left)");
                            ui.label("Left Front");
                            ui.monospace("playback_FL");
                            ui.label(egui::RichText::new("● Connected").color(Color32::from_rgb(0x40, 0xd0, 0x70)));
                            ui.end_row();

                            ui.label("Channel 2 (Right)");
                            ui.label("Right Front");
                            ui.monospace("playback_FR");
                            ui.label(egui::RichText::new("● Connected").color(Color32::from_rgb(0x40, 0xd0, 0x70)));
                            ui.end_row();
                        });
                });

            ui.add_space(8.0);

            pref_grid(ui, "grid_audio_master", |ui| {
                pref_grid_row(ui, "Громкость монитора:", |ui| {
                    let db_str = if self.draft.master_volume > 0.001 {
                        format!("{:.1} dB ({:.0}%)", 20.0 * self.draft.master_volume.log10(), self.draft.master_volume * 100.0)
                    } else {
                        "-∞ dB (Muted)".to_string()
                    };
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Slider::new(&mut self.draft.master_volume, 0.0..=1.5)
                                .step_by(0.05)
                                .show_value(false),
                        );
                        ui.label(egui::RichText::new(db_str).monospace().size(11.0));
                    });
                });

                pref_grid_row(ui, "Скраббинг аудио:", |ui| {
                    pref_checkbox(ui, &mut self.draft.mute_on_scrub, "Mute on scrub (Заглушать звук при скраббинге)");
                });
            });
        });
    }

    fn render_system_hardware(&mut self, ui: &mut Ui) {
        pref_card(ui, "HARDWARE ACCELERATION & ZERO-COPY DECODE", Some("Аппаратное декодирование через VAAPI и GPU-акселерация"), |ui| {
            pref_grid(ui, "grid_hw_decode", |ui| {
                pref_grid_row(ui, "Аппаратное декодирование:", |ui| {
                    pref_checkbox(ui, &mut self.draft.hardware_decoding_enabled, "AMD / Intel VAAPI аппаратное декодирование H.264, HEVC, AV1");
                });

                pref_grid_row(ui, "Zero-copy ускорение:", |ui| {
                    pref_checkbox(ui, &mut self.draft.gpu_blending_enabled, "Прямой Zero-copy трансфер текстур через DMABUF");
                });
            });
        });

        ui.add_space(10.0);

        pref_card(ui, "PLAYBACK TIMING & SYNC", Some("Синхронизация звука и видео в режиме реального времени 1.0x"), |ui| {
            pref_grid(ui, "grid_timing_sync", |ui| {
                pref_grid_row(ui, "Пропуск кадров:", |ui| {
                    pref_checkbox(ui, &mut self.draft.drop_frames_enabled, "Drop frames during playback (Пропускать кадры при нехватке скорости)");
                });
            });

            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("💡 При включении таймлайн сохраняет реальную скорость 1x даже на тяжелых файлах, предотвращая рассинхрон звука.")
                        .size(10.0)
                        .color(Color32::from_rgb(0x82, 0x88, 0x96)),
                )
                .wrap(),
            );
        });
    }

    fn render_user_appearance(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        pref_card(ui, "DISPLAY SCALING (HiDPI / 4K UHD)", Some("Масштабирование экранов высокого разрешения"), |ui| {
            pref_grid(ui, "grid_disp_scale", |ui| {
                pref_grid_row(ui, "Заголовок окна:", |ui| {
                    pref_checkbox(ui, &mut self.draft.hide_titlebar, "Скрыть заголовок окна (Borderless / Полноэкранный вид [F10])");
                });

                pref_grid_row(ui, "Пресеты разрешения:", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let presets = [
                            (1.0, "100%"),
                            (1.25, "125%"),
                            (1.5, "150%"),
                            (1.75, "175%"),
                            (2.0, "200%"),
                            (2.5, "250%"),
                        ];

                        for (scale, label) in presets {
                            let is_active = (self.draft.ui_scale - scale).abs() < 0.02;
                            let btn = egui::Button::new(
                                egui::RichText::new(label)
                                    .size(11.0)
                                    .color(if is_active { Color32::WHITE } else { Color32::from_rgb(0xa2, 0xa8, 0xb8) }),
                            )
                            .fill(if is_active { accent } else { Color32::from_rgb(0x22, 0x24, 0x2a) });

                            if ui.add(btn).clicked() {
                                self.draft.ui_scale = scale;
                            }
                        }
                    });
                });

                pref_grid_row(ui, "Точный масштаб:", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.ui_scale, 0.75..=3.5)
                            .step_by(0.05)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });
            });

            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("💡 Быстрые хоткеи масштабирования: Ctrl + Plus / Ctrl + Minus / Ctrl + 0")
                        .size(10.0)
                        .color(Color32::from_rgb(0x82, 0x88, 0x96)),
                )
                .wrap(),
            );
        });

        ui.add_space(10.0);

        pref_card(ui, "DAVINCI RESOLVE ACCENT PALETTE", Some("Цветовая схема элементов управления и подсветки активных инструментов"), |ui| {
            pref_grid(ui, "grid_palette", |ui| {
                pref_grid_row(ui, "Цветовой акцент:", |ui| {
                    ui.horizontal(|ui| {
                        let themes = [
                            (0, "🟠 Orange"),
                            (1, "🔵 Blue"),
                            (2, "🟡 Gold"),
                        ];
                        for (idx, name) in themes {
                            let is_active = self.draft.theme_accent_idx == idx;
                            let (btn_accent, _) = get_accent_colors(idx);
                            let btn = egui::Button::new(
                                egui::RichText::new(name)
                                    .color(if is_active { Color32::WHITE } else { Color32::from_rgb(0xa2, 0xa8, 0xb8) }),
                            )
                            .fill(if is_active { btn_accent } else { Color32::from_rgb(0x22, 0x24, 0x2a) });

                            if ui.add(btn).clicked() {
                                self.draft.theme_accent_idx = idx;
                            }
                        }
                    });
                });
            });
        });

        ui.add_space(10.0);

        pref_card(ui, "TIMECODE DISPLAY FORMAT", Some("Формат отображения таймкода и текущей позиции плейхеда"), |ui| {
            pref_grid(ui, "grid_tc_format", |ui| {
                pref_grid_row(ui, "Режим отображения:", |ui| {
                    ui.vertical(|ui| {
                        ui.radio_value(&mut self.draft.timecode_display_frames, false, "SMPTE Timecode (01:21:30:03)");
                        ui.radio_value(&mut self.draft.timecode_display_frames, true, "Кадры таймлайна (Frames: 0, 1, 2 ..)");
                    });
                });
            });
        });
    }

    fn render_user_editing(&mut self, ui: &mut Ui) {
        pref_card(ui, "SNAPPING & MAGNETIC TIMELINE", Some("Поведение магнитного прилипания курсора и границ клипов"), |ui| {
            pref_grid(ui, "grid_snapping", |ui| {
                pref_grid_row(ui, "Магнитное прилипание:", |ui| {
                    pref_checkbox(ui, &mut self.draft.snapping_enabled, "Включить магнитное прилипание клипов [N]");
                });

                pref_grid_row(ui, "Чувствительность:", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.snapping_threshold_px, 2.0..=30.0)
                            .suffix(" px"),
                    );
                });

                pref_grid_row(ui, "Шаг зума таймлайна:", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.zoom_step_percent, 1.0..=25.0)
                            .step_by(1.0)
                            .suffix("%"),
                    );
                });
            });
        });

        ui.add_space(10.0);

        pref_card(ui, "CLIP LINKING & PLAYBACK", Some("Связность аудио/видео клипов и навигация"), |ui| {
            pref_grid(ui, "grid_clip_linking", |ui| {
                pref_grid_row(ui, "Связанное выделение:", |ui| {
                    pref_checkbox(ui, &mut self.draft.linked_selection, "Связанное выделение видео и аудио [Ctrl+Shift+L]");
                });

                pref_grid_row(ui, "Конец таймлайна:", |ui| {
                    pref_checkbox(ui, &mut self.draft.stop_at_timeline_end, "Останавливать воспроизведение в конце таймлайна");
                });

                pref_grid_row(ui, "Зацикливание:", |ui| {
                    pref_checkbox(ui, &mut self.draft.loop_enabled, "Зацикливать воспроизведение (Loop Playback [Ctrl+/])");
                });
            });
        });

        ui.add_space(10.0);

        pref_card(ui, "NEW PROJECT TIMELINE DEFAULTS", Some("Параметры по умолчанию для новых проектов и сиквенсов"), |ui| {
            pref_grid(ui, "grid_proj_defaults", |ui| {
                pref_grid_row(ui, "Частота кадров (FPS):", |ui| {
                    egui::ComboBox::from_id_salt("pref_fps_combo")
                        .selected_text(format!("{} fps", self.draft.default_fps))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.default_fps, 24, "24.00 fps (Cinema)");
                            ui.selectable_value(&mut self.draft.default_fps, 25, "25.00 fps (PAL)");
                            ui.selectable_value(&mut self.draft.default_fps, 30, "30.00 fps (NTSC / Web)");
                            ui.selectable_value(&mut self.draft.default_fps, 50, "50.00 fps (PAL Broadcast)");
                            ui.selectable_value(&mut self.draft.default_fps, 60, "60.00 fps (Smooth Studio)");
                        });
                });

                pref_grid_row(ui, "Разрешение таймлайна:", |ui| {
                    let cur_res_str = format!("{} x {}", self.draft.default_width, self.draft.default_height);
                    egui::ComboBox::from_id_salt("pref_res_combo")
                        .selected_text(cur_res_str)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.draft.default_width == 1920 && self.draft.default_height == 1080, "1920 x 1080 (Full HD 16:9)").clicked() {
                                self.draft.default_width = 1920;
                                self.draft.default_height = 1080;
                            }
                            if ui.selectable_label(self.draft.default_width == 2560 && self.draft.default_height == 1440, "2560 x 1440 (2K QHD 16:9)").clicked() {
                                self.draft.default_width = 2560;
                                self.draft.default_height = 1440;
                            }
                            if ui.selectable_label(self.draft.default_width == 3840 && self.draft.default_height == 2160, "3840 x 2160 (4K UHD 16:9)").clicked() {
                                self.draft.default_width = 3840;
                                self.draft.default_height = 2160;
                            }
                            if ui.selectable_label(self.draft.default_width == 1080 && self.draft.default_height == 1920, "1080 x 1920 (Vertical 9:16 Shorts)").clicked() {
                                self.draft.default_width = 1080;
                                self.draft.default_height = 1920;
                            }
                        });
                });
            });
        });
    }

    fn render_user_playback_cache(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        pref_card(ui, "RENDER CACHE MODE (DaVinci Resolve)", Some("Интеллектуальное кэширование эффектов, титров и склеек"), |ui| {
            pref_grid(ui, "grid_cache_mode", |ui| {
                pref_grid_row(ui, "Режим кэша рендера:", |ui| {
                    pref_segmented_pills(ui, &mut self.draft.render_cache_mode, &[
                        (0, "None"),
                        (1, "Smart Cache"),
                        (2, "User Cache"),
                    ], accent);
                });
            });

            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new("💡 В режиме Smart Cache система сама анализирует тяжелые эффекты и переходы и кэширует их в фоне.")
                        .size(10.0)
                        .color(Color32::from_rgb(0x82, 0x88, 0x96)),
                )
                .wrap(),
            );
        });

        ui.add_space(10.0);

        pref_card(ui, "PROXY GENERATION & BACKGROUND CACHING", Some("Разрешение прокси-медиа и задержка фонового рендера"), |ui| {
            pref_grid(ui, "grid_proxy_cache", |ui| {
                pref_grid_row(ui, "Разрешение прокси:", |ui| {
                    pref_segmented_pills(ui, &mut self.draft.proxy_resolution, &[
                        (0, "Full (1/1)"),
                        (1, "Half (1/2)"),
                        (2, "Quarter (1/4)"),
                        (3, "Eighth (1/8)"),
                    ], accent);
                });

                pref_grid_row(ui, "Кодек прокси-файлов:", |ui| {
                    pref_segmented_pills(ui, &mut self.draft.proxy_codec, &[
                        (0, "ProRes 422"),
                        (1, "DNxHR LB"),
                        (2, "H.264"),
                    ], accent);
                });

                pref_grid_row(ui, "Задержка рендера:", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.auto_cache_idle_secs, 0.1..=5.0)
                            .step_by(0.1)
                            .suffix(" сек при простое"),
                    );
                });
            });
        });
    }

    fn render_color_scopes(&mut self, ui: &mut Ui) {
        pref_card(ui, "COLOR MANAGEMENT", Some("Цветовые пространства и калибровка"), |ui| {
            pref_grid(ui, "grid_color_mgmt", |ui| {
                pref_grid_row(ui, "Color Science:", |ui| {
                    egui::ComboBox::from_id_salt("cs_science_combo")
                        .selected_text(match self.draft.color_science {
                            1 => "DaVinci YRGB Color Managed (RCM)",
                            2 => "ACEScc (Academy Color Encoding)",
                            _ => "DaVinci YRGB (Standard)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.color_science, 0, "DaVinci YRGB (Standard)");
                            ui.selectable_value(&mut self.draft.color_science, 1, "DaVinci YRGB Color Managed (RCM)");
                            ui.selectable_value(&mut self.draft.color_science, 2, "ACEScc (Academy Color Encoding)");
                        });
                });

                pref_grid_row(ui, "Цветовое пространство:", |ui| {
                    egui::ComboBox::from_id_salt("cs_space_combo")
                        .selected_text(match self.draft.timeline_color_space {
                            1 => "Rec.709 (Scene)",
                            2 => "Rec.2020 (HDR/UHD)",
                            3 => "DCI-P3 (Cinema)",
                            _ => "Rec.709 Gamma 2.4 (HDTV)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.timeline_color_space, 0, "Rec.709 Gamma 2.4 (HDTV)");
                            ui.selectable_value(&mut self.draft.timeline_color_space, 1, "Rec.709 (Scene)");
                            ui.selectable_value(&mut self.draft.timeline_color_space, 2, "Rec.2020 (HDR/UHD)");
                            ui.selectable_value(&mut self.draft.timeline_color_space, 3, "DCI-P3 (Cinema)");
                        });
                });
            });
        });

        ui.add_space(10.0);

        pref_card(ui, "VIDEO MONITORS & SCOPES", Some("Параметры графиков Waveform, Vectorscope и Histogram"), |ui| {
            pref_grid(ui, "grid_scopes", |ui| {
                pref_grid_row(ui, "Точность скоупов:", |ui| {
                    pref_checkbox(ui, &mut self.draft.scopes_quality_high, "Высокая точность скоупов 10-bit (High Precision)");
                });

                pref_grid_row(ui, "Яркость трассировки:", |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.draft.scopes_brightness, 0.1..=1.0)
                            .step_by(0.05)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    );
                });
            });
        });
    }
}

impl Default for PreferencesModalState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preferences_defaults_and_serde() {
        let prefs = UserPreferences::default();
        assert_eq!(prefs.default_fps, 60);
        assert_eq!(prefs.default_width, 1920);
        assert_eq!(prefs.default_height, 1080);
        assert!(prefs.snapping_enabled);
        assert_eq!(prefs.snapping_threshold_px, 10.0);
        assert!(prefs.linked_selection);
        assert_eq!(prefs.render_cache_mode, 1);
        assert_eq!(prefs.render_cache_ram_mb, 2048);
        assert!(prefs.drop_frames_enabled);
        assert!(prefs.gpu_blending_enabled);
        assert!((prefs.master_volume - 1.0).abs() < 1e-4);

        assert_eq!(prefs.zoom_step_percent, 8.0);
        assert!(!prefs.hide_titlebar);

        let json = serde_json::to_string(&prefs).expect("serialize");
        let parsed: UserPreferences = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.default_fps, prefs.default_fps);
        assert_eq!(parsed.zoom_step_percent, 8.0);
        assert!(!parsed.hide_titlebar);
        assert_eq!(parsed.render_cache_ram_mb, prefs.render_cache_ram_mb);
        assert_eq!(parsed.theme_accent_idx, prefs.theme_accent_idx);
        assert_eq!(parsed.proxy_resolution, 1);
        assert_eq!(parsed.proxy_codec, 0);
    }

    #[test]
    fn test_preferences_modal_state_sync() {
        let mut modal = PreferencesModalState::new();
        assert_eq!(modal.current_tab, PrefTab::System);
        assert_eq!(modal.current_section, PrefSection::SystemMemoryGpu);
        assert!(!modal.initialized);

        let mut custom = UserPreferences::default();
        custom.ui_scale = 2.0;
        custom.render_cache_ram_mb = 4096;
        custom.theme_accent_idx = 1; // Resolve Blue
        custom.proxy_resolution = 2;

        modal.sync_from_saved(&custom);
        assert!(modal.initialized);
        assert_eq!(modal.draft.render_cache_ram_mb, 4096);
        assert_eq!(modal.draft.theme_accent_idx, 1);
        assert_eq!(modal.draft.ui_scale, 2.0);
        assert_eq!(modal.draft.proxy_resolution, 2);
    }

    #[test]
    fn test_preferences_render_all_sections_no_panic() {
        let ctx = Context::default();
        let mut modal = PreferencesModalState::new();
        let mut open = true;

        let sections = [
            (PrefTab::System, PrefSection::SystemMemoryGpu),
            (PrefTab::System, PrefSection::SystemMediaStorage),
            (PrefTab::System, PrefSection::SystemAudio),
            (PrefTab::System, PrefSection::SystemHardware),
            (PrefTab::User, PrefSection::UserAppearance),
            (PrefTab::User, PrefSection::UserEditing),
            (PrefTab::User, PrefSection::UserPlaybackCache),
            (PrefTab::User, PrefSection::UserColorScopes),
        ];

        for (tab, sec) in sections {
            modal.current_tab = tab;
            modal.current_section = sec;

            // Тест со стандартным размером
            let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
                let _ = modal.render(ctx, &mut open);
            });
            out.textures_delta.clear();

            // Тест с критически малым размером экрана (проверка на отсутствие Negative child size)
            let mut small_input = egui::RawInput::default();
            small_input.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(10.0, 10.0)));
            let mut out_small = ctx.run_ui(small_input, |ctx| {
                let _ = modal.render(ctx, &mut open);
            });
            out_small.textures_delta.clear();
        }
    }

    #[test]
    fn test_preferences_close_button_click() {
        let ctx = Context::default();
        let mut modal = PreferencesModalState::new();
        let mut open = true;

        // Первый проход: рендер окна по центру экрана 1920x1080 (окно 920x620 -> x: 500..1420, y: 230..850)
        let mut input1 = egui::RawInput::default();
        input1.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1920.0, 1080.0)));
        let mut out1 = ctx.run_ui(input1, |ctx| {
            let _ = modal.render(ctx, &mut open);
        });
        out1.textures_delta.clear();
        assert!(open);

        let close_rect = modal.last_close_btn_rect.expect("Close button must be rendered");
        let close_pos = close_rect.center();

        // Второй проход: наведение курсора на кнопку ✕
        let mut input_hover = egui::RawInput::default();
        input_hover.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1920.0, 1080.0)));
        input_hover.events.push(egui::Event::PointerMoved(close_pos));
        let mut out_hover = ctx.run_ui(input_hover, |ctx| {
            let _ = modal.render(ctx, &mut open);
        });
        out_hover.textures_delta.clear();

        let close_pos = modal.last_close_btn_rect.expect("Close button must be rendered").center();
        let mut input2 = egui::RawInput::default();
        input2.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1920.0, 1080.0)));
        input2.events.push(egui::Event::PointerMoved(close_pos));
        input2.events.push(egui::Event::PointerButton {
            pos: close_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let mut out2 = ctx.run_ui(input2, |ctx| {
            let _ = modal.render(ctx, &mut open);
        });
        out2.textures_delta.clear();

        let mut input3 = egui::RawInput::default();
        input3.screen_rect = Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1920.0, 1080.0)));
        input3.events.push(egui::Event::PointerMoved(close_pos));
        input3.events.push(egui::Event::PointerButton {
            pos: close_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut out3 = ctx.run_ui(input3, |ctx| {
            let _ = modal.render(ctx, &mut open);
        });
        out3.textures_delta.clear();

        assert!(!open, "Clicking the ✕ button in the Preferences header must close the modal dialog");
    }
}
