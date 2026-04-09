//! egui desktop GUI.
//!
//! Four-panel layout (top bar, left layer list, center preview, right
//! inspector) plus per-extractor slider forms for every parameter. See
//! each submodule for details.
//!
//! TODO(L5-later): background processing worker, curve editor widget,
//! preview zoom/pan, undo/redo, menu/hotkeys.

use eframe::egui;

mod app;
mod history;
mod panels;
mod processing;
mod state;
mod textures;
mod widgets;
mod worker;

pub use app::InkplateApp;

/// Entry point called from `main.rs`.
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([1000.0, 700.0])
            .with_title("Inkplate"),
        ..Default::default()
    };
    eframe::run_native(
        "Inkplate",
        options,
        Box::new(|cc| Ok(Box::new(InkplateApp::new(cc.egui_ctx.clone())))),
    )
}
