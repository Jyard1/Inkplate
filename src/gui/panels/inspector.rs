//! Inspector panel — the slider form for the currently selected layer.
//!
//! This is where the bulk of the GUI's tweakability lives. The form is
//! rebuilt on each frame from the selected layer's current extractor,
//! so switching extractors instantly surfaces the right set of
//! parameters (fuzziness for color_range, s-curve for HSB underbase,
//! strength for GCR, expression box for channel_calc, etc.).
//!
//! Returns `true` if anything changed — the app uses that to decide
//! whether to rerun just this layer.

use eframe::egui::{self, Ui};

use crate::gui::state::GuiState;
use crate::gui::widgets::{
    curve_editor, ink_picker, labeled_slider_f32, labeled_slider_u32, labeled_slider_u8,
};
use inkplate::engine::halftone::{DotShape, HalftoneCurve};
use inkplate::engine::layer::{
    ColorRangeFalloff, EdgeMode, Extractor, IndexDitherKind, RenderMode,
};

pub fn show(ui: &mut Ui, state: &mut GuiState) -> bool {
    let mut changed = false;
    let selected_idx = state.selected;

    ui.heading("Inspector");
    ui.separator();

    let Some(idx) = selected_idx else {
        ui.label(egui::RichText::new("Select a layer to edit its settings.").italics());
        return false;
    };
    let Some(entry) = state.layers.get_mut(idx) else {
        return false;
    };
    let layer = &mut entry.layer;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // ------------------- Identity -------------------
            egui::CollapsingHeader::new("Identity")
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        let resp = ui.text_edit_singleline(&mut layer.name);
                        if resp.changed() {
                            changed = true;
                        }
                    });
                    if ink_picker(ui, "Ink:", &mut layer.ink).changed() {
                        changed = true;
                    }
                    let mut include = layer.include_in_export;
                    if ui.checkbox(&mut include, "Include in export").changed() {
                        layer.include_in_export = include;
                        changed = true;
                    }
                    let prev = layer.opacity;
                    labeled_slider_f32(ui, "Opacity", &mut layer.opacity, 0.0..=1.0);
                    if (layer.opacity - prev).abs() > 1e-4 {
                        changed = true;
                    }
                });

            // ------------------- Extractor -------------------
            egui::CollapsingHeader::new("Extractor")
                .default_open(true)
                .show(ui, |ui| {
                    let before = extractor_tag(&layer.extractor);
                    let mut tag = before;
                    egui::ComboBox::from_id_salt("extractor_combo")
                        .selected_text(extractor_label(tag))
                        .show_ui(ui, |ui| {
                            for t in ALL_EXTRACTOR_TAGS {
                                ui.selectable_value(&mut tag, *t, extractor_label(*t));
                            }
                        });
                    if tag != before {
                        layer.extractor = default_extractor(tag, layer.ink);
                        changed = true;
                    }

                    ui.add_space(4.0);
                    if extractor_form(ui, &mut layer.extractor, layer.ink) {
                        changed = true;
                    }
                });

            // ------------------- Tone -------------------
            egui::CollapsingHeader::new("Tone")
                .default_open(true)
                .show(ui, |ui| {
                    let prev_d = layer.tone.density;
                    labeled_slider_f32(ui, "Density", &mut layer.tone.density, 0.0..=2.0);
                    if (layer.tone.density - prev_d).abs() > 1e-4 {
                        changed = true;
                    }
                    let prev_c = layer.tone.choke;
                    labeled_slider_u32(ui, "Choke (px)", &mut layer.tone.choke, 0..=8);
                    if layer.tone.choke != prev_c {
                        changed = true;
                    }
                    ui.label(
                        egui::RichText::new(format!("curve: {} points", layer.tone.curve.len()))
                            .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(
                            "drag handles to edit · double-click to add · right-click to delete",
                        )
                        .italics()
                        .size(10.0)
                        .color(egui::Color32::from_gray(140)),
                    );
                    if curve_editor(ui, &mut layer.tone.curve) {
                        changed = true;
                    }
                    if ui.button("Reset curve").clicked() {
                        layer.tone.curve = vec![
                            inkplate::engine::tone::CurvePoint::new(0, 0),
                            inkplate::engine::tone::CurvePoint::new(255, 255),
                        ];
                        changed = true;
                    }
                });

            // ------------------- Mask shape -------------------
            egui::CollapsingHeader::new("Mask shape")
                .default_open(false)
                .show(ui, |ui| {
                    if layer.render_mode == inkplate::engine::layer::RenderMode::Solid {
                        let prev = layer.mask.solid_blur;
                        labeled_slider_f32(ui, "Solid blur σ", &mut layer.mask.solid_blur, 0.0..=5.0);
                        if (layer.mask.solid_blur - prev).abs() > 1e-4 {
                            changed = true;
                        }
                    }
                    let prev = layer.mask.smooth_radius;
                    labeled_slider_u32(ui, "Smooth radius", &mut layer.mask.smooth_radius, 0..=10);
                    if layer.mask.smooth_radius != prev {
                        changed = true;
                    }
                    let prev = layer.mask.noise_open;
                    labeled_slider_u32(ui, "Noise open", &mut layer.mask.noise_open, 0..=10);
                    if layer.mask.noise_open != prev {
                        changed = true;
                    }
                    let prev = layer.mask.holes_close;
                    labeled_slider_u32(ui, "Holes close", &mut layer.mask.holes_close, 0..=10);
                    if layer.mask.holes_close != prev {
                        changed = true;
                    }

                    ui.label("Edge mode:");
                    let before = layer.mask.edge_mode;
                    egui::ComboBox::from_id_salt("edge_mode_combo")
                        .selected_text(edge_label(before))
                        .show_ui(ui, |ui| {
                            for m in [
                                EdgeMode::Hard,
                                EdgeMode::Choke,
                                EdgeMode::Spread,
                                EdgeMode::FeatherHt,
                            ] {
                                ui.selectable_value(&mut layer.mask.edge_mode, m, edge_label(m));
                            }
                        });
                    if layer.mask.edge_mode != before {
                        changed = true;
                    }
                    let prev = layer.mask.edge_radius;
                    labeled_slider_u32(ui, "Edge radius", &mut layer.mask.edge_radius, 0..=10);
                    if layer.mask.edge_radius != prev {
                        changed = true;
                    }
                    let mut invert = layer.mask.invert;
                    if ui.checkbox(&mut invert, "Invert mask").changed() {
                        layer.mask.invert = invert;
                        changed = true;
                    }
                });

            // ------------------- Render + halftone -------------------
            egui::CollapsingHeader::new("Render")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label("Render mode:");
                    let before = layer.render_mode;
                    egui::ComboBox::from_id_salt("render_mode_combo")
                        .selected_text(render_label(before))
                        .show_ui(ui, |ui| {
                            for m in [
                                RenderMode::Solid,
                                RenderMode::Halftone,
                                RenderMode::FmDither,
                                RenderMode::BayerDither,
                                RenderMode::NoiseDither,
                                RenderMode::BlueNoise,
                            ] {
                                ui.selectable_value(&mut layer.render_mode, m, render_label(m));
                            }
                        });
                    if layer.render_mode != before {
                        changed = true;
                    }

                    if matches!(layer.render_mode, RenderMode::Halftone) {
                        let prev = layer.halftone.lpi;
                        labeled_slider_u32(
                            ui,
                            "LPI (0 = inherit global)",
                            &mut layer.halftone.lpi,
                            0..=120,
                        );
                        if layer.halftone.lpi != prev {
                            changed = true;
                        }
                        let prev = layer.halftone.angle_deg;
                        labeled_slider_f32(
                            ui,
                            "Angle° (-1 = auto)",
                            &mut layer.halftone.angle_deg,
                            -1.0..=180.0,
                        );
                        if (layer.halftone.angle_deg - prev).abs() > 1e-4 {
                            changed = true;
                        }

                        let before_dot = layer.halftone.dot_shape;
                        let mut dot = before_dot.unwrap_or_default();
                        ui.label("Dot shape:");
                        egui::ComboBox::from_id_salt("dot_shape_combo")
                            .selected_text(format!("{dot:?}"))
                            .show_ui(ui, |ui| {
                                for s in [
                                    DotShape::Round,
                                    DotShape::Square,
                                    DotShape::Ellipse,
                                    DotShape::Line,
                                ] {
                                    ui.selectable_value(&mut dot, s, format!("{s:?}"));
                                }
                            });
                        if Some(dot) != before_dot {
                            layer.halftone.dot_shape = Some(dot);
                            changed = true;
                        }

                        let before_curve = layer.halftone.curve;
                        ui.label("Halftone curve:");
                        egui::ComboBox::from_id_salt("halftone_curve_combo")
                            .selected_text(format!("{before_curve:?}"))
                            .show_ui(ui, |ui| {
                                for c in [
                                    HalftoneCurve::Linear,
                                    HalftoneCurve::SCurve,
                                    HalftoneCurve::Hard,
                                ] {
                                    ui.selectable_value(
                                        &mut layer.halftone.curve,
                                        c,
                                        format!("{c:?}"),
                                    );
                                }
                            });
                        if layer.halftone.curve != before_curve {
                            changed = true;
                        }
                    }
                });
        });

    changed
}

