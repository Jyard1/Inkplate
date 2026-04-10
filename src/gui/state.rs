//! GUI state — one struct holding everything the app needs between
//! frames.
//!
//! Keeps layers with their cached preview / processed images together
//! so the inspector can rerun a single layer without touching the
//! others. All mutation goes through the `InkplateApp` in `app.rs`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use image::{GrayImage, RgbImage};
use uuid::Uuid;

use inkplate::engine::color::Rgb;
use inkplate::engine::layer::Layer;
use inkplate::engine::pipeline::JobOpts;
use inkplate::engine::workflows::{Workflow, WorkflowOpts};

/// Background removal controls. When enabled, the GUI builds a
/// foreground mask via edge-seeded flood fill and passes it through
/// the pipeline so every layer's output has the background clamped
/// to "no ink". If the source image has an alpha channel, it takes
/// precedence over the flood-fill — the mask comes straight from
/// alpha.
#[derive(Debug, Clone)]
pub struct BackgroundRemoval {
    pub enabled: bool,
    pub tolerance: f32,
}

impl Default for BackgroundRemoval {
    fn default() -> Self {
        Self {
            enabled: false,
            tolerance: 12.0,
        }
    }
}

/// A layer plus its most recent processed outputs. The pipeline writes
/// `preview` (smooth mask) and `processed` (rasterized); the composite
/// panel uses `preview`, export uses `processed`.
#[derive(Clone)]
pub struct LayerEntry {
    pub layer: Layer,
    pub preview: Option<GrayImage>,
    pub processed: Option<GrayImage>,
    /// Last coverage fraction measured on the preview mask, for
    /// display in the layer list (%).
    pub coverage: f32,
}

impl LayerEntry {
    pub fn new(layer: Layer) -> Self {
        Self {
            layer,
            preview: None,
            processed: None,
            coverage: 0.0,
        }
    }
}

/// Which image the center panel is showing. `Composite` blends all
/// visible layer preview masks in their ink colors over the shirt
/// color; `Source` shows the raw loaded image; `Layer(i)` shows one
/// layer's mask in isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewMode {
    #[default]
    Composite,
    Source,
    Layer(usize),
}

pub struct GuiState {
    pub source_path: Option<PathBuf>,
    /// `Arc` so the processing functions can clone a reference cheaply
    /// without forcing the full image to be cloned per-layer.
    pub source: Option<Arc<RgbImage>>,
    /// Alpha channel from the source, if the loaded file was RGBA.
    /// When present, this is used as-is to build the foreground mask
    /// without running flood fill — the source editor already
    /// decided what's art and what isn't.
    pub source_alpha: Option<Arc<GrayImage>>,

    pub workflow: Workflow,
    pub workflow_opts: WorkflowOpts,
    pub job: JobOpts,

    pub layers: Vec<LayerEntry>,
    pub selected: Option<usize>,
    /// Secondary selection used for batch operations (currently
    /// "Merge → shadow halftone"). Stored as Uuid so it survives
    /// reorders and deletions without us having to rewrite indices.
    /// Ctrl+click on a layer row toggles it. Plain click clears.
    pub multi_select: HashSet<Uuid>,

    pub shirt_color: Rgb,
    pub preview_mode: PreviewMode,

    /// Background removal controls and the currently cached mask.
    /// The mask is recomputed whenever the toggle flips on, the
    /// tolerance slider changes, or a new image is loaded. Pipeline
    /// call sites pick this up via `foreground_mask_for_pipeline`.
    pub bg_removal: BackgroundRemoval,
    pub foreground_mask: Option<Arc<GrayImage>>,

    /// Short message shown in the status bar. Updated by actions and
    /// cleared on the next frame so it doesn't go stale.
    pub status: String,

    /// True while the composite preview texture needs to be rebuilt.
    /// Set by any action that changes a layer's preview mask.
    pub composite_dirty: bool,

    /// Center-panel viewport transform. `fit` means "ignore zoom and
    /// pan, scale to fit the panel"; otherwise the `zoom` (pixel-
    /// space scale) and `pan` (offset in screen pixels from the
    /// panel centre) fields drive the display.
    pub viewport: Viewport,

    /// Manual-paint brush state for `Extractor::ManualPaint` layers.
    pub brush: BrushState,

    /// Clamp near-black source pixels to pure (0,0,0) before
    /// extraction so color channels don't report spurious ink in
    /// dark areas. 0 = off, N = clamp pixels with max(R,G,B) < N.
    pub clamp_black_threshold: u8,
}

/// Manual-paint brush settings — size, mode, active state.
#[derive(Debug, Clone, Copy)]
pub struct BrushState {
    /// Brush radius in source pixels.
    pub radius: u32,
    /// True = paint ink (density 0), false = erase (density 255).
    pub paint_mode: bool,
}

impl Default for BrushState {
    fn default() -> Self {
        Self {
            radius: 16,
            paint_mode: true,
        }
    }
}

/// Zoom / pan state for the center preview panel.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub fit: bool,
    pub zoom: f32,
    pub pan: egui::Vec2,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            fit: true,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            source_path: None,
            source: None,
            source_alpha: None,
            workflow: Workflow::SimprocessDark,
            workflow_opts: WorkflowOpts::default(),
            job: JobOpts::default(),
            layers: Vec::new(),
            selected: None,
            multi_select: HashSet::new(),
            shirt_color: Rgb(24, 24, 28),
            preview_mode: PreviewMode::Composite,
            bg_removal: BackgroundRemoval::default(),
            foreground_mask: None,
            status: "ready".into(),
            composite_dirty: true,
            viewport: Viewport::default(),
            brush: BrushState::default(),
            clamp_black_threshold: 50,
        }
    }
}

impl GuiState {
    #[allow(dead_code)]
    pub fn selected_layer(&self) -> Option<&LayerEntry> {
        self.selected.and_then(|i| self.layers.get(i))
    }

    #[allow(dead_code)]
    pub fn selected_layer_mut(&mut self) -> Option<&mut LayerEntry> {
        let idx = self.selected?;
        self.layers.get_mut(idx)
    }

    /// Foreground mask to pass into `process_layer`, or `None` if no
    /// knockout is wanted. Alpha-derived masks are always honored,
    /// even when the bg_removal toggle is off — a transparent source
    /// is an unambiguous signal from the artist about what's art.
    pub fn foreground_mask_for_pipeline(&self) -> Option<Arc<GrayImage>> {
        self.foreground_mask.clone()
    }
}
