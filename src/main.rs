//! Inkplate entry point.
//!
//! Launches the egui desktop app. For headless use (CLI batch processing,
//! tests, scripts) depend on the `inkplate` library crate directly instead
//! of invoking this binary.

mod gui;

fn main() -> eframe::Result<()> {
    gui::run()
}
