//! Tone curves, levels, and density scaling.
//!
//! Tone shaping runs after the extractor and before mask shaping — it's
//! how we boost shadows, crush highlights, adjust ink density, and apply
//! per-layer curves. Everything is a precomputed 256-entry LUT so the
//! pixel loop is a single table lookup.

use image::GrayImage;

/// A single point on a piecewise-linear curve. `x` and `y` are both
/// density-space values in `[0, 255]`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CurvePoint {
    pub x: u8,
    pub y: u8,
}

impl CurvePoint {
    pub const fn new(x: u8, y: u8) -> Self {
        Self { x, y }
    }
}

/// Identity curve: `y = x`.
pub const IDENTITY_CURVE: &[CurvePoint] = &[CurvePoint::new(0, 0), CurvePoint::new(255, 255)];

/// Build a 256-entry lookup table from a piecewise-linear curve.
///
/// Points don't need to be sorted or unique — duplicates resolve in later-
/// wins order, matching the reference Python implementation where dragging
/// two curve handles to the same x lets the newer one override.
///
/// An identity curve short-circuits to `None` so callers can skip the LUT
/// pass entirely. That's not just an optimization: it keeps exported films
/// bit-exact when the user hasn't touched the tone controls.
pub fn build_lut(points: &[CurvePoint]) -> Option<[u8; 256]> {
    if points.is_empty() || is_identity(points) {
        return None;
    }

    let mut sorted: Vec<CurvePoint> = points.to_vec();
    sorted.sort_by_key(|p| p.x);
    // Dedup by x, later-wins.
    sorted.dedup_by(|a, b| {
        if a.x == b.x {
            *b = *a;
            true
        } else {
            false
        }
    });

    // Anchor endpoints so we never interpolate off the edge.
    if sorted.first().map(|p| p.x).unwrap_or(0) > 0 {
        sorted.insert(0, CurvePoint::new(0, sorted[0].y));
    }
    if sorted.last().map(|p| p.x).unwrap_or(255) < 255 {
        let last_y = sorted.last().unwrap().y;
        sorted.push(CurvePoint::new(255, last_y));
    }

    let mut lut = [0u8; 256];
    let mut seg = 0;
    for i in 0..=255u16 {
        while seg + 1 < sorted.len() && (sorted[seg + 1].x as u16) < i {
            seg += 1;
        }
        let a = sorted[seg];
        let b = sorted[(seg + 1).min(sorted.len() - 1)];
        let t = if a.x == b.x {
            0.0
        } else {
            (i as f32 - a.x as f32) / (b.x as f32 - a.x as f32)
        };
        let y = a.y as f32 + t * (b.y as f32 - a.y as f32);
        lut[i as usize] = y.round().clamp(0.0, 255.0) as u8;
    }
    Some(lut)
}

fn is_identity(points: &[CurvePoint]) -> bool {
    if points.len() != 2 {
        return false;
    }
    points[0] == CurvePoint::new(0, 0) && points[1] == CurvePoint::new(255, 255)
}

/// Apply a pre-built LUT to a density map in place.
pub fn apply_lut_in_place(img: &mut GrayImage, lut: &[u8; 256]) {
    for p in img.iter_mut() {
        *p = lut[*p as usize];
    }
}

/// Apply a curve directly (builds and applies LUT). Returns the input
/// unchanged if the curve is the identity.
pub fn apply_curve(img: &GrayImage, points: &[CurvePoint]) -> GrayImage {
    match build_lut(points) {
        None => img.clone(),
        Some(lut) => {
            let mut out = img.clone();
            apply_lut_in_place(&mut out, &lut);
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Levels + density
// ---------------------------------------------------------------------------

/// Classic Photoshop-style levels: input black / white / gamma.
///
/// `gamma = 1.0` is neutral. Values above 1 push midtones brighter (less
/// ink); below 1 push them darker (more ink).
pub fn apply_levels(img: &GrayImage, in_black: u8, in_white: u8, gamma: f32) -> GrayImage {
    let mut out = img.clone();
    let lo = in_black as f32;
    let hi = (in_white as f32).max(lo + 1.0);
    let inv_gamma = 1.0 / gamma.max(0.01);
    for p in out.iter_mut() {
        let v = ((*p as f32 - lo) / (hi - lo)).clamp(0.0, 1.0);
        let v = v.powf(inv_gamma);
        *p = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Density multiplier — ink coverage scaling. `density = 1.0` is neutral.
/// `density < 1` lightens (less ink); `density > 1` darkens (more ink).
///
/// Implemented against the density convention (0 = ink, 255 = no ink) so
/// scaling 0.5 pushes pixels halfway toward white, not halfway toward zero.
pub fn apply_density(img: &GrayImage, density: f32) -> GrayImage {
    if (density - 1.0).abs() < 1e-4 {
        return img.clone();
    }
    let mut out = img.clone();
    let d = density.max(0.0);
    for p in out.iter_mut() {
        // Remap: ink amount = (255 - p) * d, then flip back.
        let ink = (255.0 - *p as f32) * d;
        let v = 255.0 - ink;
        *p = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma};

    #[test]
    fn identity_curve_short_circuits() {
        assert!(build_lut(IDENTITY_CURVE).is_none());
        assert!(build_lut(&[]).is_none());
    }

    #[test]
    fn linear_lut_is_monotonic() {
        let lut = build_lut(&[CurvePoint::new(0, 0), CurvePoint::new(255, 255)]);
        assert!(lut.is_none()); // identity caught
        let lut = build_lut(&[
            CurvePoint::new(0, 0),
            CurvePoint::new(128, 200),
            CurvePoint::new(255, 255),
        ])
        .unwrap();
        for i in 1..256 {
            assert!(lut[i] >= lut[i - 1]);
        }
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        assert!(lut[128] >= 195 && lut[128] <= 205);
    }

    #[test]
    fn density_neutral_is_no_op() {
        let img = ImageBuffer::from_fn(4, 1, |x, _| Luma([(x * 50) as u8]));
        let out = apply_density(&img, 1.0);
        assert_eq!(img.into_raw(), out.into_raw());
    }

    #[test]
    fn density_double_doubles_ink() {
        let img: GrayImage = ImageBuffer::from_pixel(1, 1, Luma([200])); // a bit of ink
        let out = apply_density(&img, 2.0);
        // ink = (255-200)*2 = 110, new value = 255-110 = 145
        assert_eq!(out.get_pixel(0, 0)[0], 145);
    }
}
