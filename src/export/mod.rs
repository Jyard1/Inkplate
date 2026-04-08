//! Film export pipeline.
//!
//! Turns a [`Layer`] + source image into publication-ready PNG films:
//! correct DPI metadata, optional registration marks, optional film
//! border with caption, optional contact sheet.
//!
//! Important invariant carried forward from the Python reference:
//!
//! > **Never resample a rasterized halftone.** When the export DPI is
//! > higher than the preview DPI, re-run `process_layer` at export DPI
//! > instead of upscaling the existing raster. Resampling a halftone
//! > smudges the dots into halos and destroys the screen.

pub mod border;
pub mod contact_sheet;
pub mod film;
pub mod reg_marks;

pub use border::BorderOpts;
pub use contact_sheet::{build as build_contact_sheet, ContactSheetOpts};
pub use film::{export_all, export_layer, ExportOpts};
pub use reg_marks::RegMarkOpts;
