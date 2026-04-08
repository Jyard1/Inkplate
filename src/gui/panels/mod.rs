//! The four UI regions of the app. Each submodule exposes one `show`
//! function that takes `&mut GuiState` plus a `&mut Ui` (or a context
//! where a full panel is drawn) and mutates state in response to
//! clicks/drags.
//!
//! The `app` module is in charge of sequencing these and deciding
//! which reruns to kick off based on what changed.

pub mod global;
pub mod inspector;
pub mod layers;
pub mod preview;
