//! Main `eframe::App` implementation. Holds `GuiState` + `TextureCache`,
//! lays out the four panels, and routes panel actions back into the
//! processing helpers.

use std::sync::Arc;

use eframe::egui;
use inkplate::engine::layer::Layer;
use inkplate::export::{export_all, BorderOpts, ExportOpts, RegMarkOpts};
use inkplate::project::{Project, CURRENT_VERSION};

use super::history::{History, Snapshot};
use super::panels::{global, inspector, layers, preview};
use super::processing::{
    auto_rerun, merge_same_ink_layers, merge_to_shadow_halftone, rebuild_foreground_mask,
    rerun_workflow_layers_only, sync_spot_reach_weights,
};
use super::state::GuiState;
use super::textures::TextureCache;
use super::worker::{Job, JobKind, Worker};

pub struct InkplateApp {
    state: GuiState,
    textures: TextureCache,
    history: History,
    worker: Worker,
    /// Monotonic counter bumped whenever the layer list changes
    /// structurally (workflow rerun, delete, reorder). Every job
    /// submitted to the worker carries this value; results whose
    /// generation is smaller than the current counter are stale and
    /// get dropped instead of being written back into state.layers.
    generation: u64,
    /// True when an inspector drag is in progress. The first frame
    /// of a new drag takes an undo snapshot; subsequent frames skip
    /// the snapshot so one slider drag = one undo step.
    inspector_drag_active: bool,
}

impl InkplateApp {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            state: GuiState::default(),
            textures: TextureCache::default(),
            history: History::default(),
            worker: Worker::spawn(ctx),
            generation: 0,
            inspector_drag_active: false,
        }
    }
}

impl Default for InkplateApp {
    fn default() -> Self {
        // Only used by tests / tooling that construct the app before
        // an egui context exists. The worker won't receive any
        // request_repaint calls until the app is re-created via
        // `InkplateApp::new(ctx)`, but it'll still process jobs.
        Self::new(egui::Context::default())
    }
}

impl eframe::App for InkplateApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---- Drain any finished layer results from the worker
        // thread and apply them. Stale results (from before the
        // last structural change to the layer list) are dropped. ----
        for result in self.worker.drain_results() {
            if result.generation != self.generation {
                continue;
            }
            if let Some(entry) = self.state.layers.get_mut(result.idx) {
                entry.coverage = result.coverage;
                entry.preview = Some(result.processed.preview);
                entry.processed = Some(result.processed.processed);
                self.state.composite_dirty = true;
                self.textures.invalidate();
            }
        }

        // ---- Ctrl+Z / Ctrl+Y keyboard shortcuts, handled at the
        // top of the frame so every panel sees the updated state. ----
        ctx.input_mut(|i| {
            let undo_pressed = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Z,
            ));
            let redo_pressed = i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::Y,
            )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::Z,
            ));
            (undo_pressed, redo_pressed)
        });
        // Re-read without consuming so we can actually act. We do
        // the consume above to keep egui from also dispatching these
        // keys to text widgets.
        let (undo_hit, redo_hit) = ctx.input(|i| {
            let undo = i.key_pressed(egui::Key::Z)
                && i.modifiers.command
                && !i.modifiers.shift;
            let redo = (i.key_pressed(egui::Key::Y) && i.modifiers.command)
                || (i.key_pressed(egui::Key::Z)
                    && i.modifiers.command
                    && i.modifiers.shift);
            (undo, redo)
        });
        if undo_hit {
            self.apply_undo();
        }
        if redo_hit {
            self.apply_redo();
        }

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
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        let mut inspector_changed = false;
        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(320.0)
            .min_width(260.0)
            .show(ctx, |ui| {
                if inspector::show(ui, &mut self.state) {
                    inspector_changed = true;
                }
            });
        if inspector_changed {
            // First edit in a new slider drag → snapshot.
            if !self.inspector_drag_active {
                self.history.push(self.snapshot());
                self.inspector_drag_active = true;
            }
            if let Some(idx) = self.state.selected {
                // If the user dragged a Reach slider on a SpotAa
                // layer, propagate the new weight into every other
                // SpotAa layer's target list and re-process them
                // all. Otherwise it's a one-layer edit and we
                // only need to rerun the selected layer.
                let is_spot_aa = matches!(
                    self.state.layers.get(idx).map(|e| &e.layer.extractor),
                    Some(inkplate::engine::layer::Extractor::SpotAa { .. })
                );
                if is_spot_aa {
                    sync_spot_reach_weights(&mut self.state, idx);
                    self.submit_all_layers_job();
                } else {
                    self.submit_layer_job(idx);
                }
            }
        }
        // Drag ended once the mouse comes up → arm the next slider
        // edit to take a fresh snapshot.
        if !pointer_down {
            self.inspector_drag_active = false;
        }

        // ---- center: preview ----
        let mut paint_stroke = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            paint_stroke = preview::show(ui, &mut self.state, &mut self.textures);
        });
        if paint_stroke {
            // Treat a brush stroke just like an inspector edit: one
            // undo slot per drag, then rerun the selected layer so
            // the preview mask updates.
            if !self.inspector_drag_active {
                self.history.push(self.snapshot());
                self.inspector_drag_active = true;
            }
            if let Some(idx) = self.state.selected {
                self.submit_layer_job(idx);
            }
        }
    }
}

