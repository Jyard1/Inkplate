//! Index separation workflows — FS and Bayer dither variants.
//!
//! Index separation assigns every pixel to exactly one of N palette
//! colors via a dither pattern. No halftone — every "dot" is one
//! source pixel, placed pseudo-randomly to approximate the color. This
//! avoids moiré entirely and is the right tool for:
//!
//! - pixel art (integer-aligned hard edges, small color count)
//! - sharp-edged illustration with limited palette
//! - anything where halftone dots would destroy the character of the art
//!
//! Each palette entry becomes one layer using the `index_assignment`
//! extractor, which internally runs the dither once and returns only
//! the pixels assigned to that entry's index.

use image::RgbImage;

use crate::engine::color::color_name;
use crate::engine::layer::{Extractor, IndexDitherKind, Layer, LayerKind, RenderMode};
use crate::engine::palette::{auto_palette, snap_extremes, PaletteOpts};

#[derive(Debug, Clone, Copy)]
pub struct IndexOpts {
    pub max_colors: usize,
}

impl Default for IndexOpts {
    fn default() -> Self {
        // 12 is the sweet spot for index separation: enough hue
        // diversity for most pixel art and limited-palette
        // illustration, few enough that each plate has legible
        // coverage after dithering.
        Self { max_colors: 12 }
    }
}

pub fn build_fs(source: &RgbImage, opts: IndexOpts) -> Vec<Layer> {
    build(source, opts, IndexDitherKind::Fs)
}

pub fn build_bayer(source: &RgbImage, opts: IndexOpts) -> Vec<Layer> {
    build(source, opts, IndexDitherKind::Bayer)
}

fn build(source: &RgbImage, opts: IndexOpts, kind: IndexDitherKind) -> Vec<Layer> {
    let pixels = source.as_raw();
    let (mut palette, _quant) = auto_palette(
        pixels,
        PaletteOpts {
            max_colors: opts.max_colors,
            merge_delta_e: 6.0,
            min_coverage: 0.002,
        },
    );
    snap_extremes(&mut palette);

    let rgb_palette: Vec<_> = palette.iter().map(|e| e.rgb).collect();

    let mut layers = Vec::with_capacity(rgb_palette.len());
    for (idx, entry) in palette.iter().enumerate() {
        let mut layer = Layer::new_spot(entry.rgb);
        layer.name = format!("{:02} {}", idx + 1, color_name(entry.rgb));
        layer.kind = LayerKind::Color;
        layer.extractor = Extractor::IndexAssignment {
            palette: rgb_palette.clone(),
            index: idx as u32,
            dither: kind,
        };
        // Index layers are already binary — render as solid, no
        // halftone needed.
        layer.render_mode = RenderMode::Solid;
        layer.print_index = idx as u32;
        layers.push(layer);
    }
    layers
}
