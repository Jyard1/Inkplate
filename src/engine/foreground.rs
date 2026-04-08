//! Background detection and foreground masking.
//!
//! Two jobs, in order: find the dominant background color of an image,
//! then build a boolean foreground mask. The foreground mask is used
//! by the pipeline to clamp every layer's output to "no ink" outside
//! the art — otherwise a dark source background gets pulled into the
//! black plate, a bright source background gets pulled into the
//! underbase, and so on.
//!
//! Strategy:
//!
//! - **alpha channel** — if the caller provides an RGBA source, trust
//!   it directly. `alpha ≥ 128` is foreground, everything else is
//!   background. No heuristics needed.
//! - **edge-seeded flood fill** — otherwise, sample the four edges of
//!   the image to estimate the background color, then flood-fill from
//!   every border pixel that matches, stopping when the neighbor's
//!   LAB ΔE to the seed color exceeds the caller-provided tolerance.
//!   The result is "everything connected to the border by similar
//!   color" = background. Anything else = foreground.
//!
//! The flood fill is iterative (a Vec-backed stack) so deep images
//! don't blow the call stack. Memory is one byte per pixel for the
//! visited grid.

use std::collections::VecDeque;

use image::{GrayImage, ImageBuffer, Luma, RgbImage, RgbaImage};

use crate::engine::color::{rgb_to_lab, Lab, Rgb};

/// Return the most common pixel color in an RGB image.
pub fn detect_background_rgb(img: &RgbImage) -> Rgb {
    use std::collections::HashMap;
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for p in img.pixels() {
        *counts.entry(p.0).or_insert(0) += 1;
    }
    let best = counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(rgb, _)| rgb)
        .unwrap_or([255, 255, 255]);
    Rgb::from_array(best)
}

/// Sample border pixels and return the most common color along the
/// edges. More accurate than `detect_background_rgb` when the art
/// dominates the interior, because border sampling only looks at the
/// frame.
pub fn detect_background_from_border(img: &RgbImage) -> Rgb {
    use std::collections::HashMap;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Rgb::WHITE;
    }
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    let mut count = |p: &image::Rgb<u8>| {
        *counts.entry(p.0).or_insert(0) += 1;
    };
    for x in 0..w {
        count(img.get_pixel(x, 0));
        count(img.get_pixel(x, h - 1));
    }
    for y in 0..h {
        count(img.get_pixel(0, y));
        count(img.get_pixel(w - 1, y));
    }
    let best = counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(rgb, _)| rgb)
        .unwrap_or([255, 255, 255]);
    Rgb::from_array(best)
}

/// Build a foreground mask (255 = foreground / art, 0 = background).
///
/// If `alpha` is provided, that short-circuits the heuristic. Otherwise
/// an edge-seeded flood fill is performed against the dominant border
/// color with `tolerance_delta_e` as the LAB ΔE stopping threshold.
pub fn detect_foreground_mask(
    img: &RgbImage,
    alpha: Option<&RgbaImage>,
    tolerance_delta_e: f32,
) -> GrayImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return ImageBuffer::new(w, h);
    }

    if let Some(a) = alpha {
        let mut out: GrayImage = ImageBuffer::new(w, h);
        for (x, y, p) in a.enumerate_pixels() {
            out.put_pixel(x, y, Luma([if p.0[3] >= 128 { 255 } else { 0 }]));
        }
        return out;
    }

    flood_fill_background(img, tolerance_delta_e)
}

/// Edge-seeded BFS flood fill. Everything reached from any border
/// pixel (while the color stays within `tol` LAB ΔE of the border
/// seed) becomes background; everything else becomes foreground.
fn flood_fill_background(img: &RgbImage, tol: f32) -> GrayImage {
    let (w, h) = img.dimensions();
    let bg_seed = rgb_to_lab(detect_background_from_border(img));

    // 1 byte per pixel: 0 = unknown, 1 = visited (background).
    let mut visited = vec![0u8; (w * h) as usize];
    let idx = |x: u32, y: u32| (y * w + x) as usize;

    let matches = |p: &image::Rgb<u8>| -> bool {
        let lab: Lab = rgb_to_lab(Rgb::from_array(p.0));
        lab.delta_e(bg_seed) < tol
    };

    let mut queue: VecDeque<(u32, u32)> = VecDeque::new();

    // Seed every border pixel that matches the background color.
    for x in 0..w {
        for &y in &[0u32, h - 1] {
            if matches(img.get_pixel(x, y)) && visited[idx(x, y)] == 0 {
                visited[idx(x, y)] = 1;
                queue.push_back((x, y));
            }
        }
    }
    for y in 0..h {
        for &x in &[0u32, w - 1] {
            if matches(img.get_pixel(x, y)) && visited[idx(x, y)] == 0 {
                visited[idx(x, y)] = 1;
                queue.push_back((x, y));
            }
        }
    }

    // 4-connected flood fill.
    while let Some((x, y)) = queue.pop_front() {
        let neighbors = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in neighbors {
            if nx >= w || ny >= h {
                continue;
            }
            let i = idx(nx, ny);
            if visited[i] != 0 {
                continue;
            }
            if matches(img.get_pixel(nx, ny)) {
                visited[i] = 1;
                queue.push_back((nx, ny));
            }
        }
    }

    // Foreground = everything NOT visited.
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for (i, v) in visited.iter().enumerate() {
        let x = (i as u32) % w;
        let y = (i as u32) / w;
        out.put_pixel(x, y, Luma([if *v == 0 { 255 } else { 0 }]));
    }
    out
}

