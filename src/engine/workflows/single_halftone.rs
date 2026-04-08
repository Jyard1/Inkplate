//! Single-halftone workflow — one B&W channel, for grayscale art.
//!
//! Covers manga panels, woodcut illustration, B&W comic work, line art
//! with tones, and similar grayscale sources. One layer, one screen,
//! one ink — usually black on a light shirt.
//!
//! The extractor is `channel_calc` with expression `1 - L` so the
//! grayscale lightness is inverted into ink amount. Render mode is
//! Halftone at slightly coarser LPI (45) because detail loss is more
//! visible on single-screen jobs.

use image::RgbImage;

use crate::engine::color::Rgb;
use crate::engine::halftone::{DotShape, HalftoneCurve};
use crate::engine::layer::{Extractor, HalftoneOverrides, Layer, LayerKind, RenderMode};

pub fn build(_source: &RgbImage) -> Vec<Layer> {
    let mut layer = Layer::new_spot(Rgb::BLACK);
    layer.name = "black halftone".into();
    layer.kind = LayerKind::Color;
    layer.extractor = Extractor::ChannelCalc {
        expr: "1 - L".to_string(),
    };
    layer.render_mode = RenderMode::Halftone;
    layer.halftone = HalftoneOverrides {
        lpi: 45, // coarser: single-screen jobs show detail loss more
        angle_deg: 22.5,
        dot_shape: Some(DotShape::Round),
        curve: HalftoneCurve::Linear,
    };
    layer.print_index = 0;
    vec![layer]
}
