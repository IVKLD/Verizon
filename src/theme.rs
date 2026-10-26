use egui::{Color32, Context, CornerRadius, Stroke, Visuals};

/// Фирменные цвета DaVinci Resolve 21 (Blackmagic Design)
pub struct ResolveColors;

impl ResolveColors {
    // Фоновые цвета панелей
    pub const BG_MAIN: Color32 = Color32::from_rgb(0x13, 0x14, 0x17);
    pub const BG_PANEL: Color32 = Color32::from_rgb(0x1a, 0x1b, 0x1f);
    pub const BG_HEADER: Color32 = Color32::from_rgb(0x23, 0x25, 0x2a);
    pub const BG_INPUT: Color32 = Color32::from_rgb(0x10, 0x11, 0x13);
    pub const BORDER: Color32 = Color32::from_rgb(0x11, 0x12, 0x15);
    pub const BORDER_LIGHT: Color32 = Color32::from_rgb(0x33, 0x36, 0x3e);

    // Фирменный сочный оранжевый акцент Resolve (Playhead, активные инструменты, выделения)
    pub const ACCENT_ORANGE: Color32 = Color32::from_rgb(0xff, 0x6e, 0x1f);
    pub const ACCENT_ORANGE_HOVER: Color32 = Color32::from_rgb(0xff, 0x85, 0x3d);

    // Дополнительные акцентные цвета Resolve (Fairlight / Cut / Fusion)
    pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x2f, 0x7b, 0xe5);
    pub const ACCENT_BLUE_HOVER: Color32 = Color32::from_rgb(0x4a, 0x90, 0xeb);
    pub const ACCENT_GOLD: Color32 = Color32::from_rgb(0xf5, 0xa6, 0x23);
    pub const ACCENT_GOLD_HOVER: Color32 = Color32::from_rgb(0xf7, 0xb8, 0x4f);

    // Цвета дорожек и клипов (DaVinci Resolve 21 Color Palette)
    pub const VIDEO_CLIP: Color32 = Color32::from_rgb(0x27, 0x62, 0x97);
    pub const VIDEO_CLIP_SELECTED: Color32 = Color32::from_rgb(0x38, 0x82, 0xc4);

    pub const AUDIO_CLIP: Color32 = Color32::from_rgb(0x24, 0x7a, 0x54);
    pub const AUDIO_CLIP_SELECTED: Color32 = Color32::from_rgb(0x32, 0x9e, 0x6e);

    // Текст
    pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xeb, 0xef, 0xf5);
    pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x82, 0x88, 0x96);
    pub const TEXT_TIMECODE: Color32 = Color32::from_rgb(0xf2, 0xf5, 0xfa);

    // VU Meter цвета DaVinci Fairlight
    pub const VU_GREEN: Color32 = Color32::from_rgb(0x22, 0xc5, 0x5e);
    pub const VU_YELLOW: Color32 = Color32::from_rgb(0xfa, 0xcc, 0x15);
    pub const VU_RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
}

/// Получить цвет акцента по индексу (0: DaVinci Orange, 1: Resolve Blue, 2: Studio Gold)
pub fn get_accent_colors(idx: usize) -> (Color32, Color32) {
    match idx {
        1 => (ResolveColors::ACCENT_BLUE, ResolveColors::ACCENT_BLUE_HOVER),
        2 => (ResolveColors::ACCENT_GOLD, ResolveColors::ACCENT_GOLD_HOVER),
        _ => (ResolveColors::ACCENT_ORANGE, ResolveColors::ACCENT_ORANGE_HOVER),
    }
}

/// Применение визуальной темы DaVinci Resolve с выбранным акцентным цветом
pub fn apply_resolve_theme_with_accent(ctx: &Context, accent_idx: usize) {
    let (accent, accent_hover) = get_accent_colors(accent_idx);
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
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(2);

    // При нажатии / активные
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent_hover);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(2);

    // Поля ввода текста
    visuals.widgets.open.bg_fill = ResolveColors::BG_INPUT;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, accent);

    visuals.selection.bg_fill = accent.gamma_multiply(0.4);
    visuals.selection.stroke = Stroke::new(1.0, accent);

    ctx.set_visuals(visuals);
}
