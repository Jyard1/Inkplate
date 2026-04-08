//! Automatic palette extraction and hue-family consolidation.
//!
//! Two related jobs live here:
//!
//! 1. **`auto_palette`** — pick a small set of representative colors from
//!    an input image. Seeded by median-cut, merged by LAB ΔE, pruned by
//!    coverage, then the image is re-quantized to the exact nearest
//!    remaining color. This is what spot and index workflows use to build
//!    their starting layer list.
//!
//! 2. **`consolidate_by_hue`** — given an existing palette, collapse colors
//!    that belong to the same hue family (dark red + bright red) into one
//!    screen while keeping near-grays and distinct hues separate. This is
//!    the "same-hue family" grouping from the rebuild plan, and it's why
//!    spot workflows don't produce a screen per shade of the same ink.

use crate::engine::color::{rgb_to_lab, Lab, Rgb};

/// A palette entry with its coverage fraction (0–1).
#[derive(Debug, Clone)]
pub struct PaletteEntry {
    pub rgb: Rgb,
    pub coverage: f32,
}

/// Options for [`auto_palette`]. Defaults match the pro spot-color workflow.
#[derive(Debug, Clone, Copy)]
pub struct PaletteOpts {
    pub max_colors: usize,
    /// LAB ΔE threshold below which two palette entries get merged.
    pub merge_delta_e: f32,
    /// Drop any entry whose coverage is below this fraction.
    pub min_coverage: f32,
}

impl Default for PaletteOpts {
    fn default() -> Self {
        Self {
            max_colors: 12,
            merge_delta_e: 8.0,
            min_coverage: 0.005,
        }
    }
}

/// Median-cut → LAB-merge → coverage-prune palette extraction.
///
/// Returns a palette sorted by descending coverage, plus a re-quantized
/// RGB buffer where every pixel has been snapped to its nearest remaining
/// palette color. That re-quantized buffer is what extractor `spot_solid`
/// should work against — it means the tolerance slider becomes meaningful
/// even for anti-aliased edges.
// The pair-merge inner loop uses both indices to splice from `seeds`,
// so the iter().enumerate().skip() rewrite would just hide the math.
#[allow(clippy::needless_range_loop)]
pub fn auto_palette(pixels: &[u8], opts: PaletteOpts) -> (Vec<PaletteEntry>, Vec<u8>) {
    assert_eq!(pixels.len() % 3, 0);
    let total = pixels.len() / 3;
    if total == 0 {
        return (vec![], vec![]);
    }

    // --- Stage 1: median cut to seed the palette.
    let indices: Vec<usize> = (0..total).collect();
    let mut boxes: Vec<Vec<usize>> = vec![indices];
    while boxes.len() < opts.max_colors * 2 {
        let (idx, _) = match boxes
            .iter()
            .enumerate()
            .max_by_key(|(_, b)| box_axis_range(pixels, b))
        {
            Some(v) => v,
            None => break,
        };
        if box_axis_range(pixels, &boxes[idx]) == 0 {
            break;
        }
        let b = boxes.remove(idx);
        let (a, c) = split_box(pixels, &b);
        if a.is_empty() || c.is_empty() {
            boxes.push(b);
            break;
        }
        boxes.push(a);
        boxes.push(c);
    }

    let mut seeds: Vec<(Rgb, usize)> = boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| (box_mean(pixels, b), b.len()))
        .collect();

    // --- Stage 2: merge any two seeds whose LAB ΔE is below threshold.
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for i in 0..seeds.len() {
            let li = rgb_to_lab(seeds[i].0);
            for j in (i + 1)..seeds.len() {
                let lj = rgb_to_lab(seeds[j].0);
                let d = li.delta_e(lj);
                if d < opts.merge_delta_e && best.map_or(true, |(_, _, bd)| d < bd) {
                    best = Some((i, j, d));
                }
            }
        }
        match best {
            None => break,
            Some((i, j, _)) => {
                let (ci, ni) = seeds[i];
                let (cj, nj) = seeds[j];
                let total = (ni + nj) as f32;
                let wi = ni as f32 / total;
                let wj = nj as f32 / total;
                let merged = Rgb(
                    (ci.0 as f32 * wi + cj.0 as f32 * wj).round() as u8,
                    (ci.1 as f32 * wi + cj.1 as f32 * wj).round() as u8,
                    (ci.2 as f32 * wi + cj.2 as f32 * wj).round() as u8,
                );
                seeds[i] = (merged, ni + nj);
                seeds.remove(j);
            }
        }
    }

    // --- Stage 3: re-quantize entire image to the nearest seed.
    let seed_labs: Vec<Lab> = seeds.iter().map(|(c, _)| rgb_to_lab(*c)).collect();
    let mut counts = vec![0usize; seeds.len()];
    let mut quantized = vec![0u8; pixels.len()];
    let mut acc = vec![(0.0f64, 0.0f64, 0.0f64); seeds.len()];

    for i in 0..total {
        let px = Rgb(pixels[i * 3], pixels[i * 3 + 1], pixels[i * 3 + 2]);
        let lab = rgb_to_lab(px);
        let mut best = 0usize;
        let mut best_d = f32::INFINITY;
        for (k, &sl) in seed_labs.iter().enumerate() {
            let d = lab.delta_e(sl);
            if d < best_d {
                best_d = d;
                best = k;
            }
        }
        counts[best] += 1;
        acc[best].0 += px.0 as f64;
        acc[best].1 += px.1 as f64;
        acc[best].2 += px.2 as f64;
    }

    // Refine centroid to the actual mean of assigned pixels.
    for k in 0..seeds.len() {
        if counts[k] > 0 {
            let n = counts[k] as f64;
            seeds[k].0 = Rgb(
                (acc[k].0 / n).round() as u8,
                (acc[k].1 / n).round() as u8,
                (acc[k].2 / n).round() as u8,
            );
            seeds[k].1 = counts[k];
        }
    }

    // Second pass: write quantized output using refined palette.
    let seed_labs: Vec<Lab> = seeds.iter().map(|(c, _)| rgb_to_lab(*c)).collect();
    for i in 0..total {
        let px = Rgb(pixels[i * 3], pixels[i * 3 + 1], pixels[i * 3 + 2]);
        let lab = rgb_to_lab(px);
        let mut best = 0usize;
        let mut best_d = f32::INFINITY;
        for (k, &sl) in seed_labs.iter().enumerate() {
            let d = lab.delta_e(sl);
            if d < best_d {
                best_d = d;
                best = k;
            }
        }
        let c = seeds[best].0;
        quantized[i * 3] = c.0;
        quantized[i * 3 + 1] = c.1;
        quantized[i * 3 + 2] = c.2;
    }

    // --- Stage 4: prune low-coverage entries and clamp to max_colors.
    let mut entries: Vec<PaletteEntry> = seeds
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(c, n)| PaletteEntry {
            rgb: c,
            coverage: n as f32 / total as f32,
        })
        .filter(|e| e.coverage >= opts.min_coverage)
        .collect();
    entries.sort_by(|a, b| b.coverage.partial_cmp(&a.coverage).unwrap());
    entries.truncate(opts.max_colors);

    (entries, quantized)
}

