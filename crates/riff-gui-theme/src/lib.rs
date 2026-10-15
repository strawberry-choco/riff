//! riff-gui-theme — the design authority for riff's widgetry (ADR 0018).
//!
//! Every visual value riff ships with is declared here and read from here
//! (ADR 0004): the token and palette store ([`theme`]), the vendored Inter
//! font wiring ([`fonts`]), and the Lucide icon set with its tinted-texture
//! cache ([`icons`]), compiled in from the crate's own `assets/fonts` and
//! `assets/icons` with the licences that cover them.
//!
//! No `riff-*` crate may depend into this one or be named by it: the crate
//! takes the GUI toolkit and the icon rasterizer and nothing else, so the
//! design authority can be read without taking any widget library.

pub mod fonts;
pub mod icons;
pub mod theme;
