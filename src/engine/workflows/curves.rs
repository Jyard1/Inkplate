//! Shared tone curve presets used by the workflow builders.
//!
//! These are the curves that make the sim-process output actually look
//! right. The raw extractor output is almost never what you want on film:
//!
//! - Color-range output needs a sharp clip so the edge isn't muddy.
//! - Sim-process channels need mid-tone crushing so the darks read dark.
//! - Underbase needs a solid interior with a feathered edge so it doesn't
//!   peek out from under the color screens.
//!
//! Each constant is a `&'static [CurvePoint]` that can be assigned
//! directly to a layer's `tone.curve` field.

use crate::engine::tone::CurvePoint;

/// Sharp binary clip for Color-Range output. Crushes everything below
/// the threshold to no-ink and everything above to full ink, with a
/// narrow feathered transition in between. The narrow transition keeps
/// anti-aliased source edges from turning into ink halos.
pub const CLIP_COLOR_RANGE: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(90, 0),
    CurvePoint::new(115, 255),
    CurvePoint::new(255, 255),
];

/// Sim-process color channel curve. Identity through the midtones,
/// then crushes the tail so dark transitions read cleanly on screen
/// without losing highlight detail.
pub const SIM_PROCESS: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(150, 150),
    CurvePoint::new(220, 255),
    CurvePoint::new(255, 255),
];

/// Underbase curve — smooth monotonic ramp from full ink at very
/// bright source pixels down to no ink at dark ones. No plateaus:
/// plateaus create visible halos at lineart edges because the curve
/// jumps from "full ink" to "feathering" in one step, amplifying any
/// anti-aliased source pixels across the boundary.
///
/// Shape:
/// - bright source (density 0..60) → nearly full ink
/// - bright-mid (60..140)           → mostly ink, fading
/// - mid-dark (140..220)            → mostly no ink
/// - dark (220..255)                → no ink
///
/// For non-black source backgrounds, combine with the background
/// removal pass in the GUI — the curve alone can't distinguish a
/// "bright art highlight" from a "bright area of the source
/// background" because both look identical to the HSB extractor.
pub const UNDERBASE: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(60, 30),
    CurvePoint::new(140, 180),
    CurvePoint::new(220, 250),
    CurvePoint::new(255, 255),
];

/// Black plate curve — keeps only the deepest shadows. Most of the
/// tone range maps to no-ink; only pixels that are almost fully black
/// in the GCR extractor output print.
pub const BLACK_PLATE: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(80, 0),
    CurvePoint::new(160, 200),
    CurvePoint::new(255, 255),
];

/// Highlight white curve — the mirror of the black plate, biased toward
/// only the brightest pixels. Used for the near-last "highlight" screen
/// on sim-process dark jobs.
pub const HIGHLIGHT_WHITE: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(100, 0),
    CurvePoint::new(180, 220),
    CurvePoint::new(255, 255),
];

/// Hard-clip underbase for CMYK process on dark shirts. Binarizes
/// the HSB brightness-inverted mask into solid white ink wherever
/// the source has actual art content. The clip is aggressive: only
/// pixels with density below ~55% (well into the "dark source"
/// range) get full white ink. This prevents specular highlights,
/// near-white textures, and anti-aliased edge fragments from
/// printing as random white speckles on the shirt. Combined with
/// `noise_open: 2` on the mask, the result is a clean binary plate
/// with solid fills and no stray dots.
pub const UNDERBASE_SOLID: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(140, 0),
    CurvePoint::new(160, 255),
    CurvePoint::new(255, 255),
];

/// Underbase for dark shirts (sim-process and CMYK). Much more
/// permissive than `UNDERBASE_SOLID` — keeps white ink under
/// everything except near-black source pixels. Dark browns, maroons,
/// and deep blues all need white underneath or the color ink is
/// invisible on a dark shirt. Only truly dark pixels (density > ~220,
/// i.e. source brightness below ~13%) get dropped.
///
/// Gradual feathered transition (203→237, 34 units wide) centered
/// on the density-220 threshold so the effective binarize cutoff
/// matches the original hard step. The pipeline applies a Gaussian
/// blur (σ=1.5) after this curve and before binarization, so
/// isolated noise pixels in the transition zone get averaged into
/// their neighborhood and fall cleanly on one side of the 128
/// threshold. The result is clean binary edges with no scattered
/// dots and correct black handling.
pub const UNDERBASE_SIM: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(203, 0),
    CurvePoint::new(237, 255),
    CurvePoint::new(255, 255),
];

/// Composite-union underbase curve. The input is the per-pixel min
/// density across all non-black color layers' previews (0 = at least
/// one layer has full ink, 255 = no layer has ink). Very permissive:
/// any pixel with >5% color ink gets full underbase — on a dark shirt
/// even light color coverage needs white underneath to be visible.
/// The 15-unit transition zone (235→250) feeds the Gaussian blur so
/// the edge comes out smooth after binarization.
pub const UNDERBASE_COMPOSITE: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(235, 0),
    CurvePoint::new(250, 255),
    CurvePoint::new(255, 255),
];

/// CMYK channel curve — near-identity with a gentle shoulder that
/// clips very light coverage (below ~6% ink) to zero. Prevents
/// isolated halftone dots in near-white areas that would read as
/// speckle noise on the print. The user can reshape per-layer.
pub const CMYK_CHANNEL: &[CurvePoint] = &[
    CurvePoint::new(0, 0),
    CurvePoint::new(200, 200),
    CurvePoint::new(240, 252),
    CurvePoint::new(255, 255),
];
