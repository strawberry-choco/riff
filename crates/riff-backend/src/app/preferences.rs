//! The Preferences module: owns the Settings round-trip.
//!
//! A preference change is durable by construction, never by call-site
//! discipline: [`Preferences::hydrate`] loads the Application Store's
//! Settings into the two sessions once at startup and seeds the
//! last-committed snapshot, and [`Preferences::commit_if_changed`] runs at
//! frame end and saves exactly when the sessions have drifted from that
//! snapshot. There is no per-handler persist call to forget — a handler only
//! mutates a session, and the next frame-end commit lands it in the store.
//!
//! Scalars only: Library Paths and Watch States are structural preferences
//! with their own single mutation sites (`save_library_paths` /
//! `save_watch_states`), not part of the scalar diff.
//!
//! The enum↔store-code maps (`RepeatMode` ↔ 0/1/2, the scan-pref fields)
//! live here beside their [`ScalarSettings`] semantics, so this module is
//! the only place that knows both the session shapes and the persisted row.

use std::sync::{Arc, Mutex};

use riff_persistence::store::ScalarSettings;
use riff_playback::app::state::PlaybackSession;
use riff_playback::app::transport::Transport;

use crate::app::MutexExt;
use crate::app::state::{BrowserLayout, LibrarySession, LibraryStatus, ScanPrefs};
use crate::app::store::{Settings, SettingsStore};
use crate::domain::RepeatMode;

/// The stored side of the scalar Settings round-trip: hydrate once at
/// startup, diff-commit at frame end.
#[derive(Default)]
pub struct Preferences {
    last_committed: ScalarSettings,
}

impl Preferences {
    /// First frame: hydrate both sessions from the Application Store and
    /// seed `last_committed` so the first diff-commit is a no-op.
    ///
    /// The transport receives one `SetVolume` command built from the
    /// restored (effective) volume, so the engine starts at the persisted
    /// level even before the user touches anything.
    pub fn hydrate(
        playback: &Arc<Mutex<PlaybackSession>>,
        library: &Arc<Mutex<LibrarySession>>,
        store: &dyn SettingsStore,
        transport: &dyn Transport,
    ) -> Self {
        let settings = match store.load_settings() {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!("Failed to load settings from the store: {e}");
                Settings::default()
            }
        };

        // Two session locks in sequence (contract: never both at once).
        // Playback first so the volume path lands with `replaygain_enabled`
        // already set; library second for paths, statuses, watch states, and
        // UI flags.
        {
            let mut session = playback.lock_or_recover();
            if let Some(vol) = settings.scalars.volume {
                session.current_volume = vol;
            }
            session.replaygain_enabled = settings.scalars.replaygain_enabled;
            // Restore the player-bar toggles so shuffle/repeat survive restarts.
            session.queue.shuffle = settings.scalars.shuffle;
            session.queue.repeat = repeat_mode_from_store_code(settings.scalars.repeat_mode);
            // Route through effective_volume so a muted app (once mute
            // state is restored) never emits sound at startup.
            transport.set_volume(&session, session.effective_volume());
        }
        {
            let mut session = library.lock_or_recover();
            if !settings.library_paths.is_empty() {
                for path in &settings.library_paths {
                    let status = if path.exists() {
                        LibraryStatus::Idle
                    } else {
                        LibraryStatus::Unavailable
                    };
                    session.library_statuses.insert(path.clone(), status);
                }
                session.library_paths.clone_from(&settings.library_paths);
            }

            session.watch_states.clone_from(&settings.watch_states);

            session.ui_flags.advanced_mode = settings.scalars.advanced_mode;
            session.ui_flags.high_contrast = settings.scalars.high_contrast;
            session.browser_layout =
                BrowserLayout::from_store_code(settings.scalars.browser_layout);
            session.scan_prefs = ScanPrefs {
                skip_hidden_files: settings.scalars.skip_hidden_files,
                scan_formats: settings.scalars.scan_formats.clone(),
                read_embedded_artwork: settings.scalars.read_embedded_artwork,
                missing_artwork_strategy: settings.scalars.missing_artwork_strategy,
            };
        }

        // Seed the snapshot from the freshly hydrated sessions so the first
        // frame-end commit is a no-op. The locks are taken in sequence,
        // never nested: the playback guard is gone (cloned) before the
        // library guard is taken for the builder call.
        let playback_snapshot = playback.lock_or_recover().clone();
        let last_committed = scalar_settings(&playback_snapshot, &library.lock_or_recover());
        Self { last_committed }
    }

    /// Frame end: build the scalar row from the two sessions via the one
    /// canonical builder and save it only when it differs from the
    /// last-committed snapshot. A failed save warns and leaves the snapshot
    /// untouched, so the change stays pending and retries on a later frame.
    pub fn commit_if_changed(
        &mut self,
        playback: &PlaybackSession,
        library: &LibrarySession,
        store: &mut dyn SettingsStore,
    ) {
        let scalars = scalar_settings(playback, library);
        if scalars == self.last_committed {
            return;
        }
        if let Err(e) = store.save_scalars(&scalars) {
            tracing::warn!("Failed to save settings: {e}");
            return;
        }
        self.last_committed = scalars;
    }
}

/// The one canonical builder: the current sessions projected onto the
/// persisted scalar row. Both the hydrate snapshot and every diff-commit go
/// through it, so the two directions of the round-trip can never drift.
fn scalar_settings(playback: &PlaybackSession, library: &LibrarySession) -> ScalarSettings {
    ScalarSettings {
        volume: Some(playback.current_volume),
        advanced_mode: library.ui_flags.advanced_mode,
        high_contrast: library.ui_flags.high_contrast,
        replaygain_enabled: playback.replaygain_enabled,
        shuffle: playback.queue.shuffle,
        repeat_mode: repeat_mode_to_store_code(playback.queue.repeat),
        browser_layout: library.browser_layout.as_store_code(),
        skip_hidden_files: library.scan_prefs.skip_hidden_files,
        scan_formats: library.scan_prefs.scan_formats.clone(),
        read_embedded_artwork: library.scan_prefs.read_embedded_artwork,
        missing_artwork_strategy: library.scan_prefs.missing_artwork_strategy,
    }
}

/// `RepeatMode` → persisted scalar code: 0 = off, 1 = all, 2 = one.
fn repeat_mode_to_store_code(repeat: RepeatMode) -> i64 {
    match repeat {
        RepeatMode::None => 0,
        RepeatMode::All => 1,
        RepeatMode::One => 2,
    }
}

/// Persisted scalar code → `RepeatMode`; unknown values fall back to off so
/// a hand-edited store can never break the session.
fn repeat_mode_from_store_code(code: i64) -> RepeatMode {
    match code {
        1 => RepeatMode::All,
        2 => RepeatMode::One,
        _ => RepeatMode::None,
    }
}
