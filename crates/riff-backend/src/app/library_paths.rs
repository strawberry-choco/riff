//! **Library Path** — the one fact-set per registered root.
//!
//! A Library Path is not a string: it is the path, its Readiness, its Watch
//! State, its live filesystem watcher, and its rows in the Application Store.
//! Those five facts must move together, so they live behind this module's
//! private fields and the operations below are the only way in. `LibrarySession`
//! holds one [`LibraryPaths`] value; nothing outside this module can update
//! three of the five and leave the rest disagreeing.
//!
//! Both edges of that fact-set are owned here, one operation each:
//! [`LibraryPaths::retire`] gives all five up in the order that leaves the
//! least harmful state after a crash, and [`LibraryPaths::hydrate`] takes
//! them back — a restored Watch State is acted on, so a root the user asked
//! to follow is followed again at launch rather than merely reported as
//! followed.
//!
//! Persistence follows the fact-set rather than the other way round: every
//! mutation that has a durable half commits it here. Reading the store back
//! is the restore edge, which `Preferences` calls on the Settings round-trip
//! the Composition Root performs once at launch.
//!
//! What this module does **not** own: the watcher's lifecycle. Turning it into
//! a request-channel worker with a real shutdown is a recorded follow-up; here
//! the module is merely the only caller of `start_watching` / `stop_watching`
//! and the only reader of the optional manager for watch purposes.

use crate::app::state::{LibraryStatus, WatchState};
use crate::app::store::{Settings, SettingsStore};
use crate::app::watcher_manager::WatcherManager;
use riff_persistence::store::LibraryMutationStore;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The registered roots and their three session-held facts.
#[derive(Debug, Clone, Default)]
pub struct LibraryPaths {
    paths: Vec<PathBuf>,
    /// Per-root Readiness as the scan worker last reported it. A slot this
    /// module exposes, not a value it computes.
    statuses: HashMap<PathBuf, LibraryStatus>,
    watch_states: HashMap<PathBuf, WatchState>,
}

impl LibraryPaths {
    // --- Reads: the whole fact-set, never its pieces -----------------------

    /// The registered roots, in registration order.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The root's persisted Watch choice, defaulting to `Disabled`.
    #[must_use]
    pub fn watch_state(&self, root: &Path) -> WatchState {
        self.watch_states
            .get(root)
            .cloned()
            .unwrap_or(WatchState::Disabled)
    }

    /// The root's reported Readiness, defaulting to `Idle` for a root nobody
    /// has reported on yet.
    #[must_use]
    pub fn readiness(&self, root: &Path) -> LibraryStatus {
        self.statuses.get(root).cloned().unwrap_or_default()
    }

    /// Whether any root is being watched — the "watch any" control's state.
    #[must_use]
    pub fn watches_any(&self) -> bool {
        self.paths
            .iter()
            .any(|path| self.watch_state(path) == WatchState::Enabled)
    }

    // --- Mutations: the only way in ----------------------------------------

    /// Register a root: it enters the list with an empty Readiness slot and
    /// the path list is committed. A store failure is logged, not fatal — the
    /// session keeps working and the next save writes the list again.
    ///
    /// Registering starts no Library Scan and no watcher: indexing waits for
    /// the user's rescan, and following a root is a separate choice.
    pub fn register(&mut self, path: PathBuf, settings: &mut dyn SettingsStore) {
        let canonical = std::fs::canonicalize(&path).unwrap_or(path);
        if self.paths.contains(&canonical) {
            return;
        }
        self.paths.push(canonical.clone());
        self.statuses.entry(canonical).or_default();
        if let Err(e) = settings.save_library_paths(&self.paths) {
            tracing::warn!("Failed to save library paths: {e}");
        }
    }

    /// Retire a root, and with it all five of its facts: the list entry, the
    /// Readiness slot, the Watch State, the live watcher, and the store rows.
    /// The other edge of that fact-set is [`Self::hydrate`].
    ///
    /// The two settings writes happen in this order — paths, then Watch
    /// States — so the state a crash can leave is "a Watch State with no
    /// path", which the next save rewrites away, rather than "a path that lost
    /// its Watch State".
    pub fn retire(
        &mut self,
        root: &Path,
        watcher: &mut Option<WatcherManager>,
        settings: &mut dyn SettingsStore,
        mutations: &mut dyn LibraryMutationStore,
    ) {
        if let Err(e) = mutations.remove_library_path(root) {
            tracing::error!("Failed to remove {root:?} from store: {e}");
        }
        if let Some(manager) = watcher {
            manager.stop_watching(root);
        }
        self.paths.retain(|path| path != root);
        self.statuses.remove(root);
        self.watch_states.remove(root);

        if let Err(e) = settings.save_library_paths(&self.paths) {
            tracing::warn!("Failed to save library paths: {e}");
        }
        if let Err(e) = settings.save_watch_states(&self.watch_states) {
            tracing::warn!("Failed to save watch states: {e}");
        }
    }

