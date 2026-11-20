//! The riff frontend on nsh: the `riff-nsh` binary, a second frontend beside
//! `riff-gui` until feature parity, when the three egui crates are deleted
//! (ADR 0022).
//!
//! N2 ports the design system: [`theme`] carries the palette, tokens,
//! fonts and icons ported from `crates/riff-gui-theme/src/theme.rs`, the
//! authoritative runtime store is the `Signal<Palette>` in [`app`], and
//! `theme.json`'s `cssVars` maps restyle the `nsh_base` defaults. Nothing
//! here reads the backend or the Application Store yet. The next units
//! land in this crate: N3 adds the `riff-backend` edge and the bridge that
//! moves `AppRuntime` into the UI model in `src/app.rs`; N4 replaces the
//! fixed window size in `src/main.rs` with persisted geometry.

mod app;
pub mod theme;

#[cfg(debug_assertions)]
mod screens;

pub use app::App;
use nsh_core::prelude::*;

/// The window title, and the name `nsh.json` carries. A scaffold test keeps
/// the two equal.
pub const APP_NAME: &str = "riff";

/// The application id: the taskbar's grouping key on Windows, the data
/// directory's name on Linux. `nsh.json` carries the same string; a scaffold
/// test keeps the two equal.
pub const APP_ID: &str = "io.github.strawberry-choco.riff";

/// The theme file, compiled in: riff's palette under shadcn's token names
/// over the nsh defaults, so `nsh_base` controls (menus, dialogs, toasts)
/// sit close to riff. Never the riff palette's store — that is the
/// `Signal<Palette>` in [`app`].
const THEME_JSON: &str = include_str!("../theme.json");

/// The design tokens the app renders with: `theme.json` over nsh's defaults.
///
/// # Panics
///
/// When `theme.json` does not parse. The file is committed beside this crate
/// and a scaffold test parses it, so a failure here is a mistake in the
/// committed file, not runtime input.
pub fn theme_tokens() -> Tokens {
    nsh_base::tokens_of(Some(THEME_JSON)).expect("theme.json is committed and scaffold-tested")
}

/// The app's UI set (`nsh.json` `"rust"`): what a TSX entry imports from
/// `riff/native`, and what the generated `registry()` hands a test.
#[native_module("riff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    registry.assets(assets!());
}
