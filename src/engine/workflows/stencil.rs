//! Stencil workflow — one binary layer, user-picks-the-threshold.
//!
//! This is the "high-contrast silhouette" workflow — think Obama Hope
//! posters, single-color silhouette tees, one-color distressed art.
//! Binarizes the source at a fixed luminance cutoff and prints the
//! result solid.

use image::RgbImage;

use crate::engine::color::Rgb;
use crate::engine::layer::{Extractor, Layer, LayerKind, RenderMode};

pub fn build(_source: &RgbImage, threshold: u8) -> Vec<Layer> {
    let mut layer = Layer::new_spot(Rgb::BLACK);
    layer.name = "stencil".into();
    layer.kind = LayerKind::Color;
    layer.extractor = Extractor::LuminanceThreshold {
        threshold,
        above: false,
    };
    layer.render_mode = RenderMode::Solid;
    layer.print_index = 0;
    vec![layer]
}
