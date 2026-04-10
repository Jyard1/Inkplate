//! Film export — one PNG per layer at the target export DPI.
//!
//! Key invariant carried forward from the Python reference: when the
//! export DPI is higher than the preview DPI, we **re-run the pipeline
//! at the export DPI** instead of resampling the already-rasterized
//! halftone. Upscaling a finished halftone smudges the dots into halos
//! and destroys the screen.
//!
//! Resizing before the halftone stage is fine (and necessary), so the
//! flow is:
//!
//! 1. Decide output width in pixels from `width_inches * dpi`.
//! 2. Resize the *source* RGB image to that size with LANCZOS.
//! 3. Call `process_layer` against the resized source with the new
//!    JobOpts (export DPI + same LPI/angle).
//! 4. Write the `processed` field as PNG with a pHYs chunk encoding the
//!    DPI so downstream RIPs open it at the correct physical size.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{imageops::FilterType, GrayImage, RgbImage};

use crate::engine::layer::{Extractor, Layer};
use crate::engine::pipeline::{
    compute_composite_union, process_layer, process_layer_with_extraction, JobOpts,
};
use crate::engine::preprocess;

use super::border::BorderOpts;
use super::reg_marks::RegMarkOpts;

#[derive(Debug, Clone)]
pub struct ExportOpts {
    /// Target film DPI. Overrides whatever DPI the preview was using.
    pub dpi: u32,
    /// Physical film width in inches. If `None`, export at native size.
    pub width_inches: Option<f32>,
    /// Lines per inch for halftone layers. `None` inherits from the
    /// job defaults.
    pub lpi: Option<f32>,
    /// When true, write the smooth preview mask instead of the
    /// rasterized halftone. Useful for proofs and debugging.
    pub preview_only: bool,
    /// Registration marks in all four corners.
    pub reg_marks: Option<RegMarkOpts>,
    /// Film border with caption (layer number, name, ink, LPI, angle).
    pub border: Option<BorderOpts>,
    /// Optional foreground mask applied after `process_layer` to
    /// knock out the source background. Must match the resized-source
    /// dimensions used at export time — if `width_inches` is set, the
    /// caller should pass in a mask built at the same physical size.
    pub foreground_mask: Option<Arc<GrayImage>>,
}

impl Default for ExportOpts {
    fn default() -> Self {
        Self {
            dpi: 300,
            width_inches: None,
            lpi: None,
            preview_only: false,
            reg_marks: None,
            border: None,
            foreground_mask: None,
        }
    }
}

/// Export a single layer to `path` as a PNG. Returns the final image
/// dimensions written so callers can report progress.
pub fn export_layer(
    source: &RgbImage,
    layer: &Layer,
    layer_index: usize,
    path: &Path,
    opts: &ExportOpts,
) -> anyhow::Result<(u32, u32)> {
    let resized = preprocess::clamp_near_black(&resize_source_for_export(source, opts), 50);
    let job = JobOpts {
        dpi: opts.dpi,
        default_lpi: opts.lpi.unwrap_or(55.0),
        default_angle_deg: 22.5,
    };
    // The ExportOpts mask may have been built at a different size
    // than the resized source (if width_inches forced a resample).
    // Only pass it to the pipeline when the dimensions match;
    // otherwise skip and the export won't have a knockout. A TODO
    // for Landing 6 is to resample the mask to match the resized
    // source size, but that requires nearest-neighbor to keep the
    // binary edges crisp.
    let fg_ref: Option<&GrayImage> = opts
        .foreground_mask
        .as_deref()
        .filter(|fg| fg.dimensions() == resized.dimensions());
    let processed = process_layer(&resized, layer, job, fg_ref);
    let film = if opts.preview_only {
        processed.preview
    } else {
        processed.processed
    };
    let mut film = film;

    if let Some(reg) = &opts.reg_marks {
        super::reg_marks::draw(&mut film, reg);
    }
    if let Some(border) = &opts.border {
        film = super::border::draw(&film, border, layer, layer_index, job.default_lpi);
    }

    write_png_with_dpi(&film, path, opts.dpi)?;
    Ok(film.dimensions())
}

