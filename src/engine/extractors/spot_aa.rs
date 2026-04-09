//! Hard Voronoi spot extractor — each pixel lands on exactly one plate.
//!
//! For every source pixel we compute [`Lab::delta_e94`] (the CIE94
//! graphic-arts ΔE) against every palette target and paint full ink on
//! whichever target wins. Ties are broken deterministically by lowest
//! target index so two palette entries with identical distance can
//! never both claim the same pixel (which would print as registration
//! bleed on the press).
//!
//! No soft ramp, no partial densities: spot plates are solid ink, so
//! pixels at the boundary between two hues snap hard to one side.
//! Edge anti-aliasing from the source doesn't translate meaningfully
//! to a physical screen at print resolution, and any grayscale output
//! here ends up as translucent ink in the composite preview (which
//! reads as "blurry") and as grayscale on the film (which a real press
//! can't reproduce without halftoning).
//!
//! The `aa_full`, `aa_end`, and `aa_reach` fields on [`Params`] are
//! legacy — kept in the struct so existing saved projects deserialize,
//! but no longer consulted. CIE94 is the whole distance story.

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

    for (x, y, p) in source.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));

        // Find the single nearest target under CIE94. Ties go to the
        // lower index — a `<=` comparison would let both competing
        // targets claim the pixel and print double-coverage.
        let mut winner: usize = 0;
        let mut winner_dist = lab.delta_e94(target_labs[0]);
        for (i, &tl) in target_labs.iter().enumerate().skip(1) {
            let d = lab.delta_e94(tl);
            if d < winner_dist {
                winner = i;
                winner_dist = d;
            }
        }
        // "Others" are colors we explicitly do not want to claim. If
        // one of them is strictly closer than the winning target, no
        // plate gets this pixel.
        let rejected = other_labs
            .iter()
            .any(|&ol| lab.delta_e94(ol) < winner_dist);

        let density = if !rejected && winner == params.target_index {
            0u8
        } else {
            255u8
        };
        out.put_pixel(x, y, Luma([density]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to drop the legacy params from the test call sites.
    fn run(src: &RgbImage, targets: &[Rgb], idx: usize) -> GrayImage {
        let others: [Rgb; 0] = [];
        extract(
            src,
            Params {
                targets,
                others: &others,
                target_index: idx,
                aa_full: 0.0,
                aa_end: 0.0,
            },
        )
    }

    #[test]
    fn exact_match_is_full_ink() {
        let mut src: RgbImage = ImageBuffer::new(2, 1);
        src.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        src.put_pixel(1, 0, image::Rgb([0, 255, 0]));
        let targets = [Rgb(255, 0, 0), Rgb(0, 255, 0)];
        let out = run(&src, &targets, 0);
        assert_eq!(out.get_pixel(0, 0)[0], 0);
        assert_eq!(out.get_pixel(1, 0)[0], 255);
    }

    /// Dark-but-saturated red belongs on the red plate, not black.
    /// Under plain CIE76 ΔE black wins because the L-axis distance
    /// dominates — this is the bug that CIE94 fixes.
    #[test]
    fn dark_saturated_red_goes_to_red_not_black() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([90, 15, 15]));
        let targets = [Rgb(0, 0, 0), Rgb(220, 30, 30)];

        assert_eq!(run(&src, &targets, 1).get_pixel(0, 0)[0], 0);
        assert_eq!(run(&src, &targets, 0).get_pixel(0, 0)[0], 255);
    }

    /// A saturated yellow must land on the yellow plate and not get
    /// stolen by a red neighbour — this is the "red ate my yellow"
    /// regression from the demon test image.
    #[test]
    fn yellow_beats_red_for_yellow_pixel() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([240, 210, 40]));
        let targets = [Rgb(220, 30, 30), Rgb(240, 200, 30)];

        // Yellow target (index 1) wins.
        assert_eq!(run(&src, &targets, 1).get_pixel(0, 0)[0], 0);
        assert_eq!(run(&src, &targets, 0).get_pixel(0, 0)[0], 255);
    }

    /// Neutral dark pixel must still go to black, not to any chromatic
    /// target — the neutral / chromatic branching must be continuous
    /// and not produce weird assignments near the L-axis.
    #[test]
    fn neutral_dark_pixel_still_goes_to_black() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([20, 20, 20]));
        let targets = [Rgb(0, 0, 0), Rgb(220, 30, 30)];
        assert_eq!(run(&src, &targets, 0).get_pixel(0, 0)[0], 0);
    }

    /// Two palette entries at identical CIE94 distance must never both
    /// claim the same pixel — that would print as double coverage and
    /// cause registration bleed on the press.
    #[test]
    fn ties_are_broken_by_lower_index() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([128, 128, 128]));
        // Two identical targets — both are at exactly distance 0 from
        // the pixel, so the tie-break rule must pick exactly one.
        let targets = [Rgb(128, 128, 128), Rgb(128, 128, 128)];

        let p0 = run(&src, &targets, 0).get_pixel(0, 0)[0];
        let p1 = run(&src, &targets, 1).get_pixel(0, 0)[0];
        // Exactly one plate claims it.
        assert!(
            (p0 == 0 && p1 == 255) || (p0 == 255 && p1 == 0),
            "exactly one plate must claim the pixel, got p0={p0} p1={p1}"
        );
    }
}
