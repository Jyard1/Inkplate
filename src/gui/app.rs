//! Main `eframe::App` implementation. Holds `GuiState` + `TextureCache`,
//! lays out the four panels, and routes panel actions back into the
//! processing helpers.

use std::sync::Arc;

use eframe::egui;
use inkplate::engine::layer::Layer;
use inkplate::export::{export_all, BorderOpts, ExportOpts, RegMarkOpts};
use inkplate::project::{Project, CURRENT_VERSION};

use super::panels::{global, inspector, layers, preview};
use super::processing::{
    auto_rerun, rebuild_foreground_mask, rerun_all_layers, rerun_layer, rerun_workflow,
};
use super::state::GuiState;
use super::textures::TextureCache;

#[derive(Default)]
pub struct InkplateApp {
    state: GuiState,
    textures: TextureCache,
}

impl eframe::App for InkplateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---- top bar ----
        egui::TopBottomPanel::top("top_bar")
            .resizable(false)
            .show(ctx, |ui| {
                let action = global::show(ui, &mut self.state);
                self.handle_global_action(action);
            });

        // ---- status bar ----
        egui::TopBottomPanel::bottom("status_bar")
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&self.state.status).small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{} layers", self.state.layers.len()))
                                .small(),
                        );
                    });
                });
            });

        // ---- left: layer list ----
        egui::SidePanel::left("layers_panel")
            .resizable(true)
            .default_width(280.0)
            .min_width(220.0)
            .show(ctx, |ui| {
                let action = layers::show(ui, &mut self.state);
                self.handle_layers_action(action);
            });

        // ---- right: inspector ----
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(320.0)
            .min_width(260.0)
            .show(ctx, |ui| {
                if inspector::show(ui, &mut self.state) {
                    if let Some(idx) = self.state.selected {
                        rerun_layer(&mut self.state, idx);
                        self.textures.invalidate();
                    }
                }
            });

        // ---- center: preview ----
        egui::CentralPanel::default().show(ctx, |ui| {
            preview::show(ui, &mut self.state, &mut self.textures);
        });
    }
}

impl InkplateApp {
    fn handle_global_action(&mut self, action: global::Action) {
        match action {
            global::Action::None => {}
            global::Action::OpenImage => self.open_image_dialog(),
            global::Action::RunWorkflow => {
                rerun_workflow(&mut self.state);
                self.textures.invalidate();
            }
            global::Action::JobChanged => {
                rerun_all_layers(&mut self.state);
                self.textures.invalidate();
            }
            global::Action::SaveProject => self.save_project_dialog(),
            global::Action::OpenProject => self.open_project_dialog(),
            global::Action::ExportFilms => self.export_films_dialog(),
            global::Action::ExportContactSheet => self.export_contact_sheet_dialog(),
            global::Action::BackgroundRemovalChanged => {
                rebuild_foreground_mask(&mut self.state);
                rerun_all_layers(&mut self.state);
                self.textures.invalidate();
                self.state.status = if self.state.bg_removal.enabled {
                    format!(
                        "background removal on (tolerance {:.1})",
                        self.state.bg_removal.tolerance
                    )
                } else {
                    "background removal off".into()
                };
            }
        }
    }

    fn handle_layers_action(&mut self, action: layers::Action) {
        match action {
            layers::Action::None => {}
            layers::Action::VisibilityChanged => {
                self.state.composite_dirty = true;
                self.textures.invalidate();
            }
            layers::Action::SelectionChanged => {
                // No rerun needed — just affects inspector population
                // and preview-mode options.
            }
            layers::Action::DeleteSelected => {
                if let Some(idx) = self.state.selected {
                    if idx < self.state.layers.len() {
                        self.state.layers.remove(idx);
                        self.state.selected = if self.state.layers.is_empty() {
                            None
                        } else {
                            Some(idx.saturating_sub(1).min(self.state.layers.len() - 1))
                        };
                        self.state.composite_dirty = true;
                        self.textures.invalidate();
                    }
                }
            }
            layers::Action::MoveUp => {
                if let Some(idx) = self.state.selected {
                    if idx > 0 {
                        self.state.layers.swap(idx - 1, idx);
                        self.state.selected = Some(idx - 1);
                        reindex_print_order(&mut self.state);
                        self.state.composite_dirty = true;
                        self.textures.invalidate();
                    }
                }
            }
            layers::Action::MoveDown => {
                if let Some(idx) = self.state.selected {
                    if idx + 1 < self.state.layers.len() {
                        self.state.layers.swap(idx, idx + 1);
                        self.state.selected = Some(idx + 1);
                        reindex_print_order(&mut self.state);
                        self.state.composite_dirty = true;
                        self.textures.invalidate();
                    }
                }
            }
        }
    }

