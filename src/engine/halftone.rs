//! Amplitude-modulated halftone rasterization.
//!
//! Turns a smooth density map into screen-printable dots at a given LPI
//! and screen angle. Dot shape, screen angle, and tone curve are all
//! per-layer so sim-process jobs can cycle through offset angles to avoid
//! moiré.
//!
//! The real shop defaults (from the research in the rebuild plan):
//!
//! - **LPI**: 55 manual press, 65 automatic press
//! - **Angle**: 22.5° (not 45° — that aligns with mesh threads)
//! - **Dot**: round by default, elliptical for smooth gradients
//! - **Mesh**: 4–5× the LPI

use image::{GrayImage, ImageBuffer, Luma};

/// Screen angle cycle used by sim-process workflows when the user hasn't
/// overridden per-layer angles. Taken directly from the Python reference
/// so output is bit-comparable during the port.
/// Every consecutive pair is ≥30° apart. First 4 entries match classic
/// CMYK separations (K, M, C, complement) to minimize moiré between the
/// layers most likely to overlap.
pub const HALFTONE_ANGLE_CYCLE: &[f32] = &[
    45.0, 75.0, 15.0, 105.0, 0.0, 60.0, 30.0, 90.0, 22.5, 67.5, 112.5, 157.5,
];

/// Return a screen angle for the Nth layer in print order.
pub fn auto_angle_for_index(index: usize) -> f32 {
    HALFTONE_ANGLE_CYCLE[index % HALFTONE_ANGLE_CYCLE.len()]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DotShape {
    #[default]
    Round,
    Square,
    Ellipse,
    Line,
}

/// Tone-curve shape applied to the cell's average density before the dot
/// is drawn. `Linear` is neutral; `SCurve` adds contrast; `Hard` is a
/// sharp threshold useful for high-density ink areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum HalftoneCurve {
    #[default]
    Linear,
    SCurve,
    Hard,
}

/// Per-rasterization halftone settings.
#[derive(Debug, Clone, Copy)]
pub struct HalftoneOpts {
    pub dpi: u32,
    pub lpi: f32,
    pub angle_deg: f32,
    pub dot: DotShape,
    pub curve: HalftoneCurve,
    /// Supersample factor for anti-aliased dot edges. 3 is a good default;
    /// raising to 4 helps at very low LPI, 2 is fine for thumbnails.
    pub supersample: u32,
}

impl Default for HalftoneOpts {
    fn default() -> Self {
        Self {
            dpi: 300,
            lpi: 55.0,
            angle_deg: 22.5,
            dot: DotShape::Round,
            curve: HalftoneCurve::Linear,
            supersample: 3,
        }
    }
}

/// Rasterize a density map into an AM halftone.
///
/// Cell size is `dpi / lpi` pixels per cell. For each cell we average the
/// input density, optionally shape it through a tone curve, and draw a
/// centered dot whose area matches the target coverage. The whole cell
/// grid is rotated by `angle_deg` around the image center.
pub fn make_halftone(src: &GrayImage, opts: HalftoneOpts) -> GrayImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return src.clone();
    }
    // Derive effective DPI from the image dimensions, assuming the
    // longest edge maps to ~13 inches (standard chest print). This
    // makes LPI meaningful relative to the actual image pixels —
    // the user's DPI setting only affects export metadata, not the
    // halftone cell size. Clamped to 72 minimum so tiny images
    // don't degenerate into noise.
    let effective_dpi = (w.max(h) as f32 / 13.0).max(72.0);
    let cell = (effective_dpi / opts.lpi.max(1.0)).max(1.0);
    let ss = opts.supersample.max(1) as f32;
    let ss_w = (w as f32 * ss) as u32;
    let ss_h = (h as f32 * ss) as u32;

    let angle = opts.angle_deg.to_radians();
    let (sin_a, cos_a) = angle.sin_cos();
    let cx = ss_w as f32 * 0.5;
    let cy = ss_h as f32 * 0.5;
    let ss_cell = cell * ss;

    let mut out = ImageBuffer::from_pixel(ss_w, ss_h, Luma([255u8]));

    // Walk cells in the rotated grid. Over-range by a margin so the
    // corners of the output stay covered.
    let margin = 2;
    let max_cells = ((ss_w.max(ss_h) as f32 / ss_cell).ceil() as i32) + margin;

    for cj in -max_cells..=max_cells {
        for ci in -max_cells..=max_cells {
            // Center of this cell in rotated space.
            let lx = (ci as f32 + 0.5) * ss_cell;
            let ly = (cj as f32 + 0.5) * ss_cell;
            // Rotate back into image space for the coverage sample.
            let sx = cx + lx * cos_a - ly * sin_a;
            let sy = cy + lx * sin_a + ly * cos_a;

            let srcx = (sx / ss) as i32;
            let srcy = (sy / ss) as i32;
            if srcx < 0 || srcy < 0 || srcx >= w as i32 || srcy >= h as i32 {
                continue;
            }

            // Average density across all source pixels inside this cell.
            // Single-pixel sampling causes banding in gradients and aliases
            // fine detail into erratic dot sizes. Use floor so the averaging
            // window stays within the cell — ceil at high LPI (small cells)
            // overshoots and blurs detail across neighboring cells.
            let half_src = (cell * 0.5).floor().max(1.0) as i32;
            let ax0 = (srcx - half_src).max(0);
            let ay0 = (srcy - half_src).max(0);
            let ax1 = (srcx + half_src).min(w as i32 - 1);
            let ay1 = (srcy + half_src).min(h as i32 - 1);
            let mut acc: u32 = 0;
            let mut count: u32 = 0;
            for ay in ay0..=ay1 {
                for ax in ax0..=ax1 {
                    acc += src.get_pixel(ax as u32, ay as u32)[0] as u32;
                    count += 1;
                }
            }
            let avg = if count > 0 { acc / count } else { 255 };
            let coverage = shape_curve(1.0 - (avg as f32 / 255.0), opts.curve);
            if coverage <= 0.001 {
                continue;
            }

            // Dot radius (or half-axis) for this coverage level.
            let half = ss_cell * 0.5;
            let draw_center = (sx, sy);
            draw_dot(&mut out, draw_center, half, coverage, angle, opts.dot);
        }
    }

    downsample(&out, w, h, opts.supersample)
}

