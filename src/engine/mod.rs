//! Headless image processing engine.
//!
//! Every piece of the separation pipeline — color science, palette
//! extraction, morphology, tone curves, halftoning, dithering, extractors,
//! workflows — lives under this module. Nothing here depends on the GUI,
//! so the entire engine is usable from CLI tools and unit tests.
//!
//! The pipeline entry point is [`pipeline::process_layer`], which takes a
//! source image and a [`layer::Layer`] spec and returns both a smooth
//! "preview" density mask and a rasterized "processed" film-ready image.

pub mod color;
pub mod dither;
pub mod extractors;
pub mod foreground;
pub mod halftone;
pub mod layer;
pub mod morphology;
pub mod palette;
pub mod pipeline;
pub mod preprocess;
pub mod tone;
pub mod workflows;

/// Single-channel 8-bit image (H×W), used everywhere as the density-map
/// currency.  `0` means full ink, `255` means no ink — this matches the
/// original Python convention and keeps the math identical when porting
/// pixel-for-pixel.
pub type GrayImage = image::GrayImage;

/// 8-bit RGB image. Source artwork and color thumbnails.
pub type RgbImage = image::RgbImage;

/// 8-bit RGBA image. Used when the source has an alpha channel and the
/// foreground mask needs to respect transparency.
pub type RgbaImage = image::RgbaImage;
