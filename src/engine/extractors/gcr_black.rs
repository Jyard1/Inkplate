//! GCR black extractor — the K channel from gray component replacement.
//!
//! In CMYK printing, "gray component replacement" pulls the gray portion
//! of a CMY mix out and replaces it with black (K) ink. The formula for
//! the K channel is the minimum of the three complement values:
//!
//! ```text
//! K = min(1 - R, 1 - G, 1 - B) * strength
//! ```
//!
//! This is how you get a clean **black plate** for sim-process jobs —
//! it picks only the pixels that are dark in all three channels, which
//! is exactly what you want the black screen to print. The same
//! extractor with `invert_input = true` gives you a **highlight white**
//! channel: run it on the inverted source and the bright pixels come
//! out instead of the dark ones.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

pub fn extract(source: &RgbImage, strength: f32, invert_input: bool) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let s = strength.clamp(0.0, 2.0);

    for (x, y, p) in source.enumerate_pixels() {
        let (r, g, b) = if invert_input {
            (255 - p[0], 255 - p[1], 255 - p[2])
        } else {
            (p[0], p[1], p[2])
        };
        // min(1-R, 1-G, 1-B) is the largest complement, i.e. the
        // smallest primary value — we want `255 - max(R,G,B)`.
        let max_c = r.max(g).max(b);
        let k = 255 - max_c;
        let ink = ((k as f32 / 255.0) * s * 255.0).round().clamp(0.0, 255.0) as u8;
        let density = 255 - ink;
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_black_is_full_ink() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([0, 0, 0]));
        let out = extract(&src, 1.0, false);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
    }

    #[test]
    fn pure_white_is_no_ink() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([255, 255, 255]));
        let out = extract(&src, 1.0, false);
        assert_eq!(out.get_pixel(0, 0)[0], 255);
    }

    #[test]
    fn strong_red_gives_no_black() {
        // max channel is high, so K should be low → no black ink.
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([220, 20, 20]));
        let out = extract(&src, 1.0, false);
        assert!(out.get_pixel(0, 0)[0] > 200);
    }

    #[test]
    fn inverted_input_swaps_extremes() {
        let src: RgbImage = ImageBuffer::from_pixel(1, 1, image::Rgb([255, 255, 255]));
        let out = extract(&src, 1.0, true);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
    }
}
