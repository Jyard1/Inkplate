//! Thin wrappers around the engine pipeline for GUI use.
//!
//! These exist so the panels don't have to care about the state layout;
//! they just call `rerun_layer(state, idx)` or `rerun_all(state)` after
//! mutating something.

use std::collections::HashSet;
use std::sync::Arc;

use image::{GrayImage, ImageBuffer, Luma};
use inkplate::engine::color::{rgb_to_lab, Rgb};
use inkplate::engine::foreground::detect_foreground_mask;
use inkplate::engine::halftone::{DotShape, HalftoneCurve};
use inkplate::engine::layer::{
    Extractor, HalftoneOverrides, Layer, LayerKind, MaskShape, RenderMode, Tone,
};
use inkplate::engine::pipeline::process_layer;
use inkplate::engine::workflows::run as run_workflow;
use uuid::Uuid;

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

/// Run the workflow against the current source and replace the
/// layer list, but *don't* call `process_layer` on each entry. The
/// worker thread will fill in preview/processed masks asynchronously.
///
/// This is the "cheap" half of rerun_workflow that stays on the UI
/// thread because palette extraction and layer construction are fast.
pub fn rerun_workflow_layers_only(state: &mut GuiState) {
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
    state.composite_dirty = true;
    state.status = format!(
        "ran {} → {} layers",
        state.workflow.label(),
        state.layers.len()
    );
}

/// Auto-detect workflow + rebuild layer list. Does not process layers.
pub fn auto_rerun(state: &mut GuiState) {
    if let Some(source) = state.source.clone() {
        state.workflow = inkplate::engine::workflows::auto_detect::detect(&source);
    }
    rerun_workflow_layers_only(state);
}