/// Clamp `density` to "no ink" (255) wherever `foreground` is 0. Does
/// nothing if the dimensions don't match.
///
/// This is the post-extraction knockout variant. It's still exposed for
/// cases where the caller wants to mask a density map directly, but the
/// preferred path for the GUI is [`apply_mask_to_source`] — substituting
/// pure black for background pixels *before* extraction, so the
/// extractors never see the background in the first place.
pub fn apply_mask_inplace(density: &mut GrayImage, foreground: &GrayImage) {
    if density.dimensions() != foreground.dimensions() {
        return;
    }
    for (d, f) in density.iter_mut().zip(foreground.iter()) {
        if *f == 0 {
            *d = 255;
        }
    }
}

/// Produce a new source image with every background pixel replaced by
/// `replacement`. Foreground pixels are copied through unchanged.
///
/// Choice of replacement color matters — different extractors respond
/// differently to black vs white backgrounds:
///
/// | Replacement | Underbase | Black plate | Color channels | Highlight white |
/// |-------------|-----------|-------------|----------------|-----------------|
/// | White       | full ink  | no ink      | no ink         | full ink        |
/// | Black       | no ink    | full ink    | no ink         | no ink          |
/// | Shirt color | varies    | varies      | mostly no ink  | varies          |
///
/// **White is the default** because it matches the Photoshop
/// "color range needs a white background" assumption that every
/// sim-process workflow preset is built around. The UNDERBASE and
/// HIGHLIGHT_WHITE tone curves are meant to be paired with a
/// foreground-isolated source that has white where the art isn't.
/// If you're using the underbase or highlight screens on a source
/// whose background is already what you want, flip the replacement
/// to match the shirt color or disable bg removal entirely.
pub fn apply_mask_to_source(
    source: &RgbImage,
    foreground: &GrayImage,
    replacement: Rgb,
) -> RgbImage {
    let (w, h) = source.dimensions();
    if foreground.dimensions() != (w, h) {
        return source.clone();
    }
    let mut out = source.clone();
    let fill = image::Rgb([replacement.0, replacement.1, replacement.2]);
    for (x, y, p) in out.enumerate_pixels_mut() {
        if foreground.get_pixel(x, y)[0] == 0 {
            *p = fill;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;

    #[test]
    fn flood_fill_isolates_interior_blob() {
        // 10x10 white background with a 4x4 dark blob in the middle.
        // Flood fill should mark everything outside the blob as background.
        let mut img: RgbImage = ImageBuffer::from_pixel(10, 10, image::Rgb([255, 255, 255]));
        for y in 3..=6 {
            for x in 3..=6 {
                img.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let mask = detect_foreground_mask(&img, None, 10.0);
        // Interior blob = foreground
        assert_eq!(mask.get_pixel(4, 4)[0], 255);
        // Outside = background
        assert_eq!(mask.get_pixel(0, 0)[0], 0);
        assert_eq!(mask.get_pixel(9, 9)[0], 0);
    }

    #[test]
    fn interior_color_matching_background_survives() {
        // This is the regression the old global-ΔE version failed.
        // An image with a white interior region on a white frame
        // should keep the interior as FOREGROUND as long as the
        // interior isn't reachable from the border through a white
        // path. Here the interior white is surrounded by black, so it
        // should survive.
        let mut img: RgbImage = ImageBuffer::from_pixel(10, 10, image::Rgb([255, 255, 255]));
        // Black ring
        for y in 2..=7 {
            for x in 2..=7 {
                img.put_pixel(x, y, image::Rgb([0, 0, 0]));
            }
        }
        // White interior
        for y in 4..=5 {
            for x in 4..=5 {
                img.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
        let mask = detect_foreground_mask(&img, None, 15.0);
        assert_eq!(
            mask.get_pixel(4, 4)[0],
            255,
            "interior white should be foreground"
        );
        assert_eq!(mask.get_pixel(0, 0)[0], 0, "border is background");
    }

    #[test]
    fn mask_knockout_clamps_density() {
        let mut density = ImageBuffer::from_fn(4, 1, |x, _| Luma([(x * 40) as u8]));
        let fg = ImageBuffer::from_fn(4, 1, |x, _| Luma([if x < 2 { 255u8 } else { 0 }]));
        apply_mask_inplace(&mut density, &fg);
        assert_eq!(density.get_pixel(0, 0)[0], 0);
        assert_eq!(density.get_pixel(1, 0)[0], 40);
        assert_eq!(density.get_pixel(2, 0)[0], 255);
        assert_eq!(density.get_pixel(3, 0)[0], 255);
    }
}
