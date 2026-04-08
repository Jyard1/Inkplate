//! Auto-detect heuristic — sniff the source image and pick a workflow.
//!
//! Not magic, just a decision tree against four cheap features:
//!
//! | feature         | how it's measured                                   |
//! |-----------------|------------------------------------------------------|
//! | is_grayscale    | 95th-percentile LAB chroma < 8                       |
//! | unique_colors   | HashSet of quantized RGB triples                     |
//! | peak_coverage   | largest bucket in a 16-bin RGB histogram / total     |
//! | area            | width × height                                       |
//!
//! From those four, the decision cascade is:
//!
//! 1. Grayscale → `single_halftone`
//! 2. Very few colors (<12) with high coverage per color → `spot`
//! 3. Few colors (<50) → `cel_shaded`
//! 4. Small image with limited palette → `index_bayer` (pixel art)
//! 5. Otherwise → `simprocess_dark` (safe default)
//!
//! Edit the thresholds if this picks wrong for your art.

use image::RgbImage;

use crate::engine::color::{rgb_to_lab, Rgb};
use crate::engine::workflows::Workflow;

pub fn detect(source: &RgbImage) -> Workflow {
    let (w, h) = source.dimensions();
    let total = (w * h) as usize;
    if total == 0 {
        return Workflow::Spot;
    }

    let (unique, peak_coverage) = histogram_stats(source);
    let grayscale = is_near_grayscale(source);

    if grayscale {
        return Workflow::SingleHalftone;
    }
    if unique < 12 && peak_coverage > 0.08 {
        return Workflow::Spot;
    }
    if unique < 50 {
        return Workflow::CelShaded;
    }
    if total < 200_000 && unique < 64 {
        return Workflow::IndexBayer;
    }
    Workflow::SimprocessDark
}

/// Quantize to 16 levels per channel and return `(unique_colors,
/// peak_bucket_coverage)`. The 16-level quantization is generous
/// enough that noisy source art still collapses into a small number of
/// buckets.
fn histogram_stats(source: &RgbImage) -> (usize, f32) {
    use std::collections::HashMap;
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for p in source.pixels() {
        let key = [p[0] & 0xF0, p[1] & 0xF0, p[2] & 0xF0];
        *counts.entry(key).or_insert(0) += 1;
    }
    let total = source.pixels().count() as f32;
    let peak = counts.values().copied().max().unwrap_or(0) as f32 / total.max(1.0);
    (counts.len(), peak)
}

/// True if the 95th percentile of LAB chroma across the image is below
/// `8.0` — that threshold is loose enough to capture sepia and faintly
/// tinted grayscale, tight enough to reject anything with real color.
fn is_near_grayscale(source: &RgbImage) -> bool {
    // Subsample on a ~100-pixel grid for speed; the 95th percentile is
    // stable at this sample size for any reasonable input.
    let (w, h) = source.dimensions();
    let step = (w.min(h) / 100).max(1);
    let mut chromas: Vec<f32> = Vec::new();
    let mut y = 0u32;
    while y < h {
        let mut x = 0u32;
        while x < w {
            let p = source.get_pixel(x, y);
            let lab = rgb_to_lab(Rgb(p[0], p[1], p[2]));
            chromas.push(lab.chroma());
            x += step;
        }
        y += step;
    }
    if chromas.is_empty() {
        return false;
    }
    chromas.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = chromas[(chromas.len() as f32 * 0.95) as usize];
    p95 < 8.0
}