/// Synchronous fallback used at app startup before the worker has
/// had a chance to process anything — also the path the sync tests
/// exercise. Production callers should go through the worker.
#[allow(dead_code)]
pub fn rerun_all_layers_sync(state: &mut GuiState) {
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

/// Propagate the reach weights from the selected SpotAa layer onto
/// every other SpotAa layer that shares a target color.
///
/// Every spot workflow layer carries its own clone of the targets
/// list. When the user drags the Reach slider on layer A, we need
/// layer B's spot_aa evaluation to *also* see the new weight for
/// A's ink, otherwise B will keep claiming pixels that A's higher
/// reach was supposed to win. This walks the layer list and writes
/// each target color's weight into every matching slot.
///
/// Keyed by ink RGB, not index, because layers may have reordered
/// or had other layers inserted between them.
pub fn sync_spot_reach_weights(state: &mut GuiState, source_layer_idx: usize) {
    // Snapshot the source layer's (ink, weight) pairs.
    let Some(src) = state.layers.get(source_layer_idx) else {
        return;
    };
    let Extractor::SpotAa {
        targets,
        target_weights,
        ..
    } = &src.layer.extractor
    else {
        return;
    };
    if targets.len() != target_weights.len() {
        return;
    }
    let pairs: Vec<(Rgb, f32)> = targets
        .iter()
        .copied()
        .zip(target_weights.iter().copied())
        .collect();

    // Write those weights into every other SpotAa layer. Any target
    // the other layer doesn't know about is skipped.
    for (i, entry) in state.layers.iter_mut().enumerate() {
        if i == source_layer_idx {
            continue;
        }
        if let Extractor::SpotAa {
            targets,
            target_weights,
            ..
        } = &mut entry.layer.extractor
        {
            if target_weights.len() != targets.len() {
                target_weights.clear();
                target_weights.resize(targets.len(), 0.0);
            }
            for (idx, color) in targets.iter().enumerate() {
                if let Some((_, w)) = pairs.iter().find(|(c, _)| c == color) {
                    target_weights[idx] = *w;
                }
            }
        }
    }
}

/// Fraction of pixels that are dark (< 128) in a preview mask.
pub fn coverage_fraction(img: &image::GrayImage) -> f32 {
    let total = img.width() as f32 * img.height() as f32;
    if total <= 0.0 {
        return 0.0;
    }
    let ink = img.iter().filter(|&&p| p < 128).count() as f32;
    ink / total
}

// ---------------------------------------------------------------------------
// Merge → shadow halftone
// ---------------------------------------------------------------------------

/// Result of [`merge_to_shadow_halftone`] — mostly for the caller's
/// status line so we don't duplicate the description string.
pub struct ShadowMergeReport {
    pub merged_count: usize,
    pub keeper_name: String,
}

/// Collapse a set of same-hue layers into a single bright-color
/// plate plus a black shadow halftone plate.
///
/// Algorithm:
///
/// 1. Gather the selected layers (by Uuid) in their current print
///    order. Need at least two to do anything meaningful.
/// 2. Pick the *keeper*: whichever layer has the lightest ink
///    (highest LAB L*). That's the plate whose ink we'll print
///    across the union of all the selected regions.
/// 3. Compute the **union mask** — a pixel has ink if *any* of the
///    selected layers had ink there. That becomes the keeper's new
///    preview mask.
/// 4. Compute the **shadow mask**: for each pixel, find which
///    selected layer originally owned it and emit a grayscale
///    density proportional to the lightness gap between that
///    layer's ink and the keeper's ink. Keeper's own pixels get
///    no shadow ink; the darkest shade gets the most.
/// 5. Drop the other selected layers from the list, leaving the
///    keeper (with its unioned mask) plus a new black Halftone
///    shadow layer printing just after it.
///
/// Uses the pre-computed `LayerEntry.preview` masks, so every
/// selected layer must have been processed at least once — i.e.
/// the caller needs to make sure no worker jobs are still in
/// flight for any of them before invoking this.
pub fn merge_to_shadow_halftone(
    state: &mut GuiState,
    selection: &HashSet<Uuid>,
) -> Option<ShadowMergeReport> {
    if selection.len() < 2 {
        return None;
    }

    // Gather everything we need from the selected layers in one
    // immutable-borrow pass, keyed by Uuid. After this block we
    // only touch `state.layers` through mutable borrows.
    struct SelSnap {
        id: Uuid,
        mask: GrayImage,
        lightness: f32,
        name: String,
        ink: Rgb,
        print_index: u32,
    }

    let snaps: Vec<SelSnap> = state
        .layers
        .iter()
        .filter(|e| selection.contains(&e.layer.id))
        .filter_map(|e| {
            let mask = e.preview.as_ref()?.clone();
            Some(SelSnap {
                id: e.layer.id,
                mask,
                lightness: rgb_to_lab(e.layer.ink).l,
                name: e.layer.name.clone(),
                ink: e.layer.ink,
                print_index: e.layer.print_index,
            })
        })
        .collect();
    if snaps.len() < 2 {
        return None;
    }

    // Every selected layer must have preview masks at the same size.
    let (w, h) = snaps[0].mask.dimensions();
    if snaps.iter().any(|s| s.mask.dimensions() != (w, h)) {
        return None;
    }

    // Pick the keeper: lightest ink wins. Sort by descending L*.
    let mut ordered = snaps;
    ordered.sort_by(|a, b| {
        b.lightness
            .partial_cmp(&a.lightness)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let keeper_id = ordered[0].id;
    let keeper_l = ordered[0].lightness;
    let keeper_name = ordered[0].name.clone();
    let keeper_print_index = ordered[0].print_index;
    let _keeper_ink = ordered[0].ink;

    // Precompute per-layer shadow ink values. Keeper is 0 ink (no
    // darkening), each darker layer gets a black-ink percentage
    // proportional to its lightness gap to the keeper. Clamp the
    // denominator so a keeper at L*≈0 doesn't divide by zero.
    let denom = keeper_l.max(1.0);
    let shadow_ink_for: Vec<(Uuid, u8)> = ordered
        .iter()
        .map(|s| {
            let ratio = ((keeper_l - s.lightness).max(0.0) / denom).clamp(0.0, 1.0);
            // Ink ∈ [0, 255]. Keeper (ratio=0) → 0 ink. Full black
            // (ratio=1) → 255 ink. Density = 255 − ink.
            let ink = (ratio * 255.0).round().clamp(0.0, 255.0) as u8;
            (s.id, ink)
        })
        .collect();

    // Walk every pixel once. For each pixel find whichever selected
    // layer owned it (first match in descending-lightness order).
    let mut union_mask: GrayImage = ImageBuffer::from_pixel(w, h, Luma([255u8]));
    let mut shadow_mask: GrayImage = ImageBuffer::from_pixel(w, h, Luma([255u8]));

    for y in 0..h {
        for x in 0..w {
            let mut owning_shadow_ink: Option<u8> = None;
            for snap in &ordered {
                if snap.mask.get_pixel(x, y)[0] < 128 {
                    owning_shadow_ink = shadow_ink_for
                        .iter()
                        .find(|(i, _)| *i == snap.id)
                        .map(|(_, ink)| *ink);
                    break;
                }
            }
            if let Some(ink) = owning_shadow_ink {
                union_mask.put_pixel(x, y, Luma([0]));
                shadow_mask.put_pixel(x, y, Luma([255u8.saturating_sub(ink)]));
            }
        }
    }
    let merged_count = ordered.len();
    let drop_ids: HashSet<Uuid> = ordered
        .iter()
        .filter(|s| s.id != keeper_id)
        .map(|s| s.id)
        .collect();
    // ordered drops here — no more immutable borrows of state.

    // Build the new shadow layer. Black ink, halftone-rendered.
    // The computed shadow density lives inside the ManualPaint
    // buffer so that the background worker returns the same mask
    // every time it reprocesses — if we just stashed it in the
    // LayerEntry preview, the next inspector tweak would trigger a
    // rerun that overwrote the shadow with a blank mask.
    let shadow_buf = inkplate::engine::layer::ManualPaintBuf {
        width: w,
        height: h,
        pixels: shadow_mask.as_raw().clone(),
    };
    let mut shadow_layer = Layer::new_spot(Rgb::BLACK);
    shadow_layer.name = format!("{keeper_name} shadow");
    shadow_layer.kind = LayerKind::Shadow;
    shadow_layer.extractor = Extractor::ManualPaint {
        buf: Some(shadow_buf),
    };
    shadow_layer.tone = Tone::default();
    shadow_layer.mask = MaskShape::default();
    shadow_layer.render_mode = RenderMode::Halftone;
    shadow_layer.halftone = HalftoneOverrides {
        lpi: 0,
        angle_deg: -1.0,
        dot_shape: Some(DotShape::Round),
        curve: HalftoneCurve::Linear,
    };
    // Print the shadow immediately after the keeper.
    shadow_layer.print_index = keeper_print_index + 1;

    let shadow_coverage = coverage_fraction(&shadow_mask);
    let mut shadow_entry = LayerEntry::new(shadow_layer);
    shadow_entry.preview = Some(shadow_mask.clone());
    shadow_entry.processed = Some(shadow_mask);
    shadow_entry.coverage = shadow_coverage;

    // Rewrite state.layers: keep the keeper (with its new union
    // mask baked into preview + processed), drop the other selected
    // entries, and insert the shadow layer right after the keeper.
    let keeper_abs_idx = state
        .layers
        .iter()
        .position(|e| e.layer.id == keeper_id)
        .unwrap();

    // Bake the union mask onto the keeper *before* we mutate the
    // list so the old preview doesn't linger.
    {
        let keeper = &mut state.layers[keeper_abs_idx];
        keeper.preview = Some(union_mask.clone());
        keeper.processed = Some(union_mask);
        keeper.coverage = coverage_fraction(keeper.preview.as_ref().unwrap());
        // Convert the keeper's extractor to ManualPaint so the
        // worker doesn't overwrite our freshly-baked union mask on
        // the next rerun. ManualPaint just returns whatever buffer
        // is in place, which after mask->buffer conversion becomes
        // our union.
        let width = keeper.preview.as_ref().unwrap().width();
        let height = keeper.preview.as_ref().unwrap().height();
        let pixels = keeper.preview.as_ref().unwrap().as_raw().clone();
        keeper.layer.extractor = Extractor::ManualPaint {
            buf: Some(inkplate::engine::layer::ManualPaintBuf {
                width,
                height,
                pixels,
            }),
        };
    }

    // Drop the other selected layers.
    state.layers.retain(|e| !drop_ids.contains(&e.layer.id));

    // Insert the shadow layer right after the keeper (which may have
    // shifted position after the retain).
    let insert_at = state
        .layers
        .iter()
        .position(|e| e.layer.id == keeper_id)
        .map(|i| i + 1)
        .unwrap_or(state.layers.len());
    state.layers.insert(insert_at, shadow_entry);

    // Reindex print order so the new list is clean.
    for (i, entry) in state.layers.iter_mut().enumerate() {
        entry.layer.print_index = i as u32;
    }

    state.composite_dirty = true;
    Some(ShadowMergeReport {
        merged_count,
        keeper_name,
    })
}

// ---------------------------------------------------------------------------
// Merge same-ink layers (for collapsing multiple shadow halftones
// after several "Merge → shadow HT" ops)
// ---------------------------------------------------------------------------

/// Result of [`merge_same_ink_layers`] — mostly for the status line.
pub struct SameInkMergeReport {
    pub merged_count: usize,
    pub keeper_name: String,
}

/// Collapse the multi-selection into a single layer by unioning
/// their density masks. Every selected layer must share the same
/// ink color, otherwise this is a no-op and returns `None`.
///
/// The keeper (first in current list order) is converted to
/// `ManualPaint` holding the unioned mask so the worker returns
/// the same bytes on every rerun. The other selected layers are
/// dropped. Render mode, halftone overrides, tone curve, and
/// mask-shape settings are inherited from the keeper.
///
/// Typical use: you've done `merge_to_shadow_halftone` three
/// times (reds, teals, yellows) and now have three separate
/// black halftone plates that could live on one screen.
pub fn merge_same_ink_layers(
    state: &mut GuiState,
    selection: &HashSet<Uuid>,
) -> Option<SameInkMergeReport> {
    if selection.len() < 2 {
        return None;
    }

    // Snapshot what we need up front so we can mutate state.layers
    // later without holding any borrows.
    struct SelSnap {
        id: Uuid,
        mask: GrayImage,
    }
    let snaps: Vec<(SelSnap, Rgb, String)> = state
        .layers
        .iter()
        .filter(|e| selection.contains(&e.layer.id))
        .filter_map(|e| {
            let mask = e.preview.as_ref()?.clone();
            Some((
                SelSnap {
                    id: e.layer.id,
                    mask,
                },
                e.layer.ink,
                e.layer.name.clone(),
            ))
        })
        .collect();
    if snaps.len() < 2 {
        return None;
    }

    // All selected layers must share the same ink — otherwise
    // unioning them would smash distinct screens together.
    let first_ink = snaps[0].1;
    if snaps.iter().any(|(_, ink, _)| *ink != first_ink) {
        return None;
    }

    // Dimensions must agree too.
    let (w, h) = snaps[0].0.mask.dimensions();
    if snaps.iter().any(|(s, _, _)| s.mask.dimensions() != (w, h)) {
        return None;
    }

    // Keeper is the first selected layer in the *current list
    // order*. We already filtered by list order above, so snaps[0]
    // is it.
    let keeper_id = snaps[0].0.id;
    let keeper_name = snaps[0].2.clone();

    // Union: per-pixel MIN of density. Density convention is
    // 0=ink / 255=no-ink, so the smallest value across all masks
    // is "the darkest any plate wanted to be here".
    let mut union_mask: GrayImage = ImageBuffer::from_pixel(w, h, Luma([255u8]));
    for y in 0..h {
        for x in 0..w {
            let mut min_d = 255u8;
            for (s, _, _) in &snaps {
                let d = s.mask.get_pixel(x, y)[0];
                if d < min_d {
                    min_d = d;
                }
            }
            union_mask.put_pixel(x, y, Luma([min_d]));
        }
    }
    let merged_count = snaps.len();
    let drop_ids: HashSet<Uuid> = snaps
        .iter()
        .filter(|(s, _, _)| s.id != keeper_id)
        .map(|(s, _, _)| s.id)
        .collect();

    // Bake the union onto the keeper via the ManualPaint buffer
    // so the worker thread returns the same bytes on re-runs.
    let keeper_abs_idx = state
        .layers
        .iter()
        .position(|e| e.layer.id == keeper_id)?;
    {
        let keeper = &mut state.layers[keeper_abs_idx];
        let pixels = union_mask.as_raw().clone();
        let coverage = coverage_fraction(&union_mask);
        keeper.preview = Some(union_mask.clone());
        keeper.processed = Some(union_mask);
        keeper.coverage = coverage;
        keeper.layer.extractor = Extractor::ManualPaint {
            buf: Some(inkplate::engine::layer::ManualPaintBuf {
                width: w,
                height: h,
                pixels,
            }),
        };
    }

    state.layers.retain(|e| !drop_ids.contains(&e.layer.id));

    // Reindex print order now that the list is shorter.
    for (i, entry) in state.layers.iter_mut().enumerate() {
        entry.layer.print_index = i as u32;
    }

    state.composite_dirty = true;
    Some(SameInkMergeReport {
        merged_count,
        keeper_name,
    })
}