// ---------------------------------------------------------------------------
// Extractor forms
// ---------------------------------------------------------------------------

fn extractor_form(
    ui: &mut Ui,
    extractor: &mut Extractor,
    layer_ink: inkplate::engine::color::Rgb,
) -> bool {
    let mut changed = false;
    match extractor {
        Extractor::SpotSolid { target, tolerance } => {
            if ink_picker(ui, "Target", target).changed() {
                changed = true;
            }
            let prev = *tolerance;
            labeled_slider_u8(ui, "Tolerance", tolerance, 0..=64);
            if *tolerance != prev {
                changed = true;
            }
        }
        Extractor::SpotAa {
            targets,
            target_weights,
            ..
        } => {
            // Look up which target this layer owns (by matching ink)
            // so the Reach slider edits the right slot in the shared
            // weights vec. If the ink drifted off every target, we
            // just show a disabled hint.
            let my_idx = targets.iter().position(|c| *c == layer_ink);

            // Lazy-expand the weights vec if the project was saved
            // with a SpotAa extractor from before target_weights
            // existed. `#[serde(default)]` gives an empty vec in
            // that case; pad with zeros so indices line up.
            if target_weights.len() != targets.len() {
                target_weights.clear();
                target_weights.resize(targets.len(), 0.0);
            }

            ui.label(
                egui::RichText::new(
                    "Reach nudges this plate's effective CIE94 distance. \
                     Higher = the plate steals pixels from its neighbours; \
                     lower (or negative) = it gives them up. Use it when \
                     the automatic Voronoi misassigns shaded regions.",
                )
                .italics()
                .size(10.0)
                .color(egui::Color32::from_gray(140)),
            );

            if let Some(idx) = my_idx {
                let prev = target_weights[idx];
                let mut val = prev;
                labeled_slider_f32(ui, "Reach", &mut val, -30.0..=30.0);
                if (val - prev).abs() > 1e-4 {
                    target_weights[idx] = val;
                    changed = true;
                }
                if ui.button("Reset reach").clicked() && target_weights[idx] != 0.0 {
                    target_weights[idx] = 0.0;
                    changed = true;
                }
            } else {
                ui.label(
                    egui::RichText::new("(layer ink is not in the targets list)")
                        .italics()
                        .size(10.0),
                );
            }
        }
        Extractor::ColorRange {
            target,
            fuzziness,
            falloff,
        } => {
            if ink_picker(ui, "Target", target).changed() {
                changed = true;
            }
            let prev = *fuzziness;
            labeled_slider_f32(ui, "Fuzziness", fuzziness, 1.0..=200.0);
            if (*fuzziness - prev).abs() > 1e-4 {
                changed = true;
            }
            let before = *falloff;
            ui.label("Falloff:");
            egui::ComboBox::from_id_salt("color_range_falloff")
                .selected_text(format!("{before:?}"))
                .show_ui(ui, |ui| {
                    for f in [
                        ColorRangeFalloff::Linear,
                        ColorRangeFalloff::Quadratic,
                        ColorRangeFalloff::Smooth,
                    ] {
                        ui.selectable_value(falloff, f, format!("{f:?}"));
                    }
                });
            if *falloff != before {
                changed = true;
            }
        }
        Extractor::HsbBrightnessInverted {
            s_curve,
            boost_under_darks,
            boost_strength,
        } => {
            let prev = *s_curve;
            labeled_slider_f32(ui, "S-curve strength", s_curve, 0.5..=3.0);
            if (*s_curve - prev).abs() > 1e-4 {
                changed = true;
            }
            let mut boost = *boost_under_darks;
            if ui
                .checkbox(&mut boost, "Boost under saturated darks")
                .changed()
            {
                *boost_under_darks = boost;
                changed = true;
            }
            let prev = *boost_strength;
            labeled_slider_f32(ui, "Boost strength", boost_strength, 0.0..=2.0);
            if (*boost_strength - prev).abs() > 1e-4 {
                changed = true;
            }
        }
        Extractor::LabLightnessInverted => {
            ui.label(egui::RichText::new("No parameters — runs on the LAB L channel.").italics());
        }
        Extractor::GcrBlack {
            strength,
            invert_input,
        } => {
            let prev = *strength;
            labeled_slider_f32(ui, "Strength", strength, 0.0..=2.0);
            if (*strength - prev).abs() > 1e-4 {
                changed = true;
            }
            let mut inv = *invert_input;
            if ui
                .checkbox(&mut inv, "Invert input (highlight white)")
                .changed()
            {
                *invert_input = inv;
                changed = true;
            }
        }
        Extractor::ChannelCalc { expr } => {
            ui.label("Expression:");
            let resp = ui.text_edit_singleline(expr);
            if resp.changed() {
                changed = true;
            }
            ui.label(
                egui::RichText::new("Variables: R G B L a b K | max,min,abs,invert,clip")
                    .italics()
                    .size(11.0),
            );
        }
        Extractor::LuminanceThreshold { threshold, above } => {
            let prev = *threshold;
            labeled_slider_u8(ui, "Threshold", threshold, 0..=255);
            if *threshold != prev {
                changed = true;
            }
            let mut a = *above;
            if ui.checkbox(&mut a, "Keep pixels above threshold").changed() {
                *above = a;
                changed = true;
            }
        }
        Extractor::IndexAssignment {
            palette,
            index,
            dither,
        } => {
            ui.label(format!("Palette: {} colors", palette.len()));
            let prev = *index;
            let mut idx = *index;
            labeled_slider_u32(
                ui,
                "Palette index",
                &mut idx,
                0..=(palette.len().saturating_sub(1).max(1) as u32),
            );
            if idx != prev {
                *index = idx;
                changed = true;
            }
            let before = *dither;
            ui.label("Dither:");
            egui::ComboBox::from_id_salt("index_dither_combo")
                .selected_text(format!("{before:?}"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(dither, IndexDitherKind::Fs, "Floyd-Steinberg");
                    ui.selectable_value(dither, IndexDitherKind::Bayer, "Bayer");
                });
            if *dither != before {
                changed = true;
            }
        }
        Extractor::CmykChannel {
            channel,
            gcr_strength,
        } => {
            ui.label("Channel:");
            let before = *channel;
            egui::ComboBox::from_id_salt("cmyk_channel_combo")
                .selected_text(format!("{before:?}"))
                .show_ui(ui, |ui| {
                    use inkplate::engine::layer::CmykProcess;
                    ui.selectable_value(channel, CmykProcess::Cyan, "Cyan");
                    ui.selectable_value(channel, CmykProcess::Magenta, "Magenta");
                    ui.selectable_value(channel, CmykProcess::Yellow, "Yellow");
                    ui.selectable_value(channel, CmykProcess::Black, "Black");
                });
            if *channel != before {
                changed = true;
            }
            let prev = *gcr_strength;
            labeled_slider_f32(ui, "GCR strength", gcr_strength, 0.0..=1.0);
            if (*gcr_strength - prev).abs() > 1e-4 {
                changed = true;
            }
        }
        Extractor::ManualPaint { buf } => {
            ui.label(
                egui::RichText::new(
                    "Click-drag on the preview (when this layer is selected in Layer view) \
                     to paint into the mask. Use the brush controls in the preview panel \
                     for size and mode.",
                )
                .italics()
                .size(11.0),
            );
            if let Some(b) = buf {
                ui.label(
                    egui::RichText::new(format!("buffer: {}×{}", b.width, b.height))
                        .size(11.0),
                );
                if ui.button("Clear strokes").clicked() {
                    *buf = Some(inkplate::engine::layer::ManualPaintBuf::blank(
                        b.width, b.height,
                    ));
                    changed = true;
                }
            } else {
                ui.label(
                    egui::RichText::new("buffer: (empty — will be allocated on first stroke)")
                        .size(11.0),
                );
            }
        }
        Extractor::CompositeUnion => {
            ui.label(
                egui::RichText::new(
                    "Derived from the composite of all non-black color layers. \
                     Wherever color ink lands, white underbase is placed underneath. \
                     Adjust with the tone curve, choke, and morphology controls below.",
                )
                .italics()
                .size(11.0),
            );
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Tag helpers — so we can offer a "switch extractor" dropdown that
// resets to reasonable defaults when the user picks a different type.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    SpotSolid,
    SpotAa,
    ColorRange,
    HsbBrightnessInverted,
    LabLightnessInverted,
    GcrBlack,
    ChannelCalc,
    LuminanceThreshold,
    IndexAssignment,
    CmykChannel,
    ManualPaint,
    CompositeUnion,
}

const ALL_EXTRACTOR_TAGS: &[Tag] = &[
    Tag::SpotSolid,
    Tag::SpotAa,
    Tag::ColorRange,
    Tag::HsbBrightnessInverted,
    Tag::LabLightnessInverted,
    Tag::GcrBlack,
    Tag::ChannelCalc,
    Tag::LuminanceThreshold,
    Tag::IndexAssignment,
    Tag::CmykChannel,
    Tag::ManualPaint,
    Tag::CompositeUnion,
];

fn extractor_tag(e: &Extractor) -> Tag {
    match e {
        Extractor::SpotSolid { .. } => Tag::SpotSolid,
        Extractor::SpotAa { .. } => Tag::SpotAa,
        Extractor::ColorRange { .. } => Tag::ColorRange,
        Extractor::HsbBrightnessInverted { .. } => Tag::HsbBrightnessInverted,
        Extractor::LabLightnessInverted => Tag::LabLightnessInverted,
        Extractor::GcrBlack { .. } => Tag::GcrBlack,
        Extractor::ChannelCalc { .. } => Tag::ChannelCalc,
        Extractor::LuminanceThreshold { .. } => Tag::LuminanceThreshold,
        Extractor::IndexAssignment { .. } => Tag::IndexAssignment,
        Extractor::CmykChannel { .. } => Tag::CmykChannel,
        Extractor::ManualPaint { .. } => Tag::ManualPaint,
        Extractor::CompositeUnion => Tag::CompositeUnion,
    }
}

fn extractor_label(t: Tag) -> &'static str {
    match t {
        Tag::SpotSolid => "spot_solid",
        Tag::SpotAa => "spot_aa",
        Tag::ColorRange => "color_range",
        Tag::HsbBrightnessInverted => "hsb_brightness_inverted",
        Tag::LabLightnessInverted => "lab_lightness_inverted",
        Tag::GcrBlack => "gcr_black",
        Tag::ChannelCalc => "channel_calc",
        Tag::LuminanceThreshold => "luminance_threshold",
        Tag::IndexAssignment => "index_assignment",
        Tag::CmykChannel => "cmyk_channel",
        Tag::ManualPaint => "manual_paint",
        Tag::CompositeUnion => "composite_union",
    }
}

fn default_extractor(t: Tag, ink: inkplate::engine::color::Rgb) -> Extractor {
    match t {
        Tag::SpotSolid => Extractor::SpotSolid {
            target: ink,
            tolerance: 0,
        },
        Tag::SpotAa => Extractor::SpotAa {
            targets: vec![ink],
            others: vec![],
            aa_full: 4.0,
            aa_end: 14.0,
            aa_reach: 2,
            target_weights: vec![0.0],
        },
        Tag::ColorRange => Extractor::ColorRange {
            target: ink,
            fuzziness: 60.0,
            falloff: ColorRangeFalloff::Smooth,
        },
        Tag::HsbBrightnessInverted => Extractor::HsbBrightnessInverted {
            s_curve: 1.6,
            boost_under_darks: true,
            boost_strength: 0.4,
        },
        Tag::LabLightnessInverted => Extractor::LabLightnessInverted,
        Tag::GcrBlack => Extractor::GcrBlack {
            strength: 1.0,
            invert_input: false,
        },
        Tag::ChannelCalc => Extractor::ChannelCalc {
            expr: "1 - L".into(),
        },
        Tag::LuminanceThreshold => Extractor::LuminanceThreshold {
            threshold: 128,
            above: false,
        },
        Tag::IndexAssignment => Extractor::IndexAssignment {
            palette: vec![ink],
            index: 0,
            dither: IndexDitherKind::Fs,
        },
        Tag::CmykChannel => Extractor::CmykChannel {
            channel: inkplate::engine::layer::CmykProcess::Cyan,
            gcr_strength: 0.75,
        },
        Tag::ManualPaint => Extractor::ManualPaint { buf: None },
        Tag::CompositeUnion => Extractor::CompositeUnion,
    }
}

fn edge_label(m: EdgeMode) -> &'static str {
    match m {
        EdgeMode::Hard => "hard",
        EdgeMode::Choke => "choke",
        EdgeMode::Spread => "spread",
        EdgeMode::FeatherHt => "feather-halftone",
    }
}

fn render_label(m: RenderMode) -> &'static str {
    match m {
        RenderMode::Solid => "solid",
        RenderMode::Halftone => "halftone",
        RenderMode::FmDither => "FM (Floyd-Steinberg)",
        RenderMode::BayerDither => "Bayer",
        RenderMode::NoiseDither => "White noise",
        RenderMode::BlueNoise => "Blue noise",
        RenderMode::IndexFs => "Index (FS)",
        RenderMode::IndexBayer => "Index (Bayer)",
    }
}
