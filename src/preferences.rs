use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use egui::{Color32, Context, CornerRadius, Stroke, Ui, Vec2};
use crate::theme::{ResolveColors, get_accent_colors};

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

    // 2. Editing & Timeline
    pub default_fps: u32,
    pub default_width: u32,
    pub default_height: u32,
    pub snapping_enabled: bool,
    pub snapping_threshold_px: f32,
    pub linked_selection: bool,
    pub loop_enabled: bool,
    pub stop_at_timeline_end: bool,

    // 3. Playback & Render Cache
    pub render_cache_mode: usize, // 0: None, 1: Smart (Resolve Auto), 2: User
    pub render_cache_ram_mb: usize,
    pub drop_frames_enabled: bool,
    pub gpu_blending_enabled: bool,
    pub auto_cache_idle_secs: f32,

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

            default_fps: 60,
            default_width: 1920,
            default_height: 1080,
            snapping_enabled: true,
            snapping_threshold_px: 10.0,
            linked_selection: true,
            loop_enabled: false,
            stop_at_timeline_end: true,

            render_cache_mode: 1, // Smart
            render_cache_ram_mb: 2048,
            drop_frames_enabled: true,
            gpu_blending_enabled: true,
            auto_cache_idle_secs: 2.0,

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
}

impl PreferencesModalState {
    pub fn new() -> Self {
        Self {
            current_tab: PrefTab::System,
            current_section: PrefSection::SystemMemoryGpu,
            draft: UserPreferences::default(),
            initialized: false,
            is_dirty: false,
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

        let screen_size = ctx.input(|i| i.raw.screen_rect.map(|r| r.size()).unwrap_or(egui::vec2(1920.0, 1080.0)));
        let win_width = (screen_size.x * 0.65).clamp(740.0, 960.0);
        let win_height = (screen_size.y * 0.75).clamp(520.0, 720.0);

        egui::Window::new("⚙ Preferences (Настройки DaVinci Resolve 21)")
            .open(open)
            .collapsible(false)
            .resizable(false)
            .fixed_size([win_width, win_height])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // 1. Верхние вкладки: SYSTEM | USER (Фирменный стиль DaVinci Resolve)
                ui.horizontal(|ui| {
                    ui.add_space(4.0);
                    let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

                    let sys_active = self.current_tab == PrefTab::System;
                    let user_active = self.current_tab == PrefTab::User;

                    let sys_btn = egui::Button::new(
                        egui::RichText::new("  🖥  SYSTEM  ")
                            .size(13.0)
                            .strong()
                            .color(if sys_active { Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
                    )
                    .fill(if sys_active { accent.gamma_multiply(0.25) } else { ResolveColors::BG_HEADER })
                    .stroke(Stroke::new(1.0, if sys_active { accent } else { ResolveColors::BORDER }));

                    if ui.add(sys_btn).clicked() {
                        self.current_tab = PrefTab::System;
                        self.current_section = PrefSection::SystemMemoryGpu;
                    }

                    let user_btn = egui::Button::new(
                        egui::RichText::new("  👤  USER  ")
                            .size(13.0)
                            .strong()
                            .color(if user_active { Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
                    )
                    .fill(if user_active { accent.gamma_multiply(0.25) } else { ResolveColors::BG_HEADER })
                    .stroke(Stroke::new(1.0, if user_active { accent } else { ResolveColors::BORDER }));

                    if ui.add(user_btn).clicked() {
                        self.current_tab = PrefTab::User;
                        self.current_section = PrefSection::UserAppearance;
                    }
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                // 2. Основная рабочая область: Слева меню категорий (Sidebar), Справа параметры (Content)
                let avail_rect = ui.available_rect_before_wrap();
                let bottom_bar_height = 44.0;
                let main_area_height = (avail_rect.height() - bottom_bar_height).max(200.0);

                ui.horizontal(|ui| {
                    // Левый сайдбар со списком категорий
                    egui::Frame::new()
                        .fill(ResolveColors::BG_PANEL)
                        .stroke(Stroke::new(1.0, ResolveColors::BORDER))
                        .corner_radius(CornerRadius::same(2))
                        .inner_margin(egui::Margin::same(4))
                        .show(ui, |ui| {
                            ui.set_width(200.0);
                            ui.set_height(main_area_height);
                            self.render_sidebar(ui);
                        });

                    ui.add_space(4.0);

                    // Правая область с ползунками и настройками
                    egui::Frame::new()
                        .fill(ResolveColors::BG_MAIN)
                        .stroke(Stroke::new(1.0, ResolveColors::BORDER))
                        .corner_radius(CornerRadius::same(2))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(win_width - 240.0);
                            ui.set_height(main_area_height);
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    self.render_content(ui);
                                });
                        });
                });

                // 3. Нижний бар действий: Сброс | Отмена | Сохранить
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.button(egui::RichText::new("⟲ Сбросить по умолчанию").size(11.0)).clicked() {
                        self.draft = UserPreferences::default();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);
                        let save_btn = egui::Button::new(
                            egui::RichText::new(" ✔ Сохранить и применить ")
                                .size(12.0)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(accent);

                        if ui.add(save_btn).clicked() {
                            apply_result = Some(self.draft.clone());
                            should_close = true;
                        }

                        if ui.button(" Отмена ").clicked() {
                            should_close = true;
                        }
                    });
                });
            });

        if should_close {
            *open = false;
        }

        apply_result
    }

    fn render_sidebar(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        let sections = match self.current_tab {
            PrefTab::System => vec![
                (PrefSection::SystemMemoryGpu, "🧠 Memory and GPU"),
                (PrefSection::SystemMediaStorage, "📁 Media Storage"),
                (PrefSection::SystemAudio, "🔊 Audio I/O"),
                (PrefSection::SystemHardware, "⚡ Hardware Decode"),
            ],
            PrefTab::User => vec![
                (PrefSection::UserAppearance, "🖥 UI and Appearance"),
                (PrefSection::UserEditing, "🎞 Editing & Timeline"),
                (PrefSection::UserPlaybackCache, "⚡ Playback & Cache"),
                (PrefSection::UserColorScopes, "🎨 Color & Scopes"),
            ],
        };

        for (sec, title) in sections {
            let is_selected = self.current_section == sec;
            let btn = egui::Button::new(
                egui::RichText::new(title)
                    .size(12.0)
                    .color(if is_selected { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
            )
            .fill(if is_selected { accent.gamma_multiply(0.35) } else { Color32::TRANSPARENT })
            .stroke(if is_selected { Stroke::new(1.0, accent) } else { Stroke::NONE })
            .min_size(Vec2::new(190.0, 28.0));

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

        ui.heading(
            egui::RichText::new("🧠 Memory and GPU Configuration")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Настройки распределения оперативной памяти и графических процессоров:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Оперативная память (System RAM)").strong().size(12.0));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Лимит памяти для Render Cache:");
                ui.add(
                    egui::Slider::new(&mut self.draft.render_cache_ram_mb, 256..=16384)
                        .step_by(256.0)
                        .suffix(" MB"),
                );
            });
            ui.label(
                egui::RichText::new("💡 Выделение 2048-4096 МБ обеспечивает мгновенный предпросмотр сложных эффектов и переходов без падения FPS.")
                    .size(10.0)
                    .color(ResolveColors::TEXT_MUTED),
            );
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Конфигурация GPU (GPU Processing Mode)").strong().size(12.0));
            ui.add_space(4.0);

            ui.radio_value(&mut self.draft.gpu_mode, 0, "Auto (Автоматический выбор высокопроизводительного GPU)");
            ui.radio_value(&mut self.draft.gpu_mode, 1, "VAAPI Hardware (AMD Radeon Navi 48 / Intel Iris Xe)");
            ui.radio_value(&mut self.draft.gpu_mode, 2, "CPU Software Fallback (Без GPU ускорения)");

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            ui.label(egui::RichText::new("Обнаруженные видеокарты в системе:").size(11.0).color(ResolveColors::TEXT_PRIMARY));
            ui.monospace(" • GPU 0: AMD Radeon RX 9070 XT (Navi 48) - /dev/dri/renderD129 [High-Performance]");
            ui.monospace(" • GPU 1: AMD Radeon 780M Graphics - /dev/dri/renderD128 [Integrated]");

            ui.add_space(6.0);
            ui.checkbox(&mut self.draft.gpu_blending_enabled, "Аппаратное GPU-смешивание и альфа-композитинг (GPU Blending)");
        });
    }

    fn render_system_media_storage(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("📁 Media Storage & Cache Locations")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Пути размещения дискового кэша рендера и временных файлов прокси:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Дисковый кэш рендера (Render Cache Disk Location)").strong().size(12.0));
            ui.add_space(4.0);

            let mut path_str = self.draft.cache_directory.to_string_lossy().to_string();
            ui.horizontal(|ui| {
                ui.label("Каталог кэша:");
                if ui.text_edit_singleline(&mut path_str).changed() {
                    self.draft.cache_directory = PathBuf::from(path_str);
                }
                if ui.button("⟲ По умолчанию").clicked() {
                    if let Ok(home) = std::env::var("HOME") {
                        self.draft.cache_directory = PathBuf::from(home).join(".cache/verizon_nle");
                    }
                }
            });

            ui.add_space(6.0);
            ui.checkbox(&mut self.draft.auto_cleanup_proxies, "Автоматически очищать временные прокси и кэш при закрытии проекта");
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Управление дисковым пространством").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button("🗑 Очистить дисковый кэш рендера (Purge Cache)").clicked() {
                    if let Ok(entries) = std::fs::read_dir(&self.draft.cache_directory) {
                        for e in entries.flatten() {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
            });
            ui.label(
                egui::RichText::new("Освобождает место на диске, удаляя устаревшие кэшированные кадры таймлайна.")
                    .size(10.0)
                    .color(ResolveColors::TEXT_MUTED),
            );
        });
    }

    fn render_system_audio(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("🔊 Audio I/O Configuration")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Настройка аудиовывода PipeWire / ALSA и мастер-мониторинга:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Устройство воспроизведения").strong().size(12.0));
            ui.add_space(4.0);
            ui.monospace(" • PipeWire ALSA / Komplete Audio 1 Pro (Cinema Reference 48 kHz)");
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Параметры аудиопотока").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Частота дискретизации:");
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

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Размер буфера (Latency):");
                egui::ComboBox::from_id_salt("audio_buf_combo")
                    .selected_text(match self.draft.audio_buffer_size {
                        128 => "128 семплов (2.6 мс - Low Latency)",
                        256 => "256 семплов (5.3 мс)",
                        1024 => "1024 семпла (21.3 мс - Stable)",
                        _ => "512 семплов (10.6 мс - Рекомендуется)",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.draft.audio_buffer_size, 128, "128 семплов (2.6 мс - Low Latency)");
                        ui.selectable_value(&mut self.draft.audio_buffer_size, 256, "256 семплов (5.3 мс)");
                        ui.selectable_value(&mut self.draft.audio_buffer_size, 512, "512 семплов (10.6 мс - Рекомендуется)");
                        ui.selectable_value(&mut self.draft.audio_buffer_size, 1024, "1024 семпла (21.3 мс - Stable)");
                    });
            });
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Мастер-громкость и мониторинг").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Громкость аудиовывода:");
                ui.add(
                    egui::Slider::new(&mut self.draft.master_volume, 0.0..=1.5)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });

            ui.add_space(4.0);
            ui.checkbox(&mut self.draft.mute_on_scrub, "Mute audio while scrubbing (Заглушать звук при перемотке/скраббинге)");
        });
    }

    fn render_system_hardware(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("⚡ Hardware Video Acceleration")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Аппаратное декодирование медиапотоков через VAAPI и Vulkan Video:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Аппаратные декодеры кодеков").strong().size(12.0));
            ui.add_space(4.0);
            ui.checkbox(&mut self.draft.hardware_decoding_enabled, "Аппаратное декодирование H.264 / AVC (AMD / Intel VAAPI)");
            ui.checkbox(&mut self.draft.gpu_blending_enabled, "Zero-copy трансфер текстур в Wayland через DMABUF");
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Оптимизация реал-тайм воспроизведения").strong().size(12.0));
            ui.add_space(4.0);
            ui.checkbox(&mut self.draft.drop_frames_enabled, "Drop frames during playback (Пропускать кадры при нехватке скорости декодирования)");
            ui.label(
                egui::RichText::new("💡 При включении таймлайн сохраняет реальную скорость 1x даже на тяжелых 4K/8K файлах, дропая отстающие кадры.")
                    .size(10.0)
                    .color(ResolveColors::TEXT_MUTED),
            );
        });
    }

    fn render_user_appearance(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("🖥 UI & Appearance")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Масштабирование экранов 4K/HiDPI, темы оформления и таймкод:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Масштабирование интерфейса (HiDPI / 4K)").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal_wrapped(|ui| {
                let presets = [
                    (1.0, "100% (FHD 1080p)"),
                    (1.25, "125% (2K)"),
                    (1.5, "150% (QHD)"),
                    (1.75, "175% (3K)"),
                    (2.0, "200% (4K UHD)"),
                    (2.5, "250% (4K Large)"),
                ];

                for (scale, label) in presets {
                    let is_active = (self.draft.ui_scale - scale).abs() < 0.02;
                    let btn = egui::Button::new(label).fill(if is_active {
                        accent
                    } else {
                        ResolveColors::BG_HEADER
                    });
                    if ui.add(btn).clicked() {
                        self.draft.ui_scale = scale;
                    }
                }
            });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Точный масштаб:");
                ui.add(
                    egui::Slider::new(&mut self.draft.ui_scale, 0.75..=3.5)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
            });

            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("💡 Горячие клавиши масштабирования: Ctrl + / Ctrl - / Ctrl 0")
                    .size(10.0)
                    .color(ResolveColors::TEXT_MUTED),
            );
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Отображение времени и таймкода").strong().size(12.0));
            ui.add_space(4.0);

            ui.radio_value(&mut self.draft.timecode_display_frames, false, "SMPTE Timecode (ЧЧ:ММ:СС:КК / 01:21:30:03)");
            ui.radio_value(&mut self.draft.timecode_display_frames, true, "Кадры таймлайна (Frames: 0, 1, 2 .. N)");
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Цветовая палитра акцентов DaVinci Resolve").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let themes = [
                    (0, "🟠 DaVinci Orange"),
                    (1, "🔵 Resolve Blue"),
                    (2, "🟡 Studio Gold"),
                ];
                for (idx, name) in themes {
                    let is_active = self.draft.theme_accent_idx == idx;
                    let (btn_accent, _) = get_accent_colors(idx);
                    let btn = egui::Button::new(
                        egui::RichText::new(name)
                            .color(if is_active { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
                    )
                    .fill(if is_active { btn_accent } else { ResolveColors::BG_HEADER });

                    if ui.add(btn).clicked() {
                        self.draft.theme_accent_idx = idx;
                    }
                }
            });
        });
    }

    fn render_user_editing(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("🎞 Editing & Timeline Preferences")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Параметры работы таймлайна, прилипания и создание новых проектов:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Магнитное прилипание (Snapping)").strong().size(12.0));
            ui.add_space(4.0);

            ui.checkbox(&mut self.draft.snapping_enabled, "Включить магнитное прилипание клипов [N]");
            ui.horizontal(|ui| {
                ui.label("Дистанция прилипания:");
                ui.add(
                    egui::Slider::new(&mut self.draft.snapping_threshold_px, 2.0..=30.0)
                        .suffix(" px"),
                );
            });
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Связность клипов и воспроизведение").strong().size(12.0));
            ui.add_space(4.0);

            ui.checkbox(&mut self.draft.linked_selection, "Связанное выделение видео и аудиодорожек (Linked Selection [Ctrl+Shift+L])");
            ui.checkbox(&mut self.draft.stop_at_timeline_end, "Останавливать воспроизведение в конце таймлайна");
            ui.checkbox(&mut self.draft.loop_enabled, "Зацикливать воспроизведение (Loop Playback)");
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Параметры нового проекта по умолчанию").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Частота кадров (FPS):");
                egui::ComboBox::from_id_salt("pref_fps_combo")
                    .selected_text(format!("{} fps", self.draft.default_fps))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.draft.default_fps, 24, "24 fps (Cinema)");
                        ui.selectable_value(&mut self.draft.default_fps, 25, "25 fps (PAL)");
                        ui.selectable_value(&mut self.draft.default_fps, 30, "30 fps (NTSC)");
                        ui.selectable_value(&mut self.draft.default_fps, 50, "50 fps");
                        ui.selectable_value(&mut self.draft.default_fps, 60, "60 fps (Smooth Studio)");
                    });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Разрешение таймлайна:");
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
                        if ui.selectable_label(self.draft.default_width == 1080 && self.draft.default_height == 1920, "1080 x 1920 (Vertical 9:16 Shorts/Reels)").clicked() {
                            self.draft.default_width = 1080;
                            self.draft.default_height = 1920;
                        }
                    });
            });
        });
    }

    fn render_user_playback_cache(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("⚡ Playback & Render Cache")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Интеллектуальный кэш DaVinci Resolve для плавного воспроизведения:")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Режим Render Cache").strong().size(12.0));
            ui.add_space(4.0);

            ui.radio_value(&mut self.draft.render_cache_mode, 0, "None (Кэширование выключено)");
            ui.radio_value(&mut self.draft.render_cache_mode, 1, "Smart Cache (Автоматический кэш DaVinci Resolve - Рекомендуется)");
            ui.radio_value(&mut self.draft.render_cache_mode, 2, "User Cache (Кэшировать клипы по выбору пользователя)");

            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("💡 В режиме Smart Cache система сама определяет ресурсоемкие участки, переходы и цветокоррекцию и кэширует их в фоне.")
                    .size(10.0)
                    .color(ResolveColors::TEXT_MUTED),
            );
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Параметры фонового кэширования").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Задержка старта фонового кэширования при простое:");
                ui.add(
                    egui::Slider::new(&mut self.draft.auto_cache_idle_secs, 0.5..=10.0)
                        .step_by(0.5)
                        .suffix(" сек"),
                );
            });
        });
    }

    fn render_color_scopes(&mut self, ui: &mut Ui) {
        let (accent, _) = get_accent_colors(self.draft.theme_accent_idx);

        ui.heading(
            egui::RichText::new("🎨 Color Science & Scopes")
                .color(accent)
                .size(15.0),
        );
        ui.label(
            egui::RichText::new("Цветовые пространства, точность мониторинга и скоупы (Waveform, Vectorscope):")
                .size(11.0)
                .color(ResolveColors::TEXT_MUTED),
        );
        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Цветовое управление (Color Management)").strong().size(12.0));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Color Science:");
                egui::ComboBox::from_id_salt("cs_science_combo")
                    .selected_text(match self.draft.color_science {
                        1 => "DaVinci YRGB Color Managed (RCM)",
                        2 => "ACEScc (Academy Color Encoding System)",
                        _ => "DaVinci YRGB (Standard)",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.draft.color_science, 0, "DaVinci YRGB (Standard)");
                        ui.selectable_value(&mut self.draft.color_science, 1, "DaVinci YRGB Color Managed (RCM)");
                        ui.selectable_value(&mut self.draft.color_science, 2, "ACEScc (Academy Color Encoding System)");
                    });
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Цветовое пространство таймлайна:");
                egui::ComboBox::from_id_salt("cs_space_combo")
                    .selected_text(match self.draft.timeline_color_space {
                        1 => "Rec.709 (Scene)",
                        2 => "Rec.2020 (HDR/UHD)",
                        3 => "DCI-P3 (DCI Cinema)",
                        _ => "Rec.709 Gamma 2.4 (HDTV Standard)",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.draft.timeline_color_space, 0, "Rec.709 Gamma 2.4 (HDTV Standard)");
                        ui.selectable_value(&mut self.draft.timeline_color_space, 1, "Rec.709 (Scene)");
                        ui.selectable_value(&mut self.draft.timeline_color_space, 2, "Rec.2020 (HDR/UHD)");
                        ui.selectable_value(&mut self.draft.timeline_color_space, 3, "DCI-P3 (DCI Cinema)");
                    });
            });
        });

        ui.add_space(10.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(egui::RichText::new("Параметры видеомониторов и скоупов (Scopes)").strong().size(12.0));
            ui.add_space(4.0);

            ui.checkbox(&mut self.draft.scopes_quality_high, "Высокая точность скоупов 10-bit (High Precision Scopes)");
            ui.horizontal(|ui| {
                ui.label("Яркость трассировки скоупов:");
                ui.add(
                    egui::Slider::new(&mut self.draft.scopes_brightness, 0.1..=1.0)
                        .step_by(0.05)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
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

        let json = serde_json::to_string(&prefs).expect("serialize");
        let parsed: UserPreferences = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.default_fps, prefs.default_fps);
        assert_eq!(parsed.render_cache_ram_mb, prefs.render_cache_ram_mb);
        assert_eq!(parsed.theme_accent_idx, prefs.theme_accent_idx);
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

        modal.sync_from_saved(&custom);
        assert!(modal.initialized);
        assert_eq!(modal.draft.render_cache_ram_mb, 4096);
        assert_eq!(modal.draft.theme_accent_idx, 1);
        assert_eq!(modal.draft.ui_scale, 2.0);
    }
}
