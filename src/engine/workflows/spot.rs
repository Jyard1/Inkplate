//! Spot-color workflow — one solid screen per auto-detected palette
//! color.
//!
//! This is the workflow for flat vector art: logos, icons, text. The
//! pipeline auto-palettes the source image down to at most `max_colors`
//! distinct colors, runs the hue-family consolidation pass so dark red
//! and bright red end up on the same screen, then emits one layer per
//! surviving palette entry.
//!
//! Each layer uses the `spot_aa` extractor so anti-aliased edges get
//! graded (not snapped), and renders in `Solid` mode since there's no
//! halftone step for flat vector.

use image::RgbImage;

use crate::engine::color::color_name;
use crate::engine::layer::{Extractor, Layer, LayerKind, MaskShape, RenderMode, Tone};
use crate::engine::palette::{auto_palette, consolidate_by_hue, HueOpts, PaletteOpts};

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
            max_colors: 12,
            merge_delta_e: 8.0,
            min_coverage: 0.005,
            consolidate_hues: true,
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

    if opts.consolidate_hues {
        palette = consolidate_by_hue(&palette, HueOpts::default());
    }

    // Collect every palette color — spot_aa needs the full list as
    // "targets" so it can do Voronoi assignment against all of them.
    let all_targets: Vec<_> = palette.iter().map(|e| e.rgb).collect();
    let empty_others: Vec<_> = Vec::new();

    let mut layers = Vec::with_capacity(palette.len());
    for (idx, entry) in palette.iter().enumerate() {
        let mut layer = Layer::new_spot(entry.rgb);
        layer.name = format!("{:02} {}", idx + 1, color_name(entry.rgb));
        layer.kind = LayerKind::Spot;
        layer.print_index = idx as u32;
        layer.extractor = Extractor::SpotAa {
            targets: all_targets.clone(),
            others: empty_others.clone(),
            aa_full: 4.0,
            aa_end: 14.0,
            aa_reach: 2,
        };
        layer.render_mode = RenderMode::Solid;
        layer.tone = Tone::default();
        layer.mask = MaskShape {
            smooth_radius: opts.smooth_radius,
            ..MaskShape::default()
        };
        layers.push(layer);
    }
    layers
}
