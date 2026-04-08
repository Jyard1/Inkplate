//! Photoshop-style "Color Range" extractor — graduated LAB ΔE selection.
//!
//! This is the workhorse for sim-process color channels. Unlike
//! `spot_solid`, which snaps pixels to the target or drops them, color
//! range produces a continuous-tone density map: pixels matching the
//! target exactly get full ink (0), pixels far away get none (255), and
//! everything in between gets a partial value via a falloff curve.
//!
//! The `fuzziness` parameter controls the width of the ramp — low values
//! (≈20) give a tight selection similar to a hard tolerance, high values
//! (≈100) pull in most of the image as partial density. This matches how
//! Photoshop's slider behaves.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::{rgb_to_lab, Rgb};
use crate::engine::layer::ColorRangeFalloff;

pub fn extract(
    source: &RgbImage,
    target: Rgb,
    fuzziness: f32,
    falloff: ColorRangeFalloff,
) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let target_lab = rgb_to_lab(target);
    let fuzz = fuzziness.max(1.0);

    for (x, y, p) in source.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));
        let d = lab.delta_e(target_lab);
        // Normalize distance to [0, 1] using fuzziness as the half-range.
        let t = (d / fuzz).clamp(0.0, 1.0);
        let alpha = 1.0 - apply_falloff(t, falloff);
        // Density convention: 0 = ink, 255 = no ink.
        let density = ((1.0 - alpha) * 255.0).round().clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

fn apply_falloff(t: f32, falloff: ColorRangeFalloff) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match falloff {
        ColorRangeFalloff::Linear => t,
        ColorRangeFalloff::Quadratic => t * t,
        // Smoothstep: gentle shoulders, sharper middle — gives a more
        // "organic" selection edge that matches Photoshop's output better
        // than a straight ramp.
        ColorRangeFalloff::Smooth => t * t * (3.0 - 2.0 * t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_target_is_full_ink() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([200, 30, 30]));
        let out = extract(&src, Rgb(200, 30, 30), 60.0, ColorRangeFalloff::Linear);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
    }

    #[test]
    fn far_color_is_no_ink() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([0, 0, 255]));
        let out = extract(&src, Rgb(255, 0, 0), 40.0, ColorRangeFalloff::Linear);
        assert_eq!(out.get_pixel(0, 0)[0], 255);
    }
}
