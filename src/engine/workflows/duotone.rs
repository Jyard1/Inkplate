//! Duotone workflow — two complementary ink layers for vintage poster
//! and risograph looks.
//!
//! Light tone via `gcr_black` on the *inverted* source (so bright
//! pixels get the light ink) and dark tone via `gcr_black` on the
//! direct source. Both halftoned on different angles.

use image::RgbImage;

use crate::engine::color::Rgb;
use crate::engine::halftone::{DotShape, HalftoneCurve};
use crate::engine::layer::{Extractor, HalftoneOverrides, Layer, LayerKind, RenderMode, Tone};
use crate::engine::workflows::curves;

pub fn build(_source: &RgbImage, light_ink: Rgb, dark_ink: Rgb) -> Vec<Layer> {
    let mut light = Layer::new_spot(light_ink);
    light.name = "light tone".into();
    light.kind = LayerKind::Color;
    light.extractor = Extractor::GcrBlack {
        strength: 1.0,
        invert_input: true,
    };
    light.tone = Tone {
        curve: curves::HIGHLIGHT_WHITE.to_vec(),
        density: 1.0,
        choke: 0,
    };
    light.render_mode = RenderMode::Halftone;
    light.halftone = HalftoneOverrides {
        lpi: 0,
        angle_deg: 15.0,
        dot_shape: Some(DotShape::Round),
        curve: HalftoneCurve::Linear,
    };
    light.print_index = 0;

    let mut dark = Layer::new_spot(dark_ink);
    dark.name = "dark tone".into();
    dark.kind = LayerKind::Color;
    dark.extractor = Extractor::GcrBlack {
        strength: 1.0,
        invert_input: false,
    };
    dark.tone = Tone {
        curve: curves::BLACK_PLATE.to_vec(),
        density: 1.0,
        choke: 0,
    };
    dark.render_mode = RenderMode::Halftone;
    dark.halftone = HalftoneOverrides {
        lpi: 0,
        angle_deg: 75.0,
        dot_shape: Some(DotShape::Round),
        curve: HalftoneCurve::Linear,
    };
    dark.print_index = 1;

    vec![light, dark]
}
