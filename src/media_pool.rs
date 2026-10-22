use egui::{CornerRadius, Margin, Stroke, Ui};
use std::path::{Path, PathBuf};
use crate::theme::ResolveColors;
use crate::timeline::{Clip, DraggedMedia, TimelineState};

#[derive(Debug, Clone)]
pub struct MediaAsset {
    pub id: u64,
    pub filename: String,
    pub path: Option<PathBuf>,
    pub resolution: String,
    pub fps: String,
    pub codec: String,
    pub duration: String,
    pub is_audio: bool,
}

#[derive(Debug, Clone)]
pub struct MediaPoolState {
    pub assets: Vec<MediaAsset>,
    pub selected_bin: String,
    pub selected_asset_id: Option<u64>,
    pub next_id: u64,
}

impl MediaPoolState {
    pub fn new_demo() -> Self {
        Self {
            assets: vec![
                MediaAsset {
                    id: 1,
                    filename: "A001_C001_0422.MOV".to_string(),
                    path: None,
                    resolution: "3840x2160 UHD".to_string(),
                    fps: "24.00 fps".to_string(),
                    codec: "Apple ProRes 422 HQ".to_string(),
                    duration: "00:00:05:00".to_string(),
                    is_audio: false,
                },
                MediaAsset {
                    id: 2,
                    filename: "A001_C002_0422.MOV".to_string(),
                    path: None,
                    resolution: "3840x2160 UHD".to_string(),
                    fps: "24.00 fps".to_string(),
                    codec: "Apple ProRes 422 HQ".to_string(),
                    duration: "00:00:07:00".to_string(),
                    is_audio: false,
                },
                MediaAsset {
                    id: 3,
                    filename: "B_ROLL_DRONE_4K.MP4".to_string(),
                    path: None,
                    resolution: "4096x2160 DCI".to_string(),
                    fps: "59.94 fps".to_string(),
                    codec: "HEVC 10-bit 4:2:2".to_string(),
                    duration: "00:00:14:12".to_string(),
                    is_audio: false,
                },
                MediaAsset {
                    id: 4,
                    filename: "VOICE_MIC_01.WAV".to_string(),
                    path: None,
                    resolution: "48.0 kHz".to_string(),
                    fps: "24-bit".to_string(),
                    codec: "PCM Broadcast Wave".to_string(),
                    duration: "00:00:12:00".to_string(),
                    is_audio: true,
                },
            ],
            selected_bin: "Master".to_string(),
            selected_asset_id: Some(1),
            next_id: 10,
        }
    }

