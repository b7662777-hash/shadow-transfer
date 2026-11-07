#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod storage;
mod theme;

fn main() -> eframe::Result<()> {
    init_logging();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1120.0, 760.0])
        .with_min_inner_size([900.0, 600.0])
        .with_title("Shadow Transfer");
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    if std::env::var_os("SHADOW_UI_TOPMOST").is_some() {
        viewport = viewport.with_window_level(eframe::egui::viewport::WindowLevel::AlwaysOnTop);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Shadow Transfer",
        options,
        Box::new(|cc| Ok(Box::new(app::RootApp::new(cc)))),
    )
}

fn init_logging() {
    if let Some(dir) = dirs::data_local_dir().map(|dir| dir.join("ShadowTransfer")) {
        if std::fs::create_dir_all(&dir).is_ok() {
            let appender = tracing_appender::rolling::daily(&dir, "shadow.log");
            let initialized = tracing_subscriber::fmt()
                .with_writer(appender)
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .with_target(false)
                .try_init();
            if initialized.is_ok() {
                return;
            }
        }
    }
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();
}

fn window_icon() -> Option<eframe::egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(eframe::egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    })
}
