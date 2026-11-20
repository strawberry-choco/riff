//! Font wiring for the nsh frontend, ported from the egui design system's
//! `fonts.rs` (Issue 02). The Inter family is vendored into `assets/fonts/`
//! and compiled into the binary: the four weights register at startup
//! through nsh's `register_font` from `include_bytes!` data, so the binary
//! is self-contained with no runtime font discovery and no dependency on a
//! configured `asset:/` location (the framework's directory bases resolve
//! `asset:/@<package>/<path>`, a packaged layout a dev `cargo run` does not
//! have; `nsh run` serves the registry's files itself).
//!
//! The `asset!` constants stay as the compile-time check that each TTF
//! parses and carries the family name, and as the registry's shipped files
//! (`assets!()` lists them for `nsh run` and packaging).
//!
//! The four vendored weights are the mockup's: regular body text, medium
//! buttons and row labels, semibold section headers, bold wordmark accents.
//! nsh picks a weight on one family (`font_weight`) rather than on four
//! named families, unlike the egui store's named-family approach — the
//! four TTFs all carry the family name `Inter`.

use nsh_core::element::{FontAsset, FontWeight, asset, register_font, spawn};

/// Font-data family of the vendored Inter set: the name `asset!` reads from
/// each TTF's name table, the one `font_family` value any riff text names,
/// and what [`register_inter_fonts`] registers the four files under.
pub const INTER_FAMILY: &str = "Inter";

/// The vendored Inter Regular face — the primary UI font
/// (`--riff-font-sans` head of chain). Registration happens through
/// [`register_inter_fonts`]; this constant is the compile-time check that
/// the file parses as a font and carries the family.
pub const INTER_REGULAR: FontAsset = asset!("fonts/Inter-Regular.ttf");

/// The vendored Inter Medium face — buttons and row labels
/// (`font-medium`).
pub const INTER_MEDIUM: FontAsset = asset!("fonts/Inter-Medium.ttf");

/// The vendored Inter `SemiBold` face — section headers, h1s, the Now
/// Playing title (`font-semibold`).
pub const INTER_SEMIBOLD: FontAsset = asset!("fonts/Inter-SemiBold.ttf");

/// The vendored Inter Bold face — the wordmark accent (`font-bold`).
pub const INTER_BOLD: FontAsset = asset!("fonts/Inter-Bold.ttf");

/// Every vendored Inter face; Regular comes first and doubles as the
/// primary face `theme.json`'s `font-sans` names.
pub const INTER_FACES: [FontAsset; 4] = [INTER_REGULAR, INTER_MEDIUM, INTER_SEMIBOLD, INTER_BOLD];

/// Registers the four vendored Inter weights under [`INTER_FAMILY`]. Called
/// once at startup, before the first frame; text naming the family lays out
/// with them from then on. Each registration's answer is awaited in a task
/// so a failed face is a log line, never a silent fallback to the system
/// font.
pub fn register_inter_fonts() {
    for (face, data) in [
        (
            INTER_REGULAR,
            include_bytes!("../../assets/fonts/Inter-Regular.ttf").as_slice(),
        ),
        (
            INTER_MEDIUM,
            include_bytes!("../../assets/fonts/Inter-Medium.ttf").as_slice(),
        ),
        (
            INTER_SEMIBOLD,
            include_bytes!("../../assets/fonts/Inter-SemiBold.ttf").as_slice(),
        ),
        (
            INTER_BOLD,
            include_bytes!("../../assets/fonts/Inter-Bold.ttf").as_slice(),
        ),
    ] {
        debug_assert_eq!(face.family, INTER_FAMILY);
        let face_name = face.family;
        spawn(async move {
            match register_font(data.to_vec()).await {
                Ok(family) => tracing::info!("riff: registered font family {family:?}"),
                Err(e) => tracing::warn!("riff: registering the vendored font {face_name}: {e}"),
            }
        });
    }
}

/// Inter Regular, the body weight.
pub const WEIGHT_REGULAR: FontWeight = FontWeight::NORMAL;
/// Inter Medium, the weight the egui store maps buttons and row labels to.
pub const WEIGHT_MEDIUM: FontWeight = FontWeight::MEDIUM;
/// Inter `SemiBold`, the weight of section headers and the hero title.
pub const WEIGHT_SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;
/// Inter Bold, the wordmark accent's weight.
pub const WEIGHT_BOLD: FontWeight = FontWeight::BOLD;
