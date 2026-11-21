//! The [`UiModel`]: the bridge's one owner of the backend runtime and the
//! signals. It lives in a `stored` on the engine thread — nsh signals are
//! `!Send` (P0 correction 9) and the model holds them, so the whole model is
//! engine-thread-only; worker threads never reach it, they hand results over
//! by channel and the pump lands them.
//!
//! The runtime is moved in whole from `AppRuntime::spawn`; the model does
//! not rebuild, re-hydrate or share any of its parts. `session_views` stays
//! the runtime's owned projection seam (P0 correction 6) and every read goes
//! through `&mut` under the model's single borrow.

use riff_backend::composition::{AppRuntime, RuntimeLifecycle};

use crate::bridge::covers::CoverCache;
use crate::bridge::pump::PumpCache;
use crate::bridge::state::UiState;
use crate::model::{CoverViews, ListWindow};

/// What a component renders with: the backend's handles, the signals, and
/// the cover halves. The pump and every handler reach the runtime through
/// the model's `stored` borrow; signal writes happen outside that borrow
/// (P0 correction 20).
pub struct UiModel {
    /// The composed application, moved in from `AppRuntime::spawn`.
    pub(crate) rt: AppRuntime,
    /// The worker threads' levers, kept so a future quit path can
    /// `shutdown()` explicitly (it has no `Drop`). `None` for a mock runtime
    /// (the headless bridge tests compose one from mock ports, whose engine
    /// thread is the test harness's own). Unread until that quit path lands:
    /// the process lifetime currently bounds the workers, exactly as the
    /// spike's did.
    #[allow(dead_code)]
    pub(crate) lifecycle: Option<RuntimeLifecycle>,
    /// The signals. Written on the engine thread only: the pump at runtime,
    /// the debug fixture and readout controls in debug builds.
    pub(crate) state: UiState,
    /// The covers decision half: what is wanted, in flight, arrived.
    pub(crate) covers: CoverCache,
    /// The covers View half: the pixels, bounded (N6's eviction policy,
    /// P0 corrections 10 and 24).
    pub(crate) cover_views: CoverViews,
    /// The list screen's warm row window, filled by the viewability handler
    /// (sequential reads, P0 correction 22) and reset by the pump when the
    /// listing's rows move.
    pub(crate) list_window: ListWindow,
    /// The pump's per-tick comparison cache.
    pub(crate) pump: PumpCache,
}

impl UiModel {
    /// A model over a spawned runtime. Runs on the engine thread: it creates
    /// signals.
    #[must_use]
    pub fn new(rt: AppRuntime, lifecycle: Option<RuntimeLifecycle>) -> Self {
        Self {
            rt,
            lifecycle,
            state: UiState::new(),
            covers: CoverCache::new(),
            cover_views: CoverViews::default(),
            list_window: ListWindow::default(),
            pump: PumpCache::new(),
        }
    }

    /// The signal handles, copied out (the bundle is `Copy`). A consumer
    /// renders from the handles and never re-borrows the model to read a
    /// signal — re-borrowing inside a binding is what the spike's first
    /// pump got wrong (P0 correction 20).
    #[must_use]
    pub fn signals(&self) -> UiState {
        self.state
    }
}

/// Spawn the runtime for the app's synthetic store. The store path is
/// resolved once (`crate::store_path`); a store open/migration failure is
/// fatal at startup — logged, and the process exits non-zero (the plan's
/// "handle the error at startup").
pub(crate) fn spawn_runtime() -> (AppRuntime, RuntimeLifecycle) {
    let store_path = crate::store_path();
    match AppRuntime::spawn(&store_path) {
        Ok(pair) => {
            tracing::info!("Backend runtime spawned over {}", store_path.display());
            pair
        }
        Err(e) => {
            tracing::error!(
                "Fatal: failed to open the Application Store at {}: {e}",
                store_path.display()
            );
            std::process::exit(1);
        }
    }
}
