//! Luminance threshold extractor — binary stencil.
//!
//! Trivial but useful: compute BT.601 luminance for each pixel and
//! threshold to binary. `above = true` keeps bright pixels as ink (for
//! "white silhouette" stencils where you want the light area), `false`
//! keeps dark pixels (standard stencil cutout).

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

pub fn extract(source: &RgbImage, threshold: u8, above: bool) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for (x, y, p) in source.enumerate_pixels() {
        let luma = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
        let l = luma.round() as u8;
        let hit = if above { l >= threshold } else { l < threshold };
        out.put_pixel(x, y, Luma([if hit { 0 } else { 255 }]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catches_darks() {
        let mut src: RgbImage = ImageBuffer::new(2, 1);
        src.put_pixel(0, 0, image::Rgb([30, 30, 30]));
        src.put_pixel(1, 0, image::Rgb([220, 220, 220]));
        let out = extract(&src, 128, false);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(1, 0)[0], 255);
    }

    #[test]
    fn above_inverts_selection() {
        let mut src: RgbImage = ImageBuffer::new(2, 1);
        src.put_pixel(0, 0, image::Rgb([30, 30, 30]));
        src.put_pixel(1, 0, image::Rgb([220, 220, 220]));
        let out = extract(&src, 128, true);
        assert_eq!(out.get_pixel(0, 0)[0], 255);
        assert_eq!(out.get_pixel(1, 0)[0], 0);
    }
}
