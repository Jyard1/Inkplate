//! Headless extractor smoke test.
//!
//! Loads an image, runs one extractor against it, and writes the
//! resulting density map as a PNG. Useful for eyeballing each extractor
//! without the GUI, and as the Landing 2 deliverable.
//!
//! Usage:
//!
//! ```text
//! cargo run --bin extract -- --image foo.png --extractor color_range \
//!     --target #c43a2f --fuzziness 60 --out out.png
//! ```
//!
//! Supported extractors (`--extractor`):
//!
//! - `spot_solid`              — requires `--target` `[--tolerance N]`
//! - `color_range`             — requires `--target` `[--fuzziness F]`
//! - `hsb_brightness_inverted` — no required args
//! - `lab_lightness_inverted`  — no required args
//! - `gcr_black`               — `[--strength F] [--invert]`
//! - `luminance_threshold`     — `[--threshold N] [--above]`
//! - `channel_calc`            — `--expr "1 - L"`
//!
//! `spot_aa` and `index_assignment` need palette arrays that are awkward
//! to pass on the command line; run them through the library instead.

use std::path::PathBuf;
use std::process::ExitCode;

use inkplate::engine::color::{hex_to_rgb, Rgb};
use inkplate::engine::layer::{ColorRangeFalloff, Extractor, Layer, LayerKind, MaskShape, Tone};
use inkplate::engine::layer::{HalftoneOverrides, RenderMode};
use inkplate::engine::pipeline::{process_layer, JobOpts};
use uuid::Uuid;

#[derive(Debug, Default)]
struct Args {
    image: Option<PathBuf>,
    out: Option<PathBuf>,
    extractor: Option<String>,
    target: Option<Rgb>,
    tolerance: Option<u8>,
    fuzziness: Option<f32>,
    strength: Option<f32>,
    invert: bool,
    threshold: Option<u8>,
    above: bool,
    expr: Option<String>,
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
    let out_path = args.out.clone().unwrap_or_else(|| PathBuf::from("out.png"));
    let extractor_name = args.extractor.clone().unwrap_or_else(|| "gcr_black".into());

    let source = match image::open(&image_path) {
        Ok(img) => img.to_rgb8(),
        Err(e) => {
            eprintln!("error: failed to open {}: {e}", image_path.display());
            return ExitCode::from(1);
        }
    };

    let extractor = match build_extractor(&extractor_name, &args) {
        Ok(e) => e,
        Err(msg) => {
            eprintln!("error: {msg}");
            return ExitCode::from(2);
        }
    };

    let layer = Layer {
        id: Uuid::new_v4(),
        name: extractor_name.clone(),
        kind: LayerKind::Color,
        ink: args.target.unwrap_or(Rgb(0, 0, 0)),
        visible: true,
        locked: false,
        include_in_export: true,
        opacity: 1.0,
        extractor,
        tone: Tone::default(),
        mask: MaskShape::default(),
        render_mode: RenderMode::Solid,
        halftone: HalftoneOverrides::default(),
        print_index: 0,
    };

    let processed = process_layer(&source, &layer, JobOpts::default(), None);
    if let Err(e) = processed.preview.save(&out_path) {
        eprintln!("error: failed to save {}: {e}", out_path.display());
        return ExitCode::from(1);
    }

    println!(
        "wrote {} ({}×{}, extractor: {})",
        out_path.display(),
        processed.preview.width(),
        processed.preview.height(),
        extractor_name
    );
    ExitCode::SUCCESS
}

fn build_extractor(name: &str, args: &Args) -> Result<Extractor, String> {
    match name {
        "spot_solid" => {
            let target = args
                .target
                .ok_or_else(|| "spot_solid needs --target #RRGGBB".to_string())?;
            Ok(Extractor::SpotSolid {
                target,
                tolerance: args.tolerance.unwrap_or(0),
            })
        }
        "color_range" => {
            let target = args
                .target
                .ok_or_else(|| "color_range needs --target #RRGGBB".to_string())?;
            Ok(Extractor::ColorRange {
                target,
                fuzziness: args.fuzziness.unwrap_or(60.0),
                falloff: ColorRangeFalloff::Smooth,
            })
        }
        "hsb_brightness_inverted" => Ok(Extractor::HsbBrightnessInverted {
            s_curve: 1.6,
            boost_under_darks: true,
            boost_strength: 0.4,
        }),
        "lab_lightness_inverted" => Ok(Extractor::LabLightnessInverted),
        "gcr_black" => Ok(Extractor::GcrBlack {
            strength: args.strength.unwrap_or(1.0),
            invert_input: args.invert,
        }),
        "luminance_threshold" => Ok(Extractor::LuminanceThreshold {
            threshold: args.threshold.unwrap_or(128),
            above: args.above,
        }),
        "channel_calc" => {
            let expr = args
                .expr
                .clone()
                .ok_or_else(|| "channel_calc needs --expr \"<formula>\"".to_string())?;
            Ok(Extractor::ChannelCalc { expr })
        }
        other => Err(format!("unknown extractor: {other}")),
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--image" => args.image = Some(PathBuf::from(next_val(&mut it, "--image")?)),
            "--out" => args.out = Some(PathBuf::from(next_val(&mut it, "--out")?)),
            "--extractor" => args.extractor = Some(next_val(&mut it, "--extractor")?),
            "--target" => {
                let v = next_val(&mut it, "--target")?;
                args.target = Some(hex_to_rgb(&v).map_err(|e| e.to_string())?);
            }
            "--tolerance" => args.tolerance = Some(parse_u8(&next_val(&mut it, "--tolerance")?)?),
            "--fuzziness" => args.fuzziness = Some(parse_f32(&next_val(&mut it, "--fuzziness")?)?),
            "--strength" => args.strength = Some(parse_f32(&next_val(&mut it, "--strength")?)?),
            "--invert" => args.invert = true,
            "--threshold" => args.threshold = Some(parse_u8(&next_val(&mut it, "--threshold")?)?),
            "--above" => args.above = true,
            "--expr" => args.expr = Some(next_val(&mut it, "--expr")?),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn next_val<I: Iterator<Item = String>>(it: &mut I, name: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{name} requires a value"))
}

fn parse_u8(s: &str) -> Result<u8, String> {
    s.parse()
        .map_err(|_| format!("expected integer, got {s:?}"))
}

fn parse_f32(s: &str) -> Result<f32, String> {
    s.parse().map_err(|_| format!("expected number, got {s:?}"))
}

fn print_help() {
    println!("Inkplate extractor smoke test");
    println!();
    println!("Usage:");
    println!("  extract --image <path> --extractor <name> [options] [--out <path>]");
    println!();
    println!("Extractors:");
    println!("  spot_solid              --target #RRGGBB [--tolerance N]");
    println!("  color_range             --target #RRGGBB [--fuzziness F]");
    println!("  hsb_brightness_inverted");
    println!("  lab_lightness_inverted");
    println!("  gcr_black               [--strength F] [--invert]");
    println!("  luminance_threshold     [--threshold N] [--above]");
    println!("  channel_calc            --expr \"<formula>\"");
}
