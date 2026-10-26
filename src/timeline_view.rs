use egui::{pos2, vec2, Color32, CornerRadius, Rect, Stroke, StrokeKind, Ui};
use crate::theme::ResolveColors;
use crate::timeline::{Clip, TimelineState, Tool};
use crate::timecode::Timecode;

pub struct TimelineView;

impl TimelineView {
    pub fn render(
        ui: &mut Ui,
        timeline: &mut TimelineState,
        waveform_mgr: &crate::waveform::WaveformManager,
        render_cache: &crate::render_cache::RenderCacheManager,
    ) {
        // 1. Верхний тулбар таймлайна
        Self::render_toolbar(ui, timeline);
        ui.separator();

        // 2. Основная область: Заголовки треков слева + Сетка таймлайна справа
        let header_width = 140.0;
        let ruler_height = 24.0;
        let total_tracks_h = timeline.total_tracks_height();
        let timeline_height = ruler_height + total_tracks_h + 16.0;

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

        // Создаем изолированные области рисования (Clipping Painters), чтобы ни один элемент
        // с таймлайна (клипы, звуковая волна, сетка, плейхед) физически не мог наложиться на панель треков слева
        let ruler_painter = painter.with_clip_rect(ruler_rect);
        let tracks_painter = painter.with_clip_rect(tracks_canvas_rect);
        let canvas_painter = painter.with_clip_rect(canvas_rect);
        let headers_painter = painter.with_clip_rect(headers_rect);

        let mouse_pos = ui.input(|i| i.pointer.hover_pos());

        // 1. Взаимодействие со скраббингом курсора ИСКЛЮЧИТЕЛЬНО по линейке времени с цифрами (Ruler как в DaVinci Resolve)
        let ruler_sense = ui.interact(ruler_rect, ui.id().with("ruler_scrub"), egui::Sense::click_and_drag());
        if ruler_sense.hovered() || ruler_sense.dragged() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
        }
        if ruler_sense.dragged() || ruler_sense.clicked() || (ruler_sense.hovered() && ui.input(|i| i.pointer.primary_down())) {
            timeline.is_scrubbing = ui.input(|i| i.pointer.primary_down());
            timeline.frame_accumulator = 0.0;
            if let Some(m_pos) = mouse_pos {
                let rel_x = m_pos.x - canvas_rect.min.x + timeline.scroll_x;
                let target_frame = (rel_x / timeline.zoom_factor).max(0.0) as i64;
                timeline.playhead_frame = target_frame;
            }
        } else if ruler_sense.drag_stopped() {
            timeline.is_scrubbing = false;
        }

        // 2. Навигация по таймлайну мышью и хоткеями как в DaVinci Resolve 21:
        // Alt + Колесико (или Ctrl + Колесико) — плавный зум с центрированием на курсоре мыши
        // Зажатие колесика мыши (Middle Mouse Button / Hand tool) — панорамирование таймлайна
        // Обычное колесико — горизонтальный скролл таймлайна
        if let Some(m_pos) = mouse_pos {
            if full_rect.contains(m_pos) {
                // Панорамирование и зум зажатием колесика мыши (MMB Drag)
                let mmb_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
                let alt_down = ui.input(|i| i.modifiers.alt || i.modifiers.ctrl);
                if mmb_down && alt_down {
                    // Alt + MMB Drag: плавный аналоговый зум движением мыши (как в DaVinci Resolve)
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ZoomIn);
                    let delta = ui.input(|i| i.pointer.delta());
                    if delta.x != 0.0 || delta.y != 0.0 {
                        let drag_zoom = ((delta.x - delta.y) * 0.012).clamp(-0.3, 0.3).exp();
                        let old_zoom = timeline.zoom_factor;
                        let new_zoom = (old_zoom * drag_zoom).clamp(0.0005, 50.0);
                        let mouse_rel_x = m_pos.x - canvas_rect.min.x;
                        let target_frame = (mouse_rel_x + timeline.scroll_x) / old_zoom;
                        timeline.zoom_factor = new_zoom;
                        timeline.scroll_x = (target_frame * new_zoom - mouse_rel_x).max(0.0);
                    }
                } else if mmb_down {
                    ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::AllScroll);
                    let delta = ui.input(|i| i.pointer.delta());
                    if delta.x != 0.0 {
                        timeline.scroll_x = (timeline.scroll_x - delta.x).max(0.0);
                    }
                }

                // Скролл и Зум колесиком мыши (поддержка любых мышей, тачпадов и колесиков с засечками)
                let (scroll_x, scroll_y) = ui.input(|i| {
                    let mut dx = i.smooth_scroll_delta.x;
                    let mut dy = i.smooth_scroll_delta.y;
                    for ev in &i.events {
                        if let egui::Event::MouseWheel { delta, .. } = ev {
                            if dx.abs() < 0.001 {
                                dx = delta.x;
                            }
                            if dy.abs() < 0.001 {
                                dy = delta.y;
                            }
                        }
                    }
                    (dx, dy)
                });

                if scroll_x != 0.0 {
                    timeline.scroll_x = (timeline.scroll_x - scroll_x * 1.5).max(0.0);
                }

