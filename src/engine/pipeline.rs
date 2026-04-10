//! Per-layer processing pipeline.
//!
//! Takes a source image and a [`Layer`] spec and runs the full chain:
//!
//! ```text
//! source → extractor → invert? → curve LUT → density → choke →
//!     mask shaping (smooth/open/close/edge) → render mode
//! ```
//!
//! The output is a pair of images:
//!
//! - `preview` — smooth density map, used for on-screen compositing
//! - `processed` — rasterized render (halftone dots, dither, etc.), used
//!   for film export
//!
//! Only the preview is cheap to rebuild; the processed rasterization is
//! the expensive step, and it's the one that gets re-run at export DPI
//! instead of being resampled (resampling halftone dots smudges them).
//!
use image::{GrayImage, ImageBuffer, Luma};

use crate::engine::extractors::run_extractor;
use crate::engine::foreground::apply_mask_inplace;
use crate::engine::halftone::HalftoneOpts;
use crate::engine::layer::{Layer, LayerKind, RenderMode};
use crate::engine::{dither, halftone, morphology, tone};

/// Result of running one layer through the pipeline.
pub struct ProcessedLayer {
    pub preview: GrayImage,
    pub processed: GrayImage,
}

/// Global job settings shared by every layer.
#[derive(Debug, Clone, Copy)]
pub struct JobOpts {
    pub dpi: u32,
    pub default_lpi: f32,
    pub default_angle_deg: f32,
}

impl Default for JobOpts {
    fn default() -> Self {
        Self {
            dpi: 300,
            default_lpi: 65.0,
            default_angle_deg: 22.5,
        }
    }
}

/// Process a single layer against a source image.
///
/// `foreground_mask`, if provided, is applied at the very end to both
/// the preview and processed outputs — any pixel where the mask is 0
/// gets clamped to 255 (no ink), regardless of what the extractor
/// said. This is how background removal works: every layer, every
/// render mode, the same knockout applied last.
pub fn process_layer(
    source: &image::RgbImage,
    layer: &Layer,
    job: JobOpts,
    foreground_mask: Option<&GrayImage>,
) -> ProcessedLayer {
    // 1. Extract the raw density map via the extractor dispatch.
    let mask = run_extractor(source, layer);
    process_layer_with_extraction(mask, layer, job, foreground_mask)
}

/// Run the pipeline from step 2 onward using a pre-computed extraction
/// mask. Used by the two-pass CompositeUnion flow where the "extraction"
/// is the union of sibling layers' previews rather than an extractor call.
pub fn process_layer_with_extraction(
    mask: GrayImage,
    layer: &Layer,
    job: JobOpts,
    foreground_mask: Option<&GrayImage>,
) -> ProcessedLayer {
    let mut mask = mask;

    // 2. Invert if the layer flag says so.
    if layer.mask.invert {
        for p in mask.iter_mut() {
            *p = 255 - *p;
        }
    }

    // 3. Tone curve LUT.
    if let Some(lut) = tone::build_lut(&layer.tone.curve) {
        tone::apply_lut_in_place(&mut mask, &lut);
    }

    // 4. Density multiplier.
    if (layer.tone.density - 1.0).abs() > 1e-4 {
        mask = tone::apply_density(&mask, layer.tone.density);
    }

    // 4.5. Solid layers: Gaussian blur → binarize.
    // The gradual tone curve produces a transition zone with gray
    // values. Source noise causes random pixels in this zone to
    // scatter across the 128 threshold, creating isolated
    // black/white dots. A small Gaussian blur (σ=1.5) averages each
    // pixel with its neighbors — isolated noise pixels get absorbed
    // into the surrounding majority, so they fall cleanly on one
    // side of the threshold. The subsequent binarize then produces
    // clean binary edges with no scattered dots. Choke and morphology
    // (steps 5–6) operate on the clean binary result.
    if layer.render_mode == RenderMode::Solid {
        if layer.mask.solid_blur > 0.01 {
            mask = morphology::smooth_mask(&mask, layer.mask.solid_blur);
        }
        for p in mask.iter_mut() {
            *p = if *p < 128 { 0 } else { 255 };
        }
    }

    // 5. Choke (post-tone erosion).
    if layer.tone.choke > 0 {
        mask = morphology::erode_ink(&mask, layer.tone.choke);
    }

    // 6. Mask shaping: smooth → open → close → edge.
    if layer.mask.smooth_radius > 0 {
        mask = morphology::smooth_mask(&mask, layer.mask.smooth_radius as f32);
    }
    if layer.mask.noise_open > 0 {
        mask = morphology::open_ink(&mask, layer.mask.noise_open);
    }
    if layer.mask.holes_close > 0 {
        mask = morphology::close_ink(&mask, layer.mask.holes_close);
    }
    if layer.mask.edge_radius > 0 {
        mask = match layer.mask.edge_mode {
            crate::engine::layer::EdgeMode::Hard => mask,
            crate::engine::layer::EdgeMode::Choke => {
                morphology::erode_ink(&mask, layer.mask.edge_radius)
            }
            crate::engine::layer::EdgeMode::Spread => {
                morphology::dilate_ink(&mask, layer.mask.edge_radius)
            }
            crate::engine::layer::EdgeMode::FeatherHt => {
                morphology::feather_halftone_edge(&mask, layer.mask.edge_radius as f32)
            }
        };
    }

    // 6.5. Apply the foreground knockout BEFORE cloning the preview
    // so that both the preview (smooth) and processed (rasterized)
    // outputs agree on what's background. Render modes like halftone
    // see a mask with the background already clamped to 255, so
    // they skip drawing dots there instead of drawing dots that get
    // clobbered later.
    if let Some(fg) = foreground_mask {
        apply_mask_inplace(&mut mask, fg);
    }

    let preview = mask.clone();

    // 7. Render mode.
    let processed = match layer.render_mode {
        RenderMode::Solid => mask, // already binarized at step 4.5
        RenderMode::Halftone => {
            let opts = HalftoneOpts {
                dpi: job.dpi,
                lpi: if layer.halftone.lpi > 0 {
                    layer.halftone.lpi as f32
                } else {
                    job.default_lpi
                },
                angle_deg: if layer.halftone.angle_deg >= 0.0 {
                    layer.halftone.angle_deg
                } else {
                    halftone::auto_angle_for_index(layer.print_index as usize)
                },
                dot: layer.halftone.dot_shape.unwrap_or_default(),
                curve: layer.halftone.curve,
                supersample: 3,
            };
            // Binarize after halftoning: every pixel on the film
            // must be 100% black (ink) or 100% white (clear). The
            // supersampled rasterizer produces anti-aliased gray
            // edges on dots, which looks good on screen but won't
            // expose screen emulsion correctly — the gray fringe
            // under-exposes and produces unreliable dot shapes on
            // the mesh. Threshold at 128 snaps everything to binary.
            binarize(&halftone::make_halftone(&mask, opts))
        }
        RenderMode::FmDither => binarize(&dither::floyd_steinberg_grayscale(&mask)),
        RenderMode::BayerDither => binarize(&dither::bayer_grayscale(&mask, 8)),
        RenderMode::NoiseDither => binarize(&dither::white_noise_grayscale(&mask)),
        RenderMode::BlueNoise => binarize(&dither::blue_noise_grayscale(&mask)),
        RenderMode::IndexFs | RenderMode::IndexBayer => {
            let mut out = mask.clone();
            for p in out.iter_mut() {
                *p = if *p < 128 { 0 } else { 255 };
            }
            out
        }
    };

    // Final safety knockout on the processed raster. Halftone cells
    // whose center lands inside the foreground but whose dot area
    // overflows into the background can leave stray ink outside the
    // art; clamp those pixels here so the film has truly zero ink
    // outside the mask.
    let mut processed = processed;
    if let Some(fg) = foreground_mask {
        apply_mask_inplace(&mut processed, fg);
    }

    ProcessedLayer { preview, processed }
}

