//! Contact sheet — all visible layers in a grid, each cell showing the
//! layer's preview mask composited in the ink color over the shirt
//! background, with a caption underneath.
//!
//! Useful for client approvals and shop floor reference — one PNG that
//! says "here's all the screens and their order".

use image::{ImageBuffer, RgbImage};

use crate::engine::color::Rgb;
use crate::engine::layer::Layer;
use crate::engine::pipeline::{process_layer, JobOpts};

#[derive(Debug, Clone, Copy)]
pub struct ContactSheetOpts {
    pub columns: u32,
    pub cell_size: u32,
    pub padding: u32,
    pub shirt: Rgb,
}

impl Default for ContactSheetOpts {
    fn default() -> Self {
        Self {
            columns: 4,
            cell_size: 300,
            padding: 16,
            shirt: Rgb(24, 24, 28),
        }
    }
}

pub fn build(source: &RgbImage, layers: &[Layer], opts: &ContactSheetOpts) -> RgbImage {
    let visible: Vec<&Layer> = layers
        .iter()
        .filter(|l| l.visible && l.include_in_export)
        .collect();

    let cols = opts.columns.max(1);
    let rows = (visible.len() as u32).div_ceil(cols);

    let cell_w = opts.cell_size;
    let cell_h = opts.cell_size + 24; // extra for caption

    let total_w = cols * cell_w + (cols + 1) * opts.padding;
    let total_h = rows * cell_h + (rows + 1) * opts.padding;
    let mut out: RgbImage =
        ImageBuffer::from_pixel(total_w.max(1), total_h.max(1), image::Rgb([20, 20, 24]));

    let job = JobOpts::default();
    for (i, layer) in visible.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let x0 = opts.padding + col * (cell_w + opts.padding);
        let y0 = opts.padding + row * (cell_h + opts.padding);

        let processed = process_layer(source, layer, job, None);
        let cell = render_cell(
            &processed.preview,
            layer.ink,
            opts.shirt,
            cell_w,
            opts.cell_size,
        );

        // Paste cell into output.
        for (x, y, p) in cell.enumerate_pixels() {
            let ox = x0 + x;
            let oy = y0 + y;
            if ox < total_w && oy < total_h {
                out.put_pixel(ox, oy, *p);
            }
        }
    }
    out
}

fn render_cell(
    mask: &image::GrayImage,
    ink: Rgb,
    shirt: Rgb,
    target_w: u32,
    target_h: u32,
) -> RgbImage {
    let (mw, mh) = mask.dimensions();
    if mw == 0 || mh == 0 {
        return ImageBuffer::from_pixel(
            target_w,
            target_h,
            image::Rgb([shirt.0, shirt.1, shirt.2]),
        );
    }
    // Letterbox-fit the mask into target dimensions.
    let scale = (target_w as f32 / mw as f32).min(target_h as f32 / mh as f32);
    let fit_w = ((mw as f32) * scale).max(1.0) as u32;
    let fit_h = ((mh as f32) * scale).max(1.0) as u32;
    let resized =
        image::imageops::resize(mask, fit_w, fit_h, image::imageops::FilterType::Lanczos3);

    let mut cell: RgbImage =
        ImageBuffer::from_pixel(target_w, target_h, image::Rgb([shirt.0, shirt.1, shirt.2]));
    let offset_x = (target_w - fit_w) / 2;
    let offset_y = (target_h - fit_h) / 2;
    for (x, y, p) in resized.enumerate_pixels() {
        let alpha = 1.0 - (p[0] as f32 / 255.0);
        let inv = 1.0 - alpha;
        let px = cell.get_pixel(offset_x + x, offset_y + y);
        let blended = [
            (px[0] as f32 * inv + ink.0 as f32 * alpha).round() as u8,
            (px[1] as f32 * inv + ink.1 as f32 * alpha).round() as u8,
            (px[2] as f32 * inv + ink.2 as f32 * alpha).round() as u8,
        ];
        cell.put_pixel(offset_x + x, offset_y + y, image::Rgb(blended));
    }
    cell
}

// TODO(L4-followup): add per-cell captions via ab_glyph (same font
// loading helper used by `border.rs`). For v1 the grid alone is useful.
