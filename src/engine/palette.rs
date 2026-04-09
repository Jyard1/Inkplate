//! Automatic palette extraction and hue-family consolidation.
//!
//! Two related jobs live here:
//!
//! 1. **`auto_palette`** — pick a small set of representative colors from
//!    an input image. Pipeline: LAB-space median-cut seeding → k-means
//!    refinement with CIE94 ΔE → coverage prune. Working in LAB (not RGB)
//!    and using CIE94 for nearest-neighbour decisions is what lets this
//!    preserve small saturated accents (the yellow torch flame, a single
//!    red button) that naive RGB median-cut will merge into a bigger
//!    cluster and lose.
//!
//! 2. **`consolidate_by_hue`** — given an existing palette, collapse colors
//!    that belong to the same hue family (dark red + bright red) into one
//!    screen while keeping near-grays and distinct hues separate. The
//!    spot workflow calls this with `consolidate_hues: true` to merge
//!    shades; the cel-shaded workflow skips it so shadow/highlight
//!    variants keep their own screens.

use crate::engine::color::{lab_to_rgb, rgb_to_lab, Lab, Rgb};

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
            max_colors: 16,
            // merge_delta_e is legacy — k-means + LAB median cut
            // produces well-separated clusters without needing a
            // post-merge pass. Kept in the struct so the GUI doesn't
            // need to change.
            merge_delta_e: 8.0,
            // Small accent colors (< 0.5% of the image) matter a lot
            // in screen art — a yellow torch flame or a single red
            // button is visually important. 0.002 (0.2%) still kills
            // stray noise pixels but preserves genuine accents.
            min_coverage: 0.002,
        }
    }
}

/// Snap near-black palette entries (L* < 15, chroma < 10) to pure
/// `#000` and near-white entries (L* > 92, chroma < 10) to pure
/// `#FFF`. Screen presses ink with pure black/white, not off-white or
/// charcoal, so the plate swatches and the ink that actually goes on
/// the shirt should match. Call this right after [`auto_palette`] in
/// any workflow that emits printable plates.
pub fn snap_extremes(palette: &mut [PaletteEntry]) {
    for e in palette.iter_mut() {
        let lab = rgb_to_lab(e.rgb);
        let near_black = lab.l < 15.0 && lab.chroma() < 10.0;
        let near_white = lab.l > 92.0 && lab.chroma() < 10.0;
        if near_black {
            e.rgb = Rgb::BLACK;
        } else if near_white {
            e.rgb = Rgb::WHITE;
        }
    }
}

/// Number of k-means refinement iterations after median-cut seeding.
/// Convergence is usually hit within 8–10 iterations; 15 leaves headroom.
const KMEANS_ITERS: usize = 15;

/// Subsample cap for clustering. Large enough for stable cluster
/// centroids, small enough to keep median-cut and k-means fast on
/// multi-megapixel input.
const SAMPLE_CAP: usize = 80_000;