    fn open_image_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "images",
                &["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp"],
            )
            .pick_file()
        {
            match image::open(&path) {
                Ok(img) => {
                    // Preserve the alpha channel if present — it's
                    // the artist's ground-truth foreground mask.
                    let rgba = img.to_rgba8();
                    let alpha_mask = super::processing::alpha_to_foreground_mask(&rgba);
                    let rgb = img.to_rgb8();
                    self.state.source = Some(Arc::new(rgb));
                    self.state.source_alpha = alpha_mask.map(Arc::new);
                    self.state.source_path = Some(path.clone());
                    self.state.status = if self.state.source_alpha.is_some() {
                        format!("loaded {} (alpha detected)", path.display())
                    } else {
                        format!("loaded {}", path.display())
                    };
                    self.textures.invalidate();
                    // Recompute the foreground mask for the new source
                    // before running the workflow so layers pick it up
                    // on their first pass.
                    rebuild_foreground_mask(&mut self.state);
                    auto_rerun(&mut self.state);
                }
                Err(e) => {
                    self.state.status = format!("failed to open {}: {e}", path.display());
                }
            }
        }
    }
}

fn reindex_print_order(state: &mut GuiState) {
    for (i, entry) in state.layers.iter_mut().enumerate() {
        entry.layer.print_index = i as u32;
    }
}

impl InkplateApp {
    fn save_project_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Inkplate project", &["inkplate"])
            .set_file_name("project.inkplate")
            .save_file()
        else {
            return;
        };
        let project = Project {
            version: CURRENT_VERSION,
            source_path: self.state.source_path.clone(),
            shirt_color: self.state.shirt_color,
            job: self.state.job,
            workflow: self.state.workflow,
            workflow_opts: self.state.workflow_opts.clone(),
            layers: self
                .state
                .layers
                .iter()
                .map(|e| e.layer.clone())
                .collect::<Vec<Layer>>(),
        };
        match project.save(&path) {
            Ok(()) => self.state.status = format!("saved {}", path.display()),
            Err(e) => self.state.status = format!("save failed: {e}"),
        }
    }

    fn open_project_dialog(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Inkplate project", &["inkplate"])
            .pick_file()
        else {
            return;
        };
        match Project::load(&path) {
            Ok(project) => {
                self.state.shirt_color = project.shirt_color;
                self.state.job = project.job;
                self.state.workflow = project.workflow;
                self.state.workflow_opts = project.workflow_opts;
                self.state.source_path = project.source_path.clone();
                // Try to reload the source image if the path still
                // resolves. If not, leave the source empty — the user
                // will have to re-open the image manually.
                if let Some(src_path) = &project.source_path {
                    if let Ok(img) = image::open(src_path) {
                        self.state.source = Some(Arc::new(img.to_rgb8()));
                    }
                }
                self.state.layers = project
                    .layers
                    .into_iter()
                    .map(super::state::LayerEntry::new)
                    .collect();
                rerun_all_layers(&mut self.state);
                self.textures.invalidate();
                self.state.status = format!("loaded {}", path.display());
            }
            Err(e) => self.state.status = format!("load failed: {e}"),
        }
    }

    fn export_films_dialog(&mut self) {
        if self.state.source.is_none() || self.state.layers.is_empty() {
            self.state.status = "nothing to export".into();
            return;
        }
        let Some(outdir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };

        let source = self
            .state
            .source
            .as_ref()
            .expect("source guard above")
            .clone();
        let layers: Vec<Layer> = self.state.layers.iter().map(|e| e.layer.clone()).collect();

        let opts = ExportOpts {
            dpi: self.state.job.dpi,
            width_inches: None,
            lpi: Some(self.state.job.default_lpi),
            preview_only: false,
            reg_marks: Some(RegMarkOpts::default()),
            border: Some(BorderOpts::default()),
            foreground_mask: self.state.foreground_mask_for_pipeline(),
        };

        match export_all(&source, &layers, &outdir, &opts) {
            Ok(files) => {
                self.state.status =
                    format!("exported {} films → {}", files.len(), outdir.display());
            }
            Err(e) => self.state.status = format!("export failed: {e}"),
        }
    }

    fn export_contact_sheet_dialog(&mut self) {
        if self.state.source.is_none() || self.state.layers.is_empty() {
            self.state.status = "nothing to export".into();
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .set_file_name("contact_sheet.png")
            .save_file()
        else {
            return;
        };

        let source = self
            .state
            .source
            .as_ref()
            .expect("source guard above")
            .clone();
        let layers: Vec<Layer> = self.state.layers.iter().map(|e| e.layer.clone()).collect();

        let opts = inkplate::export::ContactSheetOpts {
            shirt: self.state.shirt_color,
            ..Default::default()
        };
        let sheet = inkplate::export::build_contact_sheet(&source, &layers, &opts);
        match sheet.save(&path) {
            Ok(()) => self.state.status = format!("saved contact sheet → {}", path.display()),
            Err(e) => self.state.status = format!("contact sheet failed: {e}"),
        }
    }
}
