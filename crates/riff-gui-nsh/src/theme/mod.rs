//! The riff design system on nsh: the port of the egui store
//! (`crates/riff-gui-theme/src/theme.rs`, ADR 0004) into the four modules
//! the plan names. The authoritative runtime store is the
//! `Signal<Palette>` the app owns — every riff surface reads its colors
//! from that signal, `theme.json` only restyles `nsh_base` defaults, and
//! the ADR's N2 amendment records the arrangement.
//!
//! - [`palette`] — the 18 color slots and the four variants (dark, light,
//!   and the two High Contrast token sets over them).
//! - [`tokens`] — the type scale, radii, spacing, motion durations, chrome
//!   dimensions and the menu/window shadows.
//! - [`fonts`] — the four vendored Inter weights and their asset constants.
//! - [`icons`] — the 35 Lucide glyphs with the size-and-tint API.
//! - [`brand`] — the riff equalizer mark shared by the titlebar and tray.

pub mod brand;
pub mod fonts;
pub mod icons;
pub mod palette;
pub mod tokens;

pub use palette::{Palette, Variant};
