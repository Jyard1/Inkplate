//! Tritone workflow — three tone layers (highlight, mid, shadow)
//! each built from a luminance band of the source.
//!
//! Each layer uses `channel_calc` with an expression that isolates a
//! band of the L channel. The bands overlap slightly so transitions
//! between inks are smooth.

use image::RgbImage;

use crate::engine::color::Rgb;
use crate::engine::halftone::{DotShape, HalftoneCurve};
use crate::engine::layer::{Extractor, HalftoneOverrides, Layer, LayerKind, RenderMode};

pub fn build(_source: &RgbImage, highlight_ink: Rgb, mid_ink: Rgb, shadow_ink: Rgb) -> Vec<Layer> {
    vec![
        // Highlight: L > 0.7 region, ramped up to full at L=1.
        tone_layer(
            "highlight",
            highlight_ink,
            "clip((L - 0.7) * 3.3, 0, 1)",
            15.0,
            0,
        ),
        // Mid: peak at L=0.5, fades off each side.
        tone_layer(
            "mid",
            mid_ink,
            "clip(1 - abs(L - 0.5) * 3.3, 0, 1)",
            75.0,
            1,
        ),
        // Shadow: L < 0.3, ramped up to full at L=0.
        tone_layer("shadow", shadow_ink, "clip((0.3 - L) * 3.3, 0, 1)", 45.0, 2),
    ]
}

fn tone_layer(name: &str, ink: Rgb, expr: &str, angle: f32, print_index: u32) -> Layer {
    let mut layer = Layer::new_spot(ink);
    layer.name = name.to_string();
    layer.kind = LayerKind::Color;
    layer.extractor = Extractor::ChannelCalc {
        expr: expr.to_string(),
    };
    layer.render_mode = RenderMode::Halftone;
    layer.halftone = HalftoneOverrides {
        lpi: 0,
        angle_deg: angle,
        dot_shape: Some(DotShape::Round),
        curve: HalftoneCurve::Linear,
    };
    layer.print_index = print_index;
    layer
}
