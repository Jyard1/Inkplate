//! Simulated-process workflows — light shirt and dark shirt variants.
//!
//! Sim-process is the right approach for photoreal / shaded / painted
//! artwork where no single pixel "belongs" to one ink. Each layer is a
//! continuous-tone density map that gets halftoned at output time, and
//! the inks blend optically on the shirt.
//!
//! Two presets:
//!
//! - **light shirt** — no underbase. Color channels, black plate,
//!   highlight white. Light shirts already provide the white behind
//!   the colors so no white layer underneath is needed.
//! - **dark shirt** — HSB-brightness-inverted underbase first, then
//!   color channels, then black plate, then highlight white near the
//!   end. The underbase is what makes color inks show up on a dark
//!   shirt in the first place.
//!
//! Print order follows the pro recipe from the rebuild plan:
//! underbase → light colors → mid colors → dark colors → black plate →
//! highlight white.

use image::RgbImage;

use crate::engine::color::{color_name, Rgb};
use crate::engine::halftone::{DotShape, HalftoneCurve};
use crate::engine::layer::{
    ColorRangeFalloff, Extractor, HalftoneOverrides, Layer, LayerKind, MaskShape, RenderMode, Tone,
};
use crate::engine::palette::{auto_palette, consolidate_by_hue, HueOpts, PaletteOpts};
use crate::engine::workflows::curves;

#[derive(Debug, Clone, Copy)]
pub struct SimOpts {
    pub max_colors: usize,
    pub fuzziness: f32,
    /// Collapse same-hue shades onto a single halftone channel.
    /// Off by default so cranking `max_colors` actually produces
    /// more distinct plates.
    pub consolidate_hues: bool,
}

impl Default for SimOpts {
    fn default() -> Self {
        Self {
            // 10 color channels plus underbase + black + highlight
            // white is a realistic upper bound for sim-process jobs
            // without turning into a registration nightmare.
            max_colors: 10,
            // With CIE94-based color_range the fuzziness scale is
            // different (CIE94 distances are smaller), so nudge this
            // up slightly to keep selections looking the same.
            fuzziness: 40.0,
            consolidate_hues: false,
        }
    }
}

/// Sim-process for a dark shirt. Builds:
/// underbase → N color channels → black plate → highlight white.
pub fn build_dark(source: &RgbImage, opts: SimOpts) -> Vec<Layer> {
    let mut layers = Vec::new();

    layers.push(underbase_layer());

    let color_layers = color_channel_layers(source, opts);
    layers.extend(color_layers);

    layers.push(black_plate_layer());
    layers.push(highlight_white_layer());

    // Stamp final print indices so per-layer halftone angles cycle
    // through the auto angle table in the right order.
    reindex(&mut layers);
    layers
}

/// Sim-process for a light shirt. Same as dark but without the
/// underbase (the shirt color already reads through the color inks).
pub fn build_light(source: &RgbImage, opts: SimOpts) -> Vec<Layer> {
    let mut layers = Vec::new();

    let color_layers = color_channel_layers(source, opts);
    layers.extend(color_layers);

    layers.push(black_plate_layer());
    layers.push(highlight_white_layer());

    reindex(&mut layers);
    layers
}

fn color_channel_layers(source: &RgbImage, opts: SimOpts) -> Vec<Layer> {
    let pixels = source.as_raw();
    let (mut palette, _quant) = auto_palette(
        pixels,
        PaletteOpts {
            max_colors: opts.max_colors,
            merge_delta_e: 10.0,
            // Accent colors matter in sim-process too — a 0.3%
            // saturated spot can be the visual focus of the art.
            min_coverage: 0.003,
        },
    );
    if opts.consolidate_hues {
        palette = consolidate_by_hue(&palette, HueOpts::default());
    }

    // Drop near-grayscale entries — those belong to the black plate /
    // highlight white, not to a dedicated color screen.
    palette.retain(|e| {
        let lab = crate::engine::color::rgb_to_lab(e.rgb);
        lab.chroma() > 8.0
    });

    // Sort light → dark by LAB L so the print order goes light-colors
    // first, dark-colors last (matching the pro recipe).
    palette.sort_by(|a, b| {
        let la = crate::engine::color::rgb_to_lab(a.rgb).l;
        let lb = crate::engine::color::rgb_to_lab(b.rgb).l;
        lb.partial_cmp(&la).unwrap()
    });

    let mut out = Vec::with_capacity(palette.len());
    for entry in palette {
        let mut layer = Layer::new_spot(entry.rgb);
        layer.name = format!("color {}", color_name(entry.rgb));
        layer.kind = LayerKind::Color;
        layer.extractor = Extractor::ColorRange {
            target: entry.rgb,
            fuzziness: opts.fuzziness,
            falloff: ColorRangeFalloff::Smooth,
        };
        layer.tone = Tone {
            curve: curves::SIM_PROCESS.to_vec(),
            density: 1.0,
            choke: 0,
        };
        layer.render_mode = RenderMode::Halftone;
        layer.halftone = HalftoneOverrides {
            lpi: 0,
            angle_deg: -1.0, // auto-cycle from print index
            dot_shape: Some(DotShape::Round),
            curve: HalftoneCurve::Linear,
        };
        out.push(layer);
    }
    out
}

fn underbase_layer() -> Layer {
    let mut layer = Layer::new_spot(Rgb::WHITE);
    layer.name = "underbase".into();
    layer.kind = LayerKind::Underbase;
    layer.extractor = Extractor::HsbBrightnessInverted {
        s_curve: 1.6,
        boost_under_darks: true,
        boost_strength: 0.4,
    };
    layer.tone = Tone {
        curve: curves::UNDERBASE.to_vec(),
        density: 1.0,
        choke: 2, // 2-pixel choke so it doesn't peek out from color screens
    };
    layer.render_mode = RenderMode::Solid;
    layer.mask = MaskShape {
        smooth_radius: 1,
        ..MaskShape::default()
    };
    layer
}

fn black_plate_layer() -> Layer {
    let mut layer = Layer::new_spot(Rgb::BLACK);
    layer.name = "black plate".into();
    layer.kind = LayerKind::Color;
    layer.extractor = Extractor::GcrBlack {
        strength: 1.0,
        invert_input: false,
    };
    layer.tone = Tone {
        curve: curves::BLACK_PLATE.to_vec(),
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

fn highlight_white_layer() -> Layer {
    let mut layer = Layer::new_spot(Rgb::WHITE);
    layer.name = "highlight white".into();
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
