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
// Low-level box filters
// ---------------------------------------------------------------------------

/// Square max filter with radius `r` (kernel side `2r + 1`). Separable
/// implementation: horizontal pass, then vertical.
fn max_filter(img: &GrayImage, r: u32) -> GrayImage {
    let pass1 = separable_pass(img, r, true, true);
    separable_pass(&pass1, r, false, true)
}

fn min_filter(img: &GrayImage, r: u32) -> GrayImage {
    let pass1 = separable_pass(img, r, true, false);
    separable_pass(&pass1, r, false, false)
}

/// One axis of a box min/max filter.
fn separable_pass(img: &GrayImage, r: u32, horizontal: bool, take_max: bool) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = ImageBuffer::new(w, h);
    let r = r as i32;

    let extreme = |a: u8, b: u8| if take_max { a.max(b) } else { a.min(b) };

    for y in 0..h {
        for x in 0..w {
            let mut acc: u8 = img.get_pixel(x, y)[0];
            if horizontal {
                let x0 = (x as i32 - r).max(0);
                let x1 = (x as i32 + r).min(w as i32 - 1);
                for xi in x0..=x1 {
                    acc = extreme(acc, img.get_pixel(xi as u32, y)[0]);
                }
            } else {
                let y0 = (y as i32 - r).max(0);
                let y1 = (y as i32 + r).min(h as i32 - 1);
                for yi in y0..=y1 {
                    acc = extreme(acc, img.get_pixel(x, yi as u32)[0]);
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
        // should spread the 0 across a 3×3 region.
        let mut img = ImageBuffer::from_pixel(5, 5, Luma([255]));
        img.put_pixel(2, 2, Luma([0]));
        let out = dilate_ink(&img, 1);
        for y in 1..=3 {
            for x in 1..=3 {
                assert_eq!(out.get_pixel(x, y)[0], 0);
            }
        }
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
