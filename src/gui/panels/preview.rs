//! Center preview panel. Shows the composite (visible layers blended
//! over the shirt), the raw source, or one layer in isolation.
//!
//! # Viewport
//!
//! State lives on [`Viewport`](crate::gui::state::Viewport). Two
//! modes: "fit" scales the image to fill the panel with padding, and
//! free zoom/pan uses `zoom` (scale multiplier where 1.0 = one
//! device pixel per source pixel) and `pan` (screen-pixel offset
//! from panel centre) to draw wherever the user has dragged.
//!
//! Controls:
//!
//! - **Mouse wheel** on the preview: zoom in/out around the cursor.
//! - **Middle-drag** (or **right-drag**) on the preview: pan.
//! - **F**: refit (reset zoom/pan).
//! - **1**: 100% zoom (one device pixel per source pixel).

use eframe::egui::{self, Sense, Ui};
use inkplate::engine::layer::{Extractor, ManualPaintBuf};

use crate::gui::state::{GuiState, PreviewMode};
use crate::gui::textures::TextureCache;

/// Returns `true` if the user painted a manual-paint stroke this
/// frame. The caller should treat that as a layer edit (snapshot +
/// rerun) exactly like an inspector slider change.
pub fn show(
    ui: &mut Ui,
    state: &mut GuiState,
    textures: &mut TextureCache,
) -> bool {
    ui.horizontal(|ui| {
        ui.heading("Preview");
        ui.separator();

        let mut mode = state.preview_mode;
        ui.selectable_value(&mut mode, PreviewMode::Composite, "Composite");
        ui.selectable_value(&mut mode, PreviewMode::Source, "Source");
        if let Some(idx) = state.selected {
            ui.selectable_value(&mut mode, PreviewMode::Layer(idx), "Layer");
        }
        if mode != state.preview_mode {
            state.preview_mode = mode;
            textures.invalidate();
        }

        ui.separator();

        if ui
            .button("Fit")
            .on_hover_text("Refit the image to the panel (shortcut: F)")
            .clicked()
        {
            state.viewport.fit = true;
        }
        if ui
            .button("100%")
            .on_hover_text("One device pixel per source pixel (shortcut: 1)")
            .clicked()
        {
            state.viewport.fit = false;
            state.viewport.zoom = 1.0;
            state.viewport.pan = egui::Vec2::ZERO;
        }
        if !state.viewport.fit {
            ui.label(format!("{:>5.0}%", state.viewport.zoom * 100.0));
        }

        // Brush bar — only visible when we're actually looking at
        // a manual-paint layer, so it doesn't clutter the preview
        // toolbar the rest of the time.
        if is_manual_paint_layer_selected(state) {
            ui.separator();
            ui.label("Brush:");
            ui.add(egui::Slider::new(&mut state.brush.radius, 1..=256).logarithmic(true));
            ui.selectable_value(&mut state.brush.paint_mode, true, "Paint");
            ui.selectable_value(&mut state.brush.paint_mode, false, "Erase");
        }
    });
    ui.separator();

    let mut paint_stroke_applied = false;

    if state.source.is_none() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("Open an image to begin")
                    .italics()
                    .size(18.0),
            );
        });
        return false;
    }

    // Consume the viewport area and allocate an interactive rect
    // that responds to drag (for panning) and click (for focus).
    let avail = ui.available_size();
    let (viewport_rect, response) =
        ui.allocate_exact_size(avail, Sense::click_and_drag());

    // Handle keyboard shortcuts globally for this panel.
    ui.input(|i| {
        if i.key_pressed(egui::Key::F) {
            state.viewport.fit = true;
        }
        if i.key_pressed(egui::Key::Num1) {
            state.viewport.fit = false;
            state.viewport.zoom = 1.0;
            state.viewport.pan = egui::Vec2::ZERO;
        }
    });

    let Some(texture) = textures.current(ui.ctx(), state) else {
        return false;
    };
    let tex_id = texture.id();
    let img_size = texture.size_vec2();
    if img_size.x <= 0.0 || img_size.y <= 0.0 {
        return false;
    }

    // Compute the fit scale once so we can use it as both the
    // display scale in fit mode and the "reset" zoom when the user
    // switches to free-zoom mode by scrolling.
    let fit_scale = (viewport_rect.width() / img_size.x)
        .min(viewport_rect.height() / img_size.y)
        .clamp(0.01, 8.0);

    // Mouse wheel → zoom around the hovered source pixel.
    if response.hovered() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta.abs() > 0.1 {
            if state.viewport.fit {
                // First scroll in fit mode: snap to current fit
                // scale so the next zoom increment is relative to
                // what the user is already looking at.
                state.viewport.fit = false;
                state.viewport.zoom = fit_scale;
                state.viewport.pan = egui::Vec2::ZERO;
            }
            // Exponential zoom — one notch ≈ 10%.
            let factor = (scroll_delta * 0.0015).exp();
            let old_zoom = state.viewport.zoom;
            let new_zoom = (old_zoom * factor).clamp(0.05, 32.0);

            // Zoom around the cursor so the pixel under the mouse
            // stays put. Convert cursor → image space at old zoom,
            // then adjust pan so the same image point is under the
            // same screen point at new zoom.
            if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
                let center = viewport_rect.center();
                let rel = cursor - center - state.viewport.pan;
                // rel / old_zoom = image point (in source pixels)
                // at new_zoom it should land at rel again, i.e.:
                //   rel_new = rel * (new_zoom / old_zoom)
                // and pan moves to compensate:
                let scale_change = new_zoom / old_zoom;
                state.viewport.pan += rel - rel * scale_change;
            }
            state.viewport.zoom = new_zoom;
        }
    }

    // Middle- or secondary-drag → pan.
    if response.dragged_by(egui::PointerButton::Middle)
        || response.dragged_by(egui::PointerButton::Secondary)
    {
        state.viewport.fit = false;
        state.viewport.pan += response.drag_delta();
    }

    // Decide the on-screen rect for the image.
    let display_scale = if state.viewport.fit {
        fit_scale
    } else {
        state.viewport.zoom
    };
    let display_size = img_size * display_scale;
    let display_center = viewport_rect.center() + state.viewport.pan;
    let display_rect =
        egui::Rect::from_center_size(display_center, display_size);

    // Draw a dark gutter behind the image so letterboxed regions
    // don't inherit the panel background.
    let painter = ui.painter_at(viewport_rect);
    painter.rect_filled(
        viewport_rect,
        egui::Rounding::ZERO,
        egui::Color32::from_gray(18),
    );
    painter.image(
        tex_id,
        display_rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );

    // --- Manual paint: primary-button drag inside a manual_paint
    // layer stamps brush dabs into the layer's stroke buffer. We
    // screen-space → source-space transform via display_rect. ---
    if is_manual_paint_layer_selected(state)
        && matches!(state.preview_mode, PreviewMode::Layer(_))
        && (response.dragged_by(egui::PointerButton::Primary)
            || response.clicked_by(egui::PointerButton::Primary))
    {
        if let Some(cursor) = response.interact_pointer_pos() {
            if display_rect.contains(cursor) {
                // Convert cursor (screen) → source pixel coords.
                let rel_x = (cursor.x - display_rect.left()) / display_rect.width();
                let rel_y = (cursor.y - display_rect.top()) / display_rect.height();
                let src_x = (rel_x * img_size.x).round() as i32;
                let src_y = (rel_y * img_size.y).round() as i32;
                if stamp_brush_dab(state, src_x, src_y) {
                    paint_stroke_applied = true;
                    textures.invalidate();
                }
            }
        }
    }

    // Brush cursor ring (only when hovering a paintable layer).
    if is_manual_paint_layer_selected(state) && response.hovered() {
        if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
            if display_rect.contains(cursor) {
                let screen_r = state.brush.radius as f32 * display_scale;
                let ring_color = if state.brush.paint_mode {
                    egui::Color32::from_rgb(80, 200, 120)
                } else {
                    egui::Color32::from_rgb(220, 100, 100)
                };
                painter.circle_stroke(
                    cursor,
                    screen_r.max(1.0),
                    egui::Stroke::new(1.5, ring_color),
                );
            }
        }
    }

    paint_stroke_applied
}