impl InkplateApp {
    fn handle_global_action(&mut self, action: global::Action) {
        match action {
            global::Action::None => {}
            global::Action::OpenImage => {
                // New document → undo/redo history is no longer
                // meaningful, start clean.
                self.history.clear();
                self.open_image_dialog();
            }
            global::Action::RunWorkflow => {
                // A workflow rerun replaces the whole layer list —
                // big change, worth a single undo slot.
                if !self.state.layers.is_empty() {
                    self.history.push(self.snapshot());
                }
                rerun_workflow_layers_only(&mut self.state);
                self.bump_generation();
                self.submit_all_layers_job();
                self.textures.invalidate();
            }
            global::Action::JobChanged => {
                self.submit_all_layers_job();
                self.textures.invalidate();
            }
            global::Action::SaveProject => self.save_project_dialog(),
            global::Action::OpenProject => {
                self.history.clear();
                self.open_project_dialog();
            }
            global::Action::ExportFilms => self.export_films_dialog(),
            global::Action::ExportContactSheet => self.export_contact_sheet_dialog(),
            global::Action::BackgroundRemovalChanged => {
                rebuild_foreground_mask(&mut self.state);
                self.submit_all_layers_job();
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

    // --- worker submission plumbing ----------------------------------

    /// Bump the generation counter so any results from jobs submitted
    /// before this point get dropped when they arrive. Called after
    /// any structural change to the layer list (rerun workflow,
    /// delete, reorder) so in-flight jobs don't write stale masks
    /// into the new layers.
    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Submit a job that reprocesses just one layer. Used for the
    /// "slider moved in the inspector" hot path.
    fn submit_layer_job(&self, idx: usize) {
        let Some(source) = self.state.source.clone() else {
            return;
        };
        let Some(entry) = self.state.layers.get(idx) else {
            return;
        };
        self.worker.submit(Job {
            generation: self.generation,
            kind: JobKind::Single {
                idx,
                layer: entry.layer.clone(),
            },
            source,
            job_opts: self.state.job,
            foreground_mask: self.state.foreground_mask_for_pipeline(),
        });
    }

    /// Submit a job that reprocesses every layer. Used after a
    /// workflow rerun, DPI/LPI change, or background-mask rebuild.
    fn submit_all_layers_job(&self) {
        let Some(source) = self.state.source.clone() else {
            return;
        };
        if self.state.layers.is_empty() {
            return;
        }
        let layers: Vec<_> = self
            .state
            .layers
            .iter()
            .map(|e| e.layer.clone())
            .collect();
        self.worker.submit(Job {
            generation: self.generation,
            kind: JobKind::All { layers },
            source,
            job_opts: self.state.job,
            foreground_mask: self.state.foreground_mask_for_pipeline(),
        });
    }

    fn handle_layers_action(&mut self, action: layers::Action) {
        match action {
            layers::Action::None => {}
            layers::Action::VisibilityChanged => {
                self.history.push(self.snapshot());
                self.state.composite_dirty = true;
                self.textures.invalidate();
            }
            layers::Action::SelectionChanged => {
                // No rerun needed — just affects inspector population
                // and preview-mode options. Not worth an undo slot.
            }
            layers::Action::DeleteSelected => {
                if let Some(idx) = self.state.selected {
                    if idx < self.state.layers.len() {
                        self.history.push(self.snapshot());
                        self.state.layers.remove(idx);
                        self.state.selected = if self.state.layers.is_empty() {
                            None
                        } else {
                            Some(idx.saturating_sub(1).min(self.state.layers.len() - 1))
                        };
                        self.bump_generation();
                        self.state.composite_dirty = true;
                        self.textures.invalidate();
                    }
                }
            }
            layers::Action::MoveUp => {
                if let Some(idx) = self.state.selected {
                    if idx > 0 {
                        self.history.push(self.snapshot());
                        self.state.layers.swap(idx - 1, idx);
                        self.state.selected = Some(idx - 1);
                        reindex_print_order(&mut self.state);
                        self.bump_generation();
                        self.state.composite_dirty = true;
                        self.textures.invalidate();
                    }
                }
            }
            layers::Action::MoveDown => {
                if let Some(idx) = self.state.selected {
                    if idx + 1 < self.state.layers.len() {
                        self.history.push(self.snapshot());
                        self.state.layers.swap(idx, idx + 1);
                        self.state.selected = Some(idx + 1);
                        reindex_print_order(&mut self.state);
                        self.bump_generation();
                        self.state.composite_dirty = true;
                        self.textures.invalidate();
                    }
                }
            }
            layers::Action::MergeShadowHalftone => {
                // Snapshot first — this is a destructive multi-layer
                // operation and the user will absolutely want Ctrl+Z
                // if they don't like the result.
                self.history.push(self.snapshot());
                let sel = self.state.multi_select.clone();
                match merge_to_shadow_halftone(&mut self.state, &sel) {
                    Some(report) => {
                        self.state.status = format!(
                            "merged {} layers onto {} + shadow halftone",
                            report.merged_count, report.keeper_name
                        );
                        self.state.multi_select.clear();
                        // Keeper may have shifted; reselect it so
                        // the inspector lands on something sane.
                        self.state.selected = Some(0);
                        self.bump_generation();
                        self.textures.invalidate();
                    }
                    None => {
                        if let Some(s) = self.history.undo_pop() {
                            drop(s);
                        }
                        self.state.status =
                            "merge failed: need ≥2 processed layers of same dimensions".into();
                    }
                }
            }
            layers::Action::MergeSameInk => {
                self.history.push(self.snapshot());
                let sel = self.state.multi_select.clone();
                match merge_same_ink_layers(&mut self.state, &sel) {
                    Some(report) => {
                        self.state.status = format!(
                            "merged {} same-ink layers onto {}",
                            report.merged_count, report.keeper_name
                        );
                        self.state.multi_select.clear();
                        self.state.selected = Some(0);
                        self.bump_generation();
                        self.textures.invalidate();
                    }
                    None => {
                        if let Some(s) = self.history.undo_pop() {
                            drop(s);
                        }
                        self.state.status =
                            "merge failed: need ≥2 processed layers with matching ink".into();
                    }
                }
            }
        }
    }

    // --- undo / redo plumbing -------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            layers: self.state.layers.clone(),
            selected: self.state.selected,
        }
    }

    fn apply_snapshot(&mut self, snap: Snapshot) {
        self.state.layers = snap.layers;
        self.state.selected = snap.selected;
        self.state.composite_dirty = true;
        self.textures.invalidate();
    }

    fn apply_undo(&mut self) {
        if let Some(prev) = self.history.undo_pop() {
            let current = self.snapshot();
            self.history.stash_redo(current);
            self.apply_snapshot(prev);
            self.state.status = "undo".into();
        }
    }

    fn apply_redo(&mut self) {
        if let Some(next) = self.history.redo_pop() {
            let current = self.snapshot();
            self.history.stash_undo(current);
            self.apply_snapshot(next);
            self.state.status = "redo".into();
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
                    self.bump_generation();
                    self.submit_all_layers_job();
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
                self.bump_generation();
                self.submit_all_layers_job();
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
