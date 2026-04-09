//! Layer list panel — one row per layer with select/visibility/delete.
//!
//! Rows are drawn in a scrollable column. Plain click on a name
//! selects the layer and populates the inspector; **Ctrl+click**
//! toggles it into the multi-selection (shown with a blue outline),
//! which is what the "Merge → shadow halftone" button operates on.
//! The visibility checkbox toggles the layer without rerunning
//! extraction — the composite just rebuilds without that layer's
//! contribution.

use eframe::egui::{self, Ui};

use crate::gui::state::GuiState;
use crate::gui::widgets::ink_swatch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    VisibilityChanged,
    SelectionChanged,
    DeleteSelected,
    MoveUp,
    MoveDown,
    /// Collapse the multi-selection onto the lightest-ink layer and
    /// emit a new black shadow-halftone plate. See
    /// `processing::merge_to_shadow_halftone`.
    MergeShadowHalftone,
    /// Union the multi-selection's density masks into a single
    /// layer. Requires all selected layers to share the same ink
    /// color. See `processing::merge_same_ink_layers`.
    MergeSameInk,
}

pub fn show(ui: &mut Ui, state: &mut GuiState) -> Action {
    let mut action = Action::None;

    ui.heading("Layers");
    ui.horizontal(|ui| {
        if ui.button("Move up").clicked() {
            action = Action::MoveUp;
        }
        if ui.button("Move down").clicked() {
            action = Action::MoveDown;
        }
        if ui.button("Delete").clicked() {
            action = Action::DeleteSelected;
        }
    });

    // Multi-select toolbar — only appears when the user has ticked
    // at least two layers via Ctrl+click.
    let multi_count = state.multi_select.len();
    if multi_count >= 2 {
        // "Merge same ink" is only meaningful when every selected
        // layer shares an ink — detect that up front so we can
        // enable/disable the button instead of surprising the user
        // with a silent no-op.
        let same_ink = {
            let mut iter = state
                .layers
                .iter()
                .filter(|e| state.multi_select.contains(&e.layer.id))
                .map(|e| e.layer.ink);
            match iter.next() {
                Some(first) => iter.all(|ink| ink == first),
                None => false,
            }
        };

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{multi_count} layers in multi-selection"))
                    .size(11.0)
                    .italics(),
            );
            if ui
                .button("Merge → shadow HT")
                .on_hover_text(
                    "Collapse the multi-selected layers onto the lightest-ink plate \
                     and generate a black halftone shadow plate whose density \
                     reproduces the darker shades from just two inks.",
                )
                .clicked()
            {
                action = Action::MergeShadowHalftone;
            }
            let merge_btn = ui.add_enabled(
                same_ink,
                egui::Button::new("Merge same ink"),
            );
            let merge_btn = merge_btn.on_hover_text(if same_ink {
                "Union the selected masks into a single layer. \
                 All selected layers share the same ink color so \
                 they can live on one screen on the press."
            } else {
                "All selected layers must share the same ink color. \
                 Deselect the mismatched ones first."
            });
            if merge_btn.clicked() {
                action = Action::MergeSameInk;
            }
            if ui.button("Clear selection").clicked() {
                state.multi_select.clear();
            }
        });
    }

    ui.separator();

    let ctrl_held = ui.input(|i| i.modifiers.command);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for i in 0..state.layers.len() {
                let is_selected = state.selected == Some(i);
                let layer_id = state.layers[i].layer.id;
                let is_multi = state.multi_select.contains(&layer_id);
                let response = ui
                    .push_id(i, |ui| draw_row(ui, state, i, is_selected, is_multi))
                    .inner;
                if response.selected_clicked {
                    if ctrl_held {
                        // Ctrl+click → toggle in the multi-selection.
                        // Don't touch the primary selection so the
                        // inspector keeps showing whatever the user
                        // was looking at.
                        if state.multi_select.contains(&layer_id) {
                            state.multi_select.remove(&layer_id);
                        } else {
                            state.multi_select.insert(layer_id);
                        }
                    } else {
                        state.selected = Some(i);
                        // Plain click clears the multi-select so the
                        // two selection modes don't get confused.
                        state.multi_select.clear();
                        action = Action::SelectionChanged;
                    }
                }
                if response.visibility_toggled {
                    action = Action::VisibilityChanged;
                }
            }
        });

    if state.layers.is_empty() {
        ui.add_space(12.0);
        ui.label(egui::RichText::new("No layers. Open an image and click Process.").italics());
    }

    action
}

struct RowResponse {
    selected_clicked: bool,
    visibility_toggled: bool,
}

fn draw_row(
    ui: &mut Ui,
    state: &mut GuiState,
    i: usize,
    is_selected: bool,
    is_multi: bool,
) -> RowResponse {
    let mut resp = RowResponse {
        selected_clicked: false,
        visibility_toggled: false,
    };

    let base_fill = if is_selected {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    };
    let frame = egui::Frame::group(ui.style()).fill(base_fill).stroke(
        if is_multi {
            egui::Stroke::new(2.0, egui::Color32::from_rgb(120, 180, 255))
        } else {
            egui::Stroke::NONE
        },
    );

    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            let entry = &mut state.layers[i];

            // Visibility checkbox.
            let mut vis = entry.layer.visible;
            if ui.checkbox(&mut vis, "").changed() {
                entry.layer.visible = vis;
                resp.visibility_toggled = true;
            }

            ink_swatch(ui, entry.layer.ink, 18.0);

            // Clicking the name region selects the layer.
            let name_resp = ui.add(
                egui::Label::new(format!(
                    "{:02}  {}",
                    entry.layer.print_index + 1,
                    entry.layer.name
                ))
                .sense(egui::Sense::click()),
            );
            if name_resp.clicked() {
                resp.selected_clicked = true;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{:>5.1}%", entry.coverage * 100.0));
            });
        });
    });

    resp
}