// --- box helpers for median cut

fn box_mean(pixels: &[u8], indices: &[usize]) -> Rgb {
    let mut r = 0u64;
    let mut g = 0u64;
    let mut b = 0u64;
    for &i in indices {
        r += pixels[i * 3] as u64;
        g += pixels[i * 3 + 1] as u64;
        b += pixels[i * 3 + 2] as u64;
    }
    let n = indices.len().max(1) as u64;
    Rgb((r / n) as u8, (g / n) as u8, (b / n) as u8)
}

/// Return the range (`max - min`) of the longest RGB axis for sorting.
fn box_axis_range(pixels: &[u8], indices: &[usize]) -> u32 {
    if indices.is_empty() {
        return 0;
    }
    let mut mn = [255u8, 255, 255];
    let mut mx = [0u8, 0, 0];
    for &i in indices {
        for c in 0..3 {
            let v = pixels[i * 3 + c];
            if v < mn[c] {
                mn[c] = v;
            }
            if v > mx[c] {
                mx[c] = v;
            }
        }
    }
    let ranges = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
    *ranges.iter().max().unwrap() as u32
}

fn split_box(pixels: &[u8], indices: &[usize]) -> (Vec<usize>, Vec<usize>) {
    if indices.len() < 2 {
        return (indices.to_vec(), vec![]);
    }
    let mut mn = [255u8, 255, 255];
    let mut mx = [0u8, 0, 0];
    for &i in indices {
        for c in 0..3 {
            let v = pixels[i * 3 + c];
            if v < mn[c] {
                mn[c] = v;
            }
            if v > mx[c] {
                mx[c] = v;
            }
        }
    }
    // Longest axis.
    let ranges = [mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]];
    let axis = ranges
        .iter()
        .enumerate()
        .max_by_key(|(_, v)| **v)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let mut sorted = indices.to_vec();
    sorted.sort_by_key(|&i| pixels[i * 3 + axis]);
    let mid = sorted.len() / 2;
    let (a, b) = sorted.split_at(mid);
    (a.to_vec(), b.to_vec())
}

