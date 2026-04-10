//! True CMYK sim-process workflow.
//!
//! Decomposes the source image into four process-colour plates (Cyan,
//! Magenta, Yellow, Black) via mathematically exact sRGB → linear →
//! CMY conversion with Grey Component Replacement. Each plate renders
//! as a halftone screen at the standard CMYK screen angles (C=15°,
//! M=75°, Y=0°, K=45°) to minimise moiré.
//!
//! Two presets:
//!
//! - **dark shirt**: underbase (white) → C → M → Y → K → highlight white
//! - **light shirt**: C → M → Y → K → highlight white (no underbase — the
//!   shirt fabric itself provides the white)

use image::RgbImage;

use crate::engine::color::Rgb;
use crate::engine::halftone::{DotShape, HalftoneCurve};
use crate::engine::layer::{
    CmykProcess, Extractor, HalftoneOverrides, Layer, LayerKind, MaskShape, RenderMode, Tone,
};
use crate::engine::workflows::curves;

#[derive(Debug, Clone, Copy)]
pub struct CmykOpts {
    /// Grey Component Replacement strength. 0.0 = no GCR (all colour
    /// stays in CMY, K plate is blank), 1.0 = full GCR (max neutral
    /// removed, K carries all the darkness). 0.75 is a good starting
    /// point for screen printing on textiles.
    pub gcr_strength: f32,
}

impl Default for CmykOpts {
    fn default() -> Self {
        Self {
            gcr_strength: 0.75,
        }
    }
}

/// Dark-shirt CMYK: underbase → C → M → Y → K → highlight white.
pub fn build_dark(_source: &RgbImage, opts: CmykOpts) -> Vec<Layer> {
    let mut layers = Vec::with_capacity(6);
    layers.push(underbase_layer());
    layers.extend(cmyk_layers(opts));
    layers.push(highlight_white_layer());
    reindex(&mut layers);
    layers
}

/// Light-shirt CMYK: C → M → Y → K → highlight white.
pub fn build_light(_source: &RgbImage, opts: CmykOpts) -> Vec<Layer> {
    let mut layers = Vec::with_capacity(5);
    layers.extend(cmyk_layers(opts));
    layers.push(highlight_white_layer());
    reindex(&mut layers);
    layers
}

fn cmyk_layers(opts: CmykOpts) -> Vec<Layer> {
    let channels = [
        (CmykProcess::Cyan, "cyan", Rgb(0, 200, 255), 15.0),
        (CmykProcess::Magenta, "magenta", Rgb(220, 0, 180), 75.0),
        (CmykProcess::Yellow, "yellow", Rgb(255, 230, 0), 0.0),
        (CmykProcess::Black, "black", Rgb(0, 0, 0), 45.0),
    ];

    channels
        .into_iter()
        .map(|(ch, name, ink, angle)| {
            let mut layer = Layer::new_spot(ink);
            layer.name = name.into();
            layer.kind = LayerKind::Color;
            layer.extractor = Extractor::CmykChannel {
                channel: ch,
                gcr_strength: opts.gcr_strength,
            };
            layer.tone = Tone {
                curve: curves::CMYK_CHANNEL.to_vec(),
                density: 1.0,
                choke: 0,
            };
            layer.render_mode = RenderMode::Halftone;
            layer.halftone = HalftoneOverrides {
                lpi: 0,
                angle_deg: angle,
                dot_shape: Some(DotShape::Round),
                curve: HalftoneCurve::Linear,
            };
            layer.mask = MaskShape::default();
            layer
        })
        .collect()
}

fn underbase_layer() -> Layer {
    let mut layer = Layer::new_spot(Rgb::WHITE);
    layer.name = "white underbase".into();
    layer.kind = LayerKind::Underbase;
    // Derive the underbase from the composite of all non-black color
    // layers. The worker/export resolves this in a two-pass loop:
    // process color layers first, union their previews, then feed the
    // union through this layer's pipeline (curve → blur → binarize →
    // choke → morphology). The result is a clean binary plate that
    // exactly covers where color ink lands.
    layer.extractor = Extractor::CompositeUnion;
    layer.tone = Tone {
        curve: curves::UNDERBASE_COMPOSITE.to_vec(),
        density: 1.0,
        choke: 2,
    };
    layer.render_mode = RenderMode::Solid;
    layer.mask = MaskShape {
        noise_open: 2,  // kill stray ink dots outside the boundary
        holes_close: 2, // fill pinholes inside the solid underbase
        ..MaskShape::default()
    };
    layer
}

fn highlight_white_layer() -> Layer {
    let mut layer = Layer::new_spot(Rgb::WHITE);
    layer.name = "white highlight".into();
    layer.kind = LayerKind::Highlight;
    layer.extractor = Extractor::GcrBlack {
        strength: 1.0,
        invert_input: true,
    };
    layer.tone = Tone {
        curve: curves::HIGHLIGHT_WHITE.to_vec(),
        density: 1.0,
        choke: 0,
    };
    layer.render_mode = RenderMode::Halftone;
    layer.halftone = HalftoneOverrides {
        lpi: 0,
        angle_deg: -1.0,
        dot_shape: Some(DotShape::Round),
        curve: HalftoneCurve::Linear,
    };
    layer
}

fn reindex(layers: &mut [Layer]) {
    for (i, l) in layers.iter_mut().enumerate() {
        l.print_index = i as u32;
    }
}