    /// Start or stop following one root and persist the whole Watch State map
    /// as one write. A failed start degrades to a `Warning` carrying the
    /// diagnostic; either way the choice is durable.
    pub fn set_watch(
        &mut self,
        root: &Path,
        watching: bool,
        watcher: &mut Option<WatcherManager>,
        settings: &mut dyn SettingsStore,
    ) {
        self.watch_states.insert(
            root.to_path_buf(),
            Self::drive_watch(root, watching, watcher),
        );
        self.save_watch_states(settings);
    }

    /// Watch (or stop watching) every registered root as ONE batch with ONE
    /// durable write — a toggle of many roots is one change of one fact-set,
    /// not a save per root.
    pub fn set_watching_for_all(
        &mut self,
        watching: bool,
        watcher: &mut Option<WatcherManager>,
        settings: &mut dyn SettingsStore,
    ) {
        for index in 0..self.paths.len() {
            let root = self.paths[index].clone();
            let state = Self::drive_watch(&root, watching, watcher);
            self.watch_states.insert(root, state);
        }
        self.save_watch_states(settings);
    }

    /// Report a root's Readiness. This is the slot the scan worker writes
    /// through instead of reaching into the session; Readiness is volatile
    /// session state, so nothing is persisted here.
    pub fn report_readiness(&mut self, root: &Path, status: LibraryStatus) {
        self.statuses.insert(root.to_path_buf(), status);
    }

    /// Clear Library: the collection data is gone, so no root may keep
    /// claiming it is indexed. The roots themselves and their Watch States are
    /// Settings, and stand.
    pub fn clear_collection_data(&mut self) {
        for status in self.statuses.values_mut() {
            *status = LibraryStatus::default();
        }
    }

    /// The restore edge, and the mirror of [`Self::retire`]: seed the
    /// fact-set from the Settings loaded at launch. Roots present on disk
    /// start `Idle`, ones that left disk while the app was closed are
    /// reported `Unavailable`, and every registered root's recorded Watch
    /// State is **asked for again** rather than copied in.
    ///
    /// The recorded Watch State is the user's standing choice, not a
    /// diagnostic to replay: `Enabled` starts the root's watcher, and
    /// `Warning` is retried, because a Warning is a verdict about a
    /// condition that may have been fixed while the app was closed. Either
    /// way the session lands the verdict THIS attempt produced, so the
    /// Settings View is never reporting a stale one. `Disabled` starts
    /// nothing and is not even asked to drop anything — at launch no
    /// watcher is running yet.
    ///
    /// Nothing is written back. The Application Store's row is the recorded
    /// choice; the fresh verdict is live session state, and re-recording a
    /// diagnostic on every launch would turn a Warning into a habit.
    pub fn hydrate(&mut self, loaded: &Settings, watcher: &mut Option<WatcherManager>) {
        if !loaded.library_paths.is_empty() {
            for path in &loaded.library_paths {
                let status = if path.exists() {
                    LibraryStatus::Idle
                } else {
                    LibraryStatus::Unavailable
                };
                self.statuses.insert(path.clone(), status);
            }
            self.paths.clone_from(&loaded.library_paths);
        }
        for root in &self.paths {
            let recorded = loaded.watch_states.get(root);
            let wants_watching =
                matches!(recorded, Some(WatchState::Enabled | WatchState::Warning(_)));
            let state = if wants_watching {
                Self::drive_watch(root, true, watcher)
            } else {
                WatchState::Disabled
            };
            self.watch_states.insert(root.clone(), state);
        }
    }

    /// Ask the watcher for one root and turn its answer into a Watch State.
    fn drive_watch(
        root: &Path,
        watching: bool,
        watcher: &mut Option<WatcherManager>,
    ) -> WatchState {
        if !watching {
            if let Some(manager) = watcher {
                manager.stop_watching(root);
            }
            return WatchState::Disabled;
        }
        match watcher.as_mut().map_or_else(
            || Err("Watcher not initialized".to_string()),
            |manager| manager.start_watching(root),
        ) {
            Ok(()) => WatchState::Enabled,
            Err(reason) => WatchState::Warning(reason),
        }
    }

    fn save_watch_states(&self, settings: &mut dyn SettingsStore) {
        if let Err(e) = settings.save_watch_states(&self.watch_states) {
            tracing::warn!("Failed to save watch states: {e}");
        }
    }
}
