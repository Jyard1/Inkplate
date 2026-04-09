//! Index-assignment extractor — LAB-space palette dither, one layer per
//! palette entry.
//!
//! Index separations convert an arbitrary image into N layers where
//! every pixel is mapped to **exactly one** of N palette colors via an
//! error-diffused or ordered dither. No halftones — every "dot" is one
//! pixel, which is why index mode avoids moiré entirely and works well
//! for pixel art and sharp-edged illustration.
//!
//! Each layer extracts only the pixels assigned to its own palette
//! index; the other pixels come back as "no ink". Running all N layers
//! gives you the full decomposition.
//!
//! Two dither strategies:
//!
//! - **FS (Floyd-Steinberg)** — serpentine error diffusion in LAB space.
//!   Highest quality, slowest. This is the default.
//! - **Bayer** — 8×8 ordered dither. Faster, produces a uniform pattern,
//!   good for retro / halftone-style looks.
//!
//! Both walk the full image once, so the same palette/image pair
//! repeats work across N layers. See the TODO below for caching.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::{rgb_to_lab, Lab, Rgb};
use crate::engine::layer::IndexDitherKind;

// TODO(L3): cache the full palette assignment array keyed on
// `(image-hash, palette-hash, dither-kind)` so that N layers sharing one
// palette only dither once. See `_INDEX_DITHER_CACHE` in the Python
// reference. Current code re-runs the whole dither per layer.

pub fn extract(source: &RgbImage, palette: &[Rgb], index: u32, kind: IndexDitherKind) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::from_pixel(w, h, Luma([255u8]));
    if palette.is_empty() || index as usize >= palette.len() {
        return out;
    }
    let palette_lab: Vec<Lab> = palette.iter().copied().map(rgb_to_lab).collect();

    let assignments = match kind {
        IndexDitherKind::Fs => floyd_steinberg_lab(source, &palette_lab),
        IndexDitherKind::Bayer => bayer_lab(source, &palette_lab),
    };

    for (i, assigned) in assignments.iter().enumerate() {
        if *assigned == index as u16 {
            let x = (i as u32) % w;
            let y = (i as u32) / w;
            out.put_pixel(x, y, Luma([0]));
        }
    }
    out
}

/// Serpentine Floyd-Steinberg error diffusion in LAB space.
/// Returns a flat H×W vector of palette indices.
fn floyd_steinberg_lab(source: &RgbImage, palette: &[Lab]) -> Vec<u16> {
    let (w, h) = source.dimensions();
    let mut buf: Vec<Lab> = Vec::with_capacity((w * h) as usize);
    for p in source.pixels() {
        buf.push(rgb_to_lab(Rgb(p[0], p[1], p[2])));
    }

    let stride = w as usize;
    let mut assign = vec![0u16; buf.len()];

    for y in 0..h as usize {
        let left_to_right = y % 2 == 0;
        let xs: Box<dyn Iterator<Item = usize>> = if left_to_right {
            Box::new(0..stride)
        } else {
            Box::new((0..stride).rev())
        };

        for x in xs {
            let i = y * stride + x;
            let old = buf[i];
            let best = nearest_palette_index(old, palette);
            let new = palette[best];
            assign[i] = best as u16;

            let err_l = old.l - new.l;
            let err_a = old.a - new.a;
            let err_b = old.b - new.b;

            let (dx1, dx2) = if left_to_right {
                (1i32, -1i32)
            } else {
                (-1i32, 1i32)
            };
            let push = |buf: &mut [Lab], xi: i32, yi: usize, wgt: f32| {
                if xi < 0 || xi >= stride as i32 {
                    return;
                }
                let j = yi * stride + xi as usize;
                buf[j].l += err_l * wgt;
                buf[j].a += err_a * wgt;
                buf[j].b += err_b * wgt;
            };

            push(&mut buf, x as i32 + dx1, y, 7.0 / 16.0);
            if y + 1 < h as usize {
                push(&mut buf, x as i32 + dx2, y + 1, 3.0 / 16.0);
                push(&mut buf, x as i32, y + 1, 5.0 / 16.0);
                push(&mut buf, x as i32 + dx1, y + 1, 1.0 / 16.0);
            }
        }
    }
    assign
}

/// 8×8 Bayer ordered dither in LAB space. The matrix biases the L
/// channel to spread similar colors across the threshold.
fn bayer_lab(source: &RgbImage, palette: &[Lab]) -> Vec<u16> {
    // Matches the 8×8 matrix in `engine::dither::bayer`.
    const M: [[u8; 8]; 8] = [
        [0, 32, 8, 40, 2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44, 4, 36, 14, 46, 6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [3, 35, 11, 43, 1, 33, 9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47, 7, 39, 13, 45, 5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];

    let (w, h) = source.dimensions();
    let mut assign = vec![0u16; (w * h) as usize];
    for (i, p) in source.pixels().enumerate() {
        let x = (i as u32) % w;
        let y = (i as u32) / w;
        let mut lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));
        // Rescale bayer value to a small L-channel offset in [-8, +8].
        let bias = (M[(y % 8) as usize][(x % 8) as usize] as f32 - 31.5) / 31.5 * 8.0;
        lab.l += bias;
        assign[i] = nearest_palette_index(lab, palette) as u16;
    }
    assign
}

fn nearest_palette_index(lab: Lab, palette: &[Lab]) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (i, &pl) in palette.iter().enumerate() {
        let d = lab.delta_e94(pl);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_get_assigned() {
        // 2-color image, 2-color palette. Each layer should claim half
        // the pixels exactly.
        let mut src: RgbImage = ImageBuffer::new(4, 1);
        src.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        src.put_pixel(1, 0, image::Rgb([255, 0, 0]));
        src.put_pixel(2, 0, image::Rgb([0, 0, 255]));
        src.put_pixel(3, 0, image::Rgb([0, 0, 255]));
        let palette = [Rgb(255, 0, 0), Rgb(0, 0, 255)];
        let layer0 = extract(&src, &palette, 0, IndexDitherKind::Fs);
        let layer1 = extract(&src, &palette, 1, IndexDitherKind::Fs);
        assert_eq!(layer0.get_pixel(0, 0)[0], 0);
        assert_eq!(layer0.get_pixel(2, 0)[0], 255);
        assert_eq!(layer1.get_pixel(0, 0)[0], 255);
        assert_eq!(layer1.get_pixel(2, 0)[0], 0);
    }

    #[test]
    fn out_of_range_index_is_blank() {
        let src: RgbImage = ImageBuffer::from_pixel(2, 2, image::Rgb([128, 128, 128]));
        let palette = [Rgb(0, 0, 0)];
        let out = extract(&src, &palette, 5, IndexDitherKind::Bayer);
        for p in out.iter() {
            assert_eq!(*p, 255);
        }
    }
}
