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
    pub fps_num: f64,
    #[allow(dead_code)]
    pub codec: String,
    pub duration: String,
    pub duration_secs: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub audio_tracks_count: usize,
    pub is_audio_only: bool,
}

#[derive(Debug, Clone)]
pub struct MediaPoolState {
    pub assets: Vec<MediaAsset>,
    #[allow(dead_code)]
    pub selected_bin: String,
    pub selected_asset_id: Option<u64>,
    pub next_id: u64,
}

impl MediaPoolState {
    pub fn new_demo() -> Self {
        Self {
            assets: Vec::new(),
            selected_bin: "Master".to_string(),
            selected_asset_id: None,
            next_id: 1,
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
        let is_audio_ext = matches!(ext.as_str(), "wav" | "mp3" | "flac" | "aac" | "ogg" | "m4a");

        let mut res = if is_audio_ext { "48.0 kHz Stereo".to_string() } else { "1920x1080 FHD".to_string() };
        let mut fps_str = if is_audio_ext { "Audio Track".to_string() } else { "24.00 fps".to_string() };
        let mut fps_num = 24.0;
        let mut codec_str = ext.to_uppercase();
        let mut dur_str = "00:00:10:00".to_string();
        let mut duration_secs = 10.0;
        let mut has_video = !is_audio_ext;
        let mut has_audio = is_audio_ext;
        let mut audio_tracks_count = if is_audio_ext { 1 } else { 0 };

        if let Ok(ictx) = ffmpeg_next::format::input(path) {
            let v_stream = ictx.streams().best(ffmpeg_next::media::Type::Video);
            let a_stream = ictx.streams().best(ffmpeg_next::media::Type::Audio);

            has_video = v_stream.is_some();
            audio_tracks_count = ictx
                .streams()
                .filter(|s| s.parameters().medium() == ffmpeg_next::media::Type::Audio)
                .count();
            has_audio = audio_tracks_count > 0 || a_stream.is_some();

            if let Some(stream) = v_stream {
                if let Ok(decoder) = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters()) {
                    if let Ok(v_dec) = decoder.decoder().video() {
                        res = format!("{}x{}", v_dec.width(), v_dec.height());
                        let f = f64::from(stream.avg_frame_rate());
                        if f > 0.0 && !f.is_nan() {
                            fps_num = f;
                            fps_str = format!("{:.2} fps", f);
                        }
                        codec_str = format!("{:?}", stream.parameters().id()).replace("Id(", "").replace(")", "");
                    }
                }
            } else if let Some(stream) = a_stream {
                if let Ok(decoder) = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters()) {
                    if let Ok(a_dec) = decoder.decoder().audio() {
                        res = format!("{} Hz {}ch", a_dec.rate(), a_dec.channels());
                        codec_str = format!("{:?}", stream.parameters().id()).replace("Id(", "").replace(")", "");
                    }
                }
            }

            let d_secs = ictx.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
            if d_secs > 0.0 {
                duration_secs = d_secs;
                let m = (d_secs / 60.0).floor() as u32;
                let s = (d_secs % 60.0).floor() as u32;
                let f = ((d_secs - d_secs.floor()) * fps_num).round() as u32;
                dur_str = format!("00:{:02}:{:02}:{:02}", m, s, f);
            }
        }

