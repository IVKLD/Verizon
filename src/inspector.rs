use egui::{DragValue, Slider, Ui};
use crate::theme::ResolveColors;
use crate::timeline::TimelineState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorTab {
    Video,
    Audio,
}

pub struct InspectorState {
    pub current_tab: InspectorTab,
    pub zoom_linked: bool,
}

impl InspectorState {
    pub fn new() -> Self {
        Self {
            current_tab: InspectorTab::Video,
            zoom_linked: true,
        }
    }

    pub fn render(&mut self, ui: &mut Ui, timeline: &mut TimelineState) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("🔍 Inspector")
                    .size(13.0)
                    .strong()
                    .color(ResolveColors::TEXT_PRIMARY),
            );
        });

        ui.separator();

        let selected_id = timeline.selected_clip_id;
        if let Some(clip_id) = selected_id {
            if let Some(clip) = timeline.clips.iter_mut().find(|c| c.id == clip_id) {
                // Заголовок клипа
                ui.horizontal(|ui| {
                    let icon = if clip.is_audio { "🎵" } else { "🎬" };
                    ui.label(egui::RichText::new(icon).size(13.0));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&clip.name)
                                .size(12.0)
                                .strong()
                                .color(ResolveColors::ACCENT_ORANGE),
                        )
                        .truncate(),
                    );
                });

                ui.add_space(4.0);

                // Табы Video / Audio
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            self.current_tab == InspectorTab::Video,
                            egui::RichText::new("Video").size(11.0),
                        )
                        .clicked()
                    {
                        self.current_tab = InspectorTab::Video;
                    }
                    if ui
                        .selectable_label(
                            self.current_tab == InspectorTab::Audio,
                            egui::RichText::new("Audio").size(11.0),
                        )
                        .clicked()
                    {
                        self.current_tab = InspectorTab::Audio;
                    }
                });

                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| {
                    match self.current_tab {
                        InspectorTab::Video => {
                            // Секция TRANSFORM
                            egui::CollapsingHeader::new(
                                egui::RichText::new("TRANSFORM")
                                    .size(11.0)
                                    .color(ResolveColors::TEXT_PRIMARY)
                                    .strong(),
                            )
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::Grid::new("transform_grid")
                                    .num_columns(3)
                                    .spacing([8.0, 6.0])
                                    .show(ui, |ui| {
                                        // Zoom X / Y
                                        ui.label(egui::RichText::new("Zoom X").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        let mut zx = clip.zoom_x;
                                        if ui.add(DragValue::new(&mut zx).speed(0.01).range(0.1..=10.0)).changed() {
                                            clip.zoom_x = zx;
                                            if self.zoom_linked {
                                                clip.zoom_y = zx;
                                            }
                                        }
                                        if ui.button("⟲").on_hover_text("Сбросить зум").clicked() {
                                            clip.zoom_x = 1.0;
                                            clip.zoom_y = 1.0;
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new("Zoom Y").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        let mut zy = clip.zoom_y;
                                        if ui.add(DragValue::new(&mut zy).speed(0.01).range(0.1..=10.0)).changed() {
                                            clip.zoom_y = zy;
                                            if self.zoom_linked {
                                                clip.zoom_x = zy;
                                            }
                                        }
                                        ui.checkbox(&mut self.zoom_linked, "🔗");
                                        ui.end_row();

                                        // Position X / Y
                                        ui.label(egui::RichText::new("Position X").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(DragValue::new(&mut clip.position_x).speed(1.0));
                                        if ui.button("⟲").on_hover_text("Сбросить позицию X").clicked() {
                                            clip.position_x = 0.0;
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new("Position Y").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(DragValue::new(&mut clip.position_y).speed(1.0));
                                        if ui.button("⟲").on_hover_text("Сбросить позицию Y").clicked() {
                                            clip.position_y = 0.0;
                                        }
                                        ui.end_row();

                                        // Rotation
                                        ui.label(egui::RichText::new("Rotation").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(DragValue::new(&mut clip.rotation).speed(0.5).range(-180.0..=180.0).suffix("°"));
                                        if ui.button("⟲").on_hover_text("Сбросить вращение").clicked() {
                                            clip.rotation = 0.0;
                                        }
                                        ui.end_row();

                                        // Flip H / V (DaVinci Resolve Mirroring)
                                        ui.label(egui::RichText::new("Flip").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.horizontal(|ui| {
                                            let h_btn = egui::Button::new(
                                                egui::RichText::new("↔ H")
                                                    .size(10.0)
                                                    .color(if clip.flip_h { egui::Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
                                            )
                                            .fill(if clip.flip_h { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
                                            if ui.add(h_btn).on_hover_text("Отразить по горизонтали (Flip Horizontal)").clicked() {
                                                clip.flip_h = !clip.flip_h;
                                            }

                                            let v_btn = egui::Button::new(
                                                egui::RichText::new("↕ V")
                                                    .size(10.0)
                                                    .color(if clip.flip_v { egui::Color32::WHITE } else { ResolveColors::TEXT_MUTED }),
                                            )
                                            .fill(if clip.flip_v { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BG_HEADER });
                                            if ui.add(v_btn).on_hover_text("Отразить по вертикали (Flip Vertical)").clicked() {
                                                clip.flip_v = !clip.flip_v;
                                            }
                                        });
                                        if ui.button("⟲").on_hover_text("Сбросить отражение").clicked() {
                                            clip.flip_h = false;
                                            clip.flip_v = false;
                                        }
                                        ui.end_row();
                                    });
                            });

                            ui.add_space(6.0);

                            // Секция CROPPING (DaVinci Resolve обрезка кадра)
                            egui::CollapsingHeader::new(
                                egui::RichText::new("CROPPING")
                                    .size(11.0)
                                    .color(ResolveColors::TEXT_PRIMARY)
                                    .strong(),
                            )
                            .default_open(false)
                            .show(ui, |ui| {
                                egui::Grid::new("cropping_grid")
                                    .num_columns(3)
                                    .spacing([8.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new("Crop Left").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(Slider::new(&mut clip.crop_left, 0.0..=50.0).suffix("%"));
                                        if ui.button("⟲").on_hover_text("Сбросить Crop Left").clicked() {
                                            clip.crop_left = 0.0;
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new("Crop Right").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(Slider::new(&mut clip.crop_right, 0.0..=50.0).suffix("%"));
                                        if ui.button("⟲").on_hover_text("Сбросить Crop Right").clicked() {
                                            clip.crop_right = 0.0;
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new("Crop Top").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(Slider::new(&mut clip.crop_top, 0.0..=50.0).suffix("%"));
                                        if ui.button("⟲").on_hover_text("Сбросить Crop Top").clicked() {
                                            clip.crop_top = 0.0;
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new("Crop Bottom").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(Slider::new(&mut clip.crop_bottom, 0.0..=50.0).suffix("%"));
                                        if ui.button("⟲").on_hover_text("Сбросить Crop Bottom").clicked() {
                                            clip.crop_bottom = 0.0;
                                        }
                                        ui.end_row();
                                    });
                            });

                            ui.add_space(6.0);

                            // Секция COMPOSITING
                            egui::CollapsingHeader::new(
                                egui::RichText::new("COMPOSITE")
                                    .size(11.0)
                                    .color(ResolveColors::TEXT_PRIMARY)
                                    .strong(),
                            )
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::Grid::new("composite_grid")
                                    .num_columns(2)
                                    .spacing([8.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new("Opacity").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        ui.add(Slider::new(&mut clip.opacity, 0.0..=100.0).suffix("%"));
                                        ui.end_row();
                                    });
                            });
                        }
                        InspectorTab::Audio => {
                            // Секция AUDIO
                            egui::CollapsingHeader::new(
                                egui::RichText::new("VOLUME & PAN")
                                    .size(11.0)
                                    .color(ResolveColors::TEXT_PRIMARY)
                                    .strong(),
                            )
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::Grid::new("audio_grid")
                                    .num_columns(3)
                                    .spacing([8.0, 6.0])
                                    .show(ui, |ui| {
                                        ui.label(egui::RichText::new("Volume").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        let v_ch = ui.add(Slider::new(&mut clip.volume_db, -48.0..=12.0).suffix(" dB")).changed();
                                        let v_rst = ui.button("⟲").clicked();
                                        if v_rst {
                                            clip.volume_db = 0.0;
                                        }
                                        ui.end_row();

                                        ui.label(egui::RichText::new("Pan").size(11.0).color(ResolveColors::TEXT_MUTED));
                                        let p_ch = ui.add(Slider::new(&mut clip.pan, -100.0..=100.0).suffix("%")).changed();
                                        let p_rst = ui.button("⟲").clicked();
                                        if p_rst {
                                            clip.pan = 0.0;
                                        }
                                        ui.end_row();

                                        if v_ch || v_rst || p_ch || p_rst {
                                            timeline.audio_dirty = true;
                                        }
                                    });
                            });
                        }
                    }
                });
            }
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("Выберите клип на таймлайне\nдля настройки параметров")
                        .size(11.0)
                        .color(ResolveColors::TEXT_MUTED),
                );
            });
        }
    }
}
