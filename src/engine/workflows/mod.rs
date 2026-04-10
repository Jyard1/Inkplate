//! Workflow presets — the 11 "this is the thing you actually want to do"
//! entry points that build a starting layer list from a source image.
//!
//! Each workflow is a pure function: `source + opts → Vec<Layer>`. The
//! returned layers are fully specified (extractor, tone, mask, render
//! mode) so running them through the pipeline produces a complete set
//! of films without any UI interaction.
//!
//! Callers should prefer the [`Workflow`] enum + [`run`] dispatch so
//! that the GUI (and auto-detect) can work from a single source of
//! truth. For more control, call the submodule `build` functions
//! directly with custom opts.
//!
//! | Preset                 | Auto-builds                                                                 |
//! |------------------------|-----------------------------------------------------------------------------|
//! | `spot`                 | One spot per LAB-clustered palette color                                    |
//! | `cel_shaded`           | Same as spot + smoothing + looser merge                                     |
//! | `simprocess_light`     | N color channels + black plate + highlight white (no underbase)             |
//! | `simprocess_dark`      | Underbase + N color channels + black plate + highlight white                |
//! | `single_halftone`      | One B&W channel (inverted L)                                                |
//! | `black_only`           | One solid black layer                                                       |
//! | `stencil`              | One luminance-threshold binary layer                                        |
//! | `duotone`              | Light tone + dark tone                                                      |
//! | `tritone`              | Highlight + mid + shadow                                                    |
//! | `index_fs`             | Floyd-Steinberg palette assignment, one layer per palette entry             |
//! | `index_bayer`          | Bayer palette assignment, one layer per palette entry                       |

use image::RgbImage;
use serde::{Deserialize, Serialize};

use crate::engine::color::Rgb;
use crate::engine::layer::Layer;

pub mod auto_detect;
pub mod black_only;
pub mod cel_shaded;
pub mod cmyk_process;
pub mod curves;
pub mod duotone;
pub mod index;
pub mod simprocess;
pub mod single_halftone;
pub mod spot;
pub mod stencil;
pub mod tritone;

/// The 11 workflow presets, as a closed enum so the GUI and auto-detect
/// can reference workflows by value instead of by string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Workflow {
    Spot,
    CelShaded,
    SimprocessLight,
    SimprocessDark,
    CmykProcessLight,
    CmykProcessDark,
    SingleHalftone,
    BlackOnly,
    Stencil,
    Duotone,
    Tritone,
    IndexFs,
    IndexBayer,
}

impl Workflow {
    /// Short kebab-case identifier used on the command line and in
    /// `.inkplate` project files.
    pub fn id(self) -> &'static str {
        match self {
            Workflow::Spot => "spot",
            Workflow::CelShaded => "cel_shaded",
            Workflow::SimprocessLight => "simprocess_light",
            Workflow::SimprocessDark => "simprocess_dark",
            Workflow::CmykProcessLight => "cmyk_process_light",
            Workflow::CmykProcessDark => "cmyk_process_dark",
            Workflow::SingleHalftone => "single_halftone",
            Workflow::BlackOnly => "black_only",
            Workflow::Stencil => "stencil",
            Workflow::Duotone => "duotone",
            Workflow::Tritone => "tritone",
            Workflow::IndexFs => "index_fs",
            Workflow::IndexBayer => "index_bayer",
        }
    }

    /// Human-readable label for GUI menus and logs.
    pub fn label(self) -> &'static str {
        match self {
            Workflow::Spot => "Spot color",
            Workflow::CelShaded => "Cel-shaded",
            Workflow::SimprocessLight => "Sim-process (light shirt)",
            Workflow::SimprocessDark => "Sim-process (dark shirt)",
            Workflow::CmykProcessLight => "CMYK process (light shirt)",
            Workflow::CmykProcessDark => "CMYK process (dark shirt)",
            Workflow::SingleHalftone => "Single halftone",
            Workflow::BlackOnly => "Black only",
            Workflow::Stencil => "Stencil",
            Workflow::Duotone => "Duotone",
            Workflow::Tritone => "Tritone",
            Workflow::IndexFs => "Index (Floyd-Steinberg)",
            Workflow::IndexBayer => "Index (Bayer)",
        }
    }

    pub fn all() -> &'static [Workflow] {
        &[
            Workflow::Spot,
            Workflow::CelShaded,
            Workflow::SimprocessLight,
            Workflow::SimprocessDark,
            Workflow::CmykProcessLight,
            Workflow::CmykProcessDark,
            Workflow::SingleHalftone,
            Workflow::BlackOnly,
            Workflow::Stencil,
            Workflow::Duotone,
            Workflow::Tritone,
            Workflow::IndexFs,
            Workflow::IndexBayer,
        ]
    }

    /// Parse a workflow by its id.
    pub fn from_id(id: &str) -> Option<Workflow> {
        Self::all().iter().copied().find(|w| w.id() == id)
    }
}

