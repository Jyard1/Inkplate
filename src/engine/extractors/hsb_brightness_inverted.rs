//! Underbase extractor — HSB brightness channel, inverted, S-curved.
//!
//! This is the **correct** underbase recipe from the research in the
//! rebuild plan. Summary of the pro workflow:
//!
//! 1. Convert RGB to HSB; take the B (value) channel.
//! 2. Invert it — bright source pixels become dark = lots of underbase ink.
//! 3. Apply an S-curve for contrast (lighten highlights, darken shadows).
//! 4. Boost density under saturated dark colors — plastisol opacity is
//!    poor, so reds and blues need more white than their luminance
//!    suggests, otherwise the color sits wrong on the shirt.
//!
//! Choke is applied later in the pipeline, not here.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

pub fn extract(
    source: &RgbImage,
    s_curve: f32,
    boost_under_darks: bool,
    boost_strength: f32,
) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let s = s_curve.max(0.1);
    let boost = boost_strength.clamp(0.0, 2.0);

    for (x, y, p) in source.enumerate_pixels() {
        let r = p[0] as f32 / 255.0;
        let g = p[1] as f32 / 255.0;
        let b = p[2] as f32 / 255.0;

        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let value = max_c; // HSB B
        let saturation = if max_c <= 1e-6 {
            0.0
        } else {
            (max_c - min_c) / max_c
        };

        // Amount of underbase ink on [0, 1] — proportional to brightness.
        // The brighter the source, the more white ink we need to lay
        // down on a dark shirt. (The rebuild plan phrases this as
        // "invert the brightness channel", but the inversion is in the
        // display convention — in our 0=ink internal convention we use
        // brightness directly as the ink amount.)
        let mut amount = logistic_s(value, s);

        if boost_under_darks {
            // Saturated dark pixels (reds, blues, maroons) need extra
            // white ink despite their low luminance — plastisol opacity
            // is poor, so the color ink alone can't carry the value on a
            // dark shirt.
            //
            // Brightness gate: near-black pixels (value < 0.10) get no
            // boost at all, ramping to full boost by value 0.25. Without
            // this, JPEG noise and anti-aliasing give near-black pixels
            // a slight color cast → high saturation at low brightness →
            // massive boost → unwanted underbase under black art.
            let gate = ((value - 0.10) / 0.15).clamp(0.0, 1.0);
            let dark_boost = saturation * (1.0 - value) * gate;
            amount = (amount + dark_boost * boost).clamp(0.0, 1.0);
        }

        // Density convention: 0 = ink, 255 = no ink.
        let density = ((1.0 - amount) * 255.0).round().clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

/// S-curve via `y = x^a / (x^a + (1-x)^a)`. At `a = 1` this is the
/// identity function; at `a > 1` the middle steepens into a classic
/// photography S-curve.
fn logistic_s(x: f32, a: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if (a - 1.0).abs() < 1e-4 {
        return x;
    }
    let xa = x.powf(a);
    let ya = (1.0 - x).powf(a);
    let denom = xa + ya;
    if denom <= 1e-9 {
        x
    } else {
        xa / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bright_source_is_full_underbase() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([255, 255, 255]));
        let out = extract(&src, 1.6, true, 0.4);
        // Bright white → full underbase → density ≈ 0
        assert!(out.get_pixel(0, 0)[0] < 30);
    }

    #[test]
    fn black_source_is_no_underbase() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([0, 0, 0]));
        let out = extract(&src, 1.6, true, 0.4);
        assert_eq!(out.get_pixel(0, 0)[0], 255);
    }

    #[test]
    fn saturated_dark_gets_boost() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([120, 0, 0]));
        let without = extract(&src, 1.6, false, 0.0);
        let with = extract(&src, 1.6, true, 0.8);
        assert!(with.get_pixel(0, 0)[0] <= without.get_pixel(0, 0)[0]);
    }
}
