//! The nine density-map extractors.
//!
//! Each submodule owns one extractor function + its tests. The
//! [`run_extractor`] dispatcher walks the [`Extractor`] enum on a
//! [`Layer`] and calls the right one. `pipeline::process_layer` is the
//! only caller — everything else goes through the pipeline so tone /
//! mask shaping / render modes happen in the right order.
//!
//! Extractor list (matches `engine::layer::Extractor`):
//!
//! | # | Module                     | Used for                                  |
//! |---|----------------------------|-------------------------------------------|
//! | 1 | [`spot_solid`]             | Flat vector fills, logos                  |
//! | 2 | [`spot_aa`]                | Anti-aliased spot with Voronoi soft edge  |
//! | 3 | [`color_range`]            | Sim-process color channels                |
//! | 4 | [`hsb_brightness_inverted`]| White underbase (the correct recipe)      |
//! | 5 | [`lab_lightness_inverted`] | Underbase fallback                        |
//! | 6 | [`gcr_black`]              | Black plate, highlight white              |
//! | 7 | [`channel_calc`]           | Custom per-channel expressions            |
//! | 8 | [`luminance_threshold`]    | Stencil / silhouette                      |
//! | 9 | [`index_assignment`]       | Index dither (FS/Bayer) palette entries   |

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::layer::{Extractor, Layer};

pub mod channel_calc;
pub mod color_range;
pub mod gcr_black;
pub mod hsb_brightness_inverted;
pub mod index_assignment;
pub mod lab_lightness_inverted;
pub mod luminance_threshold;
pub mod spot_aa;
pub mod spot_solid;

/// Run the extractor for a single layer against a source image. Returns
/// a density map in the engine convention (0 = ink, 255 = no ink).
///
/// A [`Extractor::ManualPaint`] layer returns a blank (all-255) mask —
/// the pipeline is expected to overlay the user's brush strokes from a
/// cache field after calling this function.
pub fn run_extractor(source: &RgbImage, layer: &Layer) -> GrayImage {
    match &layer.extractor {
        Extractor::SpotSolid { target, tolerance } => {
            spot_solid::extract(source, *target, *tolerance)
        }
        Extractor::SpotAa {
            targets,
            others,
            aa_full,
            aa_end,
            aa_reach: _,
            target_weights,
        } => {
            // The aa layer needs to know *which* of `targets` belongs to
            // this layer. We match by ink color; if none of the targets
            // equals the layer's ink, fall back to index 0 so the layer
            // still produces something visible.
            let target_index = targets.iter().position(|c| *c == layer.ink).unwrap_or(0);
            spot_aa::extract(
                source,
                spot_aa::Params {
                    targets,
                    others,
                    target_index,
                    aa_full: *aa_full,
                    aa_end: *aa_end,
                    target_weights: if target_weights.len() == targets.len() {
                        Some(target_weights)
                    } else {
                        None
                    },
                },
            )
        }
        Extractor::ColorRange {
            target,
            fuzziness,
            falloff,
        } => color_range::extract(source, *target, *fuzziness, *falloff),
        Extractor::HsbBrightnessInverted {
            s_curve,
            boost_under_darks,
            boost_strength,
        } => {
            hsb_brightness_inverted::extract(source, *s_curve, *boost_under_darks, *boost_strength)
        }
        Extractor::LabLightnessInverted => lab_lightness_inverted::extract(source),
        Extractor::GcrBlack {
            strength,
            invert_input,
        } => gcr_black::extract(source, *strength, *invert_input),
        Extractor::ChannelCalc { expr } => match channel_calc::parse(expr) {
            Ok(compiled) => channel_calc::extract(source, &compiled),
            Err(_) => blank(source),
        },
        Extractor::LuminanceThreshold { threshold, above } => {
            luminance_threshold::extract(source, *threshold, *above)
        }
        Extractor::IndexAssignment {
            palette,
            index,
            dither,
        } => index_assignment::extract(source, palette, *index, *dither),
        Extractor::ManualPaint { buf } => manual_paint_mask(source, buf.as_ref()),
    }
}

/// Return the user-painted mask for a [`Extractor::ManualPaint`]
/// layer, resizing the stored buffer if the source dimensions drift
/// (e.g. the user re-opened the image at a different resolution).
/// If no buffer has been allocated yet, return a blank all-255 mask.
fn manual_paint_mask(
    source: &RgbImage,
    buf: Option<&crate::engine::layer::ManualPaintBuf>,
) -> GrayImage {
    let (sw, sh) = source.dimensions();
    let Some(buf) = buf else {
        return blank(source);
    };
    if buf.width == sw && buf.height == sh {
        // Fast path: dimensions match, reconstruct directly.
        return ImageBuffer::from_raw(buf.width, buf.height, buf.pixels.clone())
            .unwrap_or_else(|| blank(source));
    }
    // Dimensions drifted — scale the stored buffer into the source
    // size with nearest-neighbour so binary edges stay crisp.
    let stored: GrayImage =
        match ImageBuffer::from_raw(buf.width, buf.height, buf.pixels.clone()) {
            Some(img) => img,
            None => return blank(source),
        };
    image::imageops::resize(&stored, sw, sh, image::imageops::FilterType::Nearest)
}

/// All-255 density map (no ink) at the same size as the source. Used for
/// error fallbacks and the `ManualPaint` no-op.
fn blank(source: &RgbImage) -> GrayImage {
    let (w, h) = source.dimensions();
    ImageBuffer::from_pixel(w, h, Luma([255u8]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::layer::{Extractor, Layer, ManualPaintBuf};

    #[test]
    fn manual_paint_returns_blank_without_buffer() {
        let src = RgbImage::from_pixel(4, 4, image::Rgb([200, 100, 50]));
        let mut layer = Layer::new_spot(crate::engine::color::Rgb::BLACK);
        layer.extractor = Extractor::ManualPaint { buf: None };
        let mask = run_extractor(&src, &layer);
        for &p in mask.iter() {
            assert_eq!(p, 255, "empty manual paint layer must have no ink");
        }
    }

    #[test]
    fn manual_paint_round_trips_buffer() {
        let src = RgbImage::from_pixel(4, 4, image::Rgb([200, 100, 50]));
        let mut buf = ManualPaintBuf::blank(4, 4);
        // Paint a single pixel at (1, 2) → ink.
        buf.pixels[2 * 4 + 1] = 0;
        let mut layer = Layer::new_spot(crate::engine::color::Rgb::BLACK);
        layer.extractor = Extractor::ManualPaint { buf: Some(buf) };
        let mask = run_extractor(&src, &layer);
        assert_eq!(mask.get_pixel(1, 2)[0], 0);
        assert_eq!(mask.get_pixel(0, 0)[0], 255);
    }

    #[test]
    fn manual_paint_resizes_on_dim_drift() {
        // Buffer at 2x2, source at 4x4 → should nearest-scale up.
        let src = RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0]));
        let mut buf = ManualPaintBuf::blank(2, 2);
        buf.pixels = vec![0, 255, 255, 255]; // ink in top-left of the 2x2
        let mut layer = Layer::new_spot(crate::engine::color::Rgb::BLACK);
        layer.extractor = Extractor::ManualPaint { buf: Some(buf) };
        let mask = run_extractor(&src, &layer);
        // Top-left quadrant should be ink (0), rest no-ink (255).
        assert_eq!(mask.get_pixel(0, 0)[0], 0);
        assert_eq!(mask.get_pixel(1, 1)[0], 0);
        assert_eq!(mask.get_pixel(3, 3)[0], 255);
    }
}