// ---------------------------------------------------------------------------
// Hue-family consolidation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct HueOpts {
    /// Hue angle tolerance in degrees for merging same-hue colors.
    pub hue_tolerance_deg: f32,
    /// Any color with chroma below this is treated as near-grayscale and
    /// merged only with other near-grays of similar lightness.
    pub gray_chroma: f32,
    /// ab-plane distance threshold for tight same-hue merges.
    pub ab_threshold: f32,
}

impl Default for HueOpts {
    fn default() -> Self {
        Self {
            hue_tolerance_deg: 15.0,
            gray_chroma: 6.0,
            // Two shades of the same hue sit ~20-35 LAB units apart in
            // (a, b), so 6.0 was absurdly tight and would never merge
            // anything real. 50 is a loose backup to hue angle; the
            // primary grouping signal is the hue direction, not distance.
            ab_threshold: 50.0,
        }
    }
}

/// Collapse palette entries that belong to the same hue family into a
/// single representative color (coverage-weighted mean). Returns the new
/// palette, sorted by descending coverage.
pub fn consolidate_by_hue(palette: &[PaletteEntry], opts: HueOpts) -> Vec<PaletteEntry> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let labs: Vec<Lab> = palette.iter().map(|e| rgb_to_lab(e.rgb)).collect();

    for i in 0..palette.len() {
        let li = labs[i];
        let placed = groups.iter_mut().find(|g| {
            g.iter().any(|&j| {
                let lj = labs[j];
                let is_gray_i = li.chroma() < opts.gray_chroma;
                let is_gray_j = lj.chroma() < opts.gray_chroma;
                if is_gray_i != is_gray_j {
                    return false;
                }
                if is_gray_i && is_gray_j {
                    return (li.l - lj.l).abs() < 10.0;
                }
                let dh = hue_distance(li.hue_deg(), lj.hue_deg());
                let dab = ((li.a - lj.a).powi(2) + (li.b - lj.b).powi(2)).sqrt();
                dh < opts.hue_tolerance_deg && dab < opts.ab_threshold
            })
        });
        match placed {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }

    let mut out: Vec<PaletteEntry> = groups
        .into_iter()
        .map(|g| {
            let total_cov: f32 = g.iter().map(|&i| palette[i].coverage).sum();
            let mut r = 0.0f32;
            let mut gr = 0.0f32;
            let mut b = 0.0f32;
            for &i in &g {
                let e = &palette[i];
                r += e.rgb.0 as f32 * e.coverage;
                gr += e.rgb.1 as f32 * e.coverage;
                b += e.rgb.2 as f32 * e.coverage;
            }
            let w = total_cov.max(1e-6);
            PaletteEntry {
                rgb: Rgb(
                    (r / w).round() as u8,
                    (gr / w).round() as u8,
                    (b / w).round() as u8,
                ),
                coverage: total_cov,
            }
        })
        .collect();
    out.sort_by(|a, b| b.coverage.partial_cmp(&a.coverage).unwrap());
    out
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_two_color_image_splits() {
        // 50/50 red and blue, two colors expected.
        let mut px = vec![];
        for _ in 0..64 {
            px.extend_from_slice(&[200, 20, 20]);
        }
        for _ in 0..64 {
            px.extend_from_slice(&[20, 20, 200]);
        }
        let (pal, _) = auto_palette(&px, PaletteOpts::default());
        assert_eq!(pal.len(), 2);
        assert!((pal[0].coverage - 0.5).abs() < 0.01);
    }

    #[test]
    fn hue_consolidation_merges_shades() {
        // Dark red + bright red → one hue family; blue stays separate.
        let pal = vec![
            PaletteEntry {
                rgb: Rgb(220, 30, 30),
                coverage: 0.4,
            },
            PaletteEntry {
                rgb: Rgb(120, 20, 20),
                coverage: 0.3,
            },
            PaletteEntry {
                rgb: Rgb(20, 30, 200),
                coverage: 0.3,
            },
        ];
        let out = consolidate_by_hue(&pal, HueOpts::default());
        assert_eq!(out.len(), 2);
    }
}
