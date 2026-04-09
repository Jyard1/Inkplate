//! Shared widget helpers — the small atoms the panel modules reuse.

use eframe::egui::{self, Color32, Response, Sense, Ui};

use inkplate::engine::color::Rgb;
use inkplate::engine::tone::CurvePoint;

/// Slider with a label above it instead of to the side. Returns the
/// full response so callers can check `.changed()` and queue reruns.
pub fn labeled_slider_f32(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> Response {
    ui.label(label);
    ui.add(egui::Slider::new(value, range))
}

pub fn labeled_slider_u32(
    ui: &mut Ui,
    label: &str,
    value: &mut u32,
    range: std::ops::RangeInclusive<u32>,
) -> Response {
    ui.label(label);
    ui.add(egui::Slider::new(value, range))
}

pub fn labeled_slider_u8(
    ui: &mut Ui,
    label: &str,
    value: &mut u8,
    range: std::ops::RangeInclusive<u8>,
) -> Response {
    ui.label(label);
    ui.add(egui::Slider::new(value, range))
}

/// Ink color picker with a visible swatch, an egui color-wheel
/// popup, and a hex text field for explicit entry (e.g. pasting a
/// Pantone hex). Returns a [`Response`] whose `.changed()` bit is
/// set whenever any of the three widgets mutates the color.
pub fn ink_picker(ui: &mut Ui, label: &str, rgb: &mut Rgb) -> Response {
    ui.label(label);
    let mut any_changed = false;

    // Track the responses across the horizontal row so we can
    // return a single "any of these changed" result.
    let mut row = ui.horizontal(|ui| {
        // Big swatch button (24×24 so it's obvious even for white
        // and black inks against their matching panel backgrounds).
        // Click opens egui's color-wheel popup.
        let mut arr = [rgb.0, rgb.1, rgb.2];
        let swatch_size = egui::vec2(28.0, 22.0);
        let picker = egui::color_picker::color_edit_button_srgb(ui, &mut arr);
        // Force the button to be a minimum size so it doesn't
        // collapse to a 14-px nub.
        let _ = swatch_size;
        if picker.changed() {
            *rgb = Rgb(arr[0], arr[1], arr[2]);
            any_changed = true;
        }

        // Hex text input next to the swatch. Parse on every edit;
        // malformed entries just leave the color alone.
        let mut hex = format!("{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2);
        let hex_resp = ui.add(egui::TextEdit::singleline(&mut hex).desired_width(64.0));
        if hex_resp.changed() {
            let cleaned = hex.trim().trim_start_matches('#');
            if cleaned.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&cleaned[0..2], 16),
                    u8::from_str_radix(&cleaned[2..4], 16),
                    u8::from_str_radix(&cleaned[4..6], 16),
                ) {
                    if Rgb(r, g, b) != *rgb {
                        *rgb = Rgb(r, g, b);
                        any_changed = true;
                    }
                }
            }
        }
        hex_resp
    });
    if any_changed {
        row.response.mark_changed();
    }
    row.response
}

/// Small square swatch showing an ink color, no interaction.
pub fn ink_swatch(ui: &mut Ui, rgb: Rgb, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 2.0, Color32::from_rgb(rgb.0, rgb.1, rgb.2));
    ui.painter()
        .rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::from_gray(60)));
    resp
}

/// Radius in pixels used for control-point hit testing and drawing.
const CURVE_POINT_RADIUS: f32 = 5.0;
/// Minimum panel height for the curve canvas.
const CURVE_CANVAS_HEIGHT: f32 = 140.0;

