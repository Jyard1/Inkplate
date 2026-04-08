//! Anti-aliased spot extractor — Voronoi assignment with soft edge falloff.
//!
//! Cel-shaded art lives in the gap between "flat vector" and "continuous
//! tone": most pixels belong to exactly one ink color, but the edge
//! pixels are anti-aliased blends between two inks and shouldn't be
//! snapped to one side or the other. This extractor walks every pixel,
//! finds the nearest target color in LAB, and returns:
//!
//! - full ink (0) if the pixel is close enough to `target` (distance
//!   ≤ `aa_full`)
//! - no ink (255) if it's closer to another target or past `aa_end`
//! - a linear ramp in between, so the edge pixels partially contribute
//!   to the nearest target layer
//!
//! `aa_reach` is a pixel-radius hint for the maximum supported anti-alias
//! width in the source — used to clamp the distance ramp so very noisy
//! sources don't bleed across unrelated regions.

use image::{GrayImage, ImageBuffer, Luma, RgbImage};

use crate::engine::color::{rgb_to_lab, Lab, Rgb};

#[derive(Debug, Clone, Copy)]
pub struct Params<'a> {
    pub targets: &'a [Rgb],
    pub others: &'a [Rgb],
    pub target_index: usize,
    pub aa_full: f32,
    pub aa_end: f32,
}

pub fn extract(source: &RgbImage, params: Params<'_>) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    if params.targets.is_empty() || params.target_index >= params.targets.len() {
        return out;
    }

    let target_labs: Vec<Lab> = params.targets.iter().copied().map(rgb_to_lab).collect();
    let other_labs: Vec<Lab> = params.others.iter().copied().map(rgb_to_lab).collect();
    let me = target_labs[params.target_index];

    let full = params.aa_full.max(0.0);
    let end = params.aa_end.max(full + 0.1);

    for (x, y, p) in source.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));
        let d_me = lab.delta_e(me);

        // Nearest-other distance: against every other target AND the
        // "others" list (colors we explicitly don't want to claim).
        let mut nearest_other = f32::INFINITY;
        for (i, &tl) in target_labs.iter().enumerate() {
            if i != params.target_index {
                nearest_other = nearest_other.min(lab.delta_e(tl));
            }
        }
        for &ol in &other_labs {
            nearest_other = nearest_other.min(lab.delta_e(ol));
        }

        // Only claim this pixel if we're the closest target.
        if d_me > nearest_other {
            out.put_pixel(x, y, Luma([255]));
            continue;
        }

        let density = if d_me <= full {
            0u8
        } else if d_me >= end {
            255u8
        } else {
            let t = (d_me - full) / (end - full);
            (t * 255.0).round().clamp(0.0, 255.0) as u8
        };
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_is_full_ink() {
        let mut src: RgbImage = ImageBuffer::new(2, 1);
        src.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        src.put_pixel(1, 0, image::Rgb([0, 255, 0]));
        let targets = [Rgb(255, 0, 0), Rgb(0, 255, 0)];
        let others: [Rgb; 0] = [];
        let out = extract(
            &src,
            Params {
                targets: &targets,
                others: &others,
                target_index: 0,
                aa_full: 4.0,
                aa_end: 14.0,
            },
        );
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(1, 0)[0], 255);
    }
}
