//! Layer data model.
//!
//! A [`Layer`] is a single screen in a separation job: its identity, the
//! extractor that turns the source image into a density map, the tone
//! curve that shapes it, the mask-shaping morphology that cleans it up,
//! and the render mode that rasterizes it into a film.
//!
//! Everything in this file is `serde`-serializable and lives inside
//! `.inkplate` project files. Cache fields (rebuildable from source + spec)
//! go on [`LayerCache`], which is *not* persisted.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::engine::color::Rgb;
use crate::engine::halftone::{DotShape, HalftoneCurve};
use crate::engine::tone::CurvePoint;

/// Broad role a layer plays in a separation. Drives defaults and print
/// order, but the real behavior comes from `extractor` + `render_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum LayerKind {
    Spot,
    #[default]
    Color,
    Underbase,
    Highlight,
    Shadow,
}

/// Which density-map extractor to run against the source image.
///
/// The enum variants match the 9 extractors in the rebuild plan plus
/// `ManualPaint` for user-painted touch-up layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Extractor {
    /// Binary match: pixel == target ± tolerance. Flat vector fills.
    SpotSolid { target: Rgb, tolerance: u8 },
    /// Soft anti-aliased spot with Voronoi + soft-AA falloff.
    SpotAa {
        targets: Vec<Rgb>,
        others: Vec<Rgb>,
        aa_full: f32,
        aa_end: f32,
        aa_reach: u32,
        /// Per-target distance offsets, parallel to `targets`.
        /// Positive value = "this plate reaches further" (its
        /// CIE94 distance is subtracted by this amount, so it
        /// wins pixels it would otherwise lose). Negative =
        /// "reach less". Default is all zeros, which is plain
        /// Voronoi. Must have the same length as `targets`;
        /// deserialization fills in zeros for projects saved
        /// before the field existed.
        #[serde(default)]
        target_weights: Vec<f32>,
    },
    /// Photoshop Color Range: LAB-ΔE density ramp with falloff.
    ColorRange {
        target: Rgb,
        fuzziness: f32,
        falloff: ColorRangeFalloff,
    },
    /// HSB brightness inverted — the correct underbase recipe.
    HsbBrightnessInverted {
        s_curve: f32,
        boost_under_darks: bool,
        boost_strength: f32,
    },
    /// LAB L inverted — underbase fallback for non-saturated artwork.
    LabLightnessInverted,
    /// GCR black: `min(1-R, 1-G, 1-B) * strength`. Black plate / highlight.
    GcrBlack { strength: f32, invert_input: bool },
    /// Tiny expression DSL: `max(0, R-G)`, `1-L`, etc.
    ChannelCalc { expr: String },
    /// Luminance threshold → binary stencil.
    LuminanceThreshold { threshold: u8, above: bool },
    /// Resolve one palette index via FS or Bayer dither.
    IndexAssignment {
        palette: Vec<Rgb>,
        index: u32,
        // Renamed from `kind` because the enum uses #[serde(tag = "kind")]
        // and the two can't coexist.
        dither: IndexDitherKind,
    },
    /// User-painted mask. The pipeline returns the [`ManualPaintBuf`]
    /// buffer verbatim (density convention: 0 = ink, 255 = no ink).
    /// The buffer is `None` until the user actually paints something,
    /// at which point the GUI allocates one matching the source
    /// dimensions.
    ManualPaint { buf: Option<ManualPaintBuf> },
}

/// Raw stroke buffer for [`Extractor::ManualPaint`].
///
/// Stored as flat 8-bit grayscale bytes so it round-trips through
/// serde without needing a base64 escape hatch. For multi-megapixel
/// layers this is chunky in `.inkplate` project files but simple and
/// lossless; we can swap in PNG-compressed storage later if needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualPaintBuf {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl ManualPaintBuf {
    /// Allocate a fresh "no ink anywhere" buffer at the given size.
    pub fn blank(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![255u8; (width as usize) * (height as usize)],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorRangeFalloff {
    Linear,
    Quadratic,
    Smooth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexDitherKind {
    Fs,
    Bayer,
}

/// How a finalized density map is rasterized into a film.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RenderMode {
    #[default]
    Solid,
    Halftone,
    FmDither,
    BayerDither,
    NoiseDither,
    BlueNoise,
    IndexFs,
    IndexBayer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum EdgeMode {
    #[default]
    Hard,
    Choke,
    Spread,
    FeatherHt,
}

/// All the mask-shaping morphology settings for a layer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MaskShape {
    pub smooth_radius: u32,
    pub noise_open: u32,
    pub holes_close: u32,
    pub edge_mode: EdgeMode,
    pub edge_radius: u32,
    pub invert: bool,
}

impl Default for MaskShape {
    fn default() -> Self {
        Self {
            smooth_radius: 0,
            noise_open: 0,
            holes_close: 0,
            edge_mode: EdgeMode::Hard,
            edge_radius: 0,
            invert: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tone {
    pub curve: Vec<CurvePoint>,
    pub density: f32,
    pub choke: u32,
}

impl Default for Tone {
    fn default() -> Self {
        Self {
            curve: vec![CurvePoint::new(0, 0), CurvePoint::new(255, 255)],
            density: 1.0,
            choke: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HalftoneOverrides {
    /// `0` means inherit from the job-global LPI.
    pub lpi: u32,
    /// `-1.0` means auto-cycle based on the layer's print index.
    pub angle_deg: f32,
    pub dot_shape: Option<DotShape>,
    pub curve: HalftoneCurve,
}

impl Default for HalftoneOverrides {
    fn default() -> Self {
        Self {
            lpi: 0,
            angle_deg: -1.0,
            dot_shape: None,
            curve: HalftoneCurve::Linear,
        }
    }
}

/// A single separation layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: Uuid,
    pub name: String,
    pub kind: LayerKind,
    pub ink: Rgb,
    pub visible: bool,
    pub locked: bool,
    pub include_in_export: bool,
    pub opacity: f32,

    pub extractor: Extractor,
    pub tone: Tone,
    pub mask: MaskShape,
    pub render_mode: RenderMode,
    pub halftone: HalftoneOverrides,

    /// Print order index, assigned by the GUI when the layer list is
    /// reordered. Kept on the layer so background workers don't need
    /// access to the list.
    pub print_index: u32,
}

impl Layer {
    /// Fresh spot-color layer named after its ink swatch. Used as the
    /// baseline for every new layer created by workflow presets and the
    /// GUI "Add layer" button.
    pub fn new_spot(ink: Rgb) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: format!("{ink}"),
            kind: LayerKind::Spot,
            ink,
            visible: true,
            locked: false,
            include_in_export: true,
            opacity: 1.0,
            extractor: Extractor::SpotSolid {
                target: ink,
                tolerance: 0,
            },
            tone: Tone::default(),
            mask: MaskShape::default(),
            render_mode: RenderMode::Solid,
            halftone: HalftoneOverrides::default(),
            print_index: 0,
        }
    }
}