/// Options shared by every workflow. Workflows pick what they need and
/// ignore the rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowOpts {
    pub max_colors: usize,
    pub fuzziness: f32,
    pub stencil_threshold: u8,
    /// When true, the spot and sim-process workflows collapse
    /// same-hue shades (bright red + blood red → one red plate) via
    /// [`crate::engine::palette::consolidate_by_hue`]. Default is
    /// **off** — modern k-means + CIE94 clustering already produces
    /// well-separated centers, and merging them after the fact
    /// throws away colors the user explicitly asked for when they
    /// raised `max_colors`.
    #[serde(default)]
    pub consolidate_hues: bool,
    /// GCR (Grey Component Replacement) strength for the CMYK
    /// workflow. 0.0 = no GCR, 1.0 = full GCR. Default 0.75.
    #[serde(default = "default_gcr")]
    pub gcr_strength: f32,
    pub duotone_light: Rgb,
    pub duotone_dark: Rgb,
    pub tritone_highlight: Rgb,
    pub tritone_mid: Rgb,
    pub tritone_shadow: Rgb,
}

impl Default for WorkflowOpts {
    fn default() -> Self {
        Self {
            max_colors: 16,
            fuzziness: 40.0,
            stencil_threshold: 128,
            consolidate_hues: false,
            gcr_strength: 0.75,
            duotone_light: Rgb(240, 220, 200),
            duotone_dark: Rgb(30, 40, 80),
            tritone_highlight: Rgb(240, 220, 200),
            tritone_mid: Rgb(160, 80, 60),
            tritone_shadow: Rgb(30, 20, 20),
        }
    }
}

/// Run a workflow against a source image, returning the layer list that
/// the pipeline should then process.
pub fn run(workflow: Workflow, source: &RgbImage, opts: &WorkflowOpts) -> Vec<Layer> {
    match workflow {
        Workflow::Spot => spot::build(
            source,
            spot::SpotOpts {
                max_colors: opts.max_colors,
                consolidate_hues: opts.consolidate_hues,
                ..spot::SpotOpts::default()
            },
        ),
        Workflow::CelShaded => cel_shaded::build(source),
        Workflow::SimprocessLight => simprocess::build_light(
            source,
            simprocess::SimOpts {
                max_colors: opts.max_colors,
                fuzziness: opts.fuzziness,
                consolidate_hues: opts.consolidate_hues,
            },
        ),
        Workflow::SimprocessDark => simprocess::build_dark(
            source,
            simprocess::SimOpts {
                max_colors: opts.max_colors,
                fuzziness: opts.fuzziness,
                consolidate_hues: opts.consolidate_hues,
            },
        ),
        Workflow::CmykProcessLight => cmyk_process::build_light(
            source,
            cmyk_process::CmykOpts {
                gcr_strength: opts.gcr_strength,
            },
        ),
        Workflow::CmykProcessDark => cmyk_process::build_dark(
            source,
            cmyk_process::CmykOpts {
                gcr_strength: opts.gcr_strength,
            },
        ),
        Workflow::SingleHalftone => single_halftone::build(source),
        Workflow::BlackOnly => black_only::build(source),
        Workflow::Stencil => stencil::build(source, opts.stencil_threshold),
        Workflow::Duotone => duotone::build(source, opts.duotone_light, opts.duotone_dark),
        Workflow::Tritone => tritone::build(
            source,
            opts.tritone_highlight,
            opts.tritone_mid,
            opts.tritone_shadow,
        ),
        Workflow::IndexFs => index::build_fs(
            source,
            index::IndexOpts {
                max_colors: opts.max_colors,
            },
        ),
        Workflow::IndexBayer => index::build_bayer(
            source,
            index::IndexOpts {
                max_colors: opts.max_colors,
            },
        ),
    }
}

/// Convenience wrapper: auto-detect + run in one call.
pub fn auto_run(source: &RgbImage, opts: &WorkflowOpts) -> (Workflow, Vec<Layer>) {
    let workflow = auto_detect::detect(source);
    let layers = run(workflow, source, opts);
    (workflow, layers)
}

fn default_gcr() -> f32 {
    0.75
}
