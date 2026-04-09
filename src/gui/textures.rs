//! Image → egui texture conversion, plus composite building.
//!
//! egui expects `egui::ColorImage` buffers for its textures. Our engine
//! works in `image::GrayImage` / `image::RgbImage`. This file handles
//! the conversion and owns the cached textures so the preview panel
//! doesn't re-upload every frame.

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use image::{GrayImage, RgbImage};

use inkplate::engine::color::Rgb;

use super::state::{GuiState, LayerEntry, PreviewMode};

#[derive(Default)]
pub struct TextureCache {
    pub composite: Option<TextureHandle>,
    pub source: Option<TextureHandle>,
}

impl TextureCache {
    /// Return the texture that should be displayed in the center
    /// panel, rebuilding whichever one is stale. Clears
    /// `state.composite_dirty` after a successful composite rebuild so
    /// we don't re-upload on every frame.
    pub fn current<'a>(
        &'a mut self,
        ctx: &egui::Context,
        state: &mut GuiState,
    ) -> Option<&'a TextureHandle> {
        match state.preview_mode {
            PreviewMode::Composite => {
                if state.composite_dirty || self.composite.is_none() {
                    let img = build_composite(state);
                    self.composite =
                        Some(ctx.load_texture("composite", img, TextureOptions::NEAREST));
                    state.composite_dirty = false;
                }
                self.composite.as_ref()
            }
            PreviewMode::Source => {
                if self.source.is_none() {
                    if let Some(src) = state.source.as_deref() {
                        let img = rgb_to_color_image(src);
                        self.source = Some(ctx.load_texture("source", img, TextureOptions::NEAREST));
                    }
                }
                self.source.as_ref()
            }
            PreviewMode::Layer(idx) => {
                // Single-layer previews rebuild every frame because they
                // change with selection — no need to cache keyed on index.
                let entry = state.layers.get(idx)?;
                let preview = entry.preview.as_ref()?;
                let rgb = mask_over_shirt(preview, entry.layer.ink, state.shirt_color);
                let img = rgb_to_color_image(&rgb);
                self.composite = Some(ctx.load_texture("layer", img, TextureOptions::NEAREST));
                self.composite.as_ref()
            }
        }
    }

    /// Force rebuild of the cached textures on the next `current()`.
    pub fn invalidate(&mut self) {
        self.composite = None;
        self.source = None;
    }
}

/// Rebuild the composite from the current layer list. Walks visible
/// layers in print order and blends each preview mask in the layer's
/// ink color over the shirt background.
pub fn build_composite(state: &GuiState) -> ColorImage {
    let dims = state
        .source
        .as_deref()
        .map(|img| img.dimensions())
        .unwrap_or((16, 16));
    let (w, h) = dims;

    let shirt = state.shirt_color;
    let mut rgb = RgbImage::from_pixel(w, h, image::Rgb([shirt.0, shirt.1, shirt.2]));

    // Sort by print_index ascending so back-to-front matches the press.
    let mut order: Vec<&LayerEntry> = state
        .layers
        .iter()
        .filter(|e| e.layer.visible && e.layer.include_in_export)
        .collect();
    order.sort_by_key(|e| e.layer.print_index);

    for entry in order {
        let Some(mask) = entry.preview.as_ref() else {
            continue;
        };
        if mask.dimensions() != (w, h) {
            continue;
        }
        composite_mask_over(&mut rgb, mask, entry.layer.ink);
    }

    rgb_to_color_image(&rgb)
}

/// Paint `ink` through `mask` onto `dst`. Density convention: 0 = full
/// ink, 255 = no ink. `alpha = (255 - mask) / 255`.
fn composite_mask_over(dst: &mut RgbImage, mask: &GrayImage, ink: Rgb) {
    let (w, h) = dst.dimensions();
    if mask.dimensions() != (w, h) {
        return;
    }
    for (x, y, p) in dst.enumerate_pixels_mut() {
        let m = mask.get_pixel(x, y)[0] as f32;
        let alpha = (1.0 - m / 255.0).clamp(0.0, 1.0);
        let inv = 1.0 - alpha;
        p[0] = (p[0] as f32 * inv + ink.0 as f32 * alpha)
            .round()
            .clamp(0.0, 255.0) as u8;
        p[1] = (p[1] as f32 * inv + ink.1 as f32 * alpha)
            .round()
            .clamp(0.0, 255.0) as u8;
        p[2] = (p[2] as f32 * inv + ink.2 as f32 * alpha)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
}

/// Build a single-layer preview: just the ink color blended over the
/// shirt through the mask.
fn mask_over_shirt(mask: &GrayImage, ink: Rgb, shirt: Rgb) -> RgbImage {
    let (w, h) = mask.dimensions();
    let mut rgb = RgbImage::from_pixel(w, h, image::Rgb([shirt.0, shirt.1, shirt.2]));
    composite_mask_over(&mut rgb, mask, ink);
    rgb
}

/// `image::RgbImage` → `egui::ColorImage`. Unavoidable copy because
/// egui wants a `Vec<Color32>` rather than an interleaved `u8` slice.
pub fn rgb_to_color_image(img: &RgbImage) -> ColorImage {
    let (w, h) = img.dimensions();
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for p in img.pixels() {
        pixels.push(egui::Color32::from_rgb(p[0], p[1], p[2]));
    }
    ColorImage {
        size: [w as usize, h as usize],
        pixels,
    }
}
