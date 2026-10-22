use egui::{pos2, vec2, Color32, CornerRadius, Rect, Stroke, StrokeKind, Ui};
use crate::theme::ResolveColors;
use crate::timeline::{Clip, TimelineState, Tool};
use crate::timecode::Timecode;

pub struct TimelineView;

impl TimelineView {
    pub fn render(ui: &mut Ui, timeline: &mut TimelineState) {
        // 1. Верхний тулбар таймлайна
        Self::render_toolbar(ui, timeline);
        ui.separator();

        // 2. Основная область: Заголовки треков слева + Сетка таймлайна справа
        let header_width = 110.0;
        let ruler_height = 24.0;
        let track_height = 44.0;
        let total_tracks = timeline.tracks.len();
        let timeline_height = ruler_height + (total_tracks as f32 * track_height) + 16.0;

        let available_size = vec2(
            ui.available_width().max(100.0),
            timeline_height.max((ui.available_height() - 10.0).max(100.0)),
        );
        let (full_rect, _resp) = ui.allocate_exact_size(available_size, egui::Sense::hover());

        let painter = ui.painter_at(full_rect);
        // Фон всего блока
        painter.rect_filled(full_rect, CornerRadius::ZERO, ResolveColors::BG_MAIN);

        // Область заголовков треков
        let headers_rect = Rect::from_min_size(full_rect.min, vec2(header_width, full_rect.height()));
        painter.rect_filled(headers_rect, CornerRadius::ZERO, ResolveColors::BG_PANEL);
        painter.line_segment(
            [pos2(headers_rect.max.x, full_rect.min.y), pos2(headers_rect.max.x, full_rect.max.y)],
            Stroke::new(1.0, ResolveColors::BORDER),
        );

        // Область холста таймлайна (справа от заголовков)
        let canvas_rect = Rect::from_min_size(
            pos2(full_rect.min.x + header_width, full_rect.min.y),
            vec2(full_rect.width() - header_width, full_rect.height()),
        );

        let ruler_rect = Rect::from_min_size(canvas_rect.min, vec2(canvas_rect.width(), ruler_height));
        let tracks_canvas_rect = Rect::from_min_max(
            pos2(canvas_rect.min.x, ruler_rect.max.y),
            canvas_rect.max,
        );

        let mouse_pos = ui.input(|i| i.pointer.hover_pos());

        // 1. Взаимодействие со скраббингом исключительно по линейке времени (Ruler)
        let ruler_sense = ui.interact(ruler_rect, ui.id().with("ruler_scrub"), egui::Sense::click_and_drag());
        if ruler_sense.dragged() || ruler_sense.clicked() {
            if let Some(m_pos) = mouse_pos {
                let rel_x = m_pos.x - canvas_rect.min.x + timeline.scroll_x;
                let target_frame = (rel_x / timeline.zoom_factor).max(0.0) as i64;
                timeline.playhead_frame = target_frame;
            }
        }

        // Зум таймлайна колесиком мыши с Ctrl
        if canvas_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta != 0.0 {
                if ui.input(|i| i.modifiers.ctrl) {
                    let zoom_delta = if scroll_delta > 0.0 { 1.2 } else { 0.8 };
                    timeline.zoom_factor = (timeline.zoom_factor * zoom_delta).clamp(0.2, 20.0);
                } else {
                    timeline.scroll_x = (timeline.scroll_x - scroll_delta * 2.0).max(0.0);
                }
            }
        }

        // 3. Отрисовка Ruler (Линейки времени)
        Self::draw_ruler(&painter, ruler_rect, timeline);

        // 4. Отрисовка дорожек (Tracks) и заголовков
        for (i, track) in timeline.tracks.iter().enumerate() {
            let y_top = ruler_rect.max.y + (i as f32 * track_height);
            let track_row_rect = Rect::from_min_size(pos2(canvas_rect.min.x, y_top), vec2(canvas_rect.width(), track_height));
            let header_row_rect = Rect::from_min_size(pos2(full_rect.min.x, y_top), vec2(header_width, track_height));

            // Заголовок дорожки
            Self::draw_track_header(&painter, header_row_rect, track, i);

            // Фон полосы дорожки
            let bg_color = if i % 2 == 0 {
                Color32::from_rgb(0x19, 0x1a, 0x1d)
            } else {
                Color32::from_rgb(0x1c, 0x1d, 0x20)
            };
            painter.rect_filled(track_row_rect, CornerRadius::ZERO, bg_color);
            painter.line_segment(
                [pos2(track_row_rect.min.x, track_row_rect.max.y), pos2(track_row_rect.max.x, track_row_rect.max.y)],
                Stroke::new(1.0, ResolveColors::BORDER),
            );
        }

