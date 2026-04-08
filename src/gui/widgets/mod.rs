//! Shared widget helpers — the small atoms the panel modules reuse.

use eframe::egui::{self, Color32, Response, Ui};

use inkplate::engine::color::Rgb;

/// Slider with a label above it instead of to the side. Returns the
/// full response so callers can check `.changed()` and queue reruns.
pub fn labeled_slider_f32(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> Response {
    ui.label(label);
    ui.add(egui::Slider::new(value, range))
}

pub fn labeled_slider_u32(
    ui: &mut Ui,
    label: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
) -> Response {
    ui.label(label);
    ui.add(egui::Slider::new(value, range))
}

pub fn labeled_slider_u8(
    ui: &mut Ui,
    label: &str,
    value: &mut u8,
    range: std::ops::RangeInclusive<u8>,
) -> Response {
    ui.label(label);
    ui.add(egui::Slider::new(value, range))
}

/// Ink color picker button — shows the current swatch and opens an
/// egui color picker on click.
pub fn ink_picker(ui: &mut Ui, label: &str, rgb: &mut Rgb) -> Response {
    ui.label(label);
    let mut c = [rgb.0, rgb.1, rgb.2];
    let resp = ui.color_edit_button_srgb(&mut c);
    if resp.changed() {
        *rgb = Rgb(c[0], c[1], c[2]);
    }
    resp
}

/// Small square swatch showing an ink color, no interaction.
pub fn ink_swatch(ui: &mut Ui, rgb: Rgb, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 2.0, Color32::from_rgb(rgb.0, rgb.1, rgb.2));
    ui.painter()
        .rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::from_gray(60)));
    resp
}
