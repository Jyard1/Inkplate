//! True CMYK channel extractor — sRGB → linear → CMY with GCR.
//!
//! This is the mathematically exact process-color decomposition used by
//! the CMYK sim-process workflow. Each call extracts one of the four
//! channels (C, M, Y, or K) from the source image.
//!
//! The conversion works in **linear** RGB (not gamma-encoded sRGB)
//! because CMY values are physically meaningful ink ratios: C = 1 − R
//! only holds when R is linear light. Working in gamma space would
//! produce midtones that are too dark (~21.6% linear at sRGB 128
//! instead of the 50% a naive `1 − (128/255)` would suggest).
//!
//! Grey Component Replacement (GCR) removes the neutral component
//! from C+M+Y and transfers it to K. `gcr_strength` controls how
//! much: 0.0 = no GCR (all colour in CMY, K is zero), 1.0 = full
//! GCR (max neutral removed, K carries all the darkness).

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::srgb_to_linear;
use crate::engine::layer::CmykProcess;

pub fn extract(source: &RgbImage, channel: CmykProcess, gcr_strength: f32) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let gcr = gcr_strength.clamp(0.0, 1.0);

    for (x, y, p) in source.enumerate_pixels() {
        let r = srgb_to_linear(p[0]);
        let g = srgb_to_linear(p[1]);
        let b = srgb_to_linear(p[2]);

        // CMY from linear RGB.
        let mut c = 1.0 - r;
        let mut m = 1.0 - g;
        let mut yy = 1.0 - b;

        // Grey Component Replacement.
        let k_raw = c.min(m).min(yy);
        let k = k_raw * gcr;
        c = (c - k).max(0.0);
        m = (m - k).max(0.0);
        yy = (yy - k).max(0.0);

        let v = match channel {
            CmykProcess::Cyan => c,
            CmykProcess::Magenta => m,
            CmykProcess::Yellow => yy,
            CmykProcess::Black => k,
        };

        // Density convention: 0 = full ink, 255 = no ink.
        // channel value 1.0 → full ink → density 0.
        // channel value 0.0 → no ink → density 255.
        let density = ((1.0 - v) * 255.0).round().clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_pixel(r: u8, g: u8, b: u8, ch: CmykProcess, gcr: f32) -> u8 {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([r, g, b]));
        extract(&src, ch, gcr).get_pixel(0, 0)[0]
    }

    #[test]
    fn pure_red_channels() {
        // Red: C=0, M=1, Y=1. Cyan plate should have no ink.
        assert_eq!(one_pixel(255, 0, 0, CmykProcess::Cyan, 0.75), 255);
        // Magenta and yellow should have full ink.
        assert_eq!(one_pixel(255, 0, 0, CmykProcess::Magenta, 0.0), 0);
        assert_eq!(one_pixel(255, 0, 0, CmykProcess::Yellow, 0.0), 0);
    }

    #[test]
    fn pure_white_is_no_ink() {
        for ch in [
            CmykProcess::Cyan,
            CmykProcess::Magenta,
            CmykProcess::Yellow,
            CmykProcess::Black,
        ] {
            assert_eq!(one_pixel(255, 255, 255, ch, 1.0), 255);
        }
    }

    #[test]
    fn pure_black_full_gcr() {
        // With full GCR, black should be entirely on the K plate.
        // C, M, Y should be zero (density 255).
        assert_eq!(one_pixel(0, 0, 0, CmykProcess::Black, 1.0), 0);
        assert_eq!(one_pixel(0, 0, 0, CmykProcess::Cyan, 1.0), 255);
        assert_eq!(one_pixel(0, 0, 0, CmykProcess::Magenta, 1.0), 255);
        assert_eq!(one_pixel(0, 0, 0, CmykProcess::Yellow, 1.0), 255);
    }

    #[test]
    fn mid_gray_gcr_transfers_to_k() {
        // sRGB(128,128,128) → linear ≈ 0.216. CMY all ≈ 0.784.
        // With GCR=1.0: K=0.784, C=M=Y=0.
        let k = one_pixel(128, 128, 128, CmykProcess::Black, 1.0);
        assert!(k < 60, "mid-gray K should have heavy ink, got density {k}");
        let c = one_pixel(128, 128, 128, CmykProcess::Cyan, 1.0);
        assert!(c > 240, "mid-gray C with full GCR should have near-zero ink, got {c}");
    }
}
