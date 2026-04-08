//! Film border + caption. Adds a thin margin around the image with a
//! text strip below containing layer metadata (index, name, ink hex,
//! LPI, angle, render mode).
//!
//! Font loading tries common Windows / macOS / Linux paths and falls
//! back to skipping the caption entirely if no TTF can be found. That's
//! fine for a first pass — a bundled fallback font is a Landing 6
//! polish item.

use std::sync::OnceLock;

use ab_glyph::{FontArc, PxScale};
use image::{GrayImage, ImageBuffer, Luma};
use imageproc::drawing::{draw_hollow_rect_mut, draw_text_mut};
use imageproc::rect::Rect;

use crate::engine::layer::{Layer, RenderMode};

#[derive(Debug, Clone, Copy)]
pub struct BorderOpts {
    /// Margin in pixels added on all four sides.
    pub margin_px: u32,
    /// Extra height added at the bottom for the caption strip.
    pub caption_px: u32,
    /// Font size in pixels. Auto-computed from image width if 0.
    pub font_px: u32,
}

impl Default for BorderOpts {
    fn default() -> Self {
        Self {
            margin_px: 24,
            caption_px: 56,
            font_px: 0,
        }
    }
}

/// Draw a border + caption around `film`, returning a new image. The
/// caller is expected to chain this after `process_layer` and any
/// registration-mark pass.
pub fn draw(
    film: &GrayImage,
    opts: &BorderOpts,
    layer: &Layer,
    layer_index: usize,
    lpi: f32,
) -> GrayImage {
    let (iw, ih) = film.dimensions();
    let out_w = iw + opts.margin_px * 2;
    let out_h = ih + opts.margin_px * 2 + opts.caption_px;
    let mut out: GrayImage = ImageBuffer::from_pixel(out_w, out_h, Luma([255]));

    // Paste the film into the top region, offset by the margin.
    for y in 0..ih {
        for x in 0..iw {
            out.put_pixel(
                x + opts.margin_px,
                y + opts.margin_px,
                *film.get_pixel(x, y),
            );
        }
    }

    // Thin rectangle outline around the film area.
    let rect = Rect::at(opts.margin_px as i32, opts.margin_px as i32).of_size(iw, ih);
    draw_hollow_rect_mut(&mut out, rect, Luma([0]));

    // Caption below.
    if let Some(font) = font() {
        let font_px = if opts.font_px == 0 {
            (iw as f32 / 70.0).clamp(14.0, 40.0)
        } else {
            opts.font_px as f32
        };
        let scale = PxScale::from(font_px);
        let caption = format_caption(layer, layer_index, lpi);
        let x = opts.margin_px as i32;
        let y = (opts.margin_px + ih + opts.margin_px / 2) as i32;
        draw_text_mut(&mut out, Luma([0]), x, y, scale, font, &caption);
    }

    out
}

fn format_caption(layer: &Layer, layer_index: usize, lpi: f32) -> String {
    let ink = format!("#{:02X}{:02X}{:02X}", layer.ink.0, layer.ink.1, layer.ink.2);
    let mode = match layer.render_mode {
        RenderMode::Solid => "solid",
        RenderMode::Halftone => "halftone",
        RenderMode::FmDither => "FM",
        RenderMode::BayerDither => "Bayer",
        RenderMode::NoiseDither => "noise",
        RenderMode::BlueNoise => "blue noise",
        RenderMode::IndexFs => "idx FS",
        RenderMode::IndexBayer => "idx Bayer",
    };
    let lpi_field = if matches!(layer.render_mode, RenderMode::Halftone) {
        let effective_lpi = if layer.halftone.lpi > 0 {
            layer.halftone.lpi as f32
        } else {
            lpi
        };
        format!("  {}lpi", effective_lpi.round() as u32)
    } else {
        String::new()
    };
    let angle_field =
        if matches!(layer.render_mode, RenderMode::Halftone) && layer.halftone.angle_deg >= 0.0 {
            format!("  {:.0}°", layer.halftone.angle_deg)
        } else {
            String::new()
        };
    format!(
        "{:02}  {}  {}  {}{}{}",
        layer_index + 1,
        layer.name,
        ink,
        mode,
        lpi_field,
        angle_field,
    )
}

// ---------------------------------------------------------------------------
// Font loading — cached in a OnceLock so we only pay the filesystem +
// parse cost once per process.
// ---------------------------------------------------------------------------

static FONT: OnceLock<Option<FontArc>> = OnceLock::new();

fn font() -> Option<&'static FontArc> {
    FONT.get_or_init(load_font).as_ref()
}

fn load_font() -> Option<FontArc> {
    const CANDIDATES: &[&str] = &[
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "C:/Windows/Fonts/tahoma.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
    ];
    for path in CANDIDATES {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(font) = FontArc::try_from_vec(bytes) {
                return Some(font);
            }
        }
    }
    // TODO(L6): bundle a small fallback font (e.g. Inter or DejaVu
    // Sans subset) in the binary so captions always render.
    None
}
