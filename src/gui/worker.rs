//! Background processing worker.
//!
//! `process_layer` can take tens of milliseconds on a multi-megapixel
//! source, which is enough to stutter slider drags when it runs on
//! the UI thread. This module hands that work off to a dedicated
//! worker thread and feeds the results back through a channel.
//!
//! Design:
//!
//! - One worker thread, spawned once per [`InkplateApp`] and kept
//!   alive for the life of the program.
//! - A `Mutex<Option<Job>>` used as a single-slot inbox. Producers
//!   overwrite whatever is pending so a fast slider drag coalesces
//!   to "process the latest value" instead of queueing every frame.
//! - A [`std::sync::mpsc`] channel for results, drained from the UI
//!   thread at the top of every [`eframe::App::update`] frame.
//! - A [`egui::Context`] handle so the worker can call
//!   `request_repaint` the moment a job finishes.
//!
//! Generation-counter invalidation:
//!
//! Layer indices can shift between the moment a job is enqueued and
//! the moment it finishes — the user may have deleted a layer,
//! reordered the list, or re-run the workflow. Every job carries a
//! `generation` value, and the app keeps a current counter that
//! bumps on any structural change. Stale results (whose generation
//! is smaller than the current counter) get dropped on arrival.

use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;

use eframe::egui;
use image::{GrayImage, RgbImage};
use inkplate::engine::layer::Layer;
use inkplate::engine::pipeline::{process_layer, JobOpts, ProcessedLayer};

/// A single pending job for the worker thread.
pub struct Job {
    pub generation: u64,
    pub kind: JobKind,
    pub source: Arc<RgbImage>,
    pub job_opts: JobOpts,
    pub foreground_mask: Option<Arc<GrayImage>>,
}

/// What the worker should actually process on this job.
pub enum JobKind {
    /// Reprocess one layer only (the common case after a slider
    /// tweak in the inspector).
    Single { idx: usize, layer: Layer },
    /// Reprocess every layer in order. Used on job opts changes
    /// (DPI / LPI) and background-mask rebuilds.
    All { layers: Vec<Layer> },
}

/// One layer's finished output, shipped back to the UI thread.
pub struct LayerResult {
    pub generation: u64,
    pub idx: usize,
    pub processed: ProcessedLayer,
    pub coverage: f32,
}

/// Handle the UI thread keeps to talk to the worker.
pub struct Worker {
    pending: Arc<(Mutex<Option<Job>>, Condvar)>,
    results: mpsc::Receiver<LayerResult>,
}

impl Worker {
    /// Spawn the worker thread. The [`egui::Context`] is cloned into
    /// the worker so it can call `request_repaint` after every job.
    pub fn spawn(ctx: egui::Context) -> Self {
        let pending = Arc::new((Mutex::new(None::<Job>), Condvar::new()));
        let (tx, rx) = mpsc::channel::<LayerResult>();

        let pending_clone = pending.clone();
        thread::Builder::new()
            .name("inkplate-worker".into())
            .spawn(move || worker_loop(pending_clone, tx, ctx))
            .expect("failed to spawn inkplate worker thread");

        Worker {
            pending,
            results: rx,
        }
    }

    /// Submit a job, replacing whatever was previously pending. The
    /// worker will pick up the newest job as soon as it finishes
    /// whatever it was doing.
    pub fn submit(&self, job: Job) {
        let (lock, cvar) = &*self.pending;
        let mut slot = lock.lock().unwrap();
        *slot = Some(job);
        cvar.notify_one();
    }

    /// Drain all finished results, returning them in the order they
    /// arrived. Non-blocking; safe to call every frame.
    pub fn drain_results(&self) -> Vec<LayerResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.results.try_recv() {
            out.push(r);
        }
        out
    }
}

fn worker_loop(
    pending: Arc<(Mutex<Option<Job>>, Condvar)>,
    results: mpsc::Sender<LayerResult>,
    ctx: egui::Context,
) {
    let (lock, cvar) = &*pending;
    loop {
        // Wait for work. The take() is inside the lock so a producer
        // can replace the pending job the instant we're not looking.
        let job = {
            let mut slot = lock.lock().unwrap();
            while slot.is_none() {
                slot = cvar.wait(slot).unwrap();
            }
            slot.take()
        };
        let Some(job) = job else {
            continue;
        };

        match job.kind {
            JobKind::Single { idx, layer } => {
                let processed = process_layer(
                    &job.source,
                    &layer,
                    job.job_opts,
                    job.foreground_mask.as_deref(),
                );
                let coverage = coverage_fraction(&processed.preview);
                let _ = results.send(LayerResult {
                    generation: job.generation,
                    idx,
                    processed,
                    coverage,
                });
            }
            JobKind::All { layers } => {
                for (idx, layer) in layers.iter().enumerate() {
                    // If a newer job has arrived, bail out of the
                    // loop and pick it up on the next iteration.
                    // This is what makes slider drags feel snappy
                    // during a workflow rerun.
                    let superseded = {
                        let slot = lock.lock().unwrap();
                        slot.as_ref()
                            .map(|j| j.generation > job.generation)
                            .unwrap_or(false)
                    };
                    if superseded {
                        break;
                    }

                    let processed = process_layer(
                        &job.source,
                        layer,
                        job.job_opts,
                        job.foreground_mask.as_deref(),
                    );
                    let coverage = coverage_fraction(&processed.preview);
                    let _ = results.send(LayerResult {
                        generation: job.generation,
                        idx,
                        processed,
                        coverage,
                    });
                    // Wake the UI after every layer so the composite
                    // updates incrementally instead of in one big
                    // burst at the end.
                    ctx.request_repaint();
                }
            }
        }
        ctx.request_repaint();
    }
}

/// Fraction of pixels that are dark (< 128) in a preview mask.
fn coverage_fraction(img: &GrayImage) -> f32 {
    let total = img.width() as f32 * img.height() as f32;
    if total <= 0.0 {
        return 0.0;
    }
    let ink = img.iter().filter(|&&p| p < 128).count() as f32;
    ink / total
}
