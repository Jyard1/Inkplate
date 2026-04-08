//! Cel-shaded workflow — spot color with a smoothing pass and slightly
//! looser edge handling.
//!
//! Cel-shaded illustration (cartoons, anime art, comic panels) lives in
//! the gap between flat vector and continuous tone: most of the image
//! is flat color regions, but the edges are anti-aliased blends between
//! two inks and the lineart has soft halos. The spot workflow already
//! handles the anti-aliased edges via `spot_aa`, so this workflow is
//! basically "spot + small smoothing radius + slightly looser AA".

use image::RgbImage;

use crate::engine::layer::Layer;
use crate::engine::workflows::spot::{self, SpotOpts};

pub fn build(source: &RgbImage) -> Vec<Layer> {
    spot::build(
        source,
        SpotOpts {
            max_colors: 10,
            merge_delta_e: 6.0,
            min_coverage: 0.003,
            // Cel-shaded art usually has intentional hue shifts
            // (shadows, highlights) that we want as distinct screens,
            // so hue consolidation is OFF here.
            consolidate_hues: false,
            // One pixel of smoothing cleans up lineart halos without
            // softening the fills.
            smooth_radius: 1,
        },
    )
}
