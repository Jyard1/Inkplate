//! Soft-edge Voronoi spot extractor — each pixel lands on exactly one
//! plate, with a smooth gradient at boundaries.
//!
//! For every source pixel we compute [`Lab::delta_e94`] (the CIE94
//! graphic-arts ΔE) against every palette target. The winning target
//! (lowest distance, ties broken by lowest index) claims the pixel.
//!
//! Instead of outputting hard 0/255, the extractor produces a gradient
//! at Voronoi boundaries: pixels deep inside a color region get full
//! ink (0) or full no-ink (255), while pixels near the boundary get
//! intermediate values based on the distance margin between the
//! winning and runner-up targets. The pipeline's Gaussian blur then
//! smooths this gradient, and the binarize step (threshold 128) traces
//! a clean, non-jagged contour that follows the true color boundary
//! instead of a pixel-aligned staircase.
//!
//! Deterministic tie-breaking (±0.5 density bias toward the Voronoi
//! winner) guarantees that exactly one plate claims each pixel after
//! binarization — no double coverage on the press.
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
    /// Per-target distance offsets, parallel to `targets`. When
    /// `Some`, the effective distance to `targets[i]` is
    /// `delta_e94(...) − target_weights[i]`. Positive weight makes
    /// that plate "reach further" and claim pixels it would
    /// otherwise lose. When `None`, plain CIE94 Voronoi applies.
    pub target_weights: Option<&'a [f32]>,
}

pub fn extract(source: &RgbImage, params: Params<'_>) -> GrayImage {
    let (w, h) = source.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    if params.targets.is_empty() || params.target_index >= params.targets.len() {
        return out;
    }

    let target_labs: Vec<Lab> = params.targets.iter().copied().map(rgb_to_lab).collect();
    let other_labs: Vec<Lab> = params.others.iter().copied().map(rgb_to_lab).collect();
    let weight_of = |i: usize| -> f32 {
        params
            .target_weights
            .and_then(|w| w.get(i).copied())
            .unwrap_or(0.0)
    };

    // Soft-edge ramp width in ΔE94 units. Within ±RAMP of the Voronoi
    // boundary, pixels get a gradient instead of a hard 0/255 snap.
    // The pipeline's Gaussian blur smooths this gradient and the
    // binarize step (threshold 128) traces a clean contour — no more
    // staircase aliasing at color boundaries.
    const RAMP: f32 = 6.0;
    let scale = 128.0 / RAMP;

    for (x, y, p) in source.enumerate_pixels() {
        let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));

        // Single pass: find the Voronoi winner (for tie-breaking),
        // our distance, and the closest competing target's distance.
        let mut winner: usize = 0;
        let mut winner_dist = f32::MAX;
        let mut d_self = f32::MAX;
        let mut d_best_other = f32::MAX;

        for (i, &tl) in target_labs.iter().enumerate() {
            let d = lab.delta_e94(tl) - weight_of(i);
            if d < winner_dist {
                winner = i;
                winner_dist = d;
            }
            if i == params.target_index {
                d_self = d;
            } else if d < d_best_other {
                d_best_other = d;
            }
        }

        // "Others" are colors we explicitly do not want to claim. If
        // one of them is strictly closer than the winning target, no
        // plate gets this pixel.
        let rejected = other_labs
            .iter()
            .any(|&ol| lab.delta_e94(ol) < winner_dist);

        let density = if rejected {
            255u8
        } else {
            // margin > 0 → inside our territory (we beat competitors)
            // margin = 0 → exactly on the Voronoi boundary
            // margin < 0 → outside our territory
            let margin = d_best_other - d_self;
            let raw = 128.0 - margin * scale;
            // ±0.5 bias ensures the Voronoi winner (lowest index at
            // equal distance) always claims the pixel, preventing
            // double coverage on the press.
            if winner == params.target_index {
                (raw - 0.5).clamp(0.0, 255.0) as u8
            } else {
                (raw + 0.5).clamp(0.0, 255.0) as u8
            }
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
                target_weights: None,
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

    /// Per-target reach weights should be able to flip the winner
    /// on a pixel that's equidistant to two targets. Default (no
    /// weights) picks the lower index; a positive weight on the
    /// higher index flips it.
    #[test]
    fn target_weight_biases_voronoi() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([128, 128, 128]));
        let targets = [Rgb(100, 100, 100), Rgb(160, 160, 160)];
        let others: [Rgb; 0] = [];

        // Without weights, the two targets are close enough that
        // the lower-index one (darker gray) wins.
        let baseline = extract(
            &src,
            Params {
                targets: &targets,
                others: &others,
                target_index: 0,
                aa_full: 0.0,
                aa_end: 0.0,
                target_weights: None,
            },
        );
        let baseline_pixel = baseline.get_pixel(0, 0)[0];

        // Bias target 1 way up — it should now win.
        let weights = [0.0f32, 50.0];
        let biased_0 = extract(
            &src,
            Params {
                targets: &targets,
                others: &others,
                target_index: 0,
                aa_full: 0.0,
                aa_end: 0.0,
                target_weights: Some(&weights),
            },
        );
        let biased_1 = extract(
            &src,
            Params {
                targets: &targets,
                others: &others,
                target_index: 1,
                aa_full: 0.0,
                aa_end: 0.0,
                target_weights: Some(&weights),
            },
        );
        // Without weights the two targets are equidistant, so the
        // lower-index winner gets density < 128 (ink after binarize)
        // via the soft-edge gradient. With heavy bias on target 1 the
        // margins are enormous, so the outputs saturate to 0/255.
        assert!(
            baseline_pixel < 128,
            "lower index should win equidistant tie, got {baseline_pixel}"
        );
        assert_eq!(biased_0.get_pixel(0, 0)[0], 255);
        assert_eq!(biased_1.get_pixel(0, 0)[0], 0);
    }

    /// Two palette entries at identical CIE94 distance must never both
    /// claim the same pixel — that would print as double coverage and
    /// cause registration bleed on the press. With soft-edge output the
    /// densities are no longer hard 0/255, but exactly one must be
    /// below the binarize threshold (128 → ink) and the other at or
    /// above it (no ink).
    #[test]
    fn ties_are_broken_by_lower_index() {
        let mut src: RgbImage = ImageBuffer::new(1, 1);
        src.put_pixel(0, 0, image::Rgb([128, 128, 128]));
        // Two identical targets — both are at exactly distance 0 from
        // the pixel, so the tie-break rule must pick exactly one.
        let targets = [Rgb(128, 128, 128), Rgb(128, 128, 128)];

        let p0 = run(&src, &targets, 0).get_pixel(0, 0)[0];
        let p1 = run(&src, &targets, 1).get_pixel(0, 0)[0];
        // Exactly one plate claims it (< 128 = ink after binarize).
        assert!(
            (p0 < 128 && p1 >= 128) || (p0 >= 128 && p1 < 128),
            "exactly one plate must claim the pixel (< 128 = ink), got p0={p0} p1={p1}"
        );
    }
}
