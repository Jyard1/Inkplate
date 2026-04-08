//! Center preview panel. Shows the composite (visible layers blended
//! over the shirt), the raw source, or one layer in isolation.
//!
//! Fit-to-panel only for v1 — zoom and pan live on the TODO list and
//! will land with the background worker so the GUI can stay responsive
//! during heavy reprocessing.

use eframe::egui::{self, Ui};

use crate::gui::state::{GuiState, PreviewMode};
use crate::gui::textures::TextureCache;

pub fn show(ui: &mut Ui, state: &mut GuiState, textures: &mut TextureCache) {
    ui.horizontal(|ui| {
        ui.heading("Preview");
        ui.separator();

        let mut mode = state.preview_mode;
        ui.selectable_value(&mut mode, PreviewMode::Composite, "Composite");
        ui.selectable_value(&mut mode, PreviewMode::Source, "Source");
        if let Some(idx) = state.selected {
            ui.selectable_value(&mut mode, PreviewMode::Layer(idx), "Layer");
        }
        if mode != state.preview_mode {
            state.preview_mode = mode;
            textures.invalidate();
        }
    });
    ui.separator();

    let avail = ui.available_size();

    if state.source.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("Open an image to begin")
                    .italics()
                    .size(18.0),
            );
        });
        return;
    }

    let texture = textures.current(ui.ctx(), state);
    if let Some(tex) = texture {
        let img_size = tex.size_vec2();
        // Fit to available space, preserving aspect ratio.
        let scale = (avail.x / img_size.x).min(avail.y / img_size.y).min(4.0);
        let display_size = img_size * scale;
        let sized = egui::load::SizedTexture::new(tex.id(), img_size);
        ui.add(egui::Image::new(sized).fit_to_exact_size(display_size));
    }
}
