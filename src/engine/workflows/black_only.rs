//! Black-only workflow — one solid black layer, no halftone.
//!
//! For pure line art and single-color silhouettes where halftoning
//! would just fuzz the edges. Uses the `luminance_threshold` extractor
//! with a mid threshold so anti-aliased edges snap cleanly.

use image::RgbImage;

use crate::engine::color::Rgb;
use crate::engine::layer::{Extractor, Layer, LayerKind, RenderMode};

pub fn build(_source: &RgbImage) -> Vec<Layer> {
    let mut layer = Layer::new_spot(Rgb::BLACK);
    layer.name = "black".into();
    layer.kind = LayerKind::Color;
    layer.extractor = Extractor::LuminanceThreshold {
        threshold: 128,
        above: false,
    };
    layer.render_mode = RenderMode::Solid;
    layer.print_index = 0;
    vec![layer]
}
