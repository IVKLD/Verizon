use egui::{Color32, Context, CornerRadius, Stroke, Visuals};

/// Фирменные цвета DaVinci Resolve (Blackmagic Design)
pub struct ResolveColors;

impl ResolveColors {
    pub const BG_MAIN: Color32 = Color32::from_rgb(0x15, 0x16, 0x18);
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x1c, 0x1d, 0x20);
    pub const BG_HEADER: Color32 = Color32::from_rgb(0x22, 0x24, 0x28);
    pub const BG_INPUT: Color32 = Color32::from_rgb(0x13, 0x14, 0x16);
    pub const BORDER: Color32 = Color32::from_rgb(0x10, 0x11, 0x13);
    pub const BORDER_LIGHT: Color32 = Color32::from_rgb(0x32, 0x35, 0x3c);

    // Фирменный оранжевый акцент Resolve (Playhead, активные кнопки, маркеры)
    pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(0xe8, 0x7c, 0x24);
    pub const ACCENT_ORANGE_HOVER: Color32 = Color32::from_rgb(0xf5, 0x8f, 0x3d);

    // Цвета дорожек и клипов
    pub const VIDEO_CLIP: Color32 = Color32::from_rgb(0x2a, 0x58, 0x85);
    pub const VIDEO_CLIP_SELECTED: Color32 = Color32::from_rgb(0x38, 0x73, 0xad);
    pub const AUDIO_CLIP: Color32 = Color32::from_rgb(0x23, 0x69, 0x48);
    pub const AUDIO_CLIP_SELECTED: Color32 = Color32::from_rgb(0x2d, 0x89, 0x5e);

    // Текст
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xd8, 0xdc, 0xe2);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x7a, 0x80, 0x8c);
    pub const TEXT_TIMECODE: Color32 = Color32::from_rgb(0xeb, 0xee, 0xf2);

    // VU Meter цвета
    pub const VU_GREEN: Color32 = Color32::from_rgb(0x2d, 0xbc, 0x5c);
    pub const VU_YELLOW: Color32 = Color32::from_rgb(0xd6, 0xa5, 0x24);
    pub const VU_RED: Color32 = Color32::from_rgb(0xd9, 0x38, 0x38);
}

/// Применение визуальной темы DaVinci Resolve к egui Context
pub fn apply_resolve_theme(ctx: &Context) {
    let mut visuals = Visuals::dark();

    // Панели и окна
    visuals.panel_fill = ResolveColors::BG_MAIN;
    visuals.window_fill = ResolveColors::BG_PANEL;
    visuals.window_stroke = Stroke::new(1.0, ResolveColors::BORDER);
    visuals.window_corner_radius = CornerRadius::same(2);

    // Неинтерактивные элементы
    visuals.widgets.noninteractive.bg_fill = ResolveColors::BG_PANEL;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, ResolveColors::BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, ResolveColors::TEXT_PRIMARY);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(2);

    // Кнопки и неактивные поля
    visuals.widgets.inactive.bg_fill = ResolveColors::BG_HEADER;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, ResolveColors::BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, ResolveColors::TEXT_PRIMARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(2);

    // При наведении (Hover)
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x2d, 0x30, 0x36);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ResolveColors::ACCENT_ORANGE);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(2);

    // При нажатии / активные
    visuals.widgets.active.bg_fill = ResolveColors::ACCENT_ORANGE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ResolveColors::ACCENT_ORANGE_HOVER);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(2);

    // Поля ввода текста
    visuals.widgets.open.bg_fill = ResolveColors::BG_INPUT;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ResolveColors::ACCENT_ORANGE);

    visuals.selection.bg_fill = ResolveColors::ACCENT_ORANGE.gamma_multiply(0.4);
    visuals.selection.stroke = Stroke::new(1.0, ResolveColors::ACCENT_ORANGE);

    ctx.set_visuals(visuals);
}