fn shape_curve(x: f32, curve: HalftoneCurve) -> f32 {
    let x = x.clamp(0.0, 1.0);
    match curve {
        HalftoneCurve::Linear => x,
        HalftoneCurve::SCurve => {
            // Smoothstep variant: gentler shoulders than a hard gamma.
            x * x * (3.0 - 2.0 * x)
        }
        HalftoneCurve::Hard => {
            if x < 0.5 {
                (x * 0.2).clamp(0.0, 1.0)
            } else {
                (0.1 + (x - 0.5) * 1.8).clamp(0.0, 1.0)
            }
        }
    }
}

fn draw_dot(
    out: &mut GrayImage,
    center: (f32, f32),
    half: f32,
    coverage: f32,
    angle: f32,
    shape: DotShape,
) {
    let (cx, cy) = center;
    let c = coverage.clamp(0.0, 1.0);

    // Radius that makes dot area == coverage × cell area.
    // Cell area = (2*half)^2. Round dot area = π r². So r = half·√(4c/π).
    let r = half * (4.0 * c / std::f32::consts::PI).sqrt();

    let x0 = ((cx - half).floor() as i32).max(0);
    let y0 = ((cy - half).floor() as i32).max(0);
    let x1 = ((cx + half).ceil() as i32).min(out.width() as i32 - 1);
    let y1 = ((cy + half).ceil() as i32).min(out.height() as i32 - 1);

    let (sin_a, cos_a) = angle.sin_cos();

    for py in y0..=y1 {
        for px in x0..=x1 {
            let dx = px as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;

            // Rotate into cell-local space so square / line / ellipse
            // orient with the screen.
            let lx = dx * cos_a + dy * sin_a;
            let ly = -dx * sin_a + dy * cos_a;

            let inside = match shape {
                DotShape::Round => (lx * lx + ly * ly) <= r * r,
                DotShape::Square => {
                    let s = half * (c.sqrt());
                    lx.abs() <= s && ly.abs() <= s
                }
                DotShape::Ellipse => {
                    let rx = half * (4.0 * c / std::f32::consts::PI).sqrt();
                    let ry = rx * 0.6;
                    if rx < 1e-4 || ry < 1e-4 {
                        false
                    } else {
                        let nx = lx / rx;
                        let ny = ly / ry;
                        nx * nx + ny * ny <= 1.0
                    }
                }
                DotShape::Line => {
                    let thickness = half * c * 2.0;
                    ly.abs() <= thickness && lx.abs() <= half
                }
            };

            if inside {
                out.put_pixel(px as u32, py as u32, Luma([0]));
            }
        }
    }
}

/// Box-filter downsample from supersampled space to target size. Gives
/// dots a soft anti-aliased edge without the ringing of lanczos.
fn downsample(src: &GrayImage, w: u32, h: u32, factor: u32) -> GrayImage {
    if factor <= 1 {
        return src.clone();
    }
    let mut out = ImageBuffer::new(w, h);
    let f = factor;
    for y in 0..h {
        for x in 0..w {
            let mut acc: u32 = 0;
            for dy in 0..f {
                for dx in 0..f {
                    let sx = x * f + dx;
                    let sy = y * f + dy;
                    acc += src.get_pixel(sx, sy)[0] as u32;
                }
            }
            let v = (acc / (f * f)) as u8;
            out.put_pixel(x, y, Luma([v]));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_cycle_wraps() {
        assert_eq!(auto_angle_for_index(0), 45.0);
        assert_eq!(auto_angle_for_index(HALFTONE_ANGLE_CYCLE.len()), 45.0);
    }

    #[test]
    fn flat_black_stays_black_ish() {
        // Fully inked input → output should be mostly ink (near 0).
        let src: GrayImage = ImageBuffer::from_pixel(60, 60, Luma([0]));
        let ht = make_halftone(&src, HalftoneOpts::default());
        let avg: u32 = ht.iter().map(|p| *p as u32).sum::<u32>() / ht.len() as u32;
        assert!(avg < 60, "expected mostly ink, got avg={avg}");
    }

    #[test]
    fn flat_white_stays_white() {
        let src: GrayImage = ImageBuffer::from_pixel(40, 40, Luma([255]));
        let ht = make_halftone(&src, HalftoneOpts::default());
        for p in ht.iter() {
            assert_eq!(*p, 255);
        }
    }
}