/// Palette extraction in LAB space with CIE94 distance.
///
/// Pipeline:
///
/// 1. Subsample the image down to [`SAMPLE_CAP`] pixels and convert
///    to LAB. Everything after this works in LAB.
/// 2. Run median-cut on the LAB axes to produce `max_colors * 2`
///    initial cluster seeds (2× oversampling so minor hues survive
///    the refinement + prune passes).
/// 3. Run [`KMEANS_ITERS`] rounds of k-means using CIE94 ΔE. Each
///    iteration reassigns sample pixels to their nearest center and
///    moves the center to the LAB mean of its assigned pixels.
/// 4. Quantize the full image to the refined centers, counting real
///    coverage per cluster.
/// 5. Drop centers below `min_coverage`, sort by descending coverage,
///    truncate to `max_colors`.
///
/// Returns the palette plus a re-quantized RGB buffer (each source
/// pixel replaced with its cluster's LAB-space representative).
pub fn auto_palette(pixels: &[u8], opts: PaletteOpts) -> (Vec<PaletteEntry>, Vec<u8>) {
    assert_eq!(pixels.len() % 3, 0);
    let total = pixels.len() / 3;
    if total == 0 || opts.max_colors == 0 {
        return (vec![], pixels.to_vec());
    }

    // --- Stage 1: subsample + LAB conversion.
    let stride = total.div_ceil(SAMPLE_CAP).max(1);
    let sample: Vec<Lab> = (0..total)
        .step_by(stride)
        .map(|i| rgb_to_lab(Rgb(pixels[i * 3], pixels[i * 3 + 1], pixels[i * 3 + 2])))
        .collect();

    if sample.is_empty() {
        return (vec![], pixels.to_vec());
    }

    // --- Stage 2: LAB-space median cut to seed k-means.
    let target_seeds = (opts.max_colors * 2).max(4);
    let mut boxes: Vec<Vec<usize>> = vec![(0..sample.len()).collect()];
    while boxes.len() < target_seeds {
        let Some((idx, max_range)) = boxes
            .iter()
            .enumerate()
            .map(|(i, b)| (i, box_axis_range_lab(&sample, b)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        else {
            break;
        };
        if max_range < 1.0 {
            // Every remaining box is essentially a single LAB point.
            break;
        }
        let b = boxes.remove(idx);
        let (lo, hi) = split_box_lab(&sample, &b);
        if lo.is_empty() || hi.is_empty() {
            boxes.push(b);
            break;
        }
        boxes.push(lo);
        boxes.push(hi);
    }

    let mut centers: Vec<Lab> = boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| box_mean_lab(&sample, b))
        .collect();

    if centers.is_empty() {
        return (vec![], pixels.to_vec());
    }

    // --- Stage 3: k-means refinement with CIE94.
    for _ in 0..KMEANS_ITERS {
        let mut sums = vec![(0.0f64, 0.0f64, 0.0f64); centers.len()];
        let mut counts = vec![0usize; centers.len()];
        for &sl in &sample {
            let best = nearest_center_idx(&centers, sl);
            sums[best].0 += sl.l as f64;
            sums[best].1 += sl.a as f64;
            sums[best].2 += sl.b as f64;
            counts[best] += 1;
        }
        let mut moved = 0.0f32;
        for (k, c) in centers.iter_mut().enumerate() {
            if counts[k] > 0 {
                let n = counts[k] as f64;
                let nc = Lab {
                    l: (sums[k].0 / n) as f32,
                    a: (sums[k].1 / n) as f32,
                    b: (sums[k].2 / n) as f32,
                };
                moved += c.delta_e94(nc);
                *c = nc;
            }
        }
        if moved < 0.5 {
            break;
        }
    }

    // Drop any cluster that ended up with zero assignments.
    let sample_assignment: Vec<usize> = sample
        .iter()
        .map(|&l| nearest_center_idx(&centers, l))
        .collect();
    let mut sample_counts = vec![0usize; centers.len()];
    for &a in &sample_assignment {
        sample_counts[a] += 1;
    }
    let retained: Vec<Lab> = centers
        .iter()
        .zip(sample_counts.iter())
        .filter_map(|(&c, &n)| if n > 0 { Some(c) } else { None })
        .collect();
    if retained.is_empty() {
        return (vec![], pixels.to_vec());
    }
    let centers = retained;

    // --- Stage 4: quantize the full image and count real coverage.
    let center_rgbs: Vec<Rgb> = centers.iter().map(|&l| lab_to_rgb(l)).collect();
    let mut coverage = vec![0usize; centers.len()];
    let mut quantized = vec![0u8; pixels.len()];
    for i in 0..total {
        let px = Rgb(pixels[i * 3], pixels[i * 3 + 1], pixels[i * 3 + 2]);
        let lab = rgb_to_lab(px);
        let best = nearest_center_idx(&centers, lab);
        coverage[best] += 1;
        let c = center_rgbs[best];
        quantized[i * 3] = c.0;
        quantized[i * 3 + 1] = c.1;
        quantized[i * 3 + 2] = c.2;
    }

    // --- Stage 5: prune, sort, truncate.
    let mut entries: Vec<PaletteEntry> = center_rgbs
        .into_iter()
        .zip(coverage.iter())
        .map(|(rgb, &n)| PaletteEntry {
            rgb,
            coverage: n as f32 / total as f32,
        })
        .filter(|e| e.coverage >= opts.min_coverage)
        .collect();
    entries.sort_by(|a, b| b.coverage.partial_cmp(&a.coverage).unwrap());
    entries.truncate(opts.max_colors);

    (entries, quantized)
}

// --- LAB-space cluster helpers

fn nearest_center_idx(centers: &[Lab], lab: Lab) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (k, &c) in centers.iter().enumerate() {
        let d = lab.delta_e94(c);
        if d < best_d {
            best_d = d;
            best = k;
        }
    }
    best
}

fn box_axis_range_lab(labs: &[Lab], indices: &[usize]) -> f32 {
    if indices.is_empty() {
        return 0.0;
    }
    let (mut lmn, mut lmx) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut amn, mut amx) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut bmn, mut bmx) = (f32::INFINITY, f32::NEG_INFINITY);
    for &i in indices {
        let p = labs[i];
        if p.l < lmn {
            lmn = p.l;
        }
        if p.l > lmx {
            lmx = p.l;
        }
        if p.a < amn {
            amn = p.a;
        }
        if p.a > amx {
            amx = p.a;
        }
        if p.b < bmn {
            bmn = p.b;
        }
        if p.b > bmx {
            bmx = p.b;
        }
    }
    // Scale L by 0.5 so splits prefer chroma/hue axes when they
    // have comparable range — this keeps the palette hue-diverse
    // instead of burning half the seeds on lightness shades.
    let rl = (lmx - lmn) * 0.5;
    let ra = amx - amn;
    let rb = bmx - bmn;
    rl.max(ra).max(rb)
}

