//! Headless workflow runner — load an image, run a workflow, write all
//! resulting layer density maps as PNGs to a directory.
//!
//! This is the Landing 3 deliverable: proof that the full workflow
//! chain (auto-detect → preset → extractors → pipeline → render) works
//! end-to-end without the GUI.
//!
//! Usage:
//!
//! ```text
//! cargo run --bin separate -- --image art.png --outdir films/
//! cargo run --bin separate -- --image art.png --workflow simprocess_dark --outdir films/
//! cargo run --bin separate -- --image art.png --workflow index_fs --max-colors 6
//! ```
//!
//! If `--workflow` is omitted, auto-detect picks one.

use std::path::PathBuf;
use std::process::ExitCode;

use inkplate::engine::layer::Extractor;
use inkplate::engine::pipeline::{
    compute_composite_union, process_layer, process_layer_with_extraction, JobOpts,
};
use inkplate::engine::preprocess;
use inkplate::engine::workflows::{auto_run, run, Workflow, WorkflowOpts};

#[derive(Debug, Default)]
struct Args {
    image: Option<PathBuf>,
    outdir: Option<PathBuf>,
    workflow: Option<String>,
    max_colors: Option<usize>,
    dpi: Option<u32>,
    lpi: Option<f32>,
    preview: bool,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!("run with --help for usage");
            return ExitCode::from(2);
        }
    };

    let image_path = match args.image.clone() {
        Some(p) => p,
        None => {
            eprintln!("error: --image is required");
            return ExitCode::from(2);
        }
    };
    let outdir = args
        .outdir
        .clone()
        .unwrap_or_else(|| PathBuf::from("films"));

    let source = match image::open(&image_path) {
        Ok(img) => img.to_rgb8(),
        Err(e) => {
            eprintln!("error: failed to open {}: {e}", image_path.display());
            return ExitCode::from(1);
        }
    };

    let mut opts = WorkflowOpts::default();
    if let Some(n) = args.max_colors {
        opts.max_colors = n;
    }

    let (workflow, layers) = match args.workflow.as_deref() {
        None => {
            let (w, ls) = auto_run(&source, &opts);
            println!("auto-detected workflow: {}", w.label());
            (w, ls)
        }
        Some(id) => match Workflow::from_id(id) {
            Some(w) => {
                let ls = run(w, &source, &opts);
                (w, ls)
            }
            None => {
                eprintln!("error: unknown workflow {id:?}");
                eprintln!("known workflows:");
                for w in Workflow::all() {
                    eprintln!("  {:<22}  {}", w.id(), w.label());
                }
                return ExitCode::from(2);
            }
        },
    };

    if layers.is_empty() {
        eprintln!("warning: workflow {} produced 0 layers", workflow.id());
        return ExitCode::SUCCESS;
    }

    if let Err(e) = std::fs::create_dir_all(&outdir) {
        eprintln!("error: failed to create {}: {e}", outdir.display());
        return ExitCode::from(1);
    }

    let job = JobOpts {
        dpi: args.dpi.unwrap_or(300),
        default_lpi: args.lpi.unwrap_or(55.0),
        default_angle_deg: 22.5,
    };

    println!(
        "running {} layers through workflow {}",
        layers.len(),
        workflow.id()
    );

    // Clamp near-black source pixels to true black so color extractors
    // don't report spurious ink for dark areas.
    let source = preprocess::clamp_near_black(&source, 50);

    // Two-pass: process non-CompositeUnion layers first, then derive
    // CompositeUnion layers from their previews.
    let (w, h) = source.dimensions();
    let mut previews: Vec<Option<image::GrayImage>> = vec![None; layers.len()];
    let mut results: Vec<Option<(image::GrayImage, image::GrayImage)>> =
        vec![None; layers.len()];

    for (i, layer) in layers.iter().enumerate() {
        if matches!(layer.extractor, Extractor::CompositeUnion) {
            continue;
        }
        let processed = process_layer(&source, layer, job, None);
        previews[i] = Some(processed.preview.clone());
        results[i] = Some((processed.preview, processed.processed));
    }
    for (i, layer) in layers.iter().enumerate() {
        if !matches!(layer.extractor, Extractor::CompositeUnion) {
            continue;
        }
        let union = compute_composite_union(&layers, &previews, i, w, h, Some(&source));
        let processed = process_layer_with_extraction(union, layer, job, None);
        results[i] = Some((processed.preview, processed.processed));
    }

    for (i, layer) in layers.iter().enumerate() {
        let Some((ref preview, ref processed_img)) = results[i] else {
            continue;
        };
        let image = if args.preview { preview } else { processed_img };
        let filename = format!(
            "{:02}_{}_{:02x}{:02x}{:02x}.png",
            layer.print_index,
            sanitize(&layer.name),
            layer.ink.0,
            layer.ink.1,
            layer.ink.2
        );
        let path = outdir.join(&filename);
        match image.save(&path) {
            Ok(()) => println!("  wrote {}", path.display()),
            Err(e) => eprintln!("  error writing {}: {e}", path.display()),
        }
    }

    ExitCode::SUCCESS
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--image" => args.image = Some(PathBuf::from(next(&mut it, "--image")?)),
            "--outdir" => args.outdir = Some(PathBuf::from(next(&mut it, "--outdir")?)),
            "--workflow" => args.workflow = Some(next(&mut it, "--workflow")?),
            "--max-colors" => {
                args.max_colors = Some(
                    next(&mut it, "--max-colors")?
                        .parse()
                        .map_err(|_| "--max-colors needs an integer".to_string())?,
                )
            }
            "--dpi" => {
                args.dpi = Some(
                    next(&mut it, "--dpi")?
                        .parse()
                        .map_err(|_| "--dpi needs an integer".to_string())?,
                )
            }
            "--lpi" => {
                args.lpi = Some(
                    next(&mut it, "--lpi")?
                        .parse()
                        .map_err(|_| "--lpi needs a number".to_string())?,
                )
            }
            "--preview" => args.preview = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn next<I: Iterator<Item = String>>(it: &mut I, name: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{name} needs a value"))
}

fn print_help() {
    println!("Inkplate workflow runner");
    println!();
    println!("Usage:");
    println!("  separate --image <path> [--workflow <name>] [--outdir <dir>]");
    println!("           [--max-colors N] [--dpi N] [--lpi N] [--preview]");
    println!();
    println!("Workflows:");
    for w in Workflow::all() {
        println!("  {:<22}  {}", w.id(), w.label());
    }
    println!();
    println!("If --workflow is omitted, auto-detect picks one.");
    println!("--preview writes smooth density masks; default writes rasterized halftones.");
}