    /// Импорт пути (файла или папки)
    pub fn import_path(&mut self, path: &Path) -> usize {
        let mut count = 0;
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        count += self.import_path(&p);
                    } else if Self::is_media_file(&p) {
                        self.add_file(&p);
                        count += 1;
                    }
                }
            }
        } else if Self::is_media_file(path) {
            self.add_file(path);
            count += 1;
        }
        count
    }

    pub fn is_media_file(path: &Path) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        matches!(
            ext.as_str(),
            "mp4" | "mov" | "mkv" | "avi" | "webm" | "flv" | "wmv" | "wav" | "mp3" | "flac" | "aac" | "ogg" | "m4a"
        )
    }

    pub fn add_file(&mut self, path: &Path) {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("clip")
            .to_string();

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_audio = matches!(ext.as_str(), "wav" | "mp3" | "flac" | "aac" | "ogg" | "m4a");

        let mut res = if is_audio { "48.0 kHz Stereo".to_string() } else { "1920x1080 FHD".to_string() };
        let mut fps_str = if is_audio { "24-bit".to_string() } else { "24.00 fps".to_string() };
        let mut codec_str = ext.to_uppercase();
        let mut dur_str = "00:00:10:00".to_string();

        if let Ok(ictx) = ffmpeg_next::format::input(path) {
            if let Some(stream) = ictx.streams().best(if is_audio {
                ffmpeg_next::media::Type::Audio
            } else {
                ffmpeg_next::media::Type::Video
            }) {
                if !is_audio {
                    if let Ok(decoder) = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters()) {
                        if let Ok(v_dec) = decoder.decoder().video() {
                            res = format!("{}x{}", v_dec.width(), v_dec.height());
                            let f = f64::from(stream.avg_frame_rate());
                            if f > 0.0 {
                                fps_str = format!("{:.2} fps", f);
                            }
                            codec_str = format!("{:?}", stream.parameters().id()).replace("Id(", "").replace(")", "");
                        }
                    }
                }
            }
            let duration_secs = ictx.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
            if duration_secs > 0.0 {
                let m = (duration_secs / 60.0).floor() as u32;
                let s = (duration_secs % 60.0).floor() as u32;
                let f = ((duration_secs - duration_secs.floor()) * 24.0).round() as u32;
                dur_str = format!("00:{:02}:{:02}:{:02}", m, s, f);
            }
        }

        let asset = MediaAsset {
            id: self.next_id,
            filename,
            path: Some(path.to_path_buf()),
            resolution: res,
            fps: fps_str,
            codec: codec_str,
            duration: dur_str,
            is_audio,
        };
        self.next_id += 1;
        self.selected_asset_id = Some(asset.id);
        self.assets.push(asset);
    }

    /// Вставить выбранный ассет на таймлайн
    pub fn insert_selected_to_timeline(&self, timeline: &mut TimelineState) {
        if let Some(id) = self.selected_asset_id {
            if let Some(asset) = self.assets.iter().find(|a| a.id == id) {
                timeline.push_history();
                let track_idx = if asset.is_audio { 2 } else { 1 };
                let dur = 120; // 5 сек при 24fps
                let start = timeline.playhead_frame;

                let clip = Clip {
                    id: timeline.next_clip_id,
                    name: asset.filename.clone(),
                    track_index: track_idx,
                    start_frame: start,
                    duration_frames: dur,
                    is_audio: asset.is_audio,
                    color: if asset.is_audio {
                        ResolveColors::AUDIO_CLIP
                    } else {
                        ResolveColors::VIDEO_CLIP
                    },
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
                timeline.selected_clip_id = Some(clip.id);
                timeline.clips.push(clip);
                timeline.playhead_frame = start + dur;
            }
        }
    }

    pub fn render(&mut self, ui: &mut Ui, timeline: &mut TimelineState) {
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new("📁 Media Pool").size(13.0).strong().color(ResolveColors::TEXT_PRIMARY));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(egui::RichText::new("📂 Import Files...").size(11.0)).clicked() {
                    if let Some(files) = rfd::FileDialog::new()
                        .add_filter("Video & Audio", &["mp4", "mov", "mkv", "avi", "webm", "wav", "mp3", "flac"])
                        .pick_files()
                    {
                        for f in files {
                            self.import_path(&f);
                        }
                    }
                }
                if ui.button(egui::RichText::new("📁 Import Folder...").size(11.0)).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.import_path(&folder);
                    }
                }
            });
        });

        ui.separator();

        ui.columns(2, |cols| {
            // Левая колонка: Bins
            cols[0].vertical(|ui| {
                ui.label(egui::RichText::new("PROJECT BINS").size(10.0).color(ResolveColors::TEXT_MUTED));
                let bins = ["Master", "  📹 Video B-Roll", "  🎙 Audio & Music", "  ✨ Timelines"];
                for bin in bins {
                    let is_active = self.selected_bin == bin.trim();
                    let text = egui::RichText::new(bin)
                        .size(11.0)
                        .color(if is_active { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_PRIMARY });
                    if ui.selectable_label(is_active, text).clicked() {
                        self.selected_bin = bin.trim().to_string();
                    }
                }

                ui.add_space(8.0);
                ui.separator();
                if ui.button(egui::RichText::new("➕ Add to Timeline [Enter]").size(11.0)).clicked() {
                    self.insert_selected_to_timeline(timeline);
                }
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("💡 Перетащите клип мышью\nпрямо на любую дорожку V1/A1")
                        .size(9.0)
                        .color(ResolveColors::TEXT_MUTED),
                );
            });

            // Правая колонка: Клипы в бине с поддержкой Drag & Drop на таймлайн
            cols[1].vertical(|ui| {
                ui.label(egui::RichText::new(format!("CLIPS ({}) - Drag to Track", self.assets.len())).size(10.0).color(ResolveColors::TEXT_MUTED));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for asset in &self.assets {
                        let is_sel = self.selected_asset_id == Some(asset.id);
                        let bg = if is_sel {
                            ResolveColors::ACCENT_ORANGE.gamma_multiply(0.2)
                        } else {
                            ResolveColors::BG_PANEL
                        };

                        let frame_response = egui::Frame::new()
                            .fill(bg)
                            .stroke(Stroke::new(
                                1.0,
                                if is_sel { ResolveColors::ACCENT_ORANGE } else { ResolveColors::BORDER },
                            ))
                            .corner_radius(CornerRadius::same(2))
                            .inner_margin(Margin::same(4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let icon = if asset.is_audio { "🎵" } else { "🎬" };
                                    ui.label(egui::RichText::new(icon).size(14.0));
                                    ui.vertical(|ui| {
                                        let name = egui::RichText::new(&asset.filename)
                                            .size(11.0)
                                            .color(if is_sel { ResolveColors::ACCENT_ORANGE } else { ResolveColors::TEXT_PRIMARY })
                                            .strong();
                                        ui.label(name);
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{} | {} | {} | {}",
                                                asset.resolution, asset.fps, asset.codec, asset.duration
                                            ))
                                            .size(9.0)
                                            .color(ResolveColors::TEXT_MUTED),
                                        );
                                    });
                                });
                            });

                        // Интерактивность карточки: клик для выбора, перетаскивание на дорожку
                        let interact_resp = ui.interact(
                            frame_response.response.rect,
                            ui.id().with(format!("asset_card_{}", asset.id)),
                            egui::Sense::click_and_drag(),
                        );

                        if interact_resp.clicked() {
                            self.selected_asset_id = Some(asset.id);
                        }

                        // ИНИЦИАЛИЗАЦИЯ ПЕРЕТАСКИВАНИЯ НА ТАЙМЛАЙН
                        if interact_resp.drag_started() {
                            timeline.dragged_media = Some(DraggedMedia {
                                name: asset.filename.clone(),
                                is_audio: asset.is_audio,
                                duration_frames: 120, // 5 сек дефолт
                            });
                        }

                        ui.add_space(2.0);
                    }
                });
            });
        });
    }
}
