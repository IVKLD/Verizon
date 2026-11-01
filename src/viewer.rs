use egui::{pos2, vec2, Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, Ui};
use crate::theme::ResolveColors;
use crate::timeline::TimelineState;
use crate::timecode::Timecode;
use crate::video_decoder::VideoPlayer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomMode {
    Fit,
    Fifty,
    Hundred,
    TwoHundred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectMask {
    Off,
    Cinema239, // 2.39:1 Anamorphic
    Cinema185, // 1.85:1 Flat
    Vertical916, // 9:16 Shorts/Reels
    Square11,    // 1:1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChannelMode {
    RGB,
    Red,
    Green,
    Blue,
    Monochrome,
}

pub struct ViewerState {
    pub single_viewer_mode: bool,
    pub zoom_mode: ZoomMode,
    pub aspect_mask: AspectMask,
    pub show_safe_areas: bool,
    pub show_rule_of_thirds: bool,
    pub show_center_cross: bool,
    pub channel_mode: ColorChannelMode,
    pub show_transform_overlay: bool,
    pub cinema_mode: bool,

    pub pan_offset: egui::Vec2,
    pub custom_zoom: Option<f32>,

    /// Сырые (непримагниченные) координаты и масштаб при перетаскивании Transform OSC,
    /// исключающие «залипание» клипа на границе магнитного порога при малых дельтах мыши
    pub transform_raw_pos: Option<(f32, f32)>,
    pub transform_raw_zoom: Option<(f32, f32)>,

    pub vu_level_left: f32,
    pub vu_level_right: f32,
}

impl ViewerState {
    pub fn new() -> Self {
        Self {
            single_viewer_mode: false,
            zoom_mode: ZoomMode::Fit,
            aspect_mask: AspectMask::Off,
            show_safe_areas: false,
            show_rule_of_thirds: false,
            show_center_cross: false,
            channel_mode: ColorChannelMode::RGB,
            show_transform_overlay: true,
            cinema_mode: false,

            pan_offset: egui::Vec2::ZERO,
            custom_zoom: None,
            transform_raw_pos: None,
            transform_raw_zoom: None,

            vu_level_left: 0.65,
            vu_level_right: 0.62,
        }
    }

    /// Магнитное прилипание позиции клипа (Transform OSC) к краям экрана 1920x1080 (±960, ±540) и центру (0, 0)
    /// с учетом текущего масштаба и обрезки (Crop). Возвращает `(snapped_x, snapped_y, snap_guide_x, snap_guide_y)`.
    pub fn snap_transform_position(
        raw_x: f32,
        raw_y: f32,
        clip: &crate::timeline::Clip,
        snap_proj_threshold: f32,
    ) -> (f32, f32, Option<f32>, Option<f32>) {
        let half_w = 960.0 * clip.zoom_x;
        let half_h = 540.0 * clip.zoom_y;
        let left_off = -half_w + (2.0 * half_w) * (clip.crop_left / 100.0);
        let right_off = half_w - (2.0 * half_w) * (clip.crop_right / 100.0);
        let top_off = -half_h + (2.0 * half_h) * (clip.crop_top / 100.0);
        let bottom_off = half_h - (2.0 * half_h) * (clip.crop_bottom / 100.0);

        let mut snapped_x = raw_x;
        let mut guide_x = None;
        let x_candidates = [
            (-960.0 - left_off, -960.0),  // Левый край клипа к левому краю экрана
            (960.0 - right_off, 960.0),   // Правый край клипа к правому краю экрана
            (0.0, 0.0),                   // Центр клипа к центру экрана X
            (-left_off, 0.0),             // Левый край клипа к центру экрана X
            (-right_off, 0.0),            // Правый край клипа к центру экрана X
        ];
        let mut best_dx = snap_proj_threshold + 1e-4;
        for (target_pos, guide_coord) in x_candidates {
            let dist = (raw_x - target_pos).abs();
            if dist < best_dx {
                best_dx = dist;
                snapped_x = target_pos;
                guide_x = Some(guide_coord);
            }
        }

        let mut snapped_y = raw_y;
        let mut guide_y = None;
        let y_candidates = [
            (-540.0 - top_off, -540.0),    // Верхний край клипа к верхнему краю экрана
            (540.0 - bottom_off, 540.0),   // Нижний край клипа к нижнему краю экрана
            (0.0, 0.0),                    // Центр клипа к центру экрана Y
            (-top_off, 0.0),               // Верхний край клипа к центру экрана Y
            (-bottom_off, 0.0),            // Нижний край клипа к центру экрана Y
        ];
        let mut best_dy = snap_proj_threshold + 1e-4;
        for (target_pos, guide_coord) in y_candidates {
            let dist = (raw_y - target_pos).abs();
            if dist < best_dy {
                best_dy = dist;
                snapped_y = target_pos;
                guide_y = Some(guide_coord);
            }
        }

        (snapped_x, snapped_y, guide_x, guide_y)
    }

    /// Магнитное прилипание масштаба клипа (Transform Resize) к границам экрана 1920x1080 (±960, ±540) и масштабу 1.0x.
    /// Возвращает `(snapped_zx, snapped_zy, snap_guide_x, snap_guide_y)`.
    pub fn snap_transform_zoom(
        raw_zx: f32,
        raw_zy: f32,
        clip: &crate::timeline::Clip,
        uniform: bool,
        snap_proj_threshold: f32,
    ) -> (f32, f32, Option<f32>, Option<f32>) {
        let ratio = if raw_zx.abs() > 0.001 { raw_zy / raw_zx } else { 1.0 };
        let l_factor = -960.0 * (1.0 - 2.0 * (clip.crop_left / 100.0));
        let r_factor = 960.0 * (1.0 - 2.0 * (clip.crop_right / 100.0));
        let t_factor = -540.0 * (1.0 - 2.0 * (clip.crop_top / 100.0));
        let b_factor = 540.0 * (1.0 - 2.0 * (clip.crop_bottom / 100.0));

        let mut snapped_zx = raw_zx;
        let mut snapped_zy = raw_zy;
        let mut guide_x = None;
        let mut guide_y = None;

        // Проверка прилипания по X (левый край к -960, правый к +960, или масштаб 1.0)
        let mut x_targets: Vec<(f32, Option<f32>)> = vec![(1.0, None)];
        if l_factor.abs() > 1.0 {
            let zx_l = (-960.0 - clip.position_x) / l_factor;
            if zx_l > 0.05 {
                x_targets.push((zx_l, Some(-960.0)));
            }
        }
        if r_factor.abs() > 1.0 {
            let zx_r = (960.0 - clip.position_x) / r_factor;
            if zx_r > 0.05 {
                x_targets.push((zx_r, Some(960.0)));
            }
        }

        let mut best_dx_px = snap_proj_threshold;
        for (cand_zx, g_x) in x_targets {
            let err_px = (raw_zx - cand_zx).abs() * 960.0;
            if err_px <= best_dx_px {
                best_dx_px = err_px;
                snapped_zx = cand_zx;
                guide_x = g_x;
                if uniform {
                    snapped_zy = (cand_zx * ratio).clamp(0.05, 10.0);
                }
            }
        }

        // Проверка прилипания по Y (верхний край к -540, нижний к +540, или масштаб 1.0)
        let mut y_targets: Vec<(f32, Option<f32>)> = vec![(1.0, None)];
        if t_factor.abs() > 1.0 {
            let zy_t = (-540.0 - clip.position_y) / t_factor;
            if zy_t > 0.05 {
                y_targets.push((zy_t, Some(-540.0)));
            }
        }
        if b_factor.abs() > 1.0 {
            let zy_b = (540.0 - clip.position_y) / b_factor;
            if zy_b > 0.05 {
                y_targets.push((zy_b, Some(540.0)));
            }
        }

        let mut best_dy_px = snap_proj_threshold;
        for (cand_zy, g_y) in y_targets {
            let err_px = (raw_zy - cand_zy).abs() * 540.0;
            if err_px <= best_dy_px && (!uniform || err_px < best_dx_px) {
                best_dy_px = err_px;
                snapped_zy = cand_zy;
                guide_y = g_y;
                if uniform && ratio.abs() > 0.001 {
                    snapped_zx = (cand_zy / ratio).clamp(0.05, 10.0);
                }
            }
        }

        (snapped_zx.clamp(0.05, 10.0), snapped_zy.clamp(0.05, 10.0), guide_x, guide_y)
    }

    pub fn render_dual_viewers(
        &mut self,
        ui: &mut Ui,
        timeline: &mut TimelineState,
        video_player: &mut VideoPlayer,
        render_cache: &mut crate::render_cache::RenderCacheManager,
        vu_levels: (f32, f32),
        ctx: &egui::Context,
    ) {
        // 1. Проверяем Render Cache: если кадр уже отрендерен — мгновенный 0 мс доступ без вызова FFmpeg/DRM
        let current_sig = crate::render_cache::RenderCacheManager::compute_decode_signature(timeline, timeline.playhead_frame);
        let cached_img = if render_cache.mode != crate::render_cache::RenderCacheMode::None {
            render_cache.get(timeline.playhead_frame, current_sig)
        } else {
            None
        };

        // Декодирование текущего кадра активного клипа на таймлайне (приоритет: самая верхняя видеодорожка и последний перемещенный клип)
        let active_clip = timeline.active_video_clip_at(timeline.playhead_frame);

        if let Some(clip) = active_clip {
            if let Some(path) = &clip.source_path {
                let need_file_open = cached_img.is_none() || (timeline.is_playing && !timeline.is_scrubbing && {
                    let lookahead_end = (timeline.playhead_frame + 24).min(clip.end_frame());
                    (timeline.playhead_frame..lookahead_end).any(|f| !render_cache.is_cached(f))
                });
                if need_file_open && video_player.current_path.as_deref() != Some(path.as_path()) {
                    let _ = video_player.open_file(path, ctx);
                }
                let source_frame = clip.source_frame_from_timeline(timeline.playhead_frame, timeline.fps);
                if timeline.is_playing && !timeline.is_scrubbing {
                    // Вычисляем ближайшую монтажную склейку (конец текущего клипа или начало клипа на более приоритетной дорожке V2 > V1)
                    let clip_prio = (clip.track_index, if timeline.selected_clip_id == Some(clip.id) { 0usize } else { 1usize });
                    let clip_end = clip.start_frame + clip.duration_frames;
                    let cut_frame = timeline
                        .clips
                        .iter()
                        .filter(|c| {
                            !c.is_audio
                                && c.id != clip.id
                                && c.start_frame > timeline.playhead_frame
                                && c.start_frame < clip_end
                                && (c.track_index, if timeline.selected_clip_id == Some(c.id) { 0usize } else { 1usize }) < clip_prio
                        })
                        .map(|c| c.start_frame)
                        .min()
                        .unwrap_or(clip_end);

                    let last_tl_frame = (cut_frame - 1).max(clip.start_frame);
                    let last_src = ((clip.source_in_secs + (last_tl_frame - clip.start_frame) as f64 / timeline.fps.max(1) as f64) * clip.source_fps).round() as i64;

                    let next_clip = timeline
                        .clips
                        .iter()
                        .filter(|c| !c.is_audio && c.contains_frame(cut_frame))
                        .min_by_key(|c| (c.track_index, if timeline.selected_clip_id == Some(c.id) { 0usize } else { 1usize }))
                        .or_else(|| {
                            timeline
                                .clips
                                .iter()
                                .filter(|c| !c.is_audio && c.start_frame >= cut_frame)
                                .min_by_key(|c| (c.start_frame, c.track_index))
                        });

                    let upcoming_cut = next_clip.and_then(|nc| {
                        if nc.id != clip.id && nc.source_path.as_deref() == Some(path.as_path()) {
                            let nc_tl_frame = cut_frame.max(nc.start_frame);
                            let next_src = ((nc.source_in_secs + (nc_tl_frame - nc.start_frame) as f64 / timeline.fps.max(1) as f64) * nc.source_fps).round() as i64;
                            Some((last_src, next_src))
                        } else {
                            None
                        }
                    });

                    video_player.update_playback_with_cut(source_frame, true, upcoming_cut, cached_img.clone(), ctx);
                } else {
                    video_player.update_playback_with_cut(source_frame, false, None, cached_img.clone(), ctx);
                }
            } else {
                if timeline.is_playing && !timeline.is_scrubbing {
                    video_player.update_playback_with_cut(timeline.playhead_frame, true, None, cached_img.clone(), ctx);
                } else {
                    video_player.update_playback_with_cut(timeline.playhead_frame, false, None, cached_img.clone(), ctx);
                }
            }
        } else {
            // Если под плейхедом пауза — отображаем чистый кадр и при воспроизведении заранее готовим декодер к следующему клипу
            let prefetch_src = if timeline.is_playing {
                timeline
                    .clips
                    .iter()
                    .filter(|c| !c.is_audio && c.start_frame > timeline.playhead_frame)
                    .min_by_key(|c| (c.start_frame, c.track_index))
                    .and_then(|c| {
                        if c.source_path.as_deref() == video_player.current_path.as_deref() {
                            Some((c.source_in_secs * c.source_fps).round() as i64)
                        } else {
                            None
                        }
                    })
            } else {
                None
            };
            video_player.show_blank_frame_with_prefetch(prefetch_src, ctx);
        }

        // Автоматически сохраняем декодированные кадры в Render Cache
        if render_cache.mode != crate::render_cache::RenderCacheMode::None {
            let mut inserted_any = false;

            // 1) Точное кэширование текущего положения плейхеда при кликах и на паузе:
            // СТРОГИЙ ИНВАРИАНТ: кэшируем ТОЛЬКО при 100% точном совпадении (last_exact_frame == target_src и last_decoded_frame == target_src),
            // исключая отравление Render Cache приблизительными кадрами скраб-кэша или соседними кадрами!
            if let Some(clip) = active_clip {
                let is_same_file = clip.source_path.as_deref() == video_player.current_path.as_deref();
                if is_same_file {
                    let target_src = clip.source_frame_from_timeline(timeline.playhead_frame, timeline.fps);
                    if let Some(ref img) = video_player.last_displayed_image {
                        let presented_src = video_player.last_exact_frame;
                        let is_exact_display = presented_src == video_player.last_decoded_frame;
                        let is_eof_hold = video_player.total_frames > 0
                            && target_src >= video_player.total_frames - 2
                            && presented_src >= video_player.total_frames - 2;
                        let is_match = is_exact_display && (presented_src == target_src || is_eof_hold);
                        let is_top = timeline.active_video_clip_at(timeline.playhead_frame).map(|ac| ac.id) == Some(clip.id);
                        if is_match && is_top && clip.contains_frame(timeline.playhead_frame) {
                            let sig = crate::render_cache::RenderCacheManager::compute_decode_signature(timeline, timeline.playhead_frame);
                            if !render_cache.is_cached(timeline.playhead_frame) {
                                render_cache.insert(timeline.playhead_frame, std::sync::Arc::clone(img), sig);
                                inserted_any = true;
                            }
                            if is_eof_hold {
                                for tl_f in timeline.playhead_frame..clip.end_frame() {
                                    if clip.contains_frame(tl_f) && timeline.active_video_clip_at(tl_f).map(|ac| ac.id) == Some(clip.id) {
                                        let sig = crate::render_cache::RenderCacheManager::compute_decode_signature(timeline, tl_f);
                                        if !render_cache.is_cached(tl_f) {
                                            render_cache.insert(tl_f, std::sync::Arc::clone(img), sig);
                                            inserted_any = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 2) Кэширование всех точно декодированных кадров из фоновой очереди (безусловная запись точных кадров)
            if let Some(ref current_path) = video_player.current_path {
                let current_path_buf = current_path.clone();
                for (src_f, img) in video_player.newly_cached_frames.drain(..) {
                    for clip in &timeline.clips {
                        if clip.is_audio || clip.is_disabled {
                            continue;
                        }
                        if clip.source_path.as_ref() == Some(&current_path_buf) {
                            let base_tl = clip.timeline_frame_from_source(src_f, timeline.fps);
                            let search_range = ((timeline.fps as f64 / clip.source_fps.max(1.0) / clip.speed.abs().max(0.01)).ceil() as i64).max(2);
                            for tl_f in (base_tl - search_range)..=(base_tl + search_range) {
                                if clip.contains_frame(tl_f) && timeline.active_video_clip_at(tl_f).map(|ac| ac.id) == Some(clip.id) {
                                    let tl_src = clip.source_frame_from_timeline(tl_f, timeline.fps);
                                    let is_match = tl_src == src_f
                                        || (video_player.total_frames > 0 && src_f >= video_player.total_frames - 2 && tl_src >= video_player.total_frames - 2);
                                    if is_match {
                                        let sig = crate::render_cache::RenderCacheManager::compute_decode_signature(timeline, tl_f);
                                        let was_cached = render_cache.is_cached(tl_f);
                                        render_cache.insert(tl_f, std::sync::Arc::clone(&img), sig);
                                        if !was_cached {
                                            inserted_any = true;
                                        }
                                    }
                                }
                            }

                            // Если это последний кадр файла (хвост клипа), заполняем ВСЕ оставшиеся кадры клипа до его конца (Hold Frame),
                            // устраняя перманентные красные сектора из-за разницы длительностей видео и контейнера!
                            if video_player.total_frames > 0 && src_f >= video_player.total_frames - 2 {
                                for tl_f in base_tl..clip.end_frame() {
                                    if clip.contains_frame(tl_f) && timeline.active_video_clip_at(tl_f).map(|ac| ac.id) == Some(clip.id) {
                                        let sig = crate::render_cache::RenderCacheManager::compute_decode_signature(timeline, tl_f);
                                        if !render_cache.is_cached(tl_f) {
                                            render_cache.insert(tl_f, std::sync::Arc::clone(&img), sig);
                                            inserted_any = true;
                                        }
                                    }
                                }
                            }

                            // Если это самый первый кадр клипа, гарантируем кэширование от самого начала клипа clip.start_frame
                            if src_f == 0 && clip.source_in_secs <= 0.001 {
                                for tl_f in clip.start_frame..=base_tl {
                                    if clip.contains_frame(tl_f) && timeline.active_video_clip_at(tl_f).map(|ac| ac.id) == Some(clip.id) {
                                        let sig = crate::render_cache::RenderCacheManager::compute_decode_signature(timeline, tl_f);
                                        if !render_cache.is_cached(tl_f) {
                                            render_cache.insert(tl_f, std::sync::Arc::clone(&img), sig);
                                            inserted_any = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                video_player.newly_cached_frames.clear();
            }

            if inserted_any {
                ctx.request_repaint();
            }
        } else {
            video_player.newly_cached_frames.clear();
        }

        // Обновление реальных уровней громкости VU-метра
        if timeline.is_playing {
            self.vu_level_left = (vu_levels.0 * 1.5).clamp(0.0, 1.0);
            self.vu_level_right = (vu_levels.1 * 1.5).clamp(0.0, 1.0);
        } else {
            self.vu_level_left = (self.vu_level_left - 0.1).max(0.0);
            self.vu_level_right = (self.vu_level_right - 0.1).max(0.0);
        }

        // ВЕРХНИЙ ТУЛБАР МОНИТОРА RESOLVE (Display Modes)
        self.render_viewer_toolbar(ui, video_player, render_cache);
        ui.separator();

        // ОБЛАСТЬ МОНИТОРОВ: Одиночный или Двойной режим (в Cinema Mode всегда полноэкранный программный монитор)
        if self.single_viewer_mode || self.cinema_mode {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                self.render_program_viewer(ui, timeline, video_player);
            });
        } else {
            ui.columns(2, |cols| {
                egui::Frame::group(cols[0].style()).show(&mut cols[0], |ui| {
                    self.render_source_viewer(ui, video_player);
                });
                egui::Frame::group(cols[1].style()).show(&mut cols[1], |ui| {
                    self.render_program_viewer(ui, timeline, video_player);
                });
            });
        }
    }

    fn render_viewer_toolbar(&mut self, ui: &mut Ui, video_player: &mut VideoPlayer, render_cache: &mut crate::render_cache::RenderCacheManager) {
        ui.horizontal(|ui| {
            // Кнопка Скрыть / Раскрыть Source
            let src_text = if self.single_viewer_mode { "👁 Source" } else { "👁 Source" };
            let src_btn = egui::Button::new(
                egui::RichText::new(src_text)
                    .size(11.0)
                    .strong()
                    .color(if self.single_viewer_mode { ResolveColors::TEXT_PRIMARY } else { Color32::WHITE }),
            )
            .fill(if self.single_viewer_mode { ResolveColors::BG_HEADER } else { ResolveColors::ACCENT_ORANGE });
            if ui.add(src_btn).on_hover_text(if self.single_viewer_mode { "Раскрыть Source Viewer (Dual Monitor)" } else { "Скрыть Source Viewer (100% Timeline)" }).clicked() {
                self.single_viewer_mode = !self.single_viewer_mode;
            }

            ui.separator();

            // 1. Zoom Mode (Fit / 50% / 100% / 200% / Custom smooth zoom)
            let custom_str;
            let zoom_label = if let Some(cz) = self.custom_zoom {
                custom_str = format!("{:.0}%", cz * 100.0);
                &custom_str
            } else {
                match self.zoom_mode {
                    ZoomMode::Fit => "Fit",
                    ZoomMode::Fifty => "50%",
                    ZoomMode::Hundred => "100%",
                    ZoomMode::TwoHundred => "200%",
                }
            };
            egui::ComboBox::from_id_salt("viewer_zoom_combo")
                .selected_text(zoom_label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.custom_zoom.is_none() && self.zoom_mode == ZoomMode::Fit, "Fit").clicked() {
                        self.zoom_mode = ZoomMode::Fit;
                        self.custom_zoom = None;
                        self.pan_offset = egui::Vec2::ZERO;
                    }
                    if ui.selectable_label(self.custom_zoom.is_none() && self.zoom_mode == ZoomMode::Fifty, "50%").clicked() {
                        self.zoom_mode = ZoomMode::Fifty;
                        self.custom_zoom = None;
                        self.pan_offset = egui::Vec2::ZERO;
                    }
                    if ui.selectable_label(self.custom_zoom.is_none() && self.zoom_mode == ZoomMode::Hundred, "100%").clicked() {
                        self.zoom_mode = ZoomMode::Hundred;
                        self.custom_zoom = None;
                        self.pan_offset = egui::Vec2::ZERO;
                    }
                    if ui.selectable_label(self.custom_zoom.is_none() && self.zoom_mode == ZoomMode::TwoHundred, "200%").clicked() {
                        self.zoom_mode = ZoomMode::TwoHundred;
                        self.custom_zoom = None;
                        self.pan_offset = egui::Vec2::ZERO;
                    }
                });

            ui.separator();

            // 2. Aspect Ratio / Framing Masks (Каше: 2.39:1, 9:16 Shorts...)
            let mask_label = match self.aspect_mask {
                AspectMask::Off => "16:9",
                AspectMask::Cinema239 => "2.39:1",
                AspectMask::Cinema185 => "1.85:1",
                AspectMask::Vertical916 => "9:16",
                AspectMask::Square11 => "1:1",
            };
            egui::ComboBox::from_id_salt("viewer_aspect_combo")
                .selected_text(mask_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Off, "Off (16:9)");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Cinema239, "2.39:1 Cinema");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Cinema185, "1.85:1 Flat");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Vertical916, "9:16 Shorts");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Square11, "1:1 Square");
                });

            ui.separator();

            // 3. Сетки и направляющие (Guides: Thirds, Safe, Cross)
            ui.menu_button("📐 Guides", |ui| {
                ui.checkbox(&mut self.show_safe_areas, "Safe Action / Title");
                ui.checkbox(&mut self.show_rule_of_thirds, "Rule of Thirds");
                ui.checkbox(&mut self.show_center_cross, "Center Crosshair");
            });

            // 4. Изоляция цветовых каналов (Channel Mode: RGB, R, G, B, Mono)
            ui.menu_button("🎨 Channels", |ui| {
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::RGB, "RGB (Full)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Red, "Red (R)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Green, "Green (G)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Blue, "Blue (B)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Monochrome, "Monochrome");
            });

            // 5. Интерактивная рамка трансформации на мониторе (Transform OSC)
            let tf_btn = egui::Button::new(
                egui::RichText::new("⛶ Transform")
                    .size(10.0)
                    .color(if self.show_transform_overlay { Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
            )
            .fill(if self.show_transform_overlay { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(tf_btn).on_hover_text("Интерактивная рамка трансформации клипа на мониторе").clicked() {
                self.show_transform_overlay = !self.show_transform_overlay;
            }

            ui.separator();

            // 6. Селектор режима Render Cache (DaVinci Resolve Smart Cache)
            let stats = render_cache.stats();
            let cache_label = match stats.mode {
                crate::render_cache::RenderCacheMode::None => "⚡ Off",
                crate::render_cache::RenderCacheMode::Smart => "⚡ Smart",
                crate::render_cache::RenderCacheMode::User => "⚡ User",
            };
            egui::ComboBox::from_id_salt("viewer_render_cache_combo")
                .selected_text(cache_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut render_cache.mode, crate::render_cache::RenderCacheMode::None, "Off (Без кэша)");
                    ui.selectable_value(&mut render_cache.mode, crate::render_cache::RenderCacheMode::Smart, "Smart Cache (DaVinci Auto)");
                    ui.selectable_value(&mut render_cache.mode, crate::render_cache::RenderCacheMode::User, "User Cache (По запросу)");
                });
            if stats.mode != crate::render_cache::RenderCacheMode::None && stats.cached_frames > 0 {
                ui.label(
                    egui::RichText::new(format!("({} k / {:.1} MB)", stats.cached_frames, stats.memory_mb))
                        .size(10.0)
                        .color(ResolveColors::ACCENT_ORANGE),
                )
                .on_hover_text("Объем кэшированных кадров в оперативной памяти (0 мс воспроизведение)");
            }

            // 6. Статус аппаратного ускорения GPU и индикатор Drop Frame (DaVinci Resolve)
            if ui.available_width() > 140.0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.available_width() > 180.0 {
                        let (badge_text, badge_color) = if video_player.gpu_accelerated {
                            (video_player.gpu_badge.as_str(), ResolveColors::ACCENT_ORANGE)
                        } else {
                            ("⚡ GPU: Idle / CPU", ResolveColors::TEXT_MUTED)
                        };
                        ui.label(egui::RichText::new(badge_text).size(10.0).strong().color(badge_color))
                            .on_hover_text("Аппаратное ускорение видеодекодирования на GPU (VA-API / CUDA)");

                        ui.separator();
                    }

                    // Индикатор частоты кадров и Drop Frame в стиле DaVinci Resolve
                    let is_active_playback = video_player.is_playing_mode;
                    let (fps_color, fps_val) = if is_active_playback {
                        let cur_fps = if video_player.real_playback_fps > 0.0 {
                            video_player.real_playback_fps
                        } else {
                            video_player.fps as f32
                        };
                        if video_player.is_dropping_frames {
                            (Color32::from_rgb(255, 69, 58), cur_fps) // Красный (Drop Frame)
                        } else {
                            (Color32::from_rgb(48, 209, 88), cur_fps) // Зеленый (Nominal Real-time)
                        }
                    } else {
                        (ResolveColors::TEXT_MUTED, video_player.fps as f32)
                    };

                    let drop_info = if is_active_playback && video_player.dropped_frames_count > 0 {
                        format!("● {:.1} fps (Drop: {})", fps_val, video_player.dropped_frames_count)
                    } else {
                        format!("● {:.1} fps", fps_val)
                    };

                    ui.label(egui::RichText::new(drop_info).size(10.0).strong().color(fps_color))
                        .on_hover_text("Реальный FPS предпросмотра и индикатор пропуска кадров (DaVinci Resolve Drop Frame Indicator)");

                    // Кнопка режима воспроизведения: Real-Time Drop Frame vs Show All Frames
                    if ui.available_width() > 120.0 {
                        let mode_text = if video_player.drop_frames_enabled {
                            "⏱ Real-Time (Drop)"
                        } else {
                            "🐢 All Frames (Slow-Mo)"
                        };
                        let mode_btn = egui::Button::new(
                            egui::RichText::new(mode_text)
                                .size(10.0)
                                .color(if video_player.drop_frames_enabled { Color32::WHITE } else { ResolveColors::ACCENT_ORANGE }),
                        )
                        .fill(if video_player.drop_frames_enabled { ResolveColors::BG_HEADER } else { ResolveColors::BG_MAIN });

                        if ui.add(mode_btn)
                            .on_hover_text("Переключение режима: Real-Time Sync (пропуск кадров для сохранения 1.0x скорости звука, как в DaVinci) или Show All Frames (slow-mo)")
                            .clicked()
                        {
                            video_player.drop_frames_enabled = !video_player.drop_frames_enabled;
                        }
                    }
                });
            }
        });
    }

    fn render_source_viewer(&mut self, ui: &mut Ui, video_player: &VideoPlayer) {
        ui.horizontal(|ui| {
            let label = if let Some(p) = &video_player.current_path {
                p.file_name().and_then(|n| n.to_str()).unwrap_or("No Media Loaded")
            } else {
                "No Media Loaded"
            };
            ui.label(
                egui::RichText::new("SOURCE: ")
                    .size(11.0)
                    .strong()
                    .color(ResolveColors::TEXT_MUTED),
            );
            ui.label(
                egui::RichText::new(label)
                    .size(11.0)
                    .strong()
                    .color(ResolveColors::TEXT_PRIMARY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let duration_tc = Timecode::new(video_player.total_frames, (video_player.fps.round() as u32).max(1)).to_smpte();
                ui.label(
                    egui::RichText::new(duration_tc)
                        .size(11.0)
                        .color(ResolveColors::TEXT_TIMECODE)
                        .monospace(),
                );
            });
        });

        ui.separator();

        let avail_w = ui.available_width().max(80.0);
        let avail_h = (ui.available_height() - 34.0).max(60.0);
        let aspect = 16.0 / 9.0;
        let pad_w = (avail_w - 24.0).max(60.0);
        let pad_h = (avail_h - 24.0).max(45.0);
        let (fit_w, fit_h) = if pad_w / pad_h > aspect {
            (pad_h * aspect, pad_h)
        } else {
            (pad_w, pad_w / aspect)
        };
        let monitor_scale = self.custom_zoom.unwrap_or_else(|| match self.zoom_mode {
            ZoomMode::Fit => 1.0,
            ZoomMode::Fifty => 0.5,
            ZoomMode::Hundred => 1.0 * (1080.0 / fit_h.max(1.0)),
            ZoomMode::TwoHundred => 2.0,
        });
        let video_w = fit_w * monitor_scale;
        let video_h = fit_h * monitor_scale;
        let (rect, response) = ui.allocate_exact_size(vec2(avail_w, avail_h), egui::Sense::click_and_drag());

        let pointer_in_source = ui.input(|i| i.pointer.hover_pos().map(|p| rect.contains(p)).unwrap_or(false));
        if pointer_in_source {
            let (zoom_delta, raw_wheel_y) = ui.input(|i| {
                let zd = i.zoom_delta();
                let mut wy = i.smooth_scroll_delta.y;
                if wy.abs() < 0.01 {
                    for ev in &i.events {
                        if let egui::Event::MouseWheel { delta, .. } = ev {
                            wy += delta.y * 24.0;
                        }
                    }
                }
                (zd, wy)
            });
            let factor = if (zoom_delta - 1.0).abs() > 0.001 {
                zoom_delta.clamp(0.75, 1.35)
            } else if raw_wheel_y.abs() > 0.05 {
                (1.0f32 + raw_wheel_y * 0.0025f32).clamp(0.8f32, 1.25f32)
            } else {
                1.0
            };
            if (factor - 1.0).abs() > 0.001 {
                let cur = self.custom_zoom.unwrap_or(monitor_scale);
                self.custom_zoom = Some((cur * factor).clamp(0.1, 10.0));
                ui.ctx().request_repaint();
            }
        }
        if response.double_clicked() {
            self.zoom_mode = ZoomMode::Fit;
            self.custom_zoom = None;
            self.pan_offset = egui::Vec2::ZERO;
        }

        let painter = ui.painter_at(rect);
        // Задний фон монитора (нейтральный студийный тёмно-серый как в DaVinci Resolve)
        painter.rect_filled(rect, CornerRadius::same(2), Color32::from_rgb(0x13, 0x14, 0x18));
        painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, ResolveColors::BORDER), StrokeKind::Inside);

        // Экран проекта (Screen Canvas) — чисто чёрный (#000000) прямоугольник с чёткими границами
        let screen_rect = Rect::from_center_size(rect.center() + self.pan_offset, vec2(video_w, video_h));
        painter.rect_filled(screen_rect, CornerRadius::ZERO, Color32::BLACK);
        painter.rect_stroke(screen_rect, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x38, 0x3c, 0x48)), StrokeKind::Outside);

        if let Some(tex) = &video_player.texture {
            painter.with_clip_rect(screen_rect.intersect(rect)).image(
                tex.id(),
                screen_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let out_tc = Timecode::new(video_player.total_frames, (video_player.fps.round() as u32).max(1)).to_smpte();
            ui.label(egui::RichText::new("In: 00:00:00:00").size(10.0).color(ResolveColors::TEXT_MUTED));
            ui.separator();
            ui.label(egui::RichText::new(format!("Out: {}", out_tc)).size(10.0).color(ResolveColors::TEXT_MUTED));
        });
    }

    fn render_program_viewer(&mut self, ui: &mut Ui, timeline: &mut TimelineState, video_player: &VideoPlayer) {
        let current_tc = Timecode::new(timeline.playhead_frame, timeline.fps).to_smpte();

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("TIMELINE: Master Sequence")
                    .size(11.0)
                    .strong()
                    .color(ResolveColors::TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(current_tc)
                        .size(13.0)
                        .strong()
                        .color(ResolveColors::ACCENT_ORANGE)
                        .monospace(),
                );
            });
        });

        ui.separator();

        let vu_width = 28.0;
        let transport_h = 36.0;
        let avail_w = (ui.available_width() - vu_width - 8.0).max(80.0);
        let avail_h = (ui.available_height() - transport_h - 6.0).max(60.0);

        let aspect = 16.0 / 9.0;
        // Студийный отступ вокруг холста 16:9, чтобы рамка Transform OSC и ручка вращения (32px сверху) никогда не обрезались!
        let pad_w = (avail_w - 56.0).max(60.0);
        let pad_h = (avail_h - 68.0).max(45.0);
        let (fit_w, fit_h) = if pad_w / pad_h > aspect {
            (pad_h * aspect, pad_h)
        } else {
            (pad_w, pad_w / aspect)
        };

        // Применение зума монитора (Fit / 50% / 100% / 200% / Custom)
        let monitor_scale = self.custom_zoom.unwrap_or_else(|| match self.zoom_mode {
            ZoomMode::Fit => 1.0,
            ZoomMode::Fifty => 0.5,
            ZoomMode::Hundred => 1.0 * (1080.0 / fit_h.max(1.0)),
            ZoomMode::TwoHundred => 2.0,
        });

        ui.horizontal(|ui| {
            // Контейнер предпросмотра занимает полную высоту контейнера dock с поддержкой панорамирования и зума
            let (rect, rect_resp) = ui.allocate_exact_size(vec2(avail_w, avail_h), egui::Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            // Проверка геометрического вхождения курсора в rect (не зависит от перехвата hover дочерним виджетом Transform)
            let pointer_in_viewer = ui.input(|i| i.pointer.hover_pos().map(|p| rect.contains(p)).unwrap_or(false));
            let is_space = ui.input(|i| i.key_down(egui::Key::Space));
            let mmb_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
            let lmb_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let pointer_delta = ui.input(|i| i.pointer.delta());

            if pointer_in_viewer && (mmb_down || (is_space && lmb_down)) {
                self.pan_offset += pointer_delta;
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                ui.ctx().request_repaint();
            } else if pointer_in_viewer && is_space {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
            }

            // Масштабирование предпросмотра (Ctrl + Колесо мыши через zoom_delta() ИЛИ обычное колесо мыши над монитором)
            if pointer_in_viewer {
                let (zoom_delta, raw_wheel_y, hover_pos) = ui.input(|i| {
                    let zd = i.zoom_delta();
                    let mut wy = i.smooth_scroll_delta.y;
                    if wy.abs() < 0.01 {
                        for ev in &i.events {
                            if let egui::Event::MouseWheel { delta, .. } = ev {
                                wy += delta.y * 24.0;
                            }
                        }
                    }
                    (zd, wy, i.pointer.hover_pos())
                });

                let factor = if (zoom_delta - 1.0).abs() > 0.001 {
                    zoom_delta.clamp(0.75, 1.35)
                } else if raw_wheel_y.abs() > 0.05 {
                    (1.0f32 + raw_wheel_y * 0.0025f32).clamp(0.8f32, 1.25f32)
                } else {
                    1.0
                };

                if (factor - 1.0).abs() > 0.001 {
                    let old_scale = self.custom_zoom.unwrap_or(monitor_scale);
                    let new_scale = (old_scale * factor).clamp(0.1, 10.0);
                    let actual_ratio = new_scale / old_scale.max(0.001);
                    if let Some(cursor) = hover_pos {
                        let rel = cursor - (rect.center() + self.pan_offset);
                        self.pan_offset -= rel * (actual_ratio - 1.0);
                    }
                    self.custom_zoom = Some(new_scale);
                    ui.ctx().request_repaint();
                }
            }

            if rect_resp.double_clicked() {
                self.zoom_mode = ZoomMode::Fit;
                self.custom_zoom = None;
                self.pan_offset = egui::Vec2::ZERO;
            }

            let effective_scale = self.custom_zoom.unwrap_or(monitor_scale);
            let screen_w = fit_w * effective_scale;
            let screen_h = fit_h * effective_scale;

            // 1. Задний фон монитора (студийный тёмно-серый backdrop DaVinci Resolve)
            painter.rect_filled(rect, CornerRadius::same(2), Color32::from_rgb(0x13, 0x14, 0x18));
            painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, ResolveColors::BORDER), StrokeKind::Inside);

            // 2. Экран проекта (Screen Canvas) — чисто чёрный (#000000) прямоугольник 16:9
            // Это границы кадра проекта, которые видит зритель и которые идут на финальный рендер
            let screen_rect = Rect::from_center_size(rect.center() + self.pan_offset, vec2(screen_w, screen_h));
            painter.rect_filled(screen_rect, CornerRadius::ZERO, Color32::BLACK);
            painter.rect_stroke(
                screen_rect,
                CornerRadius::ZERO,
                Stroke::new(1.0, Color32::from_rgb(0x38, 0x3c, 0x48)),
                StrokeKind::Outside,
            );

            let top_clip = timeline.active_video_clip_at(timeline.playhead_frame).cloned();

            if let Some(mut top_clip) = top_clip {
                // Коэффициент масштаба пикселей таймлайна (в координатах растра 1920x1080 DaVinci Resolve)
                let px_scale = (screen_rect.width() / 1920.0).max(0.001);

                let clip_w = screen_rect.width() * top_clip.zoom_x;
                let clip_h = screen_rect.height() * top_clip.zoom_y;
                let clip_center = screen_rect.center() + vec2(top_clip.position_x * px_scale, top_clip.position_y * px_scale);
                let video_box_rect = Rect::from_center_size(clip_center, vec2(clip_w, clip_h));

                // Учет обрезки кадра (Cropping Left/Right/Top/Bottom) и отражения (Flip H/V)
                let u0 = (top_clip.crop_left / 100.0).clamp(0.0, 0.49);
                let u1 = (1.0 - top_clip.crop_right / 100.0).clamp(0.51, 1.0);
                let v0 = (top_clip.crop_top / 100.0).clamp(0.0, 0.49);
                let v1 = (1.0 - top_clip.crop_bottom / 100.0).clamp(0.51, 1.0);

                let (uv_min_x, uv_max_x) = if top_clip.flip_h { (u1, u0) } else { (u0, u1) };
                let (uv_min_y, uv_max_y) = if top_clip.flip_v { (v1, v0) } else { (v0, v1) };

                let crop_l_px = clip_w * (top_clip.crop_left / 100.0);
                let crop_r_px = clip_w * (top_clip.crop_right / 100.0);
                let crop_t_px = clip_h * (top_clip.crop_top / 100.0);
                let crop_b_px = clip_h * (top_clip.crop_bottom / 100.0);

                let cropped_rect = Rect::from_min_max(
                    pos2(video_box_rect.min.x + crop_l_px, video_box_rect.min.y + crop_t_px),
                    pos2(video_box_rect.max.x - crop_r_px, video_box_rect.max.y - crop_b_px),
                );

                // Угол поворота клипа
                let rot_deg = top_clip.rotation;
                let rot_rad = rot_deg.to_radians();
                let (sin_a, cos_a) = rot_rad.sin_cos();
                let rotate_pt = |p: Pos2| -> Pos2 {
                    let dx = p.x - clip_center.x;
                    let dy = p.y - clip_center.y;
                    pos2(clip_center.x + dx * cos_a - dy * sin_a, clip_center.y + dx * sin_a + dy * cos_a)
                };

                // Изоляция цветовых каналов и поддержка динамического Fade In / Fade Out непрозрачности
                let effective_opacity = top_clip.effective_opacity_at(timeline.playhead_frame);
                let alpha = (effective_opacity / 100.0 * 255.0).round().clamp(0.0, 255.0) as u8;
                let tint = match self.channel_mode {
                    ColorChannelMode::RGB => Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
                    ColorChannelMode::Red => Color32::from_rgba_unmultiplied(255, 60, 60, alpha),
                    ColorChannelMode::Green => Color32::from_rgba_unmultiplied(60, 255, 60, alpha),
                    ColorChannelMode::Blue => Color32::from_rgba_unmultiplied(80, 120, 255, alpha),
                    ColorChannelMode::Monochrome => Color32::from_rgba_unmultiplied(200, 200, 200, alpha),
                };

                // Отрисовка живого кадра видео строго внутри границ экрана проекта (screen_rect),
                // тогда как интерактивная рамка Transform OSC рисуется поверх всего монитора (painter)!
                let video_painter = painter.with_clip_rect(screen_rect.intersect(rect));
                if let Some(tex) = &video_player.texture {
                    if rot_deg.abs() <= 0.01 {
                        let uv = Rect::from_min_max(pos2(uv_min_x, uv_min_y), pos2(uv_max_x, uv_max_y));
                        video_painter.image(tex.id(), cropped_rect, uv, tint);
                    } else {
                        let mut mesh = egui::Mesh::with_texture(tex.id());
                        let p_lt = rotate_pt(cropped_rect.left_top());
                        let p_rt = rotate_pt(cropped_rect.right_top());
                        let p_rb = rotate_pt(cropped_rect.right_bottom());
                        let p_lb = rotate_pt(cropped_rect.left_bottom());
                        let idx = mesh.vertices.len() as u32;
                        mesh.vertices.push(egui::epaint::Vertex { pos: p_lt, uv: pos2(uv_min_x, uv_min_y), color: tint });
                        mesh.vertices.push(egui::epaint::Vertex { pos: p_rt, uv: pos2(uv_max_x, uv_min_y), color: tint });
                        mesh.vertices.push(egui::epaint::Vertex { pos: p_rb, uv: pos2(uv_max_x, uv_max_y), color: tint });
                        mesh.vertices.push(egui::epaint::Vertex { pos: p_lb, uv: pos2(uv_min_x, uv_max_y), color: tint });
                        mesh.add_triangle(idx, idx + 1, idx + 2);
                        mesh.add_triangle(idx, idx + 2, idx + 3);
                        video_painter.add(egui::Shape::mesh(mesh));
                    }
                }

                // Интерактивная рамка трансформации (Transform Overlay как в DaVinci Resolve)
                if self.show_transform_overlay {
                    // Вычисляем ключевые точки рамки с учётом поворота
                    let p_lt = rotate_pt(video_box_rect.left_top());
                    let p_rt = rotate_pt(video_box_rect.right_top());
                    let p_rb = rotate_pt(video_box_rect.right_bottom());
                    let p_lb = rotate_pt(video_box_rect.left_bottom());

                    let p_tc = pos2((p_lt.x + p_rt.x) * 0.5, (p_lt.y + p_rt.y) * 0.5);
                    let p_bc = pos2((p_lb.x + p_rb.x) * 0.5, (p_lb.y + p_rb.y) * 0.5);
                    let p_lc = pos2((p_lt.x + p_lb.x) * 0.5, (p_lt.y + p_lb.y) * 0.5);
                    let p_rc = pos2((p_rt.x + p_rb.x) * 0.5, (p_rt.y + p_rb.y) * 0.5);

                    // Тонкая белая с синим свечением рамка клипа как в Resolve
                    let shadow_stroke = Stroke::new(2.5, Color32::from_rgba_unmultiplied(0, 0, 0, 160));
                    for seg in [[p_lt, p_rt], [p_rt, p_rb], [p_rb, p_lb], [p_lb, p_lt]] {
                        painter.line_segment(seg, shadow_stroke);
                    }
                    let frame_stroke = Stroke::new(1.5, Color32::from_rgb(41, 151, 255));
                    painter.line_segment([p_lt, p_rt], frame_stroke);
                    painter.line_segment([p_rt, p_rb], frame_stroke);
                    painter.line_segment([p_rb, p_lb], frame_stroke);
                    painter.line_segment([p_lb, p_lt], frame_stroke);

                    // Центральная точка привязки (Anchor Point с перекрестием в стиле Resolve)
                    let resolve_blue = Color32::from_rgb(41, 151, 255);
                    painter.circle_stroke(clip_center, 5.5, Stroke::new(1.5, resolve_blue));
                    painter.line_segment([pos2(clip_center.x - 9.0, clip_center.y), pos2(clip_center.x + 9.0, clip_center.y)], Stroke::new(1.5, resolve_blue));
                    painter.line_segment([pos2(clip_center.x, clip_center.y - 9.0), pos2(clip_center.x, clip_center.y + 9.0)], Stroke::new(1.5, resolve_blue));

                    // Стебель и ручка вращения (Rotation Stalk & Handle вверху клипа)
                    let stalk_len = 28.0;
                    let stalk_dir = rotate_pt(pos2(clip_center.x, clip_center.y - 1.0)) - clip_center;
                    let stalk_norm = stalk_dir.normalized();
                    let p_rot = p_tc + stalk_norm * stalk_len;
                    painter.line_segment([p_tc, p_rot], Stroke::new(1.5, resolve_blue));

                    // Интерактивные области ручек управления
                    let h_sz = vec2(16.0, 16.0);
                    let resp_lt = ui.interact(Rect::from_center_size(p_lt, h_sz), ui.id().with("tr_lt"), egui::Sense::drag());
                    let resp_rt = ui.interact(Rect::from_center_size(p_rt, h_sz), ui.id().with("tr_rt"), egui::Sense::drag());
                    let resp_lb = ui.interact(Rect::from_center_size(p_lb, h_sz), ui.id().with("tr_lb"), egui::Sense::drag());
                    let resp_rb = ui.interact(Rect::from_center_size(p_rb, h_sz), ui.id().with("tr_rb"), egui::Sense::drag());

                    let resp_tc = ui.interact(Rect::from_center_size(p_tc, h_sz), ui.id().with("tr_tc"), egui::Sense::drag());
                    let resp_bc = ui.interact(Rect::from_center_size(p_bc, h_sz), ui.id().with("tr_bc"), egui::Sense::drag());
                    let resp_lc = ui.interact(Rect::from_center_size(p_lc, h_sz), ui.id().with("tr_lc"), egui::Sense::drag());
                    let resp_rc = ui.interact(Rect::from_center_size(p_rc, h_sz), ui.id().with("tr_rc"), egui::Sense::drag());

                    let resp_rot = ui.interact(Rect::from_center_size(p_rot, vec2(20.0, 20.0)), ui.id().with("tr_rot"), egui::Sense::drag());
                    let body_resp = ui.interact(video_box_rect.intersect(rect), ui.id().with("tr_body"), egui::Sense::click_and_drag());

                    if body_resp.double_clicked() {
                        self.zoom_mode = ZoomMode::Fit;
                        self.custom_zoom = None;
                        self.pan_offset = egui::Vec2::ZERO;
                    }

                    let any_handle_active = resp_lt.hovered() || resp_rt.hovered() || resp_lb.hovered() || resp_rb.hovered()
                        || resp_tc.hovered() || resp_bc.hovered() || resp_lc.hovered() || resp_rc.hovered()
                        || resp_rot.hovered() || resp_rot.dragged()
                        || resp_lt.dragged() || resp_rt.dragged() || resp_lb.dragged() || resp_rb.dragged()
                        || resp_tc.dragged() || resp_bc.dragged() || resp_lc.dragged() || resp_rc.dragged();

                    // Адаптивные курсоры мыши
                    if resp_rot.hovered() || resp_rot.dragged() {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Crosshair);
                    } else if resp_lt.hovered() || resp_rb.hovered() {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeNorthWest);
                    } else if resp_rt.hovered() || resp_lb.hovered() {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeNorthEast);
                    } else if resp_tc.hovered() || resp_bc.hovered() {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeVertical);
                    } else if resp_lc.hovered() || resp_rc.hovered() {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
                    } else if body_resp.hovered() && !any_handle_active && !is_space && !mmb_down {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
                    }
                    if body_resp.dragged_by(egui::PointerButton::Primary) && !any_handle_active && !is_space && !mmb_down {
                        ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
                    }

                    // Обработка интерактивного изменения положения, масштаба и угла клипа с магнитным прилипанием к краям и центру (DaVinci Resolve Magnetic Snap)
                    let mut clip_mutated = false;
                    let top_clip_id = top_clip.id;
                    let mut active_snap_guide_x: Option<f32> = None;
                    let mut active_snap_guide_y: Option<f32> = None;
                    let is_alt = ui.input(|i| i.modifiers.alt);
                    let snap_proj_thresh = (12.0 / px_scale).clamp(4.0, 48.0);

                    let is_moving_body = body_resp.dragged_by(egui::PointerButton::Primary) && !any_handle_active && !is_space && !mmb_down;
                    let is_resizing_any = resp_rb.dragged() || resp_lt.dragged() || resp_rt.dragged() || resp_lb.dragged()
                        || resp_rc.dragged() || resp_lc.dragged() || resp_bc.dragged() || resp_tc.dragged();

                    if !is_moving_body {
                        self.transform_raw_pos = None;
                    }
                    if !is_resizing_any {
                        self.transform_raw_zoom = None;
                    }

                    if let Some(target_clip) = timeline.clips.iter_mut().find(|c| c.id == top_clip_id) {
                        if resp_rot.dragged() {
                            let is_shift = ui.input(|i| i.modifiers.shift);
                            if let Some(mouse_pos) = ui.input(|i| i.pointer.latest_pos()) {
                                let v = mouse_pos - clip_center;
                                let mut angle = v.y.atan2(v.x).to_degrees() + 90.0;
                                while angle > 180.0 { angle -= 360.0; }
                                while angle < -180.0 { angle += 360.0; }
                                if is_shift {
                                    angle = (angle / 15.0).round() * 15.0;
                                } else if !is_alt {
                                    // Магнитное прилипание к 0°, ±90°, ±180°
                                    for cardinal in [0.0f32, 90.0, -90.0, 180.0, -180.0] {
                                        if (angle - cardinal).abs() <= 2.5 {
                                            angle = cardinal;
                                            break;
                                        }
                                    }
                                }
                                target_clip.rotation = (angle * 10.0).round() / 10.0;
                                clip_mutated = true;
                            }
                        } else if is_moving_body {
                            let delta = body_resp.drag_delta();
                            let is_shift = ui.input(|i| i.modifiers.shift);
                            let (dx, dy) = if is_shift {
                                if delta.x.abs() > delta.y.abs() {
                                    (delta.x, 0.0)
                                } else {
                                    (0.0, delta.y)
                                }
                            } else {
                                (delta.x, delta.y)
                            };
                            let (raw_x, raw_y) = self
                                .transform_raw_pos
                                .get_or_insert((target_clip.position_x, target_clip.position_y));
                            *raw_x += dx / px_scale;
                            *raw_y += dy / px_scale;

                            if !is_alt && target_clip.rotation.abs() <= 0.5 {
                                let (sx, sy, gx, gy) = Self::snap_transform_position(*raw_x, *raw_y, target_clip, snap_proj_thresh);
                                target_clip.position_x = sx;
                                target_clip.position_y = sy;
                                active_snap_guide_x = gx;
                                active_snap_guide_y = gy;
                            } else {
                                target_clip.position_x = *raw_x;
                                target_clip.position_y = *raw_y;
                            }
                            clip_mutated = true;
                        } else if resp_rb.dragged() || resp_lt.dragged() || resp_rt.dragged() || resp_lb.dragged() {
                            // Сначала переводим вектор перемещения мыши в локальную (неповёрнутую) систему координат клипа,
                            // и только затем учитываем знаки квадранта ручки — так ручки работают 1:1 при любом угле поворота!
                            let (screen_delta, sign_x, sign_y) = if resp_rb.dragged() {
                                (resp_rb.drag_delta(), 1.0f32, 1.0f32)
                            } else if resp_lt.dragged() {
                                (resp_lt.drag_delta(), -1.0f32, -1.0f32)
                            } else if resp_rt.dragged() {
                                (resp_rt.drag_delta(), 1.0f32, -1.0f32)
                            } else {
                                (resp_lb.drag_delta(), -1.0f32, 1.0f32)
                            };
                            let local_dx = (screen_delta.x * cos_a + screen_delta.y * sin_a) * sign_x;
                            let local_dy = (-screen_delta.x * sin_a + screen_delta.y * cos_a) * sign_y;
                            let half_w = (screen_rect.width() * 0.5).max(1.0);
                            let half_h = (screen_rect.height() * 0.5).max(1.0);
                            let zoom_delta = (local_dx / half_w + local_dy / half_h) * 0.5;
                            let (raw_zx, raw_zy) = self
                                .transform_raw_zoom
                                .get_or_insert((target_clip.zoom_x, target_clip.zoom_y));
                            let ratio = if raw_zx.abs() > 0.001 { *raw_zy / *raw_zx } else { 1.0 };
                            *raw_zx = (*raw_zx + zoom_delta).clamp(0.05, 10.0);
                            *raw_zy = (*raw_zx * ratio).clamp(0.05, 10.0);

                            if !is_alt && target_clip.rotation.abs() <= 0.5 {
                                let (szx, szy, gx, gy) = Self::snap_transform_zoom(*raw_zx, *raw_zy, target_clip, true, snap_proj_thresh);
                                target_clip.zoom_x = szx;
                                target_clip.zoom_y = szy;
                                active_snap_guide_x = gx;
                                active_snap_guide_y = gy;
                            } else {
                                target_clip.zoom_x = *raw_zx;
                                target_clip.zoom_y = *raw_zy;
                            }
                            clip_mutated = true;
                        } else if resp_rc.dragged() || resp_lc.dragged() {
                            let (screen_delta, sign_x) = if resp_rc.dragged() {
                                (resp_rc.drag_delta(), 1.0f32)
                            } else {
                                (resp_lc.drag_delta(), -1.0f32)
                            };
                            let local_dx = (screen_delta.x * cos_a + screen_delta.y * sin_a) * sign_x;
                            let half_w = (screen_rect.width() * 0.5).max(1.0);
                            let zoom_delta = local_dx / half_w;
                            let (raw_zx, raw_zy) = self
                                .transform_raw_zoom
                                .get_or_insert((target_clip.zoom_x, target_clip.zoom_y));
                            *raw_zx = (*raw_zx + zoom_delta).clamp(0.05, 10.0);

                            if !is_alt && target_clip.rotation.abs() <= 0.5 {
                                let (szx, _, gx, _) = Self::snap_transform_zoom(*raw_zx, *raw_zy, target_clip, false, snap_proj_thresh);
                                target_clip.zoom_x = szx;
                                active_snap_guide_x = gx;
                            } else {
                                target_clip.zoom_x = *raw_zx;
                            }
                            clip_mutated = true;
                        } else if resp_bc.dragged() || resp_tc.dragged() {
                            let (screen_delta, sign_y) = if resp_bc.dragged() {
                                (resp_bc.drag_delta(), 1.0f32)
                            } else {
                                (resp_tc.drag_delta(), -1.0f32)
                            };
                            let local_dy = (-screen_delta.x * sin_a + screen_delta.y * cos_a) * sign_y;
                            let half_h = (screen_rect.height() * 0.5).max(1.0);
                            let zoom_delta = local_dy / half_h;
                            let (raw_zx, raw_zy) = self
                                .transform_raw_zoom
                                .get_or_insert((target_clip.zoom_x, target_clip.zoom_y));
                            *raw_zy = (*raw_zy + zoom_delta).clamp(0.05, 10.0);

                            if !is_alt && target_clip.rotation.abs() <= 0.5 {
                                let (_, szy, _, gy) = Self::snap_transform_zoom(*raw_zx, *raw_zy, target_clip, false, snap_proj_thresh);
                                target_clip.zoom_y = szy;
                                active_snap_guide_y = gy;
                            } else {
                                target_clip.zoom_y = *raw_zy;
                            }
                            clip_mutated = true;
                        }
                    }

                    // Визуальные магнитные направляющие при прилипании к краям или центру экрана (DaVinci Resolve Snap Guides)
                    let snap_color = Color32::from_rgb(255, 214, 10);
                    if let Some(gx) = active_snap_guide_x {
                        let screen_x = screen_rect.center().x + gx * px_scale;
                        painter.line_segment(
                            [pos2(screen_x, screen_rect.min.y - 6.0), pos2(screen_x, screen_rect.max.y + 6.0)],
                            Stroke::new(1.8, snap_color),
                        );
                    }
                    if let Some(gy) = active_snap_guide_y {
                        let screen_y = screen_rect.center().y + gy * px_scale;
                        painter.line_segment(
                            [pos2(screen_rect.min.x - 6.0, screen_y), pos2(screen_rect.max.x + 6.0, screen_y)],
                            Stroke::new(1.8, snap_color),
                        );
                    }

                    if clip_mutated {
                        if let Some(target_clip) = timeline.clips.iter().find(|c| c.id == top_clip_id) {
                            top_clip.position_x = target_clip.position_x;
                            top_clip.position_y = target_clip.position_y;
                            top_clip.zoom_x = target_clip.zoom_x;
                            top_clip.zoom_y = target_clip.zoom_y;
                            top_clip.rotation = target_clip.rotation;
                        }
                        ui.ctx().request_repaint();
                    }

                    let any_drag_started = body_resp.drag_started()
                        || resp_rot.drag_started()
                        || resp_lt.drag_started() || resp_rt.drag_started() || resp_lb.drag_started() || resp_rb.drag_started()
                        || resp_tc.drag_started() || resp_bc.drag_started() || resp_lc.drag_started() || resp_rc.drag_started();
                    if any_drag_started {
                        timeline.push_history();
                    }

                    let any_drag_stopped = body_resp.drag_stopped()
                        || resp_rot.drag_stopped()
                        || resp_lt.drag_stopped() || resp_rt.drag_stopped() || resp_lb.drag_stopped() || resp_rb.drag_stopped()
                        || resp_tc.drag_stopped() || resp_bc.drag_stopped() || resp_lc.drag_stopped() || resp_rc.drag_stopped();
                    if any_drag_stopped {
                        let _ = timeline.save_scene_file(&crate::preferences::UserPreferences::scene_path());
                    }

                    // Отрисовка круглых ручек DaVinci Resolve (белый круг с синей каймой #2997ff)
                    let draw_handle = |painter: &egui::Painter, pos: Pos2, radius: f32, hovered: bool, dragged: bool| {
                        let (fill, stroke_color, stroke_w) = if dragged {
                            (resolve_blue, Color32::WHITE, 2.0)
                        } else if hovered {
                            (Color32::from_rgb(70, 170, 255), Color32::WHITE, 1.5)
                        } else {
                            (Color32::WHITE, resolve_blue, 1.5)
                        };
                        painter.circle_filled(pos, radius, fill);
                        painter.circle_stroke(pos, radius, Stroke::new(stroke_w, stroke_color));
                    };

                    for (pt, resp) in [
                        (p_lt, &resp_lt), (p_rt, &resp_rt), (p_lb, &resp_lb), (p_rb, &resp_rb),
                        (p_tc, &resp_tc), (p_bc, &resp_bc), (p_lc, &resp_lc), (p_rc, &resp_rc),
                    ] {
                        draw_handle(&painter, pt, 4.5, resp.hovered(), resp.dragged());
                    }

                    // Ручка вращения на конце стебля
                    draw_handle(&painter, p_rot, 5.5, resp_rot.hovered(), resp_rot.dragged());

                    // Информационный HUD вверху экрана при трансформации (показывает текущие координаты, масштаб и угол)
                    let is_transforming = any_handle_active || body_resp.dragged() || body_resp.hovered();
                    if is_transforming {
                        let mut extra = String::new();
                        if top_clip.flip_h { extra.push_str("  [Flip H]"); }
                        if top_clip.flip_v { extra.push_str("  [Flip V]"); }
                        if top_clip.crop_left > 0.0 || top_clip.crop_right > 0.0 || top_clip.crop_top > 0.0 || top_clip.crop_bottom > 0.0 {
                            extra.push_str("  [Cropped]");
                        }
                        let hud_text = format!(
                            "X: {:+.0} px   Y: {:+.0} px   Zoom: {:.2}x   Rot: {:+.1}°{}",
                            top_clip.position_x, top_clip.position_y, top_clip.zoom_x, top_clip.rotation, extra
                        );
                        let hud_w = if extra.is_empty() { 280.0 } else { 380.0 };
                        let hud_h = 24.0;
                        let hud_rect = Rect::from_min_size(
                            pos2(screen_rect.min.x + 12.0, screen_rect.min.y + 12.0),
                            vec2(hud_w, hud_h),
                        );
                        painter.rect_filled(hud_rect, CornerRadius::same(4), Color32::from_rgba_unmultiplied(12, 14, 18, 220));
                        painter.rect_stroke(hud_rect, CornerRadius::same(4), Stroke::new(1.0, resolve_blue), StrokeKind::Inside);
                        painter.text(hud_rect.center(), egui::Align2::CENTER_CENTER, hud_text, egui::FontId::monospace(11.0), Color32::WHITE);
                    }
                }
            } else {
                painter.text(
                    screen_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "NO CLIP AT PLAYHEAD",
                    egui::FontId::monospace(11.0),
                    Color32::from_rgb(0x50, 0x55, 0x60),
                );
            }

            // 1. Оверлей Каше (Aspect Ratio Mask: Cinema 2.39:1, 9:16 Shorts...) привязан к границам экрана screen_rect
            Self::draw_aspect_mask(&painter, screen_rect, self.aspect_mask);

            // 2. Направляющие Safe Areas (80% / 90%)
            if self.show_safe_areas {
                let safe_action = screen_rect.shrink2(vec2(screen_rect.width() * 0.05, screen_rect.height() * 0.05));
                let safe_title = screen_rect.shrink2(vec2(screen_rect.width() * 0.10, screen_rect.height() * 0.10));
                painter.rect_stroke(safe_action, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x3a, 0x55, 0x77)), StrokeKind::Inside);
                painter.rect_stroke(safe_title, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x66, 0x44, 0x22)), StrokeKind::Inside);
            }

            // 3. Сетка третей (Rule of Thirds)
            if self.show_rule_of_thirds {
                let w_third = screen_rect.width() / 3.0;
                let h_third = screen_rect.height() / 3.0;
                let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 60));
                painter.line_segment([pos2(screen_rect.min.x + w_third, screen_rect.min.y), pos2(screen_rect.min.x + w_third, screen_rect.max.y)], stroke);
                painter.line_segment([pos2(screen_rect.min.x + w_third * 2.0, screen_rect.min.y), pos2(screen_rect.min.x + w_third * 2.0, screen_rect.max.y)], stroke);
                painter.line_segment([pos2(screen_rect.min.x, screen_rect.min.y + h_third), pos2(screen_rect.max.x, screen_rect.min.y + h_third)], stroke);
                painter.line_segment([pos2(screen_rect.min.x, screen_rect.min.y + h_third * 2.0), pos2(screen_rect.max.x, screen_rect.min.y + h_third * 2.0)], stroke);
            }

            // 4. Центральное перекрестие (Crosshair)
            if self.show_center_cross {
                let c = screen_rect.center();
                let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(232, 124, 36, 160));
                painter.line_segment([pos2(c.x - 15.0, c.y), pos2(c.x + 15.0, c.y)], stroke);
                painter.line_segment([pos2(c.x, c.y - 15.0), pos2(c.x, c.y + 15.0)], stroke);
            }

            // Master Audio VU Meter (справа)
            self.render_vu_meter(ui, avail_h);
        });

        ui.add_space(4.0);

        // Панель управления воспроизведением (Transport Bar)
        self.render_transport_bar(ui, timeline);
    }

    fn draw_aspect_mask(painter: &egui::Painter, rect: Rect, mask: AspectMask) {
        let mask_color = Color32::from_rgba_unmultiplied(0, 0, 0, 220);
        match mask {
            AspectMask::Off => {}
            AspectMask::Cinema239 => {
                let target_h = rect.width() / 2.39;
                let bar_h = (rect.height() - target_h).max(0.0) * 0.5;
                if bar_h > 0.0 {
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), bar_h)), CornerRadius::ZERO, mask_color);
                    painter.rect_filled(Rect::from_min_size(pos2(rect.min.x, rect.max.y - bar_h), vec2(rect.width(), bar_h)), CornerRadius::ZERO, mask_color);
                }
            }
            AspectMask::Cinema185 => {
                let target_h = rect.width() / 1.85;
                let bar_h = (rect.height() - target_h).max(0.0) * 0.5;
                if bar_h > 0.0 {
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(rect.width(), bar_h)), CornerRadius::ZERO, mask_color);
                    painter.rect_filled(Rect::from_min_size(pos2(rect.min.x, rect.max.y - bar_h), vec2(rect.width(), bar_h)), CornerRadius::ZERO, mask_color);
                }
            }
            AspectMask::Vertical916 => {
                let target_w = rect.height() * (9.0 / 16.0);
                let bar_w = (rect.width() - target_w).max(0.0) * 0.5;
                if bar_w > 0.0 {
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(bar_w, rect.height())), CornerRadius::ZERO, mask_color);
                    painter.rect_filled(Rect::from_min_size(pos2(rect.max.x - bar_w, rect.min.y), vec2(bar_w, rect.height())), CornerRadius::ZERO, mask_color);
                }
            }
            AspectMask::Square11 => {
                let target_w = rect.height();
                let bar_w = (rect.width() - target_w).max(0.0) * 0.5;
                if bar_w > 0.0 {
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(bar_w, rect.height())), CornerRadius::ZERO, mask_color);
                    painter.rect_filled(Rect::from_min_size(pos2(rect.max.x - bar_w, rect.min.y), vec2(bar_w, rect.height())), CornerRadius::ZERO, mask_color);
                }
            }
        }
    }

    fn render_vu_meter(&mut self, ui: &mut Ui, height: f32) {
        let (meter_rect, _) = ui.allocate_exact_size(vec2(24.0, height.max(40.0)), egui::Sense::hover());
        let painter = ui.painter_at(meter_rect);

        painter.rect_filled(meter_rect, CornerRadius::same(2), Color32::from_rgb(0x11, 0x12, 0x14));
        painter.rect_stroke(meter_rect, CornerRadius::same(2), Stroke::new(1.0, ResolveColors::BORDER), StrokeKind::Inside);

        let ch_width = 7.0;
        let gap = 2.0;

        let left_rect = Rect::from_min_size(
            pos2(meter_rect.min.x + 3.0, meter_rect.min.y + 3.0),
            vec2(ch_width, (height - 6.0).max(10.0)),
        );
        self.draw_channel_meter(&painter, left_rect, self.vu_level_left);

        let right_rect = Rect::from_min_size(
            pos2(meter_rect.min.x + 3.0 + ch_width + gap, meter_rect.min.y + 3.0),
            vec2(ch_width, (height - 6.0).max(10.0)),
        );
        self.draw_channel_meter(&painter, right_rect, self.vu_level_right);
    }

    fn draw_channel_meter(&self, painter: &egui::Painter, rect: Rect, level: f32) {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x1b, 0x1d, 0x22));

        let active_height = rect.height() * level.clamp(0.0, 1.0);
        let filled_rect = Rect::from_min_max(
            pos2(rect.min.x, rect.max.y - active_height),
            rect.max,
        );

        let color = if level > 0.85 {
            ResolveColors::VU_RED
        } else if level > 0.65 {
            ResolveColors::VU_YELLOW
        } else {
            ResolveColors::VU_GREEN
        };

        painter.rect_filled(filled_rect, CornerRadius::ZERO, color);
    }

    fn render_transport_bar(&mut self, ui: &mut Ui, timeline: &mut TimelineState) {
        ui.horizontal(|ui| {
            // OSC (On-Screen Control) Transform Toggle в левом нижнем углу как в DaVinci Resolve
            let osc_active = self.show_transform_overlay;
            let osc_btn = egui::Button::new(
                egui::RichText::new("⛶ Transform")
                    .size(11.0)
                    .color(if osc_active { Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
            )
            .fill(if osc_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(osc_btn).on_hover_text("Интерактивная рамка трансформации клипа на экране (On-Screen Control)").clicked() {
                self.show_transform_overlay = !self.show_transform_overlay;
            }

            ui.add_space(ui.available_width() * 0.08);

            // Prev Cut
            if ui.button(egui::RichText::new("⏮").size(13.0)).on_hover_text("К началу [Home]").clicked() {
                timeline.playhead_frame = 0;
            }

            // Shuttle Reverse (J)
            if ui.button(egui::RichText::new("⏪").size(13.0)).on_hover_text("Shuttle Reverse [J]").clicked() {
                timeline.playback_speed = -2.0;
                timeline.is_playing = true;
                timeline.frame_accumulator = 0.0;
            }

            // Step Back 1 Frame (Left Arrow)
            if ui.button(egui::RichText::new("◀").size(13.0)).on_hover_text("Назад на 1 кадр [←]").clicked() {
                timeline.playhead_frame = (timeline.playhead_frame - 1).max(0);
                timeline.is_playing = false;
            }

            // Play / Pause (Space)
            let play_label = if timeline.is_playing { "⏸" } else { "▶" };
            let play_btn = egui::Button::new(
                egui::RichText::new(play_label)
                    .size(15.0)
                    .color(if timeline.is_playing { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_PRIMARY }),
            );
            if ui.add(play_btn).on_hover_text("Play / Pause [Space]").clicked() {
                timeline.is_playing = !timeline.is_playing;
                timeline.playback_speed = 1.0;
                if timeline.is_playing {
                    timeline.frame_accumulator = 0.0;
                }
            }

            // Step Forward 1 Frame (Right Arrow)
            if ui.button(egui::RichText::new("▶").size(13.0)).on_hover_text("Вперед на 1 кадр [→]").clicked() {
                timeline.playhead_frame += 1;
                timeline.is_playing = false;
            }

            // Shuttle Forward (L)
            if ui.button(egui::RichText::new("⏩").size(13.0)).on_hover_text("Shuttle Forward [L]").clicked() {
                timeline.playback_speed = 2.0;
                timeline.is_playing = true;
                timeline.frame_accumulator = 0.0;
            }

            // In / Out Buttons
            if ui.button(egui::RichText::new("[ I ]").size(11.0)).on_hover_text("Mark In [I]").clicked() {
                timeline.in_point = Some(timeline.playhead_frame);
            }
            if ui.button(egui::RichText::new("[ O ]").size(11.0)).on_hover_text("Mark Out [O]").clicked() {
                timeline.out_point = Some(timeline.playhead_frame);
            }

            // Кнопка Loop (Зацикливание)
            let loop_active = timeline.loop_enabled;
            let loop_btn = egui::Button::new(
                egui::RichText::new("🔁 Loop")
                    .size(11.0)
                    .color(if loop_active { Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
            )
            .fill(if loop_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(loop_btn).on_hover_text("Зациклить воспроизведение In/Out диапазона").clicked() {
                timeline.loop_enabled = !timeline.loop_enabled;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewer_pan_zoom_and_cinema_mode() {
        let mut viewer = ViewerState::new();
        assert_eq!(viewer.zoom_mode, ZoomMode::Fit);
        assert_eq!(viewer.pan_offset, egui::Vec2::ZERO);
        assert_eq!(viewer.custom_zoom, None);
        assert!(!viewer.cinema_mode);

        // Включение Cinema Mode
        viewer.cinema_mode = true;
        assert!(viewer.cinema_mode);

        // Панорамирование и произвольный зум
        viewer.pan_offset += egui::vec2(50.0, -30.0);
        viewer.custom_zoom = Some(1.75);
        assert_eq!(viewer.pan_offset, egui::vec2(50.0, -30.0));
        assert_eq!(viewer.custom_zoom, Some(1.75));

        // Сброс к Fit
        viewer.zoom_mode = ZoomMode::Fit;
        viewer.pan_offset = egui::Vec2::ZERO;
        viewer.custom_zoom = None;
        assert_eq!(viewer.pan_offset, egui::Vec2::ZERO);
        assert_eq!(viewer.custom_zoom, None);
    }

    #[test]
    fn test_viewer_ctrl_wheel_zoom_with_transform_overlay() {
        let ctx = egui::Context::default();
        let mut viewer = ViewerState::new();
        viewer.show_transform_overlay = true;
        viewer.single_viewer_mode = true;
        let mut timeline = TimelineState::new_empty();
        timeline.clips.push(crate::timeline::Clip {
            id: 1,
            name: "TestClip".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 200,
            is_audio: false,
            source_path: Some(std::path::PathBuf::from("/tmp/test_cache_poison.mp4")),
            source_fps: 24.0,
            ..Default::default()
        });
        let mut player = VideoPlayer::new();
        let mut cache = crate::render_cache::RenderCacheManager::new(
            crate::render_cache::RenderCacheMode::Smart,
            360,
        );

        // Первый проход для инициализации геометрии окна
        let mut init_input = egui::RawInput::default();
        init_input.screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0)));
        let mut out1 = ctx.run_ui(init_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let c = ui.ctx().clone();
                viewer.render_dual_viewers(ui, &mut timeline, &mut player, &mut cache, (0.0, 0.0), &c);
            });
        });
        out1.textures_delta.clear();

        // Второй проход: курсор в центре монитора (прямо над клипом с включенным Transform OSC) + Ctrl+Wheel (Event::Zoom)
        let mut zoom_input = egui::RawInput::default();
        zoom_input.screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0)));
        zoom_input.events.push(egui::Event::PointerMoved(pos2(600.0, 350.0)));
        zoom_input.events.push(egui::Event::Zoom(1.2));
        let mut out2 = ctx.run_ui(zoom_input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let c = ui.ctx().clone();
                viewer.render_dual_viewers(ui, &mut timeline, &mut player, &mut cache, (0.0, 0.0), &c);
            });
        });
        out2.textures_delta.clear();

        let zoomed = viewer.custom_zoom.expect("Ctrl + MouseWheel (Event::Zoom) must update custom_zoom even over Transform OSC");
        assert!(zoomed > 1.15, "Expected zoom > 1.15, got {}", zoomed);
    }

    #[test]
    fn test_render_cache_never_poisoned_by_neighbor_frame() {
        let ctx = egui::Context::default();
        let mut viewer = ViewerState::new();
        let mut timeline = TimelineState::new_empty();
        timeline.clips.push(crate::timeline::Clip {
            id: 1,
            name: "TestClip".to_string(),
            track_index: 1,
            start_frame: 0,
            duration_frames: 200,
            is_audio: false,
            source_path: Some(std::path::PathBuf::from("/tmp/test_cache_poison.mp4")),
            source_fps: 24.0,
            ..Default::default()
        });
        let mut player = VideoPlayer::new();
        let mut cache = crate::render_cache::RenderCacheManager::new(
            crate::render_cache::RenderCacheMode::Smart,
            360,
        );

        let dummy_path = std::path::PathBuf::from("/tmp/test_cache_poison.mp4");
        if let Some(clip) = timeline.clips.iter_mut().find(|c| !c.is_audio) {
            clip.start_frame = 0;
            clip.duration_frames = 200;
            clip.source_path = Some(dummy_path.clone());
        }
        player.current_path = Some(dummy_path);
        player.total_frames = 200;

        let img_100 = std::sync::Arc::new(egui::ColorImage::filled([16, 16], Color32::RED));
        player.last_displayed_image = Some(std::sync::Arc::clone(&img_100));
        player.last_exact_frame = 100;
        player.last_decoded_frame = 100;

        // Плейхед сдвинут на 101, но декодер ещё показывает старый кадр 100:
        // Кадр 101 НЕ должен быть отравлен картинкой от кадра 100!
        timeline.playhead_frame = 101;
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 720.0)));
        let mut out = ctx.run_ui(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let c = ui.ctx().clone();
                viewer.render_dual_viewers(ui, &mut timeline, &mut player, &mut cache, (0.0, 0.0), &c);
            });
        });
        out.textures_delta.clear();

        assert!(
            !cache.is_cached(101),
            "Frame 101 must NOT be poisoned in RenderCache by stale frame 100"
        );
    }

    #[test]
    fn test_transform_magnetic_edge_and_center_snapping() {
        let mut clip = crate::timeline::Clip {
            zoom_x: 0.5,
            zoom_y: 0.5,
            position_x: 0.0,
            position_y: 0.0,
            ..Default::default()
        };

        // 1. При zoom = 0.5 полуширина клипа = 480 px, полувысота = 270 px.
        // Чтобы левый край клипа коснулся левого края экрана (-960), центр должен быть в X = -480.
        // Подаём raw_x = -474 (в пределах порога 12 px) и raw_y = -265 (в пределах 12 px от верхнего края -270):
        let (sx, sy, gx, gy) = ViewerState::snap_transform_position(-474.0, -265.0, &clip, 12.0);
        assert_eq!(sx, -480.0, "Left edge of 0.5x clip must magnetically snap to left screen border (-960)");
        assert_eq!(sy, -270.0, "Top edge of 0.5x clip must magnetically snap to top screen border (-540)");
        assert_eq!(gx, Some(-960.0));
        assert_eq!(gy, Some(-540.0));

        // 2. Прилипание правого нижнего края к (+960, +540):
        let (sx2, sy2, gx2, gy2) = ViewerState::snap_transform_position(472.0, 263.0, &clip, 12.0);
        assert_eq!(sx2, 480.0);
        assert_eq!(sy2, 270.0);
        assert_eq!(gx2, Some(960.0));
        assert_eq!(gy2, Some(540.0));

        // 3. Прилипание центра к (0, 0):
        let (sx3, sy3, gx3, gy3) = ViewerState::snap_transform_position(5.0, -4.0, &clip, 12.0);
        assert_eq!(sx3, 0.0);
        assert_eq!(sy3, 0.0);
        assert_eq!(gx3, Some(0.0));
        assert_eq!(gy3, Some(0.0));

        // 4. Прилипание масштаба (Resize) к 1.0x и границам экрана:
        clip.position_x = 0.0;
        clip.position_y = 0.0;
        let (szx, szy, _, _) = ViewerState::snap_transform_zoom(0.992, 0.992, &clip, true, 12.0);
        assert!((szx - 1.0).abs() < 1e-4);
        assert!((szy - 1.0).abs() < 1e-4);
    }
}