/// Compute the composite union mask for a [`CompositeUnion`] layer.
///
/// Returns a density map where 0 = "at least one sibling color layer
/// has ink" and 255 = "no sibling has ink." Skips:
///
/// - The layer at `self_idx`
/// - Layers with `kind == Underbase`
/// - Layers with near-black ink (R, G, B all < 30)
/// - Source pixels where max(R,G,B) < 50 (near-black in the source —
///   color extractors may report spurious ink there due to fuzziness
///   and JPEG noise, but on a dark shirt the shirt itself is the black)
pub fn compute_composite_union(
    layers: &[Layer],
    previews: &[Option<GrayImage>],
    self_idx: usize,
    width: u32,
    height: u32,
    source: Option<&image::RgbImage>,
) -> GrayImage {
    let mut union: GrayImage = ImageBuffer::from_pixel(width, height, Luma([255u8]));

    for (idx, layer) in layers.iter().enumerate() {
        if idx == self_idx {
            continue;
        }
        if layer.kind == LayerKind::Underbase {
            continue;
        }
        // Skip near-black inks (black plate doesn't need white under it
        // on a dark shirt).
        if layer.ink.0 < 30 && layer.ink.1 < 30 && layer.ink.2 < 30 {
            continue;
        }

        if let Some(Some(preview)) = previews.get(idx) {
            if preview.dimensions() != (width, height) {
                continue;
            }
            // Union: per-pixel MIN of density. Density convention is
            // 0 = ink, 255 = no ink, so min = "the most ink any layer
            // wants here."
            for (dst, src) in union.iter_mut().zip(preview.iter()) {
                if *src < *dst {
                    *dst = *src;
                }
            }
        }
    }

    // Gate by source brightness: if the source pixel is near-black,
    // force no-underbase. Color extractors (especially yellow) can
    // report spurious ink for dark source pixels due to fuzziness /
    // JPEG noise, but on a dark shirt those areas don't need white.
    if let Some(src) = source {
        if src.dimensions() == (width, height) {
            let src_raw = src.as_raw();
            for (i, dst) in union.iter_mut().enumerate() {
                let r = src_raw[i * 3];
                let g = src_raw[i * 3 + 1];
                let b = src_raw[i * 3 + 2];
                if r.max(g).max(b) < 50 {
                    *dst = 255; // no underbase for near-black source
                }
            }
        }
    }

    union
}

/// Threshold every pixel to 0 (ink) or 255 (no ink). Screen-print
/// films are binary — gray pixels won't expose the emulsion
/// correctly. Called after every halftone / dither render mode.
fn binarize(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for (x, y, p) in img.enumerate_pixels() {
        out.put_pixel(x, y, Luma([if p[0] < 128 { 0 } else { 255 }]));
    }
    out
}