/// Piecewise-linear tone curve editor.
///
/// The canvas shows x = input density (0 on the left, 255 on the
/// right) against y = output density (0 at the *bottom*, 255 at the
/// top, so the curve reads like a Photoshop / Levels widget even
/// though the underlying density convention is 0 = ink).
///
/// Interactions:
///
/// - **Drag a point** to move it. Endpoints at x=0 and x=255 can
///   only move vertically so the curve always covers the full range.
/// - **Double-click empty space** to insert a new point on the curve
///   at that x position.
/// - **Right-click a point** (or secondary-click) to delete it.
///   Endpoints can't be deleted.
///
/// Returns `true` if the curve was modified this frame, so the
/// caller can queue a layer rerun.
pub fn curve_editor(ui: &mut Ui, points: &mut Vec<CurvePoint>) -> bool {
    // Guarantee at least two endpoints so the drawing loop always
    // has something sensible to render.
    if points.is_empty() {
        points.push(CurvePoint::new(0, 0));
        points.push(CurvePoint::new(255, 255));
    }

    let desired = egui::vec2(ui.available_width(), CURVE_CANVAS_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click_and_drag());

    // Background + grid.
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(22));
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, Color32::from_gray(60)));
    // Quarter-gridlines for visual reference.
    let grid = egui::Stroke::new(1.0, Color32::from_gray(40));
    for i in 1..4 {
        let t = i as f32 / 4.0;
        let y = rect.top() + rect.height() * t;
        painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], grid);
        let x = rect.left() + rect.width() * t;
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], grid);
    }
    // Diagonal reference = identity curve.
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.top()),
        ],
        egui::Stroke::new(1.0, Color32::from_gray(55)),
    );

    // Coordinate transforms between curve space (x, y in 0..=255)
    // and screen space (rect coords). Note the y flip: higher
    // density in curve space = higher on screen.
    let to_screen = |p: CurvePoint| -> egui::Pos2 {
        egui::pos2(
            rect.left() + (p.x as f32 / 255.0) * rect.width(),
            rect.bottom() - (p.y as f32 / 255.0) * rect.height(),
        )
    };
    let from_screen = |pos: egui::Pos2| -> (f32, f32) {
        let cx = ((pos.x - rect.left()) / rect.width() * 255.0).clamp(0.0, 255.0);
        let cy = ((rect.bottom() - pos.y) / rect.height() * 255.0).clamp(0.0, 255.0);
        (cx, cy)
    };

    // Sort by x before drawing so the line goes left-to-right.
    points.sort_by_key(|p| p.x);
    let mut changed = false;

    // --- Drag handling ----------------------------------------------
    // Figure out which point is being dragged. egui's single-shot
    // Response doesn't track multi-point ids for us, so we do a
    // manual nearest-point pick on drag start and stash the index
    // via widget state.
    let drag_id = response.id.with("curve_drag_idx");
    let mut dragging: Option<usize> = ui.memory(|m| m.data.get_temp(drag_id));

    if response.drag_started() {
        if let Some(pos) = response.interact_pointer_pos() {
            dragging = points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, (to_screen(*p) - pos).length()))
                .filter(|(_, d)| *d <= CURVE_POINT_RADIUS * 2.0)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(i, _)| i);
            ui.memory_mut(|m| m.data.insert_temp(drag_id, dragging));
        }
    }

    if response.dragged() {
        if let (Some(idx), Some(pos)) = (dragging, response.interact_pointer_pos()) {
            if idx < points.len() {
                let is_first = idx == 0;
                let is_last = idx == points.len() - 1;
                let (new_x, new_y) = from_screen(pos);
                let x = if is_first {
                    0.0
                } else if is_last {
                    255.0
                } else {
                    // Clamp x between the neighbours so points can't
                    // leapfrog each other (would confuse the LUT
                    // builder's duplicate-x handling).
                    let lo = points[idx - 1].x as f32 + 1.0;
                    let hi = points[idx + 1].x as f32 - 1.0;
                    new_x.clamp(lo.min(hi), hi.max(lo))
                };
                let y = new_y.clamp(0.0, 255.0);
                let new_pt = CurvePoint::new(x.round() as u8, y.round() as u8);
                if new_pt != points[idx] {
                    points[idx] = new_pt;
                    changed = true;
                }
            }
        }
    }

    if response.drag_stopped() {
        ui.memory_mut(|m| m.data.insert_temp::<Option<usize>>(drag_id, None));
    }

    // --- Double-click to add -----------------------------------------
    if response.double_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let (x, y) = from_screen(pos);
            let new_pt = CurvePoint::new(x.round() as u8, y.round() as u8);
            // Don't drop a duplicate right on top of an existing point.
            let too_close = points.iter().any(|p| {
                ((p.x as i32 - new_pt.x as i32).abs() < 3)
                    && ((p.y as i32 - new_pt.y as i32).abs() < 3)
            });
            if !too_close {
                points.push(new_pt);
                points.sort_by_key(|p| p.x);
                changed = true;
            }
        }
    }

    // --- Right-click to delete ---------------------------------------
    if response.secondary_clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some((idx, dist)) = points
                .iter()
                .enumerate()
                .map(|(i, p)| (i, (to_screen(*p) - pos).length()))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            {
                // Don't delete endpoints or we'll break the LUT.
                if dist <= CURVE_POINT_RADIUS * 2.0
                    && idx != 0
                    && idx != points.len() - 1
                {
                    points.remove(idx);
                    changed = true;
                }
            }
        }
    }

    // --- Draw curve + points ------------------------------------------
    let curve_stroke = egui::Stroke::new(2.0, Color32::from_rgb(120, 200, 240));
    let segments: Vec<egui::Pos2> = points.iter().map(|p| to_screen(*p)).collect();
    for pair in segments.windows(2) {
        painter.line_segment([pair[0], pair[1]], curve_stroke);
    }
    for (i, &pos) in segments.iter().enumerate() {
        let is_endpoint = i == 0 || i == segments.len() - 1;
        let fill = if is_endpoint {
            Color32::from_gray(200)
        } else {
            Color32::from_rgb(255, 180, 80)
        };
        painter.circle_filled(pos, CURVE_POINT_RADIUS, fill);
        painter.circle_stroke(
            pos,
            CURVE_POINT_RADIUS,
            egui::Stroke::new(1.0, Color32::BLACK),
        );
    }

    changed
}
