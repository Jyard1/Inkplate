//! Spot-solid extractor — binary mask by exact RGB match with tolerance.
//!
//! Cheapest extractor. For each pixel, if all three channels are within
//! `tolerance` of the target, mark it as ink (0); otherwise leave it as
//! no-ink (255). This is the right tool for flat vector fills and logos
//! where the source is known to be single-value per region.
//!
//! For anti-aliased spot art, use `spot_aa` instead — it grades the edge
//! pixels instead of snapping them.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::Rgb;

pub fn extract(source: &RgbImage, target: Rgb, tolerance: u8) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let tol = tolerance as i32;
    for (x, y, p) in source.enumerate_pixels() {
        let dr = (p[0] as i32 - target.0 as i32).abs();
        let dg = (p[1] as i32 - target.1 as i32).abs();
        let db = (p[2] as i32 - target.2 as i32).abs();
        let hit = dr <= tol && dg <= tol && db <= tol;
        out.put_pixel(x, y, Luma([if hit { 0 } else { 255 }]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_zero_tolerance() {
        let mut src: RgbImage = ImageBuffer::new(3, 1);
        src.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        src.put_pixel(1, 0, image::Rgb([254, 0, 0]));
        src.put_pixel(2, 0, image::Rgb([200, 0, 0]));
        let out = extract(&src, Rgb(255, 0, 0), 0);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(1, 0)[0], 255);
        assert_eq!(out.get_pixel(2, 0)[0], 255);
    }

    #[test]
    fn tolerance_grows_match_window() {
        let mut src: RgbImage = ImageBuffer::new(2, 1);
        src.put_pixel(0, 0, image::Rgb([250, 5, 5]));
        src.put_pixel(1, 0, image::Rgb([200, 50, 50]));
        let out = extract(&src, Rgb(255, 0, 0), 10);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(1, 0)[0], 255);
    }
}
