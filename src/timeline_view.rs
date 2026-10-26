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
    ) {
        // 1. Верхний тулбар таймлайна
        Self::render_toolbar(ui, timeline);
        ui.separator();

        // 2. Основная область: Заголовки треков слева + Сетка таймлайна справа
        let header_width = 140.0;
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
                // Панорамирование зажатием колесика мыши (MMB Drag)
                let mmb_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle));
                if mmb_down {
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
                        // Зум центрированный строго под курсором мыши (как в DaVinci Resolve 21)
                        let old_zoom = timeline.zoom_factor;
                        let zoom_mult = if scroll_y > 0.0 { 1.15 } else { 0.87 };
                        let new_zoom = (old_zoom * zoom_mult).clamp(0.05, 50.0);

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

        // Ограничение вертикального скролла дорожек
        let total_tracks_h = total_tracks as f32 * track_height;
        let max_scroll_y = (total_tracks_h - (tracks_canvas_rect.height() - 20.0)).max(0.0);
        timeline.scroll_y = timeline.scroll_y.clamp(0.0, max_scroll_y);

        // 3. Отрисовка Ruler (Линейки времени) строго с изоляцией ruler_painter
        Self::draw_ruler(&ruler_painter, ruler_rect, timeline);

        // 4. Отрисовка полос дорожек таймлайна с учетом вертикального скролла scroll_y
        for (i, _track) in timeline.tracks.iter().enumerate() {
            let y_top = ruler_rect.max.y + (i as f32 * track_height) - timeline.scroll_y;
            let track_row_rect = Rect::from_min_size(pos2(canvas_rect.min.x, y_top), vec2(canvas_rect.width(), track_height));

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

        for clip in &timeline.clips {
            if clip.track_index >= timeline.tracks.len() {
                continue;
            }
            let track_y = ruler_rect.max.y + (clip.track_index as f32 * track_height) - timeline.scroll_y;
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

                    let is_hovered = mouse_pos.map(|m| interact_rect.contains(m)).unwrap_or(false);
                    if is_hovered {
                        hovered_clip_id = Some(clip.id);
                        if let Some(m_pos) = mouse_pos {
                            if left_handle.is_positive() && left_handle.contains(m_pos) {
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeEast);
                            } else if right_handle.is_positive() && right_handle.contains(m_pos) {
                                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::ResizeWest);
                            }
                        }
                    }

                    let clip_resp = ui.interact(
                        interact_rect,
                        ui.id().with(format!("clip_{}", clip.id)),
                        egui::Sense::click_and_drag(),
                    );

                    if clip_resp.clicked() {
                        clicked_clip_id = Some(clip.id);
                    }

                    // Начало действия: Trimming (если зажали возле истинного края клипа) или Moving (если зажали по телу клипа)
                    if clip_resp.drag_started() && timeline.moving_clip.is_none() && timeline.trim_session.is_none() {
                        if let Some(m_pos) = mouse_pos {
                            if left_handle.is_positive() && left_handle.contains(m_pos) {
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

        if let Some(trim) = start_trim_request {
            timeline.push_history();
            timeline.trim_session = Some(trim);
        }
        if let Some(req) = start_drag_request {
            timeline.push_history();
            timeline.moving_clip = Some(req);
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

        // Обновление позиции перетаскиваемого клипа (по кадрам и между дорожками)
        if let Some((drag_id, start_mouse_x, start_mouse_y, init_start_frame, init_track)) = timeline.moving_clip {
            if let Some(m_pos) = mouse_pos {
                let dx = m_pos.x - start_mouse_x;
                let dy = m_pos.y - start_mouse_y;
                let delta_frames = (dx / timeline.zoom_factor).round() as i64;
                let delta_tracks = (dy / track_height).round() as isize;

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
                        let snap_thresh = (12.0 / timeline.zoom_factor).max(2.0) as i64;
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

                    let ghost_y = ruler_rect.max.y + (target_track_idx as f32 * track_height) - timeline.scroll_y;
                    let ghost_x = canvas_rect.min.x + (drop_frame as f32 * timeline.zoom_factor) - timeline.scroll_x;
                    let ghost_w = (media.duration_frames as f32 * timeline.zoom_factor).max(12.0);
                    let ghost_rect = Rect::from_min_size(pos2(ghost_x, ghost_y + 2.0), vec2(ghost_w, track_height - 4.0));

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

                        let dur_from_secs = (media.duration_secs * timeline.fps as f64).round().max(1.0) as i64;
                        let actual_dur_frames = if dur_from_secs > 0 { dur_from_secs } else { media.duration_frames };

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
                                duration_frames: actual_dur_frames,
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
                                    duration_frames: actual_dur_frames,
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
                                duration_frames: actual_dur_frames,
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
        for (i, track) in timeline.tracks.iter_mut().enumerate() {
            let y_top = ruler_rect.max.y + (i as f32 * track_height) - timeline.scroll_y;
            let header_row_rect = Rect::from_min_size(pos2(full_rect.min.x, y_top), vec2(header_width, track_height));
            if Self::render_interactive_track_header(ui, &headers_tracks_painter, header_row_rect, track, i) {
                audio_dirty_requested = true;
            }
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
        painter.line_segment(
            [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
            Stroke::new(1.0, ResolveColors::BORDER),
        );

        let frames_per_sec = timeline.fps as f32;
        let sec_width = frames_per_sec * timeline.zoom_factor;

        // Адаптивный шаг меток: расстояние между таймкодами всегда >= 80px, никакого наложения и мыла
        let min_spacing = 80.0;
        let step_seconds = if sec_width >= min_spacing {
            1
        } else if sec_width * 2.0 >= min_spacing {
            2
        } else if sec_width * 5.0 >= min_spacing {
            5
        } else if sec_width * 10.0 >= min_spacing {
            10
        } else if sec_width * 30.0 >= min_spacing {
            30
        } else if sec_width * 60.0 >= min_spacing {
            60
        } else if sec_width * 300.0 >= min_spacing {
            300
        } else {
            600
        };

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
    }

    fn render_interactive_track_header(
        ui: &mut Ui,
        painter: &egui::Painter,
        rect: Rect,
        track: &mut crate::timeline::Track,
        index: usize,
    ) -> bool {
        let mut audio_changed = false;

        // Фон строки заголовка
        let bg_color = Color32::from_rgb(0x1a, 0x1b, 0x1e);
        painter.rect_filled(rect, CornerRadius::ZERO, bg_color);
        painter.line_segment(
            [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
            Stroke::new(1.0, ResolveColors::BORDER),
        );

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
        let full_name = if track.is_video {
            format!("Video {}", index + 1)
        } else {
            format!("Audio {}", index.saturating_sub(1))
        };
        painter.text(
            pos2(rect.min.x + 42.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            full_name,
            egui::FontId::proportional(11.0),
            ResolveColors::TEXT_PRIMARY,
        );

        // Кнопки управления справа:
        if track.is_video {
            // Кнопка Lock 🔒
            let lock_rect = Rect::from_min_size(pos2(rect.max.x - 22.0, rect.center().y - 9.0), vec2(18.0, 18.0));
            let lock_resp = ui.interact(lock_rect, ui.id().with(format!("lock_track_{}", index)), egui::Sense::click());
            if lock_resp.clicked() {
                track.is_locked = !track.is_locked;
            }
            let lock_color = if track.is_locked { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_MUTED };
            painter.text(lock_rect.center(), egui::Align2::CENTER_CENTER, if track.is_locked { "🔒" } else { "🔓" }, egui::FontId::proportional(10.0), lock_color);

            // Кнопка включения/выключения видеодорожки (Eye / Mute)
            let eye_rect = Rect::from_min_size(pos2(rect.max.x - 42.0, rect.center().y - 9.0), vec2(18.0, 18.0));
            let eye_resp = ui.interact(eye_rect, ui.id().with(format!("eye_track_{}", index)), egui::Sense::click());
            if eye_resp.clicked() {
                track.is_muted = !track.is_muted;
            }
            let eye_color = if !track.is_muted { ResolveColors::TEXT_PRIMARY } else { Color32::from_rgb(100, 100, 100) };
            painter.text(eye_rect.center(), egui::Align2::CENTER_CENTER, if !track.is_muted { "👁" } else { "⊘" }, egui::FontId::proportional(10.0), eye_color);
        } else {
            // Аудиодорожка: Solo [S], Mute [M], Lock 🔒
            let lock_rect = Rect::from_min_size(pos2(rect.max.x - 18.0, rect.center().y - 8.0), vec2(15.0, 16.0));
            let lock_resp = ui.interact(lock_rect, ui.id().with(format!("lock_track_{}", index)), egui::Sense::click());
            if lock_resp.clicked() {
                track.is_locked = !track.is_locked;
            }
            painter.text(lock_rect.center(), egui::Align2::CENTER_CENTER, if track.is_locked { "🔒" } else { "🔓" }, egui::FontId::proportional(9.0), if track.is_locked { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_MUTED });

            let mute_rect = Rect::from_min_size(pos2(rect.max.x - 36.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            let mute_resp = ui.interact(mute_rect, ui.id().with(format!("mute_track_{}", index)), egui::Sense::click());
            if mute_resp.clicked() {
                track.is_muted = !track.is_muted;
                audio_changed = true;
            }
            let mute_bg = if track.is_muted { ResolveColors::VU_RED } else { Color32::from_rgb(0x23, 0x26, 0x2b) };
            painter.rect_filled(mute_rect, CornerRadius::same(2), mute_bg);
            painter.text(mute_rect.center(), egui::Align2::CENTER_CENTER, "M", egui::FontId::monospace(9.0), if track.is_muted { Color32::WHITE } else { ResolveColors::TEXT_MUTED });

            let solo_rect = Rect::from_min_size(pos2(rect.max.x - 54.0, rect.center().y - 8.0), vec2(16.0, 16.0));
            let solo_resp = ui.interact(solo_rect, ui.id().with(format!("solo_track_{}", index)), egui::Sense::click());
            if solo_resp.clicked() {
                track.is_solo = !track.is_solo;
                audio_changed = true;
            }
            let solo_bg = if track.is_solo { ResolveColors::VU_YELLOW } else { Color32::from_rgb(0x23, 0x26, 0x2b) };
            painter.rect_filled(solo_rect, CornerRadius::same(2), solo_bg);
            painter.text(solo_rect.center(), egui::Align2::CENTER_CENTER, "S", egui::FontId::monospace(9.0), if track.is_solo { Color32::BLACK } else { ResolveColors::TEXT_MUTED });
        }

        audio_changed
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

        // Верхняя фирменная полоска клипа Resolve (Darker Header Strip)
        let header_h = 13.0_f32.min(rect.height() * 0.35);
        let clip_header_rect = Rect::from_min_size(rect.min, vec2(rect.width(), header_h));
        painter.rect_filled(clip_header_rect, CornerRadius::same(2), Color32::from_rgba_unmultiplied(0, 0, 0, 40));

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
                        let sec_per_px = 1.0 / (timeline.zoom_factor * timeline.fps as f32);

                        let mut x = visible_start_x;
                        let step_px = 2.0;
                        let mut mesh = egui::Mesh::default();
                        let wave_color = Color32::from_rgb(0x38, 0xd4, 0x8a);
                        let silence_color = Color32::from_rgba_unmultiplied(255, 255, 255, 45);

                        while x < visible_end_x {
                            let rel_x = x - rect.min.x;
                            let sec_in_clip = rel_x as f64 * sec_per_px as f64;
                            let total_source_sec = clip.source_in_secs + sec_in_clip;
                            let sample_idx = (total_source_sec * pts_per_sec).round() as usize;

                            let amp = peaks.get(sample_idx).copied().unwrap_or(0.0);

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

        // Название клипа: прижимаем к левому видимому краю полосы (как в DaVinci Resolve)
        let label_left = (rect.min.x + 6.0).max(canvas_rect.min.x + 6.0);
        if rect.max.x - label_left > 20.0 {
            let label = if clip.link_group.is_some() {
                format!("🔗 {}", clip.name)
            } else {
                clip.name.clone()
            };
            painter.text(
                pos2(label_left, rect.min.y + 2.0),
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
