//! riff-gui-components — the presentation primitives of riff's widgetry
//! (ADR 0018).
//!
//! The contract for every module here is **props in, a response or a typed
//! intent out**: a primitive paints from the [`riff_gui_theme::theme::Palette`]
//! tokens it is handed, mutates nothing but caller-owned buffers, and reports
//! what the user did — it decides nothing about the application. Geometry and
//! colour are read from the design authority ([`riff_gui_theme`]), never
//! decided here.
//!
//! No `riff-*` crate may be named by this one: the manifest takes
//! `riff-gui-theme` and `egui` and nothing else, so the manifest edge is the
//! boundary — an offending import fails the build rather than a source sweep.

pub mod button;
pub mod empty_state;
pub mod ghost_icon_button;
pub mod linear;
pub mod menu;
pub mod option_list;
pub mod prompts;
pub mod row;
pub mod scroll_area;
pub mod scroll_control;
pub mod stage;
pub mod text_field;
pub mod toggle_switch;

pub use scroll_control::ScrollControl;

/// Full-texture UV rect for [`egui::Painter::image`]: the full-rect UVs,
/// shared by every texture blit in this crate's primitives.
pub(crate) const UV_FULL: egui::Rect =
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
