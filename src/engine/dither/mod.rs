//! Dither algorithms for grayscale → binary density maps and LAB → palette
//! assignments.
//!
//! Four algorithms live here, each in its own file:
//!
//! - [`floyd_steinberg`] — serpentine error diffusion (FM screen)
//! - [`bayer`] — ordered dither with cached 2/4/8 matrices
//! - [`blue_noise`] — void-and-cluster tile, generated once per tile size
//! - [`white_noise`] — pure white-noise threshold (cheap, for previews)
//!
//! Every module exposes a `*_grayscale` function that takes a [`GrayImage`]
//! and returns a binary [`GrayImage`] (still 8-bit, but values are only
//! 0 or 255). The palette-assignment variants used by index-mode workflows
//! live with the index extractor, not here, since they need LAB distance.

pub mod bayer;
pub mod blue_noise;
pub mod floyd_steinberg;
pub mod white_noise;

pub use bayer::bayer_grayscale;
pub use blue_noise::blue_noise_grayscale;
pub use floyd_steinberg::floyd_steinberg_grayscale;
pub use white_noise::white_noise_grayscale;
