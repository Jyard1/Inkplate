//! Spot-color workflow — one solid screen per auto-detected palette
//! color.
//!
//! This is the workflow for flat vector art, logos, and cel-shaded
//! illustration. Pipeline:
//!
//! 1. [`auto_palette`] clusters the image in LAB space with CIE94
//!    distance, producing up to `max_colors` well-separated entries.
//! 2. [`snap_extremes`] forces the near-black entry to pure `#000`
//!    and the near-white entry to pure `#FFF` so the plates match
//!    the actual inks that will hit the shirt.
//! 3. If `consolidate_hues` is on, same-hue shades are merged into
//!    one entry via [`consolidate_by_hue`]. Spot workflow turns this
//!    on; cel-shaded turns it off so highlights / shadows keep their
//!    own plates.
//! 4. Sort the palette so black prints last (highest `print_index`)
//!    and white prints first — standard screen-print stacking.
//! 5. Emit one solid `spot_aa` layer per surviving entry.
//!
//! The [`spot_aa`](crate::engine::extractors::spot_aa) extractor
//! makes the per-pixel plate decision via CIE94 nearest-target with
//! lowest-index tie-breaking, producing hard-binary masks with no
//! double coverage.

use image::RgbImage;

use crate::engine::color::{color_name, rgb_to_lab, Rgb};
use crate::engine::layer::{Extractor, Layer, LayerKind, MaskShape, RenderMode, Tone};
use crate::engine::palette::{
    auto_palette, consolidate_by_hue, snap_extremes, HueOpts, PaletteEntry, PaletteOpts,
};

#[derive(Debug, Clone, Copy)]
pub struct SpotOpts {
    pub max_colors: usize,
    pub merge_delta_e: f32,
    pub min_coverage: f32,
    pub consolidate_hues: bool,
    pub smooth_radius: u32,
}

impl Default for SpotOpts {
    fn default() -> Self {
        Self {
            max_colors: 16,
            merge_delta_e: 8.0,
            min_coverage: 0.002,
            // Off by default — the new k-means palette already gives
            // well-separated clusters, and merging them after the
            // fact throws away colors the user explicitly asked for.
            // Users who want shadow+highlight of the same ink on one
            // plate can flip this in the GUI.
            consolidate_hues: false,
            smooth_radius: 0,
        }
    }
}

pub fn build(source: &RgbImage, opts: SpotOpts) -> Vec<Layer> {
    let pixels: &[u8] = source.as_raw();
    let (mut palette, _quantized) = auto_palette(
        pixels,
        PaletteOpts {
            max_colors: opts.max_colors,
            merge_delta_e: opts.merge_delta_e,
            min_coverage: opts.min_coverage,
        },
    );

    // Pure inks: near-black → #000, near-white → #FFF. Has to happen
    // before hue consolidation so consolidate_by_hue sees a real
    // black/white on the L-axis instead of a warm off-white that it
    // might accidentally group with a chromatic entry.
    snap_extremes(&mut palette);

    if opts.consolidate_hues {
        palette = consolidate_by_hue(&palette, HueOpts::default());
    }

    // Standard screen-print order: white first, coloured inks in the
    // middle, black last (prints on top so outlines are crisp).
    sort_for_print_order(&mut palette);

    // Collect every palette color — spot_aa needs the full list as
    // "targets" so it can do Voronoi assignment against all of them.
    let all_targets: Vec<_> = palette.iter().map(|e| e.rgb).collect();
    let empty_others: Vec<_> = Vec::new();

    let zero_weights: Vec<f32> = vec![0.0; all_targets.len()];

    let mut layers = Vec::with_capacity(palette.len());
    for (idx, entry) in palette.iter().enumerate() {
        let mut layer = Layer::new_spot(entry.rgb);
        layer.name = format!("{:02} {}", idx + 1, color_name(entry.rgb));
        layer.kind = LayerKind::Spot;
        layer.print_index = idx as u32;
        layer.extractor = Extractor::SpotAa {
            targets: all_targets.clone(),
            others: empty_others.clone(),
            // Legacy soft-ramp params; the current spot_aa is hard
            // Voronoi and ignores them. Kept so existing saved
            // projects deserialize without schema changes.
            aa_full: 1.5,
            aa_end: 14.0,
            aa_reach: 2,
            target_weights: zero_weights.clone(),
        };
        layer.render_mode = RenderMode::Solid;
        layer.tone = Tone::default();
        layer.mask = MaskShape {
            smooth_radius: opts.smooth_radius,
            // Gaussian blur before binarize. σ=3.5 spreads the mask
            // over ~7px (2σ), enough to smooth the staircase steps
            // that diagonal edges produce in the hard Voronoi output.
            // The binarize threshold then traces a clean diagonal
            // instead of pixel-aligned blocks.
            solid_blur: 3.5,
            ..MaskShape::default()
        };
        layers.push(layer);
    }
    layers
}

/// Order the palette the way a press would stack the screens: pure
/// white first (underbase / highlight), then chromatic inks sorted
/// light→dark by L*, then pure black last so outlines hit on top of
/// the color fills.
fn sort_for_print_order(palette: &mut [PaletteEntry]) {
    palette.sort_by(|a, b| {
        let score_a = print_order_score(a.rgb);
        let score_b = print_order_score(b.rgb);
        score_a
            .partial_cmp(&score_b)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Lower score = printed earlier (at the back of the stack). Pure
/// white gets the minimum, pure black the maximum; chromatics sit in
/// between sorted by lightness.
fn print_order_score(rgb: Rgb) -> f32 {
    if rgb == Rgb::WHITE {
        return -1.0;
    }
    if rgb == Rgb::BLACK {
        return 1000.0;
    }
    // Lighter colors print before darker ones. Use 100 − L* so higher
    // L* → smaller score → earlier in the stack.
    100.0 - rgb_to_lab(rgb).l
}
