//! Source image preprocessing.
//!
//! Before anything hits an extractor, two variants of the source are
//! useful to have on hand:
//!
//! - **white-bg variant** — background replaced with pure white. Used by
//!   color-range extractors so the "background" subtraction actually
//!   works cleanly.
//! - **black-bg variant** — background replaced with pure black. Used by
//!   white-channel extractors (underbase, highlight white).
//!
//! Plus a chroma-strip pass (`desaturate`) for AI-generated renders that
//! have subtle color casts in what's supposed to be grayscale art.
//!
//! Landing 2 wires these into workflows. Landing 1 just ships the helpers.

use image::{ImageBuffer, RgbImage};

use crate::engine::color::{lab_to_rgb, rgb_to_lab, Lab, Rgb};
use crate::engine::foreground::detect_background_rgb;

/// Replace every near-background pixel with pure white. The result has
/// the foreground art alone on a white field.
pub fn to_white_bg(img: &RgbImage, tolerance_delta_e: f32) -> RgbImage {
    swap_bg(img, Rgb::WHITE, tolerance_delta_e)
}

/// Replace every near-background pixel with pure black.
pub fn to_black_bg(img: &RgbImage, tolerance_delta_e: f32) -> RgbImage {
    swap_bg(img, Rgb::BLACK, tolerance_delta_e)
}

fn swap_bg(img: &RgbImage, fill: Rgb, tolerance: f32) -> RgbImage {
    let bg = detect_background_rgb(img);
    let bg_lab = rgb_to_lab(bg);
    let (w, h) = img.dimensions();
    let mut out = ImageBuffer::new(w, h);
    for (x, y, p) in img.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb::from_array(p.0));
        if lab.delta_e(bg_lab) < tolerance {
            out.put_pixel(x, y, image::Rgb(fill.to_array()));
        } else {
            out.put_pixel(x, y, *p);
        }
    }
    out
}

/// Clamp near-black pixels to true (0,0,0). Source images often have
/// slight color casts in dark areas (JPEG compression, sensor noise,
/// AI upscaling) that make color extractors report spurious ink —
/// e.g. yellow sees a (20, 15, 10) pixel as having a warm tint and
/// outputs a non-zero density, turning what should be black into
/// grey-brown in the composite. Clamping max(R,G,B) < `threshold`
/// to pure black before extraction ensures color channels produce
/// zero ink there while the GCR black plate picks them up correctly.
pub fn clamp_near_black(img: &RgbImage, threshold: u8) -> RgbImage {
    let (w, h) = img.dimensions();
    let mut out = img.clone();
    for y in 0..h {
        for x in 0..w {
            let p = out.get_pixel(x, y).0;
            if p[0].max(p[1]).max(p[2]) < threshold {
                out.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
    }
    out
}

/// Strip chroma via LAB L — preserves perceptual brightness exactly. Use
/// this rather than BT.601 luma for subtle AI-render tints where the
/// gamma-weighted formula crushes the wrong tones.
pub fn desaturate(img: &RgbImage) -> RgbImage {
    let (w, h) = img.dimensions();
    let mut out = ImageBuffer::new(w, h);
    for (x, y, p) in img.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb::from_array(p.0));
        let gray = lab_to_rgb(Lab {
            l: lab.l,
            a: 0.0,
            b: 0.0,
        });
        out.put_pixel(x, y, image::Rgb(gray.to_array()));
    }
    out
}
