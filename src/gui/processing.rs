//! Thin wrappers around the engine pipeline for GUI use.
//!
//! These exist so the panels don't have to care about the state layout;
//! they just call `rerun_layer(state, idx)` or `rerun_all(state)` after
//! mutating something.

use std::sync::Arc;

use image::{GrayImage, ImageBuffer, Luma};
use inkplate::engine::foreground::detect_foreground_mask;
use inkplate::engine::pipeline::process_layer;
use inkplate::engine::workflows::run as run_workflow;

use super::state::{GuiState, LayerEntry};

/// Recompute the cached foreground mask from the current source and
/// bg-removal settings.
///
/// Priority:
/// 1. If the source has a usable alpha channel (loaded from an RGBA
///    PNG), use it directly — the artist already cut the art out.
/// 2. Else if `bg_removal.enabled`, run flood-fill detection against
///    the border color.
/// 3. Else no mask.
pub fn rebuild_foreground_mask(state: &mut GuiState) {
    // Alpha channel always wins.
    if let Some(alpha) = &state.source_alpha {
        state.foreground_mask = Some(alpha.clone());
        return;
    }

    if !state.bg_removal.enabled {
        state.foreground_mask = None;
        return;
    }
    let Some(source) = state.source.clone() else {
        state.foreground_mask = None;
        return;
    };
    let mask = detect_foreground_mask(&source, None, state.bg_removal.tolerance);
    state.foreground_mask = Some(Arc::new(mask));
}

/// Extract the alpha channel of an RGBA image as a binary GrayImage
/// (255 = opaque/foreground, 0 = transparent/background). Returns
/// `None` if every pixel is fully opaque — no need to cache a mask
/// that does nothing.
pub fn alpha_to_foreground_mask(img: &image::RgbaImage) -> Option<GrayImage> {
    let (w, h) = img.dimensions();
    let mut out: GrayImage = ImageBuffer::new(w, h);
    let mut any_transparent = false;
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] < 128 {
            any_transparent = true;
            out.put_pixel(x, y, Luma([0]));
        } else {
            out.put_pixel(x, y, Luma([255]));
        }
    }
    if any_transparent {
        Some(out)
    } else {
        None
    }
}

/// Rerun the whole workflow against the current source and replace
/// the layer list.
pub fn rerun_workflow(state: &mut GuiState) {
    let source = match state.source.clone() {
        Some(s) => s,
        None => {
            state.status = "no source image loaded".into();
            return;
        }
    };
    let layers = run_workflow(state.workflow, &source, &state.workflow_opts);
    state.layers = layers.into_iter().map(LayerEntry::new).collect();
    state.selected = if state.layers.is_empty() {
        None
    } else {
        Some(0)
    };
    rerun_all_layers(state);
    state.composite_dirty = true;
    state.status = format!(
        "ran {} → {} layers",
        state.workflow.label(),
        state.layers.len()
    );
}

/// Auto-detect + run.
pub fn auto_rerun(state: &mut GuiState) {
    if let Some(source) = state.source.clone() {
        state.workflow = inkplate::engine::workflows::auto_detect::detect(&source);
    }
    rerun_workflow(state);
}

/// Rerun `process_layer` for every entry in the current state. The
/// cached foreground mask (if any) is passed in so every layer's
/// output has the background clamped to no-ink consistently.
pub fn rerun_all_layers(state: &mut GuiState) {
    let source = match state.source.clone() {
        Some(s) => s,
        None => return,
    };
    let job = state.job;
    let fg = state.foreground_mask_for_pipeline();
    let fg_ref = fg.as_deref();
    for entry in state.layers.iter_mut() {
        let processed = process_layer(&source, &entry.layer, job, fg_ref);
        entry.coverage = coverage_fraction(&processed.preview);
        entry.preview = Some(processed.preview);
        entry.processed = Some(processed.processed);
    }
    state.composite_dirty = true;
}

/// Rerun one layer — the common case when a slider moves in the
/// inspector. Leaves the rest of the layer list untouched.
pub fn rerun_layer(state: &mut GuiState, idx: usize) {
    let source = match state.source.clone() {
        Some(s) => s,
        None => return,
    };
    let job = state.job;
    let fg = state.foreground_mask_for_pipeline();
    let fg_ref = fg.as_deref();
    if let Some(entry) = state.layers.get_mut(idx) {
        let processed = process_layer(&source, &entry.layer, job, fg_ref);
        entry.coverage = coverage_fraction(&processed.preview);
        entry.preview = Some(processed.preview);
        entry.processed = Some(processed.processed);
        state.composite_dirty = true;
    }
}

/// Rerun only the currently selected layer, if any. Not currently
/// called (the app reruns per-layer directly from the inspector) but
/// kept because hotkey handling in Landing 5-followup will want it.
#[allow(dead_code)]
pub fn rerun_selected(state: &mut GuiState) {
    if let Some(idx) = state.selected {
        rerun_layer(state, idx);
    }
}

/// Fraction of pixels that are dark (< 128) in a preview mask. That's
/// "how much ink this layer prints", as a [0, 1] coverage.
fn coverage_fraction(img: &image::GrayImage) -> f32 {
    let total = img.width() as f32 * img.height() as f32;
    if total <= 0.0 {
        return 0.0;
    }
    let ink = img.iter().filter(|&&p| p < 128).count() as f32;
    ink / total
}