/// True iff the selected layer's extractor is `ManualPaint`. Used to
/// gate the brush bar + paint input.
fn is_manual_paint_layer_selected(state: &GuiState) -> bool {
    let Some(idx) = state.selected else {
        return false;
    };
    let Some(entry) = state.layers.get(idx) else {
        return false;
    };
    matches!(entry.layer.extractor, Extractor::ManualPaint { .. })
}

/// Paint a brush dab into the selected manual-paint layer's stroke
/// buffer, centred on the given source-pixel coordinate. Allocates
/// the buffer if it doesn't exist yet (sized to match the source).
/// Returns `true` if any pixel changed.
fn stamp_brush_dab(state: &mut GuiState, cx: i32, cy: i32) -> bool {
    let Some(source) = state.source.clone() else {
        return false;
    };
    let (sw, sh) = source.dimensions();
    let Some(idx) = state.selected else {
        return false;
    };
    let Some(entry) = state.layers.get_mut(idx) else {
        return false;
    };
    let Extractor::ManualPaint { buf } = &mut entry.layer.extractor else {
        return false;
    };

    // Allocate on first stroke. Source-sized so round-tripping
    // through the extractor doesn't resample.
    if buf.as_ref().map(|b| (b.width, b.height)) != Some((sw, sh)) {
        *buf = Some(ManualPaintBuf::blank(sw, sh));
    }
    let Some(buf) = buf.as_mut() else {
        return false;
    };

    let r = state.brush.radius as i32;
    let ink_value: u8 = if state.brush.paint_mode { 0 } else { 255 };
    let r2 = r * r;
    let w = buf.width as i32;
    let h = buf.height as i32;
    let x0 = (cx - r).max(0);
    let x1 = (cx + r + 1).min(w);
    let y0 = (cy - r).max(0);
    let y1 = (cy + r + 1).min(h);
    let mut changed = false;
    for y in y0..y1 {
        let dy = y - cy;
        let dy2 = dy * dy;
        let row = (y as usize) * (buf.width as usize);
        for x in x0..x1 {
            let dx = x - cx;
            if dx * dx + dy2 <= r2 {
                let i = row + x as usize;
                if buf.pixels[i] != ink_value {
                    buf.pixels[i] = ink_value;
                    changed = true;
                }
            }
        }
    }
    changed
}
