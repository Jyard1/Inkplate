//! Layer list panel — one row per layer with select/visibility/delete.
//!
//! Rows are drawn in a scrollable column. Clicking a row selects the
//! layer (highlighting it and populating the inspector). The
//! visibility checkbox toggles the layer without rerunning extraction
//! — the composite just rebuilds without that layer's contribution.

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
}

pub fn show(ui: &mut Ui, state: &mut GuiState) -> Action {
    let mut action = Action::None;

    ui.heading("Layers");
    ui.horizontal(|ui| {
        if ui.button("↑").clicked() {
            action = Action::MoveUp;
        }
        if ui.button("↓").clicked() {
            action = Action::MoveDown;
        }
        if ui.button("Delete").clicked() {
            action = Action::DeleteSelected;
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for i in 0..state.layers.len() {
                let is_selected = state.selected == Some(i);
                let response = ui
                    .push_id(i, |ui| draw_row(ui, state, i, is_selected))
                    .inner;
                if response.selected_clicked {
                    state.selected = Some(i);
                    action = Action::SelectionChanged;
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

fn draw_row(ui: &mut Ui, state: &mut GuiState, i: usize, is_selected: bool) -> RowResponse {
    let mut resp = RowResponse {
        selected_clicked: false,
        visibility_toggled: false,
    };

    let frame = egui::Frame::group(ui.style()).fill(if is_selected {
        ui.visuals().selection.bg_fill
    } else {
        ui.visuals().faint_bg_color
    });

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