        // 5. Отрисовка и высокоточная интерактивность клипов на дорожках
        let drag_session_id = ui.id().with("clip_moving_session");
        let mut moving_session: Option<(u64, f32, i64)> = ui.data(|d| d.get_temp(drag_session_id));

        let mut hovered_clip_id = None;
        let mut clicked_clip_id = None;
        let mut cut_request = None;
        let mut start_drag_request = None;

        for clip in &timeline.clips {
            if clip.track_index >= timeline.tracks.len() {
                continue;
            }
            let track_y = ruler_rect.max.y + (clip.track_index as f32 * track_height);
            let clip_x1 = canvas_rect.min.x + (clip.start_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
            let clip_width = (clip.duration_frames as f32 * timeline.zoom_factor).max(4.0);
            let clip_rect = Rect::from_min_size(
                pos2(clip_x1, track_y + 2.0),
                vec2(clip_width, track_height - 4.0),
            );

            // Видимость клипа в пределах canvas
            if clip_rect.max.x < canvas_rect.min.x || clip_rect.min.x > canvas_rect.max.x {
                continue;
            }

            // Связанная подсветка клипов при выделении
            let is_selected = if let Some(sel_id) = timeline.selected_clip_id {
                if sel_id == clip.id {
                    true
                } else if timeline.linked_selection && clip.link_group.is_some() {
                    timeline.clips.iter().any(|c| c.id == sel_id && c.link_group == clip.link_group)
                } else {
                    false
                }
            } else {
                false
            };

            Self::draw_clip(&painter, clip_rect, clip, is_selected);

            // Четкая регистрация клика и перетаскивания клипа
            let clip_resp = ui.interact(clip_rect, ui.id().with(format!("clip_{}", clip.id)), egui::Sense::click_and_drag());
            if clip_resp.hovered() {
                hovered_clip_id = Some(clip.id);
            }

            if clip_resp.clicked() {
                if timeline.active_tool == Tool::Blade {
                    if let Some(m_pos) = mouse_pos {
                        let rel_frame = ((m_pos.x - canvas_rect.min.x + timeline.scroll_x) / timeline.zoom_factor).round() as i64;
                        cut_request = Some((clip.id, rel_frame));
                    }
                } else {
                    clicked_clip_id = Some(clip.id);
                }
            }

            // Инициализация плавного drag-перемещения клипа
            if clip_resp.drag_started() && timeline.active_tool == Tool::Selection {
                if let Some(m_pos) = mouse_pos {
                    start_drag_request = Some((clip.id, m_pos.x, clip.start_frame));
                }
            }

            // Лезвие (Blade) hover линия
            if timeline.active_tool == Tool::Blade {
                if let Some(m_pos) = mouse_pos {
                    if clip_rect.contains(m_pos) {
                        painter.line_segment(
                            [pos2(m_pos.x, clip_rect.min.y), pos2(m_pos.x, clip_rect.max.y)],
                            Stroke::new(2.0, Color32::from_rgb(0xff, 0x44, 0x44)),
                        );
                    }
                }
            }
        }

        if let Some((id, mouse_x, start_fr)) = start_drag_request {
            timeline.push_history();
            moving_session = Some((id, mouse_x, start_fr));
        }

        // Обновление позиции перетаскиваемого клипа (и всех связанных с ним)
        if let Some((drag_id, start_mouse_x, init_start_frame)) = moving_session {
            if let Some(m_pos) = mouse_pos {
                let dx = m_pos.x - start_mouse_x;
                let delta_frames = (dx / timeline.zoom_factor).round() as i64;
                let target_start = (init_start_frame + delta_frames).max(0);
                timeline.move_clip(drag_id, target_start);
            }
            if ui.input(|i| i.pointer.any_released()) {
                moving_session = None;
            }
        }
        ui.data_mut(|d| d.insert_temp(drag_session_id, moving_session));

        // Клик по пустой области дорожек (где нет клипов) — перемещение плейхеда
        if moving_session.is_none() && hovered_clip_id.is_none() && timeline.dragged_media.is_none() {
            let canvas_sense = ui.interact(tracks_canvas_rect, ui.id().with("empty_tracks_canvas"), egui::Sense::click_and_drag());
            if canvas_sense.clicked() || canvas_sense.dragged() {
                if let Some(m_pos) = mouse_pos {
                    let rel_x = m_pos.x - canvas_rect.min.x + timeline.scroll_x;
                    let target_frame = (rel_x / timeline.zoom_factor).max(0.0) as i64;
                    timeline.playhead_frame = target_frame;
                    if canvas_sense.clicked() && timeline.active_tool != Tool::Blade {
                        timeline.selected_clip_id = None;
                    }
                }
            }
        }

        // Применение действий с клипами
        if let Some(id) = clicked_clip_id {
            timeline.selected_clip_id = Some(id);
        }
        if let Some((id, cut_frame)) = cut_request {
            timeline.blade_cut_at(id, cut_frame);
        }

        // 6. ОБРАБОТКА И ОТРИСОВКА ПЕРЕТАСКИВАНИЯ НА ДОРОЖКУ (DRAG & DROP ONTO TRACK)
        let mut drop_completed = false;
        if let Some(media) = &timeline.dragged_media.clone() {
            if let Some(m_pos) = mouse_pos {
                if canvas_rect.contains(m_pos) {
                    let rel_y = m_pos.y - ruler_rect.max.y;
                    let target_track_idx = ((rel_y / track_height).floor() as usize).min(timeline.tracks.len().saturating_sub(1));
                    let rel_x = m_pos.x - canvas_rect.min.x + timeline.scroll_x;
                    let raw_frame = (rel_x / timeline.zoom_factor).max(0.0) as i64;

                    // Прилипание при сбросе
                    let mut drop_frame = raw_frame;
                    if timeline.snapping_enabled {
                        let snap_thresh = (12.0 / timeline.zoom_factor) as i64;
                        if (raw_frame - timeline.playhead_frame).abs() <= snap_thresh {
                            drop_frame = timeline.playhead_frame;
                        }
                    }

                    let ghost_y = ruler_rect.max.y + (target_track_idx as f32 * track_height);
                    let ghost_x = canvas_rect.min.x + (drop_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
                    let ghost_w = (media.duration_frames as f32 * timeline.zoom_factor).max(12.0);
                    let ghost_rect = Rect::from_min_size(pos2(ghost_x, ghost_y + 2.0), vec2(ghost_w, track_height - 4.0));

                    // Полупрозрачный оранжевый фантом на дорожке под курсором
                    painter.rect_filled(
                        ghost_rect,
                        CornerRadius::same(2),
                        Color32::from_rgba_unmultiplied(232, 124, 36, 120),
                    );
                    painter.rect_stroke(
                        ghost_rect,
                        CornerRadius::same(2),
                        Stroke::new(2.0, ResolveColors::ACCENT_ORANGE),
                        StrokeKind::Inside,
                    );
                    painter.text(
                        pos2(ghost_rect.min.x + 6.0, ghost_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("➕ Drop: {}", media.name),
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );

                    // Если кнопку мыши отпустили — размещаем клип на выбранную дорожку!
                    if ui.input(|i| i.pointer.any_released()) {
                        timeline.push_history();
                        let new_clip = Clip {
                            id: timeline.next_clip_id,
                            name: media.name.clone(),
                            track_index: target_track_idx,
                            start_frame: drop_frame,
                            duration_frames: media.duration_frames,
                            is_audio: media.is_audio,
                            color: if media.is_audio {
                                ResolveColors::AUDIO_CLIP
                            } else {
                                ResolveColors::VIDEO_CLIP
                            },
                            source_path: media.path.clone(),
                            source_in_frame: 0,
                            source_fps: media.source_fps,
                            link_group: media.link_group,
                            zoom_x: 1.0,
                            zoom_y: 1.0,
                            position_x: 0.0,
                            position_y: 0.0,
                            rotation: 0.0,
                            opacity: 100.0,
                            volume_db: 0.0,
                            pan: 0.0,
                        };
                        timeline.next_clip_id += 1;
                        timeline.selected_clip_id = Some(new_clip.id);
                        timeline.clips.push(new_clip);
                        drop_completed = true;
                    }
                }
            }
        }
        if drop_completed || (timeline.dragged_media.is_some() && ui.input(|i| i.pointer.any_released())) {
            timeline.dragged_media = None;
        }

        // 7. Отрисовка фирменного оранжевого Playhead Resolve
        Self::draw_playhead(&painter, canvas_rect, ruler_rect, timeline);
    }

    fn render_toolbar(ui: &mut Ui, timeline: &mut TimelineState) {
        ui.horizontal(|ui| {
            // Кнопки инструментов
            let arrow_active = timeline.active_tool == Tool::Selection;
            let arrow_btn = egui::Button::new(
                egui::RichText::new("↖ Select (A)").size(11.0).color(if arrow_active { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
            ).fill(if arrow_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(arrow_btn).on_hover_text("Инструмент выделения [A]").clicked() {
                timeline.active_tool = Tool::Selection;
            }

            let blade_active = timeline.active_tool == Tool::Blade;
            let blade_btn = egui::Button::new(
                egui::RichText::new("✂ Blade (B)").size(11.0).color(if blade_active { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
            ).fill(if blade_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(blade_btn).on_hover_text("Лезвие для нарезки клипов [B]").clicked() {
                timeline.active_tool = Tool::Blade;
            }

            if ui.button(egui::RichText::new("✂ Cut at Playhead").size(11.0)).on_hover_text("Разрезать по плейхеду [Ctrl+B]").clicked() {
                timeline.blade_all_at_playhead();
            }

            ui.separator();

            if ui.button(egui::RichText::new("📥 Импорт медиа (Ctrl+I)").size(11.0)).on_hover_text("Импортировать медиафайлы [Ctrl+I]").clicked() {
                if let Some(files) = rfd::FileDialog::new()
                    .add_filter("Media Files", &["mp4", "mov", "mkv", "avi", "webm", "wav", "mp3", "flac"])
                    .pick_files()
                {
                    for f in files {
                        timeline.pending_imports.push(f);
                    }
                }
            }

            ui.separator();

            // Магнит (Snapping: N)
            let snap_text = if timeline.snapping_enabled { "🧲 Snapping: ON" } else { "🧲 Snapping: OFF" };
            if ui.selectable_label(timeline.snapping_enabled, egui::RichText::new(snap_text).size(11.0)).on_hover_text("Магнитное прилипание [N]").clicked() {
                timeline.snapping_enabled = !timeline.snapping_enabled;
            }

            // Связность клипов (Linked Selection: Ctrl+Shift+L)
            let link_text = if timeline.linked_selection { "🔗 Link: ON" } else { "🔗 Link: OFF" };
            if ui.selectable_label(timeline.linked_selection, egui::RichText::new(link_text).size(11.0)).on_hover_text("Связность видео и аудиоклипов [Ctrl+Shift+L]").clicked() {
                timeline.linked_selection = !timeline.linked_selection;
            }

            ui.separator();

            // Zoom контролы
            ui.label(egui::RichText::new("Zoom:").size(11.0).color(ResolveColors::TEXT_MUTED));
            if ui.button("➖").clicked() {
                timeline.zoom_factor = (timeline.zoom_factor * 0.75).clamp(0.2, 20.0);
            }
            ui.add(egui::Slider::new(&mut timeline.zoom_factor, 0.2..=10.0).show_value(false));
            if ui.button("➕").clicked() {
                timeline.zoom_factor = (timeline.zoom_factor * 1.33).clamp(0.2, 20.0);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("🗑 Delete Clip").on_hover_text("Удалить выбранный клип [Del]").clicked() {
                    timeline.delete_selected_clip();
                }
                if ui.button("⮌ Undo").on_hover_text("Отмена действия [Ctrl+Z]").clicked() {
                    timeline.undo();
                }
            });
        });
    }

    fn draw_ruler(painter: &egui::Painter, rect: Rect, timeline: &TimelineState) {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x1a, 0x1b, 0x1e));
        painter.line_segment([pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)], Stroke::new(1.0, ResolveColors::BORDER));

        let frames_per_sec = timeline.fps as f32;
        let sec_width = frames_per_sec * timeline.zoom_factor;

        let start_sec = (timeline.scroll_x / sec_width).floor() as i64;
        let end_sec = ((timeline.scroll_x + rect.width()) / sec_width).ceil() as i64;

        for s in start_sec..=end_sec {
            let x = rect.min.x + (s as f32 * sec_width) - timeline.scroll_x;
            if x < rect.min.x || x > rect.max.x {
                continue;
            }

            // Большая засечка секунды
            painter.line_segment([pos2(x, rect.max.y - 12.0), pos2(x, rect.max.y)], Stroke::new(1.0, ResolveColors::BORDER_LIGHT));

            // Таймкод текст
            let tc = Timecode::from_seconds(s as f64, timeline.fps).to_smpte();
            painter.text(
                pos2(x + 4.0, rect.min.y + 4.0),
                egui::Align2::LEFT_TOP,
                tc,
                egui::FontId::monospace(9.0),
                ResolveColors::TEXT_MUTED,
            );

            // Мелкие засечки кадров
            if sec_width > 40.0 {
                let sub_ticks = 4;
                for sub in 1..sub_ticks {
                    let sub_x = x + (sub as f32 * sec_width / sub_ticks as f32);
                    if sub_x >= rect.min.x && sub_x <= rect.max.x {
                        painter.line_segment([pos2(sub_x, rect.max.y - 6.0), pos2(sub_x, rect.max.y)], Stroke::new(1.0, ResolveColors::BORDER));
                    }
                }
            }
        }
    }

    fn draw_track_header(painter: &egui::Painter, rect: Rect, track: &crate::timeline::Track, _index: usize) {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x1e, 0x20, 0x24));
        painter.line_segment([pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)], Stroke::new(1.0, ResolveColors::BORDER));

        // Цветовой индикатор трека слева
        let badge_color = if track.is_video { ResolveColors::VIDEO_CLIP } else { ResolveColors::AUDIO_CLIP };
        let badge_rect = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
        painter.rect_filled(badge_rect, CornerRadius::ZERO, badge_color);

        // Название трека
        painter.text(
            pos2(rect.min.x + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &track.name,
            egui::FontId::proportional(11.0),
            ResolveColors::TEXT_PRIMARY,
        );

        // Иконки трека (Lock / Mute)
        painter.text(
            pos2(rect.max.x - 28.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            if track.is_video { "🎬" } else { "🔊" },
            egui::FontId::proportional(10.0),
            ResolveColors::TEXT_MUTED,
        );
    }

    fn draw_clip(painter: &egui::Painter, rect: Rect, clip: &Clip, is_selected: bool) {
        let fill_color = if is_selected {
            if clip.is_audio { ResolveColors::AUDIO_CLIP_SELECTED } else { ResolveColors::VIDEO_CLIP_SELECTED }
        } else {
            clip.color
        };

        let stroke = if is_selected {
            Stroke::new(1.5, ResolveColors::ACCENT_ORANGE)
        } else {
            Stroke::new(1.0, Color32::from_rgb(0x18, 0x33, 0x4f))
        };

        painter.rect_filled(rect, CornerRadius::same(2), fill_color);
        painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Inside);

        // Для аудиоклипов отрисовываем реалистичные волноформы (Waveforms)
        if clip.is_audio && rect.width() > 20.0 {
            let mid_y = rect.center().y;
            let step = 3.0;
            let mut x = rect.min.x + 2.0;
            let mut idx = 0;
            while x < rect.max.x - 2.0 {
                let pseudo_amp = ((idx as f32 * 0.4).sin().abs() * 0.7 + (idx as f32 * 0.9).cos().abs() * 0.3) * (rect.height() * 0.35);
                painter.line_segment(
                    [pos2(x, mid_y - pseudo_amp), pos2(x, mid_y + pseudo_amp)],
                    Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 120)),
                );
                x += step;
                idx += 1;
            }
        }

        // Название клипа
        if rect.width() > 30.0 {
            let label = if clip.link_group.is_some() {
                format!("🔗 {}", clip.name)
            } else {
                clip.name.clone()
            };
            painter.text(
                pos2(rect.min.x + 4.0, rect.min.y + 4.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
    }

    fn draw_playhead(painter: &egui::Painter, canvas: Rect, ruler: Rect, timeline: &TimelineState) {
        let playhead_x = canvas.min.x + (timeline.playhead_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
        if playhead_x < canvas.min.x || playhead_x > canvas.max.x {
            return;
        }

        let orange = ResolveColors::ACCENT_ORANGE;

        // 1. Верхний треугольный бегунок Resolve на линейке
        let tri_top = ruler.min.y;
        let tri_bottom = ruler.max.y;
        let tri_half_w = 6.0;

        let points = vec![
            pos2(playhead_x - tri_half_w, tri_top),
            pos2(playhead_x + tri_half_w, tri_top),
            pos2(playhead_x + tri_half_w, tri_bottom - 4.0),
            pos2(playhead_x, tri_bottom),
            pos2(playhead_x - tri_half_w, tri_bottom - 4.0),
        ];

        painter.add(egui::Shape::convex_polygon(points, orange, Stroke::NONE));

        // 2. Вертикальная направляющая линия через все треки
        painter.line_segment(
            [pos2(playhead_x, ruler.max.y), pos2(playhead_x, canvas.max.y)],
            Stroke::new(1.5, orange),
        );
    }
}
