//! Underbase fallback — LAB L channel inverted.
//!
//! Secondary underbase recipe. LAB L is a perceptual lightness measure
//! that matches how humans see brightness, so on non-saturated art (pure
//! grayscale, muted portraits, monochrome illustration) it produces a
//! smoother underbase than the HSB B variant. On saturated art it
//! underestimates how much white is needed under dark reds and blues —
//! that's why `hsb_brightness_inverted` is the default recommendation.
//!
//! Kept as a distinct extractor so the GUI can surface it as a fallback
//! and so projects built around it stay reproducible.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::{rgb_to_lab, Rgb};

pub fn extract(source: &RgbImage) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for (x, y, p) in source.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));
        // Amount of underbase ink is proportional to L: bright pixel =
        // high L = lots of white ink. Then convert to 0=ink density.
        let amount = (lab.l.clamp(0.0, 100.0)) / 100.0;
        let density = ((1.0 - amount) * 255.0).round().clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_is_full_ink() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([255, 255, 255]));
        let out = extract(&src);
        assert!(out.get_pixel(0, 0)[0] < 5);
    }

    #[test]
    fn black_is_no_ink() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([0, 0, 0]));
        let out = extract(&src);
        assert_eq!(out.get_pixel(0, 0)[0], 255);
    }
}
