//! Автопилот: самопроверка РЕАЛЬНОГО приложения (настоящее окно Wayland, wgpu, CPAL).
//! Включается переменными окружения:
//!   VERIZON_AUTOPILOT=/path/to/file.mkv   — файл для прогона
//! Сценарий повторяет действия пользователя: импорт -> воспроизведение -> Razor -> перемещение клипа ->
//! воспроизведение через склейки и паузу -> воспроизведение до самого конца клипа.
//! На каждом этапе замеряются: интервалы кадров UI, реально показанные видеокадры, отставание видео от плейхеда,
//! наличие звука (VU из аудио-коллбэка) и ошибка позиции звука относительно плейхеда.
//! По завершении печатается отчёт и приложение закрывается с кодом 0 (все проверки пройдены) или 1.

use crate::app::NleApp;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Default)]
struct StageStats {
    intervals_ms: Vec<f64>,
    shown_frames: u64,
    last_shown: i64,
    lags: Vec<i64>,
    vu_samples: u64,
    vu_nonzero: u64,
    audio_err_frames: Vec<f64>,
    drops_at_start: u64,
    last_vu_nonzero_at: Option<Instant>,
    max_video_frame: i64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Load,
    PlayFromStart,
    RazorAndMove,
    PlayAcrossCuts,
    PlayToEnd,
    Done,
}

pub struct Autopilot {
    file: PathBuf,
    stage: Stage,
    stage_start: Instant,
    last_tick: Instant,
    stats: StageStats,
    load_requested: bool,
    load_audio_ready_ms: Option<f64>,
    failures: Vec<String>,
    razor_step: u32,
    moved_clip: Option<u64>,
    move_origin: i64,
}

impl Autopilot {
    pub fn from_env() -> Option<Self> {
        let f = std::env::var("VERIZON_AUTOPILOT").ok()?;
        let file = PathBuf::from(f);
        if !file.exists() {
            eprintln!("[AUTOPILOT] файл не найден: {:?}", file);
            return None;
        }
        Some(Self {
            file,
            stage: Stage::Load,
            stage_start: Instant::now(),
            last_tick: Instant::now(),
            stats: StageStats::default(),
            load_requested: false,
            load_audio_ready_ms: None,
            failures: Vec::new(),
            razor_step: 0,
            moved_clip: None,
            move_origin: 0,
        })
    }

    fn begin_stage(&mut self, app: &NleApp, stage: Stage) {
        println!("[AUTOPILOT] ---- stage {:?} ----", stage);
        let now = Instant::now();
        self.stage = stage;
        self.stage_start = now;
        self.last_tick = now;
        self.stats = StageStats {
            drops_at_start: app.video_player.dropped_frames_count,
            last_shown: -1,
            max_video_frame: -1,
            ..Default::default()
        };
    }

    fn fail(&mut self, msg: String) {
        println!("[AUTOPILOT][FAIL] {}", msg);
        self.failures.push(msg);
    }

    /// Собирает метрики текущего кадра UI
    fn sample(&mut self, app: &NleApp) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64() * 1000.0;
        self.last_tick = now;
        if self.stage == Stage::Load {
            return;
        }
        self.stats.intervals_ms.push(dt);

        let tl = &app.timeline;
        let vp = &app.video_player;
        if vp.last_decoded_frame >= 0 && vp.last_decoded_frame != self.stats.last_shown {
            self.stats.last_shown = vp.last_decoded_frame;
            self.stats.shown_frames += 1;
            self.stats.max_video_frame = self.stats.max_video_frame.max(vp.last_decoded_frame);
        }

