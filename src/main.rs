mod app;
mod autopilot;
mod audio_player;
mod inspector;
mod media_page;
mod media_pool;
mod preferences;
mod theme;
mod timecode;
mod timeline;
mod timeline_view;
mod video_decoder;
mod viewer;
mod waveform;

#[cfg(test)]
mod perf_tests;

use app::NleApp;
use eframe::egui;

fn main() -> Result<(), eframe::Error> {
    // Глобальная инициализация кодеков и контейнеров FFmpeg
    let _ = ffmpeg_next::init();
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Fatal);

    // Инициализация многопоточного рантайма Tokio для высокопроизводительных фоновых задач
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to initialize Tokio runtime");
    let _guard = rt.enter();

    // Чистый нативный Wayland бэкенд (никакого X11 / XWayland)
    std::env::set_var("WINIT_UNIX_BACKEND", "wayland");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Verizon NLE - DaVinci Resolve Edition (Zero-Lag)")
            .with_inner_size([1600.0, 950.0])
            .with_min_inner_size([1024.0, 600.0])
            .with_active(true)
            .with_transparent(false),
        // Нативный Wayland + Vulkan: минимальная задержка present (1 кадр в очереди) вместо HIGH_THROUGHPUT (2 кадра)
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            surface: eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY,
            ..Default::default()
        },
        ..Default::default()
    };

    eframe::run_native(
        "Verizon NLE",
        native_options,
        Box::new(|cc| Ok(Box::new(NleApp::new(cc)))),
    )
}