fn split_box_lab(labs: &[Lab], indices: &[usize]) -> (Vec<usize>, Vec<usize>) {
    if indices.len() < 2 {
        return (indices.to_vec(), vec![]);
    }
    let (mut lmn, mut lmx) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut amn, mut amx) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut bmn, mut bmx) = (f32::INFINITY, f32::NEG_INFINITY);
    for &i in indices {
        let p = labs[i];
        lmn = lmn.min(p.l);
        lmx = lmx.max(p.l);
        amn = amn.min(p.a);
        amx = amx.max(p.a);
        bmn = bmn.min(p.b);
        bmx = bmx.max(p.b);
    }
    let rl = (lmx - lmn) * 0.5;
    let ra = amx - amn;
    let rb = bmx - bmn;
    let axis = if ra >= rl && ra >= rb {
        1 // a*
    } else if rb >= rl {
        2 // b*
    } else {
        0 // L*
    };
    let mut sorted = indices.to_vec();
    sorted.sort_by(|&i, &j| {
        let vi = match axis {
            0 => labs[i].l,
            1 => labs[i].a,
            _ => labs[i].b,
        };
        let vj = match axis {
            0 => labs[j].l,
            1 => labs[j].a,
            _ => labs[j].b,
        };
        vi.partial_cmp(&vj).unwrap()
    });
    let mid = sorted.len() / 2;
    let (lo, hi) = sorted.split_at(mid);
    (lo.to_vec(), hi.to_vec())
}

fn box_mean_lab(labs: &[Lab], indices: &[usize]) -> Lab {
    let mut sl = 0.0f64;
    let mut sa = 0.0f64;
    let mut sb = 0.0f64;
    for &i in indices {
        sl += labs[i].l as f64;
        sa += labs[i].a as f64;
        sb += labs[i].b as f64;
    }
    let n = indices.len().max(1) as f64;
    Lab {
        l: (sl / n) as f32,
        a: (sa / n) as f32,
        b: (sb / n) as f32,
    }
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
}

impl Default for HueOpts {
    fn default() -> Self {
        Self {
            // 10° is tight enough that "red" and "orange-red" stay
            // on separate plates, but loose enough that a bright red
            // and its shadow (both around hue 25-30°) merge into one
            // ink. An earlier version used 25° and was collapsing
            // genuinely distinct hues into the same family.
            hue_tolerance_deg: 10.0,
            gray_chroma: 6.0,
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
                // Pure hue-angle grouping: two chromatic entries are
                // the same family if their hue angles agree, period.
                // The old code also required ab-plane proximity, which
                // rejected legitimate merges between bright and dark
                // shades of the same hue (they're ~40+ units apart in
                // ab because saturation differs, even though the hue
                // direction is identical).
                hue_distance(li.hue_deg(), lj.hue_deg()) < opts.hue_tolerance_deg
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

    /// Small saturated accent colors must survive palette extraction.
    /// This is the demon-image regression: a 2% yellow highlight on
    /// an otherwise red/cyan/black image was being swallowed by the
    /// closest neighbour because median-cut in RGB space never split
    /// a box just for it.
    #[test]
    fn small_saturated_accent_survives() {
        // Synthesize a 100x100 image with:
        //   - 70% near-black background
        //   - 25% bright red
        //   - 3% bright yellow (the "torch flame")
        //   - 2% pure white highlights
        let mut px = vec![];
        for _ in 0..7000 {
            px.extend_from_slice(&[10, 10, 10]);
        }
        for _ in 0..2500 {
            px.extend_from_slice(&[220, 30, 30]);
        }
        for _ in 0..300 {
            px.extend_from_slice(&[240, 210, 40]);
        }
        for _ in 0..200 {
            px.extend_from_slice(&[255, 255, 255]);
        }

        let (pal, _) = auto_palette(
            &px,
            PaletteOpts {
                max_colors: 8,
                merge_delta_e: 8.0,
                min_coverage: 0.005,
            },
        );

        // All four colors must appear — nobody gets dropped.
        let has_near = |r: u8, g: u8, b: u8| -> bool {
            pal.iter().any(|e| {
                let dr = (e.rgb.0 as i32 - r as i32).abs();
                let dg = (e.rgb.1 as i32 - g as i32).abs();
                let db = (e.rgb.2 as i32 - b as i32).abs();
                dr < 40 && dg < 40 && db < 40
            })
        };
        assert!(has_near(10, 10, 10), "near-black missing: {pal:?}");
        assert!(has_near(220, 30, 30), "red missing: {pal:?}");
        assert!(has_near(240, 210, 40), "yellow accent missing: {pal:?}");
        assert!(has_near(255, 255, 255), "white missing: {pal:?}");
    }

    #[test]
    fn snap_extremes_pure_inks() {
        let mut pal = vec![
            PaletteEntry {
                rgb: Rgb(8, 7, 10),
                coverage: 0.3,
            },
            PaletteEntry {
                rgb: Rgb(250, 248, 252),
                coverage: 0.4,
            },
            PaletteEntry {
                rgb: Rgb(220, 30, 30),
                coverage: 0.3,
            },
        ];
        snap_extremes(&mut pal);
        assert_eq!(pal[0].rgb, Rgb::BLACK);
        assert_eq!(pal[1].rgb, Rgb::WHITE);
        assert_eq!(pal[2].rgb, Rgb(220, 30, 30));
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
