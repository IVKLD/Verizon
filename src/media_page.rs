use egui::{pos2, vec2, Color32, CornerRadius, Rect, Stroke, StrokeKind, Ui};
use std::path::{Path, PathBuf};
use crate::media_pool::MediaPoolState;
use crate::theme::ResolveColors;
use crate::video_decoder::VideoPlayer;

#[derive(Debug, Clone)]
pub struct StorageEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub display_name: String,
}

pub struct MediaPageState {
    pub current_directory: PathBuf,
    pub storage_files: Vec<StorageEntry>,
    pub selected_storage_file: Option<PathBuf>,
}

impl MediaPageState {
    pub fn new() -> Self {
        let default_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/home"));
        let mut state = Self {
            current_directory: default_dir.clone(),
            storage_files: Vec::new(),
            selected_storage_file: None,
        };
        state.scan_directory(&default_dir);
        state
    }

    pub fn scan_directory(&mut self, dir: &Path) {
        self.current_directory = dir.to_path_buf();
        self.storage_files.clear();
        self.selected_storage_file = None;

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_dir = path.is_dir();
                if is_dir || MediaPoolState::is_media_file(&path) {
                    let file_name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    let icon = if is_dir { "📁" } else { "🎬" };
                    let display_name = format!("{} {}", icon, file_name);
                    self.storage_files.push(StorageEntry {
                        path,
                        is_dir,
                        display_name,
                    });
                }
            }
        }
        self.storage_files.sort_by(|a, b| {
            if a.is_dir == b.is_dir {
                a.path.file_name().cmp(&b.path.file_name())
            } else if a.is_dir {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });
    }

    pub fn render(
        &mut self,
        ui: &mut Ui,
        media_pool: &mut MediaPoolState,
        video_player: &mut VideoPlayer,
        ctx: &egui::Context,
    ) {
        let total_h = ui.available_height();
        let top_h = (total_h * 0.52).max(220.0);

        // 1. ВЕРХНЯЯ ЧАСТЬ: Media Storage Browser (слева) + Media Storage Clips (центр) + Viewer/Metadata (справа)
        ui.allocate_ui(vec2(ui.available_width(), top_h), |ui| {
            ui.columns(3, |cols| {
                // КОЛОНКА 1: Дерево дисков и папок (Media Storage)
                cols[0].vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("🖴 MEDIA STORAGE")
                                .size(11.0)
                                .strong()
                                .color(ResolveColors::TEXT_PRIMARY),
                        );
                    });
                    ui.separator();

                    // Быстрые закладки
                    let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/home".to_string());
                    let bookmarks = [
                        ("🏠 Home", PathBuf::from(&home_dir)),
                        ("📁 Project Root", std::env::current_dir().unwrap_or_default()),
                        ("🎬 Videos", PathBuf::from(&home_dir).join("Videos")),
                        ("💾 Root /", PathBuf::from("/")),
                    ];

                    for (name, path) in bookmarks {
                        let is_cur = self.current_directory == path;
                        if ui
                            .selectable_label(
                                is_cur,
                                egui::RichText::new(name).size(11.0).color(if is_cur {
                                    ResolveColors::ACCENT_ORANGE
                                } else {
                                    ResolveColors::TEXT_PRIMARY
                                }),
                            )
                            .clicked()
                        {
                            self.scan_directory(&path);
                        }
                    }

                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("📁 {}", self.current_directory.display()))
                            .size(10.0)
                            .color(ResolveColors::TEXT_MUTED),
                    );

                    // Переход наверх
                    if let Some(parent) = self.current_directory.parent() {
                        if ui.button("⬆ .. (Вверх)").clicked() {
                            let p = parent.to_path_buf();
                            self.scan_directory(&p);
                        }
                    }
                });

                // КОЛОНКА 2: Список файлов в выбранной папке (Media Storage Files)
                cols[1].vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("📂 STORAGE CLIPS")
                                .size(11.0)
                                .strong()
                                .color(ResolveColors::TEXT_PRIMARY),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(egui::RichText::new("📥 Import Selected").size(10.0)).clicked() {
                                if let Some(p) = &self.selected_storage_file {
                                    let path_clone = p.clone();
                                    media_pool.import_path(&path_clone);
                                }
                            }
                        });
                    });
                    ui.separator();

                    let mut clicked_dir = None;

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for entry in &self.storage_files {
                            let is_dir = entry.is_dir;
                            let is_sel = self.selected_storage_file.as_ref() == Some(&entry.path);

                            let text = egui::RichText::new(&entry.display_name).size(11.0).color(if is_sel {
                                ResolveColors::ACCENT_ORANGE
                            } else if is_dir {
                                Color32::from_rgb(0xe0, 0xb0, 0x50)
                            } else {
                                ResolveColors::TEXT_PRIMARY
                            });

                            if ui.selectable_label(is_sel, text).clicked() {
                                if is_dir {
                                    clicked_dir = Some(entry.path.clone());
                                } else {
                                    self.selected_storage_file = Some(entry.path.clone());
                                    // Проигрывать выбранный файл
                                    let _ = video_player.open_file(&entry.path, ctx);
                                }
                            }
                        }
                    });

                    if let Some(dir) = clicked_dir {
                        self.scan_directory(&dir);
                    }
                });

                // КОЛОНКА 3: Preview Monitor & Metadata Inspector
                cols[2].vertical(|ui| {
                    ui.label(
                        egui::RichText::new("🖥 MEDIA PREVIEW")
                            .size(11.0)
                            .strong()
                            .color(ResolveColors::TEXT_PRIMARY),
                    );
                    ui.separator();

                    // Экран предпросмотра
                    let w = ui.available_width().max(80.0);
                    let h = (w * 9.0 / 16.0).max(60.0);
                    let (rect, _) = ui.allocate_exact_size(vec2(w, h), egui::Sense::hover());

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
                    } else {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "SELECT FILE TO PREVIEW",
                            egui::FontId::monospace(10.0),
                            Color32::from_rgb(0x60, 0x65, 0x70),
                        );
                    }

                    ui.add_space(6.0);

                    // Метаданные клипа
                    if let Some(path) = &self.selected_storage_file {
                        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        ui.label(egui::RichText::new(format!("File: {}", name)).size(11.0).strong().color(ResolveColors::TEXT_PRIMARY));
                        ui.label(egui::RichText::new(format!("Resolution: {}x{}", video_player.width, video_player.height)).size(10.0).color(ResolveColors::TEXT_MUTED));
                        ui.label(egui::RichText::new(format!("Frame Rate: {:.2} fps", video_player.fps)).size(10.0).color(ResolveColors::TEXT_MUTED));
                        ui.label(egui::RichText::new(format!("Duration Frames: {}", video_player.total_frames)).size(10.0).color(ResolveColors::TEXT_MUTED));
                    }
                });
            });
        });

        ui.separator();

        // 2. НИЖНЯЯ ЧАСТЬ: Master Project Media Pool
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("🎬 MASTER PROJECT MEDIA POOL")
                        .size(12.0)
                        .strong()
                        .color(ResolveColors::ACCENT_ORANGE),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(egui::RichText::new("📂 Import Files via Dialog...").size(11.0)).clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Video & Audio", &["mp4", "mov", "mkv", "avi", "webm", "wav", "mp3", "flac"])
                            .pick_files()
                        {
                            for f in files {
                                media_pool.import_path(&f);
                            }
                        }
                    }
                    if ui.button(egui::RichText::new("📁 Import Folder...").size(11.0)).clicked() {
                        if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                            media_pool.import_path(&folder);
                        }
                    }
                });
            });

            ui.separator();

            // Зона Drag & Drop
            let (drop_rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(32.0), 32.0), egui::Sense::hover());
            let painter = ui.painter_at(drop_rect);
            painter.rect_filled(drop_rect, CornerRadius::same(2), Color32::from_rgb(0x1a, 0x1d, 0x22));
            painter.rect_stroke(
                drop_rect,
                CornerRadius::same(2),
                Stroke::new(1.0, ResolveColors::ACCENT_ORANGE),
                StrokeKind::Inside,
            );
            painter.text(
                drop_rect.center(),
                egui::Align2::CENTER_CENTER,
                "📥 DRAG & DROP FOLDERS OR FILES HERE FROM DESKTOP / FILE MANAGER TO IMPORT",
                egui::FontId::proportional(11.0),
                ResolveColors::ACCENT_ORANGE,
            );

            ui.add_space(4.0);

            // Сетка ассетов проекта
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for asset in &media_pool.assets {
                        let is_sel = media_pool.selected_asset_id == Some(asset.id);
                        let bg = if is_sel {
                            ResolveColors::ACCENT_ORANGE.gamma_multiply(0.25)
                        } else {
                            ResolveColors::BG_PANEL
                        };

                        egui::Frame::new()
                            .fill(bg)
                            .stroke(Stroke::new(
                                1.0,
                                if is_sel { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BORDER },
                            ))
                            .corner_radius(CornerRadius::same(2))
                            .inner_margin(egui::Margin::same(6))
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    let icon = if asset.has_audio { "🎵" } else { "🎬" };
                                    ui.label(egui::RichText::new(icon).size(20.0));
                                    let name_text = egui::RichText::new(&asset.filename)
                                        .size(11.0)
                                        .strong()
                                        .color(if is_sel { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_PRIMARY });
                                    if ui.link(name_text).clicked() {
                                        media_pool.selected_asset_id = Some(asset.id);
                                        if let Some(p) = &asset.path {
                                            let _ = video_player.open_file(p, ctx);
                                        }
                                    }
                                    ui.label(
                                        egui::RichText::new(&asset.summary)
                                            .size(9.0)
                                            .color(ResolveColors::TEXT_MUTED),
                                    );
                                });
                            });
                        ui.add_space(4.0);
                    }
                });
            });
        });
    }
}
