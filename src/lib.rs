//! Inkplate — continuous-tone density-map engine for screen-printing color
//! separation.
//!
//! The `engine` submodule is fully headless and can be used from tests, CLI
//! tooling, or embedded in the GUI. The GUI lives in a separate `gui` module
//! that is only compiled for the binary target.

pub mod engine;
pub mod export;
pub mod presets;
pub mod project;

pub use engine::color;
pub use engine::layer::{Layer, RenderMode};
pub use engine::pipeline::{
    compute_composite_union, process_layer, process_layer_with_extraction,
};
