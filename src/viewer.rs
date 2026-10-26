use egui::{pos2, vec2, Color32, CornerRadius, Rect, Stroke, StrokeKind, Ui};
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

            vu_level_left: 0.65,
            vu_level_right: 0.62,
        }
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
        let current_sig = crate::render_cache::RenderCacheManager::compute_frame_signature(timeline, timeline.playhead_frame);
        let mut cache_hit = false;
        if render_cache.mode != crate::render_cache::RenderCacheMode::None {
            if let Some(cached_img) = render_cache.get(timeline.playhead_frame, current_sig) {
                video_player.present_playback_frame(ctx, cached_img);
                video_player.last_decoded_frame = timeline.playhead_frame;
                video_player.last_playback_target = timeline.playhead_frame;
                cache_hit = true;
            }
        }

        if !cache_hit {
            // Декодирование текущего кадра активного клипа на таймлайне (приоритет: самая верхняя видеодорожка и последний перемещенный клип)
            let active_clip = timeline
                .clips
                .iter()
                .filter(|c| !c.is_audio && c.contains_frame(timeline.playhead_frame))
                .min_by_key(|c| (c.track_index, if timeline.selected_clip_id == Some(c.id) { 0usize } else { 1usize }));

            if let Some(clip) = active_clip {
                if let Some(path) = &clip.source_path {
                    if video_player.current_path.as_deref() != Some(path.as_path()) {
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

                        video_player.update_playback_with_cut(source_frame, true, upcoming_cut, ctx);
                    } else {
                        video_player.update_playback(source_frame, false, ctx);
                    }
                } else {
                    if timeline.is_playing && !timeline.is_scrubbing {
                        video_player.update_playback(timeline.playhead_frame, true, ctx);
                    } else {
                        video_player.update_playback(timeline.playhead_frame, false, ctx);
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

            // Автоматически сохраняем успешно декодированные кадры в Render Cache для мгновенного повторного воспроизведения
            if render_cache.mode != crate::render_cache::RenderCacheMode::None {
                if let Some(ref img) = video_player.last_displayed_image {
                    render_cache.insert(timeline.playhead_frame, std::sync::Arc::clone(img), current_sig);
                }
                if let Some(clip) = active_clip {
                    for (src_f, img) in video_player.newly_cached_frames.drain(..) {
                        let tl_f = clip.timeline_frame_from_source(src_f, timeline.fps);
                        if clip.contains_frame(tl_f) {
                            let sig = crate::render_cache::RenderCacheManager::compute_frame_signature(timeline, tl_f);
                            render_cache.insert(tl_f, img, sig);
                        }
                    }
                }
            } else {
                video_player.newly_cached_frames.clear();
            }
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

        // ОБЛАСТЬ МОНИТОРОВ: Одиночный или Двойной режим
        if self.single_viewer_mode {
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
            if self.single_viewer_mode {
                let btn = egui::Button::new(
                    egui::RichText::new("👁 Раскрыть Source")
                        .size(11.0)
                        .strong()
                        .color(ResolveColors::TEXT_PRIMARY),
                )
                .fill(ResolveColors::BG_HEADER);
                if ui.add(btn).on_hover_text("Раскрыть левый монитор исходника (Dual Viewer)").clicked() {
                    self.single_viewer_mode = false;
                }
            } else {
                let btn = egui::Button::new(
                    egui::RichText::new("👁 Скрыть Source")
                        .size(11.0)
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(ResolveColors::ACCENT_ORANGE);
                if ui.add(btn).on_hover_text("Скрыть монитор исходника и развернуть Timeline Viewer на 100%").clicked() {
                    self.single_viewer_mode = true;
                }
            }

            ui.separator();

            // 1. Zoom Mode (Fit / 50% / 100% / 200%)
            ui.label(egui::RichText::new("Zoom:").size(10.0).color(ResolveColors::TEXT_MUTED));
            let zoom_label = match self.zoom_mode {
                ZoomMode::Fit => "Fit",
                ZoomMode::Fifty => "50%",
                ZoomMode::Hundred => "100%",
                ZoomMode::TwoHundred => "200%",
            };
            egui::ComboBox::from_id_salt("viewer_zoom_combo")
                .selected_text(zoom_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::Fit, "Fit (По размеру)");
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::Fifty, "50%");
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::Hundred, "100% (Попиксельно)");
                    ui.selectable_value(&mut self.zoom_mode, ZoomMode::TwoHundred, "200%");
                });

            ui.separator();

            // 2. Aspect Ratio / Framing Masks (Каше: 2.39:1, 9:16 Shorts...)
            ui.label(egui::RichText::new("Каше:").size(10.0).color(ResolveColors::TEXT_MUTED));
            let mask_label = match self.aspect_mask {
                AspectMask::Off => "Выкл",
                AspectMask::Cinema239 => "2.39:1 Cinema",
                AspectMask::Cinema185 => "1.85:1 Flat",
                AspectMask::Vertical916 => "9:16 Shorts/Reels",
                AspectMask::Square11 => "1:1 Square",
            };
            egui::ComboBox::from_id_salt("viewer_aspect_combo")
                .selected_text(mask_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Off, "Выкл (Без каше)");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Cinema239, "2.39:1 Anamorphic Cinema");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Cinema185, "1.85:1 Flat Cinema");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Vertical916, "9:16 Shorts / TikTok");
                    ui.selectable_value(&mut self.aspect_mask, AspectMask::Square11, "1:1 Square Instagram");
                });

            ui.separator();

            // 3. Сетки и направляющие (Guides: Thirds, Safe, Cross)
            ui.menu_button("📐 Сетки / Guides", |ui| {
                ui.checkbox(&mut self.show_safe_areas, "Safe Action (90%) / Safe Title (80%)");
                ui.checkbox(&mut self.show_rule_of_thirds, "Сетка третей (Rule of Thirds 3x3)");
                ui.checkbox(&mut self.show_center_cross, "Центральное перекрестие (Crosshair)");
            });

            // 4. Изоляция цветовых каналов (Channel Mode: RGB, R, G, B, Mono)
            ui.menu_button("🎨 Каналы", |ui| {
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::RGB, "RGB (Полный цвет)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Red, "Красный канал (Red)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Green, "Зеленый канал (Green)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Blue, "Синий канал (Blue)");
                ui.selectable_value(&mut self.channel_mode, ColorChannelMode::Monochrome, "Ч/Б Яркость (Monochrome)");
            });

            // 5. Интерактивная рамка трансформации на мониторе (Transform OSC)
            let tf_btn = egui::Button::new(
                egui::RichText::new("⛶ Transform Overlay")
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
                crate::render_cache::RenderCacheMode::None => "⚡ Cache: Off",
                crate::render_cache::RenderCacheMode::Smart => "⚡ Cache: Smart",
                crate::render_cache::RenderCacheMode::User => "⚡ Cache: User",
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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (badge_text, badge_color) = if video_player.gpu_accelerated {
                    (format!("⚡ GPU: {}", video_player.gpu_device_name), ResolveColors::ACCENT_ORANGE)
                } else {
                    ("⚡ GPU: Idle / CPU".to_string(), ResolveColors::TEXT_MUTED)
                };
                ui.label(egui::RichText::new(badge_text).size(10.0).strong().color(badge_color))
                    .on_hover_text("Аппаратное ускорение видеодекодирования на GPU (VA-API / CUDA)");

                ui.separator();

                // Индикатор частоты кадров и Drop Frame в стиле DaVinci Resolve
                let (fps_color, fps_val) = if video_player.is_playing_mode {
                    if video_player.is_dropping_frames {
                        (Color32::from_rgb(255, 69, 58), video_player.real_playback_fps) // Красный (Drop Frame)
                    } else {
                        (Color32::from_rgb(48, 209, 88), video_player.real_playback_fps) // Зеленый (Nominal Real-time)
                    }
                } else {
                    (ResolveColors::TEXT_MUTED, video_player.fps as f32)
                };

                let drop_info = if video_player.is_playing_mode && video_player.dropped_frames_count > 0 {
                    format!("● {:.1} fps (Drop: {})", fps_val, video_player.dropped_frames_count)
                } else {
                    format!("● {:.1} fps", fps_val)
                };

                ui.label(egui::RichText::new(drop_info).size(10.0).strong().color(fps_color))
                    .on_hover_text("Реальный FPS предпросмотра и индикатор пропуска кадров (DaVinci Resolve Drop Frame Indicator)");

                // Кнопка режима воспроизведения: Real-Time Drop Frame vs Show All Frames
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
            });
        });
    }

    fn render_source_viewer(&mut self, ui: &mut Ui, video_player: &VideoPlayer) {
        ui.horizontal(|ui| {
            let label = if let Some(p) = &video_player.current_path {
                p.file_name().and_then(|n| n.to_str()).unwrap_or("SOURCE")
            } else {
                "SOURCE: A001_C001_0422.MOV"
            };
            ui.label(
                egui::RichText::new(format!("SOURCE: {}", label))
                    .size(11.0)
                    .strong()
                    .color(ResolveColors::TEXT_MUTED),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("00:00:02:14")
                        .size(11.0)
                        .color(ResolveColors::TEXT_TIMECODE)
                        .monospace(),
                );
            });
        });

        ui.separator();

        let w = ui.available_width().max(80.0);
        let h = (w * 9.0 / 16.0).max(60.0);
        let (rect, _response) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(2), Color32::from_rgb(0x0a, 0x0b, 0x0d));
        painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, ResolveColors::BORDER), StrokeKind::Inside);

        if let Some(tex) = &video_player.texture {
            painter.image(
                tex.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("In: 00:00:00:00").size(10.0).color(ResolveColors::TEXT_MUTED));
            ui.separator();
            ui.label(egui::RichText::new("Out: 00:00:05:00").size(10.0).color(ResolveColors::TEXT_MUTED));
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
        let w = (ui.available_width() - vu_width - 8.0).max(80.0);
        let h = (w * 9.0 / 16.0).max(60.0);

        ui.horizontal(|ui| {
            let (rect, _response) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());
            let painter = ui.painter_at(rect);

            painter.rect_filled(rect, CornerRadius::same(2), Color32::from_rgb(0x0a, 0x0b, 0x0d));
            painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, ResolveColors::BORDER), StrokeKind::Inside);

            let top_clip = timeline
                .clips
                .iter()
                .rev()
                .find(|c| !c.is_audio && c.contains_frame(timeline.playhead_frame));

            if let Some(top_clip) = top_clip {
                let center = rect.center();

                // Применение зума монитора (Fit / 50% / 100% / 200%)
                let monitor_scale = match self.zoom_mode {
                    ZoomMode::Fit => 1.0,
                    ZoomMode::Fifty => 0.5,
                    ZoomMode::Hundred => 1.0 * (1080.0 / rect.height().max(1.0)),
                    ZoomMode::TwoHundred => 2.0,
                };

                let video_box_size = vec2(
                    rect.width() * top_clip.zoom_x * monitor_scale,
                    rect.height() * top_clip.zoom_y * monitor_scale,
                );
                let video_box_rect = Rect::from_center_size(
                    center + vec2(top_clip.position_x * 0.2, top_clip.position_y * 0.2),
                    video_box_size,
                );

                // Изоляция цветовых каналов
                let alpha = (top_clip.opacity / 100.0 * 255.0) as u8;
                let tint = match self.channel_mode {
                    ColorChannelMode::RGB => Color32::from_rgba_unmultiplied(255, 255, 255, alpha),
                    ColorChannelMode::Red => Color32::from_rgba_unmultiplied(255, 60, 60, alpha),
                    ColorChannelMode::Green => Color32::from_rgba_unmultiplied(60, 255, 60, alpha),
                    ColorChannelMode::Blue => Color32::from_rgba_unmultiplied(80, 120, 255, alpha),
                    ColorChannelMode::Monochrome => Color32::from_rgba_unmultiplied(200, 200, 200, alpha),
                };

                // Отрисовка живого кадра видео
                if let Some(tex) = &video_player.texture {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    // Временной GPU-блендинг соседних кадров (как interpolation в mpv): при 60 fps на мониторе 75 Гц
                    // кадры иначе повторяются неравномерно («заедающая пластинка»). Вес = доля до следующего кадра таймлайна.
                    let is_blendable = timeline.is_playing
                        && !timeline.is_scrubbing
                        && (timeline.playback_speed - 1.0).abs() < 1e-6
                        && alpha == 255
                        && (top_clip.source_fps - timeline.fps as f64).abs() < 0.1;
                    match video_player.blend_source().filter(|_| is_blendable) {
                        Some(prev) => {
                            let weight = timeline.frame_accumulator.clamp(0.0, 1.0) as f32;
                            painter.image(prev.id(), video_box_rect, uv, tint);
                            painter.image(tex.id(), video_box_rect, uv, tint.gamma_multiply(weight));
                        }
                        None => {
                            painter.image(tex.id(), video_box_rect, uv, tint);
                        }
                    }
                }

                // Интерактивная рамка трансформации (Transform Overlay как в Resolve)
                if self.show_transform_overlay {
                    painter.rect_stroke(
                        video_box_rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.5, ResolveColors::ACCENT_ORANGE),
                        StrokeKind::Inside,
                    );
                    // 4 угловые точки управления
                    let handle_sz = 6.0;
                    let corners = [
                        video_box_rect.left_top(),
                        video_box_rect.right_top(),
                        video_box_rect.left_bottom(),
                        video_box_rect.right_bottom(),
                    ];
                    for c in corners {
                        painter.rect_filled(
                            Rect::from_center_size(c, vec2(handle_sz, handle_sz)),
                            CornerRadius::ZERO,
                            ResolveColors::ACCENT_ORANGE,
                        );
                    }
                }
            } else {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "NO CLIP AT PLAYHEAD",
                    egui::FontId::monospace(11.0),
                    Color32::from_rgb(0x50, 0x55, 0x60),
                );
            }

            // 1. Оверлей Каше (Aspect Ratio Mask: Cinema 2.39:1, 9:16 Shorts...)
            Self::draw_aspect_mask(&painter, rect, self.aspect_mask);

            // 2. Направляющие Safe Areas (80% / 90%)
            if self.show_safe_areas {
                let safe_action = rect.shrink2(vec2(rect.width() * 0.05, rect.height() * 0.05));
                let safe_title = rect.shrink2(vec2(rect.width() * 0.10, rect.height() * 0.10));
                painter.rect_stroke(safe_action, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x3a, 0x55, 0x77)), StrokeKind::Inside);
                painter.rect_stroke(safe_title, CornerRadius::ZERO, Stroke::new(1.0, Color32::from_rgb(0x66, 0x44, 0x22)), StrokeKind::Inside);
            }

            // 3. Сетка третей (Rule of Thirds)
            if self.show_rule_of_thirds {
                let w_third = rect.width() / 3.0;
                let h_third = rect.height() / 3.0;
                let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 60));
                painter.line_segment([pos2(rect.min.x + w_third, rect.min.y), pos2(rect.min.x + w_third, rect.max.y)], stroke);
                painter.line_segment([pos2(rect.min.x + w_third * 2.0, rect.min.y), pos2(rect.min.x + w_third * 2.0, rect.max.y)], stroke);
                painter.line_segment([pos2(rect.min.x, rect.min.y + h_third), pos2(rect.max.x, rect.min.y + h_third)], stroke);
                painter.line_segment([pos2(rect.min.x, rect.min.y + h_third * 2.0), pos2(rect.max.x, rect.min.y + h_third * 2.0)], stroke);
            }

            // 4. Центральное перекрестие (Crosshair)
            if self.show_center_cross {
                let c = rect.center();
                let stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(232, 124, 36, 160));
                painter.line_segment([pos2(c.x - 15.0, c.y), pos2(c.x + 15.0, c.y)], stroke);
                painter.line_segment([pos2(c.x, c.y - 15.0), pos2(c.x, c.y + 15.0)], stroke);
            }

            // Master Audio VU Meter (справа)
            self.render_vu_meter(ui, h);
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
            ui.add_space(ui.available_width() * 0.12);

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
