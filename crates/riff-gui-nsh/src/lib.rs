//! The riff frontend on nsh: the `riff-nsh` binary, a second frontend beside
//! `riff-gui` until feature parity, when the three egui crates are deleted
//! (ADR 0022).
//!
//! N2 ports the design system: [`theme`] carries the palette, tokens,
//! fonts and icons ported from `crates/riff-gui-theme/src/theme.rs`, and
//! `theme.json`'s `cssVars` maps restyle the `nsh_base` defaults. N3 adds
//! the `riff-backend` edge: [`bridge`] owns the `AppRuntime` from the
//! Composition Root and the [`model`] view vocabulary, and the pump in
//! [`bridge::pump`] is the app's one signal writer. N4 adds the [`shell`]:
//! the titlebar/sidebar/player-bar chrome over the signals and the
//! transport, the keyboard map, and the window-geometry loop through the
//! settings store.

mod app;
pub mod bridge;
pub mod components;
pub mod menus;
mod model;
pub mod platform;
pub mod screens;
pub mod shell;
pub mod theme;

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
/// `Signal<Palette>` in the bridge's `UiState`.
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

/// The Application Store this frontend opens: the **synthetic** N3 store
/// under the workspace's `.scratch/nsh-n3/store/`, never the live data-local
/// store the egui binary uses (the P0 spike's method, carried into N3).
/// `RIFF_N3_STORE` overrides the location — every run and screenshot in the
/// evidence trail goes through it or through the default below.
///
/// `main` resolves this before the window opens and creates the directory;
/// the app component spawns the runtime over the same path.
#[must_use]
pub fn store_path() -> std::path::PathBuf {
    if let Some(override_path) = std::env::var_os("RIFF_N3_STORE") {
        return std::path::PathBuf::from(override_path);
    }
    // Compile-time anchor: `crates/riff-gui-nsh` -> workspace root ->
    // `.scratch/nsh-n3/store/riff.sqlite3`.
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.scratch/nsh-n3/store/riff.sqlite3")
}

/// The app's UI set (`nsh.json` `"rust"`): what a TSX entry imports from
/// `riff/native`, and what the generated `registry()` hands a test.
#[native_module("riff/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
    registry.assets(assets!());
}
