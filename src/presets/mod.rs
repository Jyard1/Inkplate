//! Built-in libraries: garment colors, ink presets, Pantone approximations,
//! mesh-LPI calculator.
//!
//! These tables are small on purpose — full Pantone is licensed and a
//! screen-printing tool that can't ship a trimmed approximation is less
//! useful than one that ships a reasonable one. Names and sRGB values are
//! best-effort visual matches, not color-managed equivalents.
//!
//! Landing 4 fills the tables in.

use crate::engine::color::Rgb;

/// Recommend a mesh count from a target LPI. Shop rule of thumb:
/// `mesh ≈ 4.5 × LPI`, rounded to the nearest common mesh size.
pub fn mesh_for_lpi(lpi: f32) -> u32 {
    let raw = (lpi * 4.5).round() as u32;
    // Snap to common mesh counts.
    const COMMON: &[u32] = &[110, 156, 200, 230, 280, 305, 355, 380];
    *COMMON
        .iter()
        .min_by_key(|&&m| (m as i32 - raw as i32).abs())
        .unwrap_or(&230)
}

// TODO(L4): GARMENT_PRESETS — 12 common shirt colors (white, ash, navy,
// black, red, gold, heather, etc.) with sRGB values and a display name.
// Exposed as a `&'static [(&'static str, Rgb)]` slice.
//
// TODO(L4): INK_PRESETS — common plastisol colors with opacity hints.
// Opacity matters because it drives the underbase density-boost-under-darks
// decision; an opaque ink doesn't need the boost.
//
// TODO(L4): PANTONE_APPROX — small subset (~200 common Pantone solid
// coated entries) plus a `nearest_pantone(Rgb) -> (name, Rgb)` function
// that does a LAB ΔE lookup. Explicitly not color-managed; document that
// in the GUI tooltip when it's wired up.

#[allow(dead_code)]
const _PRESETS_MARKER: Rgb = Rgb(0, 0, 0); // keeps the `Rgb` import alive
