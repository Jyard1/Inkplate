//! Cel-shaded workflow — spot color with per-shade palette entries.
//!
//! Cel-shaded illustration (cartoons, anime art, comic panels) is flat
//! color regions with deliberate highlight/shadow shades of each ink.
//! The spot workflow already handles these cleanly, so this variant is
//! basically "spot with hue consolidation turned off" — so a highlight
//! red and a shadow red keep their own screens instead of collapsing
//! into one mid-tone red.

use image::RgbImage;

use crate::engine::layer::Layer;
use crate::engine::workflows::spot::{self, SpotOpts};

pub fn build(source: &RgbImage) -> Vec<Layer> {
    spot::build(
        source,
        SpotOpts {
            // More palette headroom than plain spot: cel art has
            // shadow + mid + highlight variants per hue, and each
            // stays on its own screen because consolidate_hues is off.
            max_colors: 14,
            merge_delta_e: 6.0,
            // Accent details (emblems, weapon trim, eye pupils) are
            // often under half a percent of the image. Keep them.
            min_coverage: 0.0015,
            // Cel-shaded art usually has intentional hue shifts
            // (shadows, highlights) that we want as distinct screens,
            // so hue consolidation is OFF here.
            consolidate_hues: false,
            // Spot plates are hard-binary, so Gaussian-smoothing the
            // mask just produces translucent halos in the composite.
            // Leave mask shaping off by default.
            smooth_radius: 0,
        },
    )
}