                if scroll_y != 0.0 {
                    let is_zoom = ui.input(|i| i.modifiers.alt || i.modifiers.ctrl || i.modifiers.command);
                    let is_shift = ui.input(|i| i.modifiers.shift);

                    if is_zoom {
                        // Плавный гранулярный зум с настраиваемым шагом (по умолчанию 8%) под курсором мыши
                        let old_zoom = timeline.zoom_factor;
                        let step = (timeline.zoom_step_percent / 100.0).clamp(0.01, 0.35);
                        let raw_delta = if scroll_y.abs() > 30.0 { scroll_y.signum() } else { scroll_y * 0.05 };
                        let factor = (1.0 + step * raw_delta).clamp(0.5, 2.0);
                        let new_zoom = (old_zoom * factor).clamp(0.0005, 50.0);

                        let mouse_rel_x = m_pos.x - canvas_rect.min.x;
                        let target_frame = (mouse_rel_x + timeline.scroll_x) / old_zoom;
                        timeline.zoom_factor = new_zoom;
                        timeline.scroll_x = (target_frame * new_zoom - mouse_rel_x).max(0.0);
                    } else if is_shift {
                        // Shift + Колесо: вертикальный скролл дорожек
                        timeline.scroll_y = (timeline.scroll_y - scroll_y * 1.5).max(0.0);
                    } else {
                        // Обычное колесико мыши: горизонтальный скролл таймлайна
                        timeline.scroll_x = (timeline.scroll_x - scroll_y * 2.0).max(0.0);
                    }
                }
            }
        }

        // Ограничение горизонтального и вертикального скролла дорожек
        let max_scroll_x = (timeline.timeline_end_frame() as f32 * timeline.zoom_factor + canvas_rect.width() * 0.75).max(0.0);
        timeline.scroll_x = timeline.scroll_x.clamp(0.0, max_scroll_x);

        let total_tracks_h = timeline.total_tracks_height();
        let max_scroll_y = (total_tracks_h - (tracks_canvas_rect.height() - 20.0)).max(0.0);
        timeline.scroll_y = timeline.scroll_y.clamp(0.0, max_scroll_y);

        // 3. Отрисовка Ruler (Линейки времени) строго с изоляцией ruler_painter
        Self::draw_ruler(&ruler_painter, ruler_rect, timeline, render_cache);

        // 4. Отрисовка полос дорожек таймлайна с учетом индивидуальной высоты каждой дорожки
        for (i, _track) in timeline.tracks.iter().enumerate() {
            let t_h = timeline.track_height(i);
            let y_top = ruler_rect.max.y + timeline.track_top_y(i) - timeline.scroll_y;
            let track_row_rect = Rect::from_min_size(pos2(canvas_rect.min.x, y_top), vec2(canvas_rect.width(), t_h));

            // Отсечение невидимых по вертикали дорожек
            if track_row_rect.max.y < tracks_canvas_rect.min.y || track_row_rect.min.y > tracks_canvas_rect.max.y {
                continue;
            }

            // Фон полосы дорожки
            let bg_color = if i % 2 == 0 {
                Color32::from_rgb(0x19, 0x1a, 0x1d)
            } else {
                Color32::from_rgb(0x1c, 0x1d, 0x20)
            };
            tracks_painter.rect_filled(track_row_rect, CornerRadius::ZERO, bg_color);
            tracks_painter.line_segment(
                [pos2(track_row_rect.min.x, track_row_rect.max.y), pos2(track_row_rect.max.x, track_row_rect.max.y)],
                Stroke::new(1.0, ResolveColors::BORDER),
            );
        }

        // 5. Отрисовка и высокоточная интерактивность клипов на дорожках (Trimming и Moving)
        let mut hovered_clip_id = None;
        let mut clicked_clip_id = None;
        let mut start_drag_request = None;
        let mut start_trim_request = None;
        let mut start_retime_session = None;

        // Предварительное вычисление link_group выделенного клипа: O(N) один раз вместо O(N^2) сканирования
        let selected_link_group = if timeline.linked_selection {
            timeline.selected_clip_id.and_then(|sel_id| {
                timeline.clips.iter().find(|c| c.id == sel_id).and_then(|c| c.link_group)
            })
        } else {
            None
        };

        for clip in &timeline.clips {
            if clip.track_index >= timeline.tracks.len() {
                continue;
            }
            let track_y = ruler_rect.max.y + timeline.track_top_y(clip.track_index) - timeline.scroll_y;
            let track_h = timeline.track_height(clip.track_index);
            let clip_x1 = canvas_rect.min.x + (clip.start_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
            let clip_width = (clip.duration_frames as f32 * timeline.zoom_factor).max(4.0);
            let clip_rect = Rect::from_min_size(
                pos2(clip_x1, track_y + 2.0),
                vec2(clip_width, track_h - 4.0),
            );

            // Полное 2D Frustum Culling клипа: горизонтальное и вертикальное отсечение
            if clip_rect.max.x < canvas_rect.min.x
                || clip_rect.min.x > canvas_rect.max.x
                || clip_rect.max.y < tracks_canvas_rect.min.y
                || clip_rect.min.y > tracks_canvas_rect.max.y
            {
                continue;
            }

            // Связанная подсветка клипов при выделении (O(1) проверка)
            let is_selected = timeline.selected_clip_id == Some(clip.id)
                || (selected_link_group.is_some() && clip.link_group == selected_link_group);

            // Отрисовка с tracks_painter (изоляция от левой панели треков)
            Self::draw_clip(&tracks_painter, clip_rect, canvas_rect, clip, is_selected, timeline, waveform_mgr, ui.ctx());

            // Интерактивность клипа: строго ограничена видимой областью холста
            if timeline.active_tool == Tool::Selection || timeline.active_tool == Tool::Trim {
                let interact_rect = clip_rect.intersect(tracks_canvas_rect);
                if interact_rect.is_positive() {
                    // Ручки Trim привязываются СТРОГО к истинным краям клипа (clip_rect.min.x / clip_rect.max.x),
                    // а НЕ к обрезанному окном interact_rect! Иначе при выходе правого края клипа за правую границу
                    // экрана попытка передвинуть клип возле правого края окна случайно обрезает конец клипа!
                    let allow_trim_handles = timeline.active_tool == Tool::Trim || clip_rect.width() >= 36.0;
                    let trim_handle_w = if allow_trim_handles {
                        8.0_f32.min(clip_rect.width() * 0.22)
                    } else {
                        0.0
                    };
                    let left_handle = Rect::from_min_size(
                        pos2(clip_rect.min.x, interact_rect.min.y),
                        vec2(trim_handle_w, interact_rect.height()),
                    )
                    .intersect(tracks_canvas_rect);
                    let right_handle = Rect::from_min_size(
                        pos2(clip_rect.max.x - trim_handle_w, interact_rect.min.y),
                        vec2(trim_handle_w, interact_rect.height()),
                    )
                    .intersect(tracks_canvas_rect);

                    // Retime растягивающий хэндл в правом верхнем углу клипа (когда открыты Retime Controls)
                    let retime_handle_w = 14.0_f32.min(clip_rect.width() * 0.3);
                    let retime_handle = if clip.retime_controls_open {
                        Rect::from_min_max(
                            pos2(clip_rect.max.x - retime_handle_w, clip_rect.min.y),
                            pos2(clip_rect.max.x, (clip_rect.min.y + 16.0).min(clip_rect.max.y)),
                        )
                        .intersect(tracks_canvas_rect)
                    } else {
                        Rect::NOTHING
                    };

                    let is_hovered = mouse_pos.map(|m| interact_rect.contains(m)).unwrap_or(false);
                    if is_hovered {
                        hovered_clip_id = Some(clip.id);
                        if let Some(m_pos) = mouse_pos {
                            if retime_handle.is_positive() && retime_handle.contains(m_pos) {
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeHorizontal);
                            } else if left_handle.is_positive() && left_handle.contains(m_pos) {
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeEast);
                            } else if right_handle.is_positive() && right_handle.contains(m_pos) {
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeWest);
                            }
                        }
                    }

                    let clip_resp = ui.interact(
                        interact_rect,
                        ui.id().with(("clip", clip.id)),
                        egui::Sense::click_and_drag(),
                    );

                    if clip_resp.clicked() {
                        clicked_clip_id = Some(clip.id);
                    }

                    // Начало действия: Retime Stretch, Trimming или Moving клипа
                    if clip_resp.drag_started()
                        && timeline.moving_clip.is_none()
                        && timeline.trim_session.is_none()
                        && timeline.retime_session.is_none()
                    {
                        if let Some(m_pos) = mouse_pos {
                            if retime_handle.is_positive() && retime_handle.contains(m_pos) {
                                start_retime_session = Some((clip.id, m_pos.x, clip.duration_frames, clip.speed));
                            } else if left_handle.is_positive() && left_handle.contains(m_pos) {
                                start_trim_request = Some(crate::timeline::TrimSession {
                                    clip_id: clip.id,
                                    edge: crate::timeline::TrimEdge::Start,
                                    mouse_start_x: m_pos.x,
                                    init_start_frame: clip.start_frame,
                                    init_duration_frames: clip.duration_frames,
                                    init_source_in_frame: clip.source_in_frame,
                                    init_source_in_secs: clip.source_in_secs,
                                });
                            } else if right_handle.is_positive() && right_handle.contains(m_pos) {
                                start_trim_request = Some(crate::timeline::TrimSession {
                                    clip_id: clip.id,
                                    edge: crate::timeline::TrimEdge::End,
                                    mouse_start_x: m_pos.x,
                                    init_start_frame: clip.start_frame,
                                    init_duration_frames: clip.duration_frames,
                                    init_source_in_frame: clip.source_in_frame,
                                    init_source_in_secs: clip.source_in_secs,
                                });
                            } else {
                                start_drag_request = Some((clip.id, m_pos.x, m_pos.y, clip.start_frame, clip.track_index));
                            }
                        }
                    }
                }
            }
        }

        if let Some(retime) = start_retime_session {
            timeline.push_history();
            timeline.retime_session = Some(retime);
        }
        if let Some(trim) = start_trim_request {
            timeline.push_history();
            timeline.trim_session = Some(trim);
        }
        if let Some(req) = start_drag_request {
            timeline.push_history();
            timeline.moving_clip = Some(req);
        }

        // Обновление сессии растяжения Retime Control
        if let Some((retime_id, start_mouse_x, init_dur, init_speed)) = timeline.retime_session {
            if let Some(m_pos) = mouse_pos {
                let dx = m_pos.x - start_mouse_x;
                let delta_frames = (dx / timeline.zoom_factor).round() as i64;
                let new_dur = (init_dur + delta_frames).max(1);
                let new_speed = (init_dur as f64 * init_speed) / (new_dur as f64);
                let clamped_speed = if new_speed.is_sign_negative() {
                    new_speed.clamp(-20.0, -0.05)
                } else {
                    new_speed.clamp(0.05, 20.0)
                };

                let link_grp = timeline.clips.iter().find(|c| c.id == retime_id).and_then(|c| c.link_group);
                if timeline.linked_selection && link_grp.is_some() {
                    for c in &mut timeline.clips {
                        if c.link_group == link_grp {
                            c.duration_frames = new_dur;
                            c.speed = clamped_speed;
                        }
                    }
                } else if let Some(c) = timeline.clips.iter_mut().find(|c| c.id == retime_id) {
                    c.duration_frames = new_dur;
                    c.speed = clamped_speed;
                }
                timeline.audio_dirty = true;
                timeline.structure_dirty = true;
            }
            if ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down()) {
                timeline.retime_session = None;
            }
        }

        // Обновление сессии подрезки краев клипа (DaVinci Resolve Edge Trimming)
        if let Some(trim) = timeline.trim_session {
            if let Some(m_pos) = mouse_pos {
                let dx = m_pos.x - trim.mouse_start_x;
                let delta_frames = (dx / timeline.zoom_factor).round() as i64;
                match trim.edge {
                    crate::timeline::TrimEdge::Start => {
                        let target_start = trim.init_start_frame + delta_frames;
                        timeline.trim_clip_start_session(&trim, target_start);
                    }
                    crate::timeline::TrimEdge::End => {
                        let target_end = trim.init_start_frame + trim.init_duration_frames + delta_frames;
                        timeline.trim_clip_end_session(&trim, target_end);
                    }
                }
            }
            if ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down()) {
                timeline.trim_session = None;
            }
        }

        // Обновление интерактивного изменения высоты дорожки перетаскиванием границы
        if let Some((track_idx, start_mouse_y, init_h)) = timeline.resizing_track {
            if let Some(m_pos) = mouse_pos {
                let dy = m_pos.y - start_mouse_y;
                let new_h = (init_h + dy).clamp(24.0, 200.0);
                if let Some(tr) = timeline.tracks.get_mut(track_idx) {
                    tr.height = new_h;
                }
            }
            if ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down()) {
                timeline.resizing_track = None;
            }
        }

        // Обновление позиции перетаскиваемого клипа (по кадрам и между дорожками)
        if let Some((drag_id, start_mouse_x, start_mouse_y, init_start_frame, init_track)) = timeline.moving_clip {
            if let Some(m_pos) = mouse_pos {
                let dx = m_pos.x - start_mouse_x;
                let dy = m_pos.y - start_mouse_y;
                let delta_frames = (dx / timeline.zoom_factor).round() as i64;
                let delta_tracks = (dy / timeline.track_height).round() as isize;

                let target_start = (init_start_frame + delta_frames).max(0);
                let target_track = (init_track as isize + delta_tracks).clamp(0, timeline.tracks.len().saturating_sub(1) as isize) as usize;

                timeline.move_clip_to_track(drag_id, target_start, Some(target_track));
            }
            if ui.input(|i| i.pointer.any_released() || !i.pointer.primary_down()) {
                timeline.moving_clip = None;
            }
        }

        // РЕЖИМ 1: Инструмент Blade (Лезвие / Razor) — нарезка клипов БЕЗ перемещения таймлайна
        if timeline.active_tool == Tool::Blade {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Crosshair);

            if let Some(m_pos) = mouse_pos {
                if tracks_canvas_rect.contains(m_pos) {
                    let mut cut_x = m_pos.x;
                    let rel_x = m_pos.x - canvas_rect.min.x + timeline.scroll_x;
                    let mut cut_frame = (rel_x / timeline.zoom_factor).round() as i64;

                    // Магнитное прилипание лезвия к плейхеду и границам клипов
                    if timeline.snapping_enabled {
                        let snap_thresh = (timeline.snapping_threshold_px / timeline.zoom_factor).max(2.0) as i64;
                        if (cut_frame - timeline.playhead_frame).abs() <= snap_thresh {
                            cut_frame = timeline.playhead_frame;
                            cut_x = canvas_rect.min.x + (cut_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
                        } else {
                            for clip in &timeline.clips {
                                if (cut_frame - clip.start_frame).abs() <= snap_thresh {
                                    cut_frame = clip.start_frame;
                                    cut_x = canvas_rect.min.x + (cut_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
                                    break;
                                }
                                if (cut_frame - clip.end_frame()).abs() <= snap_thresh {
                                    cut_frame = clip.end_frame();
                                    cut_x = canvas_rect.min.x + (cut_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
                                    break;
                                }
                            }
                        }
                    }

                    // Красная вертикальная линия лезвия через всю высоту дорожек
                    tracks_painter.line_segment(
                        [pos2(cut_x, tracks_canvas_rect.min.y), pos2(cut_x, tracks_canvas_rect.max.y)],
                        Stroke::new(2.0, Color32::from_rgb(0xff, 0x3b, 0x30)),
                    );

                    // Регистрация клика лезвия по дорожкам: режем клипы, НЕ трогая плейхед и таймлайн!
                    let blade_sense = ui.interact(tracks_canvas_rect, ui.id().with("blade_canvas_interact"), egui::Sense::click());
                    if blade_sense.clicked() || (tracks_canvas_rect.contains(m_pos) && ui.input(|i| i.pointer.primary_clicked())) {
                        let target_ids: Vec<u64> = timeline
                            .clips
                            .iter()
                            .filter(|c| cut_frame > c.start_frame && cut_frame < c.end_frame())
                            .map(|c| c.id)
                            .collect();

                        for id in target_ids {
                            timeline.blade_cut_at(id, cut_frame);
                        }
                    }
                }
            }
        }

        // В области дорожек клик по пустому месту ТОЛЬКО снимает выделение с клипа
        // (Управление курсором плейхеда происходит ИСКЛЮЧИТЕЛЬНО на линейке с цифрами как в DaVinci Resolve)
        if (timeline.active_tool == Tool::Selection || timeline.active_tool == Tool::Trim) && timeline.moving_clip.is_none() && hovered_clip_id.is_none() && timeline.dragged_media.is_none() {
            let canvas_sense = ui.interact(tracks_canvas_rect, ui.id().with("empty_tracks_canvas"), egui::Sense::click());
            if canvas_sense.clicked() {
                timeline.selected_clip_id = None;
            }
        }

        // Индикация курсора мыши для перемещения клипов
        if timeline.moving_clip.is_some() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grabbing);
        } else if hovered_clip_id.is_some() && (timeline.active_tool == Tool::Selection || timeline.active_tool == Tool::Trim) {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Grab);
        }

        // Применение действий с клипами
        if let Some(id) = clicked_clip_id {
            timeline.selected_clip_id = Some(id);
        }

        // 6. ОБРАБОТКА И ОТРИСОВКА ПЕРЕТАСКИВАНИЯ НА ДОРОЖКУ (DRAG & DROP ONTO TRACK)
        let mut drop_completed = false;
        if let Some(media) = &timeline.dragged_media.clone() {
            if let Some(m_pos) = mouse_pos {
                if canvas_rect.contains(m_pos) {
                    let rel_y = m_pos.y - ruler_rect.max.y + timeline.scroll_y;
                    let target_track_idx = timeline.track_at_y(rel_y);
                    let target_track_h = timeline.track_height(target_track_idx);
                    let rel_x = m_pos.x - canvas_rect.min.x + timeline.scroll_x;
                    let raw_frame = (rel_x / timeline.zoom_factor).max(0.0) as i64;

                    // Прилипание при сбросе
                    let mut drop_frame = raw_frame;
                    if timeline.snapping_enabled {
                        let snap_thresh = (timeline.snapping_threshold_px / timeline.zoom_factor).max(2.0) as i64;
                        if (raw_frame - timeline.playhead_frame).abs() <= snap_thresh {
                            drop_frame = timeline.playhead_frame;
                        }
                    }

                    let ghost_y = ruler_rect.max.y + timeline.track_top_y(target_track_idx) - timeline.scroll_y;
                    let ghost_x = canvas_rect.min.x + (drop_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
                    let ghost_w = (media.duration_frames as f32 * timeline.zoom_factor).max(12.0);
                    let ghost_rect = Rect::from_min_size(pos2(ghost_x, ghost_y + 2.0), vec2(ghost_w, target_track_h - 4.0));

                    // Полупрозрачный оранжевый фантом на дорожке под курсором
                    tracks_painter.rect_filled(
                        ghost_rect,
                        CornerRadius::same(2),
                        Color32::from_rgba_unmultiplied(232, 124, 36, 120),
                    );
                    tracks_painter.rect_stroke(
                        ghost_rect,
                        CornerRadius::same(2),
                        Stroke::new(2.0, ResolveColors::ACCENT_ORANGE),
                        StrokeKind::Inside,
                    );
                    tracks_painter.text(
                        pos2(ghost_rect.min.x + 6.0, ghost_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("➕ Drop: {}", media.name),
                        egui::FontId::proportional(11.0),
                        Color32::WHITE,
                    );

                    // Если кнопку мыши отпустили — размещаем клип на выбранную дорожку!
                    if ui.input(|i| i.pointer.any_released()) {
                        timeline.push_history();
                        if timeline.clips.is_empty() && media.source_fps > 0.0 {
                            timeline.fps = media.source_fps.round().clamp(1.0, 240.0) as u32;
                        }

                        let v_dur_from_secs = (media.video_duration_secs * timeline.fps as f64).round().max(1.0) as i64;
                        let a_dur_from_secs = (media.audio_duration_secs * timeline.fps as f64).round().max(1.0) as i64;
                        let v_actual_dur = if v_dur_from_secs > 0 { v_dur_from_secs } else { media.duration_frames };
                        let a_actual_dur = if a_dur_from_secs > 0 { a_dur_from_secs } else { media.duration_frames };

                        let num_audio = media.audio_tracks_count.max(if media.has_audio { 1 } else { 0 });
                        let link_id = if media.has_video && num_audio > 0 {
                            let id = timeline.next_link_id;
                            timeline.next_link_id += 1;
                            Some(id)
                        } else {
                            media.link_group
                        };

                        if media.has_video && num_audio > 0 {
                            // Создаем связанную группу: Видео на выбранной видеодорожке + Все дорожки аудио на A1, A2...
                            let video_track = if target_track_idx < 2 { target_track_idx } else { 1 };
                            let v_clip = Clip {
                                id: timeline.next_clip_id,
                                name: media.name.clone(),
                                track_index: video_track,
                                start_frame: drop_frame,
                                duration_frames: v_actual_dur,
                                is_audio: false,
                                color: ResolveColors::VIDEO_CLIP,
                                source_path: media.path.clone(),
                                source_in_frame: 0,
                                source_in_secs: 0.0,
                                source_fps: media.source_fps,
                                link_group: link_id,
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
                            timeline.next_clip_id += 1;
                            timeline.selected_clip_id = Some(v_clip.id);
                            timeline.clips.push(v_clip);

                            for a_idx in 0..num_audio {
                                let track_idx = 2 + a_idx;
                                timeline.ensure_audio_track(track_idx);

                                let track_name = if num_audio > 1 {
                                    format!("{} [A{}]", media.name, a_idx + 1)
                                } else {
                                    media.name.clone()
                                };

                                let a_clip = Clip {
                                    id: timeline.next_clip_id,
                                    name: track_name,
                                    track_index: track_idx,
                                    start_frame: drop_frame,
                                    duration_frames: a_actual_dur,
                                    is_audio: true,
                                    color: ResolveColors::AUDIO_CLIP,
                                    source_path: media.path.clone(),
                                    source_in_frame: 0,
                                    source_in_secs: 0.0,
                                    source_fps: media.source_fps,
                                    link_group: link_id,
                                    zoom_x: 1.0,
                                    zoom_y: 1.0,
                                    position_x: 0.0,
                                    position_y: 0.0,
                                    rotation: 0.0,
                                    opacity: 100.0,
                                    volume_db: 0.0,
                                    pan: 0.0,
                                    source_audio_stream: a_idx,
                                    ..Default::default()
                                };
                                timeline.next_clip_id += 1;
                                timeline.clips.push(a_clip);
                            }
                        } else {
                            let new_clip = Clip {
                                id: timeline.next_clip_id,
                                name: media.name.clone(),
                                track_index: target_track_idx,
                                start_frame: drop_frame,
                                duration_frames: if media.is_audio { a_actual_dur } else { v_actual_dur },
                                is_audio: media.is_audio,
                                color: if media.is_audio {
                                    ResolveColors::AUDIO_CLIP
                                } else {
                                    ResolveColors::VIDEO_CLIP
                                },
                                source_path: media.path.clone(),
                                source_in_frame: 0,
                                source_in_secs: 0.0,
                                source_fps: media.source_fps,
                                link_group: link_id,
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
                            timeline.next_clip_id += 1;
                            timeline.selected_clip_id = Some(new_clip.id);
                            timeline.clips.push(new_clip);
                        }
                        timeline.audio_dirty = true;
                        drop_completed = true;
                    }
                }
            }
        }
        if drop_completed || (timeline.dragged_media.is_some() && ui.input(|i| i.pointer.any_released())) {
            timeline.dragged_media = None;
        }

        // 7. Отрисовка фирменного оранжевого Playhead Resolve (строго в пределах холста)
        Self::draw_playhead(&canvas_painter, canvas_rect, ruler_rect, timeline);

        // 8. Отрисовка интерактивных заголовков треков в фирменном стиле DaVinci Resolve 21
        let corner_header_rect = Rect::from_min_size(full_rect.min, vec2(header_width, ruler_height));
        headers_painter.rect_filled(corner_header_rect, CornerRadius::ZERO, Color32::from_rgb(0x19, 0x1b, 0x1e));
        headers_painter.line_segment(
            [pos2(corner_header_rect.min.x, corner_header_rect.max.y), pos2(corner_header_rect.max.x, corner_header_rect.max.y)],
            Stroke::new(1.0, ResolveColors::BORDER),
        );
        headers_painter.text(
            pos2(corner_header_rect.min.x + 8.0, corner_header_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "TRACKS",
            egui::FontId::monospace(10.0),
            ResolveColors::TEXT_MUTED,
        );

        // Кнопки добавления дорожек +V и +A
        let btn_v_rect = Rect::from_min_size(pos2(corner_header_rect.max.x - 48.0, corner_header_rect.center().y - 8.0), vec2(20.0, 16.0));
        let btn_a_rect = Rect::from_min_size(pos2(corner_header_rect.max.x - 24.0, corner_header_rect.center().y - 8.0), vec2(20.0, 16.0));

        let v_resp = ui.interact(btn_v_rect, ui.id().with("add_video_track_btn"), egui::Sense::click());
        if v_resp.clicked() {
            timeline.add_video_track();
        }
        let v_bg = if v_resp.hovered() { ResolveColors::VIDEO_CLIP } else { Color32::from_rgb(0x23, 0x26, 0x2b) };
        headers_painter.rect_filled(btn_v_rect, CornerRadius::same(2), v_bg);
        headers_painter.text(btn_v_rect.center(), egui::Align2::CENTER_CENTER, "+V", egui::FontId::monospace(9.0), Color32::WHITE);

        let a_resp = ui.interact(btn_a_rect, ui.id().with("add_audio_track_btn"), egui::Sense::click());
        if a_resp.clicked() {
            timeline.add_audio_track();
        }
        let a_bg = if a_resp.hovered() { ResolveColors::AUDIO_CLIP } else { Color32::from_rgb(0x23, 0x26, 0x2b) };
        headers_painter.rect_filled(btn_a_rect, CornerRadius::same(2), a_bg);
        headers_painter.text(btn_a_rect.center(), egui::Align2::CENTER_CENTER, "+A", egui::FontId::monospace(9.0), Color32::WHITE);

        // Область заголовков дорожек (скроллируется вертикально синхронно с холстом)
        let headers_tracks_rect = Rect::from_min_max(
            pos2(full_rect.min.x, ruler_rect.max.y),
            pos2(full_rect.min.x + header_width, full_rect.max.y),
        );
        let headers_tracks_painter = ui.painter().with_clip_rect(headers_tracks_rect);

        let mut audio_dirty_requested = false;
        let mut resize_track_requested = None;
        let mut accum_y = 0.0;
        let scroll_y = timeline.scroll_y;
        for (i, track) in timeline.tracks.iter_mut().enumerate() {
            let y_top = ruler_rect.max.y + accum_y - scroll_y;
            let track_h = track.height;
            accum_y += track_h;
            let header_row_rect = Rect::from_min_size(pos2(full_rect.min.x, y_top), vec2(header_width, track_h));
            let (audio_changed, resize_drag) = Self::render_interactive_track_header(ui, &headers_tracks_painter, header_row_rect, track, i);
            if audio_changed {
                audio_dirty_requested = true;
            }
            if let Some(req) = resize_drag {
                resize_track_requested = Some(req);
            }
        }
        if let Some(req) = resize_track_requested {
            timeline.resizing_track = Some(req);
        }
        if audio_dirty_requested {
            timeline.audio_dirty = true;
        }

        // Вертикальная разделительная линия между панелью заголовков и холстом
        painter.line_segment(
            [pos2(headers_rect.max.x, full_rect.min.y), pos2(headers_rect.max.x, full_rect.max.y)],
            Stroke::new(1.5, ResolveColors::BORDER_LIGHT),
        );
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

            let trim_active = timeline.active_tool == Tool::Trim;
            let trim_btn = egui::Button::new(
                egui::RichText::new("][ Trim (T)").size(11.0).color(if trim_active { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
            ).fill(if trim_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(trim_btn).on_hover_text("Инструмент подрезки краев клипов [T]").clicked() {
                timeline.active_tool = Tool::Trim;
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

            // Zoom контролы с шагом 8% и иконками Resolve
            ui.label(egui::RichText::new("🔍").size(12.0).color(ResolveColors::TEXT_MUTED));
            let step = (timeline.zoom_step_percent / 100.0).clamp(0.01, 0.5);
            if ui.button("➖").on_hover_text(format!("Уменьшить масштаб (-{:.0}%)", timeline.zoom_step_percent)).clicked() {
                timeline.zoom_factor = (timeline.zoom_factor * (1.0 - step)).clamp(0.0005, 50.0);
            }
            let mut log_zoom = timeline.zoom_factor;
            let slider = egui::Slider::new(&mut log_zoom, 0.0005..=50.0)
                .logarithmic(true)
                .show_value(false);
            if ui.add_sized([110.0, 16.0], slider).on_hover_text("Плавный логарифмический зум (от 1 кадра до 10 часов)").changed() {
                let center_x = 500.0;
                let target_frame = (center_x + timeline.scroll_x) / timeline.zoom_factor;
                timeline.zoom_factor = log_zoom;
                timeline.scroll_x = (target_frame * log_zoom - center_x).max(0.0);
            }
            if ui.button("➕").on_hover_text(format!("Увеличить масштаб (+{:.0}%)", timeline.zoom_step_percent)).clicked() {
                timeline.zoom_factor = (timeline.zoom_factor * (1.0 + step)).clamp(0.0005, 50.0);
            }
            if ui.button("⛶").on_hover_text("Показать весь проект целиком [Shift+Z]").clicked() {
                timeline.zoom_to_fit(ui.available_width().max(400.0));
            }

            ui.separator();

            // ☷ View Options
            let view_opts_active = timeline.show_view_options;
            let view_opts_btn = egui::Button::new(
                egui::RichText::new("☷").size(12.0).color(if view_opts_active { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
            ).fill(if view_opts_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(view_opts_btn).on_hover_text("Настройки отображения таймлайна и высоты дорожек").clicked() {
                timeline.show_view_options = !timeline.show_view_options;
            }

            // ⚡ Retime Controls кнопка
            let retime_active = timeline.selected_clip_id.and_then(|id| timeline.clips.iter().find(|c| c.id == id)).map(|c| c.retime_controls_open).unwrap_or(false);
            let retime_btn = egui::Button::new(
                egui::RichText::new("⚡ Retime").size(11.0).color(if retime_active { Color32::WHITE } else { ResolveColors::TEXT_PRIMARY }),
            ).fill(if retime_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
            if ui.add(retime_btn).on_hover_text("Включить/выключить Retime Controls для выбранного клипа [Ctrl+R]").clicked() {
                if let Some(sel_id) = timeline.selected_clip_id {
                    let link_grp = timeline.clips.iter().find(|c| c.id == sel_id).and_then(|c| c.link_group);
                    if timeline.linked_selection && link_grp.is_some() {
                        let new_state = !retime_active;
                        for c in &mut timeline.clips {
                            if c.link_group == link_grp {
                                c.retime_controls_open = new_state;
                            }
                        }
                    } else if let Some(c) = timeline.clips.iter_mut().find(|c| c.id == sel_id) {
                        c.retime_controls_open = !c.retime_controls_open;
                    }
                }
            }

            // Быстрые пресеты скорости для выбранного клипа
            let sel_clip_info = timeline.selected_clip_id.and_then(|id| {
                timeline.clips.iter().find(|c| c.id == id).map(|c| (id, c.speed, c.link_group))
            });
            if let Some((sel_id, cur_speed, link_grp)) = sel_clip_info {
                ui.separator();
                ui.label(egui::RichText::new(format!("⚡ {:.0}%", cur_speed * 100.0)).size(11.0).color(ResolveColors::ACCENT_ORANGE));

                let speeds = [
                    ("50%", 0.5),
                    ("100%", 1.0),
                    ("200%", 2.0),
                    ("◀ Rev", -1.0),
                ];
                for (name, sp) in speeds {
                    if ui.button(egui::RichText::new(name).size(10.0)).clicked() {
                        let target_clips: Vec<u64> = if timeline.linked_selection && link_grp.is_some() {
                            timeline.clips.iter().filter(|c| c.link_group == link_grp).map(|c| c.id).collect()
                        } else {
                            vec![sel_id]
                        };
                        for id in target_clips {
                            if let Some(c) = timeline.clips.iter_mut().find(|c| c.id == id) {
                                let orig_dur = (c.duration_frames as f64 * c.speed.abs()).round() as i64;
                                c.speed = sp;
                                c.duration_frames = (orig_dur as f64 / sp.abs()).round().max(1.0) as i64;
                            }
                        }
                        timeline.audio_dirty = true;
                        timeline.structure_dirty = true;
                    }
                }
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

        // Всплывающее меню настроек отображения View Options
        if timeline.show_view_options {
            egui::Area::new(egui::Id::new("timeline_view_options_popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(ui.min_rect().min.x + 350.0, ui.min_rect().max.y + 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::window(ui.style())
                        .fill(Color32::from_rgb(0x1a, 0x1d, 0x24))
                        .stroke(Stroke::new(1.0, ResolveColors::BORDER_LIGHT))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(280.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("☷ Timeline View Options").strong().color(Color32::WHITE));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").clicked() {
                                        timeline.show_view_options = false;
                                    }
                                });
                            });
                            ui.separator();

                            ui.label(egui::RichText::new("Высота дорожек (Track Height):").size(11.0).color(ResolveColors::TEXT_MUTED));
                            let mut h = timeline.track_height;
                            if ui.add(egui::Slider::new(&mut h, 24.0..=160.0).suffix(" px")).changed() {
                                timeline.track_height = h;
                                for t in &mut timeline.tracks {
                                    t.height = h;
                                }
                            }

                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("Пресеты высоты дорожек:").size(10.0).color(ResolveColors::TEXT_MUTED));
                            ui.horizontal_wrapped(|ui| {
                                let presets = [
                                    ("Compact (32px)", 32.0),
                                    ("Medium (52px)", 52.0),
                                    ("Large (84px)", 84.0),
                                    ("Expanded (120px)", 120.0),
                                ];
                                for (p_name, p_h) in presets {
                                    if ui.button(p_name).clicked() {
                                        timeline.track_height = p_h;
                                        for t in &mut timeline.tracks {
                                            t.height = p_h;
                                        }
                                    }
                                }
                            });
                        });
                });
        }
    }

    fn draw_ruler(
        painter: &egui::Painter,
        rect: Rect,
        timeline: &TimelineState,
        render_cache: &crate::render_cache::RenderCacheManager,
    ) {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(0x1a, 0x1b, 0x1e));
        painter.line_segment(
            [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
            Stroke::new(1.0, ResolveColors::BORDER),
        );

        let frames_per_sec = timeline.fps as f32;
        let sec_width = frames_per_sec * timeline.zoom_factor;

        // Адаптивный шаг меток: расстояние между таймкодами всегда >= 80px (от секунд до многочасовых проектов)
        let min_spacing = 80.0;
        let steps = [
            1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 14400, 28800, 86400,
        ];
        let step_seconds = steps
            .into_iter()
            .find(|&s| s as f32 * sec_width >= min_spacing)
            .unwrap_or(86400);

        let start_sec = ((timeline.scroll_x / sec_width).floor() as i64 / step_seconds).max(0) * step_seconds;
        let end_sec = (((timeline.scroll_x + rect.width()) / sec_width).ceil() as i64 / step_seconds + 1) * step_seconds;

        let mut s = start_sec;
        while s <= end_sec {
            let raw_x = rect.min.x + (s as f32 * sec_width) - timeline.scroll_x;
            let x = raw_x.round(); // Целочисленная привязка к пикселям (Pixel-Perfect) устраняет размытие

            if x >= rect.min.x && x <= rect.max.x {
                // Большая засечка секунды
                painter.line_segment(
                    [pos2(x, rect.max.y - 10.0), pos2(x, rect.max.y)],
                    Stroke::new(1.0, ResolveColors::BORDER_LIGHT),
                );

                // Четкий контрастный таймкод
                let tc = Timecode::from_seconds(s as f64, timeline.fps).to_smpte();
                painter.text(
                    pos2(x + 4.0, (rect.min.y + 4.0).round()),
                    egui::Align2::LEFT_TOP,
                    tc,
                    egui::FontId::monospace(10.0),
                    ResolveColors::TEXT_PRIMARY,
                );
            }

            // Промежуточные мелкие засечки
            let sub_count = if step_seconds == 1 { 4 } else { 5 };
            let sub_step_px = (step_seconds as f32 * sec_width) / sub_count as f32;
            if sub_step_px >= 8.0 {
                for sub in 1..sub_count {
                    let sub_x = (raw_x + sub as f32 * sub_step_px).round();
                    if sub_x >= rect.min.x && sub_x <= rect.max.x {
                        painter.line_segment(
                            [pos2(sub_x, rect.max.y - 5.0), pos2(sub_x, rect.max.y)],
                            Stroke::new(1.0, ResolveColors::BORDER),
                        );
                    }
                }
            }

            s += step_seconds;
        }

        // Индикатор Render Cache в стиле DaVinci Resolve (полоса синий/красный внизу линейки)
        if render_cache.mode != crate::render_cache::RenderCacheMode::None {
            let bar_y_min = rect.max.y - 3.0;
            let bar_y_max = rect.max.y;
            let blue_cache = Color32::from_rgb(0x29, 0x97, 0xff);
            let red_dirty = Color32::from_rgb(0xe0, 0x31, 0x31);

            let visible_start_frame = ((timeline.scroll_x / timeline.zoom_factor).floor() as i64).max(0);
            let visible_end_frame = (((timeline.scroll_x + rect.width()) / timeline.zoom_factor).ceil() as i64).max(0);

            let intervals = render_cache.cached_intervals();
            let mut cache_mesh = egui::Mesh::default();

            // 1) Объединяем интервалы всех активных видеоклипов для фоновой красной сетки
            let mut video_spans: Vec<(i64, i64)> = Vec::new();
            for clip in &timeline.clips {
                if clip.is_audio || clip.is_disabled {
                    continue;
                }
                let c_start = clip.start_frame.max(visible_start_frame);
                let c_end = clip.end_frame().min(visible_end_frame);
                if c_start < c_end {
                    video_spans.push((c_start, c_end));
                }
            }
            video_spans.sort_unstable_by_key(|&(s, _)| s);
            let mut merged_spans: Vec<(i64, i64)> = Vec::new();
            for (s, e) in video_spans {
                if let Some(last) = merged_spans.last_mut() {
                    if s <= last.1 {
                        last.1 = last.1.max(e);
                        continue;
                    }
                }
                merged_spans.push((s, e));
            }

            // Фоновая красная полоса — рисуется один раз по объединенным видеосегментам
            for &(s, e) in &merged_spans {
                let clip_x1 = rect.min.x + (s as f32 * timeline.zoom_factor) - timeline.scroll_x;
                let clip_x2 = rect.min.x + (e as f32 * timeline.zoom_factor) - timeline.scroll_x;
                if clip_x2 > rect.min.x && clip_x1 < rect.max.x {
                    let r = Rect::from_min_max(
                        pos2(clip_x1.max(rect.min.x), bar_y_min),
                        pos2(clip_x2.min(rect.max.x), bar_y_max),
                    );
                    cache_mesh.add_colored_rect(r, red_dirty);
                }
            }

            // 2) Поверх наносим синие сегменты готового кэша (O(1) на интервал), никогда не перетираясь красным
            for &(i_start, i_end) in &intervals {
                for &(s, e) in &merged_spans {
                    let overlap_start = i_start.max(s);
                    let overlap_end = (i_end + 1).min(e);
                    if overlap_start < overlap_end {
                        let x1 = rect.min.x + (overlap_start as f32 * timeline.zoom_factor) - timeline.scroll_x;
                        let x2 = rect.min.x + (overlap_end as f32 * timeline.zoom_factor) - timeline.scroll_x;
                        let draw_x2 = (x1 + 3.0).max(x2);
                        if draw_x2 > rect.min.x && x1 < rect.max.x {
                            let r = Rect::from_min_max(
                                pos2(x1.max(rect.min.x), bar_y_min),
                                pos2(draw_x2.min(rect.max.x), bar_y_max),
                            );
                            cache_mesh.add_colored_rect(r, blue_cache);
                        }
                    }
                }
            }
            if !cache_mesh.is_empty() {
                painter.add(egui::Shape::mesh(cache_mesh));
            }
        }
    }

    fn render_interactive_track_header(
        ui: &mut Ui,
        painter: &egui::Painter,
        rect: Rect,
        track: &mut crate::timeline::Track,
        index: usize,
    ) -> (bool, Option<(usize, f32, f32)>) {
        let mut audio_changed = false;
        let mut resize_drag = None;

        // Фон строки заголовка
        let bg_color = Color32::from_rgb(0x1a, 0x1b, 0x1e);
        painter.rect_filled(rect, CornerRadius::ZERO, bg_color);
        painter.line_segment(
            [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
            Stroke::new(1.0, ResolveColors::BORDER),
        );

        // Интерактивная нижняя граница для изменения высоты дорожки (как в DaVinci Resolve)
        let border_handle_rect = Rect::from_min_max(
            pos2(rect.min.x, rect.max.y - 3.0),
            pos2(rect.max.x, rect.max.y + 3.0),
        );
        let border_resp = ui.interact(
            border_handle_rect,
            ui.id().with(("track_border_h", index)),
            egui::Sense::click_and_drag(),
        );
        if border_resp.hovered() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeVertical);
        }
        if border_resp.drag_started() {
            if let Some(m_pos) = ui.input(|i| i.pointer.hover_pos()) {
                resize_drag = Some((index, m_pos.y, track.height));
            }
        }

        // Индикатор типа дорожки (узкая полоска слева)
        let indicator_color = if track.is_video { ResolveColors::VIDEO_CLIP } else { ResolveColors::AUDIO_CLIP };
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius::ZERO, indicator_color);

        // Кнопка назначения дорожки DaVinci Destination Patching: [V1], [A1]
        let dest_rect = Rect::from_min_size(pos2(rect.min.x + 8.0, rect.center().y - 10.0), vec2(28.0, 20.0));
        let dest_stroke = if !track.is_muted {
            Stroke::new(1.0, ResolveColors::ACCENT_ORANGE)
        } else {
            Stroke::new(1.0, ResolveColors::BORDER_LIGHT)
        };
        painter.rect_filled(dest_rect, CornerRadius::same(2), Color32::from_rgb(0x14, 0x15, 0x17));
        painter.rect_stroke(dest_rect, CornerRadius::same(2), dest_stroke, StrokeKind::Inside);
        painter.text(
            dest_rect.center(),
            egui::Align2::CENTER_CENTER,
            &track.name,
            egui::FontId::monospace(10.0),
            if !track.is_muted { Color32::WHITE } else { ResolveColors::TEXT_MUTED },
        );

        // Название дорожки
        painter.text(
            pos2(rect.min.x + 42.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &track.name,
            egui::FontId::proportional(11.0),
            ResolveColors::TEXT_PRIMARY,
        );

        // Кнопки управления справа:
        if track.is_video {
            // Кнопка Lock 🔒
            let lock_rect = Rect::from_min_size(pos2(rect.max.x - 22.0, rect.center().y - 9.0), vec2(18.0, 18.0));
            let lock_resp = ui.interact(lock_rect, ui.id().with(("lock_v", index)), egui::Sense::click());
            if lock_resp.clicked() {
                track.is_locked = !track.is_locked;
            }
            let lock_color = if track.is_locked { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_MUTED };
            painter.text(lock_rect.center(), egui::Align2::CENTER_CENTER, if track.is_locked { "🔒" } else { "🔓" }, egui::FontId::proportional(10.0), lock_color);

            // Кнопка включения/выключения видеодорожки (Eye / Mute)
            let eye_rect = Rect::from_min_size(pos2(rect.max.x - 42.0, rect.center().y - 9.0), vec2(18.0, 18.0));
            let eye_resp = ui.interact(eye_rect, ui.id().with(("eye_v", index)), egui::Sense::click());
            if eye_resp.clicked() {
                track.is_muted = !track.is_muted;
            }
            let eye_color = if !track.is_muted { ResolveColors::TEXT_PRIMARY } else { Color32::from_rgb(100, 100, 100) };
            painter.text(eye_rect.center(), egui::Align2::CENTER_CENTER, if !track.is_muted { "👁" } else { "⊘" }, egui::FontId::proportional(10.0), eye_color);
        } else {
            // Аудиодорожка: Solo [S], Mute [M], Lock 🔒
            let lock_rect = Rect::from_min_size(pos2(rect.max.x - 18.0, rect.center().y - 8.0), vec2(15.0, 16.0));
            let lock_resp = ui.interact(lock_rect, ui.id().with(("lock_a", index)), egui::Sense::click());
            if lock_resp.clicked() {
                track.is_locked = !track.is_locked;
            }
            painter.text(lock_rect.center(), egui::Align2::CENTER_CENTER, if track.is_locked { "🔒" } else { "🔓" }, egui::FontId::proportional(9.0), if track.is_locked { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_MUTED });

            let mute_rect = Rect::from_min_size(pos2(rect.max.x - 36.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            let mute_resp = ui.interact(mute_rect, ui.id().with(("mute_a", index)), egui::Sense::click());
            if mute_resp.clicked() {
                track.is_muted = !track.is_muted;
                audio_changed = true;
            }
            let mute_bg = if track.is_muted { ResolveColors::VU_RED } else { Color32::from_rgb(0x23, 0x26, 0x2b) };
            painter.rect_filled(mute_rect, CornerRadius::same(2), mute_bg);
            painter.text(mute_rect.center(), egui::Align2::CENTER_CENTER, "M", egui::FontId::monospace(9.0), if track.is_muted { Color32::WHITE } else { ResolveColors::TEXT_MUTED });

            let solo_rect = Rect::from_min_size(pos2(rect.max.x - 54.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            let solo_resp = ui.interact(solo_rect, ui.id().with(("solo_a", index)), egui::Sense::click());
            if solo_resp.clicked() {
                track.is_solo = !track.is_solo;
                audio_changed = true;
            }
            let solo_bg = if track.is_solo { ResolveColors::VU_YELLOW } else { Color32::from_rgb(0x23, 0x26, 0x2b) };
            painter.rect_filled(solo_rect, CornerRadius::same(2), solo_bg);
            painter.text(solo_rect.center(), egui::Align2::CENTER_CENTER, "S", egui::FontId::monospace(9.0), if track.is_solo { Color32::BLACK } else { ResolveColors::TEXT_MUTED });
        }

        (audio_changed, resize_drag)
    }

    fn draw_clip(
        painter: &egui::Painter,
        rect: Rect,
        canvas_rect: Rect,
        clip: &Clip,
        is_selected: bool,
        timeline: &TimelineState,
        waveform_mgr: &crate::waveform::WaveformManager,
        ctx: &egui::Context,
    ) {
        let fill_color = if clip.is_disabled {
            Color32::from_rgb(0x18, 0x1b, 0x20)
        } else if is_selected {
            if clip.is_audio { ResolveColors::AUDIO_CLIP_SELECTED } else { ResolveColors::VIDEO_CLIP_SELECTED }
        } else {
            clip.color
        };

        let stroke = if is_selected {
            Stroke::new(1.5, ResolveColors::ACCENT_ORANGE)
        } else if clip.is_disabled {
            Stroke::new(1.0, Color32::from_rgb(0x2a, 0x2e, 0x38))
        } else {
            Stroke::new(1.0, Color32::from_rgb(0x18, 0x33, 0x4f))
        };

        painter.rect_filled(rect, CornerRadius::same(2), fill_color);
        painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Inside);

        // Визуальные шевроны скорости Retime Controls (› › › или ‹ ‹ ‹)
        if clip.retime_controls_open && rect.width() > 24.0 && rect.height() > 18.0 {
            let step = (32.0 / clip.speed.abs().clamp(0.2, 5.0) as f32).clamp(10.0, 80.0);
            let mut cx = rect.min.x + step;
            let arrow_y = rect.center().y + 3.0;
            let arrow_char = if clip.speed < 0.0 { "‹" } else { "›" };
            let arrow_color = Color32::from_rgba_unmultiplied(255, 255, 255, 50);
            while cx < rect.max.x - 8.0 {
                if cx >= canvas_rect.min.x && cx <= canvas_rect.max.x {
                    painter.text(
                        pos2(cx, arrow_y),
                        egui::Align2::CENTER_CENTER,
                        arrow_char,
                        egui::FontId::monospace(11.0),
                        arrow_color,
                    );
                }
                cx += step;
            }
        }

        // Визуальные скобки выделения и подрезки DaVinci Resolve [ ]
        if is_selected && rect.width() > 16.0 {
            let b_stroke = Stroke::new(2.5, ResolveColors::ACCENT_ORANGE);
            let b_len = 6.0;
            // Левая скобка [
            painter.line_segment([pos2(rect.min.x + 1.0, rect.min.y + 1.0), pos2(rect.min.x + 1.0, rect.max.y - 1.0)], b_stroke);
            painter.line_segment([pos2(rect.min.x + 1.0, rect.min.y + 1.0), pos2(rect.min.x + 1.0 + b_len, rect.min.y + 1.0)], b_stroke);
            painter.line_segment([pos2(rect.min.x + 1.0, rect.max.y - 1.0), pos2(rect.min.x + 1.0 + b_len, rect.max.y - 1.0)], b_stroke);

            // Правая скобка ]
            painter.line_segment([pos2(rect.max.x - 1.0, rect.min.y + 1.0), pos2(rect.max.x - 1.0, rect.max.y - 1.0)], b_stroke);
            painter.line_segment([pos2(rect.max.x - 1.0 - b_len, rect.min.y + 1.0), pos2(rect.max.x - 1.0, rect.min.y + 1.0)], b_stroke);
            painter.line_segment([pos2(rect.max.x - 1.0 - b_len, rect.max.y - 1.0), pos2(rect.max.x - 1.0, rect.max.y - 1.0)], b_stroke);
        }

        // Подсветка активной границы при подрезке (Trimming Session)
        if let Some(trim) = &timeline.trim_session {
            if trim.clip_id == clip.id {
                let active_x = match trim.edge {
                    crate::timeline::TrimEdge::Start => rect.min.x,
                    crate::timeline::TrimEdge::End => rect.max.x,
                };
                painter.line_segment(
                    [pos2(active_x, rect.min.y - 2.0), pos2(active_x, rect.max.y + 2.0)],
                    Stroke::new(3.0, Color32::WHITE),
                );
            }
        }

        // Retime Controls верхняя полоса с процентом скорости и ручкой растяжения
        if clip.retime_controls_open && rect.width() > 20.0 {
            let retime_h = 16.0_f32.min(rect.height() * 0.45);
            let retime_rect = Rect::from_min_size(rect.min, vec2(rect.width(), retime_h));
            painter.rect_filled(retime_rect, CornerRadius::same(2), Color32::from_rgb(0x16, 0x2c, 0x42));
            painter.rect_stroke(retime_rect, CornerRadius::same(2), Stroke::new(1.0, Color32::from_rgb(0x29, 0x58, 0x82)), StrokeKind::Inside);

            let speed_pct = (clip.speed * 100.0).abs();
            let speed_str = if clip.speed < 0.0 {
                format!("◀ {:.1}%", speed_pct)
            } else {
                format!("▶ {:.1}%", speed_pct)
            };
            let text_left = (rect.min.x + 4.0).max(canvas_rect.min.x + 4.0);
            if rect.max.x - text_left > 30.0 {
                painter.text(
                    pos2(text_left, retime_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    speed_str,
                    egui::FontId::monospace(9.5),
                    Color32::from_rgb(0x8a, 0xcc, 0xff),
                );
            }

            // Правая интерактивная ручка растяжения Retime
            let handle_w = 14.0_f32.min(rect.width() * 0.3);
            let handle_rect = Rect::from_min_max(pos2(rect.max.x - handle_w, rect.min.y), pos2(rect.max.x, rect.min.y + retime_h));
            painter.rect_filled(handle_rect, CornerRadius::same(2), Color32::from_rgb(0x2d, 0x6d, 0xa1));
            painter.text(
                handle_rect.center(),
                egui::Align2::CENTER_CENTER,
                "⬌",
                egui::FontId::monospace(9.0),
                Color32::WHITE,
            );
        } else {
            // Обычная верхняя фирменная полоска клипа Resolve (Darker Header Strip)
            let header_h = 13.0_f32.min(rect.height() * 0.35);
            let clip_header_rect = Rect::from_min_size(rect.min, vec2(rect.width(), header_h));
            painter.rect_filled(clip_header_rect, CornerRadius::same(2), Color32::from_rgba_unmultiplied(0, 0, 0, 40));
        }

        // Для аудиоклипов: реальная асинхронная расшифровка звуковой волны (Waveform)
        if clip.is_audio && rect.width() > 10.0 {
            let mid_y = rect.center().y + 4.0;
            let half_h = (rect.height() * 0.35).max(2.0);

            let visible_start_x = (rect.min.x + 2.0).max(canvas_rect.min.x);
            let visible_end_x = (rect.max.x - 2.0).min(canvas_rect.max.x);

            if visible_end_x > visible_start_x {
                if let Some(path) = &clip.source_path {
                    if let Some(peaks) = waveform_mgr.get_waveform_stream(path, clip.source_audio_stream) {
                        let pts_per_sec = 100.0;
                        let sec_per_px = 1.0 / (timeline.zoom_factor * timeline.fps as f32) as f64;
                        let step_px: f32 = 2.0;

                        // Привязываем сетку полос жестко к началу клипа rect.min.x,
                        // чтобы при горизонтальном скролле волна монолитно и плавно двигалась с клипом без дёргания
                        let first_k = if rect.min.x < canvas_rect.min.x {
                            (((canvas_rect.min.x - rect.min.x) / step_px).floor() as usize).max(0)
                        } else {
                            0
                        };

                        let mut x = rect.min.x + (first_k as f32 * step_px);
                        let mut k = first_k;

                        let num_steps = (((visible_end_x - x).max(0.0) / step_px).ceil() as usize + 2).max(1);
                        let mut mesh = egui::Mesh::default();
                        mesh.reserve_triangles(num_steps * 2);
                        mesh.reserve_vertices(num_steps * 4);
                        let wave_color = Color32::from_rgb(0x38, 0xd4, 0x8a);
                        let silence_color = Color32::from_rgba_unmultiplied(255, 255, 255, 45);

                        let peaks_slice = peaks.as_slice();
                        let peaks_len = peaks_slice.len();

                        // Коэффициент усиления клипа в линейной шкале
                        let gain = if clip.volume_db != 0.0 {
                            10.0f32.powf(clip.volume_db / 20.0)
                        } else {
                            1.0
                        };

                        while x < visible_end_x {
                            let t_start = clip.source_in_secs + (k as f64 * step_px as f64 * sec_per_px);
                            let t_end = t_start + (step_px as f64 * sec_per_px);
                            let s_start = t_start * pts_per_sec;
                            let s_end = t_end * pts_per_sec;

                            let idx_start = (s_start.max(0.0) as usize).min(peaks_len);
                            let idx_end = (s_end.ceil() as usize).min(peaks_len);

                            let base_amp = if idx_start >= peaks_len {
                                0.0
                            } else if idx_end <= idx_start + 1 {
                                // Zoom In (сильное приближение): плавная субпиксельная интерполяция
                                let frac = (s_start - idx_start as f64).clamp(0.0, 1.0) as f32;
                                let p0 = peaks_slice[idx_start];
                                let p1 = if idx_start + 1 < peaks_len { peaks_slice[idx_start + 1] } else { p0 };
                                p0 * (1.0 - frac) + p1 * frac
                            } else {
                                // Zoom Out (отдаление): Peak-Hold агрегация по пиксельному окну (устраняет 100% дёрганий и алиасинга)
                                let mut max_val = peaks_slice[idx_start];
                                let stride = ((idx_end - idx_start) / 32).max(1);
                                let mut i = idx_start + 1;
                                while i < idx_end {
                                    if peaks_slice[i] > max_val {
                                        max_val = peaks_slice[i];
                                    }
                                    i += stride;
                                }
                                max_val
                            };

                            let amp = (base_amp * gain).clamp(0.0, 1.0);

                            if amp > 0.005 {
                                let bar_h = (amp * half_h).max(1.0);
                                mesh.add_colored_rect(
                                    Rect::from_min_max(pos2(x - 0.75, mid_y - bar_h), pos2(x + 0.75, mid_y + bar_h)),
                                    wave_color,
                                );
                            } else {
                                mesh.add_colored_rect(
                                    Rect::from_min_max(pos2(x, mid_y - 0.5), pos2(x + step_px, mid_y + 0.5)),
                                    silence_color,
                                );
                            }
                            x += step_px;
                            k += 1;
                        }
                        if !mesh.is_empty() {
                            painter.add(egui::Shape::mesh(mesh));
                        }
                    } else {
                        waveform_mgr.request_waveform_stream(path, clip.source_audio_stream, ctx.clone());
                        painter.line_segment(
                            [pos2(visible_start_x, mid_y), pos2(visible_end_x, mid_y)],
                            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
                        );
                    }
                } else {
                    painter.line_segment(
                        [pos2(visible_start_x, mid_y), pos2(visible_end_x, mid_y)],
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 50)),
                    );
                }
            }
        }

        // Название клипа: прижимаем к левому видимому краю полосы без аллокаций
        let label_left = (rect.min.x + 6.0).max(canvas_rect.min.x + 6.0);
        if rect.max.x - label_left > 20.0 {
            let mut x_pos = label_left;
            let y_label = if clip.retime_controls_open {
                rect.min.y + 18.0
            } else {
                rect.min.y + 2.0
            };
            if clip.is_disabled {
                painter.text(
                    pos2(x_pos, y_label),
                    egui::Align2::LEFT_TOP,
                    "[D] ",
                    egui::FontId::monospace(10.0),
                    ResolveColors::TEXT_MUTED,
                );
                x_pos += 20.0;
            }
            if clip.link_group.is_some() {
                painter.text(
                    pos2(x_pos, y_label),
                    egui::Align2::LEFT_TOP,
                    "🔗 ",
                    egui::FontId::proportional(10.0),
                    Color32::WHITE,
                );
                x_pos += 12.0;
            }
            painter.text(
                pos2(x_pos, y_label),
                egui::Align2::LEFT_TOP,
                &clip.name,
                egui::FontId::proportional(10.0),
                if clip.is_disabled { ResolveColors::TEXT_MUTED } else { Color32::WHITE },
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
