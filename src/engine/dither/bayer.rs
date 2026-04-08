//! Ordered (Bayer) dither. Faster and more uniform than error diffusion,
//! at the cost of a visible repeating pattern. Cached 2/4/8 matrices.

use image::{GrayImage, ImageBuffer, Luma};

const BAYER_2: [[u8; 2]; 2] = [[0, 2], [3, 1]];
const BAYER_4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
const BAYER_8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Matrix size: 2, 4, or 8. Anything else falls back to 8.
pub fn bayer_grayscale(src: &GrayImage, matrix_size: u32) -> GrayImage {
    let (w, h) = src.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    match matrix_size {
        2 => apply(src, &mut out, &BAYER_2, 2, 4),
        4 => apply(src, &mut out, &BAYER_4, 4, 16),
        _ => apply(src, &mut out, &BAYER_8, 8, 64),
    }
    out
}

fn apply<const N: usize>(
    src: &GrayImage,
    out: &mut GrayImage,
    matrix: &[[u8; N]; N],
    size: u32,
    scale: u32,
) {
    let (w, h) = src.dimensions();
    for y in 0..h {
        for x in 0..w {
            let m = matrix[(y % size) as usize][(x % size) as usize] as u32;
            // Threshold = (m + 0.5) * 256 / scale
            let threshold = ((m * 256 + 128) / scale) as u8;
            let src_val = src.get_pixel(x, y)[0];
            let bit = if src_val > threshold { 255 } else { 0 };
            out.put_pixel(x, y, Luma([bit]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_output() {
        let src: GrayImage = ImageBuffer::from_fn(8, 8, |x, _| Luma([(x * 30) as u8]));
        let out = bayer_grayscale(&src, 8);
        for p in out.iter() {
            assert!(*p == 0 || *p == 255);
        }
    }

    #[test]
    fn all_matrices_work() {
        let src: GrayImage = ImageBuffer::from_pixel(16, 16, Luma([128]));
        for s in [2, 4, 8] {
            let out = bayer_grayscale(&src, s);
            assert_eq!(out.dimensions(), (16, 16));
        }
    }
}
