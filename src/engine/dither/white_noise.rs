//! Pure white-noise dither. Cheapest possible path — useful for preview
//! thumbnails and as a baseline the nicer algorithms are measured against.

use image::{GrayImage, ImageBuffer, Luma};

/// Threshold against uniform white noise. The RNG is a fast xorshift so
/// repeated runs on the same input are *not* bit-identical — callers that
/// need determinism should snapshot the output.
pub fn white_noise_grayscale(src: &GrayImage) -> GrayImage {
    let (w, h) = src.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let mut state: u32 = 0x9E37_79B9;
    for (x, y, p) in src.enumerate_pixels() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let threshold = (state & 0xFF) as u8;
        let bit = if p[0] > threshold { 255 } else { 0 };
        out.put_pixel(x, y, Luma([bit]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_output() {
        let src: GrayImage = ImageBuffer::from_fn(8, 8, |x, _| Luma([(x * 30) as u8]));
        let out = white_noise_grayscale(&src);
        for p in out.iter() {
            assert!(*p == 0 || *p == 255);
        }
    }
}