/// Export every layer in `layers` to `outdir`, filename template
/// `{idx:02}_{name}_{hex}.png`. Skips hidden / excluded layers.
///
/// Uses two-pass processing so [`Extractor::CompositeUnion`] layers
/// (underbases) are derived from the union of the other layers'
/// previews at export resolution.
pub fn export_all(
    source: &RgbImage,
    layers: &[Layer],
    outdir: &Path,
    opts: &ExportOpts,
) -> anyhow::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(outdir)?;

    let resized = preprocess::clamp_near_black(&resize_source_for_export(source, opts), 50);
    let job = JobOpts {
        dpi: opts.dpi,
        default_lpi: opts.lpi.unwrap_or(55.0),
        default_angle_deg: 22.5,
    };
    let fg_ref: Option<&GrayImage> = opts
        .foreground_mask
        .as_deref()
        .filter(|fg| fg.dimensions() == resized.dimensions());

    // Pass 1: process non-CompositeUnion layers, collect previews.
    let mut previews: Vec<Option<GrayImage>> = vec![None; layers.len()];
    let mut results: Vec<Option<(image::GrayImage, image::GrayImage)>> =
        vec![None; layers.len()];

    for (i, layer) in layers.iter().enumerate() {
        if !layer.visible || !layer.include_in_export {
            continue;
        }
        if matches!(layer.extractor, Extractor::CompositeUnion) {
            continue;
        }
        let processed = process_layer(&resized, layer, job, fg_ref);
        previews[i] = Some(processed.preview.clone());
        results[i] = Some((processed.preview, processed.processed));
    }

    // Pass 2: process CompositeUnion layers from the union.
    let (w, h) = resized.dimensions();
    for (i, layer) in layers.iter().enumerate() {
        if !layer.visible || !layer.include_in_export {
            continue;
        }
        if !matches!(layer.extractor, Extractor::CompositeUnion) {
            continue;
        }
        let union = compute_composite_union(layers, &previews, i, w, h, Some(&resized));
        let processed = process_layer_with_extraction(union, layer, job, fg_ref);
        results[i] = Some((processed.preview, processed.processed));
    }

    // Write PNGs.
    let mut written = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        if !layer.visible || !layer.include_in_export {
            continue;
        }
        let Some((preview, processed_img)) = results[i].take() else {
            continue;
        };
        let mut film = if opts.preview_only {
            preview
        } else {
            processed_img
        };
        if let Some(reg) = &opts.reg_marks {
            super::reg_marks::draw(&mut film, reg);
        }
        if let Some(border) = &opts.border {
            film = super::border::draw(&film, border, layer, i, job.default_lpi);
        }
        let filename = format!(
            "{:02}_{}_{:02x}{:02x}{:02x}.png",
            layer.print_index,
            sanitize(&layer.name),
            layer.ink.0,
            layer.ink.1,
            layer.ink.2
        );
        let path = outdir.join(&filename);
        write_png_with_dpi(&film, &path, opts.dpi)?;
        written.push(path);
    }
    Ok(written)
}

fn resize_source_for_export(source: &RgbImage, opts: &ExportOpts) -> RgbImage {
    let Some(width_in) = opts.width_inches else {
        return source.clone();
    };
    let target_w = (width_in * opts.dpi as f32).round() as u32;
    if target_w == 0 || target_w == source.width() {
        return source.clone();
    }
    let (w, h) = source.dimensions();
    let target_h = (h as f32 * target_w as f32 / w as f32).round() as u32;
    image::imageops::resize(source, target_w, target_h, FilterType::Lanczos3)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

/// Write an 8-bit grayscale PNG with a `pHYs` chunk encoding the DPI
/// so RIP software opens the film at the correct physical size.
///
/// Goes through the `png` crate's encoder directly (not `image`) so
/// the pHYs chunk is written cleanly alongside the IHDR instead of
/// being spliced in after the fact. The chunk stores pixels per
/// meter (CIE unit 1 = meters) as a big-endian `u32` in each axis.
pub fn write_png_with_dpi(img: &GrayImage, path: &Path, dpi: u32) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let w = BufWriter::new(file);

    // 1 inch = 0.0254 m, so pixels/meter = dpi / 0.0254
    let ppm = ((dpi as f32) / 0.0254).round() as u32;

    let mut encoder = png::Encoder::new(w, img.width(), img.height());
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_pixel_dims(Some(png::PixelDimensions {
        xppu: ppm,
        yppu: ppm,
        unit: png::Unit::Meter,
    }));

    let mut writer = encoder.write_header()?;
    writer.write_image_data(img.as_raw())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma};

    /// Writing a tiny grayscale PNG should produce a file with a
    /// `pHYs` chunk whose pixels-per-meter round-trips to the DPI
    /// we passed in.
    #[test]
    fn phys_chunk_inserted() {
        let img: GrayImage = ImageBuffer::from_pixel(4, 4, Luma([128]));
        let tmp = std::env::temp_dir().join("inkplate_phys_test.png");
        write_png_with_dpi(&img, &tmp, 300).unwrap();

        // Read it back through the png crate and inspect the pHYs.
        let file = std::fs::File::open(&tmp).unwrap();
        let decoder = png::Decoder::new(file);
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        let dims = info.pixel_dims.expect("pHYs chunk should be present");
        assert!(matches!(dims.unit, png::Unit::Meter));
        // 300 DPI → 11811 ppm (±1 due to rounding).
        let expected = (300.0_f32 / 0.0254).round() as u32;
        assert!(
            dims.xppu.abs_diff(expected) <= 1,
            "xppu {} != expected {}",
            dims.xppu,
            expected
        );
        assert_eq!(dims.xppu, dims.yppu);

        std::fs::remove_file(&tmp).ok();
    }
}