        let asset = MediaAsset {
            id: self.next_id,
            filename,
            path: Some(path.to_path_buf()),
            resolution: res,
            fps: fps_str,
            fps_num,
            codec: codec_str,
            duration: dur_str,
            duration_secs,
            has_video,
            has_audio,
            audio_tracks_count,
            is_audio_only: has_audio && !has_video,
        };
        self.next_id += 1;
        self.selected_asset_id = Some(asset.id);
        self.assets.push(asset);
    }

    /// Вставить выбранный ассет на таймлайн в позицию плейхеда
    pub fn insert_selected_to_timeline(&self, timeline: &mut TimelineState) {
        if let Some(id) = self.selected_asset_id {
            if let Some(asset) = self.assets.iter().find(|a| a.id == id) {
                // Если таймлайн пустой, переключаем FPS проекта на FPS первого импортированного видео!
                if timeline.clips.is_empty() && asset.fps_num > 0.0 {
                    timeline.fps = asset.fps_num.round().clamp(1.0, 240.0) as u32;
                }

                timeline.push_history();
                let dur = (asset.duration_secs * timeline.fps as f64).round().max(1.0) as i64;
                let start = timeline.playhead_frame;
                let num_audio = asset.audio_tracks_count.max(if asset.has_audio { 1 } else { 0 });
                let link_id = if asset.has_video && num_audio > 0 {
                    let id = timeline.next_link_id;
                    timeline.next_link_id += 1;
                    Some(id)
                } else {
                    None
                };

                // 1. Создаем видеоклип на V1, если есть видео
                if asset.has_video {
                    let v_clip = Clip {
                        id: timeline.next_clip_id,
                        name: asset.filename.clone(),
                        track_index: 1, // V1
                        start_frame: start,
                        duration_frames: dur,
                        is_audio: false,
                        color: ResolveColors::VIDEO_CLIP,
                        source_path: asset.path.clone(),
                        source_in_frame: 0,
                        source_in_secs: 0.0,
                        source_fps: asset.fps_num,
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
                }

                // 2. Создаем аудиоклипы для ВСЕХ аудиодорожек файла (A1, A2...)
                if num_audio > 0 {
                    for a_idx in 0..num_audio {
                        let target_track = 2 + a_idx;
                        timeline.ensure_audio_track(target_track);

                        let track_title = if num_audio > 1 {
                            format!("{} [A{}]", asset.filename, a_idx + 1)
                        } else {
                            asset.filename.clone()
                        };

                        let a_clip = Clip {
                            id: timeline.next_clip_id,
                            name: track_title,
                            track_index: target_track,
                            start_frame: start,
                            duration_frames: dur,
                            is_audio: true,
                            color: ResolveColors::AUDIO_CLIP,
                            source_path: asset.path.clone(),
                            source_in_frame: 0,
                            source_in_secs: 0.0,
                            source_fps: asset.fps_num,
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
                        if !asset.has_video && a_idx == 0 {
                            timeline.selected_clip_id = Some(a_clip.id);
                        }
                        timeline.clips.push(a_clip);
                    }
                }
                timeline.audio_dirty = true;
            }
        }
    }

    pub fn render(&mut self, ui: &mut Ui, timeline: &mut TimelineState) {
        self.render_panel(ui, timeline);
    }

    pub fn render_panel(&mut self, ui: &mut Ui, timeline: &mut TimelineState) {
        ui.vertical(|ui| {
            // Верхняя панель управления Media Pool
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("📁 Media Pool").color(ResolveColors::TEXT_PRIMARY).strong().size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("📥 Import").clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Media Files", &["mp4", "mov", "mkv", "avi", "webm", "wav", "mp3", "flac"])
                            .pick_files()
                        {
                            for f in files {
                                self.import_path(&f);
                                self.insert_selected_to_timeline(timeline);
                            }
                        }
                    }
                });
            });

            ui.separator();

            // Основная зона: список ассетов
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.assets.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(egui::RichText::new("Нет импортированных клипов").size(12.0).color(ResolveColors::TEXT_MUTED));
                        ui.add_space(5.0);
                        if ui.button("📥 Добавить медиа (Ctrl+I)").clicked() {
                            if let Some(files) = rfd::FileDialog::new()
                                .add_filter("Media Files", &["mp4", "mov", "mkv", "avi", "webm", "wav", "mp3", "flac"])
                                .pick_files()
                            {
                                for f in files {
                                    self.import_path(&f);
                                    self.insert_selected_to_timeline(timeline);
                                }
                            }
                        }
                    });
                    return;
                }

                ui.vertical(|ui| {
                    for asset in &self.assets {
                        let is_sel = self.selected_asset_id == Some(asset.id);
                        let bg_color = if is_sel {
                            ResolveColors::BG_HEADER
                        } else {
                            ResolveColors::BG_PANEL
                        };

                        let frame_response = egui::Frame::new()
                            .fill(bg_color)
                            .stroke(if is_sel { Stroke::new(1.0, ResolveColors::ACCENT_ORANGE) } else { Stroke::NONE })
                            .corner_radius(CornerRadius::same(3))
                            .inner_margin(Margin::same(4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let icon = if asset.has_video { "🎬" } else { "🎵" };
                                    ui.label(egui::RichText::new(icon).size(14.0));
                                    ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(&asset.filename).color(ResolveColors::TEXT_PRIMARY).size(11.0).strong());
                                        ui.horizontal(|ui| {
                                            ui.label(egui::RichText::new(&asset.duration).color(ResolveColors::TEXT_MUTED).size(9.0));
                                            ui.label(egui::RichText::new("•").color(ResolveColors::TEXT_MUTED).size(9.0));
                                            ui.label(egui::RichText::new(&asset.fps).color(ResolveColors::TEXT_MUTED).size(9.0));
                                            ui.label(egui::RichText::new("•").color(ResolveColors::TEXT_MUTED).size(9.0));
                                            ui.label(egui::RichText::new(&asset.resolution).color(ResolveColors::TEXT_MUTED).size(9.0));
                                        });
                                    });
                                });
                            });

                        let interact_resp = ui.interact(
                            frame_response.response.rect,
                            ui.id().with(format!("asset_card_{}", asset.id)),
                            egui::Sense::click_and_drag(),
                        );

                        if interact_resp.clicked() {
                            self.selected_asset_id = Some(asset.id);
                        }

                        if interact_resp.double_clicked() {
                            self.selected_asset_id = Some(asset.id);
                            self.insert_selected_to_timeline(timeline);
                        }

                        // ИНИЦИАЛИЗАЦИЯ ПЕРЕТАСКИВАНИЯ НА ТАЙМЛАЙН
                        if interact_resp.drag_started() {
                            let dur_frames = (asset.duration_secs * timeline.fps as f64).round().max(1.0) as i64;
                            timeline.dragged_media = Some(DraggedMedia {
                                name: asset.filename.clone(),
                                path: asset.path.clone(),
                                is_audio: asset.is_audio_only,
                                has_video: asset.has_video,
                                has_audio: asset.has_audio,
                                audio_tracks_count: asset.audio_tracks_count,
                                duration_frames: dur_frames,
                                duration_secs: asset.duration_secs,
                                source_fps: asset.fps_num,
                                link_group: None,
                            });
                        }

                        ui.add_space(2.0);
                    }
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_import_replay_mkv() {
        let p = PathBuf::from("/mnt/kdenlive-cache/epic-movie/Replay 2026-10-13 21-12-56.mkv");
        if !p.exists() {
            return;
        }
        let mut pool = MediaPoolState::new_demo();
        pool.assets.clear();
        pool.add_file(&p);
        assert_eq!(pool.assets.len(), 1);
        let asset = &pool.assets[0];
        assert!(asset.has_video);
        assert!(asset.has_audio);
        assert_eq!(asset.audio_tracks_count, 2);

        let mut timeline = TimelineState::new_empty();
        pool.insert_selected_to_timeline(&mut timeline);

        // 1 видеоклип на V1 + 2 связанных аудиоклипа на A1 и A2
        assert_eq!(timeline.clips.len(), 3);
        let video_clips: Vec<_> = timeline.clips.iter().filter(|c| !c.is_audio).collect();
        let audio_clips: Vec<_> = timeline.clips.iter().filter(|c| c.is_audio).collect();
        assert_eq!(video_clips.len(), 1);
        assert_eq!(audio_clips.len(), 2);
        assert_eq!(audio_clips[0].track_index, 2); // A1
        assert_eq!(audio_clips[1].track_index, 3); // A2

        // Все клипы связаны общим link_group
        let link_id = video_clips[0].link_group;
        assert!(link_id.is_some());
        assert_eq!(audio_clips[0].link_group, link_id);
        assert_eq!(audio_clips[1].link_group, link_id);
    }
}
