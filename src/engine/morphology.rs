//! Morphological operations on single-channel density maps.
//!
//! Remember the engine-wide density convention: **0 = full ink, 255 = no
//! ink.** "Erode ink" means *grow the white* — we shrink the region where
//! ink prints. This is the opposite of textbook morphology where erode
//! shrinks the foreground, so pay attention to the helper names and not
//! instinct.

use image::{GrayImage, ImageBuffer, Luma};
use imageproc::filter::gaussian_blur_f32;

/// Shrink the inked region by `radius` pixels. Internally: box dilate of
/// the brightness channel (max filter), so the "no-ink" regions grow.
pub fn erode_ink(img: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return img.clone();
    }
    max_filter(img, radius)
}

/// Grow the inked region by `radius` pixels. Box erode of the brightness
/// channel (min filter) — "ink" expands because zeros spread.
pub fn dilate_ink(img: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return img.clone();
    }
    min_filter(img, radius)
}

/// Remove specks: erode then dilate. Kills isolated dark pixels without
/// thinning large fills.
pub fn open_ink(img: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return img.clone();
    }
    dilate_ink(&erode_ink(img, radius), radius)
}

/// Fill small holes: dilate then erode. Closes pinholes inside solid ink
/// regions without bloating edges.
pub fn close_ink(img: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return img.clone();
    }
    erode_ink(&dilate_ink(img, radius), radius)
}

/// Gaussian blur on the density map. Preserves grayscale ramps — this is
/// the right tool for smoothing soft edges before halftoning.
pub fn smooth_mask(img: &GrayImage, radius: f32) -> GrayImage {
    if radius <= 0.0 {
        return img.clone();
    }
    gaussian_blur_f32(img, radius)
}

/// Same as [`smooth_mask`]; kept as a distinct name because the Python
/// reference code uses it at a different point in the pipeline (feathering
/// just before halftone rasterization).
pub fn feather_halftone_edge(img: &GrayImage, radius: f32) -> GrayImage {
    smooth_mask(img, radius)
}

// ---------------------------------------------------------------------------
// Circular (Euclidean distance) filters
// ---------------------------------------------------------------------------

/// Circular max filter with radius `r`. Uses a precomputed disk mask
/// so edges erode/dilate smoothly instead of in blocky squares.
fn max_filter(img: &GrayImage, r: u32) -> GrayImage {
    let offsets = disk_offsets(r);
    apply_filter(img, &offsets, true)
}

fn min_filter(img: &GrayImage, r: u32) -> GrayImage {
    let offsets = disk_offsets(r);
    apply_filter(img, &offsets, false)
}

/// Precompute the (dx, dy) offsets within a circle of radius `r`.
fn disk_offsets(r: u32) -> Vec<(i32, i32)> {
    let r_i = r as i32;
    let r_sq = (r_i * r_i) as f32;
    let mut offsets = Vec::new();
    for dy in -r_i..=r_i {
        for dx in -r_i..=r_i {
            if (dx * dx + dy * dy) as f32 <= r_sq {
                offsets.push((dx, dy));
            }
        }
    }
    offsets
}

fn apply_filter(img: &GrayImage, offsets: &[(i32, i32)], take_max: bool) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = ImageBuffer::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let mut acc: u8 = img.get_pixel(x, y)[0];
            for &(dx, dy) in offsets {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                    let v = img.get_pixel(nx as u32, ny as u32)[0];
                    acc = if take_max { acc.max(v) } else { acc.min(v) };
                }
            }
            out.put_pixel(x, y, Luma([acc]));
        }
    }
    out
}

// TODO(L2): the separable pass above is O(w · h · r). For the radii we
// use in practice (≤ 8 px) that's fine, but if we ever need bigger
// radii we should switch to a rolling histogram so the per-pixel cost
// becomes O(1) in `r`.

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn checker(w: u32, h: u32) -> GrayImage {
        ImageBuffer::from_fn(w, h, |x, y| {
            if (x + y) % 2 == 0 {
                Luma([0])
            } else {
                Luma([255])
            }
        })
    }

    #[test]
    fn zero_radius_is_identity() {
        let src = checker(6, 6);
        assert_eq!(erode_ink(&src, 0).into_raw(), src.clone().into_raw());
        assert_eq!(dilate_ink(&src, 0).into_raw(), src.clone().into_raw());
        assert_eq!(open_ink(&src, 0).into_raw(), src.clone().into_raw());
        assert_eq!(close_ink(&src, 0).into_raw(), src.into_raw());
    }

    #[test]
    fn dilate_expands_ink() {
        // Single dark pixel in the middle of white. Dilate (min filter)
        // with circular kernel spreads 0 to the 4-connected neighbors
        // (cross pattern), not the full 3×3 square.
        let mut img = ImageBuffer::from_pixel(5, 5, Luma([255]));
        img.put_pixel(2, 2, Luma([0]));
        let out = dilate_ink(&img, 1);
        // Center + 4-connected neighbors should be ink.
        assert_eq!(out.get_pixel(2, 2)[0], 0);
        assert_eq!(out.get_pixel(1, 2)[0], 0);
        assert_eq!(out.get_pixel(3, 2)[0], 0);
        assert_eq!(out.get_pixel(2, 1)[0], 0);
        assert_eq!(out.get_pixel(2, 3)[0], 0);
        // Diagonal corners stay white (distance √2 > radius 1).
        assert_eq!(out.get_pixel(1, 1)[0], 255);
        assert_eq!(out.get_pixel(3, 3)[0], 255);
        assert_eq!(out.get_pixel(0, 0)[0], 255);
    }

    #[test]
    fn erode_shrinks_ink() {
        // Single dark pixel inside white. r=1 max filter is enough to
        // wipe it completely. (A 3×3 block wouldn't erode at r=1 because
        // the center pixel's neighborhood is still entirely inside the
        // block — that's a geometric limit of the kernel, not a bug.)
        let mut img = ImageBuffer::from_pixel(5, 5, Luma([255]));
        img.put_pixel(2, 2, Luma([0]));
        let out = erode_ink(&img, 1);
        for y in 0..5 {
            for x in 0..5 {
                assert_eq!(out.get_pixel(x, y)[0], 255);
            }
        }
    }
}
