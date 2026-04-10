//! Top bar: file open, workflow picker, process button, job options
//! (DPI / LPI / angle), shirt color picker.
//!
//! Emits an `Action` enum so the app loop can decide whether to rerun
//! one layer, the whole workflow, or nothing — panels don't call
//! processing functions directly.

use eframe::egui::{self, Ui};

use crate::gui::state::GuiState;
use crate::gui::widgets::{ink_picker, labeled_slider_f32, labeled_slider_u32};
use inkplate::engine::workflows::Workflow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    OpenImage,
    RunWorkflow,
    JobChanged,
    SaveProject,
    OpenProject,
    ExportFilms,
    ExportContactSheet,
    BackgroundRemovalChanged,
}

pub fn show(ui: &mut Ui, state: &mut GuiState) -> Action {
    let mut action = Action::None;

    ui.horizontal_wrapped(|ui| {
        if ui.button("Open image…").clicked() {
            action = Action::OpenImage;
        }
        if ui.button("Open project…").clicked() {
            action = Action::OpenProject;
        }
        if ui.button("Save project…").clicked() {
            action = Action::SaveProject;
        }

        ui.separator();
        ui.label("Workflow:");
        let mut workflow = state.workflow;
        egui::ComboBox::from_id_salt("workflow_combo")
            .selected_text(workflow.label())
            .show_ui(ui, |ui| {
                for &w in Workflow::all() {
                    ui.selectable_value(&mut workflow, w, w.label());
                }
            });
        if workflow != state.workflow {
            state.workflow = workflow;
            action = Action::RunWorkflow;
        }

        if ui.button("Process").clicked() {
            action = Action::RunWorkflow;
        }

        ui.separator();
        if ui.button("Export films…").clicked() {
            action = Action::ExportFilms;
        }
        if ui.button("Contact sheet…").clicked() {
            action = Action::ExportContactSheet;
        }

        ui.separator();
        ink_picker(ui, "Shirt:", &mut state.shirt_color);
        if let Some(path) = &state.source_path {
            ui.separator();
            ui.label(
                path.file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
    });

    ui.separator();

    ui.horizontal_wrapped(|ui| {
        ui.vertical(|ui| {
            ui.set_width(140.0);
            let prev = state.job.dpi;
            labeled_slider_u32(ui, "DPI", &mut state.job.dpi, 72..=1200);
            if state.job.dpi != prev {
                action = Action::JobChanged;
            }
        });
        ui.vertical(|ui| {
            ui.set_width(140.0);
            let prev = state.job.default_lpi;
            labeled_slider_f32(ui, "LPI", &mut state.job.default_lpi, 20.0..=300.0);
            if (state.job.default_lpi - prev).abs() > 1e-4 {
                action = Action::JobChanged;
            }
        });
        ui.vertical(|ui| {
            ui.set_width(140.0);
            let prev = state.job.default_angle_deg;
            labeled_slider_f32(
                ui,
                "Default angle°",
                &mut state.job.default_angle_deg,
                0.0..=180.0,
            );
            if (state.job.default_angle_deg - prev).abs() > 1e-4 {
                action = Action::JobChanged;
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(160.0);
            let prev = state.workflow_opts.max_colors as u32;
            let mut mc = prev;
            labeled_slider_u32(ui, "Max palette colors", &mut mc, 2..=48);
            if mc != prev {
                state.workflow_opts.max_colors = mc as usize;
            }
            ui.checkbox(&mut state.workflow_opts.consolidate_hues, "Merge same hue")
                .on_hover_text(
                    "Collapse shades of the same hue (e.g. bright red + blood red) \
                     onto a single plate. Off by default — leave this on only if you \
                     actually want shadow/highlight variants merged into one ink.",
                );
        });
        ui.vertical(|ui| {
            ui.set_width(160.0);
            labeled_slider_f32(
                ui,
                "Color Range fuzziness",
                &mut state.workflow_opts.fuzziness,
                1.0..=200.0,
            );
            // GCR slider — only relevant for CMYK process workflows.
            if matches!(
                state.workflow,
                inkplate::engine::workflows::Workflow::CmykProcessLight
                    | inkplate::engine::workflows::Workflow::CmykProcessDark
            ) {
                labeled_slider_f32(
                    ui,
                    "GCR strength",
                    &mut state.workflow_opts.gcr_strength,
                    0.0..=1.0,
                );
            }
        });

        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(200.0);
            let prev_enabled = state.bg_removal.enabled;
            let mut enabled = prev_enabled;
            if ui.checkbox(&mut enabled, "Remove background").changed() {
                state.bg_removal.enabled = enabled;
                action = Action::BackgroundRemovalChanged;
            }
            let tol_resp = ui.add_enabled(
                enabled,
                egui::Slider::new(&mut state.bg_removal.tolerance, 1.0..=60.0).text("BG tolerance"),
            );
            if tol_resp.changed() {
                action = Action::BackgroundRemovalChanged;
            }
            if state.source_alpha.is_some() {
                ui.label(
                    egui::RichText::new("source has alpha — using it directly")
                        .italics()
                        .size(10.0),
                );
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.set_width(180.0);
            let prev_threshold = state.clamp_black_threshold;
            let mut val = state.clamp_black_threshold as u32;
            let on = val > 0;
            let mut toggled_on = on;
            if ui
                .checkbox(&mut toggled_on, "Clamp near-black")
                .on_hover_text(
                    "Force near-black source pixels to pure (0,0,0) before \
                     extraction. Prevents color channels from picking up \
                     noise in dark areas.",
                )
                .changed()
            {
                if toggled_on && val == 0 {
                    val = 50;
                } else if !toggled_on {
                    val = 0;
                }
            }
            let resp = ui.add_enabled(
                toggled_on,
                egui::Slider::new(&mut val, 1..=100).text("threshold"),
            );
            if resp.changed() && val == 0 {
                val = 1; // slider min is 1 when enabled
            }
            state.clamp_black_threshold = val as u8;
            if state.clamp_black_threshold != prev_threshold {
                action = Action::JobChanged;
            }
        });
    });

    action
}
