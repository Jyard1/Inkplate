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
use image::GrayImage;

use crate::engine::extractors::run_extractor;
use crate::engine::foreground::apply_mask_inplace;
use crate::engine::halftone::HalftoneOpts;
use crate::engine::layer::{Layer, RenderMode};
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
            default_lpi: 55.0,
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
    // 1. Extract the raw density map via the 9-extractor dispatch.
    let mut mask = run_extractor(source, layer);

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
        RenderMode::Solid => mask,
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
            halftone::make_halftone(&mask, opts)
        }
        RenderMode::FmDither => dither::floyd_steinberg_grayscale(&mask),
        RenderMode::BayerDither => dither::bayer_grayscale(&mask, 8),
        RenderMode::NoiseDither => dither::white_noise_grayscale(&mask),
        RenderMode::BlueNoise => dither::blue_noise_grayscale(&mask),
        RenderMode::IndexFs | RenderMode::IndexBayer => {
            // Index modes already rasterize inside the extractor; we just
            // need to binarize here.
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