        if tl.is_playing {
            // Отставание видео: ожидаемый исходный кадр под плейхедом (с учетом приоритета верхней дорожки V2 > V1)
            if let Some(c) = tl
                .clips
                .iter()
                .filter(|c| !c.is_audio && c.contains_frame(tl.playhead_frame))
                .min_by_key(|c| (c.track_index, if tl.selected_clip_id == Some(c.id) { 0usize } else { 1usize }))
            {
                let off = (tl.playhead_frame - c.start_frame) as f64 / tl.fps as f64;
                let src = ((c.source_in_secs + off) * c.source_fps).round() as i64;
                self.stats.lags.push((src - vp.last_decoded_frame).max(0));
            }

            // Звук: VU из реального аудио-коллбэка
            let (l, r) = app.audio_player.get_vu_levels();
            self.stats.vu_samples += 1;
            if l.max(r) > 0.001 {
                self.stats.vu_nonzero += 1;
                self.stats.last_vu_nonzero_at = Some(now);
            }

            // Позиция звука относительно плейхеда (в кадрах таймлайна)
            let pos = app.audio_player.engine.pos.load(std::sync::atomic::Ordering::Relaxed);
            let audio_secs = pos as f64 / 2.0 / crate::audio_player::AUDIO_RATE as f64;
            let ph_secs = tl.playhead_frame as f64 / tl.fps as f64;
            self.stats.audio_err_frames.push(((audio_secs - ph_secs) * tl.fps as f64).abs());
        }
    }

    fn pct(v: &mut Vec<f64>, p: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() as f64 * p) as usize).min(v.len() - 1)]
    }

    fn report(&mut self, app: &NleApp, expect_frames: f64, strict_av: bool) {
        let secs = self.stage_start.elapsed().as_secs_f64();
        let mut iv = std::mem::take(&mut self.stats.intervals_ms);
        let avg = if iv.is_empty() { 0.0 } else { iv.iter().sum::<f64>() / iv.len() as f64 };
        let p99 = Self::pct(&mut iv, 0.99);
        let max = iv.last().copied().unwrap_or(0.0);
        let mut lags: Vec<f64> = self.stats.lags.iter().map(|x| *x as f64).collect();
        let lag_avg = if lags.is_empty() { 0.0 } else { lags.iter().sum::<f64>() / lags.len() as f64 };
        let lag_max = Self::pct(&mut lags, 1.0);
        let mut errs = std::mem::take(&mut self.stats.audio_err_frames);
        let err_p95 = Self::pct(&mut errs, 0.95);
        let vu_pct = if self.stats.vu_samples > 0 { 100.0 * self.stats.vu_nonzero as f64 / self.stats.vu_samples as f64 } else { 0.0 };
        let drops = app.video_player.dropped_frames_count - self.stats.drops_at_start.min(app.video_player.dropped_frames_count);
        println!(
            "[AUTOPILOT] {:?}: {:.1}s | UI frame ms avg {:.1} p99 {:.1} max {:.1} | video shown {} (expected ~{:.0}) | lag avg {:.1} max {:.0} src-frames | drops {} | audio non-silent {:.0}% | audio pos err p95 {:.1} fr",
            self.stage, secs, avg, p99, max, self.stats.shown_frames, expect_frames, lag_avg, lag_max, drops, vu_pct, err_p95
        );

        let shown_ratio = self.stats.shown_frames as f64 / expect_frames.max(1.0);
        if shown_ratio < 0.95 {
            self.fail(format!("{:?}: показано {:.0}% кадров (frame drop)", self.stage, shown_ratio * 100.0));
        }
        if max > 80.0 {
            self.fail(format!("{:?}: UI завис на {:.0} ms", self.stage, max));
        }
        if strict_av {
            if vu_pct < 5.0 {
                self.fail(format!("{:?}: звук отсутствует ({:.0}% кадров с сигналом)", self.stage, vu_pct));
            }
            if err_p95 > 8.0 {
                self.fail(format!("{:?}: звук рассинхронизован с плейхедом (p95 {:.1} кадров)", self.stage, err_p95));
            }
        }
    }

    /// Вызывается каждый кадр UI после шага воспроизведения и синхронизации звука
    pub fn tick(&mut self, app: &mut NleApp, ctx: &egui::Context) {
        ctx.request_repaint();
        self.sample(app);
        let elapsed = self.stage_start.elapsed();
        let fps = app.timeline.fps.max(1) as f64;

        match self.stage {
            Stage::Load => {
                if !self.load_requested {
                    println!("[AUTOPILOT] импорт {:?}", self.file);
                    app.timeline.pending_imports.push(self.file.clone());
                    self.load_requested = true;
                    return;
                }
                let have_clips = !app.timeline.clips.is_empty();
                let end_frame = app.timeline.timeline_end_frame();
                let min_samples = ((end_frame as f64 / fps) * crate::audio_player::AUDIO_RATE as f64 * 2.0 * 0.95) as usize;
                let audio_ready = app
                    .audio_player
                    .audio_cache
                    .lock()
                    .map(|c| {
                        c.len() >= 2
                            && c.values()
                                .all(|b| b.finished.load(std::sync::atomic::Ordering::Acquire) || b.len() >= min_samples)
                    })
                    .unwrap_or(false);
                if have_clips && audio_ready && app.video_player.texture.is_some() {
                    let ms = elapsed.as_secs_f64() * 1000.0;
                    self.load_audio_ready_ms = Some(ms);
                    println!(
                        "[AUTOPILOT] импорт завершён за {:.0} ms: клипов {}, конец таймлайна {} кадров ({:.2} c)",
                        ms,
                        app.timeline.clips.len(),
                        app.timeline.timeline_end_frame(),
                        app.timeline.timeline_end_frame() as f64 / fps
                    );
                    app.timeline.playhead_frame = 0;
                    app.timeline.is_playing = true;
                    app.timeline.frame_accumulator = 0.0;
                    app.last_frame_time = Instant::now();
                    self.begin_stage(app, Stage::PlayFromStart);
                } else if elapsed > Duration::from_secs(60) {
                    self.fail("Load: за 60 с не завершились импорт/декодирование звука".into());
                    self.stage = Stage::Done;
                }
            }
            Stage::PlayFromStart => {
                if elapsed >= Duration::from_secs(8) {
                    app.timeline.is_playing = false;
                    self.report(app, 8.0 * fps, true);
                    let ph = app.timeline.playhead_frame;
                    if (ph as f64 - 8.0 * fps).abs() > fps * 0.5 {
                        self.fail(format!("PlayFromStart: плейхед {} кадров вместо ~{} (slow-mo?)", ph, (8.0 * fps) as i64));
                    }
                    self.begin_stage(app, Stage::RazorAndMove);
                }
            }
            Stage::RazorAndMove => {
                // Пошагово: два разреза (Razor), затем «перетаскивание» правого сегмента мышью кадр за кадром
                match self.razor_step {
                    0 => {
                        for cut in [180i64, 420] {
                            app.timeline.playhead_frame = cut;
                            app.timeline.blade_all_at_playhead();
                        }
                        app.timeline.playhead_frame = 100;
                        let cutter = app.timeline.clips.iter().filter(|c| !c.is_audio).count();
                        println!("[AUTOPILOT] разрезов выполнено, видеосегментов: {}", cutter);
                        if cutter != 3 {
                            self.fail(format!("Razor: ожидалось 3 видеосегмента, получено {}", cutter));
                        }
                        let mid = app.timeline.clips.iter().find(|c| !c.is_audio && c.start_frame == 180).map(|c| c.id);
                        self.moved_clip = mid;
                        self.move_origin = 180;
                        app.timeline.push_history();
                        self.razor_step = 1;
                    }
                    1 => {
                        // Перемещаем средний сегмент на V2 (track 0), чтобы при сдвиге на +360 кадров
                        // образовалась пауза 180..420 и оверлей V2 поверх V1 без коллизии на одной дорожке
                        let n = (elapsed.as_millis() / 8) as i64; // шаг ~каждые 8 мс
                        let shift = n.min(360);
                        if let Some(id) = self.moved_clip {
                            app.timeline.move_clip_to_track(id, self.move_origin + shift, Some(0));
                        }
                        if shift >= 360 {
                            self.razor_step = 2;
                        }
                    }
                    _ => {
                        self.report(app, 0.0, false);
                        app.timeline.playhead_frame = 100;
                        app.timeline.is_playing = false;
                        app.video_player.wait_for_frame(100, ctx, Duration::from_millis(500));
                        app.timeline.is_playing = true;
                        app.timeline.frame_accumulator = 0.0;
                        app.last_frame_time = Instant::now();
                        self.begin_stage(app, Stage::PlayAcrossCuts);
                    }
                }
            }
            Stage::PlayAcrossCuts => {
                if elapsed >= Duration::from_secs(10) {
                    app.timeline.is_playing = false;
                    // За 10 с (600 кадров таймлайна 100..700) с паузой в 180..420 (240 кадров пустоты)
                    // и 2 монтажными склейками внутри 2-секундных GOP ожидается ~310-360 реальных видеокадров
                    self.report(app, 320.0, false);
                    let end = app.timeline.timeline_end_frame();
                    let target_start = (end - (4.0 * fps) as i64).max(0);
                    app.timeline.playhead_frame = target_start;
                    app.video_player.update_playback(target_start, false, ctx);
                    app.video_player.wait_for_frame(target_start, ctx, Duration::from_millis(2500));
                    self.begin_stage(app, Stage::PlayToEnd);
                    app.timeline.is_playing = true;
                    app.timeline.frame_accumulator = 0.0;
                    app.last_frame_time = Instant::now();
                }
            }
            Stage::PlayToEnd => {
                let stopped = !app.timeline.is_playing;
                if stopped || elapsed >= Duration::from_secs(9) {
                    let end = app.timeline.timeline_end_frame();
                    println!(
                        "[AUTOPILOT] конец: плейхед {} / конец таймлайна {}, playing={}, макс. показанный видеокадр {}",
                        app.timeline.playhead_frame, end, app.timeline.is_playing, self.stats.max_video_frame
                    );
                    if !stopped {
                        self.fail("PlayToEnd: воспроизведение не остановилось в конце таймлайна".into());
                    }
                    let tail_gap = self.stats.last_vu_nonzero_at.map(|t| Instant::now().duration_since(t).as_secs_f64());
                    println!("[AUTOPILOT] последний ненулевой звук был {:?} с назад", tail_gap);
                    app.timeline.is_playing = false;
                    self.report(app, (end - (end - (4.0 * fps) as i64).max(0)) as f64, true);
                    self.stage = Stage::Done;
                }
            }
            Stage::Done => {
                println!("[AUTOPILOT] ==== ИТОГ: {} ошибок ====", self.failures.len());
                for f in &self.failures {
                    println!("  - {}", f);
                }
                std::process::exit(if self.failures.is_empty() { 0 } else { 1 });
            }
        }
    }
}
