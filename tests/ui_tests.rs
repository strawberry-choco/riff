// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    // --- Settings persist through the Application Store port -------------------
    //
    // The settings surface saves each preference change straight to the
    // Application Store and hydrates from it on startup. These replacements
    // for the former eframe-storage tests drive the same port object type the
    // UI holds (`Box<dyn SettingsStore>`) over a real SQLite database,
    // dropping and reopening it to simulate a restart.

    use crate::integration_helpers::create_test_sessions;
    use riff_backend::app::store::{LibraryMutationStore, PlaylistStore, SettingsStore};
    use std::path::PathBuf;

    /// Open a real store-backed settings port at a fresh temp location,
    /// exactly as the UI receives it: a boxed `SettingsStore`.
    fn boxed_store(dir: &tempfile::TempDir) -> Box<dyn SettingsStore> {
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        Box::new(
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("opening a fresh store must work"),
        )
    }

    /// Open a real store-backed playlists port at a fresh temp location,
    /// exactly as the UI receives it: a boxed `PlaylistStore`.
    fn boxed_playlist_store(dir: &tempfile::TempDir) -> Box<dyn PlaylistStore> {
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        Box::new(
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("opening a fresh store must work"),
        )
    }

    /// A real store-backed playlists port plus a `SessionViews` seam sharing
    /// its session playlist generation — the same pairing the composition
    /// root wires, so commits through the port invalidate the seam's
    /// playlist projection exactly like production.
    fn boxed_playlist_seam(
        dir: &tempfile::TempDir,
    ) -> (
        Box<dyn PlaylistStore>,
        riff_backend::app::views::SessionViews,
    ) {
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");
        let views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );
        (Box::new(store), views)
    }

    /// Open a real store-backed library-mutations port at a fresh temp
    /// location, exactly as the UI receives it: a boxed
    /// `LibraryMutationStore`.
    pub(super) fn boxed_library_store(dir: &tempfile::TempDir) -> Box<dyn LibraryMutationStore> {
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        Box::new(
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("opening a fresh store must work"),
        )
    }

    /// A `SessionViews` seam over the store already living at `dir`, for
    /// reading playlists the way the UI does.
    fn seam_views(dir: &tempfile::TempDir) -> riff_backend::app::views::SessionViews {
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");
        riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        )
    }

    // --- Playlists persist through the Application Store port ------------------
    //
    // The playlists surface commits every mutation straight to the
    // Application Store and hydrates from it on startup. This test drives the
    // same port object type the UI holds (`Box<dyn PlaylistStore>`) over a
    // real SQLite database, dropping and reopening it to simulate a restart.

    #[test]
    fn test_playlist_mutations_roundtrip_through_the_store_across_restart() {
        let dir = tempfile::tempdir().unwrap();

        // Fresh store: no playlists.
        assert!(
            boxed_playlist_store(&dir)
                .load_playlists()
                .unwrap()
                .is_empty()
        );

        // Create + edit, then drop the connection (the "restart").
        let pid;
        {
            let mut store = boxed_playlist_store(&dir);
            pid = store.create_playlist("Gym", &[]).unwrap();
            assert!(
                store
                    .add_playlist_entry(&pid, &TrackId("hype.mp3".to_string()))
                    .unwrap()
            );
            assert!(store.rename_playlist(&pid, "Workout").unwrap());
        }

        // Reopen: name, entries, and order all survived.
        let reopened = boxed_playlist_store(&dir);
        let playlists = reopened.load_playlists().unwrap();
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].id, pid);
        assert_eq!(playlists[0].name, "Workout");
        assert_eq!(playlists[0].tracks, vec![TrackId("hype.mp3".to_string())]);

        // Delete commits instantly too.
        {
            let mut store = boxed_playlist_store(&dir);
            assert!(store.delete_playlist(&pid).unwrap());
        }
        assert!(
            boxed_playlist_store(&dir)
                .load_playlists()
                .unwrap()
                .is_empty()
        );
    }

    // --- Library collection cutover (ticket 05) --------------------------------
    //
    // The Library collection lives solely in the Application Store and is
    // read live through the `LibraryQueryStore` port; there is no startup
    // hydration of an in-memory copy. The legacy JSON cache is never read
    // or written and stays untouched on disk.

    #[test]
    fn test_legacy_json_cache_is_never_read_or_written() {
        let dir = tempfile::tempdir().unwrap();
        // A corrupt legacy cache sits next to the store; first-frame restore
        // must ignore it entirely — the store is the only source.
        let legacy_path = dir.path().join("library_cache.json");
        std::fs::write(&legacy_path, "{{{ corrupt legacy json").unwrap();
        let legacy_bytes_before = std::fs::read(&legacy_path).unwrap();

        let (playback, library) = create_test_sessions();
        riff_backend::app::preferences::Preferences::hydrate(
            &playback,
            &library,
            boxed_store(&dir).as_ref(),
            &crate::mocks::MockTransport::new(),
        );

        assert_eq!(
            std::fs::read(&legacy_path).unwrap(),
            legacy_bytes_before,
            "the legacy JSON file remains byte-for-byte untouched"
        );
    }

    #[test]
    fn test_volume_roundtrips_through_the_store_across_restart() {
        let dir = tempfile::tempdir().unwrap();

        // Fresh store: volume unset.
        assert_eq!(
            boxed_store(&dir).load_settings().unwrap().scalars.volume,
            None
        );

        // Change the volume, then drop the connection (the "restart").
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    volume: Some(0.75),
                    ..Default::default()
                })
                .expect("saving scalars must work");
        }

        // Reopen: the value survived in its typed column.
        assert_eq!(
            boxed_store(&dir).load_settings().unwrap().scalars.volume,
            Some(0.75)
        );
    }

    #[test]
    fn test_advanced_mode_roundtrips_and_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();

        // Fresh store: advanced mode off.
        assert!(
            !boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .advanced_mode
        );

        // Turning it on survives a restart...
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    advanced_mode: true,
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .advanced_mode
        );

        // ...and turning it back off does too.
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    advanced_mode: false,
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            !boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .advanced_mode
        );
    }

    #[test]
    fn test_high_contrast_roundtrips_and_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();

        // Fresh store: high contrast off.
        assert!(
            !boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .high_contrast
        );

        // Turning it on survives a restart...
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    high_contrast: true,
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .high_contrast
        );

        // ...and turning it back off does too.
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    high_contrast: false,
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            !boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .high_contrast
        );
    }

    #[test]
    fn test_library_scan_prefs_hydrate_into_the_library_session_on_restore() {
        use riff_backend::app::state::ScanPrefs;

        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = boxed_store(&dir);
            let scalars = riff_backend::app::state::ScalarSettings {
                skip_hidden_files: false,
                scan_formats: vec!["flac".to_string(), "mp3".to_string()],
                read_embedded_artwork: false,
                ..Default::default()
            };
            store.save_scalars(&scalars).unwrap();
        }

        let (playback, library) = create_test_sessions();
        riff_backend::app::preferences::Preferences::hydrate(
            &playback,
            &library,
            boxed_store(&dir).as_ref(),
            &crate::mocks::MockTransport::new(),
        );

        assert_eq!(
            library.lock_or_recover().scan_prefs,
            ScanPrefs {
                skip_hidden_files: false,
                scan_formats: vec!["flac".to_string(), "mp3".to_string()],
                read_embedded_artwork: false,
            },
            "first-frame restore must hydrate the Library Scan preferences into the session"
        );
    }

    #[test]
    fn test_library_scan_prefs_default_to_the_historical_scanner_behavior() {
        use riff_backend::app::state::ScanPrefs;

        let dir = tempfile::tempdir().unwrap();
        let (playback, library) = create_test_sessions();
        riff_backend::app::preferences::Preferences::hydrate(
            &playback,
            &library,
            boxed_store(&dir).as_ref(),
            &crate::mocks::MockTransport::new(),
        );

        let prefs = library.lock_or_recover().scan_prefs.clone();
        assert!(prefs.skip_hidden_files);
        assert!(prefs.read_embedded_artwork);
        assert_eq!(prefs.scan_formats, ScanPrefs::default().scan_formats);
    }

    #[test]
    fn test_replaygain_roundtrips_and_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();

        // Fresh store: ReplayGain off.
        assert!(
            !boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .replaygain_enabled
        );

        // Turning it on survives a restart...
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    replaygain_enabled: true,
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .replaygain_enabled
        );

        // ...and turning it back off does too.
        {
            let mut store = boxed_store(&dir);
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    replaygain_enabled: false,
                    ..Default::default()
                })
                .unwrap();
        }
        assert!(
            !boxed_store(&dir)
                .load_settings()
                .unwrap()
                .scalars
                .replaygain_enabled
        );
    }

    #[test]
    fn test_library_paths_roundtrip_through_the_store_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![
            std::path::PathBuf::from("path1"),
            std::path::PathBuf::from("path2"),
        ];

        // Register the paths, then drop the connection (the "restart").
        {
            let mut store = boxed_store(&dir);
            store.save_library_paths(&paths).unwrap();
        }

        // Reopen: the list survived in registration order.
        let reloaded = boxed_store(&dir).load_settings().unwrap();
        assert_eq!(reloaded.library_paths, paths);
    }

    #[test]
    fn test_watch_states_roundtrip_through_the_store_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut states = std::collections::HashMap::new();
        states.insert(std::path::PathBuf::from("path1"), WatchState::Enabled);

        // Persist the watch state, then drop the connection (the "restart").
        {
            let mut store = boxed_store(&dir);
            store.save_watch_states(&states).unwrap();
        }

        // Reopen: the state survived.
        let reloaded = boxed_store(&dir).load_settings().unwrap();
        assert_eq!(reloaded.watch_states, states);
    }

    // (REQ-UI-007) high-contrast coverage now lives in the theme-token tests
    // below (`test_high_contrast_style_keeps_focus_unmistakable`), which
    // checks the focus guarantees over both palettes.

    // --- Pure UI helpers (seek clamp, duration formatting) -------------------

    #[test]
    fn test_clamp_seek_within_bounds_passes_through() {
        assert_eq!(
            clamp_seek(45.5, Some(std::time::Duration::from_secs(245))),
            std::time::Duration::from_secs_f32(45.5)
        );
    }

    #[test]
    fn test_clamp_seek_past_end_clamps_to_total() {
        assert_eq!(
            clamp_seek(999.0, Some(std::time::Duration::from_secs(245))),
            std::time::Duration::from_secs(245)
        );
    }

    #[test]
    fn test_clamp_seek_negative_clamps_to_zero() {
        assert_eq!(
            clamp_seek(-5.0, Some(std::time::Duration::from_secs(245))),
            std::time::Duration::ZERO
        );
    }

    #[test]
    fn test_clamp_seek_unknown_total_falls_back_to_start() {
        assert_eq!(clamp_seek(30.0, None), std::time::Duration::ZERO);
    }

    #[test]
    fn test_clamp_seek_non_finite_falls_back_to_start() {
        let total = Some(std::time::Duration::from_secs(245));
        assert_eq!(clamp_seek(f32::NAN, total), std::time::Duration::ZERO);
        assert_eq!(clamp_seek(f32::INFINITY, total), std::time::Duration::ZERO);
    }

    #[test]
    fn test_format_duration_minutes_seconds() {
        assert_eq!(format_duration(std::time::Duration::from_secs(0)), "00:00");
        assert_eq!(format_duration(std::time::Duration::from_secs(65)), "01:05");
        // Minutes accumulate past an hour (no hour segment).
        assert_eq!(
            format_duration(std::time::Duration::from_secs(3723)),
            "62:03"
        );
    }

    // --- Linux folder-picker helpers (pure, platform-independent) ---------------

    /// Serializes the tests that mutate the process-global `HOME` env var so
    /// they cannot race each other (tests run in parallel threads).
    static HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Run `body` with `HOME` set to `value` (or unset when `None`), restoring
    /// the original value afterwards. Caller must hold `HOME_LOCK`.
    fn with_home(value: Option<&str>, body: impl FnOnce()) {
        let original = std::env::var("HOME").ok();
        match value {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            // FIXME: Audit that the environment access only happens in single-threaded code.
            None => unsafe { std::env::remove_var("HOME") },
        }
        body();
        match original {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            // FIXME: Audit that the environment access only happens in single-threaded code.
            None => unsafe { std::env::remove_var("HOME") },
        }
    }

    #[test]
    fn test_expand_tilde_passes_through_without_leading_tilde() {
        assert_eq!(
            expand_tilde("/usr/share/music"),
            std::path::PathBuf::from("/usr/share/music")
        );
        assert_eq!(
            expand_tilde("relative/dir"),
            std::path::PathBuf::from("relative/dir")
        );
        // A `~` that is not the leading segment is left alone.
        assert_eq!(
            expand_tilde("/music/~band/live"),
            std::path::PathBuf::from("/music/~band/live")
        );
        assert_eq!(expand_tilde(""), std::path::PathBuf::from(""));
    }

    #[test]
    fn test_expand_tilde_expands_leading_tilde_against_home() {
        let _guard = HOME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        with_home(Some("/fake/home"), || {
            // Build the expectation with `join` so the platform separator
            // matches whatever `expand_tilde` produces.
            let expected = std::path::PathBuf::from("/fake/home").join("Music");
            assert_eq!(expand_tilde("~/Music"), expected);
            assert_eq!(expand_tilde("~"), std::path::PathBuf::from("/fake/home"));
        });
    }

    /// `~/Music` means the same thing to the input's suggestions, its Confirm,
    /// and the session's stored root.
    #[test]
    fn test_a_tilde_candidate_registers_the_expanded_root() {
        let _guard = HOME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("a scratch directory");
        let music = dir.path().join("home-music");
        std::fs::create_dir(&music).expect("a scratch directory");
        let mut library = riff_backend::app::state::LibrarySession::default();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut store =
            crate::mocks::MockSettingsStore::with_shared_calls(std::sync::Arc::clone(&calls));

        with_home(
            Some(dir.path().to_str().expect("a utf-8 scratch path")),
            || {
                riff_gui::ui::app::register_library_path("~/home-music", &mut library, &mut store)
                    .expect("the tilde path resolves to a real directory");
            },
        );

        assert_eq!(
            library.library_paths.paths(),
            &[music.canonicalize().expect("the canonical root")],
            "the tilde candidate registered as its expanded, canonical root"
        );
    }

    #[test]
    fn test_expand_tilde_passes_through_when_home_unset() {
        let _guard = HOME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        with_home(None, || {
            assert_eq!(expand_tilde("~/Music"), std::path::PathBuf::from("~/Music"));
            assert_eq!(expand_tilde("~"), std::path::PathBuf::from("~"));
        });
    }

    #[test]
    fn test_suggest_directories_matches_prefix_case_insensitively_and_excludes_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("Music")).unwrap();
        std::fs::create_dir_all(root.path().join("music videos")).unwrap();
        std::fs::create_dir_all(root.path().join("Pictures")).unwrap();
        // A file matching the prefix must NOT be suggested (dirs only).
        std::fs::write(root.path().join("mu-notes.txt"), b"x").unwrap();

        let input = format!("{}/mu", root.path().display());
        let suggestions = suggest_directories(&input, 8);

        let names: Vec<String> = suggestions
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Sorted (byte order: "Music" < "music videos"), deduped, dirs only.
        assert_eq!(names, vec!["Music", "music videos"]);
    }

    #[test]
    fn test_suggest_directories_respects_max_cap() {
        let root = tempfile::tempdir().unwrap();
        for name in ["alpha", "also", "another", "zebra"] {
            std::fs::create_dir_all(root.path().join(name)).unwrap();
        }

        let input = format!("{}/a", root.path().display());
        let suggestions = suggest_directories(&input, 2);

        let names: Vec<String> = suggestions
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Three dirs match "a"; the cap keeps the first two in sort order.
        assert_eq!(names, vec!["alpha", "also"]);
    }

    #[test]
    fn test_suggest_directories_lists_children_when_input_ends_with_separator() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("sub")).unwrap();
        std::fs::create_dir_all(root.path().join("sub2")).unwrap();

        let input = format!("{}/", root.path().display());
        let suggestions = suggest_directories(&input, 8);

        let names: Vec<String> = suggestions
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // Trailing separator => list that directory's children, unfiltered.
        assert_eq!(names, vec!["sub", "sub2"]);
    }

    #[test]
    fn test_suggest_directories_empty_for_nonexistent_parent() {
        let suggestions = suggest_directories("/definitely/not/here/xyz", 8);
        assert!(suggestions.is_empty());
    }

    // --- cover-cache LRU helper ---------------------------------------------------

    #[test]
    fn test_lru_insert_dedupes_by_moving_key_to_most_recent_end() {
        let mut keys = Vec::new();
        assert!(lru_insert(&mut keys, "a".to_string(), 3).is_empty());
        assert!(lru_insert(&mut keys, "b".to_string(), 3).is_empty());
        assert!(lru_insert(&mut keys, "c".to_string(), 3).is_empty());

        // Re-inserting "a" moves it to the end instead of duplicating it.
        assert!(lru_insert(&mut keys, "a".to_string(), 3).is_empty());
        assert_eq!(
            keys,
            vec!["b".to_string(), "c".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn test_lru_insert_evicts_oldest_beyond_cap_in_fifo_order() {
        let mut keys = Vec::new();
        for k in ["a", "b", "c"] {
            lru_insert(&mut keys, k.to_string(), 2);
        }
        // Cap 2 keeps only the two most recent keys...
        assert_eq!(keys, vec!["b".to_string(), "c".to_string()]);
        // ...and the next insert evicts the oldest survivor ("b").
        let evicted = lru_insert(&mut keys, "d".to_string(), 2);
        assert_eq!(evicted, vec!["b".to_string()]);
        assert_eq!(keys, vec!["c".to_string(), "d".to_string()]);
    }

    // --- Clear Library (ticket 10) -------------------------------------------
    //
    // After the maintenance wipe, the collection section is empty while
    // playlists and settings restore exactly as before — no special casing
    // anywhere in the UI layer.

    #[test]
    fn test_restore_after_clear_library_sees_empty_collection_and_kept_curation() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");

        // Seed a full store: collection + playlist + settings.
        {
            let (changes_tx, _changes_rx) =
                crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
            let mut store =
                riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
            store
                .apply_scan_batch(&[crate::test_utils::create_test_track_with_metadata(
                    "f:\\cl\\a.mp3",
                    "f:\\cl\\a.mp3",
                    "Artist",
                    "Title",
                    "Album",
                )])
                .unwrap();
            store.create_playlist("Keep Me", &[]).unwrap();
            store
                .save_scalars(&riff_backend::app::state::ScalarSettings {
                    volume: Some(0.5),
                    ..Default::default()
                })
                .unwrap();
            store.clear_library().expect("clear works");
        }

        let (playback, library) = create_test_sessions();
        riff_backend::app::preferences::Preferences::hydrate(
            &playback,
            &library,
            boxed_store(&dir).as_ref(),
            &crate::mocks::MockTransport::new(),
        );
        let mut views = seam_views(&dir);

        // The store kept the curation across the wipe...
        assert_eq!(
            boxed_playlist_store(&dir).load_playlists().unwrap().len(),
            1,
            "curation survived the wipe in the store"
        );

        // ...and the seam sees both facts: curation kept, collection empty.
        let playlists = views.playlists();
        assert_eq!(playlists.len(), 1, "playlists read back as usual");
        assert_eq!(playlists[0].name, "Keep Me");
        assert_eq!(
            views.track_list("", 0).total,
            0,
            "the wiped collection reads empty through the seam"
        );
    }

    // --- Theme token foundation (Issue 01) ------------------------------------
    //
    // Every literal asserted below is transcribed from the redesign's
    // design-token sheet (`colors_and_type.css`) — the independent source of
    // truth for the mockup palette. The dark palette must match it exactly.

    use riff_gui::ui::theme;

    #[test]
    fn test_dark_surface_and_ink_tokens_match_the_mockup() {
        // Surfaces: --riff-bg / --riff-surface / --riff-surface-2 /
        // --riff-surface-3.
        //
        // These are NO LONGER the design handoff's extracted hex literals
        // (#101013 / #17171b / #1e1e23 / #26262d). They are a deliberate
        // amendment to it, made in the same commit as this comment: the ramp
        // now steps 8/9/9 from a deeper base (was 7/7/8, which read as one
        // flat field) and carries a slight warm bias (red ≥ green ≥ blue) so
        // panels sit in the same system as the amber brand instead of reading
        // as cool gray behind it. Every surface went darker, so the ink ladder
        // below keeps — and slightly improves — its AA headroom: `ink_3` on
        // `surface_3` measures ~4.70:1 where the original pair measured
        // 4.63:1. The contrast test is what holds that floor; this test only
        // pins the numbers so a future drift has to be a decision.
        assert_eq!(theme::SURFACE_BG, egui::Color32::from_rgb(0x0e, 0x0d, 0x0c));
        assert_eq!(theme::SURFACE, egui::Color32::from_rgb(0x16, 0x15, 0x14));
        assert_eq!(theme::SURFACE_2, egui::Color32::from_rgb(0x1f, 0x1d, 0x1b));
        assert_eq!(theme::SURFACE_3, egui::Color32::from_rgb(0x28, 0x25, 0x21));

        // Ink ladder. `ink` is the mockup's; the two muted rungs are the
        // design handoff's hexes lifted until they clear AA, so the literals
        // here are the post-retune tokens and the contrast test is what says
        // why they are these numbers.
        assert_eq!(theme::INK, egui::Color32::from_rgb(0xed, 0xed, 0xf0));
        assert_eq!(theme::INK_2, egui::Color32::from_rgb(0xa8, 0xa8, 0xb4));
        assert_eq!(theme::INK_3, egui::Color32::from_rgb(0x8e, 0x8e, 0x9a));
    }

    #[test]
    fn test_dark_line_tokens_match_the_mockup_alphas() {
        // --riff-line: rgba(255,255,255,0.08); --riff-border: rgba(255,255,255,0.10).
        // Alpha bytes are the CSS alphas scaled to u8 (0.08*255 ≈ 20, 0.10*255 ≈ 26).
        assert_eq!(
            theme::LINE,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20)
        );
        assert_eq!(
            theme::BORDER,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 26)
        );
    }

    #[test]
    fn test_brand_amber_scale_matches_the_mockup() {
        // --riff-brand-50 … --riff-brand-700; 500 is the primary.
        assert_eq!(theme::BRAND_50, egui::Color32::from_rgb(0xff, 0xf8, 0xe7));
        assert_eq!(theme::BRAND_100, egui::Color32::from_rgb(0xff, 0xef, 0xcc));
        assert_eq!(theme::BRAND_200, egui::Color32::from_rgb(0xff, 0xe0, 0x99));
        assert_eq!(theme::BRAND_300, egui::Color32::from_rgb(0xff, 0xcc, 0x66));
        assert_eq!(theme::BRAND_400, egui::Color32::from_rgb(0xff, 0xb8, 0x33));
        assert_eq!(theme::BRAND_500, egui::Color32::from_rgb(0xf0, 0x82, 0x1e));
        assert_eq!(theme::BRAND_600, egui::Color32::from_rgb(0xd9, 0x8a, 0x0d));
        assert_eq!(theme::BRAND_700, egui::Color32::from_rgb(0xa6, 0x67, 0x09));
    }

    #[test]
    fn test_status_color_tokens_hold_their_own_hues() {
        // --riff-state-success/error/info are the mockup's. --riff-state-warning
        // was brand-500, which is how amber came to mean "primary action",
        // "playback progress", "keyboard focus" and "this field differs" at the
        // same time (review P2-17); it is now a yellow of its own.
        assert_eq!(
            theme::STATE_SUCCESS,
            egui::Color32::from_rgb(0x22, 0xc5, 0x5e)
        );
        assert_eq!(
            theme::STATE_WARNING,
            egui::Color32::from_rgb(0xea, 0xb3, 0x08)
        );
        assert_eq!(theme::STATE_INFO, egui::Color32::from_rgb(0x3b, 0x82, 0xf6));
        assert_ne!(theme::STATE_WARNING, theme::BRAND_500);
        // `error` is deliberately no longer the mockup's `#ef4444`: the slot
        // paints text, and that value read 3.99:1 on the raised fill, so each
        // family now wears its own end of the red ramp. What the test still
        // holds both ends to is the claim its name makes — the red stays the
        // red, and stays its own hue instead of borrowing amber or blue —
        // because a value that keeps missing a contrast floor will be re-picked
        // again, and the hue is the part that has to survive that.
        assert_eq!(
            theme::STATE_ERROR,
            egui::Color32::from_rgb(0xff, 0x52, 0x52)
        );
        assert_eq!(
            theme::STATE_ERROR_LIGHT,
            egui::Color32::from_rgb(0x99, 0x1b, 0x1b)
        );
        for error in [theme::STATE_ERROR, theme::STATE_ERROR_LIGHT] {
            // Hue 0 exactly: red is the largest channel and green equals blue.
            // That is what puts both ends of the ramp on one hue, so the light
            // and dark values differ in lightness — the same colour, legibly
            // — rather than in hue, which is what "one semantic role" means.
            assert!(
                error.r() > error.g() && error.g() == error.b(),
                "{error:?} is off the pure-red hue the error role is held to"
            );
        }
        // ...and the ring has its own token, on its own hue.
        assert_eq!(theme::FOCUS_RING, egui::Color32::from_rgb(0xa7, 0x8b, 0xfa));
        assert_ne!(theme::FOCUS_RING, theme::BRAND_500);
        assert_ne!(theme::FOCUS_RING, theme::STATE_WARNING);
    }

    #[test]
    fn test_spacing_scale_steps_are_the_values_the_views_already_used() {
        // The scale was declared from the numbers in the views' own
        // `item_spacing` assignments and gap geometry, so adopting it moved
        // values rather than changing them. Pinned like the radius and brand
        // ramps: a step that drifts is a restyle, not a refactor.
        assert!((theme::SPACE_XS - 4.0).abs() < f32::EPSILON);
        assert!((theme::SPACE_SM - 6.0).abs() < f32::EPSILON);
        assert!((theme::SPACE_MD - 8.0).abs() < f32::EPSILON);
        assert!((theme::SPACE_LG - 12.0).abs() < f32::EPSILON);
        assert!((theme::SPACE_XL - 16.0).abs() < f32::EPSILON);
        assert!((theme::SPACE_XXL - 24.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_radius_scale_constants_match_the_mockup() {
        // --riff-radius-sm/md/lg/xl/full. Lifted 2 px per step off the
        // extracted mockup scale (4/8/12/16): the mockup's small controls were
        // nearly square next to 12–14 px type. A radius moves no box edge, so
        // this is the one generosity lever the pinned chrome geometry has no
        // say in — pinned here so the new numbers are a decision, not drift.
        assert!((theme::RADIUS_SM - 6.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_MD - 10.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_LG - 14.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_XL - 18.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_FULL - 999.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_chrome_dimension_constants_match_the_mockup() {
        // --riff-titlebar-h / --riff-sidebar-w / --riff-playerbar-h.
        assert!((theme::TITLEBAR_H - 56.0).abs() < f32::EPSILON);
        assert!((theme::SIDEBAR_W - 280.0).abs() < f32::EPSILON);
        assert!((theme::PLAYERBAR_H - 88.0).abs() < f32::EPSILON);
        // The elastic column stage's sizing tokens: the 280px entity column
        // preferred width, the 200px / 320px floors, and the 300px inspector
        // (the former selection panel's width).
        assert!((theme::COLUMN_WIDTH - 280.0).abs() < f32::EPSILON);
        assert!((theme::COLUMN_MIN_W - 200.0).abs() < f32::EPSILON);
        assert!((theme::LAST_COLUMN_MIN_W - 320.0).abs() < f32::EPSILON);
        assert!((theme::INSPECTOR_WIDTH - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_dark_palette_binds_the_mockup_tokens_to_semantic_roles() {
        let p = theme::Palette::dark();

        assert!(p.dark);
        assert_eq!(p.background, theme::SURFACE_BG);
        assert_eq!(p.surface, theme::SURFACE);
        assert_eq!(p.surface_2, theme::SURFACE_2);
        assert_eq!(p.surface_3, theme::SURFACE_3);
        assert_eq!(p.ink, theme::INK);
        assert_eq!(p.ink_2, theme::INK_2);
        assert_eq!(p.ink_3, theme::INK_3);
        assert_eq!(p.line, theme::LINE);
        assert_eq!(p.border, theme::BORDER);
        // --riff-primary is brand-500 and --riff-primary-foreground the deep ink
        // text on it. The ring is its own token now: an amber ring read as a
        // primary action everywhere else it appears (review P2-17).
        assert_eq!(p.brand_primary, theme::BRAND_500);
        assert_eq!(p.focus_ring, theme::FOCUS_RING);
        assert_ne!(p.focus_ring, p.brand_primary);
        assert_eq!(p.on_brand, egui::Color32::from_rgb(0x0e, 0x0d, 0x0c));
        assert_eq!(p.success, theme::STATE_SUCCESS);
        assert_eq!(p.warning, theme::STATE_WARNING);
        assert_eq!(p.error, theme::STATE_ERROR);
        assert_eq!(p.info, theme::STATE_INFO);
    }

    #[test]
    fn test_row_hover_uses_the_design_amber_wash() {
        // The design's row hover is a literal amber wash (#2a1c0e), extracted
        // from the highlighted sidebar/track rows in the source SVGs — a
        // brand-derived tint, not a surface-ramp step.
        assert_eq!(theme::ROW_HOVER, egui::Color32::from_rgb(0x2a, 0x1c, 0x0e));

        // Dark palette binds the wash verbatim.
        let dark = theme::Palette::dark();
        assert_eq!(dark.row_hover, theme::ROW_HOVER);

        // The light family derives its wash by rule (ADR 0004): the unchanged
        // brand amber at the ~9% coverage the dark wash reads over its
        // surface, so the hover stays a warm amber tint on both palettes.
        let light = theme::Palette::light();
        assert_eq!(
            light.row_hover,
            egui::Color32::from_rgba_unmultiplied(0xf0, 0x82, 0x1e, 24)
        );
    }

    #[test]
    fn test_light_palette_is_derived_from_dark_by_rule() {
        // ADR 0004: surfaces invert (channel-wise mirror), ink flips, brand
        // amber unchanged. `mirror` restates that rule independently here.
        fn mirror(c: egui::Color32) -> egui::Color32 {
            egui::Color32::from_rgb(255 - c.r(), 255 - c.g(), 255 - c.b())
        }
        fn lum(c: egui::Color32) -> u32 {
            u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b())
        }
        let dark = theme::Palette::dark();
        let light = theme::Palette::light();

        assert!(!light.dark);

        // Surfaces invert: worked example first (#0e0d0c → #f1f2f3 — note the
        // warm bias mirrors to a cool one, per the rule), then the rule for
        // the remaining ramp.
        assert_eq!(light.background, egui::Color32::from_rgb(0xf1, 0xf2, 0xf3));
        assert_eq!(light.surface_row, mirror(dark.surface_row));
        assert_eq!(light.surface, mirror(dark.surface));
        assert_eq!(light.surface_2, mirror(dark.surface_2));
        assert_eq!(light.surface_3, mirror(dark.surface_3));
        // The ramp order flips: dark bg is the darkest step, light bg the
        // lightest. `surface_row` is a step, not a beside: it sits between the
        // background and the card plane in both families.
        assert!(
            lum(dark.background) < lum(dark.surface_row)
                && lum(dark.surface_row) < lum(dark.surface)
                && lum(dark.surface) < lum(dark.surface_2)
                && lum(dark.surface_2) < lum(dark.surface_3)
        );
        assert!(
            lum(light.background) > lum(light.surface_row)
                && lum(light.surface_row) > lum(light.surface)
                && lum(light.surface) > lum(light.surface_2)
                && lum(light.surface_2) > lum(light.surface_3)
        );

        // Ink flips while preserving the faintness hierarchy: on dark the
        // primary ink is brightest; on light it is darkest. `ink` mirrors; the
        // muted rungs do not, because the channel-wise flip of an AA-compliant
        // dark gray lands under 4.5:1 on a light panel — light's surfaces need
        // darker text, not inverted text. They are chosen against the light
        // ramp, which the contrast test enforces.
        assert_eq!(light.ink, egui::Color32::from_rgb(0x12, 0x12, 0x0f));
        assert_eq!(light.ink_2, egui::Color32::from_rgb(0x47, 0x47, 0x40));
        assert_eq!(light.ink_3, egui::Color32::from_rgb(0x5e, 0x5e, 0x55));
        assert_ne!(light.ink_2, mirror(dark.ink_2));
        assert_ne!(light.ink_3, mirror(dark.ink_3));
        assert!(lum(dark.ink) > lum(dark.ink_2) && lum(dark.ink_2) > lum(dark.ink_3));
        assert!(lum(light.ink) < lum(light.ink_2) && lum(light.ink_2) < lum(light.ink_3));

        // Lines flip their base white→black but keep the same alphas.
        assert_eq!((light.line.r(), light.line.g(), light.line.b()), (0, 0, 0));
        assert_eq!(light.line.a(), dark.line.a());
        assert_eq!(light.border.a(), dark.border.a());

        // Brand amber is identical across palettes. The status colors mostly
        // are too, except the two that paint text on a panel — warning is a
        // bright yellow on dark and a deep amber on light, where a bright one
        // reads at 1.25:1; error is a bright red on dark and the deep end of the
        // same red on light, where the bright one reads at 3.06:1. `success` and
        // `info` still inherit unchanged, which is the rule holding rather than
        // an oversight: both are only ever an 8 px readiness dot, never a glyph,
        // so the text floor does not reach them. The ring follows the same rule
        // as a UI component: light needs the darker violet to clear 3:1 against
        // its own surfaces.
        assert_eq!(light.brand_primary, dark.brand_primary);
        assert_eq!(light.brand_primary, theme::BRAND_500);
        assert_eq!(light.success, dark.success);
        assert_eq!(light.warning, egui::Color32::from_rgb(0x85, 0x4d, 0x0e));
        assert_ne!(light.warning, dark.warning);
        assert_eq!(light.error, theme::STATE_ERROR_LIGHT);
        assert_ne!(light.error, dark.error);
        assert_eq!(light.info, dark.info);
        assert_eq!(light.focus_ring, egui::Color32::from_rgb(0x6d, 0x28, 0xd9));
        assert_ne!(light.focus_ring, light.brand_primary);
        // Dark text stays correct on the unchanged amber fill.
        assert_eq!(light.on_brand, dark.on_brand);
    }

    // --- The music-icon placeholder tile (shared, themed) ----------------------
    //
    // Tracks and albums without artwork render the shared placeholder tile —
    // a surface well with a music glyph — instead of a per-item colour
    // block. Nothing derives per identity anymore: every artless item
    // resolves the SAME tile, which only answers to the active palette
    // family (`surface_2` well, `ink_3` glyph).

    #[test]
    fn test_placeholder_tile_is_shared_across_identities() {
        use riff_gui::ui::cover_placeholder::lookup_cover_texture;
        use riff_gui::ui::theme::Palette;
        use riff_library::app::traits::RequestedSize;

        let ctx = placeholder_ctx();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();
        let thumb = RequestedSize {
            width: 56,
            height: 56,
        };

        // The removed colour block was the only per-identity input, so two
        // artless items resolve to one shared cached tile.
        let a = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            "a.mp3",
            thumb,
        );
        let b = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            "b.mp3",
            thumb,
        );
        assert!(a == b, "one tile serves every artless item");

        // Real art is one texture per requested size, but the tile is scaled
        // at draw time, so a miss answers with the same tile at any box.
        let hero = RequestedSize {
            width: 512,
            height: 512,
        };
        let c = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            "a.mp3",
            hero,
        );
        assert!(
            a == c,
            "the shared tile answers a miss at a size it was not first resolved at"
        );
    }

    #[test]
    fn test_placeholder_tile_follows_the_palette_family() {
        use riff_gui::ui::cover_placeholder::placeholder_image;
        use riff_gui::ui::theme::Palette;

        // The derivation answers to the active palette: the well and glyph
        // colours come from `surface_2` / `ink_3`, which differ across the
        // families, so dark and light tiles must differ.
        assert_ne!(
            placeholder_image(&Palette::dark()).pixels,
            placeholder_image(&Palette::light()).pixels,
            "dark and light tiles must differ"
        );
    }

    // --- The placeholder tile flows through the shared texture cache ----------
    //
    // A texture miss resolves the shared tile into the SAME map + LRU the
    // real covers use (keyed by a reserved key, so the request flow keeps
    // treating the track as artless), real art always wins over the tile,
    // and eviction paths touch only the tile.

    fn placeholder_ctx() -> egui::Context {
        egui::Context::default()
    }

    fn real_texture(ctx: &egui::Context, name: &str) -> egui::TextureHandle {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [2, 2],
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
        );
        ctx.load_texture(name, image, egui::TextureOptions::default())
    }

    #[test]
    fn test_lookup_caches_the_shared_placeholder_tile_on_a_full_miss() {
        use riff_gui::ui::app::cover_cache_key;
        use riff_gui::ui::cover_placeholder::{
            lookup_cover_texture, placeholder_cache_key, placeholder_image,
        };
        use riff_gui::ui::theme::Palette;
        use riff_library::app::traits::RequestedSize;

        let ctx = placeholder_ctx();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();
        let identity = "f:\\music\\artless.mp3";
        let size = RequestedSize {
            width: 56,
            height: 56,
        };
        let tile_key = placeholder_cache_key();

        let tile = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            identity,
            size,
        );

        // The tile lands in the shared cache under its size-independent key,
        // and the miss itself is never filed under the track's own key — that
        // is what keeps real art able to win later.
        assert!(
            textures.contains_key(&tile_key),
            "the tile is cached for reuse"
        );
        assert!(
            !textures.contains_key(&cover_cache_key(identity, size)),
            "an artless miss does not occupy the track's own key"
        );
        assert!(
            lru_keys.contains(&tile_key),
            "the tile rides the shared LRU"
        );

        // ...is the composed placeholder: the surface well with the music
        // glyph tinted ink_3 (pixel truth checked at the pure seam, where
        // pixels are readable)...
        let image = placeholder_image(&Palette::dark());
        let well = Palette::dark().surface_2;
        assert_eq!(
            image.pixels[0], well,
            "the tile's first pixel is the well fill"
        );
        assert!(
            image.pixels.iter().any(|p| *p != well),
            "the tile carries the music glyph beyond the well fill"
        );

        // ...and repeat lookups reuse it instead of regenerating.
        let again = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            identity,
            size,
        );
        assert!(tile == again, "the cached tile is served, not rebuilt");
    }

    #[test]
    fn test_real_art_wins_over_the_placeholder_tile() {
        use riff_gui::ui::app::cover_cache_key;
        use riff_gui::ui::cover_placeholder::lookup_cover_texture;
        use riff_gui::ui::theme::Palette;
        use riff_library::app::traits::RequestedSize;

        let ctx = placeholder_ctx();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();
        let identity = "f:\\music\\artful.mp3";
        let size = RequestedSize {
            width: 56,
            height: 56,
        };

        // The tile is cached first (the artless window)...
        let _ = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            identity,
            size,
        );

        // ...then real art arrives through the poll path under the track's
        // own (identity, size) key.
        let art = real_texture(&ctx, identity);
        textures.insert(cover_cache_key(identity, size), art.clone());
        lru_keys.push(cover_cache_key(identity, size));

        let resolved = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            identity,
            size,
        );
        assert!(resolved == art, "real art wins over the placeholder tile");

        // The win is per size: a box that never received art still misses.
        let hero = RequestedSize {
            width: 512,
            height: 512,
        };
        let other_size = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            identity,
            hero,
        );
        assert!(
            other_size != art,
            "art cached at one size must not answer a request at another"
        );
    }

    #[test]
    fn test_evict_generated_removes_only_the_placeholder_tile() {
        use riff_gui::ui::app::cover_cache_key;
        use riff_gui::ui::cover_placeholder::{
            evict_generated, lookup_cover_texture, placeholder_cache_key,
        };
        use riff_gui::ui::theme::Palette;
        use riff_library::app::traits::RequestedSize;

        let ctx = placeholder_ctx();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();
        let size = RequestedSize {
            width: 56,
            height: 56,
        };

        let _ = lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            "a.mp3",
            size,
        );
        let art = real_texture(&ctx, "c.mp3");
        let art_key = cover_cache_key("c.mp3", size);
        textures.insert(art_key.clone(), art);
        lru_keys.push(art_key.clone());

        evict_generated(&mut textures, &mut lru_keys);

        assert!(
            !textures.contains_key(&placeholder_cache_key()),
            "the tile leaves the cache"
        );
        assert!(
            !lru_keys.contains(&placeholder_cache_key()),
            "the LRU list drops the tile"
        );
        assert!(
            textures.contains_key(&art_key),
            "real covers survive the eviction"
        );
        assert!(lru_keys.contains(&art_key));
    }

    // --- The artwork presentation primitive (component-layer issue 13) --------
    //
    // One primitive paints real art and the palette-aware placeholder for every
    // surface that shows cover art, under a fit the CALLER selects. The artwork
    // cache-key space and the shared placeholder tile live in that module
    // rather than in `ui::app`, which is what removes the app ↔ artwork
    // sibling-module cycle.

    /// Paint one artwork block over a flat background and return the frame.
    /// `build` supplies the block's props once the harness's own `Context` (and
    /// therefore its textures) exist.
    fn render_artwork_frame(
        canvas: egui::Vec2,
        palette: &theme::Palette,
        build: impl Fn(&mut egui::Ui, &theme::Palette),
    ) -> image::RgbaImage {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(canvas)
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                build(ui, palette);
            });
        harness.run();
        harness
            .render()
            .expect("the artwork block renders headlessly")
    }

    fn count_frame_pixels(image: &image::RgbaImage, color: egui::Color32) -> usize {
        image
            .pixels()
            .filter(|p| {
                p.0[0].abs_diff(color.r()) <= 4
                    && p.0[1].abs_diff(color.g()) <= 4
                    && p.0[2].abs_diff(color.b()) <= 4
            })
            .count()
    }

    /// A square texture stretched over a 200x50 block covers the whole block;
    /// the same texture letterboxed covers only the 50x50 it can keep. The
    /// expected counts are the block's own geometry, not a re-run of the
    /// primitive's arithmetic.
    #[test]
    fn test_artwork_fit_policy_stretches_or_letterboxes_the_texture() {
        use riff_gui::ui::artwork::{self, Artwork, Fit};
        use riff_gui::ui::theme::Palette;

        let palette = Palette::dark();
        let red = egui::Color32::from_rgb(200, 30, 30);
        let block = egui::vec2(200.0, 50.0);

        let red_pixels_for = |fit: Fit| {
            let frame = render_artwork_frame(egui::vec2(260.0, 90.0), &palette, |ui, pal| {
                let texture = ui.ctx().load_texture(
                    "riff artwork fit test",
                    egui::ColorImage::new([2, 2], vec![red; 4]),
                    egui::TextureOptions::default(),
                );
                artwork::paint(
                    ui.painter(),
                    pal,
                    &Artwork {
                        rect: egui::Rect::from_min_size(egui::pos2(30.0, 20.0), block),
                        texture: Some(texture.id()),
                        fit,
                        tint: theme::TEXTURE_TINT,
                        placeholder: None,
                        border: None,
                    },
                );
            });
            count_frame_pixels(&frame, red)
        };

        let stretched = red_pixels_for(Fit::Fill);
        let letterboxed = red_pixels_for(Fit::Contain { aspect: 1.0 });
        assert!(
            stretched > 9_500,
            "Fit::Fill stretches the texture over the whole 200x50 block, saw {stretched}"
        );
        assert!(
            (2_300..=2_700).contains(&letterboxed),
            "Fit::Contain keeps a square texture inside the 50px-tall block, saw {letterboxed}"
        );
    }

    /// A block with no texture is never a hole: the well fills it from the
    /// active palette, the glyph variant carries the music mark, and a palette
    /// change repaints it from the new tokens.
    #[test]
    fn test_artwork_placeholder_is_a_themed_well_not_a_hole() {
        use riff_gui::ui::artwork::{self, Artwork, Fit, Placeholder};
        use riff_gui::ui::icons::{Icon, IconCache};
        use riff_gui::ui::theme::Palette;

        let block = egui::vec2(80.0, 80.0);
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), block);

        let well_pixels = |palette: &Palette| {
            let frame = render_artwork_frame(egui::vec2(120.0, 120.0), palette, |ui, pal| {
                artwork::paint(
                    ui.painter(),
                    pal,
                    &Artwork {
                        rect,
                        texture: None,
                        fit: Fit::Fill,
                        tint: theme::TEXTURE_TINT,
                        placeholder: Some(Placeholder::Well { radius: 4.0 }),
                        border: None,
                    },
                );
            });
            count_frame_pixels(&frame, palette.surface_2)
        };

        // The block is 80x80 = 6,400 px on a 120x120 canvas, so well-colored
        // pixels can only come from the block itself being filled.
        let dark_well = well_pixels(&Palette::dark());
        assert!(
            (6_000..=6_600).contains(&dark_well),
            "a block with no texture paints the palette's well, never a hole: {dark_well} of 6,400"
        );

        // The same block under the light palette answers to the light tokens.
        let light_well = well_pixels(&Palette::light());
        assert!(
            (6_000..=6_600).contains(&light_well),
            "the well re-derives from the active palette, saw {light_well} light pixels"
        );

        // The glyph variant carries a music mark OVER its well: the well's own
        // color no longer fills the block.
        let glyphed =
            render_artwork_frame(egui::vec2(120.0, 120.0), &Palette::dark(), |ui, pal| {
                let mut cache = IconCache::new();
                let glyph = cache.texture(ui.ctx(), Icon::Music, 16.0, pal.ink_3);
                artwork::paint(
                    ui.painter(),
                    pal,
                    &Artwork {
                        rect,
                        texture: None,
                        fit: Fit::Fill,
                        tint: theme::TEXTURE_TINT,
                        placeholder: Some(Placeholder::GlyphWell {
                            radius: 4.0,
                            glyph,
                            inset: 6.0,
                        }),
                        border: None,
                    },
                );
            });
        let glyphed_well = count_frame_pixels(&glyphed, Palette::dark().surface_2);
        assert!(
            glyphed_well < dark_well - 200,
            "the glyph well carries the music mark, not just a flat block: {glyphed_well} vs {dark_well}"
        );
    }

    /// The cycle guard: `ui::cover_placeholder` used to `use crate::ui::app::…`
    /// for the very cache-key space the app module imported the placeholder
    /// back from. `ui::artwork` owns that key space now, so no sibling module
    /// reaches into `ui::app` for it.
    #[test]
    fn test_no_ui_module_imports_the_artwork_key_space_from_the_app_module() {
        const ARTWORK_KEY_SPACE: [&str; 5] = [
            "CoverCacheKey",
            "cover_cache_key",
            "COVER_THUMB",
            "COVER_CARD",
            "COVER_HERO",
        ];
        let outside_app: Vec<_> = ui_source_lines()
            .into_iter()
            .filter(|(path, _, _)| !path.display().to_string().contains("/ui/app"))
            .collect();
        let offenders = violations(
            |line| {
                line.contains("ui::app::")
                    && ARTWORK_KEY_SPACE.iter().any(|symbol| line.contains(symbol))
            },
            &outside_app,
        );
        assert!(
            offenders.is_empty(),
            "the artwork key space belongs to ui::artwork:\n{}",
            offenders.join("\n")
        );
    }

    /// The Clear Library confirmation presents its affirmative action the way
    /// the design presents a destructive one — the error ink of the shared
    /// button family, never the brand fill — and its copy says what survives.
    #[test]
    fn test_clear_library_confirmation_presents_its_action_as_destructive() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::prompts::{CLEAR_LIBRARY_CONFIRM_COPY, clear_library_confirm};
        use riff_gui::ui::theme::Palette;

        let palette = Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 88.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let background = ui.ctx().layer_painter(egui::LayerId::background());
                background.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                let mut cache = riff_gui::ui::icons::IconCache::new();
                let _ = clear_library_confirm(ui, &mut cache, &palette);
            });
        harness.run();

        assert!(
            harness.query_by_label(CLEAR_LIBRARY_CONFIRM_COPY).is_some(),
            "the confirmation names what the wipe keeps"
        );
        let frame = harness.render().expect("the confirmation renders");
        assert!(
            count_frame_pixels(&frame, palette.error) > 0,
            "the destructive action carries the error ink"
        );
        assert_eq!(
            count_frame_pixels(&frame, palette.brand_primary),
            0,
            "a destructive confirmation is never brand-filled"
        );
    }

    #[test]
    fn test_high_contrast_is_a_variant_over_each_base_not_a_third_design() {
        let hc_dark = theme::Palette::dark().high_contrast();
        let hc_light = theme::Palette::light().high_contrast();

        // Each variant keeps its base's identity: mode, surfaces, brand.
        assert!(hc_dark.dark);
        assert!(!hc_light.dark);
        assert_eq!(hc_dark.background, theme::Palette::dark().background);
        assert_eq!(hc_light.background, theme::Palette::light().background);
        assert_eq!(hc_dark.surface, theme::Palette::dark().surface);
        assert_eq!(hc_light.surface, theme::Palette::light().surface);
        assert_eq!(hc_dark.brand_primary, theme::BRAND_500);
        assert_eq!(hc_light.brand_primary, theme::BRAND_500);

        // ...while text is pinned to the extreme of each base.
        assert_eq!(hc_dark.ink, egui::Color32::WHITE);
        assert_eq!(hc_light.ink, egui::Color32::BLACK);

        // Lines strengthen over their base but keep the base's hue family.
        assert!(hc_dark.border.a() > theme::BORDER.a());
        assert!(hc_light.border.a() > theme::Palette::light().border.a());

        // The focus ring leaves the brand tone for an unmistakable signal
        // (REQ-UI-007 behavior carried over).
        assert_ne!(hc_dark.focus_ring, theme::Palette::dark().focus_ring);
        assert_ne!(hc_light.focus_ring, theme::Palette::light().focus_ring);

        // Derived from different bases, so the variants differ from each other.
        assert_ne!(hc_dark, hc_light);
    }

    #[test]
    fn test_style_from_applies_dark_tokens_to_the_global_style() {
        let v = theme::style_from(&theme::Palette::dark()).visuals;

        // Window background + panel surfaces from the surface tokens.
        assert_eq!(v.panel_fill, theme::SURFACE);
        assert_eq!(v.window_fill, theme::SURFACE_BG);

        // Text from the ink tokens.
        assert_eq!(v.override_text_color, Some(theme::INK));

        // Hover fills come from surface-2; text-edit wells from --riff-input
        // (aliases surface-2).
        assert_eq!(v.widgets.hovered.weak_bg_fill, theme::SURFACE_2);
        assert_eq!(v.extreme_bg_color, theme::SURFACE_2);

        // Corner radii from the radius scale: sm widgets, md menus, lg windows
        // (6/10/14 px from --riff-radius-*, after the +2 px-per-step lift —
        // see the radius-scale test for why).
        assert_eq!(
            v.widgets.inactive.corner_radius,
            egui::CornerRadius::same(6)
        );
        assert_eq!(v.menu_corner_radius, egui::CornerRadius::same(10));
        assert_eq!(v.window_corner_radius, egui::CornerRadius::same(14));

        // Strokes from the line tokens; the selection ring from the focus
        // token.
        assert_eq!(v.widgets.inactive.bg_stroke.color, theme::BORDER);
        assert_eq!(v.selection.stroke.color, theme::Palette::dark().focus_ring);
        assert!(v.dark_mode);
    }

    #[test]
    fn test_style_from_applies_light_tokens_when_given_the_light_palette() {
        let light = theme::Palette::light();
        let v = theme::style_from(&light).visuals;

        assert!(!v.dark_mode);
        assert_eq!(v.panel_fill, light.surface);
        assert_eq!(v.window_fill, light.background);
        assert_eq!(v.override_text_color, Some(light.ink));
        assert_eq!(v.widgets.hovered.weak_bg_fill, light.surface_2);
        assert_eq!(v.widgets.inactive.bg_stroke.color, light.border);
    }

    #[test]
    fn test_high_contrast_style_keeps_focus_unmistakable() {
        // REQ-UI-007 carried over: focused/selected elements get strokes
        // thicker than egui's 1.0 default, over either base.
        for base in [theme::Palette::dark(), theme::Palette::light()] {
            let v = theme::style_from(&base.high_contrast()).visuals;
            assert!(
                v.selection.stroke.width > 1.0,
                "selection stroke for {} base",
                if base.dark { "dark" } else { "light" }
            );
            assert!(
                v.widgets.active.bg_stroke.width > 1.0,
                "focused-widget border for {} base",
                if base.dark { "dark" } else { "light" }
            );
            assert_eq!(v.dark_mode, base.dark);
        }
    }

    #[test]
    fn test_install_applies_the_palette_to_the_context() {
        let ctx = egui::Context::default();
        let light = theme::Palette::light();

        theme::install(&ctx, &light);

        let style = ctx.global_style();
        assert_eq!(style.visuals.panel_fill, light.surface);
        assert_eq!(style.visuals.override_text_color, Some(light.ink));
        assert!(!style.visuals.dark_mode);
    }

    #[test]
    fn test_focus_ring_stroke_appears_only_while_focused_and_thickens_in_high_contrast() {
        // Handoff issue 16: custom-painted rows show a visible keyboard-focus
        // ring from the palette's focus-ring token — nothing extra when idle
        // — and High Contrast thickens it (REQ-UI-007), like the search well.
        let dark = theme::Palette::dark();
        assert!(
            theme::focus_ring_stroke(&dark, false).is_none(),
            "an unfocused row paints no ring"
        );
        let ring = theme::focus_ring_stroke(&dark, true).expect("a focused row paints the ring");
        assert_eq!(ring.color, dark.focus_ring, "the ring is the ring token");

        let hc = dark.high_contrast();
        let hc_ring = theme::focus_ring_stroke(&hc, true).expect("high contrast keeps the ring");
        assert!(
            hc_ring.width > ring.width,
            "high contrast thickens the ring"
        );
    }

    // --- Typography: vendored Inter + text styles (Issue 02) --------------------
    //
    // Independent sources of truth: `colors_and_type.css`
    // (`--riff-font-sans: Inter, "PingFang SC", "Microsoft YaHei", …`;
    // `--riff-font-mono`) and the mockup pages' Tailwind usage — text-xs 12 /
    // text-sm 14 / text-xl 20 / text-3xl 30, with font-medium/semibold/bold
    // accents on buttons, section headers, and the wordmark.

    use riff_gui::ui::fonts;

    #[test]
    fn test_text_scale_constants_match_the_mockup_tailwind_usage() {
        // Tailwind rem scale: xs = 0.75rem = 12 px, sm = 0.875rem = 14 px,
        // xl = 1.25rem = 20 px, 3xl = 1.875rem = 30 px.
        assert!((theme::TEXT_XS - 12.0).abs() < f32::EPSILON);
        assert!((theme::TEXT_SM - 14.0).abs() < f32::EPSILON);
        assert!((theme::TEXT_XL - 20.0).abs() < f32::EPSILON);
        assert!((theme::TEXT_3XL - 30.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_vendored_inter_faces_are_embedded_and_valid() {
        // The faces are compiled into the binary via include_bytes!, so merely
        // reaching this assertion proves they were vendored into assets/.
        assert!(
            !fonts::INTER_FACES.is_empty(),
            "at least one Inter face must be vendored"
        );
        for (name, bytes) in fonts::INTER_FACES {
            assert!(!bytes.is_empty(), "{name} is empty");
            let signature: [u8; 4] = bytes[..4].try_into().expect("font magic is 4 bytes");
            let valid = signature == [0x00, 0x01, 0x00, 0x00]
                || &signature == b"OTTO"
                || &signature == b"true";
            assert!(
                valid,
                "{name} does not start with a recognized font signature"
            );
        }
        assert!(
            fonts::INTER_FACES
                .iter()
                .any(|(name, _)| name.contains("regular")),
            "a regular-weight face must be vendored as the primary UI font"
        );
    }

    #[test]
    fn test_font_definitions_render_inter_first_with_fallbacks_preserved() {
        let defs = fonts::font_definitions();

        let proportional = &defs.families[&egui::FontFamily::Proportional];
        assert_eq!(
            proportional.first().map(String::as_str),
            Some(fonts::INTER_PRIMARY_KEY),
            "Inter must be the primary UI font"
        );

        // egui's bundled fallbacks survive behind Inter so emoji and glyphs
        // Inter lacks keep rendering.
        for builtin in ["Ubuntu-Light", "NotoEmoji-Regular"] {
            assert!(
                proportional.iter().any(|name| name == builtin),
                "bundled fallback {builtin} must stay in the chain"
            );
        }

        // CJK fallback preserved (Issue 02): when a system CJK font is found
        // it must sit *behind* Inter — Inter owns Latin, CJK covers the rest.
        if defs.font_data.contains_key(fonts::CJK_FALLBACK_KEY) {
            let cjk_position = proportional
                .iter()
                .position(|name| name == fonts::CJK_FALLBACK_KEY);
            assert!(
                cjk_position.is_some_and(|position| position > 0),
                "the CJK fallback must come after Inter in the proportional chain"
            );
        }
    }

    #[test]
    fn test_weight_families_are_registered_for_the_design_weights() {
        // The mockup leans on font-medium (buttons, row labels), font-semibold
        // (section headers, h1) and font-bold (wordmark); each gets its own
        // egui family so text_styles can reference it by name.
        let defs = fonts::font_definitions();
        for family in [
            fonts::family_medium(),
            fonts::family_semibold(),
            fonts::family_bold(),
        ] {
            let chain = &defs.families[&family];
            assert!(
                chain.first().is_some_and(|name| name.starts_with("inter-")),
                "weight family {family:?} must start with an Inter face"
            );
        }
    }

    #[test]
    fn test_monospace_family_is_registered_for_time_readouts() {
        // Seek/volume time displays render through FontFamily::Monospace; the
        // override must leave that family resolvable.
        let defs = fonts::font_definitions();
        assert!(
            !defs.families[&egui::FontFamily::Monospace].is_empty(),
            "the monospace family must resolve to at least one font"
        );
    }

    #[test]
    fn test_configure_fonts_installs_the_definitions_on_a_context() {
        let ctx = egui::Context::default();
        fonts::configure_fonts(&ctx);

        // The font view only exists after a pass has begun, so tick one
        // headless frame before reading back what got installed.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_ui| {});
        // egui 0.36 asserts on drop if texture deltas are never applied;
        // a real backend would upload them, so clear them here instead.
        output.textures_delta.clear();
        let installed = ctx.fonts(|view| view.definitions().clone());
        assert_eq!(
            installed.families[&egui::FontFamily::Proportional]
                .first()
                .map(String::as_str),
            Some(fonts::INTER_PRIMARY_KEY),
            "the context renders Inter after configure_fonts"
        );
    }

    #[test]
    fn test_text_styles_map_egui_keys_onto_the_design_scale() {
        let styles = theme::text_styles();

        // Body carries the workhorse text-sm.
        let body = &styles[&egui::TextStyle::Body];
        assert!((body.size - theme::TEXT_SM).abs() < f32::EPSILON);
        assert_eq!(body.family, egui::FontFamily::Proportional);

        // Small carries text-xs (muted labels, meta lines).
        let small = &styles[&egui::TextStyle::Small];
        assert!((small.size - theme::TEXT_XS).abs() < f32::EPSILON);

        // Heading carries text-xl at semibold (mockup h1s).
        let heading = &styles[&egui::TextStyle::Heading];
        assert!((heading.size - theme::TEXT_XL).abs() < f32::EPSILON);
        assert_eq!(heading.family, fonts::family_semibold());

        // Buttons carry text-sm at medium weight (mockup buttons).
        let button = &styles[&egui::TextStyle::Button];
        assert!((button.size - theme::TEXT_SM).abs() < f32::EPSILON);
        assert_eq!(button.family, fonts::family_medium());

        // Monospace stays on the monospace family for time readouts.
        let mono = &styles[&egui::TextStyle::Monospace];
        assert!((mono.size - theme::TEXT_SM).abs() < f32::EPSILON);
        assert_eq!(mono.family, egui::FontFamily::Monospace);
    }

    #[test]
    fn test_hero_title_font_names_the_now_playing_3xl() {
        // The mockup's single text-3xl usage is the Now Playing title
        // (text-3xl font-semibold); it gets a named constructor so view code
        // references the scale by name instead of hardcoding 30.0.
        let font = theme::hero_title_font();
        assert!((font.size - theme::TEXT_3XL).abs() < f32::EPSILON);
        assert_eq!(font.family, fonts::family_semibold());
    }

    #[test]
    fn test_install_applies_the_text_styles_to_the_context() {
        let ctx = egui::Context::default();
        theme::install(&ctx, &theme::Palette::dark());

        let style = ctx.global_style();
        assert_eq!(style.text_styles, theme::text_styles());
    }

    // --- Frameless window chrome (Issue 04, ADR 0005) --------------------------
    //
    // riff launches undecorated and draws its own titlebar: a drag region
    // plus custom minimize/close controls. The headless seams are the launch
    // viewport configuration, the control→viewport-command contract, and the
    // drag-region gesture decision; the pixels themselves are covered by the
    // golden-image harness later (issue 05).

    #[test]
    fn test_launch_viewport_is_frameless_while_keeping_the_window_size_contract() {
        let builder = riff_gui::ui::chrome::viewport_builder();

        // The OS title bar is gone; riff's custom chrome replaces it.
        assert_eq!(builder.decorations, Some(false));
        // The decorated window's launch/minimum sizes carry over unchanged.
        assert_eq!(builder.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(builder.min_inner_size, Some(egui::vec2(800.0, 600.0)));
    }

    #[test]
    fn test_window_controls_minimize_collapses_and_close_hides_or_quits_by_platform() {
        use riff_gui::ui::chrome::WindowControl;

        // Minimize collapses the window.
        assert_eq!(
            WindowControl::Minimize.viewport_command(),
            egui::ViewportCommand::Minimized(true)
        );

        // Split-close-paths (owner decision 2026-09-19): the close-to-tray
        // veto is gone, so the custom close is no longer "a Close the veto
        // turns into a hide". On Linux there is no tray, and the custom X
        // really closes.
        #[cfg(target_os = "linux")]
        assert_eq!(
            WindowControl::Close.viewport_command(),
            egui::ViewportCommand::Close
        );

        // On macOS/Windows the custom X is the only hide gesture: it enqueues
        // a frontend-local VisibilityMessage(false) that logic() applies one
        // frame later — never a Close, which now always quits.
        #[cfg(not(target_os = "linux"))]
        {
            use riff_gui::ui::app::CUSTOM_TITLEBAR_CLOSE;
            use riff_gui::ui::window_visibility::VisibilityMessage;
            assert_eq!(CUSTOM_TITLEBAR_CLOSE, VisibilityMessage(false));
        }
    }

    #[test]
    fn test_drag_region_gestures_decide_between_drag_and_maximize_toggle() {
        use riff_gui::ui::chrome::{DragRegionAction, drag_region_action};

        // A primary-button press-and-move starts an OS window move.
        assert_eq!(
            drag_region_action(true, false),
            Some(DragRegionAction::StartDrag)
        );
        // A double-click toggles maximize/restore (titlebar convention).
        assert_eq!(
            drag_region_action(false, true),
            Some(DragRegionAction::ToggleMaximize)
        );
        // Double-click wins over the drag start that precedes it in the same
        // frame, or a jittery double-click would drag instead of maximizing.
        assert_eq!(
            drag_region_action(true, true),
            Some(DragRegionAction::ToggleMaximize)
        );
        // Plain clicks and hover mean nothing to the drag region.
        assert_eq!(drag_region_action(false, false), None);
    }

    // --- Tray "Show Window" -> viewport commands (REQ-SI-001) -------------------
    //
    // The tray runs on its own thread and cannot call into the egui context, so
    // it sends a visibility request and the frame loop turns it into viewport
    // commands. That mapping is the whole contract, and it is where the reported
    // failure lived: the loop gated the show on `viewport().visible()`, which
    // egui derives only from minimized/occluded state — state egui-winit never
    // fills in — so a window hidden to the tray kept reporting as visible and
    // the request was dropped. The commands must follow the *request*.

    #[test]
    fn test_a_tray_show_request_carries_the_window_back_to_the_foreground() {
        use riff_gui::ui::window_visibility::{VisibilityMessage, viewport_commands_for};

        // The regression itself: the request is honored whatever the loop
        // believes about the current state, and focus comes last because the OS
        // will not focus a window it has not un-hidden yet.
        assert_eq!(
            viewport_commands_for(VisibilityMessage(true), false),
            vec![
                egui::ViewportCommand::Visible(true),
                egui::ViewportCommand::Focus,
            ]
        );
        // A window the user minimized (titlebar button) is a separate OS state:
        // un-hiding alone leaves it iconified, so the show restores it too.
        assert_eq!(
            viewport_commands_for(VisibilityMessage(true), true),
            vec![
                egui::ViewportCommand::Visible(true),
                egui::ViewportCommand::Minimized(false),
                egui::ViewportCommand::Focus,
            ]
        );
        // The hide request is what the custom titlebar X (or the tray) issues
        // through the visibility channel.
        assert_eq!(
            viewport_commands_for(VisibilityMessage(false), false),
            vec![egui::ViewportCommand::Visible(false)]
        );
    }

    // --- The tray icon's brand mark (REQ-SI-001) ---------------------------------
    //
    // The tray painted a flat blue rectangle for as long as it existed: nothing
    // fed it a glyph. It now rasterizes the titlebar's equalizer mark from the
    // same geometry, so what is worth asserting is the *shape* — four bars on a
    // transparent field, in the brand's orange — because that is the difference
    // between the mark and a block.

    /// The mark as its inked column runs: `(center x, tallest column)` per bar.
    fn brand_mark_runs(rgba: &[u8], side: usize) -> Vec<(usize, usize)> {
        let inked = |x: usize, y: usize| rgba[(y * side + x) * 4 + 3] > 0;
        let column = |x: usize| (0..side).filter(|&y| inked(x, y)).count();
        let mut runs = Vec::new();
        let mut x = 0;
        while x < side {
            if column(x) == 0 {
                x += 1;
                continue;
            }
            let start = x;
            let mut tallest = 0;
            while x < side && column(x) > 0 {
                tallest = tallest.max(column(x));
                x += 1;
            }
            runs.push(((start + x - 1) / 2, tallest));
        }
        runs
    }

    #[test]
    fn test_the_tray_icon_rasterizes_the_equalizer_mark() {
        let side = riff_gui::ui::chrome::APP_ICON_PX as usize;
        let rgba = riff_gui::ui::chrome::icon_rgba().expect("the brand mark must rasterize");
        assert_eq!(rgba.len(), side * side * 4);
        assert_eq!(rgba[3], 0, "the icon's corner stays transparent");

        let runs = brand_mark_runs(&rgba, side);
        assert_eq!(runs.len(), 4, "four bars, not one block");

        // The fixed profile [0.55, 0.95, 0.7, 0.4]: the second bar is the
        // tallest and the last the shortest, and no two bars share a height.
        let heights: Vec<usize> = runs.iter().map(|&(_, h)| h).collect();
        assert_eq!(heights[1], *heights.iter().max().unwrap_or(&0));
        assert_eq!(heights[3], *heights.iter().min().unwrap_or(&0));
        assert_eq!(
            heights
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4,
            "the bars keep distinct heights: {heights:?}"
        );

        // And it is the brand's own orange, not the placeholder's blue.
        let brand = riff_gui::ui::theme::BRAND_500;
        let (bar_x, _) = runs[0];
        let ink = (side / 2 * side + bar_x) * 4;
        assert_eq!(
            (rgba[ink], rgba[ink + 1], rgba[ink + 2]),
            (brand.r(), brand.g(), brand.b())
        );
    }

    #[test]
    fn test_egui_reports_no_visibility_so_the_app_keeps_its_own_record() {
        // Documents the trap the test above guards: `ViewportInfo::visible` is
        // derived from minimized/occluded state only, and egui-winit reports
        // neither for a hidden window. If upstream starts reporting real
        // visibility, this fails and the app-side record can be retired.
        assert_eq!(egui::ViewportInfo::default().visible(), None);
    }

    // --- Token-authority sweeps (Issue 03, ADR 0004) -----------------------------
    //
    // `theme.rs` is both the store and the read source for every design value,
    // so the UI layer's source gets scanned for what a view must not do:
    // construct or derive a color, declare a dimension of its own, or set a
    // spacing gap by number. Each rule is mechanical, which is the point — the
    // acceptance for the token-authority work is "grep-verifiable", and encoded
    // here it stops being something a later restyle ticket can quietly drift
    // out of.

    /// True when a code line derives an egui color instead of reading one: a
    /// `Color32`/`Rgba` construction from scratch (any `from_*` constructor or
    /// associated constant), or a palette color scaled into a new one at the
    /// call site (`gamma_multiply`, `linear_multiply`, `to_opaque`, …).
    ///
    /// The constructor path alone was not enough: `palette.error
    /// .gamma_multiply(0.1)` reads a token and passed clean while still
    /// deciding a color outside the design system. Reading a color's channels
    /// (`color.r()` for a texture name, `px.a() == 0` to skip a pixel) is not
    /// a derivation and stays unflagged, and [`theme::blend_over`] stays
    /// sanctioned too — it is the helper the token module exposes for
    /// composing colors, and its one view caller composites rasterized
    /// texture pixels rather than choosing a style.
    fn hardcoded_color_literal(line: &str) -> bool {
        for marker in ["Color32::", "Rgba::"] {
            let mut search = 0;
            while let Some(rel) = line[search..].find(marker) {
                let start = search + rel + marker.len();
                if line[start..].starts_with("from_")
                    || line[start..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase())
                {
                    return true;
                }
                search = start;
            }
        }
        [
            ".gamma_multiply(",
            ".gamma_multiply_u8(",
            ".linear_multiply(",
            ".to_opaque(",
            ".to_normalized_rgba(",
            ".to_normalized_gamma_f32(",
        ]
        .iter()
        .any(|derives| line.contains(derives))
    }

    /// Every `(path, line number, trimmed source)` triple the token sweeps
    /// judge: `.rs` files under `crates/riff-gui/src/ui`, minus `theme.rs` —
    /// which is the store, so the literals are its job — and minus comment
    /// lines, so prose may name the APIs a sweep bans.
    fn ui_source_lines() -> Vec<(std::path::PathBuf, usize, String)> {
        fn scan_dir(dir: &std::path::Path, lines: &mut Vec<(std::path::PathBuf, usize, String)>) {
            let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
                .expect("the src/ui directory must be readable")
                .map(|entry| entry.expect("directory entries must resolve").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    scan_dir(&path, lines);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && path.file_name().and_then(|name| name.to_str()) != Some("theme.rs")
                {
                    let source =
                        std::fs::read_to_string(&path).expect("source files must be UTF-8");
                    for (idx, line) in source.lines().enumerate() {
                        let trimmed = line.trim_start();
                        if trimmed.starts_with("//") {
                            continue;
                        }
                        lines.push((path.clone(), idx + 1, trimmed.to_owned()));
                    }
                }
            }
        }

        let mut lines = Vec::new();
        scan_dir(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("crates")
                .join("riff-gui")
                .join("src")
                .join("ui"),
            &mut lines,
        );
        lines
    }

    /// `path:line: source` for every swept line a token sweep objects to.
    fn violations<'a>(
        mut offending: impl FnMut(&str) -> bool,
        lines: impl IntoIterator<Item = &'a (std::path::PathBuf, usize, String)>,
    ) -> Vec<String> {
        lines
            .into_iter()
            .filter(|(_, _, line)| offending(line))
            .map(|(path, num, line)| format!("{}:{num}: {line}", path.display()))
            .collect()
    }

    /// Collect `path:line` pairs where UI-layer source derives an egui color.
    fn hardcoded_color_violations() -> Vec<String> {
        violations(hardcoded_color_literal, &ui_source_lines())
    }

    /// True when a line gives a view a dimension of its own: a `const` of a
    /// measured type declared outside `theme.rs`.
    ///
    /// A derived declaration counts (`MIN_INNER_H = PLAY_BTN + 16.0 + 8.0`) —
    /// the arithmetic belongs beside the tokens it reads, which is where the
    /// surfaces that share a control disagree today. What a view may still own
    /// is what no designer would retune and no second surface reads: a `usize`
    /// count of rows to ask its read model for, a `[f32; 4]` of shape data for
    /// one hand-painted glyph, a texture's raster resolution.
    fn view_owned_dimension(line: &str) -> bool {
        let Some(decl) = line
            .strip_prefix("const ")
            .or_else(|| line.strip_prefix("pub const "))
        else {
            return false;
        };
        decl.split_once(": ").is_some_and(|(_, typed)| {
            ["f32 =", "egui::Vec2 ="]
                .iter()
                .any(|ty| typed.starts_with(ty))
        })
    }

    /// True when `text` sets a non-zero number: a digit run that starts on a
    /// word boundary, so the `2` in `vec2` contributes nothing and `0.0` reads
    /// as the zero it is.
    fn sets_a_nonzero_number(text: &str) -> bool {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let starts_a_literal =
                bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric());
            if !starts_a_literal {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && matches!(bytes[i], b'0'..=b'9' | b'.') {
                i += 1;
            }
            if text[start..i]
                .parse::<f32>()
                .is_ok_and(|value| value != 0.0)
            {
                return true;
            }
        }
        false
    }

    /// True when a line leaves a gap between items that the spacing scale does
    /// not name. Zero stays allowed: setting `item_spacing` to nothing is how a
    /// widget opts out of egui's layout so it can place its own sub-rectangles,
    /// which is structure rather than design.
    fn numeric_item_spacing(line: &str) -> bool {
        line.contains("item_spacing") && sets_a_nonzero_number(line)
    }

    /// WCAG 2.1 relative luminance: sRGB channels linearized and weighted.
    fn relative_luminance(color: egui::Color32) -> f32 {
        let channel = |byte: u8| {
            let s = f32::from(byte) / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
    }

    /// WCAG 2.1 contrast ratio of two colors: `(lighter + 0.05) / (darker +
    /// 0.05)`, from 1.0 (identical) to 21.0 (black on white).
    fn contrast_ratio(a: egui::Color32, b: egui::Color32) -> f32 {
        let (x, y) = (relative_luminance(a), relative_luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// The AA floor for normal-size text: what every muted label in the app is,
    /// and what `ink_3` was failing at 3.40:1 on a panel while carrying
    /// required text (design-handoff review P1-9).
    const AA_NORMAL: f32 = 4.5;

    /// A translucent [`egui::Color32`] over an opaque one. `Color32` stores its
    /// channels premultiplied by their own alpha, so source-over here is the
    /// stored top added onto the scaled bottom — which is why this is not
    /// [`theme::blend_over`], whose contract is a straight-alpha color, like
    /// the rasterized texture pixels it composes.
    fn over(bottom: egui::Color32, top: egui::Color32) -> egui::Color32 {
        let alpha = f32::from(top.a()) / 255.0;
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let channel = |b: u8, t: u8| {
            (f32::from(t) + f32::from(b) * (1.0 - alpha))
                .min(255.0)
                .round() as u8
        };
        egui::Color32::from_rgb(
            channel(bottom.r(), top.r()),
            channel(bottom.g(), top.g()),
            channel(bottom.b(), top.b()),
        )
    }

    /// Every fill a text token can be painted on, for the contrast test: the
    /// surface ramp plus the row washes. `row_hover` is composited because the
    /// light family's wash is translucent and paints *over* a panel, while the
    /// dark one is opaque.
    ///
    /// Listed in ramp order so a new step reads as a neighbour of the ones it
    /// sits between. The wash appears twice on purpose: once over the card
    /// plane (`surface`, where a popover or menu lands) and once over the row
    /// plane (`surface_row`, where a hovered list row lands). The two are not
    /// the same color in the light family, whose wash is translucent, so
    /// checking one does not check the other.
    fn text_fills(palette: &theme::Palette) -> [(&'static str, egui::Color32); 7] {
        [
            ("background", palette.background),
            ("surface_row", palette.surface_row),
            ("surface", palette.surface),
            ("surface_2", palette.surface_2),
            ("surface_3", palette.surface_3),
            ("row_hover", over(palette.surface, palette.row_hover)),
            (
                "row_hover_on_row",
                over(palette.surface_row, palette.row_hover),
            ),
        ]
    }

    #[test]
    fn test_tokens_clear_their_wcag_floor_on_every_fill_they_paint_on() {
        for (dark, high_contrast) in [(true, false), (false, false), (true, true), (false, true)] {
            let palette = theme::resolve(dark, high_contrast);
            let family = format!(
                "{}{}",
                if dark { "dark" } else { "light" },
                if high_contrast { "+HC" } else { "" }
            );
            let inks = [
                ("ink", palette.ink),
                ("ink_2", palette.ink_2),
                ("ink_3", palette.ink_3),
            ];
            for (ink_name, ink) in inks {
                for (fill_name, fill) in text_fills(&palette) {
                    let ratio = contrast_ratio(ink, fill);
                    assert!(
                        ratio >= AA_NORMAL,
                        "{family}: {ink_name} on {fill_name} reads {ratio:.2}:1, under the \
                         {AA_NORMAL}:1 AA floor — retune the token in theme.rs, not the call site"
                    );
                }
            }
            // Text on brand fills has its own token, so the pair is checked too.
            let on_brand = contrast_ratio(palette.on_brand, palette.brand_primary);
            assert!(
                on_brand >= AA_NORMAL,
                "{family}: on_brand on brand_primary reads {on_brand:.2}:1"
            );

            // `warning` is a text color in three views — the Clear Library
            // confirmation, a tag row's `(different)` state, an unindexed
            // path's status dot — so it holds the same floor as the ink ladder.
            let warning = contrast_ratio(palette.warning, palette.surface);
            assert!(
                warning >= AA_NORMAL,
                "{family}: warning on surface reads {warning:.2}:1, but it is painted as text"
            );

            // The error role paints text in three places — the destructive
            // button's label, a failed scan's error line, an unreadable path —
            // and its fill is never the panel alone: the button's hover wash is
            // the error token itself composited over the surface. So it holds
            // the same floor on every fill a text token can land on, which is
            // also what forces the per-family re-pick: no single red clears
            // 4.5:1 on both a light panel and a raised dark one.
            for (fill_name, fill) in text_fills(&palette) {
                let error = contrast_ratio(palette.error, fill);
                assert!(
                    error >= AA_NORMAL,
                    "{family}: error on {fill_name} reads {error:.2}:1, but it is painted as \
                     text — retune the token in theme.rs, not the call site"
                );
            }

            // The focus ring is non-text UI, which WCAG 1.4.11 puts at 3:1
            // against the colors beside it — the floor that `HC_FOCUS_RING`
            // clears on dark and its deep-gold twin has to clear on light.
            for (fill_name, fill) in text_fills(&palette) {
                let ring = contrast_ratio(palette.focus_ring, fill);
                assert!(
                    ring >= 3.0,
                    "{family}: focus_ring against {fill_name} reads {ring:.2}:1, under the \
                     3:1 floor for a non-text control indicator"
                );
            }
        }
    }

    #[test]
    fn test_ink_ladder_stays_distinguishable_after_the_aa_retune() {
        // Clearing AA pulled the muted rungs toward the top of the ramp; this
        // checks they did not collapse into one gray. 4.5 L* is around the
        // smallest difference these labels show at text sizes.
        for (dark, high_contrast) in [(true, false), (false, false), (true, true), (false, true)] {
            let palette = theme::resolve(dark, high_contrast);
            let rung = |luminance: f32| {
                let t = if luminance > 0.008_856 {
                    luminance.cbrt()
                } else {
                    luminance / 0.206_897 + 0.128_42
                };
                116.0 * t - 16.0
            };
            let (ink, ink_2, ink_3) = (
                rung(relative_luminance(palette.ink)),
                rung(relative_luminance(palette.ink_2)),
                rung(relative_luminance(palette.ink_3)),
            );
            let family = if dark { "dark" } else { "light" };
            let (gap_1, gap_2) = if dark {
                (ink - ink_2, ink_2 - ink_3)
            } else {
                (ink_2 - ink, ink_3 - ink_2)
            };
            assert!(
                gap_1 >= 4.5 && gap_2 >= 4.5,
                "{family} ink ladder collapsed: rungs at {ink:.1}/{ink_2:.1}/{ink_3:.1} L* \
                 (gaps {gap_1:.1} and {gap_2:.1})"
            );
        }
    }

    #[test]
    fn test_view_code_contains_no_hardcoded_color_literals() {
        let violations = hardcoded_color_violations();
        assert!(
            violations.is_empty(),
            "view code must style itself from theme tokens (ADR 0004); \
             found hardcoded colors:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn test_the_color_sweep_flags_derivations_but_not_channel_reads() {
        // The hole the widening closed: a palette color scaled at a call site
        // decides a color outside the design system without naming a
        // constructor, so it must fail the sweep — as must a literal.
        for derives in [
            "    let fill = palette.error.gamma_multiply(0.1);",
            "    let wash = palette.ink_3.gamma_multiply(0.4);",
            "    let ring = theme::INK.linear_multiply(0.5);",
            "    let edge = palette.border.gamma_multiply_u8(200);",
            "    let flat = Color32::from_rgb(0, 0, 0);",
        ] {
            assert!(hardcoded_color_literal(derives), "sweep misses {derives}");
        }
        // Reading a color's channels composes nothing, and `blend_over` is the
        // helper the token module exposes for composing them: both stay clean,
        // or the sweep would be arguing with the design system instead of
        // protecting it.
        for benign in [
            "    let key = (icon, px, color.r(), color.g(), color.b());",
            "    if px.a() == 0 { continue; }",
            "    image.pixels[i] = theme::blend_over(image.pixels[i], *px);",
            "    painter.galley(pos, galley, palette.ink);",
        ] {
            assert!(
                !hardcoded_color_literal(benign),
                "sweep false-positives {benign}"
            );
        }
    }

    /// The geometry half of the same claim as the color sweep: `theme.rs` is
    /// the store, so a view that declares a measured constant has taken a
    /// design value into its own hands. Before the views were cleaned up this
    /// caught nothing, because every one of these constants was already gone;
    /// it exists so the next one cannot land quietly.
    #[test]
    fn test_view_code_declares_no_dimensions_of_its_own() {
        let violations = violations(view_owned_dimension, &ui_source_lines());
        assert!(
            violations.is_empty(),
            "component geometry belongs in theme.rs's `geometry` section, next \
             to the surface that paints it (ADR 0004); found view-owned \
             dimensions:\n{}",
            violations.join("\n")
        );

        for declares in [
            "const ROW_H: f32 = 40.0;",
            "pub const HEADER_H: f32 = 28.0;",
            "const MIN_INNER_H: f32 = PLAY_BTN + 16.0 + 8.0;",
            "pub const MIN_STAGE_SIZE: egui::Vec2 = egui::vec2(520.0, 456.0);",
        ] {
            assert!(view_owned_dimension(declares), "sweep misses {declares}");
        }
        for owns in [
            // What the rule deliberately does not reach: a read-model bound,
            // one glyph's shape data, a raster resolution, a token module's
            // own `const fn`, and a plain local.
            "pub const UP_NEXT_LIMIT: usize = 5;",
            "const WORDMARK_BARS: [f32; 4] = [0.55, 0.95, 0.7, 0.4];",
            "const TILE_PX: usize = 256;",
            "pub const fn corner(radius: f32) -> CornerRadius {",
            "    let row_h = theme::geometry::sidebar::ROW_H;",
        ] {
            assert!(!view_owned_dimension(owns), "sweep false-positives {owns}");
        }
    }

    /// The spacing half: a gap between items is a scale step, so a numeric
    /// `item_spacing` assignment in a view is a gap the design system has
    /// never heard of.
    #[test]
    fn test_view_code_sets_no_spacing_of_its_own() {
        let violations = violations(numeric_item_spacing, &ui_source_lines());
        assert!(
            violations.is_empty(),
            "spacing between items comes from theme's SPACE_* scale (ADR 0004); \
             found numeric item_spacing assignments:\n{}",
            violations.join("\n")
        );

        for sets in [
            "ui.spacing_mut().item_spacing.x = 6.0;",
            "ui.spacing_mut().item_spacing.x = 10.0;",
            "ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);",
        ] {
            assert!(numeric_item_spacing(sets), "sweep misses {sets}");
        }
        for reads in [
            "ui.spacing_mut().item_spacing.x = 0.0;",
            "strip_ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);",
            "ui.spacing_mut().item_spacing.x = theme::SPACE_MD;",
        ] {
            assert!(
                !numeric_item_spacing(reads),
                "sweep false-positives {reads}"
            );
        }
    }

    #[test]
    fn test_resolve_builds_the_active_palette_for_a_theme_selection() {
        // The plain bases resolve to themselves...
        assert_eq!(theme::resolve(true, false), theme::Palette::dark());
        assert_eq!(theme::resolve(false, false), theme::Palette::light());
        // ...and High Contrast resolves as a variant over the selected base,
        // never a third design (ADR 0004).
        assert_eq!(
            theme::resolve(true, true),
            theme::Palette::dark().high_contrast()
        );
        assert_eq!(
            theme::resolve(false, true),
            theme::Palette::light().high_contrast()
        );
    }

    // --- App shell & shared chrome (Issue 06) ----------------------------------
    //
    // The shell merges the frameless titlebar (issue 04) with the former top
    // bar at exact token dimensions (56/280/88), vendors the Lucide glyphs
    // behind an icon helper, and routes nav so exactly one View is visible.

    use riff_backend::app::state::{BrowseMode, ViewMode};
    use riff_gui::ui::{chrome, icons};

    #[test]
    fn test_min_window_size_fits_the_fixed_chrome() {
        use theme::geometry::window;
        // The chrome-fitting minimum must leave room for the fixed panels
        // PLUS a usable main stage: sidebar + stage across, titlebar +
        // playerbar + stage down. A window below this would collapse the
        // fixed chrome.
        let min = window::MIN_WINDOW_SIZE;
        assert!(min.x >= theme::SIDEBAR_W + window::MIN_STAGE_SIZE.x);
        assert!(min.y >= theme::TITLEBAR_H + theme::PLAYERBAR_H + window::MIN_STAGE_SIZE.y);

        let builder = chrome::viewport_builder();
        assert_eq!(builder.min_inner_size, Some(min));
        // The frameless launch contract from issue 04 carries over unchanged.
        assert_eq!(builder.decorations, Some(false));
        assert_eq!(builder.inner_size, Some(egui::vec2(1200.0, 800.0)));
    }

    #[test]
    fn test_nav_destination_active_pins_exactly_one_view() {
        use chrome::NavDestination;

        // Library/Folders are the two library browse destinations...
        assert_eq!(
            NavDestination::active(ViewMode::Library, BrowseMode::Library),
            Some(NavDestination::Library)
        );
        assert_eq!(
            NavDestination::active(ViewMode::Library, BrowseMode::Folders),
            Some(NavDestination::Folders)
        );
        // ...Settings is its own view regardless of the dormant browse mode...
        assert_eq!(
            NavDestination::active(ViewMode::Settings, BrowseMode::Library),
            Some(NavDestination::Settings)
        );
        assert_eq!(
            NavDestination::active(ViewMode::Settings, BrowseMode::Folders),
            Some(NavDestination::Settings)
        );
        // ...and Now Playing REPLACES the active view, so no nav destination
        // is highlighted while it is up.
        assert_eq!(
            NavDestination::active(ViewMode::NowPlaying, BrowseMode::Library),
            None
        );
    }

    #[test]
    fn test_nav_apply_routes_to_exactly_one_view_from_any_state() {
        use chrome::NavDestination;

        // From EVERY starting state, routing to any destination must land on
        // exactly that one visible view — never two, never none.
        for start_view in [ViewMode::Library, ViewMode::NowPlaying, ViewMode::Settings] {
            for start_browse in [BrowseMode::Library, BrowseMode::Folders] {
                for dest in [
                    NavDestination::Library,
                    NavDestination::Folders,
                    NavDestination::Settings,
                ] {
                    let mut view = start_view;
                    let mut browse = start_browse;
                    dest.apply(&mut view, &mut browse);
                    assert_eq!(
                        NavDestination::active(view, browse),
                        Some(dest),
                        "routing to {dest:?} from ({start_view:?}, {start_browse:?}) \
                         must leave exactly that one view visible"
                    );
                }
            }
        }
    }

    #[test]
    fn test_icon_inventory_is_vendored_and_complete() {
        // The redesign vendors ~22 Lucide glyphs; the helper must serve them
        // all to later tickets from one place.
        assert!(
            icons::Icon::ALL.len() >= 22,
            "expected at least 22 vendored glyphs, got {}",
            icons::Icon::ALL.len()
        );
        let mut names: Vec<&str> = icons::Icon::ALL.iter().map(|i| i.asset_name()).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique, "icon asset names must be unique");
        for icon in icons::Icon::ALL {
            let svg = icon.svg();
            assert!(
                svg.contains("<svg"),
                "{} must embed its vendored Lucide SVG source",
                icon.asset_name()
            );
            assert!(
                svg.contains("currentColor"),
                "{} uses currentColor so the helper can tint it per palette",
                icon.asset_name()
            );
        }
    }

    #[test]
    fn test_rasterize_tints_icons_with_the_requested_color() {
        use riff_gui::ui::theme::INK;

        let image = icons::rasterize(icons::Icon::Play.svg(), 24, INK)
            .expect("the play glyph rasterizes headlessly");
        assert_eq!(image.size, [24, 24]);

        // Some pixels are painted...
        let painted: Vec<_> = image.pixels.iter().filter(|p| p.a() > 0).collect();
        assert!(!painted.is_empty(), "the glyph paints at least one pixel");
        // Fully-opaque pixels carry EXACTLY the tint color in straight
        // alpha — the icon follows the palette, not a flat literal.
        // (Anti-aliased edge pixels legitimately round off the tint while
        // crossing the premultiplied buffer, so only full coverage asserts.)
        let opaque: Vec<_> = painted.iter().filter(|p| p.a() == 255).collect();
        assert!(!opaque.is_empty(), "the glyph has full-coverage pixels");
        for p in &opaque {
            assert_eq!((p.r(), p.g(), p.b()), (INK.r(), INK.g(), INK.b()));
        }
    }

    #[test]
    fn test_icon_cache_reuses_one_texture_per_icon_size_and_color() {
        let ctx = egui::Context::default();
        let mut cache = icons::IconCache::new();
        let ink = theme::Palette::dark().ink;

        let first = cache.texture(&ctx, icons::Icon::Play, 16.0, ink);
        let again = cache.texture(&ctx, icons::Icon::Play, 16.0, ink);
        assert_eq!(
            first, again,
            "same icon/size/color must reuse the cached texture"
        );

        let other_icon = cache.texture(&ctx, icons::Icon::Pause, 16.0, ink);
        assert_ne!(first, other_icon);

        let recolored = cache.texture(&ctx, icons::Icon::Play, 16.0, theme::BRAND_500);
        assert_ne!(first, recolored, "tint participates in the key");

        let resized = cache.texture(&ctx, icons::Icon::Play, 32.0, ink);
        assert_ne!(first, resized, "size participates in the key");
    }

    #[test]
    fn test_titlebar_clicks_report_window_and_nav_actions() {
        use riff_gui::ui::chrome::{TitleBarAction, TitleBarContent, show_titlebar};
        use riff_gui::ui::icons::IconCache;

        // Harness label queries resolve through kittest's accessibility tree.
        use egui_kittest::kittest::Queryable;

        let content = TitleBarContent {
            scan_status: None,
            theme_dark: true,
            advanced_mode: false,
            // Library is the active destination in this fixture.
            active_nav: Some(chrome::NavDestination::Library),
        };
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut widget_actions = Vec::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 56.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions| {
                    // ACCUMULATE across frames: a click fires its action on
                    // exactly one frame, and harness.run() settles over
                    // further no-op frames afterwards.
                    widget_actions.clear();
                    let mut query = String::new();
                    show_titlebar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut query,
                        &mut widget_actions,
                    );
                    actions.append(&mut widget_actions);
                },
                Vec::new(),
            );
        harness.run();

        // Custom window controls keep their issue-04 contract, now surfaced as
        // TitleBarActions the app resolves: Close hides through the
        // visibility channel on macOS/Windows and really closes on Linux.
        harness.get_by_label("Close").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::Close));

        harness.get_by_label("Minimize").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::Minimize));

        harness.get_by_label("Maximize").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::ToggleMaximize));

        // The former top-bar controls live in the merged titlebar now.
        harness.get_by_label("Settings").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::GoSettings));

        harness.get_by_label("Theme").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::ToggleTheme));

        harness.get_by_label("Now Playing").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::ToggleNowPlaying));

        harness.get_by_label("Advanced: Off").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::ToggleAdvanced));
    }

    // --- Sidebar (design-handoff issue 07) --------------------------------------
    //
    // The sidebar matches the design: a search box with a focus-ring border,
    // the flat sectioned nav (LIBRARY / SMART LISTS / PLAYLISTS) on 40px tree
    // rows with hover states and right-aligned live counts, an animated
    // equalizer-bars indicator on the now-playing row, playlist rows whose
    // hover-revealed edit/delete drive the EXISTING rename/delete Store flows
    // (ADR 0002 projection refresh), and the Add-folder / last-scan footer.
    //
    // The widgets live behind headless seams in `riff_gui::ui::sidebar`; the
    // pixels are pinned by the `sidebar_dark` golden image.

    use riff_gui::ui::sidebar;

    #[test]
    fn test_library_section_defaults_to_all_tracks() {
        // The sidebar's LIBRARY rows select the browser variant through the
        // session; a fresh session starts on All Tracks (design-handoff
        // issue 07, consumed by the browser column in issue 08).
        assert_eq!(
            riff_backend::app::state::LibrarySection::default(),
            riff_backend::app::state::LibrarySection::AllTracks,
        );
    }

    #[test]
    fn test_sidebar_tree_rows_use_the_mockup_40px_height_and_indent_scale() {
        // Mockup: tree rows are exactly 40px tall...
        assert!((theme::geometry::sidebar::ROW_H - 40.0).abs() < f32::EPSILON);
        // ...on the three-level indent scale 12/28/44px, one 16px step per
        // level. Half the mockup's 12/44/80: at the wider scale a deep folder
        // tree spent a third of the 280px column on nesting alone.
        assert!((sidebar::indent_px(0) - 12.0).abs() < f32::EPSILON);
        assert!((sidebar::indent_px(1) - 28.0).abs() < f32::EPSILON);
        assert!((sidebar::indent_px(2) - 44.0).abs() < f32::EPSILON);
        // Deeper nesting keeps stepping at the same rate so deep trees never
        // fold into one edge.
        assert!((sidebar::indent_px(3) - 60.0).abs() < f32::EPSILON);
        assert!((sidebar::indent_px(4) - 76.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_equalizer_bar_heights_animate_over_time_within_bounds() {
        let t0 = sidebar::equalizer_heights(0.0);
        let t1 = sidebar::equalizer_heights(0.35);
        let t2 = sidebar::equalizer_heights(1.7);

        let arrays_differ =
            |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).any(|(x, y)| (x - y).abs() > f32::EPSILON);
        assert!(arrays_differ(t0, t1), "bars must move as time advances");
        assert!(
            arrays_differ(t1, t2),
            "bars must keep moving past one cycle"
        );
        for h in t0.into_iter().chain(t1).chain(t2) {
            assert!(
                (0.0..=1.0).contains(&h),
                "bar heights are normalized: got {h}"
            );
        }
    }

    #[test]
    fn test_search_box_ring_uses_the_focus_token_only_when_focused() {
        let dark = theme::Palette::dark();

        let idle = sidebar::search_ring_stroke(&dark, false);
        assert_eq!(
            idle.color, dark.border,
            "idle border comes from the line token"
        );

        let focused = sidebar::search_ring_stroke(&dark, true);
        assert_eq!(
            focused.color, dark.focus_ring,
            "the focus ring is the palette's ring token"
        );
        assert!(
            focused.width > idle.width,
            "the focused ring reads stronger than the hairline border"
        );
    }

    #[test]
    fn test_tree_row_reports_clicks_for_selection() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, events: &mut Vec<&'static str>| {
                    let row = sidebar::tree_row(
                        ui,
                        &mut cache,
                        &palette,
                        sidebar::TreeRow {
                            indent_level: 1,
                            icon: Some(icons::Icon::Music),
                            cover: None,
                            label: "All Tracks",
                            count: None,
                            meta: None,
                            favorite: None,
                            selected: false,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                    if row.response.clicked() {
                        events.push("clicked");
                    }
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("All Tracks").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec!["clicked"],
            "a row click must be observable so selection keeps working"
        );
    }

    #[test]
    fn test_tree_row_favorite_control_owns_its_click_and_reports_the_new_flag() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, events: &mut Vec<&'static str>| {
                    let row = sidebar::tree_row(
                        ui,
                        &mut cache,
                        &palette,
                        sidebar::TreeRow {
                            indent_level: 0,
                            icon: None,
                            cover: None,
                            label: "All Tracks",
                            count: None,
                            meta: None,
                            favorite: Some(false),
                            selected: false,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                    if row.response.clicked() {
                        events.push("row");
                    }
                    if let Some(favorite) = row.favorite_toggled {
                        events.push(if favorite {
                            "favorite on"
                        } else {
                            "favorite off"
                        });
                    }
                },
                Vec::new(),
            );
        harness.run();

        // The heart owns its click: it reports the flag's NEW value, and the
        // row it sits in must NOT also select (its click area starts to the
        // right of the heart's cell).
        harness.get_by_label("Add to Favorites").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec!["favorite on"],
            "a heart click toggles the flag and never selects the row"
        );

        // The row body keeps its own gesture, unchanged.
        harness.get_by_label("All Tracks").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec!["favorite on", "row"],
            "clicking the row body still reports the row's click"
        );
    }

    #[test]
    fn test_tree_row_live_count_shows_in_the_row_label() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, _events: &mut Vec<()>| {
                    // The LIBRARY nav rows pass the live count from the counts
                    // read model; the row paints it and exposes it in the
                    // accessibility label so it is queryable.
                    sidebar::tree_row(
                        ui,
                        &mut cache,
                        &palette,
                        sidebar::TreeRow {
                            indent_level: 0,
                            icon: Some(icons::Icon::ListMusic),
                            cover: None,
                            label: "All Tracks",
                            count: Some(12),
                            meta: None,
                            favorite: None,
                            selected: false,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                    sidebar::tree_row(
                        ui,
                        &mut cache,
                        &palette,
                        sidebar::TreeRow {
                            indent_level: 0,
                            icon: Some(icons::Icon::Music),
                            cover: None,
                            label: "Artists",
                            count: None,
                            meta: None,
                            favorite: None,
                            selected: false,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label("All Tracks (12)").is_some(),
            "a counted row exposes its live count in the accessibility label"
        );
        assert!(
            harness.query_by_label("Artists").is_some(),
            "a countless row keeps its bare label"
        );
    }

    // --- Smart lists: always visible, two Advanced-only (handoff issue 07) -----
    //
    // The sidebar's SMART LISTS section shows Recently Added, Recently Played,
    // Most Played, and Favorites without Advanced mode; Never Played and Lost
    // Gems relocate behind Advanced (relocated, not deleted). The open rule
    // matches the visibility rule: an Advanced-only list can only open while
    // Advanced mode is on (e.g. it was left open when the toggle flipped off).

    #[test]
    fn test_smart_list_kinds_show_the_core_four_without_advanced_mode() {
        use crate::domain::SmartPlaylistKind;
        use riff_gui::ui::app::smart_list_kinds;

        let kinds = smart_list_kinds(false);
        assert_eq!(
            kinds,
            vec![
                SmartPlaylistKind::RecentlyAdded,
                SmartPlaylistKind::RecentlyPlayed,
                SmartPlaylistKind::MostPlayed,
                SmartPlaylistKind::Favorites,
            ],
            "the four core smart lists render in the design's order \
             without Advanced mode"
        );
    }

    #[test]
    fn test_smart_list_kinds_relocate_never_played_and_lost_gems_behind_advanced() {
        use crate::domain::SmartPlaylistKind;
        use riff_gui::ui::app::smart_list_kinds;

        let kinds = smart_list_kinds(true);
        assert!(kinds.contains(&SmartPlaylistKind::NeverPlayed));
        assert!(kinds.contains(&SmartPlaylistKind::LostGems));
        assert_eq!(kinds.len(), 6, "Advanced mode shows every smart list");
    }

    #[test]
    fn test_smart_list_openability_follows_the_visibility_rule() {
        use crate::domain::SmartPlaylistKind;
        use riff_gui::ui::app::smart_list_openable;

        for kind in [
            SmartPlaylistKind::RecentlyAdded,
            SmartPlaylistKind::RecentlyPlayed,
            SmartPlaylistKind::MostPlayed,
            SmartPlaylistKind::Favorites,
        ] {
            assert!(
                smart_list_openable(kind, false),
                "core list {kind:?} opens without Advanced mode"
            );
        }
        assert!(
            !smart_list_openable(SmartPlaylistKind::NeverPlayed, false),
            "Never Played stays closed while Advanced mode is off"
        );
        assert!(
            !smart_list_openable(SmartPlaylistKind::LostGems, false),
            "Lost Gems stays closed while Advanced mode is off"
        );
        for kind in SmartPlaylistKind::ALL {
            assert!(
                smart_list_openable(kind, true),
                "every list opens once Advanced mode is on"
            );
        }
    }

    // --- Sidebar footer: "Last scan X ago" (handoff issue 07) -------------------

    #[test]
    fn test_last_scan_age_formats_the_coarse_human_buckets() {
        use std::time::Duration;

        let fmt = riff_gui::ui::sidebar::format_last_scan_ago;
        assert_eq!(fmt(Duration::from_secs(0)), "just now");
        assert_eq!(fmt(Duration::from_secs(59)), "just now");
        assert_eq!(fmt(Duration::from_secs(60)), "1m ago");
        assert_eq!(fmt(Duration::from_secs(5 * 60 + 59)), "5m ago");
        assert_eq!(fmt(Duration::from_secs(3 * 3600)), "3h ago");
        assert_eq!(fmt(Duration::from_secs(23 * 3600 + 59 * 60)), "23h ago");
        assert_eq!(fmt(Duration::from_secs(2 * 86_400)), "2d ago");
        assert_eq!(fmt(Duration::from_secs(40 * 86_400)), "40d ago");
    }

    #[test]
    fn test_sidebar_footer_reports_add_folder_and_shows_the_scan_stamp() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, clicks: &mut Vec<&'static str>| {
                    if riff_gui::ui::sidebar::sidebar_footer(
                        ui,
                        &mut cache,
                        &palette,
                        Some("Last scan 5m ago"),
                    ) {
                        clicks.push("add_folder");
                    }
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Add folder").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec!["add_folder"],
            "the footer's Add folder action must be clickable"
        );
        assert!(
            harness.query_by_label("Last scan 5m ago").is_some(),
            "the footer shows the last-scan stamp"
        );
    }

    #[test]
    fn test_sidebar_footer_renders_without_a_scan_stamp() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, _clicks: &mut Vec<&'static str>| {
                    riff_gui::ui::sidebar::sidebar_footer(ui, &mut cache, &palette, None);
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label_contains("Last scan").is_none(),
            "no stamp renders before any scan has completed"
        );
        assert!(
            harness.query_by_label("Add folder").is_some(),
            "Add folder stays available without a scan stamp"
        );
    }

    #[test]
    fn test_playlist_row_hover_reveal_reports_open_edit_delete() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::sidebar::PlaylistRowAction;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlaylistRowAction>| {
                    if let Some(action) =
                        sidebar::playlist_row(ui, &mut cache, &palette, "Gym", "Gym (3)", false)
                    {
                        actions.push(action);
                    }
                },
                Vec::new(),
            );
        harness.run();

        // The hover-revealed affordances stay in the accessibility tree so
        // they are reachable (and clickable) even before the pointer hovers.
        harness.get_by_label("Rename playlist").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![PlaylistRowAction::Rename],
            "the pencil affordance must report Rename"
        );

        harness.get_by_label("Delete playlist").click();
        harness.run();
        assert!(
            harness.state().contains(&PlaylistRowAction::Delete),
            "the trash affordance must report Delete"
        );

        harness.get_by_label("Gym").click();
        harness.run();
        assert!(
            harness.state().contains(&PlaylistRowAction::Open),
            "clicking the row itself opens the playlist"
        );
    }

    // --- Playlist hover actions drive the existing Store flows -----------------
    //
    // ADR 0002: writes commit to the Store and nothing else — the seam's
    // playlist projection invalidates itself via the mutation adapter's
    // generation bump, so the next `views.playlists()` read reflects the
    // commit with zero caller action. The restyled rows report actions;
    // these tests pin that the action handler drives the SAME rename/delete
    // Store flows the pre-restyle buttons used.

    #[test]
    fn test_playlist_row_delete_action_commits_through_store_and_refreshes_projection() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut views) = boxed_playlist_seam(&dir);
        let _keep = store.create_playlist("Keep", &[]).unwrap();
        let gone = store.create_playlist("Gone", &[]).unwrap();

        let mut view = Some(gone.clone());
        let mut smart_view = None;
        let mut rename_slot = None;
        let mut create_slot = None;

        riff_gui::ui::app::apply_playlist_row_action(
            sidebar::PlaylistRowAction::Delete,
            &gone,
            store.as_mut(),
            &mut views,
            riff_gui::ui::app::PlaylistPromptSlots {
                view: &mut view,
                smart_view: &mut smart_view,
                rename: &mut rename_slot,
                create_name: &mut create_slot,
            },
        );

        assert!(
            !store.load_playlists().unwrap().iter().any(|p| p.id == gone),
            "the delete committed through the PlaylistStore"
        );
        let playlists = views.playlists();
        assert_eq!(
            playlists.len(),
            1,
            "the projection refreshed from the store after the committed write"
        );
        assert_eq!(
            playlists[0].name, "Keep",
            "only the deleted playlist went away"
        );
        assert_eq!(
            view, None,
            "deleting the open playlist closes it (pre-restyle behavior)"
        );
        assert!(
            rename_slot.is_none(),
            "delete never opens the rename prompt"
        );
    }

    #[test]
    fn test_playlist_row_rename_action_opens_the_existing_rename_prompt_flow() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut views) = boxed_playlist_seam(&dir);
        let pid = store.create_playlist("Gym", &[]).unwrap();

        let mut view = None;
        let mut smart_view = None;
        let mut rename_slot = None;
        let mut create_slot = Some(String::new());

        riff_gui::ui::app::apply_playlist_row_action(
            sidebar::PlaylistRowAction::Rename,
            &pid,
            store.as_mut(),
            &mut views,
            riff_gui::ui::app::PlaylistPromptSlots {
                view: &mut view,
                smart_view: &mut smart_view,
                rename: &mut rename_slot,
                create_name: &mut create_slot,
            },
        );

        assert_eq!(
            rename_slot,
            Some((pid.clone(), "Gym".to_string())),
            "the pencil affordance opens the existing inline rename prompt"
        );
        assert_eq!(
            create_slot, None,
            "opening rename closes the create prompt (pre-restyle behavior)"
        );
        assert!(
            store.load_playlists().unwrap()[0].name == "Gym",
            "rename alone commits nothing yet — Save does"
        );

        // Saving the prompt commits through the same Store flow; the seam's
        // next read reflects it with zero caller action (ADR 0002).
        riff_gui::ui::app::commit_playlist_rename(store.as_mut(), &pid, "  Cardio  ");
        assert_eq!(
            store.load_playlists().unwrap()[0].name,
            "Cardio",
            "the trimmed name persisted through the PlaylistStore"
        );
        assert_eq!(
            views.playlists()[0].name,
            "Cardio",
            "the projection refreshed after the committed rename"
        );
    }

    #[test]
    fn test_playlist_row_open_action_selects_the_playlist_view() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut views) = boxed_playlist_seam(&dir);
        let pid = store.create_playlist("Focus", &[]).unwrap();

        let mut view = None;
        let mut smart_view = Some(SmartPlaylistKind::MostPlayed);
        let mut rename_slot = None;
        let mut create_slot = None;

        riff_gui::ui::app::apply_playlist_row_action(
            sidebar::PlaylistRowAction::Open,
            &pid,
            store.as_mut(),
            &mut views,
            riff_gui::ui::app::PlaylistPromptSlots {
                view: &mut view,
                smart_view: &mut smart_view,
                rename: &mut rename_slot,
                create_name: &mut create_slot,
            },
        );

        assert_eq!(view, Some(pid), "opening selects the playlist view");
        assert_eq!(
            smart_view, None,
            "opening a user playlist closes any open smart playlist"
        );
    }

    // --- Player bar restyle (Issue 08) ------------------------------------------
    //
    // The playerbar matches the mockup: a 56×56 cover with a gradient
    // placeholder (Mesh strip from surface-2 to surface-3) fed by the existing
    // LRU texture cache, circular ghost transport buttons around a 40px
    // primary-filled play, a 4px seek row with fill and monospace time
    // readouts, a styled volume slider (4px track, round thumb), shuffle and
    // repeat toggles, and a queue position label. Every control still emits
    // its engine command.
    //
    // Headless seams (`riff_gui::ui::playerbar`): the mockup dimensions it
    // paints with (read from the token module), the monospace readout font,
    // the seek-fraction math, and the control→action contract. The pixels are
    // pinned by the `playerbar_dark` golden image; the action→command wiring is
    // covered further below.

    use riff_gui::ui::playerbar;

    #[test]
    fn test_playerbar_dimensions_match_the_mockup() {
        use theme::geometry::{playerbar as pb, seek};
        // Mockup: a 56×56 cover...
        assert!((pb::COVER - 56.0).abs() < f32::EPSILON);
        // ...a 40px primary-filled play button among circular ghost
        // transport buttons...
        assert!((pb::PLAY_BTN - 40.0).abs() < f32::EPSILON);
        assert!((pb::GHOST_BTN - 32.0).abs() < f32::EPSILON);
        // ...and 4px tracks for both the seek row and the volume slider.
        assert!((seek::TRACK_H - 4.0).abs() < f32::EPSILON);
        // The now-playing zone is elastic with a floor: widest 250px, never
        // narrower than 140px before it sheds lines.
        assert!((pb::NOW_PLAYING_W - 250.0).abs() < f32::EPSILON);
        assert!((pb::NOW_PLAYING_MIN_W - 140.0).abs() < f32::EPSILON);
        // The protected center column must always fit the transport row
        // (ghost + 12 + play + 12 + ghost) with room to spare.
        let transport_w = pb::GHOST_BTN * 2.0 + pb::PLAY_BTN + 12.0 * 2.0;
        assert!(
            pb::CENTER_MIN_W >= transport_w,
            "the center floor ({}) must fit the {}px transport row",
            pb::CENTER_MIN_W,
            transport_w
        );
    }

    #[test]
    fn test_time_readouts_render_in_the_monospace_family() {
        // Acceptance: elapsed/total times render in the monospace family so
        // digits align while counting. Mirrors the hero_title_font precedent:
        // view code references the scale by name instead of hardcoding it.
        let font = playerbar::time_font();
        assert_eq!(font.family, egui::FontFamily::Monospace);
        assert!((font.size - theme::TEXT_XS).abs() < f32::EPSILON);
    }

    #[test]
    fn test_seek_fraction_computes_clamped_progress() {
        use std::time::Duration;

        let total = Some(Duration::from_secs(200));
        assert!(crate::test_utils::float_close(
            playerbar::seek_fraction(Duration::from_secs(50), total),
            0.25
        ));
        // No total (unknown duration) reads as no progress.
        assert!(
            crate::test_utils::float_close(
                playerbar::seek_fraction(Duration::from_secs(50), None),
                0.0
            ),
            "unknown totals read as no progress"
        );
        // Zero-length totals never divide by zero.
        assert!(
            crate::test_utils::float_close(
                playerbar::seek_fraction(Duration::from_secs(50), Some(Duration::ZERO)),
                0.0
            ),
            "zero-length totals read as no progress"
        );
        // Progress clamps into 0..=1 no matter what the engine reports.
        assert!(
            crate::test_utils::float_close(
                playerbar::seek_fraction(Duration::from_secs(999), total),
                1.0
            ),
            "past-end positions clamp to full progress"
        );
    }

    /// Representative bar content for interaction tests: playing, two minutes
    /// into a 245s track, mid volume, shuffle on, nothing muted, with a
    /// current track titled "Roygbiv".
    fn playing_content() -> playerbar::PlayerBarContent<'static> {
        playerbar::PlayerBarContent {
            cover: None,
            title: Some("Roygbiv".into()),
            meta_line: Some("Boards of Canada".into()),
            playback: PlaybackState::Playing,
            position: std::time::Duration::from_mins(2),
            total: Some(std::time::Duration::from_secs(245)),
            volume: 0.65,
            muted: false,
            shuffle: true,
            repeat: RepeatMode::None,
            queue_position: "3/12",
            queue_open: false,
            expanded: false,
            advanced: false,
        }
    }

    /// Draw one playerbar frame into the caller's retained readout/buffer
    /// handles — the shared body of the harness closures below.
    fn draw_playerbar_frame(
        ui: &mut egui::Ui,
        cache: &mut icons::IconCache,
        palette: &theme::Palette,
        content: &playerbar::PlayerBarContent<'static>,
        readouts: &mut riff_gui::ui::playerbar::SeekReadouts,
        buf: &mut Vec<PlayerBarAction>,
    ) {
        buf.clear();
        playerbar::show_player_bar(ui, cache, palette, content, readouts, buf);
    }

    /// Run a playerbar harness against `content` and click each label in
    /// turn, asserting its expected action was reported (actions accumulate
    /// across frames; `harness.run()` settles between clicks).
    fn click_playerbar_sequence(
        palette: &theme::Palette,
        content: &playerbar::PlayerBarContent<'static>,
        clicks: &[(&str, PlayerBarAction)],
    ) {
        use egui_kittest::kittest::Queryable;

        let mut cache = icons::IconCache::new();
        let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
        let mut buf = Vec::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    buf.clear();
                    draw_playerbar_frame(ui, &mut cache, palette, content, &mut readouts, &mut buf);
                    actions.append(&mut buf);
                },
                Vec::new(),
            );
        harness.run();
        for (label, expected) in clicks {
            harness.get_by_label(label).click();
            harness.run();
            assert!(
                harness.state().contains(expected),
                "clicking {label:?} must report {expected:?}, got {:?}",
                harness.state()
            );
        }
    }

    /// The now-playing zone (cover + text column) is one hit target that
    /// reports the existing expand intent, reached through its full,
    /// un-elided label (issue 03).
    #[test]
    fn test_now_playing_zone_click_reports_toggle_expanded() {
        use riff_gui::ui::playerbar::PlayerBarAction;

        let palette = theme::Palette::dark();
        click_playerbar_sequence(
            &palette,
            &playing_content(),
            &[(
                "Roygbiv — Boards of Canada",
                PlayerBarAction::ToggleExpanded,
            )],
        );

        // Idle: the zone is still a hit target under the idle copy.
        let idle = playerbar::PlayerBarContent {
            title: None,
            meta_line: None,
            ..playing_content()
        };
        click_playerbar_sequence(
            &palette,
            &idle,
            &[("Nothing playing", PlayerBarAction::ToggleExpanded)],
        );
    }

    /// The layout contract (issue 05): the zone width rides its elastic band
    /// at every width that affords it, yields to the protected center column
    /// when it cannot, and the painted zone never overlaps the center — the
    /// bar degrades in the fixed order down to cover-only.
    #[test]
    fn test_playerbar_now_playing_zone_geometry() {
        use theme::geometry::playerbar as pb;

        // The fixed right cluster's reservation, per the mockup contract:
        // expand 32+16, volume 90+8, mute 32+14, queue 32+6, repeat 32+6,
        // shuffle 32+10, queue label 52 = 362.
        let cluster_w = 362.0;
        let zone_w = |window_w: f32| playerbar::now_playing_zone_width(window_w - 32.0, cluster_w);
        // What the bar actually paints for that zone width: below
        // `MIN_TEXT_W` of text room the column is shed and the cover stands
        // alone (issue 01's degradation order).
        let painted_w = |window_w: f32| {
            let zone = zone_w(window_w);
            if zone - pb::COVER - pb::NOW_PLAYING_GAP < pb::MIN_TEXT_W {
                pb::COVER
            } else {
                zone
            }
        };
        // The center column: what is left of the painted zone and the two
        // 20px gaps bracketing it.
        let center_w = |window_w: f32| window_w - 32.0 - painted_w(window_w) - cluster_w - 40.0;

        // The elastic band at the widths that afford it: 214 at the 800px
        // harness (768 − 362 − 40 − 152), the 250px cap above that.
        assert!(crate::test_utils::float_close(zone_w(800.0), 214.0));
        for width in [1100.0, 1600.0] {
            assert!(crate::test_utils::float_close(
                zone_w(width),
                pb::NOW_PLAYING_W
            ));
        }
        // The protected center column holds at every width the shell can
        // reach (MIN_WINDOW_SIZE.x is 800) — exactly CENTER_MIN_W at 800.
        assert!(crate::test_utils::float_close(
            center_w(800.0),
            pb::CENTER_MIN_W
        ));
        for width in [800.0, 1100.0, 1600.0] {
            assert!(
                center_w(width) >= pb::CENTER_MIN_W - f32::EPSILON,
                "center column {}px at {width}px must not fall below CENTER_MIN_W",
                center_w(width)
            );
        }
        // Below the floor's affordability the zone yields rather than
        // overlapping: at 640 the formula's room (54) wins over the 140
        // floor, the text column is shed to the bare cover, and the center
        // still fits the 128px transport row.
        let room_640 = 640.0 - 32.0 - cluster_w - 40.0 - pb::CENTER_MIN_W;
        assert!(crate::test_utils::float_close(zone_w(640.0), room_640));
        assert!(room_640 < pb::NOW_PLAYING_MIN_W);
        assert!(crate::test_utils::float_close(painted_w(640.0), pb::COVER));
        let transport_w = pb::GHOST_BTN * 2.0 + pb::PLAY_BTN + 12.0 * 2.0;
        assert!(
            center_w(640.0) >= transport_w,
            "the center column must still fit the transport row at 640px, got {}",
            center_w(640.0)
        );
        // Never negative, at any width.
        for width in [640.0, 800.0, 1100.0, 1600.0] {
            assert!((0.0..=pb::NOW_PLAYING_W).contains(&zone_w(width)));
        }
    }

    /// A single-row (issue 04) layout: an unbroken token far wider than the
    /// column still elides to exactly one row inside the given width — the
    /// text block's height is constant, so a long title can never grow the
    /// bar.
    #[test]
    fn test_now_playing_text_is_one_row_and_elided() {
        let long_title = "Riffmixtape_2026_".repeat(12); // 192 chars, no break opportunity
        let ctx = egui::Context::default();
        // egui installs its fonts during the first `run`, so layout needs one frame.
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let job = playerbar::now_playing_text_job(
            &long_title,
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
            150.0,
        );
        let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
        assert_eq!(
            galley.rows.len(),
            1,
            "an over-long title must lay out as exactly one row"
        );
        assert!(galley.elided, "the row must be marked truncated");
        assert!(
            galley.size().x <= 150.0,
            "the elided row must fit the column, got {}",
            galley.size().x
        );
        assert!(
            galley.rows[0].text().ends_with('\u{2026}'),
            "the truncated row must carry the elide character"
        );
    }

    /// The round trip (issue 04): the painted title is elided at the bar's
    /// real width, but the accessible name — and with it the hover tooltip —
    /// carries the full, untruncated string.
    #[test]
    fn test_elided_title_keeps_full_accessible_label() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let long_title = "Riffmixtape_2026_".repeat(12);
        let content = playerbar::PlayerBarContent {
            title: Some(long_title.as_str().into()),
            ..playing_content()
        };
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness
                .query_by_label(&format!("{long_title} \u{2014} Boards of Canada"))
                .is_some(),
            "the zone's accessible name must be the full un-elided title + meta"
        );
    }

    #[test]
    fn test_transport_clicks_report_playback_actions() {
        use riff_gui::ui::playerbar::PlayerBarAction;

        let palette = theme::Palette::dark();

        // Playing: Previous and Next flank the primary Pause.
        click_playerbar_sequence(
            &palette,
            &playing_content(),
            &[
                ("Previous track", PlayerBarAction::Previous),
                ("Pause", PlayerBarAction::Pause),
                ("Next track", PlayerBarAction::Next),
            ],
        );

        // Paused: the same primary button reports Resume.
        let paused = playerbar::PlayerBarContent {
            playback: PlaybackState::Paused,
            ..playing_content()
        };
        click_playerbar_sequence(&palette, &paused, &[("Play", PlayerBarAction::Resume)]);

        // Stopped: the primary button asks the app to play the selection.
        let stopped = playerbar::PlayerBarContent {
            playback: PlaybackState::Stopped,
            ..playing_content()
        };
        click_playerbar_sequence(
            &palette,
            &stopped,
            &[("Play", PlayerBarAction::PlaySelected)],
        );
    }
    #[test]
    fn test_stop_button_only_exists_in_advanced_mode() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();

        // Advanced mode: Stop is present and reports Stop.
        let advanced = playerbar::PlayerBarContent {
            advanced: true,
            ..playing_content()
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &advanced,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();
        harness.get_by_label("Stop").click();
        harness.run();
        assert!(harness.state().contains(&PlayerBarAction::Stop));

        // Minimal mode: no Stop affordance at all (REQ-UI-006).
        let minimal = playing_content();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &minimal,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness.query_by_label("Stop").is_none(),
            "Stop stays an advanced-only affordance"
        );
    }

    #[test]
    fn test_shuffle_repeat_mute_report_toggle_actions() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let content = playing_content();
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Toggle shuffle").click();
        harness.run();
        assert!(harness.state().contains(&PlayerBarAction::ToggleShuffle));

        harness.get_by_label("Cycle repeat mode").click();
        harness.run();
        assert!(harness.state().contains(&PlayerBarAction::ToggleRepeat));

        harness.get_by_label("Mute").click();
        harness.run();
        assert!(harness.state().contains(&PlayerBarAction::ToggleMute));
    }

    #[test]
    fn test_queue_open_and_expand_buttons_report_toggle_actions() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        // Closed/rested bar: the queue button opens the queue panel and the
        // expand button enlarges the player view (handoff issue 13).
        let closed = playing_content();
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &closed,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Open queue").click();
        harness.run();
        assert!(
            harness.state().contains(&PlayerBarAction::ToggleQueue),
            "the queue-open button must report ToggleQueue, got {:?}",
            harness.state()
        );

        harness.get_by_label("Expand player").click();
        harness.run();
        assert!(
            harness.state().contains(&PlayerBarAction::ToggleExpanded),
            "the expand button must report ToggleExpanded, got {:?}",
            harness.state()
        );

        // Open/expanded bar: the same controls flip their labels with their
        // state (mute-button precedent) and keep reporting the toggles.
        let open = playerbar::PlayerBarContent {
            queue_open: true,
            expanded: true,
            ..playing_content()
        };
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &open,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Close queue").click();
        harness.run();
        assert!(harness.state().contains(&PlayerBarAction::ToggleQueue));

        harness.get_by_label("Exit expanded player").click();
        harness.run();
        assert!(harness.state().contains(&PlayerBarAction::ToggleExpanded));
    }

    #[test]
    fn test_keyboard_focus_reaches_the_queue_and_expand_buttons() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let content = playing_content();
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        // Both new controls are individually focusable and Enter activates
        // them — same report as a click (settings-nav precedent).
        harness.get_by_label("Open queue").focus();
        harness.run();
        assert!(harness.get_by_label("Open queue").is_focused());
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness.state().contains(&PlayerBarAction::ToggleQueue),
            "Enter on the focused queue button must toggle the queue, got {:?}",
            harness.state()
        );

        harness.get_by_label("Expand player").focus();
        harness.run();
        assert!(harness.get_by_label("Expand player").is_focused());
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness.state().contains(&PlayerBarAction::ToggleExpanded),
            "Enter on the focused expand button must toggle the expanded view, got {:?}",
            harness.state()
        );
    }

    /// Run `step` against a harness drawing the playerbar (queue open) plus
    /// the queue panel over it — the shell layout while the panel is
    /// revealed.
    fn with_queue_panel(
        palette: &theme::Palette,
        content: &playerbar::PlayerBarContent<'static>,
        entries: &[riff_gui::ui::now_playing::UpNextEntry],
        step: impl FnOnce(&mut egui_kittest::Harness<'_, Vec<riff_gui::ui::playerbar::PlayerBarAction>>),
    ) {
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        palette,
                        content,
                        &mut readouts,
                        &mut buf,
                    );
                    riff_gui::ui::playerbar::show_queue_panel(
                        ui, &mut cache, palette, entries, &mut buf,
                    );
                    actions.append(&mut buf);
                },
                Vec::new(),
            );
        step(&mut harness);
    }

    #[test]
    fn test_queue_panel_rows_queue_their_track_to_play_next() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::now_playing::UpNextEntry;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let palette = theme::Palette::dark();
        let open = playerbar::PlayerBarContent {
            queue_open: true,
            ..playing_content()
        };
        let entries = vec![
            UpNextEntry {
                id: TrackId("one.flac".to_string()),
                label: "A - One".to_string(),
            },
            UpNextEntry {
                id: TrackId("two.flac".to_string()),
                label: "B - Two".to_string(),
            },
        ];
        with_queue_panel(&palette, &open, &entries, |harness| {
            harness.run();

            harness.get_by_label("A - One").click();
            harness.run();
            assert!(
                harness
                    .state()
                    .contains(&PlayerBarAction::PlayNext(TrackId("one.flac".to_string()))),
                "clicking an Up Next row must queue that track, got {:?}",
                harness.state()
            );

            harness.get_by_label("B - Two").click();
            harness.run();
            assert!(
                harness
                    .state()
                    .contains(&PlayerBarAction::PlayNext(TrackId("two.flac".to_string()))),
                "every row queues its own track, got {:?}",
                harness.state()
            );
        });
    }

    #[test]
    fn test_transport_stays_live_while_the_queue_panel_is_open() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let palette = theme::Palette::dark();
        let open = playerbar::PlayerBarContent {
            queue_open: true,
            ..playing_content()
        };
        with_queue_panel(&palette, &open, &[], |harness| {
            harness.run();

            // The panel never blocks the bar: the transport underneath still
            // reports its actions (playing, seeking, and queue navigation keep
            // working from the revealed queue).
            harness.get_by_label("Pause").click();
            harness.run();
            assert!(
                harness.state().contains(&PlayerBarAction::Pause),
                "the transport stays reachable while the queue panel is open, got {:?}",
                harness.state()
            );
        });
    }

    #[test]
    fn test_queue_panel_shows_its_empty_state() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let open = playerbar::PlayerBarContent {
            queue_open: true,
            ..playing_content()
        };
        with_queue_panel(&palette, &open, &[], |harness| {
            harness.run();
            assert!(
                harness.query_by_label("Queue is empty").is_some(),
                "an empty queue reads as an empty state, not blank space"
            );
        });
    }

    #[test]
    fn test_seek_click_reports_seek_to_clicked_fraction() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let content = playing_content(); // total 245s
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        // A center click lands halfway along the bar.
        harness.get_by_label("Seek").click();
        harness.run();
        let expected = std::time::Duration::from_secs_f32(122.5);
        assert!(
            harness.state().contains(&PlayerBarAction::Seek(expected)),
            "clicking the seek row must report Seek at the clicked fraction, got {:?}",
            harness.state()
        );
    }

    #[test]
    fn test_volume_click_reports_set_volume_at_clicked_fraction() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::playerbar::PlayerBarAction;

        let content = playing_content();
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        // A center click sets volume to one half.
        harness.get_by_label("Volume").click();
        harness.run();
        assert!(
            harness
                .state()
                .iter()
                .any(|a| matches!(a, PlayerBarAction::SetVolume(v) if (*v - 0.5).abs() < 1e-3)),
            "clicking the volume slider must report SetVolume at the clicked fraction, got {:?}",
            harness.state()
        );
    }

    #[test]
    fn test_queue_position_label_renders() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::IconCache;

        let content = playing_content(); // "3/12"
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<riff_gui::ui::playerbar::PlayerBarAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label("3/12").is_some(),
            "the queue position label renders where the mockup places it"
        );
    }

    // --- Player bar actions drive the transport intents (Issue 08) ---------------
    //
    // "Every control still emits its engine command": the restyled widgets
    // report [`PlayerBarAction`]s and the app maps each one through the SAME
    // Transport intents and state paths the pre-restyle buttons used. These
    // tests pin that contract headlessly over a recording mock.

    use riff_gui::ui::app::apply_player_bar_action;
    use riff_gui::ui::playerbar::PlayerBarAction;

    use crate::mocks::TransportIntent;
    /// Apply one action against fresh `PlaybackSession` + `LibrarySession` +
    /// recording transport, returning all three for inspection. The two
    /// session values are the type the [`apply_player_bar_action`] function
    /// takes after the two-session split.
    #[allow(clippy::type_complexity)]
    fn applied(
        action: PlayerBarAction,
    ) -> (PlaybackSession, LibrarySession, crate::mocks::MockTransport) {
        let mut playback = PlaybackSession::default();
        let mut library = LibrarySession::default();
        let transport = crate::mocks::MockTransport::new();
        apply_player_bar_action(action, &mut library, &mut playback, &transport);
        (playback, library, transport)
    }

    #[test]
    fn test_transport_actions_emit_the_same_engine_commands() {
        // Straight pass-through intents: the Action → intent mapping table.
        for (action, expected) in [
            (PlayerBarAction::Previous, TransportIntent::Previous),
            (PlayerBarAction::Pause, TransportIntent::Pause),
            (PlayerBarAction::Resume, TransportIntent::Resume),
            (PlayerBarAction::Next, TransportIntent::Next),
            (PlayerBarAction::Stop, TransportIntent::Stop),
        ] {
            let (_, _, transport) = applied(action);
            assert_eq!(
                transport.recorded(),
                vec![expected.clone()],
                "{expected:?} must still be issued to the engine"
            );
        }
    }

    #[test]
    fn test_play_selected_plays_the_selected_track() {
        let (mut playback, mut library, transport) = applied(PlayerBarAction::PlaySelected); // no selection yet
        assert!(
            transport.recorded().is_empty(),
            "no selection means no play intent"
        );

        library.selected_track = Some(TrackId("song.flac".to_string()));
        apply_player_bar_action(
            PlayerBarAction::PlaySelected,
            &mut library,
            &mut playback,
            &transport,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::Play(TrackId("song.flac".to_string()))]
        );
    }
    #[test]
    fn test_seek_action_is_clamped_against_the_live_total() {
        let (mut playback, _, _) = applied(PlayerBarAction::Pause);
        playback.current_position.total = Some(std::time::Duration::from_secs(245));

        let transport = crate::mocks::MockTransport::new();
        apply_player_bar_action(
            PlayerBarAction::Seek(std::time::Duration::from_secs_f32(999.0)),
            &mut LibrarySession::default(),
            &mut playback,
            &transport,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::Seek(std::time::Duration::from_secs(245))],
            "an out-of-range seek target clamps to the track duration"
        );
    }

    #[test]
    fn test_volume_action_updates_the_session_and_sends_effective_volume() {
        let (playback, _, transport) = applied(PlayerBarAction::SetVolume(0.7));

        assert!(
            (playback.current_volume - 0.7).abs() < 1e-6,
            "slider value lands"
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::ApplyVolume(0.7)],
            "the engine hears the new volume"
        );
        // Durability is the frame-end `Preferences` diff commit's job
        // (covered by the app_tests preferences suite), not the handler's.
    }

    #[test]
    fn test_mute_toggle_sends_zero_and_keeps_slider_value() {
        let (mut playback, _, _) = applied(PlayerBarAction::SetVolume(0.7));
        let transport = crate::mocks::MockTransport::new();
        let mut library = LibrarySession::default();

        // Muting sends the muted (zero) volume to the engine...
        apply_player_bar_action(
            PlayerBarAction::ToggleMute,
            &mut library,
            &mut playback,
            &transport,
        );
        assert!(playback.muted);
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::ApplyVolume(0.0)],
            "muting zeroes what the engine hears"
        );
        assert!(
            (playback.current_volume - 0.7).abs() < 1e-6,
            "slider keeps its value"
        );

        // ...and a slider change while muted still edits current_volume
        // while the engine keeps receiving zero until unmuted.
        apply_player_bar_action(
            PlayerBarAction::SetVolume(0.9),
            &mut library,
            &mut playback,
            &transport,
        );
        assert!((playback.current_volume - 0.9).abs() < 1e-6);
        assert_eq!(
            transport.recorded().last(),
            Some(&TransportIntent::ApplyVolume(0.0))
        );

        // Unmuting restores the slider's value to the engine.
        apply_player_bar_action(
            PlayerBarAction::ToggleMute,
            &mut library,
            &mut playback,
            &transport,
        );
        assert!(!playback.muted);
        assert_eq!(
            transport.recorded().last(),
            Some(&TransportIntent::ApplyVolume(0.9)),
            "unmuting restores the slider's volume"
        );
    }

    #[test]
    fn test_shuffle_and_repeat_toggles_flip_queue_state() {
        let (mut playback, _, _) = applied(PlayerBarAction::Pause);
        let transport = crate::mocks::MockTransport::new();
        let mut library = LibrarySession::default();

        let was = playback.queue.shuffle;
        apply_player_bar_action(
            PlayerBarAction::ToggleShuffle,
            &mut library,
            &mut playback,
            &transport,
        );
        assert_ne!(playback.queue.shuffle, was, "shuffle flips");

        assert_eq!(playback.queue.repeat, RepeatMode::None);
        apply_player_bar_action(
            PlayerBarAction::ToggleRepeat,
            &mut library,
            &mut playback,
            &transport,
        );
        assert_eq!(
            playback.queue.repeat,
            RepeatMode::All,
            "repeat cycles off → all"
        );
    }

    #[test]
    fn test_queue_toggle_flips_the_session_flag_without_touching_playback() {
        let (playback, library, transport) = applied(PlayerBarAction::ToggleQueue);
        assert!(library.queue_open, "the queue panel opens");
        assert!(
            transport.recorded().is_empty(),
            "opening the queue issues no engine intent"
        );
        assert_eq!(
            playback.playback_state,
            PlaybackState::Stopped,
            "playback is untouched"
        );

        let (mut playback, mut library, transport) = applied(PlayerBarAction::Pause);
        library.queue_open = true;
        apply_player_bar_action(
            PlayerBarAction::ToggleQueue,
            &mut library,
            &mut playback,
            &transport,
        );
        assert!(!library.queue_open, "the queue panel closes again");
    }

    #[test]
    fn test_expand_toggle_routes_through_the_now_playing_mode() {
        // From the Library view: expanding lands on the enlarged player view.
        let (playback, library, transport) = applied(PlayerBarAction::ToggleExpanded);
        assert_eq!(
            library.view_mode,
            riff_backend::app::state::ViewMode::NowPlaying
        );
        assert!(
            transport.recorded().is_empty(),
            "expanding issues no engine intent"
        );
        assert_eq!(
            playback.playback_state,
            PlaybackState::Stopped,
            "playback state is preserved untouched"
        );

        // While expanded: the same button returns to the Library view.
        let (mut playback, mut library, transport) = applied(PlayerBarAction::Pause);
        library.view_mode = riff_backend::app::state::ViewMode::NowPlaying;
        apply_player_bar_action(
            PlayerBarAction::ToggleExpanded,
            &mut library,
            &mut playback,
            &transport,
        );
        assert_eq!(
            library.view_mode,
            riff_backend::app::state::ViewMode::Library,
            "leaving the enlarged view lands on the Library view"
        );
    }

    #[test]
    fn test_queue_panel_row_reports_play_next_for_its_track() {
        let (mut playback, mut library, _) = applied(PlayerBarAction::Pause);
        let transport = crate::mocks::MockTransport::new();
        apply_player_bar_action(
            PlayerBarAction::PlayNext(TrackId("next.flac".to_string())),
            &mut library,
            &mut playback,
            &transport,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::PlayNext(TrackId("next.flac".to_string()))],
            "a queue-panel row queues its track to play next"
        );
    }

    // --- Library stage empty-state hero (Issue 09) -------------------------------
    //
    // Independent sources of truth: the issue checklist plus the mockup's
    // index.html main-stage section — a 160px disc circle (`w-40 h-40`) with
    // an 80px glyph (`w-20 h-20`), `mb-6`/`mb-1` copy gaps inside a `p-8`
    // stage, verbatim hero copy — and its `.riff-disc-glow` rule
    // (`box-shadow: 0 0 60px -20px brand@15%`), approximated with layered
    // translucent fills because egui has no blur.

    use riff_gui::ui::library;

    #[test]
    fn test_library_hero_dimensions_match_the_mockup_stage() {
        use theme::geometry::hero;
        // w-40 h-40 disc circle with an 80px (w-20 h-20) glyph.
        assert!((hero::DISC_SIZE - 160.0).abs() < f32::EPSILON);
        assert!((hero::DISC_ICON_SIZE - 80.0).abs() < f32::EPSILON);
        // mb-6 below the circle, mb-1 between title and subtitle, p-8 inset.
        assert!((hero::TITLE_GAP - 24.0).abs() < f32::EPSILON);
        assert!((hero::SUBTITLE_GAP - 4.0).abs() < f32::EPSILON);
        assert!((hero::STAGE_INSET - 32.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_library_hero_copy_matches_the_mockup_verbatim() {
        assert_eq!(library::HERO_TITLE, "Select a track to view details");
        assert_eq!(
            library::HERO_SUBTITLE,
            "Your library is ready. Choose something from the sidebar."
        );
    }

    #[test]
    fn test_disc_glow_layers_approximate_the_mockup_shadow() {
        // `.riff-disc-glow`: box-shadow 0 0 60px -20px brand@15%. The layered
        // approximation must stack several translucent fills, painted
        // largest-first, whose brand alphas fall off toward the outside and
        // never exceed the CSS shadow's 15% ceiling.
        let layers = theme::geometry::glow::LAYERS;
        assert!(
            layers.len() >= 2,
            "a single flat fill cannot stand in for a blur"
        );
        for pair in layers.windows(2) {
            assert!(
                pair[0].spread > pair[1].spread,
                "layers are declared largest-first so painting order stacks them"
            );
            assert!(
                pair[0].alpha < pair[1].alpha,
                "alpha falls off toward the outside of the glow"
            );
        }
        for layer in &layers {
            assert!(layer.spread > 0.0, "each layer extends past the disc edge");
            assert!(
                layer.alpha > 0.0 && layer.alpha <= 0.15,
                "brand alpha stays within the mockup shadow's 15% peak"
            );
        }
    }

    #[test]
    fn test_disc_glow_color_is_derived_from_the_brand_token() {
        // ADR 0004: no flat color literals in view code — every glow tint is
        // the palette's brand primary scaled by the layer's alpha fraction,
        // and the scaling is the token module's helper, not a call site.
        let palette = theme::Palette::dark();
        for layer in &theme::geometry::glow::LAYERS {
            assert_eq!(
                theme::glow(&palette, layer.alpha),
                palette.brand_primary.gamma_multiply(layer.alpha),
                "the glow tint derives from brand_primary"
            );
        }
    }

    // --- Now Playing restyle (Issue 10) ------------------------------------------
    //
    // The mockup's now-playing.html stage: a 240px cover with the
    // extra-large radius and the layered brand glow, the 3xl title, a meta
    // line, and Up Next rows reflecting the Playback Queue order. Now Playing
    // is a MODE that replaces the active View (resolved gaps), so its close
    // button always returns to the Library View no matter which View was up
    // before.

    use riff_gui::ui::app::apply_now_playing_action;
    use riff_gui::ui::now_playing::{self, NowPlayingAction, UpNextEntry};

    #[test]
    fn test_now_playing_cover_uses_the_mockup_dimension() {
        assert!(
            (theme::geometry::now_playing::COVER_SIZE - 240.0).abs() < f32::EPSILON,
            "the mockup cover is exactly 240px"
        );
    }

    #[test]
    fn test_now_playing_close_always_returns_to_the_library_view() {
        use riff_backend::app::state::{BrowseMode, ViewMode};

        for start in [ViewMode::Library, ViewMode::NowPlaying, ViewMode::Settings] {
            let mut library = LibrarySession {
                view_mode: start,
                browse_mode: BrowseMode::Folders,
                ..LibrarySession::default()
            };

            let transport = crate::mocks::MockTransport::new();
            apply_now_playing_action(
                NowPlayingAction::Close,
                &mut library,
                &PlaybackSession::default(),
                &transport,
            );

            assert_eq!(
                library.view_mode,
                ViewMode::Library,
                "closing Now Playing from {start:?} must land on the Library View"
            );
            assert!(
                transport.recorded().is_empty(),
                "closing is pure navigation; it never touches the engine"
            );
        }
    }

    #[test]
    fn test_now_playing_play_next_queues_the_clicked_track() {
        let transport = crate::mocks::MockTransport::new();
        apply_now_playing_action(
            NowPlayingAction::PlayNext(TrackId("t9.mp3".to_string())),
            &mut LibrarySession::default(),
            &PlaybackSession::default(),
            &transport,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::PlayNext(TrackId("t9.mp3".to_string()))],
            "clicking an Up Next row queues it via the SAME PlayNext intent as before"
        );
    }

    #[test]
    fn test_now_playing_seek_action_clamps_against_the_live_total() {
        let playback = PlaybackSession::default();
        let mut library = LibrarySession::default();
        let transport = crate::mocks::MockTransport::new();
        apply_now_playing_action(
            NowPlayingAction::Seek(std::time::Duration::from_secs_f32(999.0)),
            &mut library,
            &{
                let mut p = playback;
                p.current_position.total = Some(std::time::Duration::from_secs(100));
                p
            },
            &transport,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::Seek(std::time::Duration::from_secs(100))],
            "the in-view seek clamps exactly like the playerbar's"
        );
    }

    /// A resolved Up Next window of three tagged tracks — what the playback
    /// projection hands over for a four-track queue playing the first. The
    /// queue-to-window mapping itself is covered by the app-layer
    /// `PlaybackProjection` tests.
    fn up_next_window_fixture() -> Vec<Track> {
        (2..=4)
            .map(|i| {
                crate::test_utils::create_test_track_with_metadata(
                    &format!("t{i}.mp3"),
                    &format!("music/t{i}.mp3"),
                    "Artist",
                    &format!("Song {i}"),
                    "Album",
                )
            })
            .collect()
    }

    #[test]
    fn test_up_next_entries_format_the_resolved_window_in_order() {
        let window = up_next_window_fixture();

        let rows = now_playing::up_next_entries(&window, 5);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.0.as_str()).collect();
        assert_eq!(
            ids,
            vec!["t2.mp3", "t3.mp3", "t4.mp3"],
            "Up Next rows keep the resolved window's order"
        );
        assert_eq!(
            rows[0].label, "Artist - Song 2",
            "each row is preformatted as \"Artist - Title\""
        );

        // The limit caps how many rows are built.
        assert_eq!(now_playing::up_next_entries(&window, 2).len(), 2);
    }

    #[test]
    fn test_up_next_entries_empty_for_an_empty_window() {
        assert!(now_playing::up_next_entries(&[], 5).is_empty());
    }

    /// The neutral `up_next` module is the single owner of the Up Next entry
    /// model and label builder: `now_playing`'s historical path is a re-export
    /// of it, not a second copy. This guards against the two surfaces
    /// (Now Playing and the queue sheet) silently diverging back into owning
    /// their own row data — the coupling ticket 05 removed.
    #[test]
    fn test_up_next_model_has_one_owner_shared_by_both_surfaces() {
        use riff_gui::ui::up_next;
        let window = up_next_window_fixture();

        let neutral = up_next::up_next_entries(&window, 5);
        let via_now_playing = now_playing::up_next_entries(&window, 5);
        assert_eq!(
            neutral, via_now_playing,
            "the re-exported `now_playing::` path and the neutral `up_next::` \
             path build identical rows — one owner, not two copies"
        );
        // The entry type is literally the same type across both paths.
        let _: &up_next::UpNextEntry = &via_now_playing[0];
    }

    #[test]
    fn test_metadata_details_line_hides_missing_fields() {
        use riff_backend::domain::TrackMetadata;

        // Everything present: year · genre · track/disc.
        let full = TrackMetadata {
            year: Some(2013),
            genre: Some("Synthwave".to_string()),
            track_number: Some(1),
            disc_number: Some(2),
            ..TrackMetadata::default()
        };
        assert_eq!(
            now_playing::metadata_details(&full).as_deref(),
            Some("2013 \u{b7} Synthwave \u{b7} Track 1 / Disc 2")
        );

        // Missing fields are hidden, never shown as "Unknown" (spec).
        let bare = TrackMetadata::default();
        assert_eq!(now_playing::metadata_details(&bare), None);
    }

    #[test]
    fn test_now_playing_close_button_reports_the_close_action() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let content = now_playing::NowPlayingContent {
            cover: None,
            title: Some("Nightcall".into()),
            meta_line: Some("Kavinsky - OutRun".into()),
            details: None,
            position: std::time::Duration::from_secs(83),
            total: Some(std::time::Duration::from_mins(4)),
            up_next: Vec::new().into(),
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(520.0, 456.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<NowPlayingAction>| {
                    // ACCUMULATE across frames: a click fires its action on
                    // exactly one frame; harness.run() settles afterwards.
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    now_playing::show_now_playing(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Close Now Playing").click();
        harness.run();
        assert!(
            harness.state().contains(&NowPlayingAction::Close),
            "the stage's own close affordance reports Close"
        );
    }

    #[test]
    fn test_now_playing_up_next_row_click_reports_play_next() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let content = now_playing::NowPlayingContent {
            cover: None,
            title: Some("Nightcall".into()),
            meta_line: Some("Kavinsky - OutRun".into()),
            details: None,
            position: std::time::Duration::from_secs(83),
            total: Some(std::time::Duration::from_mins(4)),
            up_next: vec![
                UpNextEntry {
                    id: TrackId("a.flac".to_string()),
                    label: "Artist - Alpha".to_string(),
                },
                UpNextEntry {
                    id: TrackId("b.flac".to_string()),
                    label: "Artist - Beta".to_string(),
                },
            ]
            .into(),
        };
        let mut harness = egui_kittest::Harness::builder()
            // Tall enough that both Up Next rows fit below the fixed cover +
            // copy block without scrolling (the min-window stage would clip
            // the second row out of the scroll view).
            .with_size(egui::vec2(520.0, 680.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<NowPlayingAction>| {
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    now_playing::show_now_playing(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    actions.extend(buf);
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Artist - Beta").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&NowPlayingAction::PlayNext(TrackId("b.flac".to_string()))),
            "clicking an Up Next row reports PlayNext for THAT row"
        );
    }

    // --- Settings stage + ToggleSwitch (Issue 11) ---------------------------------
    //
    // Independent sources of truth: the issue checklist plus the mockup's
    // settings.html main stage — a Back button + xl heading, a Music Libraries
    // card whose rows carry a per-path Readiness dot (`w-2 h-2`, state colors)
    // beside Scan / Watch / trash actions, Add Library + Scan All below, a
    // destructive ghost button ("text-destructive hover:bg-destructive/10"),
    // and a Preferences card of three rows driven by a 36×20 pill toggle
    // switch (`w-9 h-5`, 16px knob at a 2px inset, `peer-checked:bg-primary`)
    // sliding 16px when checked.
    //
    // Readiness is its own concept (CONTEXT.md): whether the path is present
    // on disk AND indexed into the Library — never its Watch State.

    use riff_gui::ui::settings::{self, LibraryRow, Readiness, SettingsAction, SettingsContent};
    use riff_gui::ui::toggle_switch;

    #[test]
    fn test_toggle_switch_dimensions_match_the_mockup_pill() {
        use theme::geometry::toggle;
        // w-9 h-5 pill with a w-4 h-4 knob inset by 0.5 (2px).
        assert!((toggle::TOGGLE_W - 36.0).abs() < f32::EPSILON);
        assert!((toggle::TOGGLE_H - 20.0).abs() < f32::EPSILON);
        assert!((toggle::KNOB_SIZE - 16.0).abs() < f32::EPSILON);
        assert!((toggle::KNOB_INSET - 2.0).abs() < f32::EPSILON);
        // peer-checked:translate-x-4 — the knob slides exactly 16px.
        assert!((toggle::KNOB_TRAVEL - 16.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_toggle_switch_colors_derive_from_the_palette_tokens() {
        // bg-input (aliases surface-2) unchecked, bg-primary checked,
        // primary-foreground knob — resolved through Palette so both palettes
        // and High Contrast re-theme the widget (ADR 0004).
        let dark = theme::Palette::dark();
        assert_eq!(
            toggle_switch::pill_color(&dark, false),
            theme::SURFACE_2,
            "the unchecked pill is the input well token"
        );
        assert_eq!(
            toggle_switch::pill_color(&dark, true),
            theme::BRAND_500,
            "the checked pill is brand primary"
        );
        assert_eq!(toggle_switch::knob_color(&dark), dark.on_brand);

        let light = theme::Palette::light();
        assert_eq!(toggle_switch::pill_color(&light, false), light.surface_2);
        assert_eq!(toggle_switch::pill_color(&light, true), light.brand_primary);
        assert_eq!(toggle_switch::knob_color(&light), light.on_brand);
    }

    #[test]
    fn test_readiness_maps_status_and_indexing_per_the_glossary() {
        use LibraryStatus::{Idle, Scanned, Scanning, Unavailable};

        // Present on disk + indexed → Ready.
        assert_eq!(settings::readiness(&Scanned(12), 12), Readiness::Ready);
        // A hydrated store counts too: Idle but tracks live under the root.
        assert_eq!(settings::readiness(&Idle, 7), Readiness::Ready);
        // Present but nothing indexed yet → Not Indexed.
        assert_eq!(settings::readiness(&Scanned(0), 0), Readiness::NotIndexed);
        assert_eq!(settings::readiness(&Idle, 0), Readiness::NotIndexed);
        // Path gone → Missing, regardless of what was indexed before.
        assert_eq!(settings::readiness(&Unavailable, 12), Readiness::Missing);
        // Mid-scan reads as Scanning.
        assert_eq!(
            settings::readiness(&Scanning { files_found: 3 }, 0),
            Readiness::Scanning
        );
    }

    #[test]
    fn test_readiness_is_independent_of_watch_state() {
        // Two rows describing the SAME path health, differing only in their
        // persisted watcher choice: identical Readiness and identical dot.
        let status = LibraryStatus::Scanned(4);
        let dark = theme::Palette::dark();
        let idle_row = LibraryRow {
            path: PathBuf::from("C:\\Music"),
            status: status.clone(),
            watch: WatchState::Disabled,
            indexed_tracks: 4,
        };
        let watching_row = LibraryRow {
            path: PathBuf::from("C:\\Music"),
            status,
            watch: WatchState::Warning("inotify limit".to_string()),
            indexed_tracks: 4,
        };
        assert_eq!(idle_row.readiness(), watching_row.readiness());
        assert_eq!(
            idle_row.readiness().dot_color(&dark),
            watching_row.readiness().dot_color(&dark),
            "the dot must not move because watching changed"
        );
    }

    #[test]
    fn test_readiness_dot_colors_come_from_the_status_tokens() {
        let dark = theme::Palette::dark();
        assert_eq!(Readiness::Ready.dot_color(&dark), dark.success);
        assert_eq!(Readiness::Scanning.dot_color(&dark), dark.info);
        assert_eq!(Readiness::NotIndexed.dot_color(&dark), dark.warning);
        assert_eq!(Readiness::Missing.dot_color(&dark), dark.error);
    }

    #[test]
    fn test_readiness_labels_read_as_health_not_watch() {
        assert_eq!(Readiness::Ready.label(), "Ready");
        assert_eq!(Readiness::Scanning.label(), "Scanning");
        assert_eq!(Readiness::NotIndexed.label(), "Not indexed");
        assert_eq!(Readiness::Missing.label(), "Missing");
    }

    #[test]
    fn test_settings_section_headers_match_the_mockup() {
        assert_eq!(settings::SECTION_LIBRARIES, "MUSIC LIBRARIES");
        assert_eq!(settings::SECTION_ADVANCED_INFO, "ADVANCED & PLATFORM INFO");
    }

    /// Regression: `truncate_path` cuts at a byte offset, so a multi-byte
    /// character straddling the cut used to panic the render loop ("byte
    /// index is not a char boundary") for any library folder whose path
    /// exceeds 48 bytes with non-ASCII content. The cut must land on a char
    /// boundary and keep whole characters. The sweeps below place a 2-byte
    /// and a 3-byte character such that some suffix length puts the byte cut
    /// squarely inside it — the old code panicked on exactly those inputs.
    #[test]
    fn test_truncate_path_cuts_on_char_boundaries() {
        use riff_gui::ui::settings::truncate_path;

        // Short paths pass through untouched.
        assert_eq!(
            truncate_path("C:\\music\\song.mp3", 48),
            "C:\\music\\song.mp3"
        );

        // ASCII tail behavior is unchanged: keep the last max_len-3 bytes.
        let ascii = format!("C:\\music\\{}", "a".repeat(48));
        let out = truncate_path(&ascii, 48);
        assert_eq!(out, format!("...{}", "a".repeat(45)));

        // Sweep every alignment for a 2-byte (é) and a 3-byte (乐) character.
        for n in 0..60usize {
            let path = format!("{}é{}", "a".repeat(20), "b".repeat(n));
            if path.len() > 48 {
                let out = truncate_path(&path, 48);
                assert!(out.starts_with("..."), "got {out:?}");
                assert!(
                    path.ends_with(&out[3..]),
                    "2-byte tail must stay whole characters: {out:?}"
                );
            }
            let path = format!("{}乐{}", "a".repeat(30), "b".repeat(n));
            if path.len() > 48 {
                let out = truncate_path(&path, 48);
                assert!(out.starts_with("..."), "got {out:?}");
                assert!(
                    path.ends_with(&out[3..]),
                    "3-byte tail must stay whole characters: {out:?}"
                );
            }
        }

        // 4-byte characters (emoji) in the tail.
        let emoji = format!("{}🎵🎶", "a".repeat(46));
        let out = truncate_path(&emoji, 48);
        assert!(
            emoji.ends_with(&out[3..]),
            "emoji tail must survive intact: {out:?}"
        );

        // Degenerate max_len must not underflow.
        assert_eq!(truncate_path("C:\\music\\song.mp3", 2), "...");
    }

    #[test]
    fn test_preference_rows_match_the_mockup_copy_verbatim() {
        assert_eq!(
            settings::PREF_ADVANCED,
            (
                "Advanced mode",
                "Expose extra metadata fields and per-track actions."
            )
        );
        assert_eq!(
            settings::PREF_HIGH_CONTRAST,
            (
                "High contrast",
                "Increase contrast for text and focus outlines."
            )
        );
        assert_eq!(
            settings::PREF_REPLAYGAIN,
            (
                "ReplayGain",
                "Normalize loudness across tracks when available."
            )
        );
    }

    #[test]
    fn test_destructive_action_is_a_ghost_button_using_glossary_language() {
        // CONTEXT.md: the action is "Clear Library"; "Clear Library Cache" is
        // a retired term even though the mockup uses it.
        assert_eq!(settings::CLEAR_LIBRARY_LABEL, "Clear Library");
        // The action's explanatory copy moved with it into the confirm dialog
        // when the in-pane clear row was deleted (ticket 02): the footer has
        // one note, `FOOTER_NOTE`, so the dedicated line under the button is
        // gone. The glossary guard follows the copy that still ships.
        assert_eq!(
            riff_gui::ui::prompts::CLEAR_LIBRARY_CONFIRM_COPY,
            "Remove every indexed track? Playlists and settings are kept, and the \
             collection rebuilds on the next scan."
        );
        // The recovery clause the footer note used to carry must survive
        // somewhere, or a destructive action loses its consequence.
        assert!(
            riff_gui::ui::prompts::CLEAR_LIBRARY_CONFIRM_COPY.contains("next scan"),
            "the confirm copy must still say the collection rebuilds on the next scan"
        );
        assert!(
            !riff_gui::ui::prompts::CLEAR_LIBRARY_CONFIRM_COPY.contains("Cache")
                && !settings::CLEAR_LIBRARY_LABEL.contains("Cache"),
            "no user-visible Clear Library copy may use the retired 'Cache' term"
        );
        // Ghost styling: transparent until hover, then destructive @ 10%.
        let dark = theme::Palette::dark();
        assert_eq!(
            theme::destructive_fill(&dark, false),
            egui::Color32::TRANSPARENT
        );
        assert_eq!(
            theme::destructive_fill(&dark, true),
            dark.error.gamma_multiply(0.1)
        );
    }

    // --- Sectioned Settings modal + left nav (Issue 11) ------------------------
    //
    // Settings becomes a sectioned modal with a left nav (Library, Playback,
    // Appearance, Advanced, About) instead of one long scrolling stage. The
    // seams: the pure `SettingsSection` type, and the headless renderer
    // reporting `SettingsAction`s — the same shape the former stage used.

    #[test]
    fn test_settings_sections_list_the_nav_entries_in_mockup_order() {
        use riff_gui::ui::settings::SettingsSection;

        let labels: Vec<&str> = SettingsSection::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(
            labels,
            vec!["Library", "Playback", "Appearance", "Advanced", "About",],
            "the left nav lists exactly the implemented sections in order"
        );
    }

    /// Render the sectioned modal headlessly at `current`, with one
    /// representative library row, collecting reported actions.
    fn settings_modal_harness(
        content: &SettingsContent,
        current: riff_gui::ui::settings::SettingsSection,
    ) -> egui_kittest::Harness<'_, Vec<SettingsAction>> {
        use riff_gui::ui::settings::show_settings_modal;
        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 720.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, actions: &mut Vec<SettingsAction>| {
                    actions.extend(show_settings_modal(
                        ui, &mut cache, &palette, content, current,
                    ));
                },
                Vec::new(),
            )
    }

    /// The rect of the largest contiguous field of `color` in the frame, or
    /// `None` when the frame has none.
    ///
    /// Located in two steps, because no single measurement of the fill gives
    /// the card's box. A bounding box over every matching pixel spans past the
    /// card — the frame holds other small elements in the same color — so its
    /// "edges" land on unrelated pixels. The widest *unbroken run* is the card's
    /// body but only a lower bound on its width, because a control near one end
    /// (the ReplayGain toggle) breaks the run short of the edge. So the run
    /// fixes the card's height, and the left/right edges are then read off any
    /// matching pixel within those rows, which is inside the card by
    /// construction.
    fn largest_fill_rect(frame: &image::RgbaImage, color: egui::Color32) -> Option<egui::Rect> {
        let (width, height) = frame.dimensions();
        let is_fill = |x: u32, y: u32| {
            let p = frame.get_pixel(x, y);
            p.0[0] == color.r() && p.0[1] == color.g() && p.0[2] == color.b()
        };
        // The widest unbroken run of the fill on each row.
        let widest: Vec<Option<(u32, u32)>> = (0..height)
            .map(|y| {
                let mut best: Option<(u32, u32)> = None;
                let mut start: Option<u32> = None;
                for x in 0..=width {
                    let filled = x < width && is_fill(x, y);
                    match (filled, start) {
                        (true, None) => start = Some(x),
                        (false, Some(begin)) => {
                            if best.is_none_or(|(a, b)| x - 1 - begin > b - a) {
                                best = Some((begin, x - 1));
                            }
                            start = None;
                        }
                        _ => {}
                    }
                }
                best
            })
            .collect();
        let span = widest.iter().flatten().map(|(a, b)| b - a + 1).max()?;
        // Every row still nearly as wide is the card's body, so its height.
        let body = span * 3 / 5;
        let top = widest
            .iter()
            .position(|r| r.is_some_and(|(a, b)| b - a + 1 >= body))? as u32;
        let bottom = widest
            .iter()
            .rposition(|r| r.is_some_and(|(a, b)| b - a + 1 >= body))? as u32;
        // Within the body, the fill's own horizontal extremes are the card's.
        let in_body = |x: u32| (top..=bottom).any(|y| is_fill(x, y));
        let left = (0..width).find(|x| in_body(*x))?;
        let right = (0..width).rfind(|x| in_body(*x))?;
        Some(egui::Rect::from_min_max(
            egui::pos2(left as f32, top as f32),
            egui::pos2(right as f32 + 1.0, bottom as f32 + 1.0),
        ))
    }

    /// A settings card is the card plane plus the hairline that separates it
    /// from the pane behind it. The mockup defines a card by its edge, not by
    /// a fill of its own, so the fill alone leaves the card's boundary
    /// unfindable against the pane it sits on.
    ///
    /// Both halves are checked. The edge is checked as an *edge* — a
    /// border-colored pixel on each of the card's four sides — and not as a
    /// count, because a count would be satisfied by `row_separator`, which
    /// paints the same border token *inside* the card. Probing each side at its
    /// own midpoint cannot be satisfied by an in-card line, nor by one
    /// antialiased glyph: with the stroke gone the fill meets the pane directly
    /// and the pixels in between read (18, 17, 16), a third of the way down
    /// the gap, nowhere near either compositing of the border token.
    #[test]
    fn test_settings_card_paints_its_fill_and_a_border_defined_edge() {
        use riff_gui::ui::settings::SettingsSection;
        use riff_gui::ui::theme::Palette;

        let palette = Palette::dark();
        let content = sample_content();
        // Playback is the one section that is a single card, so the card's own
        // edge is separable from every other thing in the frame.
        let mut harness = settings_modal_harness(&content, SettingsSection::Playback);
        harness.run();
        let frame = harness
            .render()
            .expect("the settings pane must render headlessly");

        // The fill: the card plane, and a large field of it, so the card is
        // unambiguously located before anything is probed around it.
        let card = largest_fill_rect(&frame, palette.surface)
            .expect("a settings card paints the card plane");
        let (width, height) = frame.dimensions();
        let fill = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let px = f32::from(*x as u16);
                let py = f32::from(*y as u16);
                let inside = px >= card.left()
                    && px < card.right()
                    && py >= card.top()
                    && py < card.bottom();
                let p = frame.get_pixel(*x, *y);
                inside
                    && p.0[0].abs_diff(palette.surface.r()) <= 2
                    && p.0[1].abs_diff(palette.surface.g()) <= 2
                    && p.0[2].abs_diff(palette.surface.b()) <= 2
            })
            .count();
        assert!(
            fill > 10_000,
            "a settings card's interior paints the card plane ({fill} px in {card:?})"
        );

        // The edge: `border` is a translucent white, and the stroke is centered
        // on the card's rect, so half of it falls outside the card and
        // composites against the pane behind it. That outside half is the edge
        // a viewer reads, and it is what these probes look for.
        //
        // Composited the way egui composites its own output: `Color32` is
        // premultiplied, so the result is the stored top added onto the scaled
        // bottom — NOT `theme::blend_over`, whose straight-alpha contract would
        // give (15, 14, 13) here instead of the (39, 38, 37) the card's edge
        // actually reads. Which half of the stroke lands on which pixel depends
        // on where the rect's fractional edge falls, so both are accepted.
        let composited_on = |behind: egui::Color32| {
            let keep = 1.0 - f32::from(palette.border.a()) / 255.0;
            let channel =
                |top: u8, bottom: u8| (f32::from(top) + f32::from(bottom) * keep).round() as u8;
            egui::Color32::from_rgb(
                channel(palette.border.r(), behind.r()),
                channel(palette.border.g(), behind.g()),
                channel(palette.border.b(), behind.b()),
            )
        };
        let edges = [
            composited_on(palette.background),
            composited_on(palette.surface),
        ];
        let at = |probe: (f32, f32)| -> Option<&image::Rgba<u8>> {
            let (px, py) = (i64::from(probe.0 as i32), i64::from(probe.1 as i32));
            (px >= 0 && py >= 0 && px < i64::from(width) && py < i64::from(height))
                .then(|| frame.get_pixel(px as u32, py as u32))
        };
        let is_edge = |p: &image::Rgba<u8>| {
            edges.iter().any(|want| {
                p.0[0].abs_diff(want.r()) <= 6
                    && p.0[1].abs_diff(want.g()) <= 6
                    && p.0[2].abs_diff(want.b()) <= 6
            })
        };
        // Each side is probed at its own midpoint, starting at the pixel just
        // outside the fill and stepping two further out. The stroke is centered
        // on the card's boundary, so its outer half lands on `left - 1`,
        // `right`, `top - 1` and `bottom` respectively — the band exists because
        // a 1px stroke lands across two pixel rows whenever its rect sits on a
        // fractional coordinate.
        let (mid_x, mid_y) = (card.center().x, card.center().y);
        for (side, along, outward) in [
            ("left", card.left() - 1.0, -1.0_f32),
            ("right", card.right(), 1.0_f32),
            ("top", card.top() - 1.0, -1.0_f32),
            ("bottom", card.bottom(), 1.0_f32),
        ] {
            let vertical = side == "left" || side == "right";
            let band = (0..=2).map(move |d| {
                let along = along + d as f32 * outward;
                if vertical {
                    (along, mid_y)
                } else {
                    (mid_x, along)
                }
            });
            assert!(
                band.filter_map(at).any(is_edge),
                "a settings card's {side} edge paints the border token, composited \
                 to {edges:?} (card {card:?}, probed at {mid_x}/{mid_y})"
            );
        }
    }

    #[test]
    fn test_library_pane_toggles_report_immediate_commit_actions() {
        // The pane defaults: hidden files skipped, embedded art read, one
        // root watched. Clicking each toggle must report the commit action
        // that turns it OFF (values live in the session, not the widget).
        let content = SettingsContent {
            watch_any: true,
            skip_hidden_files: true,
            read_embedded_artwork: true,
            ..sample_content()
        };

        // The Library pane scrolls, so each row is scrolled into view
        // before its click (see [`click_into_view`]).
        let harness = click_into_view("Watch for changes", &content);
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetWatchAll(false)),
            "the Watch for changes toggle reports the watcher-wide commit"
        );

        let harness = click_into_view("Skip hidden files", &content);
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetSkipHidden(false)),
            "the Skip hidden files toggle reports the scan-preference commit"
        );

        let harness = click_into_view("Read embedded artwork", &content);
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReadEmbeddedArtwork(false)),
            "the Read embedded artwork toggle reports the artwork-policy commit"
        );
    }

    #[test]
    fn test_format_chip_click_reports_the_format_toggle() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        // FLAC enabled: clicking its chip must report disabling it.
        let content = SettingsContent {
            scan_formats: riff_backend::app::store::AUDIO_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            ..sample_content()
        };
        let mut harness = settings_modal_harness(&content, SettingsSection::Library);
        harness.run();
        harness.get_by_label("Index flac files").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetFormat("flac".to_string(), false)),
            "an enabled format chip reports disabling that format's indexing"
        );
    }

    #[test]
    fn test_rescan_now_and_footer_actions_report_their_intents() {
        let content = SettingsContent { ..sample_content() };

        let harness = click_into_view("Rescan now", &content);
        assert!(
            harness.state().contains(&SettingsAction::ScanAll),
            "Rescan now requests a full rescan"
        );
        // Done closes Settings — the same intent as the header's back arrow.

        let harness = click_into_view("Done", &content);
        assert!(
            harness.state().contains(&SettingsAction::Back),
            "Done closes the Settings modal"
        );
    }

    /// The Library pane scrolls, so a target must be scrolled into view
    /// before its click (kittest clicks at on-screen coordinates). Runs the
    /// modal, scrolls to `label`, clicks it, and returns the harness for
    /// action assertions.
    fn click_into_view<'h>(
        label: &'static str,
        content: &'h SettingsContent,
    ) -> egui_kittest::Harness<'h, Vec<SettingsAction>> {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let mut harness = settings_modal_harness(content, SettingsSection::Library);
        harness.run();
        harness.get_by_label(label).scroll_to_me();
        harness.run();
        harness.get_by_label(label).click();
        harness.run();
        harness
    }

    // --- Library pane columns (ticket 04) ---------------------------------------

    /// The Library pane's branch is chosen by `MIN_TWO_COL_W`, and the
    /// boundary itself is pinned: at exactly the breakpoint the pane splits.
    #[test]
    fn test_library_pane_splits_two_columns_at_and_above_the_breakpoint() {
        use riff_gui::ui::settings::{LibraryPaneColumns, settings_pane_columns};
        use riff_gui::ui::theme::geometry::settings::{COLUMN_GAP, MIN_TWO_COL_W};

        // Comfortably below: one stacked column at the full available width.
        for width in [MIN_TWO_COL_W - 1.0, MIN_TWO_COL_W - 200.0, 284.0] {
            assert!(
                matches!(
                    settings_pane_columns(width),
                    LibraryPaneColumns::Stacked { .. }
                ),
                "{width}px is below MIN_TWO_COL_W ({MIN_TWO_COL_W}) and must stack"
            );
        }

        // Exactly at the breakpoint: the token is the *minimum* width at which
        // the pane is allowed to split, so it splits here.
        assert!(
            matches!(
                settings_pane_columns(MIN_TWO_COL_W),
                LibraryPaneColumns::TwoColumns { .. }
            ),
            "exactly MIN_TWO_COL_W ({MIN_TWO_COL_W}) must take the two-column branch"
        );

        // Comfortably above: two columns.
        for width in [MIN_TWO_COL_W + 1.0, 900.0, 1400.0] {
            assert!(
                matches!(
                    settings_pane_columns(width),
                    LibraryPaneColumns::TwoColumns { .. }
                ),
                "{width}px is at or above MIN_TWO_COL_W and must split"
            );
        }

        // The stacked column is the whole width, and the two columns plus the
        // gap are exactly the whole width — neither branch loses or invents
        // horizontal space.
        for width in [284.0, MIN_TWO_COL_W - 1.0] {
            let LibraryPaneColumns::Stacked { width: column } = settings_pane_columns(width) else {
                unreachable!("narrow widths stack");
            };
            assert_eq!(column, width, "the stacked column takes the full width");
        }
        for width in [MIN_TWO_COL_W, 900.0, 1400.0] {
            let LibraryPaneColumns::TwoColumns { left, right } = settings_pane_columns(width)
            else {
                unreachable!("wide widths split");
            };
            assert_eq!(
                left + COLUMN_GAP + right,
                width,
                "both columns plus COLUMN_GAP must span the available width at {width}px"
            );
        }
    }

    /// The two columns are balanced — the same width each — and separated by
    /// exactly `COLUMN_GAP`.
    #[test]
    fn test_library_pane_columns_are_balanced_and_separated_by_the_token_gap() {
        use riff_gui::ui::settings::{LibraryPaneColumns, settings_pane_columns};
        use riff_gui::ui::theme::geometry::settings::COLUMN_GAP;

        for width in [620.0_f32, 621.0, 700.0, 920.0, 1400.0, 1737.0] {
            let LibraryPaneColumns::TwoColumns { left, right } = settings_pane_columns(width)
            else {
                panic!("{width}px must take the two-column branch");
            };
            assert_eq!(
                left, right,
                "the columns must be balanced at {width}px: {left} vs {right}"
            );
            assert_eq!(
                (left + right) / 2.0,
                (width - COLUMN_GAP) / 2.0,
                "each column is half the available width less half the gap, at {width}px"
            );
            assert!(
                left > 0.0,
                "each column must be usable at {width}px: {left}"
            );
        }
    }

    /// The pane's own focus order is reading order, in **both** branches.
    ///
    /// The two-column split is a layout decision, never a reordering: the
    /// sections still read Preferences, Formats, Last Full Scan, Artwork, and
    /// Tab still visits them in that order. The ticket-02 test only pinned the
    /// nav's contiguous run — a mutation that swapped Preferences and Formats
    /// inside the left column survived it, which is why this exists.
    ///
    /// Observed with `Node::is_focused()` after each `key_press(Tab)`, so the
    /// assertion is about the real focus chain, not about geometry.
    #[test]
    fn test_settings_pane_focus_order_is_reading_order_in_both_branches() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{SettingsSection, show_settings_modal};
        use riff_gui::ui::theme::Palette;

        // One probe per focusable pane widget, in reading order. These four are
        // the *whole* of the pane's tab chain: the format chips and the
        // Add Library / Scan All actions are not focusable widgets in this app,
        // so they never appear. That is why a Formats/Preferences swap is
        // invisible here and is guarded by geometry instead — see
        // `test_settings_pane_columns_read_left_then_right`.
        const READING_ORDER: [&str; 4] = [
            "Watch for changes",
            "Skip hidden files",
            "Rescan now",
            "Read embedded artwork",
        ];

        let walk_order = |width: f32| {
            let content = sample_content();
            let palette = Palette::dark();
            let mut cache = icons::IconCache::new();
            let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
                egui_kittest::Harness::builder()
                    .with_size(egui::vec2(width, 900.0))
                    .with_pixels_per_point(1.0)
                    .build_ui_state(
                        move |ui, actions: &mut Vec<SettingsAction>| {
                            actions.extend(show_settings_modal(
                                ui,
                                &mut cache,
                                &palette,
                                &content,
                                SettingsSection::Library,
                            ));
                        },
                        Vec::new(),
                    );
            harness.run();

            let mut seen: Vec<String> = Vec::new();
            for _ in 0..40 {
                harness.key_press(egui::Key::Tab);
                harness.run();
                for label in READING_ORDER {
                    if harness
                        .query_by_label(label)
                        .is_some_and(|n| n.is_focused())
                        && seen.last().map(String::as_str) != Some(label)
                    {
                        seen.push(label.to_owned());
                    }
                }
            }
            seen
        };

        // Wide: the two-column branch. Narrow: the stacked fallback. Both must
        // read the same way.
        for (label, width) in [("two-column", 1280.0_f32), ("stacked", 520.0)] {
            assert_eq!(
                walk_order(width),
                READING_ORDER,
                "the {label} branch must visit the pane in reading order"
            );
        }
    }

    // --- Narrow-width row reflow (ticket 04 follow-up) -------------------------

    /// The reflow activates below `LIBRARY_ROW_STACK_W` and not at or above it,
    /// with the boundary itself pinned.
    #[test]
    fn test_row_flow_switches_at_the_stack_threshold() {
        use riff_gui::ui::settings::{RowFlow, row_flow, row_height};
        use riff_gui::ui::theme::geometry::settings::{LIBRARY_ROW_H, LIBRARY_ROW_STACK_W};

        for width in [LIBRARY_ROW_STACK_W - 1.0, 205.0, 197.0, 100.0] {
            assert_eq!(
                row_flow(width),
                RowFlow::Stacked,
                "{width}px is below the threshold and must stack"
            );
        }
        // Exactly at the threshold: like the two-column breakpoint, the token is
        // the *minimum* width at which the row is allowed to stay inline.
        assert_eq!(row_flow(LIBRARY_ROW_STACK_W), RowFlow::Inline);
        for width in [LIBRARY_ROW_STACK_W + 1.0, 605.0, 965.0, 2000.0] {
            assert_eq!(
                row_flow(width),
                RowFlow::Inline,
                "{width}px is at or above the threshold and must stay inline"
            );
        }
        assert!(
            row_height(RowFlow::Stacked) > row_height(RowFlow::Inline),
            "a stacked row must be taller than an inline one"
        );
        assert_eq!(row_height(RowFlow::Inline), LIBRARY_ROW_H);
    }

    /// The reflow is a **no-op at every already-pinned width**. This is what
    /// protects the six Library-pane goldens: they render at 920 and 1280
    /// stages, whose rows measure 605px and 965px — both far above the
    /// threshold — so they must take the inline branch and the row's exact
    /// historical height.
    #[test]
    fn test_row_reflow_is_a_noop_at_the_pinned_widths() {
        use riff_gui::ui::settings::{RowFlow, row_flow, row_height};
        use riff_gui::ui::theme::geometry::settings::LIBRARY_ROW_H;

        // The measured row widths of the pinned renders, and the narrowest
        // already-pinned row anywhere (a preference row inside the two-column
        // Library layout).
        for pinned in [605.0_f32, 965.0, 461.5] {
            assert_eq!(
                row_flow(pinned),
                RowFlow::Inline,
                "{pinned}px is an already-pinned row width and must not reflow"
            );
            assert_eq!(row_height(row_flow(pinned)), LIBRARY_ROW_H);
        }
    }

    /// At the minimum stage no control is clipped and nothing overlaps.
    ///
    /// This is the regression guard for the defect the reflow fixes: before it,
    /// the row's control cluster (~254px) was laid into a ~205px row, so Scan
    /// overprinted the path and the strip ran off the card's right edge.
    ///
    /// The path itself is painted text, not a widget, so it has no query-tree
    /// node; the guard is that the row takes its three-band stacked form, whose
    /// bands are disjoint by construction, plus that every control node is
    /// inside the card and does not overlap its neighbours.
    #[test]
    fn test_minimum_stage_library_row_has_no_overlap_or_clipping() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{
            RowFlow, SettingsSection, row_flow, row_height, show_settings_modal,
        };
        use riff_gui::ui::theme::Palette;
        use riff_gui::ui::theme::geometry::window::MIN_STAGE_SIZE;

        let path = sample_content().libraries[0]
            .path
            .to_string_lossy()
            .to_string();
        let content = sample_content();
        let palette = Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
            egui_kittest::Harness::builder()
                .with_size(MIN_STAGE_SIZE)
                .with_pixels_per_point(1.0)
                .build_ui_state(
                    move |ui, actions: &mut Vec<SettingsAction>| {
                        actions.extend(show_settings_modal(
                            ui,
                            &mut cache,
                            &palette,
                            &content,
                            SettingsSection::Library,
                        ));
                    },
                    Vec::new(),
                );
        harness.run();

        // The row must have reflowed: the min-stage pane is far below the
        // threshold, and the stacked form is three bands, so the path band and
        // the control band cannot share a line.
        assert_eq!(row_flow(197.0), RowFlow::Stacked);

        // Every control the query tree can see, and where it sits. Scan and
        // Watch carry path-qualified accessibility labels, so they are queried
        // by those rather than by their painted text.
        let labels = [
            format!("Scan {path}"),
            format!("Watch {path}"),
            String::from("Remove library"),
        ];
        let mut controls: Vec<(String, egui::Rect)> = labels
            .iter()
            .filter_map(|label| {
                harness
                    .query_by_label(label)
                    .map(|node| (label.clone(), node.rect()))
            })
            .collect();
        assert!(
            controls.len() >= 2,
            "the library row's controls must be queryable at the minimum stage; found {:?}",
            controls.iter().map(|(l, _)| l).collect::<Vec<_>>()
        );
        controls.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));

        // Nothing runs off the pane: the rightmost control ends before the
        // stage's right edge, which is where the old strip overflowed to.
        for (label, rect) in &controls {
            assert!(
                rect.right() <= MIN_STAGE_SIZE.x,
                "{label} runs off the right edge: {} > {}",
                rect.right(),
                MIN_STAGE_SIZE.x
            );
            assert!(
                rect.width() > 0.0 && rect.height() > 0.0,
                "{label} must be laid out with a real rect: {rect:?}"
            );
        }
        // And no two controls overlap.
        for pair in controls.windows(2) {
            let (a, ra) = &pair[0];
            let (b, rb) = &pair[1];
            assert!(
                ra.right() <= rb.left()
                    || rb.right() <= ra.left()
                    || ra.bottom() <= rb.top()
                    || rb.bottom() <= ra.top(),
                "{a} {ra:?} and {b} {rb:?} overlap"
            );
        }
        // Sanity: the stacked height really is three bands, so the path band
        // and the strip band are disjoint.
        assert!(
            row_height(RowFlow::Stacked)
                > 2.0 * riff_gui::ui::theme::geometry::settings::SMALL_BTN_H
        );
    }

    /// A preference description wraps to its text column instead of being
    /// truncated, and the row grows to fit it.
    #[test]
    fn test_preference_description_wraps_instead_of_truncating() {
        use riff_gui::ui::settings::{
            RowFlow, preference_row_height, preference_text_width, row_flow,
        };
        use riff_gui::ui::theme::geometry::settings::PREF_ROW_H;

        let narrow = preference_text_width(197.0);
        let wide = preference_text_width(605.0);
        assert!(narrow > 0.0, "the narrow text column must still be usable");
        assert!(
            narrow < wide,
            "a narrower row must leave a narrower text column: {narrow} vs {wide}"
        );
        // The column is what is left after the toggle, so the description can
        // never be laid out wider than the row minus the toggle.
        assert!(narrow < 197.0, "the text column must exclude the toggle");

        // A single-line description needs no extra height; a wrapped one does,
        // and the row is never shorter than its inline height.
        let title_h = 16.0_f32;
        assert_eq!(
            preference_row_height(RowFlow::Inline, title_h, title_h),
            PREF_ROW_H,
            "the inline height ignores the measured text entirely"
        );
        let wrapped = preference_row_height(RowFlow::Stacked, title_h, title_h * 3.0);
        assert!(
            wrapped > PREF_ROW_H,
            "a three-line description must grow the row past PREF_ROW_H: {wrapped}"
        );
        assert_eq!(
            preference_row_height(RowFlow::Stacked, title_h, 0.0),
            PREF_ROW_H,
            "a stacked row is never shorter than the inline height"
        );
        assert_eq!(row_flow(197.0), RowFlow::Stacked);
    }

    // --- Format-chip wrapping (ticket 05) --------------------------------------

    /// The seven format chips' measured label widths, in `AUDIO_EXTENSIONS`
    /// order, taken from the vendored Inter set at `TEXT_XS` with a temporary
    /// probe inside `formats_card`:
    ///
    /// ```text
    /// MP3 26.19   M4A 27.34   AAC 25.56   OPUS 33.53
    /// OGG 27.16   FLAC 30.72  WAV 27.44
    /// ```
    ///
    /// The chip widths add `CHIP_LABEL_PAD * 2` to each, and a single line adds
    /// six `CHIP_GAP`s — the arithmetic behind `CHIP_ROW_NO_WRAP_W`. These are
    /// recorded constants rather than measured live: this test is about the
    /// wrapping arithmetic, and pinning the font metrics here would make it a
    /// font test that breaks on an unrelated type change. The goldens are what
    /// catch a real metric change.
    const MEASURED_CHIP_LABELS: [f32; 7] = [26.19, 27.34, 25.56, 33.53, 27.16, 30.72, 27.44];

    /// The chip widths, i.e. the measured labels plus the chip's own padding.
    fn measured_chip_widths() -> Vec<f32> {
        use riff_gui::ui::theme::geometry::settings::CHIP_LABEL_PAD;
        MEASURED_CHIP_LABELS
            .iter()
            .map(|label| CHIP_LABEL_PAD * 2.0 + label)
            .collect()
    }

    /// The content column the formats card gets at `MIN_STAGE_SIZE`, measured
    /// the same way (the pane's frame inset less the card's own inner margin).
    const MIN_STAGE_CHIP_AVAILABLE: f32 = 181.0;

    /// Content columns at the widths the goldens already pin: the 920-stage
    /// Library goldens and the 1280 two-column golden, both of which show all
    /// seven chips on one line today and must keep doing so.
    const WIDE_CHIP_AVAILABLE: [f32; 2] = [581.0, 445.5];

    /// The chip row wraps below the measured threshold and does not at or above
    /// it, with the boundary pinned exactly as `row_flow` pins its own.
    #[test]
    fn test_format_chip_row_wraps_only_below_the_measured_threshold() {
        use riff_gui::ui::settings::{ChipFlow, chip_flow};
        use riff_gui::ui::theme::geometry::settings::CHIP_ROW_NO_WRAP_W;

        for width in [
            MIN_STAGE_CHIP_AVAILABLE,
            361.0,
            CHIP_ROW_NO_WRAP_W - 1.0,
            100.0,
        ] {
            assert_eq!(
                chip_flow(width),
                ChipFlow::Wrapped,
                "{width}px is below the measured cluster and must wrap"
            );
        }
        assert_eq!(
            chip_flow(CHIP_ROW_NO_WRAP_W),
            ChipFlow::OneLine,
            "the token is the minimum width at which one line is allowed"
        );
        for width in [CHIP_ROW_NO_WRAP_W + 1.0, 581.0, 445.5, 2000.0] {
            assert_eq!(
                chip_flow(width),
                ChipFlow::OneLine,
                "{width}px fits the measured cluster and must stay on one line"
            );
        }

        // The token has to agree with the measurement it claims to come from,
        // or the two drift apart and the card sizes itself for one form while
        // painting the other.
        let cluster: f32 = measured_chip_widths().iter().sum::<f32>()
            + riff_gui::ui::theme::geometry::settings::CHIP_GAP
                * f32::from(u8::try_from(MEASURED_CHIP_LABELS.len() - 1).unwrap());
        assert!(
            CHIP_ROW_NO_WRAP_W >= cluster,
            "the token {} must not be below the measured cluster {cluster:.2}",
            CHIP_ROW_NO_WRAP_W
        );
        assert!(
            CHIP_ROW_NO_WRAP_W - cluster < 1.0,
            "the token should round the measurement up to whole pixels, not pad it"
        );
    }

    /// At the minimum stage every chip is fully inside the card and none is
    /// clipped, and at every wide width the layout is untouched.
    #[test]
    fn test_format_chips_all_fit_inside_the_card_at_the_minimum_stage() {
        use riff_gui::ui::settings::{chip_block_height, chip_placements};
        use riff_gui::ui::theme::geometry::settings::{CHIP_GAP, CHIP_H, CHIP_ROW_NO_WRAP_W};

        let widths = measured_chip_widths();
        let placements = chip_placements(&widths, MIN_STAGE_CHIP_AVAILABLE);

        // This is the defect the wrap fixes: seven chips needing 413.94px in a
        // 181px column, so the pre-wrap cursor ran off the card and clipped
        // OPUS with a fifth chip peeking past the edge.
        let needed: f32 = widths.iter().sum();
        assert!(
            needed > MIN_STAGE_CHIP_AVAILABLE,
            "the measurement that motivates the wrap: {needed:.2} > {MIN_STAGE_CHIP_AVAILABLE}"
        );

        // Every chip lands inside the column, and on its own line where it must.
        for (label, placement) in MEASURED_CHIP_LABELS.iter().zip(&placements) {
            assert!(
                placement.left >= 0.0,
                "a chip must never start left of the column: {placement:?}"
            );
            assert!(
                placement.left + placement.width <= MIN_STAGE_CHIP_AVAILABLE,
                "chip {label} is clipped: right edge {:.2} > {MIN_STAGE_CHIP_AVAILABLE}",
                placement.left + placement.width
            );
        }
        assert!(
            placements.iter().filter(|p| p.row > 0).count() > 0,
            "the min stage must actually wrap"
        );

        // No two chips on the same row may overlap.
        for (i, a) in placements.iter().enumerate() {
            for b in &placements[i + 1..] {
                if a.row != b.row {
                    continue;
                }
                assert!(
                    a.left + a.width <= b.left,
                    "chips on row {} overlap: {a:?} and {b:?}",
                    a.row
                );
            }
        }

        // The block grows to fit the rows it actually has, and the card's
        // height follows the block.
        let rows = placements.iter().map(|p| p.row + 1).max().unwrap_or(0);
        assert!(rows >= 2, "the min stage needs more than one line: {rows}");
        assert!(
            chip_block_height(&placements) > CHIP_H,
            "a wrapped block must be taller than one chip: {} vs {CHIP_H}",
            chip_block_height(&placements)
        );

        // The no-op half, which is the part the goldens depend on: at every
        // pinned wide width there is one line, at the offsets the pre-wrap
        // running cursor produced, and the block is exactly one chip tall.
        for available in WIDE_CHIP_AVAILABLE {
            let wide = chip_placements(&widths, available);
            assert!(
                wide.iter().all(|p| p.row == 0),
                "{available}px fits on one line but wrapped: {wide:?}"
            );
            assert_eq!(
                chip_block_height(&wide),
                CHIP_H,
                "a one-line block must be exactly one chip tall"
            );
            // The running cursor: each chip starts where the previous one plus
            // its own width plus one gap ends.
            let mut expected = 0.0_f32;
            for (placement, width) in wide.iter().zip(&widths) {
                assert_eq!(placement.left, expected, "one-line offsets must not move");
                expected += width + CHIP_GAP;
            }
        }

        // The exact boundary, which the two widths above cannot reach: they all
        // leave more than a gap of slack, so they would pass even with a cursor
        // that wrongly counted the trailing gap as occupied. At the token's own
        // value the cluster fills the column to within a pixel, so this is where
        // "the last chip does not wrap on its own" is actually decided.
        let at_threshold = chip_placements(&widths, CHIP_ROW_NO_WRAP_W);
        assert!(
            at_threshold.iter().all(|p| p.row == 0),
            "the last chip must not wrap on its own at the threshold: {at_threshold:?}"
        );
        assert_eq!(chip_block_height(&at_threshold), CHIP_H);
        // One pixel less and the final chip does have to move down.
        let just_under = chip_placements(&widths, CHIP_ROW_NO_WRAP_W - 1.0);
        assert!(
            just_under.iter().filter(|p| p.row > 0).count() == 1,
            "one pixel short of the cluster, exactly the last chip wraps: {just_under:?}"
        );
    }

    /// The two-column branch *reads* left column then right column, and the
    /// stacked branch reads strictly top to bottom.
    ///
    /// Guarded by rendered geometry through the query tree. The honest limit:
    /// the format chips emit no accessible node, so they are neither focusable
    /// nor queryable — Preferences/Formats order *within* the left column is
    /// therefore not machine-guardable today and rests on golden review. What
    /// this pins is the left/right split and the vertical order of the four
    /// queryable pane widgets, in both branches.
    #[test]
    fn test_settings_pane_columns_read_left_then_right() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{SettingsSection, show_settings_modal};
        use riff_gui::ui::theme::Palette;

        /// Tall enough that the whole pane is laid out unclipped, so every
        /// probed widget is visible in the first frame.
        const PANE_PROBE_H: f32 = 1600.0;

        /// The four pane widgets the query tree can see, in reading order.
        const PROBED: [&str; 4] = [
            "Watch for changes",
            "Skip hidden files",
            "Rescan now",
            "Read embedded artwork",
        ];

        let render = |width: f32| {
            let content = sample_content();
            let palette = Palette::dark();
            let mut cache = icons::IconCache::new();
            let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
                egui_kittest::Harness::builder()
                    .with_size(egui::vec2(width, PANE_PROBE_H))
                    .with_pixels_per_point(1.0)
                    .build_ui_state(
                        move |ui, actions: &mut Vec<SettingsAction>| {
                            actions.extend(show_settings_modal(
                                ui,
                                &mut cache,
                                &palette,
                                &content,
                                SettingsSection::Library,
                            ));
                        },
                        Vec::new(),
                    );
            harness.run();
            // All four rects are read in ONE frame, at ONE scroll position.
            // Scrolling each label into view first would move the pane between
            // measurements and make the tops incomparable — which silently
            // disarmed the ordering assertions (a mutation that put Artwork
            // above Last Full Scan survived until this was fixed).
            PROBED
                .iter()
                .map(|label| {
                    harness
                        .query_by_label(label)
                        .unwrap_or_else(|| {
                            panic!("{label} must be visible at {width}px without scrolling")
                        })
                        .rect()
                })
                .collect::<Vec<_>>()
        };

        // Wide: the two-column branch. Preferences/Formats (left) sit entirely
        // left of Last Full Scan/Artwork (right).
        let wide = render(1280.0);
        let (watch, skip, rescan, embedded) = (wide[0], wide[1], wide[2], wide[3]);
        assert!(
            watch.right() < rescan.left(),
            "Preferences is in the left column and must sit entirely left of the scan card: \
             {} vs {}",
            watch.right(),
            rescan.left()
        );
        assert!(
            skip.right() < rescan.left(),
            "the left column must stay clear of the right column"
        );
        assert!(
            embedded.left() > rescan.left(),
            "Artwork reads below Last Full Scan inside the right column"
        );
        assert!(
            embedded.top() > rescan.top(),
            "Artwork must read below Last Full Scan in the right column"
        );
        assert!(
            skip.top() > watch.top(),
            "Preferences reads top to bottom inside the left column"
        );

        // Narrow: one stack, strictly top to bottom across all four sections.
        let narrow = render(520.0);
        for pair in narrow.windows(2) {
            assert!(
                pair[0].top() < pair[1].top(),
                "the stacked branch reads top to bottom: {} ({}) must precede {} ({})",
                pair[0].center().y,
                pair[0].top(),
                pair[1].center().y,
                pair[1].top()
            );
        }
    }

    /// Both branches keep the *same* scroll area, so a short window scrolls
    /// rather than clipping and crossing the breakpoint does not orphan the
    /// scroll offset.
    ///
    /// Asserted behaviourally, not textually. The invariant is **"the offset is
    /// not reset"**, not "the widget lands at the same pixel": the two branches
    /// have different content heights, so a correctly-preserved offset puts the
    /// content somewhere different. A branch with its own `id_salt` builds a
    /// second scroll area starting at offset zero, which is what this catches.
    #[test]
    fn test_library_pane_scroll_offset_survives_a_branch_switch() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{SettingsSection, show_settings_modal};
        use riff_gui::ui::theme::Palette;

        // A short window, narrow enough for the stacked branch, so the pane is
        // genuinely taller than the viewport and scrolling moves something.
        const SHORT: f32 = 456.0;
        const NARROW: f32 = 520.0;
        const WIDE: f32 = 1280.0;

        let drive = |width: f32| {
            let content = sample_content();
            let palette = Palette::dark();
            let mut cache = icons::IconCache::new();
            let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
                egui_kittest::Harness::builder()
                    .with_size(egui::vec2(width, SHORT))
                    .with_pixels_per_point(1.0)
                    .build_ui_state(
                        move |ui, actions: &mut Vec<SettingsAction>| {
                            actions.extend(show_settings_modal(
                                ui,
                                &mut cache,
                                &palette,
                                &content,
                                SettingsSection::Library,
                            ));
                        },
                        Vec::new(),
                    );
            harness.run();
            harness
        };

        // Never scrolled, wide: where a *reset* scroll area would put the widget.
        let fresh = drive(WIDE);
        let wide_unscrolled = fresh.get_by_label("Rescan now").rect().top();
        drop(fresh);

        // Scroll the stacked branch, then cross the breakpoint in the SAME
        // context, holding the height so only the branch changes.
        let mut harness = drive(NARROW);
        let narrow_unscrolled = harness.get_by_label("Rescan now").rect().top();
        for _ in 0..8 {
            harness.get_by_label("Rescan now").scroll_down();
            harness.run();
        }
        let narrow_scrolled = harness.get_by_label("Rescan now").rect().top();
        assert!(
            narrow_scrolled < narrow_unscrolled,
            "the pane must actually scroll in a short window: {narrow_unscrolled} -> \
             {narrow_scrolled}"
        );

        harness.set_size(egui::vec2(WIDE, SHORT));
        harness.run();
        let wide_after_switch = harness.get_by_label("Rescan now").rect().top();
        assert!(
            (wide_after_switch - wide_unscrolled).abs() > 1.0,
            "crossing the breakpoint must not reset the pane's scroll: the two-column \
             branch shows Rescan now at {wide_after_switch}, but an unscrolled pane would \
             show it at {wide_unscrolled} — a match means the stacked branch built its own \
             scroll area"
        );
    }

    /// The Artwork card holds only "Read embedded artwork" (ticket 03).
    ///
    /// The "Missing artwork" strategy row and the separator above nothing are
    /// gone, so neither its title nor its "Generated colour" stand-in chip may
    /// be painted. Asserted through the query tree rather than pixels.
    #[test]
    fn test_artwork_card_holds_only_the_read_embedded_preference() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{SettingsSection, show_settings_modal};

        let content = sample_content();
        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
            egui_kittest::Harness::builder()
                .with_size(egui::vec2(1280.0, 840.0))
                .with_pixels_per_point(1.0)
                .build_ui_state(
                    move |ui, actions: &mut Vec<SettingsAction>| {
                        actions.extend(show_settings_modal(
                            ui,
                            &mut cache,
                            &palette,
                            &content,
                            SettingsSection::Library,
                        ));
                    },
                    Vec::new(),
                );
        harness.run();

        assert!(
            harness.query_by_label("Missing artwork").is_none(),
            "the Missing artwork strategy row must not be rendered"
        );
        assert!(
            harness.query_by_label("Generated colour").is_none(),
            "its generated-colour stand-in chip must not be rendered either"
        );
        assert!(
            harness.query_by_label("Read embedded artwork").is_some(),
            "the artwork card still holds the Read embedded artwork preference"
        );
    }

    /// The Last Full Scan card must not resize as a scan's error count moves.
    ///
    /// The golden fixture hardcodes `errors: 3`, so no golden exercises the
    /// zero case and the "still occupies its line" rule is unpinnable by
    /// pixels. This drives the real pane at `errors: 0` and `errors: 5` and
    /// asserts the card's geometry is identical.
    ///
    /// The probe is the card's own "Rescan now" button: it is laid out inside
    /// the card, so if the card's height moved the button's rect would move
    /// with it. The pure seam `scan_card_lines` is checked too, so the reason
    /// is pinned as well as the symptom — the error line is always present,
    /// and only its colour changes.
    #[test]
    fn test_scan_card_height_is_stable_across_error_counts() {
        use riff_gui::ui::settings::{
            SettingsSection, scan_card_height, scan_card_lines, scan_line_color,
            show_settings_modal,
        };
        use riff_gui::ui::theme::Palette;
        use riff_gui::ui::theme::geometry::settings::SCAN_CARD_H;

        // The pure seam: three lines always, the error line's colour keyed to
        // whether the count is non-zero.
        let summary = |errors: usize| riff_backend::app::store::FullScanSummary {
            at: std::time::SystemTime::now(),
            files: 1284,
            errors,
        };
        for errors in [0_usize, 1, 3, 999] {
            let lines = scan_card_lines(Some(&summary(errors)));
            assert_eq!(
                lines.iter().count(),
                riff_gui::ui::settings::ScanCardLines::LINES,
                "the card must always paint three lines (errors = {errors})"
            );
            assert_eq!(
                lines.error.is_signal,
                errors > 0,
                "the error line reads as a signal exactly when the count is non-zero \
                 (errors = {errors})"
            );
        }
        // No scan recorded still paints three lines, so the card cannot
        // collapse on the very first run either.
        assert_eq!(
            scan_card_lines(None).iter().count(),
            riff_gui::ui::settings::ScanCardLines::LINES,
            "a page with no scan recorded still paints three lines"
        );

        // The error line wears the error role exactly when the count is
        // non-zero, and the muted ink rung otherwise — including at zero.
        for errors in [0_usize, 1, 3, 999] {
            let lines = scan_card_lines(Some(&summary(errors)));
            let dark = Palette::dark();
            assert_eq!(
                scan_line_color(&lines.error, &dark),
                if errors > 0 { dark.error } else { dark.ink_3 },
                "the error line's colour must track whether the count is non-zero \
                 (errors = {errors})"
            );
            // The stamp is primary ink, never the error role.
            assert_eq!(
                scan_line_color(&lines.stamp, &dark),
                dark.ink_3,
                "only the error count may take the error role"
            );
        }

        // The height the card allocates is a function of the line count and
        // nothing else: every outcome allocates the same height.
        for errors in [0_usize, 1, 3, 999] {
            let lines = scan_card_lines(Some(&summary(errors)));
            assert_eq!(
                scan_card_height(&lines),
                SCAN_CARD_H,
                "the card must allocate SCAN_CARD_H whatever the counts (errors = {errors})"
            );
        }
        assert_eq!(scan_card_height(&scan_card_lines(None)), SCAN_CARD_H);

        // The rendered card: identical rect for a zero-error and a
        // many-error scan.
        let button_rect = |errors| {
            let content = SettingsContent {
                last_scan: Some(summary(errors)),
                ..sample_content()
            };
            let palette = Palette::dark();
            let mut cache = icons::IconCache::new();
            let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
                egui_kittest::Harness::builder()
                    .with_size(egui::vec2(1280.0, 840.0))
                    .with_pixels_per_point(1.0)
                    .build_ui_state(
                        move |ui, actions: &mut Vec<SettingsAction>| {
                            actions.extend(show_settings_modal(
                                ui,
                                &mut cache,
                                &palette,
                                &content,
                                SettingsSection::Library,
                            ));
                        },
                        Vec::new(),
                    );
            harness.run();
            use egui_kittest::kittest::Queryable;
            let rect = harness.get_by_label("Rescan now").rect();
            // Sanity: the probe really is the card, so a moved rect cannot be
            // explained by the button being somewhere else entirely.
            assert!(
                rect.height() > 0.0 && rect.width() > 0.0,
                "the Rescan now button must be laid out (errors = {errors})"
            );
            rect
        };

        let zero = button_rect(0);
        let many = button_rect(5);
        assert_eq!(
            zero, many,
            "the scan card must not move when the error count changes from 0 to 5"
        );
        assert!(
            SCAN_CARD_H >= zero.height(),
            "the card's token height must hold its action button"
        );
        // And the probe really does sit inside a card of the token height.
        assert!(
            zero.height() > 0.0 && zero.height() <= SCAN_CARD_H,
            "the Rescan now button must fit the card's token height"
        );
    }

    /// The page footer is part of the page frame, not the Library arm, so the
    /// destructive Clear Library action is offered by every section rather than
    /// only by Library. This drives each section in turn and clicks the real
    /// footer button.
    ///
    /// Note the button is *not* scrolled into view first: the footer sits
    /// outside the pane's `ScrollArea`, which is the point of moving it.
    #[test]
    fn test_clear_library_is_reachable_from_every_settings_section() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{SettingsSection, show_settings_modal};

        for section in SettingsSection::ALL {
            let content = sample_content();
            let palette = theme::Palette::dark();
            let mut cache = icons::IconCache::new();
            let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
                egui_kittest::Harness::builder()
                    .with_size(egui::vec2(1280.0, 840.0))
                    .with_pixels_per_point(1.0)
                    .build_ui_state(
                        move |ui, actions: &mut Vec<SettingsAction>| {
                            actions.extend(show_settings_modal(
                                ui, &mut cache, &palette, &content, section,
                            ));
                        },
                        Vec::new(),
                    );
            harness.run();

            assert!(
                harness.query_by_label("Clear Library").is_some(),
                "the footer must offer Clear Library on the {:?} section",
                section.label()
            );
            harness.get_by_label("Clear Library").click();
            harness.run();
            assert!(
                harness.state().contains(&SettingsAction::ClearLibrary),
                "clicking the footer action on the {:?} section must report ClearLibrary",
                section.label()
            );
        }
    }

    /// The Thumbnail clear has to be reachable where a listener whose Covers look
    /// wrong actually is — the Library pane's Artwork card, beside the artwork
    /// toggle — and reachable means *clickable*. A widget that exists but is
    /// clipped below a `ScrollArea` satisfies `query_by_label` and still cannot be
    /// pressed, so this asserts on the action the click returns rather than on the
    /// label being present.
    #[test]
    fn test_clear_thumbnail_cache_is_reachable_from_the_library_section() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::{SettingsSection, show_settings_modal};

        for size in [egui::vec2(1280.0, 840.0), egui::vec2(920.0, 840.0)] {
            let content = sample_content();
            let palette = theme::Palette::dark();
            let mut cache = icons::IconCache::new();
            let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
                egui_kittest::Harness::builder()
                    .with_size(size)
                    .with_pixels_per_point(1.0)
                    .build_ui_state(
                        move |ui, actions: &mut Vec<SettingsAction>| {
                            actions.extend(show_settings_modal(
                                ui,
                                &mut cache,
                                &palette,
                                &content,
                                SettingsSection::Library,
                            ));
                        },
                        Vec::new(),
                    );
            harness.run();

            harness
                .get_by_label(riff_gui::ui::prompts::CLEAR_THUMBNAIL_CACHE_LABEL)
                .click();
            harness.run();
            assert!(
                harness
                    .state()
                    .contains(&SettingsAction::ClearThumbnailCache),
                "the row in the Artwork card must answer a real click at {size:?}"
            );
        }
    }

    #[test]
    fn test_settings_nav_clicks_report_the_selected_section() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();
        let mut harness = settings_modal_harness(&content, SettingsSection::Library);
        harness.run();

        harness.get_by_label("About").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SelectSection(SettingsSection::About)),
            "clicking a nav item reports the section to show"
        );

        harness.get_by_label("Playback").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SelectSection(SettingsSection::Playback)),
            "every nav item is clickable"
        );
    }

    #[test]
    fn test_settings_pane_switches_content_with_the_current_section() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();

        // Library: the working library controls are on stage, and no other
        // section's content bleeds into the pane.
        let mut harness = settings_modal_harness(&content, SettingsSection::Library);
        harness.run();
        assert!(
            harness.query_by_label("Add Library").is_some(),
            "the Library pane keeps the library actions"
        );
        assert!(
            harness.query_by_label("Advanced mode").is_none(),
            "the Advanced preference lives in its own section, not Library"
        );

        // Advanced: the section's existing content (Advanced mode preference
        // and platform info) replaces the library controls entirely.
        let mut harness = settings_modal_harness(&content, SettingsSection::Advanced);
        harness.run();
        assert!(
            harness.query_by_label("Advanced mode").is_some(),
            "the Advanced pane shows the Advanced mode preference"
        );
        assert!(
            harness.query_by_label("Add Library").is_none(),
            "the Library card must not render in the Advanced pane"
        );

        // About has no existing content and shows a clear placeholder.
        {
            let section = SettingsSection::About;
            let mut harness = settings_modal_harness(&content, section);
            harness.run();
            let expected = format!("{} settings are not implemented yet.", section.label());
            assert!(
                harness.query_by_label(&expected).is_some(),
                "the {} pane shows its placeholder",
                section.label()
            );
            assert!(
                harness.query_by_label("Add Library").is_none(),
                "no library controls in the {} pane",
                section.label()
            );
        }
    }

    #[test]
    fn test_settings_nav_is_reachable_and_activatable_from_the_keyboard() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();
        let mut harness = settings_modal_harness(&content, SettingsSection::Library);
        harness.run();

        // Tab walks the focus order: the header's Back control first, then
        // the nav items in ALL order (Library first here).
        harness.key_press(egui::Key::Tab);
        harness.run();
        assert!(
            harness.get_by_label("Back to Library").is_focused(),
            "the first Tab lands on the header's Back control"
        );
        harness.key_press(egui::Key::Tab);
        harness.run();
        assert!(
            harness.get_by_label("Library").is_focused(),
            "the nav follows the header in focus order"
        );

        // Enter activates the focused nav item — same report as a click.
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SelectSection(SettingsSection::Library)),
            "Enter on a focused nav item selects that section"
        );

        // Deep in the nav too: focus the last item directly and activate it.
        harness.get_by_label("About").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SelectSection(SettingsSection::About)),
            "the last nav item is keyboard-activatable"
        );
    }

    fn sample_content() -> SettingsContent {
        SettingsContent {
            libraries: vec![LibraryRow {
                path: PathBuf::from("C:\\Users\\stink\\Music"),
                status: LibraryStatus::Scanned(1284),
                watch: WatchState::Enabled,
                indexed_tracks: 1284,
            }],
            advanced_mode: true,
            high_contrast: false,
            replaygain_enabled: false,
            ..SettingsContent::default()
        }
    }

    #[test]
    fn test_toggle_switch_click_reports_the_preference_action() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();

        // Advanced mode starts ON (mockup shows it checked) and lives in the
        // Advanced pane; clicking the switch must report turning it OFF.
        let mut harness = settings_modal_harness(&content, SettingsSection::Advanced);
        harness.run();
        harness.get_by_label("Advanced mode").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetAdvanced(false)),
            "the reusable ToggleSwitch drives the Advanced mode preference"
        );

        // ReplayGain starts OFF in the Playback pane...
        let mut harness = settings_modal_harness(&content, SettingsSection::Playback);
        harness.run();
        harness.get_by_label("ReplayGain").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGain(true)),
            "the same widget drives the ReplayGain preference"
        );

        // ...and High contrast starts OFF in the Appearance pane.
        let mut harness = settings_modal_harness(&content, SettingsSection::Appearance);
        harness.run();
        harness.get_by_label("High contrast").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetHighContrast(true)),
            "the same widget drives the High contrast preference"
        );
    }

    /// Render one boolean control (the toggle pill, or the shared checkbox box
    /// when `boxy`) and report whether it painted the keyboard focus ring, so a
    /// test can prove the toggle and the checkbox share the focus treatment.
    fn bool_control_painted_ring(checkbox: bool, focused: bool) -> bool {
        use riff_gui::ui::theme;
        use riff_gui::ui::toggle_switch;
        let palette = theme::Palette::dark();
        let id = egui::Id::new("bool_control");
        let ring = theme::focus_ring_stroke(&palette, true)
            .expect("a focused control has a ring")
            .color;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(80.0, 40.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                if focused {
                    ui.memory_mut(|m| m.request_focus(id));
                }
                if checkbox {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(30.0, 12.0), egui::vec2(16.0, 16.0));
                    // The shared box paints onto a response the caller already
                    // owns (the Watch slot); interact on `id` so it is a real,
                    // focusable node like that control.
                    ui.interact(rect, id, egui::Sense::click());
                    let focused = ui.memory(|m| m.has_focus(id));
                    let painter = ui.painter_at(rect);
                    toggle_switch::paint_checkbox_box(&painter, &palette, rect, true);
                    if let Some(stroke) = theme::focus_ring_stroke(&palette, focused) {
                        painter.rect_stroke(
                            rect,
                            theme::RADIUS_SM,
                            stroke,
                            egui::StrokeKind::Inside,
                        );
                    }
                } else {
                    let pill =
                        egui::Rect::from_min_size(egui::pos2(22.0, 10.0), egui::vec2(36.0, 20.0));
                    toggle_switch::toggle_switch_at(ui, &palette, id, "pref", pill, true);
                }
            });
        harness.run();
        let image = harness.render().expect("boolean control must render");
        image.pixels().any(|p| {
            p.0[0].abs_diff(ring.r()) <= 2
                && p.0[1].abs_diff(ring.g()) <= 2
                && p.0[2].abs_diff(ring.b()) <= 2
        })
    }

    /// The consolidated boolean controls share one focus treatment: the toggle
    /// pill rings only while focused, and the shared checkbox box the Watch
    /// control now paints through rings the same way — so checked, focus, and
    /// keyboard state read identically across the two kinds.
    #[test]
    fn test_toggle_and_checkbox_share_focus_ring() {
        assert!(
            bool_control_painted_ring(false, true),
            "a focused toggle switch paints the shared focus ring"
        );
        assert!(
            !bool_control_painted_ring(false, false),
            "an unfocused toggle switch paints no ring"
        );
        assert!(
            bool_control_painted_ring(true, true),
            "a focused checkbox paints the shared focus ring"
        );
        assert!(
            !bool_control_painted_ring(true, false),
            "an unfocused checkbox paints no ring"
        );
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn test_the_advanced_pane_offers_quit_on_close_where_a_tray_exists() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        // "Quit on close" defaults to minimize-to-tray (off); the row renders
        // only where a tray exists, so clicking it must report opting into
        // quitting. (On Linux the pane omits the row entirely — decision 002.)
        let content = sample_content();
        let mut harness = settings_modal_harness(&content, SettingsSection::Advanced);
        harness.run();
        harness.get_by_label("Quit on close").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetCloseQuitsApp(true)),
            "the Advanced pane's Quit-on-close toggle drives the preference"
        );
    }

    #[test]
    fn test_back_button_and_library_actions_report_actions() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();
        let mut harness = settings_modal_harness(&content, SettingsSection::Library);
        harness.run();

        harness.get_by_label("Back to Library").click();
        harness.run();
        assert!(
            harness.state().contains(&SettingsAction::Back),
            "the header's close control leaves the Settings View"
        );

        harness.get_by_label("Scan C:\\Users\\stink\\Music").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::Scan(PathBuf::from(
                    "C:\\Users\\stink\\Music"
                ))),
            "per-path Scan stays wired to the library command surface"
        );
    }

    // --- Interaction polish & performance (Issue 12) ----------------------------
    //
    // The final contract pass: playlist entries gain drag-to-reorder through
    // egui's built-in drag-and-drop support with the new order persisted via
    // the [`PlaylistStore`] port (ADR 0002), icon buttons grow tooltips, the
    // track-list virtualization is pinned by a large-library fixture, and the
    // zero-hardcoded-color sweep (Issue 03's guard above) stays green.

    /// Zero out the tooltip hover delay so headless hovers show tooltips on
    /// the very next frame instead of after the interactive grace period.
    fn make_tooltips_instant(ctx: &egui::Context) {
        ctx.global_style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.show_tooltips_only_when_still = false;
        });
    }

    #[test]
    fn test_playlist_reorder_commits_through_store_and_patches_projection() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, mut views) = boxed_playlist_seam(&dir);
        let pid = store
            .create_playlist(
                "Gym",
                &[
                    TrackId("a.mp3".to_string()),
                    TrackId("b.mp3".to_string()),
                    TrackId("c.mp3".to_string()),
                ],
            )
            .unwrap();

        // Drag entry 0 (A) onto entry 2's slot (C): A,B,C → B,C,A.
        riff_gui::ui::app::commit_playlist_reorder(&mut views, store.as_mut(), &pid, 0, 2);

        // The store committed the new order as one durable transaction...
        assert_eq!(
            store.load_playlists().unwrap()[0].tracks,
            vec![
                TrackId("b.mp3".to_string()),
                TrackId("c.mp3".to_string()),
                TrackId("a.mp3".to_string())
            ],
            "the dragged order persisted through the PlaylistStore"
        );
        // ...and the seam's projection reflects it with zero caller action —
        // the committed mutation bumped the playlist generation itself.
        assert_eq!(
            views.playlists()[0].tracks,
            vec![
                TrackId("b.mp3".to_string()),
                TrackId("c.mp3".to_string()),
                TrackId("a.mp3".to_string())
            ],
            "the projection refreshed without any explicit invalidation"
        );
        // The plan's contract for the OPEN playlist itself: the resolved
        // rows follow the new order too, read back through
        // `views.playlist_view`.
        let view = views.playlist_view(&pid).expect("known id yields a view");
        let row_order: Vec<TrackId> = view.rows.iter().map(|(id, _, _)| id.clone()).collect();
        assert_eq!(
            row_order,
            vec![
                TrackId("b.mp3".to_string()),
                TrackId("c.mp3".to_string()),
                TrackId("a.mp3".to_string())
            ],
            "the open playlist's rendered rows follow the new order"
        );
        assert!(
            view.valid_ids.is_empty(),
            "the dangling fixture entries stay flagged invalid"
        );

        // Dropping an entry back onto itself changes nothing anywhere.
        riff_gui::ui::app::commit_playlist_reorder(&mut views, store.as_mut(), &pid, 1, 1);
        assert_eq!(store.load_playlists().unwrap()[0].tracks.len(), 3);

        // Out-of-bounds gestures are ignored end to end.
        riff_gui::ui::app::commit_playlist_reorder(&mut views, store.as_mut(), &pid, 0, 9);
        assert_eq!(
            store.load_playlists().unwrap()[0].tracks,
            vec![
                TrackId("b.mp3".to_string()),
                TrackId("c.mp3".to_string()),
                TrackId("a.mp3".to_string())
            ],
            "an invalid gesture never rewrites the store"
        );
    }

    // The playlist-reorder render contract was pinned here by a
    // hand-replicated copy of the playlist render path. It is now pinned
    // through the real shell instead:
    // whole_frame_tests::test_playlist_view_reflects_a_committed_reorder_on_the_next_real_frame
    // drags a real playlist row, commits through the real drop path, and
    // asserts the next real frame's rendered order. Do not reintroduce a
    // replica: a copy of production code cannot fail when production breaks.
    #[test]
    fn test_playlist_entry_rows_drag_reorder_reports_the_move() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let labels = ["Alpha", "Beta", "Gamma"];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(256.0, theme::geometry::sidebar::ROW_H * 3.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, moves: &mut Vec<(usize, usize)>| {
                    for (i, label) in labels.iter().enumerate() {
                        let outcome = sidebar::reorderable_row(
                            ui,
                            &mut cache,
                            &palette,
                            egui::Id::new(("dnd_fixture", i)),
                            i,
                            sidebar::TreeRow {
                                indent_level: 0,
                                icon: None,
                                cover: None,
                                label,
                                count: None,
                                meta: None,
                                favorite: None,
                                selected: false,
                                now_playing: false,
                                playing: false,
                                art_slot: false,
                            },
                        );
                        if let Some(from) = outcome.drop_from {
                            moves.push((from, i));
                        }
                    }
                },
                Vec::new(),
            );
        harness.run();

        // Drag row 0 ("Alpha") onto row 2 ("Gamma"): press, move, release.
        let src = harness.get_by_label("Alpha").rect();
        let dst = harness.get_by_label("Gamma").rect();
        harness.drag_at(src.center());
        harness.run();
        harness.hover_at(dst.center());
        harness.run();
        harness.drop_at(dst.center());
        harness.run();

        assert_eq!(
            harness.state(),
            &vec![(0, 2)],
            "releasing Alpha over Gamma reports the move (from 0 to 2)"
        );
    }

    #[test]
    fn test_playlist_entry_rows_keep_click_and_context_menu_while_reorderable() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(256.0, theme::geometry::sidebar::ROW_H * 2.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, events: &mut Vec<&'static str>| {
                    let outcome = sidebar::reorderable_row(
                        ui,
                        &mut cache,
                        &palette,
                        egui::Id::new(("menu_fixture", 0)),
                        0,
                        sidebar::TreeRow {
                            indent_level: 0,
                            icon: None,
                            cover: None,
                            label: "Beta",
                            count: None,
                            meta: None,
                            favorite: None,
                            selected: false,
                            now_playing: false,
                            playing: false,
                            art_slot: false,
                        },
                    );
                    if outcome.response.clicked() {
                        events.push("clicked");
                    }
                    // Stand-in for the shared track context menu: proves the
                    // drag affordance did not swallow secondary clicks.
                    outcome.response.context_menu(|ui| {
                        let _ = ui.button("MenuProbe");
                    });
                },
                Vec::new(),
            );
        harness.run();

        // Plain clicks still select/play.
        harness.get_by_label("Beta").click();
        harness.run();
        assert!(
            harness.state().contains(&"clicked"),
            "adding drag-and-drop must not break row clicks"
        );

        // Secondary clicks still open the context menu.
        harness.get_by_label("Beta").click_secondary();
        harness.run();
        assert!(
            harness.query_by_label("MenuProbe").is_some(),
            "the context menu opens on a reorderable row"
        );
    }

    #[test]
    fn test_playerbar_icon_buttons_show_tooltips_on_hover() {
        use egui_kittest::kittest::Queryable;

        let content = playing_content();
        let palette = theme::Palette::dark();
        let mute_id = egui::Id::new("playerbar_mute");
        let shuffle_id = egui::Id::new("playerbar_shuffle");
        let play_id = egui::Id::new("playerbar_play");
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, theme::PLAYERBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, opened: &mut Vec<&'static str>| {
                    make_tooltips_instant(ui.ctx());
                    let mut readouts = riff_gui::ui::playerbar::SeekReadouts::default();
                    let mut buf = Vec::new();
                    playerbar::show_player_bar(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        &mut readouts,
                        &mut buf,
                    );
                    for (id, name) in [
                        (mute_id, "mute"),
                        (shuffle_id, "shuffle"),
                        (play_id, "play"),
                    ] {
                        if ui
                            .ctx()
                            .read_response(id)
                            .is_some_and(|r| r.is_tooltip_open())
                        {
                            opened.push(name);
                        }
                    }
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness.state().is_empty(),
            "no tooltip shows before the pointer hovers"
        );

        harness.get_by_label("Mute").hover();
        harness.run();
        assert!(
            harness.state().contains(&"mute"),
            "hovering the mute icon shows its tooltip"
        );

        harness.get_by_label("Toggle shuffle").hover();
        harness.run();
        assert!(
            harness.state().contains(&"shuffle"),
            "hovering the shuffle toggle shows its tooltip"
        );

        harness.get_by_label("Pause").hover();
        harness.run();
        assert!(
            harness.state().contains(&"play"),
            "the primary transport button shows its tooltip too"
        );
    }

    #[test]
    fn test_ghost_icon_buttons_show_tooltips_on_hover() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let btn_id = egui::Id::new("tooltip_probe");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(64.0, theme::geometry::sidebar::ROW_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, opened: &mut Vec<bool>| {
                    make_tooltips_instant(ui.ctx());
                    let rect = egui::Rect::from_center_size(
                        ui.max_rect().center(),
                        egui::vec2(24.0, 24.0),
                    );
                    let _ = sidebar::ghost_icon_button(
                        ui,
                        &mut cache,
                        &palette,
                        rect,
                        btn_id,
                        icons::Icon::Trash,
                        "Delete playlist",
                        true,
                    );
                    if ui
                        .ctx()
                        .read_response(btn_id)
                        .is_some_and(|r| r.is_tooltip_open())
                    {
                        opened.push(true);
                    }
                },
                Vec::new(),
            );
        harness.run();
        assert!(harness.state().is_empty(), "no tooltip before hovering");

        // Hover-reveal hides the glyph until hovered, but the hit target (and
        // its accessibility label) is registered every frame.
        harness.get_by_label("Delete playlist").hover();
        harness.run();
        assert!(
            !harness.state().is_empty(),
            "hovering a ghost icon button shows its tooltip"
        );
    }

    #[test]
    fn test_large_library_fixture_culls_rows_to_the_visible_window() {
        use egui_kittest::kittest::Queryable;
        use std::cell::Cell;

        /// A library far larger than any viewport: 10k rows.
        const TOTAL_ROWS: usize = 10_000;
        /// The fixture viewport fits exactly five 40px rows.
        const VIEW_ROWS: usize = 5;

        let worst_frame_rows = Cell::new(0usize);
        let frame_counter = worst_frame_rows.clone();
        let ink = theme::Palette::dark().ink;

        #[expect(clippy::cast_precision_loss)]
        let view_h = VIEW_ROWS as f32 * theme::geometry::sidebar::ROW_H;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(280.0, view_h))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, _seen: &mut Vec<()>| {
                    let mut rendered_this_frame = 0usize;
                    egui::ScrollArea::vertical()
                        .id_salt("virtualization_fixture")
                        .auto_shrink(false)
                        .show_rows(
                            ui,
                            theme::geometry::sidebar::ROW_H,
                            TOTAL_ROWS,
                            |ui, range| {
                                for i in range {
                                    rendered_this_frame += 1;
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(
                                            ui.available_width(),
                                            theme::geometry::sidebar::ROW_H,
                                        ),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().text(
                                        rect.left_center() + egui::vec2(8.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        format!("Track {i:05}"),
                                        egui::FontId::proportional(theme::TEXT_SM),
                                        ink,
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::SelectableLabel,
                                            false,
                                            format!("Track {i:05}"),
                                        )
                                    });
                                }
                            },
                        );
                    frame_counter.set(frame_counter.get().max(rendered_this_frame));
                },
                Vec::new(),
            );
        harness.run();

        // Culling: a plain widget loop would have laid out all 10_000 rows
        // every frame; row virtualization renders only the visible window.
        assert!(
            worst_frame_rows.get() <= VIEW_ROWS + 1,
            "row virtualization must cull to the visible window \
             (worst frame rendered {} rows)",
            worst_frame_rows.get()
        );

        // The window tracks scrolling: rows scrolled out leave the tree and
        // newly visible rows join it.
        assert!(
            harness.query_by_label("Track 00000").is_some(),
            "the first row renders while at the top"
        );
        assert!(
            harness.query_by_label("Track 09999").is_none(),
            "rows far below the viewport stay culled"
        );
        harness.get_by_label("Track 00001").scroll_down();
        harness.run();
        assert!(
            harness.query_by_label("Track 00000").is_none(),
            "scrolling moves the first row out of the rendered window"
        );
        assert!(
            harness.query_by_label("Track 00007").is_some(),
            "newly visible rows render after scrolling"
        );
    }
}

// --- Settings modal contract --------------------------------------------------
//
// Modal-level behavior pinned headlessly: nothing renders unless the product
// ships it. The "Reset to defaults" button is a deliberate non-feature (plan
// #2), asserted absent so a regression can't silently reintroduce it.
#[cfg(test)]
mod settings_scalar_handler_tests {
    use super::*;
    use riff_gui::ui::settings::{
        LibraryRow, SettingsAction, SettingsContent, SettingsSection, show_settings_modal,
    };
    use std::path::PathBuf;

    /// The "Reset to defaults" button was removed (plan #2): the action must
    /// not be a variant of `SettingsAction`, and the modal must not render a
    /// button with that label. Both checks live in one test so a regression
    /// (constant left behind, variant left behind) is caught by the same
    /// failure.
    #[test]
    fn test_settings_modal_does_not_render_reset_to_defaults() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::theme::Palette;

        let content = SettingsContent {
            libraries: vec![LibraryRow {
                path: PathBuf::from("C:\\Users\\stink\\Music"),
                status: LibraryStatus::Scanned(1284),
                watch: WatchState::Enabled,
                indexed_tracks: 1284,
            }],
            ..SettingsContent::default()
        };
        let palette = Palette::dark();
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let mut harness: egui_kittest::Harness<'_, Vec<SettingsAction>> =
            egui_kittest::Harness::builder()
                .with_size(egui::vec2(800.0, 720.0))
                .with_pixels_per_point(1.0)
                .build_ui_state(
                    |ui, actions: &mut Vec<SettingsAction>| {
                        actions.extend(show_settings_modal(
                            ui,
                            &mut cache,
                            &palette,
                            &content,
                            SettingsSection::Library,
                        ));
                    },
                    Vec::new(),
                );
        harness.run();

        // The label must not be painted anywhere in the modal.
        assert!(
            harness.query_by_label("Reset to defaults").is_none(),
            "the 'Reset to defaults' button must not be rendered"
        );
    }
}
// --- Background service seams in the UI ----------------------------------------
//
// The UI no longer owns worker threads or channel protocols (ADR 0006): it
// submits intent and polls outcomes through the boxed `TagEdits`/`Covers`
// handles. These tests drive the exact production code paths — the render-free
// inline Tag Edit controller, `request_cover_intent`, and `cache_polled_covers`
// that the RiffApp delegates to — over recording fakes, with no threads and
// no disk I/O.
#[cfg(test)]
mod background_service_ui_tests {
    use super::*;
    use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
    use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
    use riff_gui::ui::app::CoverCacheKey;
    use riff_gui::ui::app::{
        COVER_CACHE_CAP, COVER_HERO, COVER_IN_FLIGHT_CAP, COVER_TEXTURE_BYTE_BUDGET,
        InlineTagEditor, InspectorContent, InspectorKind, cache_polled_covers, cover_cache_key,
        folder_cover_intent, request_cache_clear, request_cover_intent, settle_cache_clear,
    };
    use riff_gui::ui::selection::{TagField, TagRow, TagRowState};
    use riff_library::app::traits::{DecodedCover, RequestedSize};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// Recording [`TagEdits`] fake: captures every submitted request and
    /// yields scripted outcomes from `poll` in order. `Clone` is a second
    /// inspection handle over the same shared state, so the controller can
    /// own one while the test inspects the other.
    #[derive(Clone)]
    struct FakeTagEdits {
        submitted: Arc<Mutex<Vec<TagEditRequest>>>,
        outcomes: Arc<Mutex<std::collections::VecDeque<TagEditOutcome>>>,
    }

    impl FakeTagEdits {
        fn new() -> Self {
            Self {
                submitted: Arc::new(Mutex::new(Vec::new())),
                outcomes: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            }
        }

        /// A fake that yields `outcomes` from `poll` in order.
        fn with_outcomes(outcomes: Vec<TagEditOutcome>) -> Self {
            let fake = Self::new();
            *fake.outcomes.lock().unwrap() = outcomes.into();
            fake
        }

        fn requests(&self) -> Vec<TagEditRequest> {
            self.submitted.lock().unwrap().clone()
        }
    }

    impl TagEdits for FakeTagEdits {
        fn submit(&self, request: TagEditRequest) {
            self.submitted.lock().unwrap().push(request);
        }

        fn poll(&self) -> Option<TagEditOutcome> {
            self.outcomes.lock().unwrap().pop_front()
        }
    }

    /// Recording [`Covers`] fake: captures request intent, serves nothing.
    /// Records cache-clear requests too, and can be handed a scripted clear
    /// outcome, so the in-progress → settled transition is driven without a thread.
    struct RecordingCovers {
        requested: Mutex<Vec<(TrackId, PathBuf, RequestedSize)>>,
        clears: AtomicUsize,
        clear_outcomes: Mutex<VecDeque<ClearCacheOutcome>>,
    }

    impl RecordingCovers {
        fn new() -> Self {
            Self {
                requested: Mutex::new(Vec::new()),
                clears: AtomicUsize::new(0),
                clear_outcomes: Mutex::new(VecDeque::new()),
            }
        }

        fn requested(&self) -> Vec<(TrackId, PathBuf, RequestedSize)> {
            self.requested.lock().unwrap().clone()
        }

        fn clear_calls(&self) -> usize {
            self.clears.load(Ordering::SeqCst)
        }

        /// Queue one settled outcome for the next `poll_cache_clear`.
        fn serve_clear(&self, outcome: ClearCacheOutcome) {
            self.clear_outcomes.lock().unwrap().push_back(outcome);
        }
    }

    impl Covers for RecordingCovers {
        fn request(&self, track_id: TrackId, path: PathBuf, size: RequestedSize) {
            self.requested.lock().unwrap().push((track_id, path, size));
        }

        /// A folder request lands in the same recording: its identity is its
        /// directory path, so the entries are told apart by what a caller puts
        /// in the slot, not by which method arrived.
        fn request_folder(&self, folder: &std::path::Path, size: RequestedSize) {
            let folder = folder.to_path_buf();
            let id = TrackId::from_path(&folder);
            self.requested.lock().unwrap().push((id, folder, size));
        }

        fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
            Vec::new()
        }

        fn clear_cache(&self) {
            self.clears.fetch_add(1, Ordering::SeqCst);
        }

        fn poll_cache_clear(&self) -> Option<ClearCacheOutcome> {
            self.clear_outcomes.lock().unwrap().pop_front()
        }
    }

    /// Fill the texture map the way a live frame does, placeholder tile included.
    fn fill_texture_map(
        ctx: &egui::Context,
        textures: &mut HashMap<CoverCacheKey, egui::TextureHandle>,
        lru_keys: &mut Vec<CoverCacheKey>,
        count: usize,
    ) {
        let box_ = RequestedSize {
            width: 8,
            height: 8,
        };
        for index in 0..count {
            let texture = ctx.load_texture(
                format!("cover-{index}"),
                egui::ColorImage::from_rgba_unmultiplied([8, 8], &vec![1u8; 8 * 8 * 4]),
                egui::TextureOptions::default(),
            );
            let key = cover_cache_key(&format!("/music/{index}.mp3"), box_);
            textures.insert(key.clone(), texture);
            lru_keys.push(key);
        }
        let placeholder = ctx.load_texture(
            "placeholder",
            egui::ColorImage::from_rgba_unmultiplied([8, 8], &vec![2u8; 8 * 8 * 4]),
            egui::TextureOptions::default(),
        );
        let key = riff_gui::ui::artwork::placeholder_cache_key();
        textures.insert(key.clone(), placeholder);
        lru_keys.push(key);
    }

    /// A clear that visibly does nothing gets pressed twice and doubted, and the
    /// listener is left wondering whether the cache went. So the *settled* outcome
    /// flushes the whole texture map — the placeholder tile included, which is what
    /// distinguishes this from `evict_generated`'s deliberate half-eviction.
    #[test]
    fn test_a_settled_cache_clear_flushes_every_texture() {
        let ctx = egui::Context::default();
        let covers = RecordingCovers::new();
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();
        fill_texture_map(&ctx, &mut textures, &mut lru_keys, 3);
        assert_eq!(textures.len(), 4, "three covers and the shared placeholder");

        let mut in_flight = false;
        assert!(
            request_cache_clear(&covers, &mut in_flight),
            "the first press reaches the worker"
        );
        assert!(in_flight, "and the frame now knows one is outstanding");
        assert_eq!(covers.clear_calls(), 1);

        // Nothing has settled: the wipe runs on the worker, so this frame must keep
        // painting the covers it already has rather than blanking on a promise.
        assert!(
            settle_cache_clear(&covers, &mut in_flight, &mut textures, &mut lru_keys).is_none()
        );
        assert_eq!(
            textures.len(),
            4,
            "a clear in progress has removed nothing yet"
        );

        covers.serve_clear(ClearCacheOutcome::Cleared);
        assert_eq!(
            settle_cache_clear(&covers, &mut in_flight, &mut textures, &mut lru_keys),
            Some(ClearCacheOutcome::Cleared)
        );
        assert!(
            textures.is_empty(),
            "every texture is gone, so the next repaint shows placeholders and the rows re-request"
        );
        assert!(
            lru_keys.is_empty(),
            "and nothing is left in the LRU order either"
        );
        assert!(
            !in_flight,
            "settled, so a later press is allowed to start a fresh clear"
        );

        // Once settled, the frame stops asking the service at all.
        assert!(
            settle_cache_clear(&covers, &mut in_flight, &mut textures, &mut lru_keys).is_none()
        );
        covers.serve_clear(ClearCacheOutcome::Cleared);
        assert!(
            settle_cache_clear(&covers, &mut in_flight, &mut textures, &mut lru_keys).is_none(),
            "an outcome is not drained into a clear nobody asked for"
        );
    }

    /// A cache that could not be cleared has left its rungs on disk, so the screen
    /// must keep what it is showing. Dropping textures here would re-request, hit
    /// the cache that is still full, and look like the failure did something.
    #[test]
    fn test_a_failed_clear_reports_but_keeps_every_texture() {
        let ctx = egui::Context::default();
        let covers = RecordingCovers::new();
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();
        fill_texture_map(&ctx, &mut textures, &mut lru_keys, 2);

        let mut in_flight = false;
        request_cache_clear(&covers, &mut in_flight);
        covers.serve_clear(ClearCacheOutcome::Failed {
            reason: "read-only volume".to_string(),
        });

        assert_eq!(
            settle_cache_clear(&covers, &mut in_flight, &mut textures, &mut lru_keys),
            Some(ClearCacheOutcome::Failed {
                reason: "read-only volume".to_string()
            }),
            "the reason reaches the caller so the status line can say it"
        );
        assert_eq!(textures.len(), 3, "a failed clear removes nothing");
        assert!(!in_flight);
    }

    /// One clear at a time, the way the Tag Edit controller holds one outstanding
    /// record: the wipe is idempotent, and a second request only delays the answer
    /// the first already promised.
    #[test]
    fn test_a_second_clear_press_while_one_is_outstanding_is_ignored() {
        let covers = RecordingCovers::new();
        let mut in_flight = false;
        assert!(request_cache_clear(&covers, &mut in_flight));
        assert!(
            !request_cache_clear(&covers, &mut in_flight),
            "the frame must not queue a second wipe behind the first"
        );
        assert_eq!(covers.clear_calls(), 1);

        covers.serve_clear(ClearCacheOutcome::Cleared);
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();
        settle_cache_clear(&covers, &mut in_flight, &mut textures, &mut lru_keys);
        assert!(
            request_cache_clear(&covers, &mut in_flight),
            "and now a press counts"
        );
        assert_eq!(covers.clear_calls(), 2);
    }

    /// A `DecodedCover` of `px`×`px` opaque pixels — big enough that a handful of
    /// them cross a byte budget a count cap cannot see.
    fn square_cover(px: u32) -> DecodedCover {
        DecodedCover {
            rgba: vec![3u8; (px * px * 4) as usize],
            width: px,
            height: px,
        }
    }

    /// The bug a count cap leaves in: the three canonical boxes differ by 80× in
    /// GPU bytes, so 200 entries is 2.4 MB of thumbnails or ~200 MB of heroes, and
    /// which one a user gets is decided by what they happened to scroll past.
    /// Eviction is now by bytes, and it bites long before the count cap does.
    #[test]
    fn test_the_texture_cache_is_bounded_by_bytes_not_by_entry_count() {
        let ctx = egui::Context::default();
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();
        let mut in_flight = HashSet::new();
        let mut in_flight_keys = Vec::new();

        // 18 uploads of 1024x1024 = 72 MB: nowhere near the 200-entry count cap, and
        // over the byte budget from the 17th on.
        let batch: Vec<(TrackId, RequestedSize, Option<DecodedCover>)> = (0..18)
            .map(|index| {
                let id = TrackId(format!("/music/big{index:02}.mp3"));
                (id, COVER_HERO, Some(square_cover(1024)))
            })
            .collect();
        cache_polled_covers(
            &CannedCovers(batch),
            &mut textures,
            &mut lru_keys,
            &mut in_flight,
            &mut in_flight_keys,
            &ctx,
        );

        let bytes: u64 = textures
            .values()
            .map(|texture| {
                let [w, h] = texture.size();
                (w * h * 4) as u64
            })
            .sum();
        assert!(
            bytes <= COVER_TEXTURE_BYTE_BUDGET,
            "{} cached textures still hold {bytes} bytes, over the budget of {COVER_TEXTURE_BYTE_BUDGET}",
            textures.len()
        );
        assert!(
            textures.len() < 18 && textures.len() > 1,
            "byte eviction, not the 200-entry cap: {} of 18 survived",
            textures.len()
        );
        assert!(
            textures.contains_key(&cover_cache_key("/music/big17.mp3", COVER_HERO)),
            "the newest entry is never the one evicted"
        );
        assert!(
            !textures.contains_key(&cover_cache_key("/music/big00.mp3", COVER_HERO)),
            "and the oldest goes first"
        );
        assert_eq!(
            lru_keys.len(),
            textures.len(),
            "the LRU order and the map agree after a budget-driven eviction"
        );
    }

    /// The shared placeholder tile is one entry drawn at every box, and
    /// `evict_generated`'s contract is that it alone is dropped when the palette
    /// moves. Under a byte budget it must be excluded from the accounting *and*
    /// skipped as a victim — otherwise the tile that guarantees every row has
    /// something to paint becomes the first thing evicted, and it is always the
    /// oldest.
    #[test]
    fn test_the_placeholder_survives_budget_driven_eviction() {
        let ctx = egui::Context::default();
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();
        let mut in_flight = HashSet::new();
        let mut in_flight_keys = Vec::new();

        // The tile first, so it is the LRU tail and the obvious victim.
        let tile = crate::ui::artwork::lookup_cover_texture(
            &mut textures,
            &mut lru_keys,
            &ctx,
            &riff_gui::ui::theme::Palette::dark(),
            "/music/none.mp3",
            COVER_HERO,
        );
        assert!(
            textures.contains_key(&crate::ui::artwork::placeholder_cache_key()),
            "a full miss uploads the shared tile, oldest in the order"
        );
        drop(tile);

        let batch: Vec<(TrackId, RequestedSize, Option<DecodedCover>)> = (0..20)
            .map(|index| {
                let id = TrackId(format!("/music/huge{index:02}.mp3"));
                (id, COVER_HERO, Some(square_cover(1024)))
            })
            .collect();
        cache_polled_covers(
            &CannedCovers(batch),
            &mut textures,
            &mut lru_keys,
            &mut in_flight,
            &mut in_flight_keys,
            &ctx,
        );

        assert!(
            textures.contains_key(&crate::ui::artwork::placeholder_cache_key()),
            "the tile outlives an eviction that would otherwise take the oldest entry"
        );
        assert!(
            lru_keys.contains(&crate::ui::artwork::placeholder_cache_key()),
            "and stays in the order, so it is still a live entry rather than a stray"
        );
    }

    /// Canned [`Covers`] fake whose single poll drains scripted results.
    struct CannedCovers(Vec<(TrackId, RequestedSize, Option<DecodedCover>)>);

    impl Covers for CannedCovers {
        fn request(&self, _track_id: TrackId, _path: PathBuf, _size: RequestedSize) {}

        fn request_folder(&self, _folder: &std::path::Path, _size: RequestedSize) {}

        fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
            self.0.clone()
        }

        fn clear_cache(&self) {}

        fn poll_cache_clear(&self) -> Option<ClearCacheOutcome> {
            None
        }
    }

    // --- Inline editor (deepen-three-modules issue 02): the render-free
    // controller -----------------------------------------------------------

    /// A readout tag row whose value is shared across the readout's tracks.
    fn tag_row(field: TagField, text: &str) -> TagRow {
        TagRow {
            field,
            state: TagRowState::Value,
            text: text.to_string(),
            originals: vec![Some(text.to_string())],
        }
    }

    /// The seven readout rows a track readout resolves: every field a shared
    /// value, so the draft opens with each buffer equal to its original.
    fn track_tags() -> Vec<TagRow> {
        vec![
            tag_row(TagField::Title, "Old Title"),
            tag_row(TagField::Artist, "Artist"),
            tag_row(TagField::Album, "Album"),
            tag_row(TagField::AlbumArtist, "Album Artist"),
            tag_row(TagField::Genre, "Genre"),
            tag_row(TagField::Year, "2001"),
            tag_row(TagField::TrackNumber, "7"),
        ]
    }

    /// The seven readout rows an album readout resolves: Title and Year
    /// shared, Genre a `(different)` row (opens empty, never rewritten),
    /// Track Number a `(none)` row.
    fn album_tags() -> Vec<TagRow> {
        vec![
            tag_row(TagField::Title, "Old Album"),
            tag_row(TagField::Artist, "Artist"),
            tag_row(TagField::Album, "Album"),
            tag_row(TagField::AlbumArtist, "Album Artist"),
            TagRow {
                field: TagField::Genre,
                state: TagRowState::Different,
                text: "(different)".to_string(),
                originals: vec![None, None, None],
            },
            tag_row(TagField::Year, "2001"),
            TagRow {
                field: TagField::TrackNumber,
                state: TagRowState::None,
                text: "(none)".to_string(),
                originals: vec![None, None, None],
            },
        ]
    }

    /// The inspector content a readout of `kind` over `track_ids` resolves.
    fn content(kind: InspectorKind, track_ids: Vec<TrackId>) -> InspectorContent {
        InspectorContent {
            visible: true,
            kind,
            title: None,
            subtitle: None,
            art_track: None,
            track_ids,
            details: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// A fresh controller over `edits` with the single-track draft for
    /// `/music/t1.mp3` open and Title typed over its original.
    fn track_editor(edits: &FakeTagEdits) -> InlineTagEditor {
        let mut editor = InlineTagEditor::new(Box::new(edits.clone()));
        editor.open_track(
            TrackId("/music/t1.mp3".to_string()),
            PathBuf::from("/music/t1.mp3"),
            &track_tags(),
        );
        if let Some(draft) = editor.draft_mut() {
            draft.fields[TagField::Title.index()] = "New Title".to_string();
        }
        editor
    }

    /// A fresh controller over `edits` with an album draft over three tracks
    /// open and Title and Year typed over their originals.
    fn album_editor(edits: &FakeTagEdits) -> InlineTagEditor {
        let mut editor = InlineTagEditor::new(Box::new(edits.clone()));
        editor.open_album(
            vec![
                (
                    TrackId("/music/a1.mp3".to_string()),
                    PathBuf::from("/music/a1.mp3"),
                ),
                (
                    TrackId("/music/a2.mp3".to_string()),
                    PathBuf::from("/music/a2.mp3"),
                ),
                (
                    TrackId("/music/a3.mp3".to_string()),
                    PathBuf::from("/music/a3.mp3"),
                ),
            ],
            &album_tags(),
        );
        if let Some(draft) = editor.draft_mut() {
            draft.fields[TagField::Title.index()] = "New Album".to_string();
            draft.fields[TagField::Year.index()] = "2002".to_string();
        }
        editor
    }

    #[test]
    fn test_editor_track_save_sends_one_request_with_the_draft_fields() {
        let edits = FakeTagEdits::new();
        let mut editor = track_editor(&edits);

        editor.save();

        let requests = edits.requests();
        assert_eq!(requests.len(), 1, "a single-track save is one request");
        let request = &requests[0];
        assert_eq!(request.track_id.0, "/music/t1.mp3");
        assert_eq!(request.path, PathBuf::from("/music/t1.mp3"));
        assert_eq!(request.edit.title.as_deref(), Some("New Title"));
        assert_eq!(request.edit.artist.as_deref(), Some("Artist"));
        assert_eq!(request.edit.album.as_deref(), Some("Album"));
        assert_eq!(request.edit.album_artist.as_deref(), Some("Album Artist"));
        assert_eq!(request.edit.genre.as_deref(), Some("Genre"));
        assert_eq!(request.edit.year, Some(2001));
        assert_eq!(request.edit.track_number, Some(7));
        let draft = editor
            .draft()
            .expect("a submitted track save keeps the draft open");
        assert!(draft.saving, "the draft flips into its saving state");
        assert!(draft.error.is_none());
    }

    #[test]
    fn test_editor_invalid_numeric_save_keeps_draft_open_without_submitting() {
        let edits = FakeTagEdits::new();
        let mut editor = track_editor(&edits);
        if let Some(draft) = editor.draft_mut() {
            draft.fields[TagField::Year.index()] = "not a number".to_string();
        }

        editor.save();

        assert!(
            edits.requests().is_empty(),
            "invalid fields must not reach the service"
        );
        let draft = editor
            .draft()
            .expect("a failed submit keeps the editor open");
        assert!(draft.error.is_some(), "the parse error surfaces inline");
        assert!(!draft.saving, "the draft stays open, not saving");
    }

    #[test]
    fn test_editor_saved_outcome_closes_draft_and_sets_status_line() {
        let edits = FakeTagEdits::with_outcomes(vec![TagEditOutcome::Saved]);
        let mut editor = track_editor(&edits);
        editor.save();
        let mut board = riff_gui::ui::feedback::FeedbackBoard::default();

        editor.poll_outcomes(&mut board);

        assert!(
            editor.draft().is_none(),
            "a saved edit closes the inline editor"
        );
        assert_eq!(
            board.display_message().as_deref(),
            Some("Tags saved for t1.mp3"),
            "the status line names the saved file"
        );
    }

    #[test]
    fn test_editor_failed_outcome_keeps_draft_open_with_inline_reason() {
        let edits = FakeTagEdits::with_outcomes(vec![TagEditOutcome::Failed {
            reason: "permission denied".to_string(),
        }]);
        let mut editor = track_editor(&edits);
        editor.save();
        let mut board = riff_gui::ui::feedback::FeedbackBoard::default();
        board.set_tag_edit(
            "earlier message",
            riff_backend::app::events::NoticeSeverity::Info,
        );

        editor.poll_outcomes(&mut board);

        let draft = editor.draft().expect("a failed edit keeps the editor open");
        assert_eq!(draft.error.as_deref(), Some("permission denied"));
        assert!(!draft.saving, "the save spinner stops");
        assert_eq!(
            board.display_message().as_deref(),
            Some("earlier message"),
            "a failed inline save does not clear the status line"
        );
    }

    #[test]
    fn test_editor_outcome_after_selection_change_still_reaches_status_line() {
        // The selection moved while the write was in flight, so the draft was
        // already discarded — the outcome must still land on the status line
        // and never resurrect the editor.
        let edits = FakeTagEdits::with_outcomes(vec![TagEditOutcome::Saved]);
        let mut editor = track_editor(&edits);
        editor.save();
        editor.reconcile(&content(
            InspectorKind::Track,
            vec![TrackId("/music/t9.mp3".to_string())],
        ));
        assert!(
            editor.draft().is_none(),
            "the moved selection discarded the draft"
        );
        let mut board = riff_gui::ui::feedback::FeedbackBoard::default();

        editor.poll_outcomes(&mut board);

        assert_eq!(
            board.display_message().as_deref(),
            Some("Tags saved for t1.mp3"),
            "the outcome still reaches the status line after the selection moved"
        );
        assert!(
            editor.draft().is_none(),
            "the discarded draft is never resurrected"
        );
    }

    #[test]
    fn test_editor_track_draft_discarded_on_selection_change() {
        let edits = FakeTagEdits::new();

        // The same track's readout keeps the draft.
        let mut editor = track_editor(&edits);
        editor.reconcile(&content(
            InspectorKind::Track,
            vec![TrackId("/music/t1.mp3".to_string())],
        ));
        assert!(
            editor.draft().is_some(),
            "the same track's readout keeps the draft"
        );

        // A different track's readout discards it.
        let mut editor = track_editor(&edits);
        editor.reconcile(&content(
            InspectorKind::Track,
            vec![TrackId("/music/t9.mp3".to_string())],
        ));
        assert!(
            editor.draft().is_none(),
            "a different track's readout discards the draft"
        );

        // An album readout never hosts a track draft.
        let mut editor = track_editor(&edits);
        editor.reconcile(&content(
            InspectorKind::Album,
            vec![TrackId("/music/t1.mp3".to_string())],
        ));
        assert!(
            editor.draft().is_none(),
            "an album readout never hosts a track draft"
        );

        // An artist readout never hosts one either.
        let mut editor = track_editor(&edits);
        editor.reconcile(&content(InspectorKind::Artist, Vec::new()));
        assert!(
            editor.draft().is_none(),
            "an artist readout never hosts a track draft"
        );
    }

    #[test]
    fn test_editor_album_save_submits_one_request_per_track_with_only_dirty_fields() {
        let edits = FakeTagEdits::new();
        let mut editor = album_editor(&edits);

        editor.save();

        let requests = edits.requests();
        assert_eq!(
            requests.len(),
            3,
            "an album save is exactly one request per album track"
        );
        assert_eq!(requests[0].track_id.0, "/music/a1.mp3");
        assert_eq!(requests[2].track_id.0, "/music/a3.mp3");
        assert_eq!(requests[0].path, PathBuf::from("/music/a1.mp3"));
        for request in &requests {
            assert_eq!(
                request.edit.title.as_deref(),
                Some("New Album"),
                "a dirty field is present on every track request"
            );
            assert_eq!(request.edit.year, Some(2002));
            // Untouched fields stay `None`: the batch never rewrites them.
            assert_eq!(request.edit.artist, None);
            assert_eq!(request.edit.album, None);
            assert_eq!(request.edit.album_artist, None);
            assert_eq!(request.edit.track_number, None);
            assert_eq!(
                request.edit.genre, None,
                "a (different) row left empty is untouched and skipped"
            );
        }
        let draft = editor
            .draft()
            .expect("a submitted album save keeps the draft open");
        assert!(draft.saving, "the draft flips into its saving state");
        assert!(draft.error.is_none());
        let status = draft.batch.as_ref().expect("a batch is now in flight");
        assert_eq!(status.total, 3);
        assert!(!status.done());
    }

    #[test]
    fn test_editor_untouched_album_save_submits_nothing() {
        // Nothing edited back to its readout value: the album save would be
        // an empty batch, so it submits nothing and writes no files.
        let edits = FakeTagEdits::new();
        let mut editor = InlineTagEditor::new(Box::new(edits.clone()));
        editor.open_album(
            vec![(
                TrackId("/music/a1.mp3".to_string()),
                PathBuf::from("/music/a1.mp3"),
            )],
            &album_tags(),
        );

        editor.save();

        assert!(edits.requests().is_empty(), "nothing dirty submits nothing");
        let draft = editor.draft().expect("the editor stays open");
        assert!(!draft.saving);
    }

    #[test]
    fn test_editor_album_invalid_numeric_save_keeps_draft_open_without_submitting() {
        let edits = FakeTagEdits::new();
        let mut editor = album_editor(&edits);
        if let Some(draft) = editor.draft_mut() {
            draft.fields[TagField::Year.index()] = "not a number".to_string();
        }

        editor.save();

        assert!(
            edits.requests().is_empty(),
            "invalid fields must not reach the service"
        );
        let draft = editor
            .draft()
            .expect("a failed submit keeps the editor open");
        assert!(draft.error.is_some(), "the parse error surfaces inline");
        assert!(!draft.saving);
    }

    #[test]
    fn test_editor_batch_outcomes_build_the_partial_failure_summary() {
        let edits = FakeTagEdits::with_outcomes(vec![
            TagEditOutcome::Saved,
            TagEditOutcome::Saved,
            TagEditOutcome::Failed {
                reason: "permission denied".to_string(),
            },
        ]);
        let mut editor = album_editor(&edits);
        editor.save();
        let mut board = riff_gui::ui::feedback::FeedbackBoard::default();
        board.set_tag_edit(
            "earlier message",
            riff_backend::app::events::NoticeSeverity::Info,
        );

        // Two saves, then one failure: the tallies land in order.
        editor.poll_outcomes(&mut board);

        assert_eq!(
            board.display_message().as_deref(),
            Some("permission denied"),
            "a failed request surfaces its reason on the status line"
        );
        let draft = editor.draft().expect("the draft stays open after a batch");
        assert!(!draft.saving, "the spinner stops when the batch lands");
        let batch = draft.batch.as_ref().expect("the draft keeps its tallies");
        assert!(batch.done());
        assert_eq!(
            batch.summary().as_deref(),
            Some("Saved 2 of 3 tracks — 1 failed: permission denied"),
            "the summary reports the partial failure honestly, first reason"
        );
    }

    #[test]
    fn test_editor_batch_all_saved_summary() {
        use riff_gui::ui::selection::BatchStatus;

        let mut status = BatchStatus {
            total: 3,
            saved: 3,
            failed: 0,
            first_failure: None,
        };
        assert!(status.done());
        assert_eq!(status.summary().as_deref(), Some("Saved 3 of 3 tracks"));

        // Still outstanding: no summary while the bar spins.
        status.saved = 1;
        assert!(!status.done());
        assert_eq!(status.summary(), None);
        assert!(
            !BatchStatus {
                total: 3,
                saved: 1,
                failed: 0,
                first_failure: None
            }
            .done()
        );
    }

    #[test]
    fn test_editor_batch_outcomes_reach_status_line_after_selection_change() {
        // The selection moved while the batch was in flight: the draft is
        // gone, but the outstanding outcomes still land on the status line —
        // the editor is never resurrected.
        let edits = FakeTagEdits::with_outcomes(vec![
            TagEditOutcome::Saved,
            TagEditOutcome::Saved,
            TagEditOutcome::Saved,
        ]);
        let mut editor = album_editor(&edits);
        editor.save();
        editor.reconcile(&content(
            InspectorKind::Album,
            vec![TrackId("/music/a9.mp3".to_string())],
        ));
        assert!(editor.draft().is_none());
        let mut board = riff_gui::ui::feedback::FeedbackBoard::default();

        editor.poll_outcomes(&mut board);

        assert_eq!(
            board.display_message().as_deref(),
            Some("Tags saved for a3.mp3"),
            "each outcome still reaches the status line after the selection moved"
        );
        assert!(
            editor.draft().is_none(),
            "the discarded draft is never resurrected"
        );
    }

    #[test]
    fn test_editor_album_draft_discarded_on_selection_change() {
        let edits = FakeTagEdits::new();

        // The same album's readout keeps the draft.
        let mut editor = album_editor(&edits);
        editor.reconcile(&content(
            InspectorKind::Album,
            vec![
                TrackId("/music/a1.mp3".to_string()),
                TrackId("/music/a2.mp3".to_string()),
                TrackId("/music/a3.mp3".to_string()),
            ],
        ));
        assert!(
            editor.draft().is_some(),
            "the same album's readout keeps the draft"
        );

        // A different album's readout discards it.
        let mut editor = album_editor(&edits);
        editor.reconcile(&content(
            InspectorKind::Album,
            vec![TrackId("/music/a9.mp3".to_string())],
        ));
        assert!(
            editor.draft().is_none(),
            "a different album's readout discards the album draft"
        );

        // A track readout discards the album draft.
        let mut editor = album_editor(&edits);
        editor.reconcile(&content(
            InspectorKind::Track,
            vec![TrackId("/music/a1.mp3".to_string())],
        ));
        assert!(
            editor.draft().is_none(),
            "a track readout discards the album draft"
        );
    }

    #[test]
    fn test_cover_intent_requests_only_when_that_exact_size_is_uncached() {
        let id = TrackId("/music/t1.mp3".to_string());
        let path = PathBuf::from("/music/t1.mp3");
        let thumb = RequestedSize {
            width: 56,
            height: 56,
        };
        let hero = RequestedSize {
            width: 512,
            height: 512,
        };
        let covers = RecordingCovers::new();
        let ctx = egui::Context::default();
        let hero_texture = ctx.load_texture(
            "hero",
            egui::ColorImage::from_rgba_unmultiplied([2, 2], &[0; 16]),
            egui::TextureOptions::default(),
        );
        let mut textures: std::collections::HashMap<_, egui::TextureHandle> =
            std::collections::HashMap::new();

        let mut in_flight = HashSet::new();
        let mut in_flight_keys = Vec::new();
        let mut texture_lru = Vec::new();

        request_cover_intent(
            &textures,
            &mut in_flight,
            &mut in_flight_keys,
            &covers,
            id.clone(),
            path.clone(),
            thumb,
        );
        assert_eq!(
            covers.requested(),
            vec![(id.clone(), path.clone(), thumb)],
            "an uncached track sends intent, with the box it wants, to the service"
        );

        // The first answer arrives and is artless, so no texture lands: the
        // thumbnail is still a miss at this box. Delivering it first is what keeps
        // this test about the *cache* rather than about the in-flight marker —
        // otherwise a suppressed second request could mean either thing.
        let mut artless = CannedCovers(vec![(id.clone(), thumb, None)]);
        cache_polled_covers(
            &artless,
            &mut textures,
            &mut texture_lru,
            &mut in_flight,
            &mut in_flight_keys,
            &ctx,
        );
        artless.0.clear();

        // A hero upload exists; a thumbnail request is still a miss. Reusing
        // the hero texture there would draw the wrong resolution and, worse,
        // never fetch the right one.
        textures.insert(cover_cache_key(&id.0, hero), hero_texture);
        request_cover_intent(
            &textures,
            &mut in_flight,
            &mut in_flight_keys,
            &covers,
            id.clone(),
            path.clone(),
            thumb,
        );
        assert_eq!(
            covers.requested(),
            vec![
                (id.clone(), path.clone(), thumb),
                (id.clone(), path.clone(), thumb),
            ],
            "a track cached at another size issues a fresh request at this size"
        );

        request_cover_intent(
            &textures,
            &mut in_flight,
            &mut in_flight_keys,
            &covers,
            id.clone(),
            path.clone(),
            hero,
        );
        assert_eq!(
            covers.requested().len(),
            2,
            "a texture already cached at exactly this box suppresses the request"
        );
    }

    /// The repaint bug `07` exists for: a row whose Cover has not landed asked
    /// again on *every* frame, against an unbounded channel. One request per
    /// outstanding `(identity, box)` is the whole fix, and the marker has to
    /// survive until the answer arrives.
    #[test]
    fn test_a_cover_request_still_outstanding_is_not_issued_again() {
        let id = TrackId("/music/slow.mp3".to_string());
        let path = PathBuf::from("/music/slow.mp3");
        let box_ = RequestedSize {
            width: 56,
            height: 56,
        };
        let covers = RecordingCovers::new();
        let textures: HashMap<CoverCacheKey, egui::TextureHandle> = HashMap::new();
        let mut in_flight = HashSet::new();
        let mut in_flight_keys = Vec::new();

        for _frame in 0..40 {
            request_cover_intent(
                &textures,
                &mut in_flight,
                &mut in_flight_keys,
                &covers,
                id.clone(),
                path.clone(),
                box_,
            );
        }
        assert_eq!(
            covers.requested().len(),
            1,
            "forty repaints of one outstanding request are still one request"
        );

        // A different box for the same row is a different job, marker or not.
        request_cover_intent(
            &textures,
            &mut in_flight,
            &mut in_flight_keys,
            &covers,
            id.clone(),
            path.clone(),
            RequestedSize {
                width: 512,
                height: 512,
            },
        );
        assert_eq!(covers.requested().len(), 2);
    }

    /// `None` is a terminal outcome too. A row that asked once and turned out to
    /// be artless must be free to ask again — otherwise a single artless answer
    /// silences that `(identity, box)` for the rest of the session.
    #[test]
    fn test_an_artless_answer_clears_the_marker_so_the_row_can_ask_again() {
        let id = TrackId("/music/artless.mp3".to_string());
        let path = PathBuf::from("/music/artless.mp3");
        let box_ = RequestedSize {
            width: 56,
            height: 56,
        };
        let covers = RecordingCovers::new();
        let ctx = egui::Context::default();
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();
        let mut in_flight = HashSet::new();
        let mut in_flight_keys = Vec::new();

        request_cover_intent(
            &textures,
            &mut in_flight,
            &mut in_flight_keys,
            &covers,
            id.clone(),
            path.clone(),
            box_,
        );
        assert_eq!(in_flight.len(), 1);

        cache_polled_covers(
            &CannedCovers(vec![(id.clone(), box_, None)]),
            &mut textures,
            &mut lru_keys,
            &mut in_flight,
            &mut in_flight_keys,
            &ctx,
        );
        assert!(
            in_flight.is_empty(),
            "an artless answer is still an answer, so the marker goes"
        );
        assert!(in_flight_keys.is_empty(), "and its place in the LRU too");
        assert!(textures.is_empty(), "artless uploads nothing");

        request_cover_intent(
            &textures,
            &mut in_flight,
            &mut in_flight_keys,
            &covers,
            id.clone(),
            path.clone(),
            box_,
        );
        assert_eq!(
            covers.requested().len(),
            2,
            "and the row is free to ask again"
        );
    }

    /// The texture map is bounded, and this set grows per `(identity, box)` rather
    /// than per cached texture — so without its own cap it would be a slow leak
    /// across a scrolled library. Bounded, and the evicted entries are the *oldest*
    /// markers, which cost one re-request each and nothing worse.
    #[test]
    fn test_the_in_flight_set_stays_within_its_cap() {
        let covers = RecordingCovers::new();
        let textures: HashMap<CoverCacheKey, egui::TextureHandle> = HashMap::new();
        let mut in_flight = HashSet::new();
        let mut in_flight_keys = Vec::new();
        let box_ = RequestedSize {
            width: 56,
            height: 56,
        };

        for index in 0..3_000 {
            let path = PathBuf::from(format!("/music/{index:04}.mp3"));
            request_cover_intent(
                &textures,
                &mut in_flight,
                &mut in_flight_keys,
                &covers,
                TrackId::from_path(&path),
                path,
                box_,
            );
        }
        assert_eq!(
            covers.requested().len(),
            3_000,
            "every distinct row still gets its request; the cap bounds the marker, not the asking"
        );
        assert!(
            in_flight.len() <= COVER_IN_FLIGHT_CAP && in_flight_keys.len() <= COVER_IN_FLIGHT_CAP,
            "set {} / list {} must stay at or under the cap of {COVER_IN_FLIGHT_CAP}",
            in_flight.len(),
            in_flight_keys.len()
        );
        assert_eq!(
            in_flight.len(),
            in_flight_keys.len(),
            "the set and its LRU list must not drift apart, or an entry is unmarkable forever"
        );
    }

    #[test]
    fn test_folder_cover_intent_asks_once_then_paints_the_cached_texture() {
        let dir = std::path::Path::new("/music/boards");
        let thumb = RequestedSize {
            width: 56,
            height: 56,
        };
        let covers = RecordingCovers::new();
        let mut textures: std::collections::HashMap<_, egui::TextureHandle> =
            std::collections::HashMap::new();

        // A cold folder has nothing to paint, so the row keeps its glyph and
        // exactly one request goes out — for the directory itself.
        assert_eq!(
            folder_cover_intent(
                &textures,
                &mut HashSet::new(),
                &mut Vec::new(),
                &covers,
                dir,
                thumb
            ),
            None,
            "an uncached folder has no texture to paint"
        );
        assert_eq!(
            covers.requested(),
            vec![(TrackId::from_path(dir), dir.to_path_buf(), thumb,)],
            "the folder row asks for its OWN directory, at the box it paints at"
        );

        // The decode landing in the cache is `cache_polled_covers`' work; here
        // it is simply already filed, and the row must find it.
        let texture = egui::Context::default().load_texture(
            "folder",
            egui::ColorImage::from_rgba_unmultiplied([2, 2], &[0; 16]),
            egui::TextureOptions::default(),
        );
        let id = texture.id();
        textures.insert(cover_cache_key(&dir.to_string_lossy(), thumb), texture);
        assert_eq!(
            folder_cover_intent(
                &textures,
                &mut HashSet::new(),
                &mut Vec::new(),
                &covers,
                dir,
                thumb
            ),
            Some(id),
            "the cached art is handed back for the row to paint"
        );
        assert_eq!(
            covers.requested().len(),
            1,
            "a folder already cached at this box sends nothing"
        );
    }

    #[test]
    fn test_cache_polled_covers_uploads_the_delivered_pixels_untouched() {
        let ctx = egui::Context::default();
        let big = RequestedSize {
            width: 512,
            height: 512,
        };
        // 3x2 pixels handed over already decoded. The texture that comes out
        // must be 3x2: a decode step would have had encoded bytes to read and
        // nothing here supplies any.
        let cover = DecodedCover {
            rgba: vec![0; 3 * 2 * 4],
            width: 3,
            height: 2,
        };
        let covers = CannedCovers(vec![
            (TrackId("/music/art.mp3".to_string()), big, Some(cover)),
            (TrackId("/music/artless.mp3".to_string()), big, None),
        ]);
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        cache_polled_covers(
            &covers,
            &mut textures,
            &mut lru_keys,
            &mut HashSet::new(),
            &mut Vec::new(),
            &ctx,
        );

        let texture = textures
            .get(&cover_cache_key("/music/art.mp3", big))
            .expect("resolved art becomes a texture keyed by track and size");
        assert_eq!(
            texture.size(),
            [3, 2],
            "the uploaded texture is exactly the delivered pixel grid"
        );
        assert!(
            !textures.contains_key(&cover_cache_key("/music/artless.mp3", big)),
            "artless results create no texture (the service negative-caches them)"
        );
        assert_eq!(lru_keys, vec![cover_cache_key("/music/art.mp3", big)]);
    }

    #[test]
    fn test_lru_eviction_counts_each_size_as_its_own_entry() {
        // Fill the cache to the cap at one size, then land a new size for the
        // OLDEST track: the cap is on cache entries, so the new composite key
        // is entry cap+1 and evicts the oldest — while the track it belongs to
        // keeps its freshly inserted texture.
        let ctx = egui::Context::default();
        let thumb = RequestedSize {
            width: 56,
            height: 56,
        };
        let hero = RequestedSize {
            width: 512,
            height: 512,
        };
        let cover = DecodedCover {
            rgba: vec![0; 4],
            width: 1,
            height: 1,
        };
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        let track_at = |i: usize| TrackId(format!("/music/t{i:03}.mp3"));
        let full: Vec<_> = (0..COVER_CACHE_CAP)
            .map(|i| (track_at(i), thumb, Some(cover.clone())))
            .collect();
        cache_polled_covers(
            &CannedCovers(full),
            &mut textures,
            &mut lru_keys,
            &mut HashSet::new(),
            &mut Vec::new(),
            &ctx,
        );
        assert_eq!(
            textures.len(),
            COVER_CACHE_CAP,
            "exactly the cap, so nothing has been evicted yet"
        );

        let oldest = track_at(0);
        let newcomer = CannedCovers(vec![(oldest.clone(), hero, Some(cover))]);
        cache_polled_covers(
            &newcomer,
            &mut textures,
            &mut lru_keys,
            &mut HashSet::new(),
            &mut Vec::new(),
            &ctx,
        );

        assert!(
            !textures.contains_key(&cover_cache_key(&oldest.0, thumb)),
            "the oldest entry of the whole cache is what the cap evicts"
        );
        assert!(
            textures.contains_key(&cover_cache_key(&oldest.0, hero)),
            "the entry that caused the eviction survives it"
        );
        assert!(
            textures.contains_key(&cover_cache_key(&track_at(1).0, thumb)),
            "the next-oldest track's thumbnail is untouched"
        );
        assert_eq!(
            lru_keys.len(),
            COVER_CACHE_CAP,
            "the LRU order stays bounded to the cap across sizes"
        );
        assert_eq!(
            textures.len(),
            COVER_CACHE_CAP,
            "one eviction per insertion past the cap"
        );
    }

    #[test]
    fn test_cache_polled_covers_keeps_one_texture_per_requested_size() {
        // The same track polled at two boxes must produce two textures: a
        // hero upload and a thumbnail upload are different pixels, and
        // neither may stand in for the other.
        let ctx = egui::Context::default();
        let id = TrackId("/music/art.mp3".to_string());
        let thumb = RequestedSize {
            width: 56,
            height: 56,
        };
        let hero = RequestedSize {
            width: 512,
            height: 512,
        };
        let covers = CannedCovers(vec![
            (
                id.clone(),
                thumb,
                Some(DecodedCover {
                    rgba: vec![0; 4],
                    width: 1,
                    height: 1,
                }),
            ),
            (
                id.clone(),
                hero,
                Some(DecodedCover {
                    rgba: vec![0; 16],
                    width: 2,
                    height: 2,
                }),
            ),
        ]);
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        cache_polled_covers(
            &covers,
            &mut textures,
            &mut lru_keys,
            &mut HashSet::new(),
            &mut Vec::new(),
            &ctx,
        );

        assert_eq!(
            textures.len(),
            2,
            "one track at two sizes is two cached textures"
        );
        assert_eq!(
            textures
                .get(&cover_cache_key(&id.0, thumb))
                .map(|t| t.size()),
            Some([1, 1]),
            "the thumbnail entry holds the thumbnail pixels"
        );
        assert_eq!(
            textures
                .get(&cover_cache_key(&id.0, hero))
                .map(|t| t.size()),
            Some([2, 2]),
            "the hero entry holds the hero pixels"
        );
        assert_eq!(
            lru_keys.len(),
            2,
            "each size is its own entry in the LRU order"
        );
    }
}

/// Issue 11: typed notices keep their source and severity across the
/// application→paint boundary by landing in a [`FeedbackBoard`] source slot.
/// Playback errors, Library Scan progress, and Tag Edit outcomes each own a
/// slot, so one stream can no longer overwrite another.
#[cfg(test)]
mod playback_notice_ui_tests {
    use riff_backend::app::events::{BackendEvent, NoticePayload, NoticeSeverity, NoticeSource};
    use riff_gui::ui::app::apply_backend_events;
    use riff_gui::ui::feedback::FeedbackBoard;

    fn notice(source: NoticeSource, severity: NoticeSeverity, message: &str) -> BackendEvent {
        BackendEvent::TypedNotice(NoticePayload {
            severity,
            source,
            message: message.to_string(),
        })
    }

    #[test]
    fn test_playback_typed_notice_routes_message_to_status_line() {
        let mut board = FeedbackBoard::default();

        apply_backend_events(
            vec![notice(
                NoticeSource::Playback,
                NoticeSeverity::Error,
                "Playback error: boom",
            )],
            &mut board,
        );

        assert_eq!(
            board.display_message().as_deref(),
            Some("Playback error: boom"),
            "the exact user-facing string reaches the status line"
        );
        assert_eq!(
            board.display().map(|f| &f.source),
            Some(&NoticeSource::Playback),
            "the notice's playback source survives the boundary"
        );
    }

    #[test]
    fn test_scan_notice_does_not_erase_a_live_playback_error() {
        let mut board = FeedbackBoard::default();
        // A playback error is already on the board.
        apply_backend_events(
            vec![notice(
                NoticeSource::Playback,
                NoticeSeverity::Error,
                "Playback error: boom",
            )],
            &mut board,
        );
        // A lower-severity Library Scan update then arrives.
        apply_backend_events(
            vec![notice(
                NoticeSource::Scan,
                NoticeSeverity::Info,
                "Scanning 12 files",
            )],
            &mut board,
        );

        // The scan notice lives in its own slot and cannot clobber the playback
        // error, which still outranks it on display.
        assert_eq!(
            board.display_message().as_deref(),
            Some("Playback error: boom"),
            "a Library Scan update must not erase unrelated playback feedback"
        );
        assert_eq!(
            board
                .display()
                .map(|f| (f.source.clone(), f.severity.clone())),
            Some((NoticeSource::Playback, NoticeSeverity::Error)),
            "the surviving notice keeps its source and severity"
        );
    }

    #[test]
    fn test_empty_notice_batch_and_clear_leave_the_line_empty() {
        let mut board = FeedbackBoard::default();
        board.set_scan("Scanning 12 files", NoticeSeverity::Info);
        assert!(!board.is_empty());

        // An empty drained batch changes nothing.
        apply_backend_events(Vec::new(), &mut board);
        assert_eq!(
            board.display_message().as_deref(),
            Some("Scanning 12 files")
        );

        // Clearing the source empties it; clearing all empties the board.
        board.clear(&NoticeSource::Scan);
        assert!(board.is_empty());
        assert_eq!(board.display_message(), None);
    }
}

#[cfg(test)]
mod titlebar_search_ui_tests {
    // --- Titlebar search (shared chrome) -----------------------------------------
    //
    // The global "Search or jump to…" field lives in the titlebar — shared
    // chrome present on every View (the content top bar was deleted). The
    // headless seams are the search field's query-buffer editing, its Escape
    // dismissal, and the Ctrl+K request-focus contract; the pixels are
    // covered by the titlebar goldens.

    use riff_gui::ui::chrome::{TitleBarContent, show_titlebar};

    /// A default titlebar content: no scan status, dark theme, no active nav.
    fn content() -> TitleBarContent<'static> {
        TitleBarContent::default()
    }

    #[test]
    fn test_titlebar_search_field_edits_the_query_buffer() {
        use egui_kittest::kittest::Queryable;

        let palette = riff_gui::ui::theme::Palette::dark();
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, riff_gui::ui::theme::TITLEBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, query| {
                    show_titlebar(ui, &mut cache, &palette, &content(), query, &mut Vec::new());
                },
                String::new(),
            );
        harness
            .ctx
            .set_fonts(riff_gui::ui::fonts::font_definitions());
        harness.run();

        // Typing into the field writes straight into the caller's query
        // buffer — the same `library.search_query` the library filters on.
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .focus();
        harness.run();
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .type_text("boards of canada");
        harness.run();

        assert_eq!(harness.state(), "boards of canada");
    }

    #[test]
    fn test_ctrl_k_focuses_global_search() {
        use riff_backend::app::state::PlaybackSession;
        use riff_gui::ui::app::handle_keyboard_shortcuts;

        let palette = riff_gui::ui::theme::Palette::dark();
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, riff_gui::ui::theme::TITLEBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, state| {
                    let mut q = String::new();
                    let _ = show_titlebar(
                        ui,
                        &mut cache,
                        &palette,
                        &content(),
                        &mut q,
                        &mut Vec::new(),
                    );
                    handle_keyboard_shortcuts(
                        ui.ctx(),
                        &PlaybackSession::default(),
                        state,
                        &crate::mocks::MockTransport::new(),
                    );
                },
                false,
            );
        harness
            .ctx
            .set_fonts(riff_gui::ui::fonts::font_definitions());
        harness.run();

        // Ctrl+K targets the titlebar search field.
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::K);
        harness.run();
        assert!(*harness.state());
    }

    #[test]
    fn test_titlebar_search_dismisses_on_escape() {
        use egui_kittest::kittest::Queryable;

        let palette = riff_gui::ui::theme::Palette::dark();
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, riff_gui::ui::theme::TITLEBAR_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, query| {
                    show_titlebar(ui, &mut cache, &palette, &content(), query, &mut Vec::new());
                },
                String::new(),
            );
        harness
            .ctx
            .set_fonts(riff_gui::ui::fonts::font_definitions());
        harness.run();

        // Type a query so there is something to dismiss.
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .focus();
        harness.run();
        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .type_text("boards of canada");
        harness.run();

        // Escape dismisses: the query clears and the field gives up focus,
        // all from the keyboard.
        harness.key_press(egui::Key::Escape);
        harness.run();

        assert_eq!(harness.state(), "", "Escape must clear the query");
        assert!(
            !harness
                .get_by_role(egui::accesskit::Role::TextInput)
                .is_focused(),
            "Escape must give up the field's keyboard focus"
        );
    }
}

// --- Browser column (design-handoff issue 08) -----------------------------------
//
// The first pane of the three-pane explorer: a generic list column that
// renders every section's rows — artists with cover thumbnails, plus All
// Tracks / Albums / Genres / Folders / smart-list / playlist rows — with an
// A–Z sort control and genre filter chips on the artist variant. The browser
// is permanently list-only: the grid render path was retired end-to-end. The
// headless seam is the `browser` widget module (the same pure-widget
// discipline as `sidebar`/`topbar`): widgets paint from data and report
// `BrowserAction`s; app state stays in `app.rs`.
#[cfg(test)]
mod browser_column_ui_tests {
    use super::*;
    use crate::ui_tests::tests::boxed_library_store;
    use riff_gui::ui::browser::{BrowserAction, BrowserColumn, BrowserItem};
    use riff_gui::ui::icons::IconCache;
    use riff_gui::ui::theme::Palette;

    /// Three fixed rows, the smallest column that proves rendering and
    /// selection reporting.
    fn fixture_items() -> Vec<BrowserItem> {
        vec![
            BrowserItem {
                key: "alpha".to_string(),
                label: "Alpha".to_string(),
                detail: None,
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
            BrowserItem {
                key: "beta".to_string(),
                label: "Beta".to_string(),
                detail: None,
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
            BrowserItem {
                key: "gamma".to_string(),
                label: "Gamma".to_string(),
                detail: None,
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
        ]
    }

    /// A provider closure over a fixture slice, bound to a mutable local so
    /// it coerces to `&mut dyn FnMut` (the production providers page through
    /// mutable Session Views caches).
    fn provider(items: &[BrowserItem]) -> impl FnMut(usize) -> Option<BrowserItem> + '_ {
        move |i| items.get(i).cloned()
    }

    #[test]
    fn test_browser_column_renders_its_rows_and_reports_row_selection() {
        use egui_kittest::kittest::Queryable;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    // ACCUMULATE across frames: a click fires its action on
                    // exactly one frame; harness.run() settles afterwards.
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: true,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        for label in ["Alpha", "Beta", "Gamma"] {
            assert!(
                harness.query_by_label(label).is_some(),
                "row '{label}' must render in the browser column"
            );
        }

        harness.get_by_label("Beta").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&BrowserAction::Select("beta".to_string())),
            "clicking a row reports its selection by key: {:?}",
            harness.state()
        );
    }

    #[test]
    fn test_browser_rows_expose_their_detail_line_in_the_accessible_label() {
        use egui_kittest::kittest::Queryable;

        // Handoff issue 16: counts and other secondary lines on browser rows
        // are painted muted text under the label — invisible to a
        // screen reader unless the row's accessibility label folds them in,
        // the same "Name (count)" shape the sidebar rows speak.
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = vec![
            BrowserItem {
                key: "jazz".to_string(),
                label: "Jazz".to_string(),
                detail: Some("12 tracks".to_string()),
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
            BrowserItem {
                key: "alpha".to_string(),
                label: "Alpha".to_string(),
                detail: None,
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 200.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label("Jazz (12 tracks)").is_some(),
            "a counted row exposes its count in the accessibility label, \
             not just as painted text"
        );
        assert!(
            harness.query_by_label("Alpha").is_some(),
            "a row with no detail line keeps its bare label"
        );
    }

    #[test]
    fn test_overlong_row_label_wraps_and_grows_the_row() {
        use egui_kittest::kittest::Queryable;

        // A label too long for the column's text width wraps onto the next
        // line and the row grows past the classic 48px slot instead of
        // painting past the pane edge; a single-line row keeps the classic
        // slot exactly.
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = vec![
            BrowserItem {
                key: "long".to_string(),
                label: "A genre name far too long to fit the column's text width".to_string(),
                detail: Some("12 tracks".to_string()),
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
            BrowserItem {
                key: "short".to_string(),
                label: "Jazz".to_string(),
                detail: Some("5 tracks".to_string()),
                thumbnail: None,
                selected: false,
                now_playing: false,
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(220.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        let long = harness
            .query_by_label("A genre name far too long to fit the column's text width (12 tracks)")
            .unwrap_or_else(|| panic!("the long row renders"));
        assert!(
            long.rect().height() > riff_gui::ui::theme::geometry::browser::ROW_H,
            "a wrapped label grows its row past the classic slot: {:?}",
            long.rect()
        );
        let short = harness.get_by_label("Jazz (5 tracks)");
        assert_eq!(
            short.rect().height(),
            riff_gui::ui::theme::geometry::browser::ROW_H,
            "a single-line row keeps the classic 48px slot"
        );
    }

    #[test]
    fn test_browser_tiles_expose_their_detail_line_in_the_accessible_label() {
        use egui_kittest::kittest::Queryable;

        // Grid tiles paint their label alone; the detail line (e.g. the
        // track count) still reaches the accessibility tree.
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = vec![BrowserItem {
            key: "jazz".to_string(),
            label: "Jazz".to_string(),
            detail: Some("12 tracks".to_string()),
            thumbnail: None,
            selected: false,
            now_playing: false,
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 240.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label("Jazz (12 tracks)").is_some(),
            "a tile's accessibility label folds in its detail line"
        );
    }

    #[test]
    fn test_browser_column_renders_a_friendly_empty_state() {
        use egui_kittest::kittest::Queryable;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: true,
                        total: 0,
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "No tracks yet",
                        empty_hint: "Add a folder to start scanning your library.",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label("No tracks yet").is_some(),
            "an empty section shows its friendly title, not a raw error"
        );
        assert!(
            harness
                .query_by_label("Add a folder to start scanning your library.")
                .is_some(),
            "the empty state carries the hint that moves the listener forward"
        );
    }

    #[test]
    fn test_browser_column_sort_control_reports_toggle_and_reflects_direction() {
        use egui_kittest::kittest::Queryable;

        let run_column = |sort_desc: bool| {
            let palette = Palette::dark();
            let mut cache = IconCache::new();
            let items = fixture_items();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(320.0, 300.0))
                .with_pixels_per_point(1.0)
                .build_ui_state(
                    move |ui, actions: &mut Vec<BrowserAction>| {
                        let mut fixture_item = provider(&items);
                        let column = BrowserColumn {
                            sort_desc,
                            show_sort: true,
                            total: items.len(),
                            item: &mut fixture_item,
                            virtualize: false,
                            empty_title: "",
                            empty_hint: "",
                        };
                        riff_gui::ui::browser::show_browser_column(
                            ui, &mut cache, &palette, column, actions,
                        );
                    },
                    Vec::new(),
                );
            harness.run();
            harness
        };

        // A–Z (ascending): the control offers the Z–A flip; clicking it
        // reports ToggleSort for the session to apply.
        let mut harness = run_column(false);
        assert!(
            harness.query_by_label("Sort Z to A").is_some(),
            "ascending state offers the Z–A flip"
        );
        harness.get_by_label("Sort Z to A").click();
        harness.run();
        assert!(
            harness.state().contains(&BrowserAction::ToggleSort),
            "clicking the sort control reports ToggleSort"
        );

        // Z–A (descending): the control offers flipping back to A–Z.
        let harness = run_column(true);
        assert!(
            harness.query_by_label("Sort A to Z").is_some(),
            "descending state offers the A–Z flip"
        );
    }

    #[test]
    fn test_browser_column_hides_the_sort_control_for_unsortable_variants() {
        use egui_kittest::kittest::Queryable;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness.query_by_label("Sort Z to A").is_none(),
            "variants the sort cannot order (paged track listings, folders) \
             render no sort control"
        );
    }

    #[test]
    fn test_virtualized_list_consults_the_provider_only_for_the_on_screen_window() {
        use egui_kittest::kittest::Queryable;

        // The artists-root idle-CPU fix: a virtualized list reserves
        // default-height slots for rows above the viewport without consulting
        // the provider, so the provider's per-row work (paged store reads,
        // cover intents) stays bounded to the on-screen window instead of
        // running once per walked row every frame.
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let total = 200usize;
        let calls: std::rc::Rc<std::cell::RefCell<Vec<usize>>> = Default::default();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let calls = std::rc::Rc::clone(&calls);
                    let mut item = move |i: usize| -> Option<BrowserItem> {
                        calls.borrow_mut().push(i);
                        Some(BrowserItem {
                            key: format!("row-{i}"),
                            label: format!("Row {i}"),
                            detail: None,
                            thumbnail: None,
                            selected: false,
                            now_playing: false,
                        })
                    };
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total,
                        item: &mut item,
                        virtualize: true,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();
        calls.borrow_mut().clear();

        // Scroll the viewport deep into the list, one rendered row at a time
        // (rows cull as they leave the viewport, exactly like the
        // artists-column test), until a row far past the first window shows.
        let mut guard = 0;
        while harness.query_by_label("Row 60").is_none() && guard < 200 {
            let mut scrolled = false;
            for probe in 0..150 {
                let label = format!("Row {probe}");
                if let Some(row) = harness.query_by_label(&label) {
                    row.scroll_down();
                    scrolled = true;
                    break;
                }
            }
            assert!(scrolled, "a rendered row must exist to scroll");
            harness.step();
            guard += 1;
        }
        assert!(
            guard < 200,
            "scrolling must reach the deep row within the step budget"
        );

        // From the settled deep position, one frame's walk consults the
        // provider only for the on-screen window — never the rows above it.
        calls.borrow_mut().clear();
        harness.step();
        let calls = calls.borrow();
        assert!(
            !calls.is_empty(),
            "the on-screen window must render after scrolling"
        );
        assert!(
            calls.iter().all(|&i| i >= 50),
            "rows above the viewport must not consult the provider: {calls:?}"
        );
        assert!(
            calls.len() < 30,
            "provider work stays bounded to the visible window: {} calls",
            calls.len()
        );
    }

    // --- Session glue (app applies widget actions) ------------------------------

    use riff_backend::app::state::BrowserSelection;
    use riff_backend::app::state::LibrarySection;
    use riff_backend::app::state::LibrarySession;
    use riff_gui::ui::app::apply_browser_action;

    /// The unit separator joins an album's `(album artist, title)` composite
    /// identity into one row key — the same identity the store keys albums
    /// by.
    fn album_key(artist: &str, title: &str) -> String {
        format!("{artist}\u{1f}{title}")
    }

    #[test]
    fn test_browser_actions_update_the_library_session() {
        let mut library = LibrarySession::default();

        // Sort starts A–Z; the widget's toggle flips it.
        apply_browser_action(BrowserAction::ToggleSort, &mut library);
        assert!(library.browser_sort_desc, "ToggleSort flips to Z–A");
        apply_browser_action(BrowserAction::ToggleSort, &mut library);
        assert!(!library.browser_sort_desc, "another toggle returns to A–Z");
    }

    #[test]
    fn test_browser_selection_resolves_per_section() {
        // Artists: the row key IS the artist name.
        let mut library = LibrarySession {
            library_section: LibrarySection::Artists,
            ..LibrarySession::default()
        };
        apply_browser_action(
            BrowserAction::Select("Aphex Twin".to_string()),
            &mut library,
        );
        assert_eq!(
            library.current_selection(),
            Some(&BrowserSelection::Artist("Aphex Twin".to_string())),
            "selecting an artist row stores the artist identity for the detail column"
        );

        // Albums: the key is the store's (album artist, title) composite.
        library = LibrarySession {
            library_section: LibrarySection::Albums,
            ..LibrarySession::default()
        };
        apply_browser_action(
            BrowserAction::Select(album_key("Boards of Canada", "Geogaddi")),
            &mut library,
        );
        assert_eq!(
            library.current_selection(),
            Some(&BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }),
        );

        // Genres: the row key IS the genre name.
        library = LibrarySession {
            library_section: LibrarySection::Genres,
            ..LibrarySession::default()
        };
        apply_browser_action(
            BrowserAction::Select("Electronic".to_string()),
            &mut library,
        );
        assert_eq!(
            library.current_selection(),
            Some(&BrowserSelection::Genre("Electronic".to_string())),
        );
    }

    // --- Detail column (handoff issue 09) ---------------------------------------
    //
    // The middle pane of the three-pane explorer. Tested at the same two
    // seams as the browser column (issue 08): the pure widget seam
    // (`ui::detail::show_detail_column`, headless kittest harness) and the
    // session-glue seam (`apply_detail_action`, real store/transport mocks).

    #[test]
    fn test_detail_column_renders_breadcrumb_and_climbs_one_level() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{Crumb, DetailAction, DetailColumn, show_detail_column};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let crumbs = vec![
            Crumb {
                label: "Artists".to_string(),
            },
            Crumb {
                label: "Boards of Canada".to_string(),
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<DetailAction>| {
                    let column = DetailColumn {
                        breadcrumb: &crumbs,
                        ..DetailColumn::empty("No albums yet", "Nothing here.")
                    };
                    show_detail_column(ui, &mut cache, &palette, column, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The trail reads `Artists / Boards of Canada` — every segment of the
        // path the listener took is visible.
        assert!(
            harness.query_by_label("Artists").is_some(),
            "the breadcrumb's root segment renders"
        );
        assert!(
            harness.query_by_label("Boards of Canada").is_some(),
            "the breadcrumb's leaf segment renders"
        );

        // Clicking the root climbs back one level: the widget reports WHICH
        // segment was clicked, `apply_detail_action` does the climbing.
        harness.get_by_label("Artists").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![DetailAction::Crumb(0)],
            "clicking a breadcrumb segment reports its level"
        );
    }

    #[test]
    fn test_album_header_offers_play_all_and_shuffle() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{
            AlbumHeader, Crumb, DetailAction, DetailColumn, show_detail_column,
        };

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let crumbs = vec![
            Crumb {
                label: "Artists".to_string(),
            },
            Crumb {
                label: "Boards of Canada".to_string(),
            },
            Crumb {
                label: "Geogaddi".to_string(),
            },
        ];
        let header = AlbumHeader {
            title: "Geogaddi".to_string(),
            subtitle: Some("Boards of Canada \u{b7} 2002".to_string()),
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<DetailAction>| {
                    let column = DetailColumn {
                        breadcrumb: &crumbs,
                        header: Some(&header),
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column(ui, &mut cache, &palette, column, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The album header names the album and its artist · year line. The
        // title also appears in the breadcrumb's leaf, so both renderings
        // must be there.
        assert!(
            harness.query_all_by_label("Geogaddi").count() >= 2,
            "the album title renders in the breadcrumb and the header"
        );
        assert!(
            harness.query_by_label("Boards of Canada · 2002").is_some(),
            "the album header renders the artist · year subtitle"
        );

        // Play all starts the album from the top; Shuffle starts it shuffled.
        harness.get_by_label("Play all").click();
        harness.run();
        harness.get_by_label("Shuffle").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![DetailAction::PlayAll, DetailAction::Shuffle],
            "the header's two playback actions report themselves"
        );
    }

    #[test]
    fn test_detail_play_all_and_shuffle_play_the_albums_tracks() {
        use riff_gui::ui::app::apply_detail_action;
        use riff_gui::ui::detail::DetailAction;

        let dir = tempfile::tempdir().unwrap();
        let mut store = boxed_library_store(&dir);
        let tracks = [
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ];
        let mut library = LibrarySession::default();
        let mut playback = PlaybackSession::default();
        let transport = crate::mocks::MockTransport::new();

        apply_detail_action(
            DetailAction::PlayAll,
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &tracks,
        );
        assert_eq!(
            transport.recorded(),
            vec![crate::mocks::TransportIntent::PlayMany(
                TrackId("a.mp3".to_string()),
                vec![TrackId("b.mp3".to_string()), TrackId("c.mp3".to_string())]
            )],
            "Play all starts the album's first track with the rest queued behind it"
        );
        assert!(
            !playback.queue.shuffle,
            "Play all never flips the queue's shuffle state"
        );

        let transport = crate::mocks::MockTransport::new();
        apply_detail_action(
            DetailAction::Shuffle,
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &tracks,
        );
        assert_eq!(
            transport.recorded(),
            vec![crate::mocks::TransportIntent::PlayMany(
                TrackId("a.mp3".to_string()),
                vec![TrackId("b.mp3".to_string()), TrackId("c.mp3".to_string())]
            )],
            "Shuffle plays the same album batch — the queue's traversal shuffles"
        );
        assert!(
            playback.queue.shuffle,
            "Shuffle turns the queue's shuffle traversal on"
        );

        // An empty album has nothing to start: no intents, no flag changes.
        let transport = crate::mocks::MockTransport::new();
        playback.queue.set_shuffle(false);
        apply_detail_action(
            DetailAction::PlayAll,
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &[],
        );
        apply_detail_action(
            DetailAction::Shuffle,
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &[],
        );
        assert!(
            transport.recorded().is_empty(),
            "an empty album starts nothing"
        );
        assert!(
            !playback.queue.shuffle,
            "an empty album never re-enables shuffle"
        );
    }

    #[test]
    fn test_track_list_renders_rows_and_reports_row_gestures() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{
            Crumb, DetailAction, DetailColumn, TrackRow, show_detail_column,
        };
        use std::time::Duration;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let crumbs = vec![Crumb {
            label: "Geogaddi".to_string(),
        }];
        let tracks = vec![
            TrackRow {
                key: "t1".to_string(),
                title: "Magic Window".to_string(),
                plays: 3,
                duration: Some(Duration::from_secs(205)),
                favorite: false,
                selected: false,
                now_playing: false,
            },
            TrackRow {
                key: "t2".to_string(),
                title: "Over the Horizon".to_string(),
                plays: 0,
                duration: None,
                favorite: false,
                selected: false,
                now_playing: false,
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 300.0))
            .with_pixels_per_point(1.0)
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(
                |ui, actions: &mut Vec<DetailAction>| {
                    let column = DetailColumn {
                        breadcrumb: &crumbs,
                        tracks: &tracks,
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column(ui, &mut cache, &palette, column, actions);
                },
                Vec::new(),
            );
        harness.run();

        // Every track's title renders as its row's label — the shared 40px
        // track row. The right-aligned album · plays · time cluster is
        // painted text, so only the row labels are (kittest-)queryable here.
        assert!(
            harness.query_by_label("Magic Window").is_some()
                && harness.query_by_label("Over the Horizon").is_some(),
            "every track's title renders as its row's label"
        );

        // Single click selects the row; double click starts it playing.
        harness.get_by_label("Magic Window").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&DetailAction::SelectTrack("t1".to_string())),
            "clicking a row selects its track"
        );
        // Kittest has no double-click primitive and its default 0.25s step
        // dt spreads one click's press/release across ~0.75s — past egui's
        // 0.3s double-click window. A 60fps step rate keeps two successive
        // clicks inside it.
        let row = harness.get_by_label("Magic Window");
        row.click();
        row.click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&DetailAction::PlayTrack("t1".to_string())),
            "double-clicking a row starts the track"
        );
    }

    #[test]
    fn test_track_table_favorite_control_reports_the_toggle() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{DetailAction, DetailColumn, TrackRow, show_detail_column};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tracks = vec![
            TrackRow {
                key: "plain".to_string(),
                title: "Not A Favorite".to_string(),
                plays: 0,
                duration: None,
                favorite: false,
                selected: false,
                now_playing: false,
            },
            TrackRow {
                key: "loved".to_string(),
                title: "Loved Song".to_string(),
                plays: 7,
                duration: None,
                favorite: true,
                selected: false,
                now_playing: false,
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<DetailAction>| {
                    let column = DetailColumn {
                        tracks: &tracks,
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column(ui, &mut cache, &palette, column, actions);
                },
                Vec::new(),
            );
        harness.run();

        // Every row carries its own favorite control, labelled by what the
        // click will do.
        assert!(
            harness.query_by_label("Add to Favorites").is_some(),
            "an unfavorite track offers to be added"
        );
        assert!(
            harness.query_by_label("Remove from Favorites").is_some(),
            "a favorite track offers to be removed"
        );

        // Clicking reports the flag's NEW value per row — the caller commits
        // exactly that.
        harness.get_by_label("Add to Favorites").click();
        harness.get_by_label("Remove from Favorites").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![
                DetailAction::SetFavorite {
                    key: "plain".to_string(),
                    favorite: true
                },
                DetailAction::SetFavorite {
                    key: "loved".to_string(),
                    favorite: false
                },
            ],
            "each row's control reports its own track's new flag value"
        );
    }

    #[test]
    fn test_detail_favorite_toggle_commits_durably_and_favorites_list_reflects_it() {
        use riff_backend::app::state::LibrarySession;
        use riff_backend::app::store::{LibraryMutationStore, LibraryQueryStore};
        use riff_backend::domain::SmartPlaylistKind;
        use riff_gui::ui::app::apply_detail_action;
        use riff_gui::ui::detail::DetailAction;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");

        // One stored track, unfavorite. The views seam pairs with the store
        // exactly the way the composition root wires it.
        let track = crate::test_utils::create_test_track("a.mp3", "a.mp3");
        store
            .apply_scan_batch(std::slice::from_ref(&track))
            .unwrap();
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );
        let mut library_mutations: Box<dyn LibraryMutationStore> = Box::new(store.clone());
        let mut library = LibrarySession::default();
        let mut playback = PlaybackSession::default();
        let transport = crate::mocks::MockTransport::new();

        // The row's control reports the flag's NEW value; applying it
        // commits through the store's favorite setter.
        apply_detail_action(
            DetailAction::SetFavorite {
                key: track.id.0.clone(),
                favorite: true,
            },
            &mut library,
            &mut playback,
            &transport,
            library_mutations.as_mut(),
            &[],
        );

        // The Favorites smart list reflects the change with zero caller
        // action — the committed mutation bumped the library generation
        // itself.
        let favorites = views.smart_list(SmartPlaylistKind::Favorites, 100);
        assert_eq!(
            favorites.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
            vec![track.id.clone()],
            "the Favorites smart list picks up the toggle"
        );

        // Durability: the flag survives closing and reopening the store.
        drop(library_mutations);
        drop(views);
        drop(store);
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening the store must work");
        let restored = reopened
            .get_track(&track.id)
            .expect("the query works")
            .expect("the track is still known");
        assert!(
            restored.favorite,
            "the favorite flag persisted through a restart"
        );
    }

    #[test]
    fn test_artist_detail_lists_albums_that_drill_deeper() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::browser::BrowserItem;
        use riff_gui::ui::detail::{Crumb, DetailAction, DetailColumn, show_detail_column};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let crumbs = vec![
            Crumb {
                label: "Artists".to_string(),
            },
            Crumb {
                label: "Boards of Canada".to_string(),
            },
        ];
        let albums = vec![BrowserItem {
            key: album_key("Boards of Canada", "Geogaddi"),
            label: "Geogaddi".to_string(),
            detail: Some("2002".to_string()),
            thumbnail: None,
            selected: false,
            now_playing: false,
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<DetailAction>| {
                    let column = DetailColumn {
                        breadcrumb: &crumbs,
                        rows: &albums,
                        ..DetailColumn::empty("No albums yet", "Nothing here.")
                    };
                    show_detail_column(ui, &mut cache, &palette, column, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The artist level lists the artist's albums (the browser column's
        // row shape, drilled one level down). The row's detail line ("2002")
        // folds into the accessibility label (issue 16).
        assert!(
            harness.query_by_label("Geogaddi (2002)").is_some(),
            "the artist's album rows render in the detail column"
        );
        harness.get_by_label("Geogaddi (2002)").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![DetailAction::SelectRow(album_key(
                "Boards of Canada",
                "Geogaddi"
            ))],
            "clicking an album row reports its key; the app resolves the level"
        );
    }

    #[test]
    fn test_detail_crumb_truncates_the_path() {
        use riff_backend::app::state::BrowserSelection;
        use riff_gui::ui::app::{apply_detail_action, apply_drill_action};
        use riff_gui::ui::detail::DetailAction;

        let dir = tempfile::tempdir().unwrap();
        let mut store = boxed_library_store(&dir);
        let mut library = LibrarySession::default();
        let mut playback = PlaybackSession::default();
        let transport = crate::mocks::MockTransport::new();

        // Album trail: Artists / Boards of Canada / Geogaddi.
        library.library_section = LibrarySection::Artists;
        apply_browser_action(
            BrowserAction::Select("Boards of Canada".to_string()),
            &mut library,
        );
        apply_drill_action(
            LibrarySection::Artists,
            1,
            album_key("Boards of Canada", "Geogaddi"),
            &mut library,
        );
        assert_eq!(
            library.browser_path,
            vec![
                BrowserSelection::Artist("Boards of Canada".to_string()),
                BrowserSelection::Album {
                    artist: "Boards of Canada".to_string(),
                    title: "Geogaddi".to_string(),
                },
            ],
            "drilling selects the album at level 1 under the root artist"
        );

        // A crumb click at level 1 climbs back to the artist; the browser
        // column highlights the artist row again.
        apply_detail_action(
            DetailAction::Crumb(1),
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &[],
        );
        assert_eq!(
            library.browser_path,
            vec![BrowserSelection::Artist("Boards of Canada".to_string())],
            "one climb up the album trail truncates the path to the artist"
        );

        // A crumb click at the root (level 0) empties the path: the browser
        // column listing takes over again.
        apply_detail_action(
            DetailAction::Crumb(0),
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &[],
        );
        assert!(
            library.browser_path.is_empty(),
            "climbing to the root clears the drill-down path"
        );

        // A crumb level at or past the current depth changes nothing.
        library.browser_path = vec![BrowserSelection::Artist("Autechre".to_string())];
        apply_detail_action(
            DetailAction::Crumb(1),
            &mut library,
            &mut playback,
            &transport,
            store.as_mut(),
            &[],
        );
        assert_eq!(
            library.browser_path,
            vec![BrowserSelection::Artist("Autechre".to_string())],
            "a crumb level at or past the current depth is inert"
        );
    }

    #[test]
    fn test_apply_drill_action_selects_at_the_level_and_truncates_deeper_entries() {
        use riff_backend::app::state::BrowserSelection;
        use riff_gui::ui::app::apply_drill_action;

        // Artists, level 1: the row key is the (album artist, title)
        // composite; the new entry lands under the root artist.
        let mut library = LibrarySession {
            library_section: LibrarySection::Artists,
            browser_path: vec![BrowserSelection::Artist("Boards of Canada".to_string())],
            ..LibrarySession::default()
        };
        apply_drill_action(
            LibrarySection::Artists,
            1,
            album_key("Boards of Canada", "Geogaddi"),
            &mut library,
        );
        assert_eq!(
            library.browser_path,
            vec![
                BrowserSelection::Artist("Boards of Canada".to_string()),
                BrowserSelection::Album {
                    artist: "Boards of Canada".to_string(),
                    title: "Geogaddi".to_string(),
                },
            ],
            "an album row on the artist level selects that album at level 1"
        );

        // Genres, level 1: the row key is the artist name.
        let mut library = LibrarySession {
            library_section: LibrarySection::Genres,
            browser_path: vec![BrowserSelection::Genre("Electronic".to_string())],
            ..LibrarySession::default()
        };
        apply_drill_action(
            LibrarySection::Genres,
            1,
            "Autechre".to_string(),
            &mut library,
        );
        assert_eq!(
            library.browser_path,
            vec![
                BrowserSelection::Genre("Electronic".to_string()),
                BrowserSelection::Artist("Autechre".to_string()),
            ],
            "an artist row on the genre level selects that artist at level 1"
        );

        // Genres, level 2: the row key is the album composite, and the
        // deeper path (a stale album entry) truncates.
        library.browser_path.push(BrowserSelection::Album {
            artist: "Autechre".to_string(),
            title: "Old".to_string(),
        });
        apply_drill_action(
            LibrarySection::Genres,
            2,
            album_key("Autechre", "Tri Repetae"),
            &mut library,
        );
        assert_eq!(
            library.browser_path,
            vec![
                BrowserSelection::Genre("Electronic".to_string()),
                BrowserSelection::Artist("Autechre".to_string()),
                BrowserSelection::Album {
                    artist: "Autechre".to_string(),
                    title: "Tri Repetae".to_string(),
                },
            ],
            "an album row at level 2 truncates any deeper entries"
        );

        // A level the section has no identity for applies nothing.
        let mut library = LibrarySession::default();
        apply_drill_action(LibrarySection::AllTracks, 1, "x".to_string(), &mut library);
        assert!(library.browser_path.is_empty(), "All Tracks drills nowhere");
    }

    #[test]
    fn test_column_plan_shapes_per_section_and_path() {
        use riff_backend::app::state::BrowserSelection;
        use riff_gui::ui::app::{ColumnKind, column_plan};

        let artist = BrowserSelection::Artist("Boards of Canada".to_string());
        let album = BrowserSelection::Album {
            artist: "Boards of Canada".to_string(),
            title: "Geogaddi".to_string(),
        };
        let genre = BrowserSelection::Genre("Electronic".to_string());

        // Artists: root only, then the albums-of-artist column, then Tracks.
        assert_eq!(
            column_plan(LibrarySection::Artists, &[]),
            vec![ColumnKind::Root],
            "Artists with an empty path renders its root column only"
        );
        assert_eq!(
            column_plan(LibrarySection::Artists, std::slice::from_ref(&artist)),
            vec![ColumnKind::Root, ColumnKind::ArtistAlbums],
        );
        assert_eq!(
            column_plan(LibrarySection::Artists, &[artist.clone(), album.clone()]),
            vec![
                ColumnKind::Root,
                ColumnKind::ArtistAlbums,
                ColumnKind::Tracks,
            ],
        );

        // Albums: root only, then the Tracks column.
        assert_eq!(
            column_plan(LibrarySection::Albums, &[]),
            vec![ColumnKind::Root],
            "Albums with an empty path renders its root column only"
        );
        assert_eq!(
            column_plan(LibrarySection::Albums, std::slice::from_ref(&album)),
            vec![ColumnKind::Root, ColumnKind::Tracks],
        );

        // Genres: one column per drill level, capped at the four-column
        // genre → artist → album → tracks shape.
        assert_eq!(
            column_plan(LibrarySection::Genres, &[]),
            vec![ColumnKind::Root],
        );
        assert_eq!(
            column_plan(LibrarySection::Genres, std::slice::from_ref(&genre)),
            vec![ColumnKind::Root, ColumnKind::GenreArtists],
        );
        assert_eq!(
            column_plan(LibrarySection::Genres, &[genre.clone(), artist.clone()]),
            vec![
                ColumnKind::Root,
                ColumnKind::GenreArtists,
                ColumnKind::GenreArtistAlbums,
            ],
        );
        assert_eq!(
            column_plan(
                LibrarySection::Genres,
                &[genre.clone(), artist.clone(), album.clone()],
            ),
            vec![
                ColumnKind::Root,
                ColumnKind::GenreArtists,
                ColumnKind::GenreArtistAlbums,
                ColumnKind::Tracks,
            ],
        );

        // All Tracks is the single flat listing column.
        assert_eq!(
            column_plan(LibrarySection::AllTracks, &[]),
            vec![ColumnKind::Flat],
        );

        // A path entry of the wrong kind for its level is ignored: it never
        // spawns a column.
        assert_eq!(
            column_plan(
                LibrarySection::Artists,
                &[BrowserSelection::Album {
                    artist: "Boards of Canada".to_string(),
                    title: "Geogaddi".to_string(),
                }]
            ),
            vec![ColumnKind::Root],
            "an album entry on the Artists root is ignored"
        );
        assert_eq!(
            column_plan(
                LibrarySection::Genres,
                &[BrowserSelection::Artist("Autechre".to_string())]
            ),
            vec![ColumnKind::Root],
            "an artist entry on the Genres root is ignored"
        );
        assert_eq!(
            column_plan(LibrarySection::Artists, &[artist.clone(), genre.clone()],),
            vec![ColumnKind::Root, ColumnKind::ArtistAlbums],
            "a wrong-kind level-1 entry still leaves the artist's albums column"
        );
    }

    #[test]
    fn test_column_widths_fit_the_available_width_without_scrolling() {
        use riff_gui::ui::app::column_widths;

        // Wide window: entity columns keep their preferred width, the last
        // column absorbs the remainder.
        assert_eq!(column_widths(1000.0, 2, false), vec![280.0, 720.0]);
        // The inspector takes its width off the top first.
        assert_eq!(
            column_widths(1300.0, 3, true),
            vec![280.0, 280.0, 440.0],
            "the inspector's 300px is reserved before the last column absorbs"
        );
        // The last column never falls below its floor: once the window can
        // pay for the floors, the non-last columns yield width toward the
        // last column's floor first (280·3 leaves only 160 for the last).
        let widths = column_widths(1000.0, 4, false);
        assert_eq!(widths.iter().sum::<f32>(), 1000.0);
        assert!(
            widths[3] >= 320.0 && widths[0] >= 200.0,
            "the last column's floor holds before the non-last columns shrink: {widths:?}"
        );

        // A single column fills the whole stage (floors do not apply).
        assert_eq!(column_widths(500.0, 1, false), vec![500.0]);
        assert_eq!(column_widths(500.0, 1, true), vec![200.0]);

        // Narrow window: when the width cannot satisfy the floors, every
        // column shrinks proportionally to its floor — never scrolls, and
        // never drops a column below a usable sliver.
        let widths = column_widths(1000.0, 4, true);
        assert_eq!(
            widths.iter().sum::<f32>(),
            700.0,
            "the four list columns plus the 300px inspector exactly fill the stage"
        );
        let expected_scale = 700.0 / (200.0 * 3.0 + 320.0);
        assert!(
            (widths[0] - 200.0 * expected_scale).abs() < f32::EPSILON
                && (widths[3] - 320.0 * expected_scale).abs() < f32::EPSILON,
            "columns shrink proportionally to their floors: {widths:?}"
        );
        assert!(
            widths.iter().all(|w| *w > 0.0),
            "every visible column stays usable: {widths:?}"
        );

        // Medium window: floors are satisfiable, and the non-last columns
        // yield toward the last column's floor before the last column
        // absorbs the remainder.
        let widths = column_widths(800.0, 3, false);
        assert_eq!(widths.iter().sum::<f32>(), 800.0);
        assert!(
            widths[2] >= 320.0 && widths[0] >= 200.0 && widths[1] >= 200.0,
            "all floors hold once the window can pay for them: {widths:?}"
        );
    }

    /// Drive the shared elastic-stage geometry seam and record the rect each
    /// slot is handed. `slots[i]` is `(column index, allocated child rect)`,
    /// where the inspector reports as index `columns`.
    fn record_stage_slots(
        available: f32,
        columns: usize,
        inspector: bool,
    ) -> Vec<(usize, egui::Rect)> {
        use riff_gui::ui::stage::{StageSlot, show_elastic_stage};
        use std::cell::RefCell;
        use std::rc::Rc;

        let slots: Rc<RefCell<Vec<(usize, egui::Rect)>>> = Rc::new(RefCell::new(Vec::new()));
        let capture = Rc::clone(&slots);
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(available, 600.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                capture.borrow_mut().clear();
                let capture = Rc::clone(&capture);
                show_elastic_stage(ui, columns, inspector, move |ui, slot| {
                    let index = match slot {
                        StageSlot::Column(i) => i,
                        StageSlot::Inspector => columns,
                    };
                    capture.borrow_mut().push((index, ui.max_rect()));
                });
            });
        // Release the harness (and the closure it owns, which holds one `Rc`
        // clone) before unwrapping the recorded slots.
        drop(harness);
        Rc::try_unwrap(slots)
            .expect("harness dropped its capture")
            .into_inner()
    }

    /// The elastic-stage geometry seam hands every slot a positive, correctly
    /// sized child ui: one slot per column plus a distinct fixed-width
    /// inspector, with the list columns together never asking for more than
    /// the stage offers. Production and the golden harness drive this same
    /// owner, so the two cannot disagree on a column's width. (The exact
    /// left-to-right edges are pinned by the `elastic_*` goldens.)
    #[test]
    fn test_elastic_stage_geometry_fits_with_stable_child_identities() {
        let inspector_w = riff_gui::ui::theme::INSPECTOR_WIDTH;

        for available in [1000.0_f32, 520.0] {
            for (columns, inspector) in [(1usize, false), (3, false), (2, true), (4, true)] {
                let mut slots = record_stage_slots(available, columns, inspector);
                let want = columns + usize::from(inspector);
                assert_eq!(
                    slots.len(),
                    want,
                    "stage @ {available}px, {columns} cols + inspector={inspector}"
                );

                // Every index 0..want appears exactly once — each slot keeps
                // one stable, positionally ordered identity.
                slots.sort_by_key(|(i, _)| *i);
                let ids: Vec<usize> = slots.iter().map(|(i, _)| *i).collect();
                assert_eq!(
                    ids,
                    (0..want).collect::<Vec<_>>(),
                    "each slot keeps one stable, positionally ordered identity"
                );

                // Every slot is handed a positive width (no collapsed column).
                for (i, rect) in &slots {
                    assert!(
                        rect.width() > 0.0,
                        "slot {i} must receive a usable width: {rect:?}"
                    );
                }

                // The list columns together never exceed the stage width, so
                // the composition cannot overflow horizontally.
                let list_sum: f32 = slots
                    .iter()
                    .filter(|(i, _)| *i < columns)
                    .map(|(_, r)| r.width())
                    .sum();
                assert!(
                    list_sum <= available + 0.5,
                    "the {columns} list columns total {list_sum} which must fit \
                     the {available}px stage (no overflow)"
                );

                // The inspector, when present, is the rightmost identity and
                // keeps its fixed token width.
                if inspector {
                    let last = slots.last().expect("at least one slot");
                    assert_eq!(last.0, columns, "the inspector is the last slot");
                    assert!(
                        (last.1.width() - inspector_w).abs() < 0.5,
                        "the inspector column keeps its token width: {} vs {inspector_w}",
                        last.1.width()
                    );
                }
            }
        }
    }

    // --- Settings page frame geometry (ticket 02) -------------------------------
    //
    // The full-stage Settings page is the one composition whose *intent* cannot
    // be read off a golden: "fills the stage", "nav is exactly NAV_W" and
    // "footer pinned to the bottom" are all claims about rects. They are
    // asserted here through the same seam the view draws from —
    // `settings::settings_page_rects` — so the test and the view cannot
    // disagree about the frame, exactly as `record_stage_slots` guarantees for
    // the elastic stage's columns.

    /// The full-stage page frame fills the stage: it is inset by `PAGE_PAD` on
    /// every side and nowhere else, and it is never capped to a card width.
    ///
    /// The centred card this replaced shrank to `MODAL_MAX_W` (760) on any wide
    /// window and floated inside a `MODAL_PAD` (32) backdrop, so a 1600px stage
    /// left 420px of empty margin on each side. That is the regression this
    /// guards: the page must grow with the stage.
    #[test]
    fn test_settings_page_fills_the_stage_with_no_backdrop_margin() {
        use riff_gui::ui::settings::settings_page_rects;
        use riff_gui::ui::theme::geometry::settings as geo;

        for stage in [1600.0_f32, 1280.0, 980.0, 760.0] {
            let available =
                egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(stage, 840.0));
            let r = settings_page_rects(available);

            // Inset by PAGE_PAD on every side — and by PAGE_PAD *only*.
            assert_eq!(
                (
                    r.page.left() - available.left(),
                    available.right() - r.page.right()
                ),
                (geo::PAGE_PAD, geo::PAGE_PAD),
                "the page is inset by PAGE_PAD left and right at a {stage}px stage"
            );
            assert_eq!(
                (
                    r.page.top() - available.top(),
                    available.bottom() - r.page.bottom()
                ),
                (geo::PAGE_PAD, geo::PAGE_PAD),
                "the page is inset by PAGE_PAD top and bottom at a {stage}px stage"
            );

            // No width cap: the page takes everything the stage offers minus
            // the inset, so a wide window is not letterboxed.
            assert_eq!(
                r.page.width(),
                stage - 2.0 * geo::PAGE_PAD,
                "the page fills the {stage}px stage rather than capping at a card width"
            );
            assert!(
                r.page.width() > 760.0 || stage < 812.0,
                "a {stage}px stage must not produce a 760px floating card"
            );
        }
    }

    /// The nav column keeps its token width, and the header spans the whole
    /// page while the footer is pinned to the page's bottom edge.
    #[test]
    fn test_settings_page_frame_pins_header_nav_and_footer() {
        use riff_gui::ui::settings::settings_page_rects;
        use riff_gui::ui::theme::geometry::settings as geo;

        let available = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 840.0));
        let r = settings_page_rects(available);

        // Header: full page width, sitting at the page's top.
        assert_eq!(
            r.header.width(),
            r.page.width(),
            "the header spans the page"
        );
        assert_eq!(
            r.header.left(),
            r.page.left(),
            "the header starts at the page edge"
        );
        assert_eq!(
            r.header.right(),
            r.page.right(),
            "the header ends at the page edge"
        );
        assert_eq!(
            r.header.top(),
            r.page.top(),
            "the header sits at the page top"
        );

        // Nav: exactly NAV_W, full body height, hard against the page's left.
        assert_eq!(r.nav.width(), geo::NAV_W, "the nav column is NAV_W wide");
        assert_eq!(
            r.nav.left(),
            r.page.left(),
            "the nav is flush with the page's left edge"
        );
        assert_eq!(
            r.nav.height(),
            r.body.height(),
            "the nav spans the body's full height"
        );

        // Pane: right of the nav by the hairline plus NAV_GAP.
        assert_eq!(
            r.pane.top(),
            r.body.top(),
            "the pane starts at the body's top"
        );
        assert_eq!(
            r.pane.bottom(),
            r.body.bottom(),
            "the pane reaches the body's bottom"
        );
        assert!(
            r.pane.left() >= r.nav.right() + geo::NAV_GAP,
            "the pane clears the nav by NAV_GAP: pane.left {} vs nav.right {} + {}",
            r.pane.left(),
            r.nav.right(),
            geo::NAV_GAP
        );

        // Footer: full page width, pinned to the page's BOTTOM edge. This is
        // the assertion that distinguishes "pinned" from "floating mid-pane".
        assert_eq!(
            r.footer.width(),
            r.page.width(),
            "the footer spans the page"
        );
        assert_eq!(
            r.footer.bottom(),
            r.page.bottom(),
            "the footer is pinned to the page's bottom"
        );
        assert_eq!(
            r.footer.left(),
            r.page.left(),
            "the footer starts at the page edge"
        );
        assert_eq!(
            r.footer.right(),
            r.page.right(),
            "the footer ends at the page edge"
        );
        assert_eq!(
            r.footer.height(),
            geo::FOOTER_H,
            "the footer is FOOTER_H tall"
        );

        // The body is what is left over, and it is positive: the footer is
        // pinned by the body taking the fill, not by an empty gap.
        assert_eq!(
            r.body.top(),
            r.header.bottom(),
            "the body begins under the header"
        );
        assert_eq!(
            r.body.bottom(),
            r.footer.top(),
            "the body ends above the footer"
        );
        assert!(
            r.body.height() > 0.0,
            "the body must keep a positive height at 840px tall: {:?}",
            r.body
        );
    }

    /// Focus order in the settings page falls out of widget-creation order, not
    /// an explicit focus call, so reordering the nav and pane scopes would
    /// silently retarget Tab. The ticket notes that "no test will catch a
    /// regression here"; this one does.
    ///
    /// egui 0.35 exposes no focus-*order* enumeration — `Memory::focus_order`
    /// does not exist, and `Memory::move_focus` is never consumed by the
    /// kittest harness (probed: three plain buttons stay at `None`). What does
    /// work is the real keyboard path, `Harness::key_press(Key::Tab)`, so the
    /// test walks the same chain a user walks.
    ///
    /// Nav item ids are reconstructible — `nav_item` registers
    /// `Id::new(("settings_nav", label))` — so the walk is classified without
    /// kittest having to hand ids back.
    #[test]
    fn test_settings_focus_order_visits_every_nav_item_before_the_pane() {
        use riff_gui::ui::settings::{
            LibraryRow, SettingsContent, SettingsSection, show_settings_modal,
        };
        use riff_gui::ui::theme::Palette;
        use std::path::PathBuf;

        let content = SettingsContent {
            libraries: vec![LibraryRow {
                path: PathBuf::from("C:\\Users\\stink\\Music"),
                status: riff_backend::app::state::LibraryStatus::Scanned(1284),
                watch: riff_backend::app::state::WatchState::Enabled,
                indexed_tracks: 1284,
            }],
            ..SettingsContent::default()
        };
        let palette = Palette::dark();

        let mut harness: egui_kittest::Harness<'_, ()> = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 840.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, ()| {
                    let mut cache = riff_gui::ui::icons::IconCache::new();
                    show_settings_modal(
                        ui,
                        &mut cache,
                        &palette,
                        &content,
                        SettingsSection::Library,
                    );
                },
                (),
            );
        harness.run();

        // Walk far enough to clear the nav and enter the pane. The page holds
        // the header control, five nav items, the Library pane's controls and
        // the footer's two actions, so 24 Tabs comfortably covers one lap.
        let mut walk: Vec<egui::Id> = Vec::new();
        for _ in 0..24 {
            harness.key_press(egui::Key::Tab);
            harness.run();
            if let Some(id) = harness.ctx.memory(|m| m.focused()) {
                walk.push(id);
            }
        }
        let readable = |ids: &[egui::Id]| -> Vec<String> {
            ids.iter().map(|id| id.short_debug_format()).collect()
        };
        assert!(
            walk.len() > SettingsSection::ALL.len(),
            "the Tab walk must reach the nav and the pane; walked {:?}",
            readable(&walk)
        );

        // The nav scope is built as one contiguous run, in `SettingsSection::ALL`
        // order. Anything interleaved into it would mean a pane widget was
        // constructed between two nav items.
        let nav_ids: Vec<egui::Id> = SettingsSection::ALL
            .iter()
            .map(|section| egui::Id::new(("settings_nav", section.label())))
            .collect();
        let first_nav = nav_ids
            .iter()
            .find_map(|id| walk.iter().position(|seen| seen == id))
            .unwrap_or_else(|| {
                panic!("no nav item is Tab-reachable; walked {:?}", readable(&walk))
            });
        for (offset, id) in nav_ids.iter().enumerate() {
            assert_eq!(
                walk.get(first_nav + offset),
                Some(id),
                "the nav must be one contiguous run in SettingsSection::ALL order; \
                 position {offset} from the first nav item is {:?}, not {:?}. walked {:?}",
                walk.get(first_nav + offset).map(|i| i.short_debug_format()),
                id.short_debug_format(),
                readable(&walk)
            );
        }

        // The widget focused immediately BEFORE the nav run is the header's Back
        // control, which `show_settings_modal` builds first. That is what pins
        // the ordering claim: build the pane scope before the nav and the id in
        // front of the run becomes a pane widget, failing here.
        let back = egui::Id::new("settings_back");
        assert_eq!(
            first_nav.checked_sub(1).and_then(|i| walk.get(i)),
            Some(&back),
            "the nav must directly follow the header's Back control, so it is built \
             before the pane scope; walked {:?}",
            readable(&walk)
        );

        // And the pane is reachable after the nav.
        let after_nav = walk.len() - (first_nav + nav_ids.len());
        assert!(
            after_nav > 0,
            "widgets after the nav run must be reachable (the pane is not built first); \
             walked {:?}",
            readable(&walk)
        );
    }

    /// Paint one neutral row band in isolation and return the rendered frame,
    /// so a test can count which design token actually filled the row.
    fn render_row_band(selected: bool, hovered: bool, focused: bool) -> image::RgbaImage {
        use riff_gui::ui::row::paint_row_band;
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                let rect =
                    egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(200.0, 40.0));
                let painter = ui.painter();
                paint_row_band(painter, &palette, rect, selected, hovered, focused);
            });
        harness.run();
        harness
            .render()
            .expect("the row band must render headlessly")
    }

    /// Count frame pixels within a hair of `color` (rounded corners antialias,
    /// so the interior is what matters).
    fn count_band_color(image: &image::RgbaImage, color: egui::Color32) -> usize {
        image
            .pixels()
            .filter(|p| {
                p.0[0].abs_diff(color.r()) <= 2
                    && p.0[1].abs_diff(color.g()) <= 2
                    && p.0[2].abs_diff(color.b()) <= 2
            })
            .count()
    }

    /// Count frame pixels within a hair of `color` inside `rect`, at the same
    /// +/-2 tolerance as [`count_band_color`] but scoped to one area, so a
    /// test can ask what a plane painted over a specific region.
    fn count_color_in(image: &image::RgbaImage, color: egui::Color32, rect: egui::Rect) -> usize {
        let (width, height) = image.dimensions();
        let x0 = rect.min.x.floor().max(0.0) as u32;
        let y0 = rect.min.y.floor().max(0.0) as u32;
        let x1 = (rect.max.x.ceil().max(0.0) as u32).min(width);
        let y1 = (rect.max.y.ceil().max(0.0) as u32).min(height);
        image
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                *x >= x0
                    && *x < x1
                    && *y >= y0
                    && *y < y1
                    && p.0[0].abs_diff(color.r()) <= 2
                    && p.0[1].abs_diff(color.g()) <= 2
                    && p.0[2].abs_diff(color.b()) <= 2
            })
            .count()
    }

    /// Render one browser column of idle rows over the card plane the shell
    /// paints behind a column, and return the frame with the rect the list
    /// occupied in it. The rows are all idle — no selection, no hover, no
    /// focus — so the only plane the list paints is the field itself, and the
    /// card plane is whatever the list failed to cover.
    fn render_idle_list_field() -> (image::RgbaImage, egui::Rect) {
        use riff_gui::ui::browser::show_browser_column;
        /// Where the list lands, captured out of the frame so a test can count
        /// the plane inside the list's own area rather than over the card's
        /// padding around it.
        #[derive(Clone, Copy, Default)]
        struct ListRect(Option<egui::Rect>);

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, list: &mut ListRect| {
                    // The pane behind the column is the card plane, exactly as
                    // the shell paints it.
                    let bg = ui.ctx().layer_painter(egui::LayerId::background());
                    bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                    list.0 = Some(egui::Rect::from_min_size(
                        ui.cursor().min,
                        ui.available_size(),
                    ));
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    show_browser_column(ui, &mut cache, &palette, column, &mut Vec::new());
                },
                ListRect::default(),
            );
        harness.run();
        let list = harness
            .state()
            .0
            .expect("the harness must have recorded where the list landed");
        let image = harness.render().expect("the list must render headlessly");
        (image, list)
    }

    /// An idle row sits on the row plane rather than on the card plane behind
    /// it. The mockup draws a list as a field, so a card's edge stays findable
    /// through it; the four-step ramp had no slot for that plane, which left
    /// idle rows painting nothing and the card showing straight through. The
    /// row plane is a property of the list rather than of a row, so the
    /// container paints it once — which is also the only way it can cover the
    /// list's whole area: these rows are virtualized, so a row-carried fill
    /// would stop at the last row actually rendered.
    #[test]
    fn test_idle_row_paints_the_row_plane_under_the_card() {
        use riff_gui::ui::theme::Palette;
        use riff_gui::ui::theme::geometry::browser::ROW_H;
        let palette = Palette::dark();
        let (field, list) = render_idle_list_field();

        assert!(
            count_color_in(&field, palette.surface_row, list) > 1_000,
            "an idle list paints the row plane"
        );

        // The card must not show through the field. The two planes sit four
        // per channel apart on the ramp by design, so an antialiased glyph
        // edge can land inside the +/-2 tolerance of either one — a few
        // scattered pixels are not a field. What a missing plane looks like is
        // the card filling the list, so the check is a budget: under 1% of the
        // list area, against 100% if the plane were never painted.
        let area = (list.width() * list.height()) as usize;
        let card = count_color_in(&field, palette.surface, list);
        assert!(
            card * 100 < area,
            "the card plane no longer shows through under idle rows: {card} of {area} px"
        );

        // And the field reaches past the last row rather than ending with it:
        // the band below the three fixture rows is row plane, all of it, with
        // no text in it to antialias against the card.
        let below =
            egui::Rect::from_min_max(egui::pos2(list.min.x, list.min.y + 3.0 * ROW_H), list.max);
        assert_eq!(
            count_color_in(&field, palette.surface, below),
            0,
            "no card plane below the last row"
        );
        assert_eq!(
            count_color_in(&field, palette.surface_row, below),
            (below.width() * below.height()) as usize,
            "the field covers the list below its last row"
        );
    }

    /// The neutral row frame maps a row's interaction state to one set of
    /// design tokens: selected wins the selected fill, otherwise hover paints
    /// the wash, an idle row paints neither, and focus adds the ring —
    /// independent of which variant (tree row or browser row) asked. Both
    /// Sections and Drill Columns now read their band from this one owner, so
    /// the states cannot drift between them.
    #[test]
    fn test_neutral_row_band_maps_state_to_tokens() {
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();

        let selected = render_row_band(true, false, false);
        assert!(
            count_band_color(&selected, palette.surface_3) > 1_000,
            "a selected row paints the selected fill"
        );
        assert_eq!(
            count_band_color(&selected, palette.row_hover),
            0,
            "a selected row never also paints the hover wash"
        );

        let hovered = render_row_band(false, true, false);
        assert!(
            count_band_color(&hovered, palette.row_hover) > 1_000,
            "a hovered row paints the hover wash"
        );
        assert_eq!(
            count_band_color(&hovered, palette.surface_3),
            0,
            "a hovered (not selected) row paints no selected fill"
        );

        let idle = render_row_band(false, false, false);
        assert_eq!(count_band_color(&idle, palette.surface_3), 0);
        assert_eq!(count_band_color(&idle, palette.row_hover), 0);

        let ring = riff_gui::ui::theme::focus_ring_stroke(&palette, true)
            .expect("a focused row has a ring stroke")
            .color;
        let focused = render_row_band(false, false, true);
        assert!(
            count_band_color(&focused, ring) > 0,
            "a focused row paints its focus ring"
        );
        assert_eq!(
            count_band_color(&idle, ring),
            0,
            "an unfocused row paints no ring"
        );
    }

    /// Render one shared icon button and return the frame, so a test can see
    /// whether the focus ring was painted and whether a disabled button took
    /// focus.
    fn render_icon_button(focused: bool, disabled: bool) -> image::RgbaImage {
        use riff_gui::ui::button::{begin_icon_button, finish_icon_button};
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let id = egui::Id::new("test_icon_button");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(80.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                if focused {
                    ui.memory_mut(|m| m.request_focus(id));
                }
                let rect =
                    egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(40.0, 40.0));
                let button = begin_icon_button(ui, rect, id, disabled);
                finish_icon_button(ui, &palette, &button, "Test button");
            });
        harness.run();
        harness
            .render()
            .expect("the icon button must render headlessly")
    }

    /// The shared icon-button foundation gives every adopted control the same
    /// focus treatment: a focused button paints the one focus ring, an
    /// unfocused one paints none, and a disabled button cannot take focus (so
    /// it never rings). This is the focus/disabled parity the four previously
    /// separate icon buttons lacked.
    #[test]
    fn test_icon_button_shares_focus_ring_and_disables_focus() {
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let ring = riff_gui::ui::theme::focus_ring_stroke(&palette, true)
            .expect("a focused button has a ring stroke")
            .color;

        let focused = render_icon_button(true, false);
        assert!(
            count_band_color(&focused, ring) > 0,
            "a focused icon button paints the shared focus ring"
        );

        let unfocused = render_icon_button(false, false);
        assert_eq!(
            count_band_color(&unfocused, ring),
            0,
            "an unfocused icon button paints no ring"
        );

        let disabled_focused = render_icon_button(true, true);
        assert_eq!(
            count_band_color(&disabled_focused, ring),
            0,
            "a disabled icon button cannot take focus, so it never rings"
        );
    }

    /// Render one shared semantic text button and return the frame, so a test
    /// can see which tokens a variant paints and whether it took the focus
    /// ring.
    fn render_text_button(
        variant: riff_gui::ui::button::Variant,
        focused: bool,
        disabled: bool,
    ) -> image::RgbaImage {
        use riff_gui::ui::button::{TextButton, text_button};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let id = egui::Id::new("test_text_button");
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(160.0, 44.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                if focused {
                    ui.memory_mut(|m| m.request_focus(id));
                }
                let rect =
                    egui::Rect::from_min_size(egui::pos2(16.0, 10.0), egui::vec2(128.0, 24.0));
                let spec = TextButton {
                    id,
                    rect,
                    label: "Go",
                    a11y: "Go",
                    tooltip: None,
                    icon: None,
                    small: false,
                    variant,
                    enabled: !disabled,
                };
                text_button(ui, &mut cache, &palette, &spec);
            });
        harness.run();
        harness
            .render()
            .expect("the text button must render headlessly")
    }

    /// Render one primary text button on the card plane with room around it for
    /// a halo to show, returning the frame and the button's rect. The rect is
    /// fixed by this helper so a test can sample the face and the plane around
    /// it without asking the harness where the button landed.
    fn render_primary_button() -> (image::RgbaImage, egui::Rect) {
        use riff_gui::ui::button::{TextButton, Variant, text_button};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::Palette;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 34.0), egui::vec2(120.0, 36.0));
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let id = egui::Id::new("test_primary_button");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 104.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                let spec = TextButton {
                    id,
                    rect,
                    label: "Add Library",
                    a11y: "Add Library",
                    tooltip: None,
                    icon: None,
                    small: false,
                    variant: Variant::Primary,
                    enabled: true,
                };
                text_button(ui, &mut cache, &palette, &spec);
            });
        harness.run();
        let frame = harness
            .render()
            .expect("the primary button must render headlessly");
        (frame, rect)
    }

    /// The most common color in `band` — the fill a region was painted, with
    /// the label's glyphs and their antialiasing ignored. A per-pixel probe
    /// would land on a letterform often enough to make any claim about the fill
    /// unreliable.
    fn modal_color_in(frame: &image::RgbaImage, band: egui::Rect) -> egui::Color32 {
        let (width, height) = frame.dimensions();
        let x0 = band.left().max(0.0) as u32;
        let y0 = band.top().max(0.0) as u32;
        let x1 = (band.right().ceil().max(0.0) as u32).min(width);
        let y1 = (band.bottom().ceil().max(0.0) as u32).min(height);
        let mut counts: std::collections::HashMap<[u8; 3], usize> =
            std::collections::HashMap::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let p = frame.get_pixel(x, y);
                *counts.entry([p.0[0], p.0[1], p.0[2]]).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .map(|(c, _)| egui::Color32::from_rgb(c[0], c[1], c[2]))
            .expect("the band must contain pixels")
    }

    /// Per-channel distance between two colors, for "which stop is this nearer".
    fn color_distance(a: egui::Color32, b: egui::Color32) -> u32 {
        u32::from(a.r().abs_diff(b.r()))
            + u32::from(a.g().abs_diff(b.g()))
            + u32::from(a.b().abs_diff(b.b()))
    }

    /// The primary button's face is a gradient, not a flat fill: the mockup
    /// lights its primary actions from above, lighter at the top, so the face
    /// has to run [`BRAND_GRADIENT_TOP`] to [`BRAND_GRADIENT_BOTTOM`] down its
    /// own height. Sampled as the modal color of five horizontal bands, which
    /// ignores the label, and required to be both non-uniform and monotonic —
    /// a pair of equal samples, or a face that jumped to the light stop and
    /// back, would not pass.
    #[test]
    fn test_primary_button_face_is_a_vertical_brand_gradient() {
        use riff_gui::ui::theme::{BRAND_GRADIENT_BOTTOM, BRAND_GRADIENT_TOP, RADIUS_MD};
        let (frame, rect) = render_primary_button();
        // Inset by the corner radius so the rounded ends stay out of the sample.
        let face = egui::Rect::from_min_max(
            egui::pos2(rect.left() + RADIUS_MD, rect.top()),
            egui::pos2(rect.right() - RADIUS_MD, rect.bottom()),
        );
        let bands: Vec<egui::Color32> = (0..5)
            .map(|i| {
                let (t0, t1) = (i as f32 / 5.0, (i + 1) as f32 / 5.0);
                modal_color_in(
                    &frame,
                    egui::Rect::from_min_max(
                        egui::pos2(face.left(), face.top() + face.height() * t0),
                        egui::pos2(face.right(), face.top() + face.height() * t1),
                    ),
                )
            })
            .collect();

        assert_ne!(
            bands[0], bands[4],
            "the primary face is not a flat fill: top {bands:?}"
        );
        assert!(
            color_distance(bands[0], BRAND_GRADIENT_TOP)
                < color_distance(bands[0], BRAND_GRADIENT_BOTTOM),
            "the top of the face is nearer the light stop, not the deep one"
        );
        assert!(
            color_distance(bands[4], BRAND_GRADIENT_BOTTOM)
                < color_distance(bands[4], BRAND_GRADIENT_TOP),
            "the bottom of the face is nearer the deep stop, not the light one"
        );
        // Monotonic top-to-bottom: each band is nearer the deep stop than the
        // one above it. A gradient that only darkened at one end, or one that
        // reversed, fails here.
        for pair in bands.windows(2) {
            assert!(
                color_distance(pair[1], BRAND_GRADIENT_BOTTOM)
                    < color_distance(pair[0], BRAND_GRADIENT_BOTTOM),
                "the face darkens monotonically down its height: {bands:?}"
            );
        }
    }

    /// A primary action sits on a soft accent glow: the brand accent, brightest
    /// at the control and falling to nothing, so the button reads as lit rather
    /// than pasted on. Checked just outside each side, where an unfocused
    /// Primary paints nothing else, and then past the halo's own reach, where
    /// the plane must be untouched — that second half is what separates a glow
    /// from a flat tinted plate.
    #[test]
    fn test_primary_button_sits_on_a_soft_accent_glow() {
        use riff_gui::ui::theme::{GLOW_ALPHA, GLOW_SPREAD, Palette, glow};
        let palette = Palette::dark();
        let (frame, rect) = render_primary_button();
        let plane = palette.surface;
        let (width, height) = frame.dimensions();
        let at = |probe: egui::Pos2| -> egui::Color32 {
            let (x, y) = (probe.x, probe.y);
            assert!(
                x >= 0.0 && y >= 0.0 && x < width as f32 && y < height as f32,
                "probe {x}/{y} is outside the frame"
            );
            let p = frame.get_pixel(x as u32, y as u32);
            egui::Color32::from_rgb(p.0[0], p.0[1], p.0[2])
        };
        // Brand-ness: how much further the pixel is toward the accent than the
        // plane was. The accent is far redder than the plane, so a real glow
        // raises red much more than it raises blue.
        let tinted_toward_brand = |p: egui::Color32| {
            i32::from(p.r()) - i32::from(p.b()) > i32::from(plane.r()) - i32::from(plane.b())
        };

        let mid_x = rect.center().x;
        let mid_y = rect.center().y;
        // The halo, sampled 3px and 5px out from each side's midpoint.
        for (side, probe) in [
            ("above", egui::pos2(mid_x, rect.top() - 3.0)),
            ("below", egui::pos2(mid_x, rect.bottom() + 3.0)),
            ("left", egui::pos2(rect.left() - 3.0, mid_y)),
            ("right", egui::pos2(rect.right() + 3.0, mid_y)),
        ] {
            let p = at(probe);
            assert_ne!(
                p, plane,
                "a soft accent glow reaches {side} the primary button"
            );
            assert!(
                tinted_toward_brand(p),
                "the glow {side} the button is tinted toward the brand accent, \
                 not just lightened: {p:?} vs plane {plane:?}"
            );
        }

        // Past the halo's own reach the plane is untouched, so this is a glow
        // with a falloff and not a translucent plate behind the button.
        let far = GLOW_SPREAD + 4.0;
        for (side, probe) in [
            ("above", egui::pos2(mid_x, rect.top() - far)),
            ("left", egui::pos2(rect.left() - far, mid_y)),
        ] {
            assert_eq!(
                at(probe),
                plane,
                "the glow has fallen off {side} the button by {far}px"
            );
        }
        // And the accent is genuinely there at the sampled strength the token
        // declares, so the halo is the token's wash and not a weaker imitation.
        let lit = glow(&palette, GLOW_ALPHA);
        assert!(
            lit.a() > 0 && lit.a() < 255,
            "GLOW_ALPHA {GLOW_ALPHA} yields a translucent wash, not an opaque one"
        );
    }

    /// Pixels lying inside the brand ramp — within a hair of the channel range
    /// spanned by the two gradient stops, so every point of the gradient counts
    /// and a color from any other part of the system does not.
    fn brand_ramp_pixels(frame: &image::RgbaImage) -> usize {
        use riff_gui::ui::theme::{BRAND_GRADIENT_BOTTOM, BRAND_GRADIENT_TOP};
        let (width, height) = frame.dimensions();
        let (lo, hi) = (BRAND_GRADIENT_BOTTOM, BRAND_GRADIENT_TOP);
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let p = frame.get_pixel(*x, *y);
                let on = |v: u8, lo: u8, hi: u8| {
                    i32::from(v) >= i32::from(lo) - 2 && i32::from(v) <= i32::from(hi) + 2
                };
                on(p.0[0], lo.r(), hi.r())
                    && on(p.0[1], lo.g(), hi.g())
                    && on(p.0[2], lo.b(), hi.b())
            })
            .count()
    }

    /// Render one artless `Placeholder::Gradient` block over the card plane
    /// with the given stops, and return the frame.
    fn render_gradient_placeholder(top: egui::Color32, bottom: egui::Color32) -> image::RgbaImage {
        use riff_gui::ui::artwork::{Artwork, Fit, Placeholder, paint};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::Palette;
        let _ = IconCache::new();
        let palette = Palette::dark();
        let rect = egui::Rect::from_min_size(egui::pos2(30.0, 30.0), egui::vec2(60.0, 40.0));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(120.0, 100.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                paint(
                    ui.painter(),
                    &palette,
                    &Artwork {
                        rect,
                        texture: None,
                        fit: Fit::Fill,
                        tint: riff_gui::ui::theme::TEXTURE_TINT,
                        placeholder: Some(Placeholder::Gradient { top, bottom }),
                        border: None,
                    },
                );
            });
        harness.run();
        harness
            .render()
            .expect("the placeholder must render headlessly")
    }

    /// The artless-artwork placeholder's gradient is the **caller's** to choose.
    /// Before this the variant's paint named `surface_2` and `surface_3` itself,
    /// so the gradient was fixed inside `artwork.rs` and a caller could not
    /// paint anything else — which is what made the design system's gradient a
    /// per-module decision rather than a token. So the assertion is that a
    /// supplied pair *reaches the pixels*: two different pairs must render two
    /// different frames, and the supplied stops must be the ones on screen.
    #[test]
    fn test_gradient_placeholder_paints_the_stops_its_caller_supplied() {
        use riff_gui::ui::theme::placeholder_gradient_stops;
        let palette = riff_gui::ui::theme::Palette::dark();

        // The system pair the app ships: the two raised-surface rungs.
        let [sys_top, sys_bottom] = placeholder_gradient_stops(&palette);
        let system = render_gradient_placeholder(sys_top, sys_bottom);

        // A pair the module could not have chosen for itself.
        let other = render_gradient_placeholder(
            riff_gui::ui::theme::BRAND_GRADIENT_TOP,
            riff_gui::ui::theme::BRAND_GRADIENT_BOTTOM,
        );
        let differing = system
            .as_raw()
            .iter()
            .zip(other.as_raw())
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            differing > 1_000,
            "a caller-supplied stop pair reaches the pixels: two different pairs \
             must render two different frames ({differing} bytes differ)"
        );

        // And the supplied stops really are the ends of what was painted: the
        // block's top row matches the top stop and its bottom row the bottom
        // stop. Read from the block's own interior, a third of the way in, so
        // neither row is an antialiased edge.
        let sample = |frame: &image::RgbaImage, y: u32| as_color(frame.get_pixel(60, y));
        let top_row = sample(&other, 30 + 40 / 3);
        let bottom_row = sample(&other, 30 + 40 * 2 / 3);
        assert!(
            color_distance(top_row, riff_gui::ui::theme::BRAND_GRADIENT_TOP)
                < color_distance(top_row, riff_gui::ui::theme::BRAND_GRADIENT_BOTTOM),
            "the supplied top stop is the lighter end of what was painted: {top_row:?}"
        );
        assert!(
            color_distance(bottom_row, riff_gui::ui::theme::BRAND_GRADIENT_BOTTOM)
                < color_distance(bottom_row, riff_gui::ui::theme::BRAND_GRADIENT_TOP),
            "the supplied bottom stop is the deeper end of what was painted: {bottom_row:?}"
        );
    }

    /// High Contrast is a *variant over* its base, so a slot it does not
    /// deliberately re-pick must come through untouched. This is the exhaustive
    /// half of that claim: the existing variant test names a handful of fields,
    /// and the whole surface ramp — `surface_row` included — is not among them,
    /// so a future edit that gave HC its own row plane would slip past it.
    #[test]
    fn test_high_contrast_inherits_every_surface_including_the_row_plane() {
        for (base, variant) in [
            (
                riff_gui::ui::theme::Palette::dark(),
                riff_gui::ui::theme::Palette::dark().high_contrast(),
            ),
            (
                riff_gui::ui::theme::Palette::light(),
                riff_gui::ui::theme::Palette::light().high_contrast(),
            ),
        ] {
            for field in [
                "background",
                "surface_row",
                "surface",
                "surface_2",
                "surface_3",
            ] {
                let value = |p: &riff_gui::ui::theme::Palette| match field {
                    "background" => p.background,
                    "surface_row" => p.surface_row,
                    "surface" => p.surface,
                    "surface_2" => p.surface_2,
                    _ => p.surface_3,
                };
                assert_eq!(
                    value(&variant),
                    value(&base),
                    "High Contrast inherits {field} from its base rather than \
                     re-picking it"
                );
            }
            // And brand is family-invariant, so the gradient's stops and the
            // glow's strength cannot drift between the two variants either.
            assert_eq!(variant.brand_primary, base.brand_primary);
        }
    }

    /// The accent glow's strength is per family, and the split is deliberate
    /// rather than cosmetic: a translucent wash's perceived strength depends on
    /// the backdrop's luminance, so one number cannot serve both. Measured in
    /// CIE L\*, the dark family's wash lifts its plane by ~22 and the light
    /// family's tints its plane by ~4, which is the read a light canvas needs
    /// for the same signal. High Contrast inherits its base family's value
    /// because it inherits its base family's surfaces, so there is no third
    /// number to keep in step.
    #[test]
    fn test_accent_glow_strength_is_split_per_family_and_hc_inherits_it() {
        use riff_gui::ui::theme::Palette;
        use riff_gui::ui::theme::glow_alpha;
        // Read through a Vec so the values are not const-folded at the call
        // site: the point of these assertions is the value, and a folded
        // comparison would be asserting the compiler's arithmetic back at it.
        let strengths: Vec<f32> = vec![
            glow_alpha(&Palette::dark()),
            glow_alpha(&Palette::light()),
            glow_alpha(&Palette::dark().high_contrast()),
            glow_alpha(&Palette::light().high_contrast()),
        ];
        let (dark, light, dark_hc, light_hc) =
            (strengths[0], strengths[1], strengths[2], strengths[3]);

        assert!(
            (dark - 0.35).abs() < 1e-6,
            "dark keeps the mockup's 0.35, got {dark}"
        );
        assert!(
            (light - 0.14).abs() < 1e-6,
            "light is the value the shadow tokens' 0.39x split implies, got {light}"
        );
        assert!(
            light < dark,
            "a light plane cannot glow — it is already at the top of the luminance \
             range — so its wash is a tint and needs less ink, not more \
             (light {light} vs dark {dark})"
        );
        assert_eq!(
            dark_hc, dark,
            "High Contrast re-picks no dark glow strength of its own"
        );
        assert_eq!(
            light_hc, light,
            "High Contrast re-picks no light glow strength of its own"
        );
    }

    /// A frame pixel as an egui color, for the distance comparisons.
    fn as_color(p: &image::Rgba<u8>) -> egui::Color32 {
        egui::Color32::from_rgb(p.0[0], p.0[1], p.0[2])
    }

    /// Render the inspector's quick-action row — the primary play action beside
    /// **Add to Queue** — and return the frame. Both buttons are primaries, so
    /// whatever the test finds on one it must find on the other.
    fn render_quick_actions() -> image::RgbaImage {
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::selection::{SelectionPanel, TagDraft};
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        // Tall enough to reach the quick-action row: it sits below the 200px art
        // block, and the panel body scrolls, so a short stage would clip it away.
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 560.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, _actions: &mut Vec<riff_gui::ui::selection::SelectionAction>| {
                    let bg = ui.ctx().layer_painter(egui::LayerId::background());
                    bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                    riff_gui::ui::selection::show_selection_panel(
                        ui,
                        &mut cache,
                        &palette,
                        SelectionPanel {
                            art: None,
                            title: Some("Music Has the Right to Children"),
                            subtitle: Some("Boards of Canada · 1998"),
                            details: &[],
                            tags: &[],
                            editor: None::<&mut TagDraft>,
                            single: false,
                            queue: true,
                        },
                        &mut Vec::new(),
                    );
                },
                Vec::new(),
            );
        harness.run();
        harness
            .render()
            .expect("the quick-action row must render headlessly")
    }

    /// **Add to Queue** sits in the same row as the primary play action, so the
    /// two must be painted by the same authority — the lit-from-above face and
    /// the accent bloom, not a flat brand fill beside a gradient. Before this
    /// the row mixed the two, which is the defect: the assertion is that *both*
    /// faces show vertical gradient variation *and* *both* sit on a glow, so a
    /// row where only one is lit cannot pass.
    #[test]
    fn test_both_quick_actions_are_painted_as_primaries() {
        use riff_gui::ui::theme::{BRAND_GRADIENT_BOTTOM, BRAND_GRADIENT_TOP};
        let frame = render_quick_actions();

        // A face is a set of COLUMNS that show more than one brand-ish colour
        // down their height — which is precisely what a vertical gradient is and
        // what a flat fill cannot be. Working in columns rather than rows means
        // the label's glyphs cannot break the measurement, and the two faces are
        // then the two horizontal clusters, separated by the row's gap.
        //
        // The tolerance is wide enough for the whole gradient between the two
        // stops and narrow enough to exclude the bloom, which composites to
        // about (49, 46, 43) and sits nowhere near either rung.
        let is_brandish = |p: &image::Rgba<u8>| {
            let near = |c: egui::Color32| {
                p.0[0].abs_diff(c.r()) <= 24
                    && p.0[1].abs_diff(c.g()) <= 24
                    && p.0[2].abs_diff(c.b()) <= 24
            };
            near(BRAND_GRADIENT_TOP) || near(BRAND_GRADIENT_BOTTOM)
        };
        let gradient_columns: Vec<u32> = (0..frame.width())
            .filter(|x| {
                let mut seen: Vec<image::Rgba<u8>> = Vec::new();
                for y in 0..frame.height() {
                    let p = *frame.get_pixel(*x, y);
                    if is_brandish(&p) && !seen.contains(&p) {
                        seen.push(p);
                    }
                }
                seen.len() >= 2
            })
            .collect();
        assert!(
            !gradient_columns.is_empty(),
            "a gradient face shows more than one brand-ish colour down its height"
        );

        // Cluster the columns; the row's two buttons are two clusters.
        let mut clusters: Vec<(u32, u32)> = Vec::new();
        for x in gradient_columns {
            match clusters.last_mut() {
                Some(last) if x <= last.1 + 1 => last.1 = x,
                _ => clusters.push((x, x)),
            }
        }
        let faces: Vec<(u32, u32)> = clusters.into_iter().filter(|(a, b)| b - a >= 20).collect();
        assert_eq!(
            faces.len(),
            2,
            "both quick actions are lit faces, so two gradient clusters are present: {faces:?}"
        );

        // Each face independently: the colour at its top must be nearer the
        // light stop than the colour at its bottom. Per face, so a row where
        // only the first is lit fails on the second.
        for (index, (left, right)) in faces.iter().enumerate() {
            let mut rows: Vec<(u32, image::Rgba<u8>)> = Vec::new();
            for y in 0..frame.height() {
                // The modal brand-ish colour on this row within the face: the
                // fill, ignoring whatever glyph lands on it.
                let mut counts: std::collections::HashMap<[u8; 3], usize> =
                    std::collections::HashMap::new();
                for x in *left..=*right {
                    let p = *frame.get_pixel(x, y);
                    if is_brandish(&p) {
                        *counts.entry([p.0[0], p.0[1], p.0[2]]).or_default() += 1;
                    }
                }
                if let Some((c, _)) = counts.into_iter().max_by_key(|(_, n)| *n) {
                    rows.push((y, image::Rgba([c[0], c[1], c[2], 255])));
                }
            }
            let (top_y, top_c) = rows.first().copied().expect("a face has rows");
            let (bottom_y, bottom_c) = rows.last().copied().expect("a face has rows");
            assert_ne!(
                top_c, bottom_c,
                "quick action {index} is a flat fill, not a gradient \
                 (top {top_c:?} at y={top_y} == bottom {bottom_c:?} at y={bottom_y})"
            );
            assert!(
                color_distance(as_color(&top_c), BRAND_GRADIENT_TOP)
                    < color_distance(as_color(&bottom_c), BRAND_GRADIENT_TOP),
                "quick action {index} is lit from above: its top {top_c:?} is nearer \
                 the light stop than its bottom {bottom_c:?}"
            );
        }
    }

    /// The quick-action row's face is painted before its button is added, so the
    /// bloom can go down first — which means the row claims the button's rect
    /// from the cursor before `add_sized` allocates it. That claim is only
    /// correct while `add_sized` still lays the button out at the cursor, so
    /// this holds the two together: a future layout change that moves the
    /// button would light the wrong rect, and this fails instead.
    #[test]
    fn test_quick_action_claims_the_rect_its_button_actually_takes() {
        use riff_gui::ui::theme::geometry::inspector::PLAY_H;
        let claimed: std::rc::Rc<std::cell::RefCell<Vec<(egui::Rect, egui::Rect)>>> =
            std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let sink = std::rc::Rc::clone(&claimed);
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(300.0, 120.0))
                .with_pixels_per_point(1.0)
                .build_ui(move |ui| {
                    ui.horizontal(|ui| {
                        for i in 0..2 {
                            let size = egui::vec2(140.0, PLAY_H);
                            let rect = egui::Rect::from_min_size(ui.cursor().min, size);
                            let button = egui::Button::new(egui::RichText::new("X"))
                                .fill(riff_gui::ui::theme::TRANSPARENT);
                            let response = ui.add_sized(size, button);
                            sink.borrow_mut().push((rect, response.rect));
                            let _ = i;
                        }
                    });
                });
            harness.run();
        }
        for (claim, taken) in claimed.borrow().iter() {
            assert_eq!(
                claim, taken,
                "a button takes the rect claimed from the cursor"
            );
        }
    }

    /// Render one accent-tier text button over the card plane, at rest or
    /// hovered. `paint_text_button` takes `hovered` as an argument, so this
    /// observes the two states in one frame each without moving a pointer —
    /// the same technique the row-band test uses for its four states.
    fn render_accent_button(hovered: bool) -> (image::RgbaImage, egui::Rect) {
        use riff_gui::ui::button::{Variant, paint_text_button};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::Palette;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 34.0), egui::vec2(120.0, 36.0));
        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 104.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                paint_text_button(
                    ui,
                    &mut cache,
                    &palette,
                    rect,
                    "Rescan now",
                    None,
                    false,
                    Variant::Accent,
                    true,
                    hovered,
                    false,
                );
            });
        harness.run();
        let frame = harness
            .render()
            .expect("the accent button must render headlessly");
        (frame, rect)
    }

    /// The mockup's second tier is not neutral: a secondary action takes a
    /// translucent **brand** wash on hover, the same gesture the destructive
    /// button makes in the error hue. So this is about the wash's *hue*, which
    /// is what separates it from the neutral hover it replaced — a
    /// `surface_3` hover is also "not the resting fill", so merely watching the
    /// face change would pass on the old code.
    #[test]
    fn test_accent_button_washes_toward_the_brand_hue_on_hover_only() {
        use riff_gui::ui::button::Variant;
        use riff_gui::ui::theme::{self, Palette, RADIUS_MD, accent_fill};
        let palette = Palette::dark();

        // Redness over blueness: the brand ramp is warm, so an accent wash
        // pushes this up hard. The warmest *neutral* on the surface ramp is
        // `surface_3` at (40, 37, 33) — a lead of 7 — so a threshold well above
        // that cannot be met by any neutral the button could be painted.
        let warmth = |c: egui::Color32| i32::from(c.r()) - i32::from(c.b());
        let face = |frame: &image::RgbaImage, rect: egui::Rect| {
            // The top band of the face, clear of the vertically centred label
            // and inset past the corner radius, so this reads the fill only.
            let band = egui::Rect::from_min_max(
                egui::pos2(rect.left() + RADIUS_MD, rect.top() + 2.0),
                egui::pos2(rect.right() - RADIUS_MD, rect.top() + 0.3 * rect.height()),
            );
            modal_color_in(frame, band)
        };

        let (resting, rect) = render_accent_button(false);
        let (hovered, _) = render_accent_button(true);
        let at_rest = face(&resting, rect);
        let on_hover = face(&hovered, rect);

        assert_eq!(
            at_rest, palette.surface_2,
            "an accent button at rest is the neutral base, unchanged"
        );
        assert!(
            warmth(at_rest) <= 8,
            "at rest there is no brand wash: {at_rest:?} is neutral (warmth {})",
            warmth(at_rest)
        );
        assert!(
            warmth(on_hover) >= 18,
            "on hover the face washes toward the brand hue, not a neutral step: \
             {on_hover:?} has warmth {} (at rest {at_rest:?} had {})",
            warmth(on_hover),
            warmth(at_rest)
        );
        // And it is the helper's wash doing it, at the coverage the helper
        // declares — not some stronger hand-picked tint.
        //
        // Composited the way egui composites its own output: `Color32` is
        // premultiplied, so the result is the stored top added onto the scaled
        // bottom. `theme::blend_over` is the straight-alpha helper and would
        // apply the alpha a second time, giving (30, 27, 25) instead — which is
        // also what a naive reading of "10% over the base" suggests and is why
        // this is worth stating explicitly.
        let wash_over = |base: egui::Color32| {
            let keep = 1.0 - f32::from(accent_fill(&palette, true).a()) / 255.0;
            let channel =
                |top: u8, bottom: u8| (f32::from(top) + f32::from(bottom) * keep).round() as u8;
            egui::Color32::from_rgb(
                channel(accent_fill(&palette, true).r(), base.r()),
                channel(accent_fill(&palette, true).g(), base.g()),
                channel(accent_fill(&palette, true).b(), base.b()),
            )
        };
        assert_eq!(
            on_hover,
            wash_over(palette.surface_2),
            "the hover wash is accent_fill composited over the base"
        );
        // The two washes are one gesture in two hues: same coverage, so a
        // destructive button and an accent button read as siblings.
        assert_eq!(
            accent_fill(&palette, true).a(),
            theme::destructive_fill(&palette, true).a(),
            "the accent and destructive washes are painted at the same coverage"
        );
        // At rest the helper contributes nothing at all, which is what keeps an
        // idle golden byte-identical.
        assert_eq!(
            accent_fill(&palette, false),
            riff_gui::ui::theme::TRANSPARENT
        );
        assert_ne!(Variant::Accent, Variant::Secondary);
    }

    /// The shared semantic-text-button foundation paints each variant from one
    /// set of tokens and gives every variant the same focus/disabled parity:
    /// Primary is brand-filled, Destructive carries the error ink, a focused
    /// enabled button rings, and a disabled button can neither take focus (so
    /// never rings) nor activate. This is the parity Settings' and the Detail
    /// Panel's hand-built buttons previously lacked.
    #[test]
    fn test_text_button_variants_share_focus_ring_and_disable() {
        use riff_gui::ui::button::Variant;
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let ring = riff_gui::ui::theme::focus_ring_stroke(&palette, true)
            .expect("a focused button has a ring stroke")
            .color;

        let primary = render_text_button(Variant::Primary, false, false);
        // Primary's face is the brand gradient, so it is painted from the light
        // rung down to the deep one rather than in one flat brand tone. The
        // invariant here is unchanged — Primary is the brand-painted variant,
        // against Destructive's "never brand-filled" below — only the pixels it
        // is stated in. The dedicated gradient test owns the stops and the
        // falloff; this one still asks for a whole field of brand paint.
        assert!(
            brand_ramp_pixels(&primary) > 1_000,
            "Primary paints the brand ramp"
        );

        let destructive = render_text_button(Variant::Destructive, false, false);
        assert!(
            count_band_color(&destructive, palette.error) > 0,
            "Destructive paints the error ink"
        );
        assert_eq!(
            count_band_color(&destructive, palette.brand_primary),
            0,
            "Destructive is never brand-filled"
        );

        let focused = render_text_button(Variant::Secondary, true, false);
        assert!(
            count_band_color(&focused, ring) > 0,
            "a focused enabled button paints the shared focus ring"
        );

        let unfocused = render_text_button(Variant::Secondary, false, false);
        assert_eq!(
            count_band_color(&unfocused, ring),
            0,
            "an unfocused button paints no ring"
        );

        let disabled_focused = render_text_button(Variant::Primary, true, true);
        assert_eq!(
            count_band_color(&disabled_focused, ring),
            0,
            "a disabled button cannot take focus, so it never rings"
        );
    }

    /// Render the shared text-field seam over an owned value buffer and return
    /// the frame, so a test can see the focus ring the well paints.
    fn run_text_field(focused: bool) -> image::RgbaImage {
        use riff_gui::ui::icons::Icon;
        use riff_gui::ui::text_field::{TextField, text_field};
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let id = egui::Id::new("test_text_field");
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 40.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, value| {
                    let bg = ui.ctx().layer_painter(egui::LayerId::background());
                    bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                    if focused {
                        ui.memory_mut(|m| m.request_focus(id));
                    }
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 24.0));
                    text_field(
                        ui,
                        &mut cache,
                        &palette,
                        value,
                        &TextField {
                            id,
                            rect,
                            hint: "Search or jump to…",
                            leading_icon: Some(Icon::Search),
                            clear_label: Some("Clear search"),
                            dismiss_on_escape: true,
                        },
                    );
                },
                String::new(),
            );
        harness.run();
        harness
            .render()
            .expect("the text field must render headlessly")
    }

    /// The shared text-field contract gives every surface the same well, clear
    /// affordance and focus treatment: the clear button appears only while the
    /// value is non-empty and empties it when clicked, and a focused field
    /// paints the search ring while an idle one paints the hairline instead.
    /// Titlebar search drives the exact same primitive end to end.
    #[test]
    fn test_text_field_shares_clear_affordance_and_focus_ring() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::icons::Icon;
        use riff_gui::ui::text_field::{TextField, text_field};
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let id = egui::Id::new("tf_clear");

        // A non-empty field exposes a clear affordance that empties the value.
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 40.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, value| {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 24.0));
                    text_field(
                        ui,
                        &mut cache,
                        &palette,
                        value,
                        &TextField {
                            id,
                            rect,
                            hint: "Search or jump to…",
                            leading_icon: Some(Icon::Search),
                            clear_label: Some("Clear search"),
                            dismiss_on_escape: true,
                        },
                    );
                },
                "boards".to_string(),
            );
        harness.run();
        harness.get_by_label("Clear search").click();
        harness.run();
        assert!(
            harness.state().is_empty(),
            "the clear affordance empties the value"
        );

        // An empty field offers no clear affordance.
        let mut cache2 = IconCache::new();
        let mut idle = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 40.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, value| {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(200.0, 24.0));
                    text_field(
                        ui,
                        &mut cache2,
                        &palette,
                        value,
                        &TextField {
                            id,
                            rect,
                            hint: "Search or jump to…",
                            leading_icon: Some(Icon::Search),
                            clear_label: Some("Clear search"),
                            dismiss_on_escape: true,
                        },
                    );
                },
                String::new(),
            );
        idle.run();
        assert!(
            idle.query_by_label("Clear search").is_none(),
            "an empty field shows no clear affordance"
        );

        // Focus drives the well's ring, not a fixed stroke.
        let ring_color = riff_gui::ui::sidebar::search_ring_stroke(&palette, true).color;
        let focused_img = run_text_field(true);
        assert!(
            count_band_color(&focused_img, ring_color) > 0,
            "a focused text field paints the search ring"
        );
        let idle_img = run_text_field(false);
        assert_eq!(
            count_band_color(&idle_img, ring_color),
            0,
            "an idle text field paints no focus ring"
        );
    }

    /// Render the shared linear-control seam and report whether it exposed an
    /// operable `Seek` slider node and how many brand-fill pixels it painted.
    fn render_linear(value: f32, interactive: bool) -> (bool, usize) {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::linear::{LinearControl, linear_control};
        use riff_gui::ui::theme::geometry::seek::TRACK_H;
        let palette = riff_gui::ui::theme::Palette::dark();
        let id = egui::Id::new("test_linear");
        let track = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, TRACK_H));
        let hit = track.expand2(egui::vec2(0.0, 12.0));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                linear_control(
                    ui,
                    &palette,
                    &LinearControl {
                        id,
                        track,
                        hit,
                        value,
                        thumb: None,
                        interactive,
                        label: "Seek",
                    },
                );
            });
        harness.run();
        let has_node = harness.query_by_label("Seek").is_some();
        let frame = harness.render().expect("linear control renders");
        let fills = frame
            .pixels()
            .filter(|p| {
                p.0[0].abs_diff(palette.brand_primary.r()) <= 2
                    && p.0[1].abs_diff(palette.brand_primary.g()) <= 2
                    && p.0[2].abs_diff(palette.brand_primary.b()) <= 2
            })
            .count();
        (has_node, fills)
    }

    /// The shared linear-control contract: an interactive seek exposes an
    /// operable slider node while a non-seekable (unknown / zero duration) one
    /// exposes none, and a zero value paints zero brand fill (no thumb-less
    /// stub) where a half value paints some. Both surfaces reach these through
    /// the same `linear::linear_control`, so their pointer→intent mapping is
    /// identical by construction (proven end to end by the Playerbar seek /
    /// volume click tests above).
    #[test]
    fn test_linear_control_zero_fill_and_seekable_gate() {
        let (live, _) = render_linear(0.5, true);
        assert!(
            live,
            "an interactive control exposes an operable slider node"
        );

        let (gated, _) = render_linear(0.5, false);
        assert!(
            !gated,
            "a non-seekable (non-interactive) control exposes no operable node"
        );

        let (_, zero_fill) = render_linear(0.0, true);
        assert_eq!(zero_fill, 0, "a zero value paints zero brand fill");

        let (_, half_fill) = render_linear(0.5, true);
        assert!(half_fill > 0, "a non-zero value paints a brand fill");
    }

    /// The shared empty-state owner renders a labelled title+hint composition
    /// through both layout models: the flowing `Ui` path (explorer columns)
    /// exposes the title and hint as readable labels, and the painter/rect path
    /// (hand-laid queue) paints non-blank ink into the given rect — never an
    /// unexplained hole.
    #[test]
    fn test_empty_state_owner_composes_labelled_state_both_paths() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::empty_state::{empty_state, empty_state_in_rect};
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();

        // Flowing Ui path: title and hint are present as readable labels.
        let mut ui_h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| empty_state(ui, &palette, "No artists yet", "Add a folder."));
        ui_h.run();
        assert!(
            ui_h.query_by_label("No artists yet").is_some(),
            "the flowing empty state labels the title"
        );
        assert!(
            ui_h.query_by_label("Add a folder.").is_some(),
            "the flowing empty state labels the hint"
        );

        // Painter/rect path: it paints ink into the rect (distinct from blank).
        let mut rect_h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                let rect = ui.max_rect();
                empty_state_in_rect(
                    &ui.painter_at(rect),
                    &palette,
                    rect,
                    "Queue is empty",
                    "Play a track to start your queue.",
                );
            });
        rect_h.run();
        let frame = rect_h.render().expect("rect empty state renders");
        let ink = frame
            .pixels()
            .filter(|p| {
                p.0[0].abs_diff(palette.ink_2.r()) <= 4
                    && p.0[1].abs_diff(palette.ink_2.g()) <= 4
                    && p.0[2].abs_diff(palette.ink_2.b()) <= 4
            })
            .count();
        assert!(
            ink > 20,
            "the painter empty state paints a title in ink, not a blank hole"
        );
    }

    /// Harness state for the detail column's data-path test: the production
    /// seam/store pairing, the library session driving it, the icon cache
    /// the widget needs, and the actions the last frame reported.
    struct DetailRenderState {
        views: riff_backend::app::views::SessionViews,
        library: riff_backend::app::state::LibrarySession,
        cache: IconCache,
        actions: Vec<riff_gui::ui::detail::DetailAction>,
    }

    /// The production Tracks-column data path, replicated frame-for-frame:
    /// `resolve_detail_content` maps the library session through the
    /// Session Views seam, and `show_detail_column` paints it. Entity
    /// listings are their own columns in the stage, so the widget receives
    /// no rows.
    fn render_detail_state_ui(ui: &mut egui::Ui, s: &mut DetailRenderState) {
        let palette = Palette::dark();
        let content = riff_gui::ui::app::resolve_detail_content(
            &mut s.views,
            &s.library,
            &s.library.search_query,
        );
        let column = riff_gui::ui::detail::DetailColumn {
            breadcrumb: &content.breadcrumb,
            header: content.header.as_ref(),
            tracks: &content.tracks,
            rows: &[],
            ..riff_gui::ui::detail::DetailColumn::empty("Nothing selected", "Pick a row.")
        };
        s.actions.clear();
        riff_gui::ui::detail::show_detail_column(
            ui,
            &mut s.cache,
            &palette,
            column,
            &mut s.actions,
        );
    }

    /// Seed the store behind `views` with Boards of Canada's "Geogaddi"
    /// (two tagged tracks: one played three times, one favorited) and
    /// return the two `Track`s.
    fn seed_geogaddi(
        store: &mut riff_infra::store::SqliteStore,
    ) -> (riff_backend::domain::Track, riff_backend::domain::Track) {
        use riff_backend::app::store::LibraryMutationStore as _;
        use riff_backend::domain::TrackMetadata;

        let mut t1 = crate::test_utils::create_test_track(
            "music/Boards of Canada/Geogaddi/01.flac",
            "music/Boards of Canada/Geogaddi/01.flac",
        );
        t1.metadata = TrackMetadata {
            title: Some("Magic Window".to_string()),
            artist: Some("Boards of Canada".to_string()),
            album: Some("Geogaddi".to_string()),
            album_artist: Some("Boards of Canada".to_string()),
            track_number: Some(1),
            year: Some(2002),
            genre: Some("Electronic".to_string()),
            ..TrackMetadata::default()
        };
        t1.duration = Some(std::time::Duration::from_secs(205));
        let mut t2 = crate::test_utils::create_test_track(
            "music/Boards of Canada/Geogaddi/02.flac",
            "music/Boards of Canada/Geogaddi/02.flac",
        );
        t2.metadata = TrackMetadata {
            title: Some("Dawn Chorus".to_string()),
            artist: Some("Boards of Canada".to_string()),
            album: Some("Geogaddi".to_string()),
            album_artist: Some("Boards of Canada".to_string()),
            track_number: Some(2),
            year: Some(2002),
            genre: Some("Electronic".to_string()),
            ..TrackMetadata::default()
        };
        t2.favorite = true;
        store.apply_scan_batch(&[t1.clone(), t2.clone()]).unwrap();
        // The favorite flag is a user fact: scan batches never write it,
        // so the fixture's favorite lands through the store's setter.
        store.set_track_favorite(&t2.id, true).unwrap();
        // Play history is an engine-owned fact: the scan batch always
        // writes play_count 0, so the plays land through the store's
        // play recorder — three finished plays of "Magic Window".
        for _ in 0..3 {
            store
                .record_track_played(&t1.id, std::time::SystemTime::UNIX_EPOCH)
                .unwrap();
        }
        (t1, t2)
    }

    /// Three tracks of one album covering every per-field aggregation state:
    /// Title/Year/Track Number differ between tracks, Album and Album Artist
    /// agree, Artist is mixed present/missing, and Genre is missing on every
    /// track. The genre-missing track (t3) also drives a Track readout's
    /// grey `(none)` state.
    fn seed_varied_album(
        store: &mut riff_infra::store::SqliteStore,
    ) -> [riff_backend::domain::Track; 3] {
        use riff_backend::app::store::LibraryMutationStore as _;
        use riff_backend::domain::TrackMetadata;

        let mut t1 = crate::test_utils::create_test_track(
            "music/Mixed Artist/Varied/01.flac",
            "music/Mixed Artist/Varied/01.flac",
        );
        t1.metadata = TrackMetadata {
            title: Some("Song A".to_string()),
            artist: Some("Artist One".to_string()),
            album: Some("Varied".to_string()),
            album_artist: Some("Mixed".to_string()),
            track_number: Some(1),
            year: Some(2001),
            ..TrackMetadata::default()
        };
        let mut t2 = crate::test_utils::create_test_track(
            "music/Mixed Artist/Varied/02.flac",
            "music/Mixed Artist/Varied/02.flac",
        );
        t2.metadata = TrackMetadata {
            title: Some("Song B".to_string()),
            artist: None,
            album: Some("Varied".to_string()),
            album_artist: Some("Mixed".to_string()),
            track_number: Some(2),
            year: Some(2002),
            ..TrackMetadata::default()
        };
        let mut t3 = crate::test_utils::create_test_track(
            "music/Mixed Artist/Varied/03.flac",
            "music/Mixed Artist/Varied/03.flac",
        );
        t3.metadata = TrackMetadata {
            title: Some("Song C".to_string()),
            artist: Some("Artist Three".to_string()),
            album: Some("Varied".to_string()),
            album_artist: Some("Mixed".to_string()),
            track_number: Some(3),
            year: Some(2003),
            ..TrackMetadata::default()
        };
        store
            .apply_scan_batch(&[t1.clone(), t2.clone(), t3.clone()])
            .unwrap();
        [t1, t2, t3]
    }

    #[test]
    fn test_tracks_column_resolves_the_librarys_data_and_stays_fresh() {
        use egui_kittest::kittest::Queryable;
        use riff_backend::app::state::{BrowserSelection, LibrarySection};
        use riff_backend::app::store::LibraryMutationStore;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");
        let (t1, _t2) = seed_geogaddi(&mut store);
        let views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );

        let state = DetailRenderState {
            views,
            library: riff_backend::app::state::LibrarySession {
                library_section: LibrarySection::Artists,
                browser_path: vec![
                    BrowserSelection::Artist("Boards of Canada".to_string()),
                    BrowserSelection::Album {
                        artist: "Boards of Canada".to_string(),
                        title: "Geogaddi".to_string(),
                    },
                ],
                ..riff_backend::app::state::LibrarySession::default()
            },
            cache: IconCache::new(),
            actions: Vec::new(),
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(render_detail_state_ui, state);
        harness.run();

        // Album level: the breadcrumb trail reads the section root and one
        // crumb per path entry, and the header + track list resolve from
        // the store.
        assert!(
            harness.query_by_label("Artists").is_some()
                && harness.query_by_label("Boards of Canada").is_some()
                && harness.query_all_by_label("Geogaddi").count() >= 2,
            "the breadcrumb and the header both name the album (trail + title)"
        );
        assert!(
            harness
                .query_by_label("Boards of Canada \u{b7} 2002")
                .is_some(),
            "the album header renders the artist \u{b7} year subtitle"
        );
        assert!(
            harness.query_by_label("Magic Window").is_some()
                && harness.query_by_label("Dawn Chorus").is_some(),
            "the track list resolves the album's tracks"
        );
        // Plays and durations ride in each row's right-aligned
        // `album · plays · time` cluster — painted text, so their values are
        // pinned visually by the detail-column golden, not by widget labels.
        assert!(
            harness.query_by_label("Remove from Favorites").is_some()
                && harness.query_by_label("Add to Favorites").is_some(),
            "each row's favorite control reflects its stored flag"
        );

        // Above the album level the Tracks column carries no header or track
        // list — entity listings are their own columns now; only
        // the breadcrumb trail renders.
        harness.state_mut().library.browser_path =
            vec![BrowserSelection::Artist("Boards of Canada".to_string())];
        harness.run();
        assert!(
            harness.query_by_label("Magic Window").is_none(),
            "the artist level renders no track table"
        );
        assert!(
            harness.query_by_label("Boards of Canada").is_some(),
            "the artist level still renders the breadcrumb trail"
        );

        // Staleness: a favorite committed through the store (exactly what
        // a tag edit or scan batch does to library data) shows up on the
        // next frame with zero caller action — the committed mutation
        // bumped the library generation itself.
        harness.state_mut().library.browser_path = vec![
            BrowserSelection::Artist("Boards of Canada".to_string()),
            BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            },
        ];
        harness.run();
        let bumped = store
            .set_track_favorite(&t1.id, true)
            .expect("the favorite commit works");
        assert!(bumped, "the flag actually moved");
        harness.run();
        assert!(
            harness.query_all_by_label("Remove from Favorites").count() == 2,
            "the fresh flag renders without any explicit invalidation"
        );

        // Genre drill: the same album's tracks resolve genre-scoped through
        // the Genres section, with the genre's own breadcrumb root.
        harness.state_mut().library.library_section = LibrarySection::Genres;
        harness.state_mut().library.browser_path = vec![
            BrowserSelection::Genre("Electronic".to_string()),
            BrowserSelection::Artist("Boards of Canada".to_string()),
            BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            },
        ];
        harness.run();
        assert!(
            harness.query_by_label("Genres").is_some()
                && harness.query_by_label("Electronic").is_some(),
            "the genre trail's breadcrumb names the genre section"
        );
        assert!(
            harness.query_by_label("Magic Window").is_some()
                && harness.query_by_label("Dawn Chorus").is_some(),
            "the genre-scoped track table resolves the album's tracks"
        );
    }

    // --- Inspector (the collapsible selection panel, handoff issue 10) ----------
    //
    // The elastic stage's rightmost column: a readout of the live selection
    // (album / artist / genre / track) with Play album and Add to Queue.
    // Tested at the same seams as the browser column (issue 08) and detail
    // column (issue 09): the pure widget seam
    // (`ui::selection::show_selection_panel`, headless kittest harness) and
    // the session-glue seam in `ui::app`.

    #[test]
    fn test_selection_panel_renders_album_details_and_reports_play_album() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionDetail, SelectionPanel, show_selection_panel,
        };

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let details = vec![
            SelectionDetail {
                label: "Artist".to_string(),
                value: "Boards of Canada".to_string(),
            },
            SelectionDetail {
                label: "Released".to_string(),
                value: "2013".to_string(),
            },
            SelectionDetail {
                label: "Tracks".to_string(),
                value: "8 \u{b7} 27:16".to_string(),
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Tomorrow's Harvest"),
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &details,
                        tags: &[],
                        editor: None,
                        single: false,
                        queue: false,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The panel names itself and the selection's kind, then reads out
        // the album: title, artist · year line, and the details grid.
        assert!(
            harness.query_by_label("SELECTION").is_some(),
            "the panel's header renders"
        );
        assert!(
            harness.query_by_label("Album").is_some(),
            "the header's kind chip renders"
        );
        assert!(
            harness.query_by_label("Tomorrow's Harvest").is_some(),
            "the album title renders"
        );
        assert!(
            harness.query_by_label("Boards of Canada · 2013").is_some(),
            "the artist · year subtitle renders"
        );
        assert!(
            harness.query_by_label("DETAILS").is_some(),
            "the details section header renders"
        );
        for (label, value) in [
            ("Artist", "Boards of Canada"),
            ("Released", "2013"),
            ("Tracks", "8 · 27:16"),
        ] {
            assert!(
                harness.query_by_label(label).is_some() && harness.query_by_label(value).is_some(),
                "the details row '{label}: {value}' renders"
            );
        }

        // The Play album action reports itself — `ui::app` starts the batch.
        harness.get_by_label("Play album").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::PlayAlbum],
            "the Play album button reports its action"
        );
    }

    /// The inspector column has a fixed height and clips its content, so a
    /// readout taller than the column must scroll — otherwise its tail is
    /// unreachable. Pinned at the widget seam: the details grid below the
    /// fold is scrolled into view, and the `SELECTION` header stays put.
    #[test]
    fn test_selection_panel_scrolls_the_readout_under_its_pinned_header() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionDetail, SelectionPanel, show_selection_panel,
        };

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let details: Vec<SelectionDetail> = (0..12)
            .map(|i| SelectionDetail {
                label: format!("Detail {i}"),
                value: format!("Value {i}"),
            })
            .collect();
        // Short enough that the 200 px art block plus the title and action
        // rows leave the details grid below the fold.
        const VIEW_H: f32 = 320.0;

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, VIEW_H))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Tomorrow's Harvest"),
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &details,
                        tags: &[],
                        editor: None,
                        single: false,
                        queue: false,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        let header_y = harness.get_by_label("SELECTION").rect().top();
        let tail = harness.get_by_label("Detail 11").rect();
        assert!(
            tail.top() > VIEW_H,
            "the last detail row starts below the fold: {tail:?} in a {VIEW_H} px column"
        );

        harness.get_by_label("Detail 11").scroll_to_me();
        harness.run();
        let scrolled = harness.get_by_label("Detail 11").rect();
        assert!(
            scrolled.top() < tail.top(),
            "the readout scrolls toward its tail: {tail:?} → {scrolled:?}"
        );
        assert!(
            scrolled.bottom() <= VIEW_H,
            "scrolling brings the last detail row into the column: {scrolled:?}"
        );
        assert!(
            harness.get_by_label("SELECTION").rect().top() == header_y,
            "the header stays pinned while the readout scrolls"
        );
    }

    #[test]
    fn test_selection_panel_renders_a_clear_empty_state() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{SelectionAction, SelectionPanel, show_selection_panel};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: None,
                        subtitle: None,
                        details: &[],
                        tags: &[],
                        editor: None,
                        single: false,
                        queue: false,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // Before any album is selected the panel says so in plain words —
        // and offers nothing to click.
        assert!(
            harness.query_by_label("Nothing selected").is_some(),
            "the empty state names itself"
        );
        assert!(
            harness.query_by_label("Play album").is_none(),
            "the empty state offers no play action"
        );
        assert!(
            harness.state().is_empty(),
            "the empty state reports nothing"
        );
    }

    #[test]
    fn test_selection_panel_renders_the_tag_section_in_the_four_states() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionPanel, TagField, TagRow, TagRowState, show_selection_panel,
        };

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        // One row per display state: a shared value, the orange `(different)`
        // aggregation, and the grey `(none)` of a field no track carries.
        // The widget renders the resolved text and state; it never re-derives.
        let tags = vec![
            TagRow {
                field: TagField::Title,
                state: TagRowState::Value,
                text: "Nothing Is Real".to_string(),
                originals: vec![Some("Nothing Is Real".to_string())],
            },
            TagRow {
                field: TagField::Year,
                state: TagRowState::Different,
                text: "(different)".to_string(),
                originals: vec![Some("1999".to_string()), Some("2013".to_string())],
            },
            TagRow {
                field: TagField::Genre,
                state: TagRowState::None,
                text: "(none)".to_string(),
                originals: vec![None],
            },
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Some Album"),
                        subtitle: Some("Some Artist \u{b7} 2013"),
                        details: &[],
                        tags: &tags,
                        editor: None,
                        single: false,
                        queue: false,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The section names itself and resolves each row's label and display
        // text exactly as the app layer resolved them.
        assert!(
            harness.query_by_label("TAGS").is_some(),
            "the tag section header renders"
        );
        for (label, text) in [
            ("Title", "Nothing Is Real"),
            ("Year", "(different)"),
            ("Genre", "(none)"),
        ] {
            assert!(
                harness.query_by_label(label).is_some() && harness.query_by_label(text).is_some(),
                "the tag row '{label}: {text}' renders"
            );
        }
    }

    #[test]
    fn test_selection_panel_inline_editor_renders_prefilled_fields_and_reports_the_bar() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionPanel, TagDraft, TagField, TagRow, TagRowState,
            show_selection_panel,
        };
        use std::path::PathBuf;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        // A readout where Title shows a value, Genre is grey `(none)`, and
        // Year shows its value — the draft must open with Title and Year
        // prefilled and the `(none)` Genre as an empty buffer.
        let tags = vec![
            TagRow {
                field: TagField::Title,
                state: TagRowState::Value,
                text: "Old Title".to_string(),
                originals: vec![Some("Old Title".to_string())],
            },
            TagRow {
                field: TagField::Genre,
                state: TagRowState::None,
                text: "(none)".to_string(),
                originals: vec![None],
            },
            TagRow {
                field: TagField::Year,
                state: TagRowState::Value,
                text: "2002".to_string(),
                originals: vec![Some("2002".to_string())],
            },
        ];
        let mut draft = TagDraft::for_track(
            TrackId("t1.mp3".to_string()),
            PathBuf::from("t1.mp3"),
            &tags,
        );
        assert_eq!(
            draft.fields[TagField::Title.index()],
            "Old Title",
            "a value row prefills its buffer from the readout"
        );
        assert_eq!(
            draft.fields[TagField::Genre.index()],
            "",
            "a (none) row opens as an empty buffer"
        );
        assert_eq!(draft.fields[TagField::Year.index()], "2002");
        assert!(
            !draft.any_dirty(),
            "a fresh draft is never dirty by construction"
        );

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 900.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Old Title"),
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                        single: true,
                        queue: true,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The editor renders the fields and the Save bar; typing into the
        // focused field writes the draft buffer and marks it dirty.
        assert!(
            harness.query_by_label("TAGS").is_some(),
            "the editor keeps the tag section header"
        );
        assert!(
            harness.query_by_label("Save").is_some() && harness.query_by_label("Cancel").is_some(),
            "the Save bar renders"
        );
        let title_field = harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .expect("the editor renders its fields");
        title_field.focus();
        harness.run();
        harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .next()
            .expect("the editor still renders its fields")
            .type_text("X");
        harness.run();
        drop(harness);

        assert!(
            draft.fields[TagField::Title.index()].contains('X'),
            "typing writes the focused field's draft buffer"
        );
        assert!(
            draft.is_dirty(TagField::Title),
            "an edited field is dirty against its original"
        );
        assert!(draft.any_dirty());
        assert!(draft.fields[TagField::Genre.index()].is_empty());
        assert!(
            !draft.is_dirty(TagField::Genre),
            "the untouched (none) row stays clean"
        );
    }

    #[test]
    fn test_selection_panel_inline_editor_reports_save_on_enter_and_cancel_on_escape() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionPanel, TagDraft, TagField, TagRow, TagRowState,
            show_selection_panel,
        };
        use std::path::PathBuf;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tags = vec![TagRow {
            field: TagField::Title,
            state: TagRowState::Value,
            text: "Old Title".to_string(),
            originals: vec![Some("Old Title".to_string())],
        }];
        let mut draft = TagDraft::for_track(
            TrackId("t1.mp3".to_string()),
            PathBuf::from("t1.mp3"),
            &tags,
        );
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 900.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Old Title"),
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                        single: true,
                        queue: true,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        harness.key_press(egui::Key::Enter);
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::SaveTagEdit],
            "Enter saves the inline edit"
        );

        // The Save button reports the same action as Enter.
        harness.get_by_label("Save").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::SaveTagEdit, SelectionAction::SaveTagEdit],
            "the Save button reports the same intent as Enter"
        );

        harness.key_press(egui::Key::Escape);
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![
                SelectionAction::SaveTagEdit,
                SelectionAction::SaveTagEdit,
                SelectionAction::CancelTagEdit,
            ],
            "Escape discards the inline edit"
        );
    }

    #[test]
    fn test_selection_panel_inline_editor_disables_save_while_a_write_is_in_flight() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionPanel, TagDraft, TagField, TagRow, TagRowState,
            show_selection_panel,
        };
        use std::path::PathBuf;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tags = vec![TagRow {
            field: TagField::Title,
            state: TagRowState::Value,
            text: "Old Title".to_string(),
            originals: vec![Some("Old Title".to_string())],
        }];
        let mut draft = TagDraft::for_track(
            TrackId("t1.mp3".to_string()),
            PathBuf::from("t1.mp3"),
            &tags,
        );
        draft.saving = true;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 900.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Old Title"),
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                        single: true,
                        queue: true,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        // The save spinner asks to repaint every frame, so step instead of run.
        harness.run_steps(2);

        harness.get_by_label("Save").click();
        harness.run_steps(2);
        assert!(
            harness.state().is_empty(),
            "a double-submit is impossible: Save is disabled while the write is in flight"
        );
    }

    #[test]
    fn test_selection_panel_inline_editor_shows_batch_progress_and_summary() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            BatchStatus, SelectionAction, SelectionPanel, TagDraft, TagField, TagRow, TagRowState,
            show_selection_panel,
        };
        use std::path::PathBuf;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tags = vec![TagRow {
            field: TagField::Title,
            state: TagRowState::Value,
            text: "Shared Title".to_string(),
            originals: vec![Some("Shared Title".to_string()); 2],
        }];
        let mut draft = TagDraft::for_album(
            vec![
                (
                    TrackId("/music/a1.mp3".to_string()),
                    PathBuf::from("/music/a1.mp3"),
                ),
                (
                    TrackId("/music/a2.mp3".to_string()),
                    PathBuf::from("/music/a2.mp3"),
                ),
            ],
            &tags,
        );
        draft.batch = Some(BatchStatus {
            total: 2,
            saved: 1,
            failed: 1,
            first_failure: Some("permission denied".to_string()),
        });
        draft.saving = false;

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 900.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Shared Title"),
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                        single: false,
                        queue: true,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        assert!(
            harness
                .query_by_label("Saved 1 of 2 tracks — 1 failed: permission denied")
                .is_some(),
            "a landed batch renders its partial-failure summary under the bar"
        );
    }

    #[test]
    fn test_selection_panel_tag_row_click_reports_start_edit() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{
            SelectionAction, SelectionPanel, TagField, TagRow, TagRowState, show_selection_panel,
        };

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tags = vec![TagRow {
            field: TagField::Title,
            state: TagRowState::Value,
            text: "Tag Title".to_string(),
            originals: vec![Some("Tag Title".to_string())],
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Panel Title"),
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: None,
                        single: false,
                        queue: false,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // A tag row value is editable: clicking it reports the edit intent
        // the app turns into a per-selection draft.
        harness.get_by_label("Tag Title").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::StartEdit],
            "clicking a tag row reports StartEdit"
        );
    }

    #[test]
    fn test_selection_play_album_starts_the_albums_tracks() {
        use riff_gui::ui::app::apply_selection_action;
        use riff_gui::ui::selection::SelectionAction;

        let tracks = [
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ];
        let transport = crate::mocks::MockTransport::new();

        apply_selection_action(SelectionAction::PlayAlbum, &transport, &tracks);
        assert_eq!(
            transport.recorded(),
            vec![crate::mocks::TransportIntent::PlayMany(
                TrackId("a.mp3".to_string()),
                vec![TrackId("b.mp3".to_string()), TrackId("c.mp3".to_string())]
            )],
            "Play album starts the album's first track with the rest queued behind it"
        );

        // An album with no resolvable tracks starts nothing.
        let transport = crate::mocks::MockTransport::new();
        apply_selection_action(SelectionAction::PlayAlbum, &transport, &[]);
        assert!(
            transport.recorded().is_empty(),
            "an album with no tracks starts nothing"
        );
    }

    #[test]
    fn test_selection_queue_appends_the_selections_tracks() {
        use riff_gui::ui::app::apply_selection_action;
        use riff_gui::ui::selection::SelectionAction;

        let tracks = [
            TrackId("a.mp3".to_string()),
            TrackId("b.mp3".to_string()),
            TrackId("c.mp3".to_string()),
        ];
        let transport = crate::mocks::MockTransport::new();
        apply_selection_action(SelectionAction::Queue, &transport, &tracks);
        assert_eq!(
            transport.recorded(),
            vec![
                crate::mocks::TransportIntent::AddToQueue(TrackId("a.mp3".to_string())),
                crate::mocks::TransportIntent::AddToQueue(TrackId("b.mp3".to_string())),
                crate::mocks::TransportIntent::AddToQueue(TrackId("c.mp3".to_string())),
            ],
            "Add to Queue appends every track of the selection, in order"
        );

        // An empty selection queues nothing.
        let transport = crate::mocks::MockTransport::new();
        apply_selection_action(SelectionAction::Queue, &transport, &[]);
        assert!(transport.recorded().is_empty(), "nothing to queue");
    }

    #[test]
    fn test_selection_panel_reports_add_to_queue() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{SelectionAction, SelectionPanel, show_selection_panel};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Tomorrow's Harvest"),
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &[],
                        tags: &[],
                        editor: None,
                        single: false,
                        queue: true,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The inspector's quick-action row offers both Play album and
        // Add to Queue; each reports its own action.
        assert!(
            harness.query_by_label("Add to Queue").is_some(),
            "the inspector renders the Add to Queue button"
        );
        harness.get_by_label("Add to Queue").click();
        harness.get_by_label("Play album").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::Queue, SelectionAction::PlayAlbum],
            "each button in the quick-action row reports its action"
        );
    }

    #[test]
    fn test_track_readout_play_plays_just_that_one_track() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::app::apply_selection_action;
        use riff_gui::ui::selection::{SelectionAction, SelectionPanel, show_selection_panel};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Magic Window"),
                        subtitle: Some("Boards of Canada"),
                        details: &[],
                        tags: &[],
                        editor: None,
                        single: true,
                        queue: true,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        // A single-track readout's primary action reads Play (never Play
        // album): the detail panel shows one track, so the button plays just
        // that one.
        assert!(
            harness.query_by_label("Play").is_some(),
            "the track readout's primary action is labeled Play"
        );
        assert!(
            harness.query_by_label("Play album").is_none(),
            "the track readout offers no Play album action"
        );
        harness.get_by_label("Play").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::PlayAlbum],
            "the track readout's Play reports the play action"
        );

        // The reported action starts exactly that one track — the batch the
        // panel resolves for a track readout is the single track, so nothing
        // queues behind it.
        let transport = crate::mocks::MockTransport::new();
        apply_selection_action(
            SelectionAction::PlayAlbum,
            &transport,
            &[TrackId("t1.mp3".to_string())],
        );
        assert_eq!(
            transport.recorded(),
            vec![crate::mocks::TransportIntent::PlayMany(
                TrackId("t1.mp3".to_string()),
                vec![],
            )],
            "a track readout's Play starts only that track"
        );
    }

    #[test]
    fn test_inspector_resolves_the_selection_and_gates_visibility() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};
        use riff_gui::ui::app::{InspectorKind, resolve_inspector};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");
        let (t1, _t2) = seed_geogaddi(&mut store);
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );

        // Album selection: the full readout — cover track, title, artist ·
        // year line, and the details grid.
        let library = riff_backend::app::state::LibrarySession {
            library_section: LibrarySection::Albums,
            browser_path: vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }],
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible, "an album selection shows the inspector");
        assert_eq!(content.kind, InspectorKind::Album);
        assert_eq!(
            content.title.as_deref(),
            Some("Geogaddi"),
            "the inspector resolves the album's title"
        );
        assert_eq!(
            content.subtitle.as_deref(),
            Some("Boards of Canada \u{b7} 2002"),
            "the inspector resolves the artist · year subtitle"
        );
        assert_eq!(
            content.art_track,
            Some(t1.id.clone()),
            "the inspector resolves the album's first track for its art"
        );
        assert_eq!(
            content.track_ids,
            vec![t1.id.clone(), _t2.id.clone()],
            "the inspector carries the album's track batch for Play/Queue"
        );

        // The details grid reads the store: artist, year, genre, the track
        // count with the known durations' total (03:25 = the one timed
        // track), and the play history's sum.
        let detail = |label: &str| {
            content
                .details
                .iter()
                .find(|d| d.label == label)
                .map(|d| d.value.clone())
        };
        assert_eq!(detail("Artist").as_deref(), Some("Boards of Canada"));
        assert!(
            detail("Released").is_none() && detail("Genre").is_none(),
            "the Released and Genre rows moved into the tag section (Issue 01)"
        );
        assert_eq!(
            detail("Tracks").as_deref(),
            Some("2 \u{b7} 03:25"),
            "the track count carries the known durations' total"
        );
        assert_eq!(detail("Plays").as_deref(), Some("3"));
        assert!(
            detail("Last played").is_some() && detail("Path").is_some(),
            "the last-played and path rows render"
        );
        assert!(
            detail("Path").is_some_and(|p| p.ends_with("Geogaddi")),
            "the path row names the album's folder"
        );

        // The album's tag section: fields the two tracks share resolve to
        // the value with both tracks' originals; the differing Title and
        // Track Number fields resolve to `(different)`, never a partial.
        let tag = |label: &str| {
            content
                .tags
                .iter()
                .find(|r| r.field.label() == label)
                .unwrap()
        };
        assert_eq!(
            content.tags.len(),
            7,
            "the tag section carries the seven fields"
        );
        for (label, value) in [
            ("Artist", "Boards of Canada"),
            ("Album", "Geogaddi"),
            ("Album Artist", "Boards of Canada"),
            ("Genre", "Electronic"),
            ("Year", "2002"),
        ] {
            let row = tag(label);
            assert_eq!(row.text, value, "the '{label}' row shows the shared value");
            assert_eq!(
                row.originals,
                vec![Some(value.to_string()); 2],
                "the '{label}' row carries both tracks' original values"
            );
        }
        assert_eq!(tag("Title").text, "(different)");
        assert_eq!(
            tag("Title").originals,
            vec![
                Some("Magic Window".to_string()),
                Some("Dawn Chorus".to_string()),
            ],
            "a differing title is a distinct value, never a partial"
        );
        assert_eq!(tag("Track Number").text, "(different)");
        assert_eq!(
            tag("Track Number").originals,
            vec![Some("1".to_string()), Some("2".to_string())],
            "a differing track number is a distinct value, never a partial"
        );

        // Artist selection: name, album count, cover, and the artist's
        // track batch.
        let library = riff_backend::app::state::LibrarySession {
            browser_path: vec![BrowserSelection::Artist("Boards of Canada".to_string())],
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible, "an artist selection shows the inspector");
        assert_eq!(content.kind, InspectorKind::Artist);
        assert_eq!(content.title.as_deref(), Some("Boards of Canada"));
        assert_eq!(content.subtitle.as_deref(), Some("1 album"));
        assert_eq!(
            content.track_ids,
            vec![t1.id.clone(), _t2.id.clone()],
            "the artist batch flattens the artist's albums in order"
        );

        // Genre selection: name and track count.
        let library = riff_backend::app::state::LibrarySession {
            browser_path: vec![BrowserSelection::Genre("Electronic".to_string())],
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible, "a genre selection shows the inspector");
        assert_eq!(content.kind, InspectorKind::Genre);
        assert_eq!(content.title.as_deref(), Some("Electronic"));
        assert_eq!(content.subtitle.as_deref(), Some("2 tracks"));
        assert!(
            content.tags.is_empty(),
            "a genre readout carries no tag section"
        );
        assert_eq!(
            content.track_ids,
            vec![t1.id.clone(), _t2.id.clone()],
            "the genre batch flattens the genre-scoped tracks in order"
        );

        // Track selection (single-list stages): the compact readout.
        let library = riff_backend::app::state::LibrarySession {
            selected_track: Some(t1.id.clone()),
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible, "a selected track shows the inspector");
        assert_eq!(content.kind, InspectorKind::Track);
        assert_eq!(content.title.as_deref(), Some("Magic Window"));
        assert_eq!(
            content.track_ids,
            vec![t1.id.clone()],
            "the track batch is the single track"
        );

        // A track single-clicked in the album's Tracks column wins over the
        // album in the drill-down path: the detail panel shows the track's
        // readout, not the album's.
        let library = riff_backend::app::state::LibrarySession {
            library_section: LibrarySection::Albums,
            browser_path: vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }],
            selected_track: Some(t1.id.clone()),
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(
            content.visible,
            "a track clicked in the Tracks column shows the inspector"
        );
        assert_eq!(
            content.kind,
            InspectorKind::Track,
            "the clicked track's readout wins over the album's"
        );
        assert_eq!(
            content.title.as_deref(),
            Some("Magic Window"),
            "the detail panel resolves the clicked track's title"
        );

        // No selection at all: the inspector is fully hidden.
        let empty = riff_backend::app::state::LibrarySession::default();
        assert!(
            !resolve_inspector(&mut views, &empty).visible,
            "no selection hides the inspector"
        );

        // A selection the store no longer carries (an album dropped by a
        // rescan) is hidden too: never stale art or wrong details.
        let gone = riff_backend::app::state::LibrarySession {
            browser_path: vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "A Few Old Tapes".to_string(),
            }],
            ..riff_backend::app::state::LibrarySession::default()
        };
        assert!(
            !resolve_inspector(&mut views, &gone).visible,
            "an unresolvable album hides the inspector"
        );
    }

    /// One Track of the "Geogaddi" album, carrying exactly the `ReplayGain`
    /// values the caller names.
    fn seed_gain_track(
        store: &mut riff_infra::store::SqliteStore,
        path: &str,
        title: &str,
        track_gain: Option<f32>,
        album_gain: Option<f32>,
    ) -> riff_backend::domain::Track {
        use riff_backend::app::store::LibraryMutationStore as _;
        use riff_backend::domain::TrackMetadata;

        let mut track = crate::test_utils::create_test_track(path, path);
        track.metadata = TrackMetadata {
            title: Some(title.to_string()),
            artist: Some("Boards of Canada".to_string()),
            album: Some("Geogaddi".to_string()),
            album_artist: Some("Boards of Canada".to_string()),
            track_number: Some(1),
            replaygain_track_gain: track_gain,
            replaygain_album_gain: album_gain,
            ..TrackMetadata::default()
        };
        store
            .apply_scan_batch(std::slice::from_ref(&track))
            .expect("the fixture commits");
        track
    }

    fn inspector_store(
        dir: &tempfile::TempDir,
    ) -> (
        riff_infra::store::SqliteStore,
        crossbeam_channel::Receiver<riff_backend::app::store::StoreChanged>,
    ) {
        let (changes_tx, changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(
            &dir.path().join("riff.sqlite3"),
            changes_tx,
        )
        .expect("opening a fresh store must work");
        (store, changes_rx)
    }

    #[test]
    fn test_track_readout_reports_the_replaygain_rows_the_file_carries() {
        use riff_backend::app::state::LibrarySession;
        use riff_gui::ui::app::resolve_inspector;

        let dir = tempfile::tempdir().unwrap();
        let (mut store, _rx) = inspector_store(&dir);
        let tagged = seed_gain_track(
            &mut store,
            "music/tagged.flac",
            "Tagged",
            Some(-6.54),
            Some(1.5),
        );
        let plain = seed_gain_track(&mut store, "music/plain.flac", "Plain", None, None);
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );
        let detail = |content: &riff_gui::ui::app::InspectorContent, label: &str| {
            content
                .details
                .iter()
                .find(|d| d.label == label)
                .map(|d| d.value.clone())
        };
        let mut readout = |id: riff_backend::domain::TrackId| {
            resolve_inspector(
                &mut views,
                &LibrarySession {
                    selected_track: Some(id),
                    ..LibrarySession::default()
                },
            )
        };

        let content = readout(tagged.id);
        // The signed two-decimal form is the row: `.2` is load-bearing, since
        // an f32 widened into the REAL column and narrowed back prints
        // `-6.540000057220459`.
        assert_eq!(
            detail(&content, "ReplayGain (track)").as_deref(),
            Some("-6.54 dB"),
            "the track gain row carries the sign and two decimals"
        );
        assert_eq!(
            detail(&content, "ReplayGain (album)").as_deref(),
            Some("+1.50 dB"),
            "a positive album gain reads with its `+`"
        );

        // A file with neither tag shows neither row: the DETAILS block is
        // exactly what it was before this feature.
        let content = readout(plain.id);
        assert_eq!(
            content
                .details
                .iter()
                .map(|d| d.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Plays", "Last played", "Path"],
            "an untagged track grows no row and prints no `(none)`"
        );
    }

    #[test]
    fn test_album_readout_reports_one_replaygain_row() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection, LibrarySession};
        use riff_gui::ui::app::resolve_inspector;

        let dir = tempfile::tempdir().unwrap();
        let (mut store, _rx) = inspector_store(&dir);
        seed_gain_track(
            &mut store,
            "music/a/01.flac",
            "One",
            Some(-6.54),
            Some(-7.12),
        );
        // A second track that carries the album value but no track value of
        // its own: the album readout must find the gain regardless of order.
        seed_gain_track(&mut store, "music/a/02.flac", "Two", None, Some(-7.12));
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );
        let mut detail = |label: &str| {
            let content = resolve_inspector(
                &mut views,
                &LibrarySession {
                    library_section: LibrarySection::Albums,
                    browser_path: vec![BrowserSelection::Album {
                        artist: "Boards of Canada".to_string(),
                        title: "Geogaddi".to_string(),
                    }],
                    ..LibrarySession::default()
                },
            );
            content
                .details
                .iter()
                .find(|d| d.label == label)
                .map(|d| d.value.clone())
        };

        assert_eq!(
            detail("ReplayGain").as_deref(),
            Some("-7.12 dB"),
            "the album readout shows the album value the tracks carry"
        );
        assert!(
            detail("ReplayGain (track)").is_none(),
            "the album readout reports the album gain only, never a track gain"
        );
    }

    #[test]
    fn test_inspector_tag_section_aggregates_each_field_across_the_album() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};
        use riff_gui::ui::app::{InspectorKind, resolve_inspector};
        use riff_gui::ui::selection::{TagField, TagRowState};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");
        let [t1, _t2, t3] = seed_varied_album(&mut store);
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );

        // The album readout resolves one tag row per field in the stable
        // modal order, with each field's rule: shared value, `(different)`
        // for differing values or a mix of present/missing, `(none)` when no
        // track carries it — missing is a distinct comparison value.
        let library = riff_backend::app::state::LibrarySession {
            library_section: LibrarySection::Albums,
            browser_path: vec![BrowserSelection::Album {
                artist: "Mixed".to_string(),
                title: "Varied".to_string(),
            }],
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible, "the varied album shows the inspector");
        assert_eq!(content.kind, InspectorKind::Album);
        let labels: Vec<&str> = content.tags.iter().map(|r| r.field.label()).collect();
        assert_eq!(
            labels,
            [
                "Title",
                "Artist",
                "Album",
                "Album Artist",
                "Genre",
                "Year",
                "Track Number",
            ],
            "the tag rows resolve in the seven-field modal order, Duration excluded"
        );
        let row = |field: TagField| content.tags.iter().find(|r| r.field == field).unwrap();
        assert_eq!(row(TagField::Title).state, TagRowState::Different);
        assert_eq!(
            row(TagField::Title).originals,
            vec![
                Some("Song A".to_string()),
                Some("Song B".to_string()),
                Some("Song C".to_string()),
            ],
            "differing titles stay distinct, never a majority guess"
        );
        assert_eq!(
            row(TagField::Artist).state,
            TagRowState::Different,
            "a mix of present and missing artists is different, never a partial value"
        );
        assert_eq!(
            row(TagField::Artist).originals,
            vec![
                Some("Artist One".to_string()),
                None,
                Some("Artist Three".to_string()),
            ],
            "a missing artist is a distinct comparison value"
        );
        assert_eq!(row(TagField::Album).state, TagRowState::Value);
        assert_eq!(row(TagField::Album).text, "Varied");
        assert_eq!(
            row(TagField::Album).originals,
            vec![Some("Varied".to_string()); 3],
            "an agreeing field carries the shared value and every original"
        );
        assert_eq!(row(TagField::AlbumArtist).state, TagRowState::Value);
        assert_eq!(
            row(TagField::AlbumArtist).text,
            "Mixed",
            "a field every track carries in agreement resolves to the value"
        );
        assert_eq!(row(TagField::Genre).state, TagRowState::None);
        assert_eq!(
            row(TagField::Genre).text,
            "(none)",
            "a field no track carries resolves to grey (none)"
        );
        assert_eq!(
            row(TagField::Genre).originals,
            vec![None, None, None],
            "the missing values are distinct per track"
        );
        assert_eq!(row(TagField::Year).state, TagRowState::Different);
        assert_eq!(row(TagField::TrackNumber).state, TagRowState::Different);

        // The album readout's detail grid keeps the non-tag facts; Released
        // and Genre rows are gone (their facts moved into the tag section).
        let detail = |label: &str| {
            content
                .details
                .iter()
                .find(|d| d.label == label)
                .map(|d| d.value.clone())
        };
        assert_eq!(detail("Artist").as_deref(), Some("Mixed"));
        assert!(detail("Released").is_none() && detail("Genre").is_none());
        assert!(detail("Tracks").is_some() && detail("Plays").is_some());
        assert!(detail("Last played").is_some() && detail("Path").is_some());

        // The track readout shows that track's value row-by-row, a missing
        // tag as grey `(none)`, and per-field originals the editor diff
        // bases (tickets 02/03) — the redundant Artist/Album/Genre rows are
        // gone, the Plays/Last played/Path rows stay.
        let library = riff_backend::app::state::LibrarySession {
            selected_track: Some(t3.id.clone()),
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible, "the selected track shows the inspector");
        assert_eq!(content.kind, InspectorKind::Track);
        let row = |field: TagField| content.tags.iter().find(|r| r.field == field).unwrap();
        assert_eq!(row(TagField::Title).state, TagRowState::Value);
        assert_eq!(row(TagField::Title).text, "Song C");
        assert_eq!(
            row(TagField::Title).originals,
            vec![Some("Song C".to_string())],
            "a single-track readout carries that track's value as its original"
        );
        assert_eq!(row(TagField::Genre).state, TagRowState::None);
        assert_eq!(
            row(TagField::Genre).text,
            "(none)",
            "a tag the track does not carry renders (none), not an em-dash"
        );
        assert_eq!(row(TagField::Genre).originals, vec![None]);
        assert_eq!(row(TagField::Year).state, TagRowState::Value);
        assert_eq!(row(TagField::Year).text, "2003");
        let detail = |label: &str| {
            content
                .details
                .iter()
                .find(|d| d.label == label)
                .map(|d| d.value.clone())
        };
        assert!(detail("Artist").is_none() && detail("Album").is_none());
        assert!(detail("Genre").is_none());
        assert!(detail("Plays").is_some() && detail("Last played").is_some());
        assert!(detail("Path").is_some());

        // Artist and Genre entity readouts carry no tag section; the artist
        // readout shows the album through the selection's name. A genre the
        // store no longer carries resolves hidden, never a stale readout.
        let library = riff_backend::app::state::LibrarySession {
            browser_path: vec![BrowserSelection::Artist("Mixed".to_string())],
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert!(content.visible);
        assert!(
            content.tags.is_empty(),
            "an artist readout has no tag section"
        );
        let library = riff_backend::app::state::LibrarySession {
            browser_path: vec![BrowserSelection::Genre("Nonexistent".to_string())],
            ..riff_backend::app::state::LibrarySession::default()
        };
        assert!(
            !resolve_inspector(&mut views, &library).visible,
            "a genre with no tracks resolves hidden"
        );

        // The selected-track-before-entity precedence keeps holding: the
        // track's rows render, the album never leaks into them.
        let library = riff_backend::app::state::LibrarySession {
            library_section: LibrarySection::Albums,
            browser_path: vec![BrowserSelection::Album {
                artist: "Mixed".to_string(),
                title: "Varied".to_string(),
            }],
            selected_track: Some(t1.id.clone()),
            ..riff_backend::app::state::LibrarySession::default()
        };
        let content = resolve_inspector(&mut views, &library);
        assert_eq!(content.kind, InspectorKind::Track);
        assert_eq!(
            content
                .tags
                .iter()
                .find(|r| r.field == TagField::Title)
                .unwrap()
                .text,
            "Song A",
            "the clicked track wins over the album in the drill-down path"
        );
    }

    // --- Flat-slot mapping (the Albums variant derives its listing from
    // the per-artist album tables) ----------------------------------------------

    use riff_gui::ui::browser::flat_slot;

    /// Two artists: the first with 2 albums, the second with 1. Prefix sums
    /// `[0, 2, 3]`, total 3.
    #[test]
    fn test_flat_slot_maps_ascending_indexes_front_to_back() {
        let counts = [0usize, 2, 3];
        assert_eq!(flat_slot(&counts, 0, false), Some((0, 0)));
        assert_eq!(flat_slot(&counts, 1, false), Some((0, 1)));
        assert_eq!(flat_slot(&counts, 2, false), Some((1, 0)));
        assert_eq!(flat_slot(&counts, 3, false), None, "past the end is None");
    }

    #[test]
    fn test_flat_slot_maps_descending_indexes_back_to_front() {
        // Z–A flips both the bucket order AND each bucket's contents, so
        // the last bucket's last item comes first.
        let counts = [0usize, 2, 3];
        assert_eq!(flat_slot(&counts, 0, true), Some((1, 0)));
        assert_eq!(flat_slot(&counts, 1, true), Some((0, 1)));
        assert_eq!(flat_slot(&counts, 2, true), Some((0, 0)));
        assert_eq!(flat_slot(&counts, 3, true), None);
    }

    // --- Keyboard operability (design-handoff issue 16) ---------------------
    //
    // Every interactive control in the three panes is reachable and operable
    // from the keyboard. `.focus()` sends the same accesskit Focus request a
    // screen reader sends — it lands only on widgets egui considers
    // focusable — and Enter/Space on the focused widget is egui's keyboard
    // primary click. The Tab walk across the panes is pinned separately by
    // the pane-order tests below.

    #[test]
    fn test_browser_controls_activate_by_keyboard() {
        use egui_kittest::kittest::Queryable;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<BrowserAction>| {
                    let mut fixture_item = provider(&items);
                    let column = BrowserColumn {
                        sort_desc: false,
                        show_sort: true,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column(
                        ui, &mut cache, &palette, column, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        // The A–Z sort control: focus it like a screen reader would, press
        // Enter, and the toggle action fires.
        harness.get_by_label("Sort Z to A").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness.state().contains(&BrowserAction::ToggleSort),
            "Enter on the focused sort control flips the sort: {:?}",
            harness.state()
        );

        // A row: Space activates it too — egui treats both as the primary
        // click for the focused widget.
        harness.get_by_label("Beta").focus();
        harness.run();
        harness.key_press(egui::Key::Space);
        harness.run();
        assert!(
            harness
                .state()
                .contains(&BrowserAction::Select("beta".to_string())),
            "Space on the focused row selects it: {:?}",
            harness.state()
        );
    }

    #[test]
    fn test_track_table_rows_activate_by_keyboard() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{DetailAction, DetailColumn, TrackRow, show_detail_column};
        use std::time::Duration;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tracks = vec![TrackRow {
            key: "t1".to_string(),
            title: "Magic Window".to_string(),
            plays: 3,
            duration: Some(Duration::from_secs(205)),
            favorite: false,
            selected: false,
            now_playing: false,
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<DetailAction>| {
                    let column = DetailColumn {
                        tracks: &tracks,
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column(ui, &mut cache, &palette, column, actions);
                },
                Vec::new(),
            );
        harness.run();

        // The row's title cell selects the track.
        harness.get_by_label("Magic Window").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness
                .state()
                .contains(&DetailAction::SelectTrack("t1".to_string())),
            "Enter on the focused track row selects it: {:?}",
            harness.state()
        );

        // The row's favorite control toggles the flag to its NEW value.
        harness.get_by_label("Add to Favorites").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness.state().contains(&DetailAction::SetFavorite {
                key: "t1".to_string(),
                favorite: true,
            }),
            "Enter on the focused favorite control toggles the flag: {:?}",
            harness.state()
        );
    }

    #[test]
    fn test_play_album_activates_by_keyboard() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{SelectionAction, SelectionPanel, show_selection_panel};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    let panel = SelectionPanel {
                        art: None,
                        title: Some("Tomorrow's Harvest"),
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &[],
                        tags: &[],
                        editor: None,
                        single: false,
                        queue: false,
                    };
                    show_selection_panel(ui, &mut cache, &palette, panel, actions);
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Play album").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![SelectionAction::PlayAlbum],
            "Enter on the focused Play album button starts the album"
        );
    }

    // --- Pane order & focus walk (design-handoff issue 16) ------------------
    //
    // egui walks focus in widget-creation order, so the composite fixture
    // below mirrors the app's creation order (titlebar → browser column →
    // selection panel → detail column; the selection panel is a right panel
    // and the CentralPanel must render last). Tab reaches the titlebar's
    // global search first among the panes' controls, then walks every control
    // of all three panes in a stable order; Shift+Tab walks the reverse chain
    // all the way home, and past the last widget Tab wraps back around — no
    // pane ever holds focus hostage.

    /// Tab until the titlebar search field has focus (bounded): the drag
    /// region and the titlebar controls precede it in creation order, so the
    /// search is no longer the very first stop of the walk.
    fn tab_until_search_focused(harness: &mut egui_kittest::Harness<'static, Vec<String>>) {
        use egui_kittest::kittest::Queryable;
        for _ in 0..32 {
            harness.key_press(egui::Key::Tab);
            harness.run();
            if harness
                .get_by_role(egui::accesskit::Role::TextInput)
                .is_focused()
            {
                return;
            }
        }
        panic!("Tab never reached the titlebar search field");
    }

    /// One composite frame: the real titlebar (search + chrome), browser
    /// column, selection panel, and detail column seams, in the app's
    /// creation order.
    fn walk_fixture() -> egui_kittest::Harness<'static, Vec<String>> {
        use riff_gui::ui::browser::BrowserColumn;
        use riff_gui::ui::chrome::{TitleBarContent, show_titlebar};
        use riff_gui::ui::detail::{AlbumHeader, Crumb, DetailColumn, TrackRow};
        use riff_gui::ui::selection::SelectionPanel;
        use riff_gui::ui::theme::TITLEBAR_H;
        use std::time::Duration;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut query = "abc".to_string();

        let crumbs = vec![
            Crumb {
                label: "Artists".to_string(),
            },
            Crumb {
                label: "Boards of Canada".to_string(),
            },
        ];
        let header = AlbumHeader {
            title: "Geogaddi".to_string(),
            subtitle: None,
        };
        let tracks = vec![TrackRow {
            key: "t1".to_string(),
            title: "Magic Window".to_string(),
            plays: 3,
            duration: Some(Duration::from_secs(205)),
            favorite: false,
            selected: false,
            now_playing: false,
        }];

        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 600.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, _state: &mut Vec<String>| {
                    // The titlebar (the real `show_titlebar` seam): its
                    // search field is the global search.
                    ui.allocate_ui(egui::vec2(1400.0, TITLEBAR_H), |ui| {
                        show_titlebar(
                            ui,
                            &mut cache,
                            &palette,
                            &TitleBarContent::default(),
                            &mut query,
                            &mut Vec::new(),
                        );
                    });

                    // The three panes side by side, in the app's panel show
                    // order: browser column (left), selection panel (right),
                    // detail column (the CentralPanel's content).
                    ui.horizontal(|ui| {
                        let mut browser_actions = Vec::new();
                        ui.allocate_ui(egui::vec2(320.0, 480.0), |ui| {
                            let mut fixture_item = provider(&items);
                            let column = BrowserColumn {
                                sort_desc: false,
                                show_sort: false,
                                total: items.len(),
                                item: &mut fixture_item,
                                virtualize: false,
                                empty_title: "",
                                empty_hint: "",
                            };
                            riff_gui::ui::browser::show_browser_column(
                                ui,
                                &mut cache,
                                &palette,
                                column,
                                &mut browser_actions,
                            );
                        });

                        let mut selection_actions = Vec::new();
                        ui.allocate_ui(egui::vec2(300.0, 480.0), |ui| {
                            let panel = SelectionPanel {
                                art: None,
                                title: Some("Tomorrow's Harvest"),
                                subtitle: None,
                                details: &[],
                                tags: &[],
                                editor: None,
                                single: false,
                                queue: false,
                            };
                            riff_gui::ui::selection::show_selection_panel(
                                ui,
                                &mut cache,
                                &palette,
                                panel,
                                &mut selection_actions,
                            );
                        });

                        let mut detail_actions = Vec::new();
                        ui.allocate_ui(egui::vec2(600.0, 480.0), |ui| {
                            let column = DetailColumn {
                                breadcrumb: &crumbs,
                                header: Some(&header),
                                tracks: &tracks,
                                ..DetailColumn::empty("", "")
                            };
                            riff_gui::ui::detail::show_detail_column(
                                ui,
                                &mut cache,
                                &palette,
                                column,
                                &mut detail_actions,
                            );
                        });
                    });
                },
                Vec::new(),
            )
    }

    #[test]
    fn test_tab_walks_the_titlebar_search_then_all_three_panes() {
        use egui_kittest::kittest::Queryable;

        let mut harness = walk_fixture();
        harness.run();

        // The titlebar search is reachable by Tab — the drag region and the
        // titlebar controls precede it in creation order — and it is the
        // anchor the no-trap clause comes home to.
        tab_until_search_focused(&mut harness);

        // From there Tab walks every control of the three panes in the
        // app's creation order: the browser rows, then the selection
        // panel's chip and Play album, then the detail column's crumbs,
        // header actions, and track row.
        for label in [
            "Clear search",
            "Alpha",
            "Beta",
            "Gamma",
            "Album",
            "Play album",
            "Artists",
            "Shuffle",
            "Play all",
            "Add to Favorites",
            "Magic Window",
        ] {
            harness.key_press(egui::Key::Tab);
            harness.run();
            assert!(
                harness.get_by_label(label).is_focused(),
                "Tab must move focus to '{label}' next"
            );
        }
    }

    #[test]
    fn test_shift_tab_walks_home_and_tab_wraps_with_no_focus_trap() {
        use egui_kittest::kittest::Queryable;

        let mut harness = walk_fixture();
        harness.run();
        tab_until_search_focused(&mut harness);

        // Walk to the last widget of the detail column: the search's clear
        // affordance, the three browser rows, the selection panel's chip
        // and Play album, then the detail column's crumb, header actions,
        // favorite, and the track row.
        for _ in [
            "Clear search",
            "Alpha",
            "Beta",
            "Gamma",
            "Album",
            "Play album",
            "Artists",
            "Shuffle",
            "Play all",
            "Add to Favorites",
            "Magic Window",
        ] {
            harness.key_press(egui::Key::Tab);
            harness.run();
        }
        assert!(
            harness.get_by_label("Magic Window").is_focused(),
            "the walk reaches the detail column's last control"
        );

        // Shift+Tab reverses the chain one widget at a time — backwards
        // through the detail column, the selection panel, the browser
        // rows, and the search's clear affordance — and lands on the
        // search.
        for label in [
            "Add to Favorites",
            "Play all",
            "Shuffle",
            "Artists",
            "Play album",
            "Album",
            "Gamma",
            "Beta",
            "Alpha",
            "Clear search",
        ] {
            harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
            harness.run();
            assert!(
                harness.get_by_label(label).is_focused(),
                "Shift+Tab must move focus back to '{label}'"
            );
        }
        // One more Shift+Tab lands on the search itself; the one after that
        // reaches the titlebar control preceding it — the keyboard is never
        // trapped inside the panes.
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        harness.run();
        assert!(
            harness
                .get_by_role(egui::accesskit::Role::TextInput)
                .is_focused(),
            "Shift+Tab from the clear affordance returns to the search"
        );
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        harness.run();
        assert!(
            harness.get_by_label("Theme").is_focused(),
            "Shift+Tab from the search reaches the titlebar Theme control"
        );

        // Forward past the last widget, Tab wraps back around — the
        // keyboard is never trapped. From the titlebar control, Tab returns
        // to the search, the forward walk reaches the end again, and Tab
        // past it keeps cycling so a bounded walk lands back on the search.
        harness.key_press(egui::Key::Tab);
        harness.run();
        assert!(
            harness
                .get_by_role(egui::accesskit::Role::TextInput)
                .is_focused(),
            "Tab from the titlebar control returns to the search"
        );
        for _ in [
            "Clear search",
            "Alpha",
            "Beta",
            "Gamma",
            "Album",
            "Play album",
            "Artists",
            "Shuffle",
            "Play all",
            "Add to Favorites",
            "Magic Window",
        ] {
            harness.key_press(egui::Key::Tab);
            harness.run();
        }
        assert!(
            harness.get_by_label("Magic Window").is_focused(),
            "the forward walk reaches the end again"
        );
        tab_until_search_focused(&mut harness);
    }
}

/// Whole-frame tests: the real app shell, driven through its `eframe::App`
/// interface by the headless kittest harness, over the mock port kit.
///
/// These pin the frame loop's contracts where they actually run instead of
/// restating them in comments or hand-replicating a slice of the render path.
/// The harness calls the app's `logic` and then its `ui` for every step, so one
/// `step()` is one frame per queued input event (and one frame when nothing is
/// queued).
///
/// Three seams are asserted through, each where it is the highest available:
/// the rendered output (the accessibility tree), the mock call records, and the
/// live sessions the app actually holds. The titlebar's status line is
/// *painted* rather than drawn as a widget, so it has no accessibility node —
/// the session slot the painter reads is that contract's observable seam, and
/// it is what the titlebar renders verbatim.
#[cfg(test)]
mod whole_frame_tests {
    use egui_kittest::kittest::Queryable;
    use riff_backend::app::MutexExt;
    use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
    use riff_backend::app::events::BackendEvents;
    use riff_backend::app::scan_service::ScanOutcome;
    use riff_backend::app::state::{LibrarySession, LibraryStatus, PlaybackSession, ViewMode};
    use riff_backend::app::store::StoreGeneration;
    use riff_backend::app::store::{
        LibraryMutationStore, LibraryQueryStore, PlaylistStore, SettingsStore,
    };
    use riff_backend::app::transport::Transport;
    use riff_backend::app::views::SessionViews;
    use riff_backend::domain::PlaylistId;
    use riff_backend::domain::TrackId;
    use riff_gui::ui::RiffApp;
    use riff_library::app::traits::{DecodedCover, RequestedSize};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use crate::mocks::{
        MockLibraryMutationStore, MockLibraryQueryStore, MockPlaylistStore, MockScans,
        MockSettingsStore, MockTagEdits, MockTransport, SettingsCall,
    };
    use crate::test_utils::{create_test_track_with_metadata, float_close};

    /// The [`Covers`] seam every [`Shell`] is wired with. Track requests are
    /// dropped exactly as `MockCovers` always did; folder requests are recorded
    /// by identity — the directory they were asked for — and `poll` serves
    /// nothing back. An empty poll means the app's texture cache never fills, so
    /// a frame keeps asking for every visible folder just as it would on a cold
    /// start, which is what a test of *what the row asks for* needs.
    struct ShellCovers(Arc<Mutex<Vec<(TrackId, RequestedSize)>>>);

    impl Covers for ShellCovers {
        fn request(&self, _track_id: TrackId, _path: PathBuf, _size: RequestedSize) {}

        fn request_folder(&self, folder: &Path, size: RequestedSize) {
            self.0
                .lock()
                .unwrap()
                .push((TrackId::from_path(folder), size));
        }

        fn poll(&self) -> Vec<(TrackId, RequestedSize, Option<DecodedCover>)> {
            Vec::new()
        }

        fn clear_cache(&self) {}

        fn poll_cache_clear(&self) -> Option<ClearCacheOutcome> {
            None
        }
    }

    /// The window size. Wide enough that the sidebar, the elastic stage, and the
    /// control bar all render without collapsing to zero.
    const WINDOW: egui::Vec2 = egui::vec2(1280.0, 800.0);

    /// A shell plus the handles a test needs to observe what a frame did.
    struct Shell {
        /// The harness owns the app; `state_mut()` reaches it between frames.
        harness: egui_kittest::Harness<'static, RiffApp>,
        /// The live playback session the app writes UI-owned fields back to.
        playback: Arc<Mutex<PlaybackSession>>,
        /// The live library session the app holds for the whole frame.
        library: Arc<Mutex<LibrarySession>>,
        /// The event inbox the app drains at the start of every frame.
        backend_events: Arc<Mutex<BackendEvents>>,
        /// The scripted scan front end the app polls every frame.
        scans: MockScans,
        /// Every settings mutation the app committed at a frame end.
        settings_calls: Arc<Mutex<Vec<SettingsCall>>>,
        /// Every folder cover request the app sent, in order.
        folder_covers: Arc<Mutex<Vec<(TrackId, RequestedSize)>>>,
    }

    /// Build a shell from fully-specified ports.
    ///
    /// `build_eframe` runs the app's `logic`/`ui` for its own warm-up frames
    /// (one to initialise AccessKit, one to settle), so by the time this
    /// returns the app has already rendered — and `first_frame` hydration has
    /// already happened. A test must therefore mutate state *after* the shell
    /// exists and then `step()` once per frame it wants to observe.
    fn build(
        transport: Box<dyn Transport>,
        scans: MockScans,
        settings_store: Box<dyn SettingsStore>,
        settings_calls: Arc<Mutex<Vec<SettingsCall>>>,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
    ) -> Shell {
        let playback = Arc::new(Mutex::new(PlaybackSession::default()));
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
        let folder_covers = Arc::new(Mutex::new(Vec::new()));

        let (app, _visibility_tx) = RiffApp::new_for_test(
            Arc::clone(&playback),
            Arc::clone(&library),
            transport,
            Box::new(scans.clone()),
            settings_store,
            playlist_store,
            library_mutations,
            views,
            Box::new(MockTagEdits),
            Box::new(ShellCovers(Arc::clone(&folder_covers))),
            Arc::clone(&backend_events),
        );

        let harness = egui_kittest::Harness::builder()
            .with_size(WINDOW)
            .build_eframe(|cc| {
                // The same font install `main.rs` performs, because the shell's
                // named families (`riff-inter-semibold` and friends) are
                // required by the render path — a frame panics without them.
                riff_gui::ui::fonts::configure_fonts(&cc.egui_ctx);
                app
            });

        Shell {
            harness,
            playback,
            library,
            backend_events,
            scans,
            settings_calls,
            folder_covers,
        }
    }

    /// A shell over the mock port kit: no Application Store, no audio device,
    /// no tray icon.
    fn mock_shell() -> Shell {
        let settings_calls = Arc::new(Mutex::new(Vec::new()));
        let scans = MockScans::default();
        build(
            Box::new(MockTransport::new()),
            scans,
            Box::new(MockSettingsStore::with_shared_calls(Arc::clone(
                &settings_calls,
            ))),
            settings_calls,
            Box::new(MockPlaylistStore::default()),
            Box::new(MockLibraryMutationStore::new()),
            SessionViews::new(
                Box::new(MockLibraryQueryStore::default()),
                Box::new(MockPlaylistStore::default()),
                StoreGeneration::new(),
                StoreGeneration::new(),
            ),
        )
    }

    /// A shell whose transport is a shared recording handle, so a test can
    /// assert exactly which play intents a frame emitted (the filtered
    /// header batch contract). The library query port is a pre-configured
    /// mock standing in for the Application Store.
    fn recording_transport_shell(mock: MockLibraryQueryStore) -> (Shell, Arc<MockTransport>) {
        let transport = Arc::new(MockTransport::new());
        #[derive(Clone)]
        struct SharedTransport(Arc<MockTransport>);
        impl Transport for SharedTransport {
            fn play(&self, track: riff_backend::domain::TrackId) {
                self.0.play(track);
            }
            fn pause(&self) {
                self.0.pause();
            }
            fn resume(&self) {
                self.0.resume();
            }
            fn stop(&self) {
                self.0.stop();
            }
            fn seek(&self, session: &PlaybackSession, secs: f32) {
                self.0.seek(session, secs);
            }
            fn set_volume(&self, session: &mut PlaybackSession, vol: f32) {
                self.0.set_volume(session, vol);
            }
            fn toggle_mute(&self, session: &mut PlaybackSession) {
                self.0.toggle_mute(session);
            }
            fn next(&self) {
                self.0.next();
            }
            fn previous(&self) {
                self.0.previous();
            }
            fn play_next(&self, track: riff_backend::domain::TrackId) {
                self.0.play_next(track);
            }
            fn add_to_queue(&self, track: riff_backend::domain::TrackId) {
                self.0.add_to_queue(track);
            }
            fn play_many(
                &self,
                first: riff_backend::domain::TrackId,
                rest: Vec<riff_backend::domain::TrackId>,
            ) {
                self.0.play_many(first, rest);
            }
            fn toggle_shuffle(&self, session: &mut PlaybackSession) {
                self.0.toggle_shuffle(session);
            }
            fn toggle_repeat(&self, session: &mut PlaybackSession) {
                self.0.toggle_repeat(session);
            }
            fn play_pause(&self, session: &PlaybackSession) {
                self.0.play_pause(session);
            }
        }

        let settings_calls = Arc::new(Mutex::new(Vec::new()));
        let shell = build(
            Box::new(SharedTransport(Arc::clone(&transport))),
            MockScans::default(),
            Box::new(MockSettingsStore::with_shared_calls(Arc::clone(
                &settings_calls,
            ))),
            settings_calls,
            Box::new(MockPlaylistStore::default()),
            Box::new(MockLibraryMutationStore::new()),
            SessionViews::new(
                Box::new(mock),
                Box::new(MockPlaylistStore::default()),
                StoreGeneration::new(),
                StoreGeneration::new(),
            ),
        );
        (shell, transport)
    }

    /// A shell over a real `SQLite` Application Store holding one playlist of
    /// three tracks, so the playlist view renders real reorderable rows.
    ///
    /// The temp dir comes back too: the store lives in it and must outlive the
    /// shell.
    fn store_shell() -> (
        Shell,
        tempfile::TempDir,
        PlaylistId,
        riff_infra::store::SqliteStore,
    ) {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("opening a fresh store must work");

        let mut track_ids = Vec::new();
        for (file, title) in [
            ("one.mp3", "Alpha"),
            ("two.mp3", "Beta"),
            ("three.mp3", "Gamma"),
        ] {
            let path = dir.path().join(file);
            std::fs::write(&path, b"fake audio bytes").expect("scratch file writes");
            let track = create_test_track_with_metadata(
                &path.to_string_lossy(),
                &path.to_string_lossy(),
                "Artist",
                title,
                "Album",
            );
            store
                .apply_scan_batch(std::slice::from_ref(&track))
                .expect("seed scan commits");
            track_ids.push(track.id);
        }

        // The playlist is committed before the shell exists, so its first read
        // already has rows rather than racing a refetch.
        let mut playlist_port = store.clone();
        let pid = playlist_port
            .create_playlist("Gym", &track_ids)
            .expect("creating the playlist works");
        let library_generation = store.library_generation();
        let playlist_generation = store.playlist_generation();

        let settings_calls = Arc::new(Mutex::new(Vec::new()));
        let shell = build(
            Box::new(MockTransport::new()),
            MockScans::default(),
            Box::new(MockSettingsStore::with_shared_calls(Arc::clone(
                &settings_calls,
            ))),
            settings_calls,
            Box::new(store.clone()),
            Box::new(store.clone()),
            SessionViews::new(
                Box::new(store.clone()),
                Box::new(store.clone()),
                library_generation,
                playlist_generation,
            ),
        );
        (shell, dir, pid, store)
    }

    /// Where a rendered node sits vertically, for order assertions. Uses the
    /// first match, so a label that also appears elsewhere cannot panic the
    /// query.
    fn y_of(harness: &egui_kittest::Harness<'static, RiffApp>, label: &str) -> f32 {
        harness
            .query_all_by_label(label)
            .next()
            .unwrap_or_else(|| panic!("no rendered node labelled {label}"))
            .rect()
            .center()
            .y
    }

    /// The harness itself, on a bare mock shell. This is the canary: if the
    /// eframe-backed harness stops driving the shell, every other test here
    /// fails for confusing reasons, and this one names the cause.
    #[test]
    fn real_frame_drives_the_app_shell() {
        let mut shell = mock_shell();
        shell.harness.step();

        assert!(
            shell.harness.query_all_by_label("Play").next().is_some(),
            "a real frame must render the transport controls"
        );
        assert!(
            shell.backend_events.lock_or_recover().events().is_empty(),
            "a frame drains the backend event inbox it rendered from"
        );
        assert!(
            shell.library.lock_or_recover().scan_status.is_none(),
            "a frame with nothing to report leaves the status line empty"
        );
        // The write-back covers only the five UI-owned fields, so the
        // engine-owned playback state is exactly what the session left.
        assert_eq!(
            shell.playback.lock_or_recover().playback_state,
            crate::domain::PlaybackState::Stopped,
            "a plain frame must not clobber the engine-owned playback state"
        );
    }

    /// Frame order, front half: the inbox is drained before anything renders,
    /// and a typed notice reaches the status line by the end of that frame.
    #[test]
    fn test_backend_notice_lands_on_the_status_line_through_a_real_frame() {
        let (notice_tx, notice_rx) = crossbeam_channel::unbounded::<String>();
        let mut shell = mock_shell();
        shell
            .backend_events
            .lock_or_recover()
            .subscribe_playback_notices(notice_rx);

        // A typed notice in the only shape the inbox produces one: the
        // coordinator's pre-formatted playback failure, which the drain stamps
        // with playback source and error severity on its way out.
        notice_tx
            .send("Playback error: the decoder fell over".to_string())
            .expect("the notice channel is live");

        assert!(
            shell.library.lock_or_recover().scan_status.is_none(),
            "nothing has been announced before the frame that drains it"
        );

        shell.harness.step();

        assert_eq!(
            shell.library.lock_or_recover().scan_status.as_deref(),
            Some("Playback error: the decoder fell over"),
            "one frame carries a drained typed notice to the status line"
        );
    }

    /// Frame order, service half: the app polls the scan service before it
    /// renders, and the polled outcome is on the status line by frame end.
    #[test]
    fn test_scan_outcome_lands_on_the_status_line_through_a_real_frame() {
        let root = std::path::PathBuf::from("/rift/music");
        let mut shell = mock_shell();

        // The outcome the worker would have published; the app polls it at the
        // start of the next frame.
        shell.scans.queue(ScanOutcome::Complete {
            path: root.clone(),
            total_files: 3,
        });

        shell.harness.step();

        let library = shell.library.lock_or_recover();
        assert_eq!(
            library.scan_status.as_deref(),
            Some("Scan complete: 3 tracks"),
            "the queued outcome is on the status line by frame end"
        );
        assert_eq!(
            library.library_paths.readiness(&root),
            LibraryStatus::Scanned(3),
            "and the per-root readiness the panes read moves with it"
        );
    }

    /// Frame order, back half: a preference change made in the sessions alone
    /// is durable by construction — the frame-end commit lands it, and it only
    /// lands when something actually drifted.
    #[test]
    fn test_preference_change_survives_a_full_frame() {
        let mut shell = mock_shell();

        // A session-only change, exactly as a settings handler leaves it: no
        // persist call anywhere, just a mutated session field.
        {
            let mut library = shell.library.lock_or_recover();
            library.scan_prefs.skip_hidden_files = !library.scan_prefs.skip_hidden_files;
        }

        assert!(
            shell.settings_calls.lock_or_recover().is_empty(),
            "nothing is persisted before the frame that sees the change"
        );

        shell.harness.step();
        assert_eq!(
            shell.settings_calls.lock_or_recover().as_slice(),
            [SettingsCall::Scalars],
            "the frame-end commit lands the drifted scalars in the store"
        );

        // The commit is a diff against the last-committed snapshot, so a frame
        // with no drift writes nothing at all.
        shell.harness.step();
        assert_eq!(
            shell.settings_calls.lock_or_recover().len(),
            1,
            "a frame with no drift does not re-save"
        );

        // ...and a second drift commits again, so the first save was the change
        // rather than a one-off.
        {
            let mut library = shell.library.lock_or_recover();
            library.scan_prefs.skip_hidden_files = !library.scan_prefs.skip_hidden_files;
        }
        shell.harness.step();
        assert_eq!(
            shell.settings_calls.lock_or_recover().len(),
            2,
            "a later drift commits too, so the diff keeps tracking the sessions"
        );
    }

    /// The frame-end write-back, pinned field by field. Every field the UI owns
    /// is asserted, so a future UI-owned field cannot silently miss the
    /// write-back and be clobbered by the next frame's snapshot.
    #[test]
    fn test_ui_owned_playback_fields_reach_the_live_session_at_frame_end() {
        let mut shell = mock_shell();
        shell.harness.step();
        let before = shell.playback.lock_or_recover().clone();

        // Drive each field through the control that owns it — the real widgets,
        // not a re-implementation of the write-back.
        shell.harness.get_by_label("Mute").click();
        shell.harness.step();

        shell.harness.get_by_label("Toggle shuffle").click();
        shell.harness.step();

        shell.harness.get_by_label("Cycle repeat mode").click();
        shell.harness.step();

        // The volume slider is one full-width hit region, so a click lands at
        // its centre: fraction 0.5. The default is not 0.5, so this is a change.
        shell.harness.get_by_label("Volume").click();
        shell.harness.step();

        // ReplayGain lives in the Settings modal's Playback pane, so open that
        // view and switch sections through their real controls. Each selection
        // is applied at the end of the frame that reported it, so the pane it
        // selects renders one frame later.
        shell.library.lock_or_recover().view_mode = ViewMode::Settings;
        shell.harness.step();
        shell.harness.get_by_label("Playback").click();
        shell.harness.step();
        shell.harness.step();
        shell.harness.get_by_label("ReplayGain").click();
        shell.harness.step();

        let live = shell.playback.lock_or_recover();
        assert_ne!(
            live.muted, before.muted,
            "mute reaches the live session at frame end (was {})",
            before.muted
        );
        assert_ne!(
            live.queue.shuffle, before.queue.shuffle,
            "shuffle reaches the live session at frame end (was {})",
            before.queue.shuffle
        );
        assert_ne!(
            live.queue.repeat, before.queue.repeat,
            "repeat reaches the live session at frame end (was {:?})",
            before.queue.repeat
        );
        assert!(
            !float_close(live.current_volume, before.current_volume),
            "volume reaches the live session at frame end (was {})",
            before.current_volume
        );
        assert_ne!(
            live.replaygain_enabled, before.replaygain_enabled,
            "replay-gain reaches the live session at frame end (was {})",
            before.replaygain_enabled
        );
    }

    /// The render contract the hand-replicated helper used to pin: a reorder
    /// committed through the real drop path is reflected by the next rendered
    /// frame, with no explicit invalidation anywhere. Here the drag, the
    /// commit, and the re-read all happen inside real frames.
    #[test]
    fn test_playlist_view_reflects_a_committed_reorder_on_the_next_real_frame() {
        let (mut shell, _dir, _pid, _store) = store_shell();
        shell.harness.step();

        // Open the playlist from the sidebar, exactly as a user does.
        shell.harness.get_by_label("Gym").click();
        shell.harness.step();

        assert!(
            y_of(&shell.harness, "Artist - Alpha") < y_of(&shell.harness, "Artist - Beta")
                && y_of(&shell.harness, "Artist - Beta") < y_of(&shell.harness, "Artist - Gamma"),
            "the first real frame renders the seeded order top to bottom"
        );

        // Drag the first row onto the third: press, move, release.
        let src = shell.harness.get_by_label("Artist - Alpha").rect();
        let dst = shell.harness.get_by_label("Artist - Gamma").rect();
        shell.harness.drag_at(src.center());
        shell.harness.step();
        shell.harness.hover_at(dst.center());
        shell.harness.step();
        shell.harness.drop_at(dst.center());
        shell.harness.step();

        assert!(
            y_of(&shell.harness, "Artist - Beta") < y_of(&shell.harness, "Artist - Gamma")
                && y_of(&shell.harness, "Artist - Gamma") < y_of(&shell.harness, "Artist - Alpha"),
            "the next real frame renders the committed order with no explicit invalidation"
        );
    }

    /// Every track listing carries the heart (the shared row's leading cell),
    /// and the All Tracks list is a track listing like any other: clicking a
    /// row's heart commits the flag through the Application Store, selects
    /// nothing, and the tapped row re-renders as a favorite on the next real
    /// frame with no explicit invalidation.
    #[test]
    fn test_flat_list_favorite_control_commits_through_the_store() {
        use riff_backend::domain::SmartPlaylistKind;

        let (mut shell, _dir, _pid, store) = store_shell();
        shell.harness.step();

        assert_eq!(
            shell.harness.query_all_by_label("Add to Favorites").count(),
            3,
            "every row of the flat list carries the favorite control"
        );

        shell
            .harness
            .query_all_by_label("Add to Favorites")
            .next()
            .expect("a heart to click")
            .click();

        // The click commits inside the frame that processes it (and that frame
        // has already painted its rows); the next frame reads the committed
        // flag through the projections, so no explicit invalidation is needed.
        shell.harness.step();
        shell.harness.step();

        assert!(
            shell.library.lock_or_recover().selected_track.is_none(),
            "a heart click must not select the row it lives in"
        );
        assert_eq!(
            store
                .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
                .expect("the Favorites smart list reads")
                .len(),
            1,
            "the heart committed the flag through the store"
        );
        assert_eq!(
            shell
                .harness
                .query_all_by_label("Remove from Favorites")
                .count(),
            1,
            "the tapped row re-renders with the committed flag"
        );
    }

    // --- Query filters the section's columns (issue 04) ---------------------
    //
    // The single-list search stage and its global "no matches" gate are gone:
    // a query leaves the open section's columns in place, and each column
    // lists only the entities that match. These tests drive the real app
    // shell over a mock query port, asserting what the user sees (which rows
    // render per section and query).

    /// An album fixture (one album per row of the Albums root).
    fn album(title: &str, artist: &str, year: u32) -> riff_backend::domain::Album {
        riff_backend::domain::Album {
            title: title.to_string(),
            artist: artist.to_string(),
            tracks: Vec::new(),
            year: Some(year),
            genre: None,
        }
    }

    /// An artist fixture (one artist per row of the Artists root). `albums`
    /// carries the artist's album keys: in the hit listing these are the
    /// hit-album keys (exactly as the store's `hit_artists_page` answers
    /// them); in the full browsing listing they are all the artist's album
    /// keys (the mock's `artist_albums` serves the same list for every
    /// artist, so a clearing fixture names its albums through this count).
    fn artist(name: &str, album_keys: &[&str]) -> riff_backend::domain::Artist {
        riff_backend::domain::Artist {
            name: name.to_string(),
            albums: album_keys.iter().map(|k| k.to_string()).collect(),
        }
    }

    /// A track fixture for the album-hit-tracks / album-tracks reads.
    fn track(id: &str, title: &str) -> riff_backend::domain::Track {
        crate::test_utils::create_test_track_with_metadata(
            id,
            &format!("/music/{id}"),
            "Artist",
            title,
            "Album",
        )
    }

    /// The Albums root under a query lists only hit albums, in canonical hit
    /// order, with the sort control hidden — the query no longer takes over
    /// the stage as a flat track list.
    #[test]
    fn test_albums_root_under_a_query_lists_hit_albums_with_sort_hidden() {
        use riff_backend::app::state::LibrarySection;

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            hit_albums: vec![
                album("Geogaddi", "Boards of Canada", 2002),
                album("Tri Repetae", "Autechre", 1995),
            ],
            matching_searches: vec!["geo".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "geo".to_string();
        }
        shell.harness.step();

        // The Albums column stays in place (not the flat search stage), and
        // its rows are the hit albums, not the full album list.
        assert!(
            shell
                .harness
                .query_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
                .is_some(),
            "a hit album's title renders in the Albums column"
        );
        assert!(
            shell
                .harness
                .query_by_label("Tri Repetae (Autechre \u{b7} 1995)")
                .is_some(),
            "a second hit album renders in the Albums column"
        );
        assert!(
            shell.harness.query_by_label("Sort Z to A").is_none()
                && shell.harness.query_by_label("Sort A to Z").is_none(),
            "the sort control is hidden under a query (canonical hit order)"
        );
    }

    /// The Albums root's empty copy is query-aware: a query matching no
    /// album explains itself, it never shows the full empty-library copy.
    #[test]
    fn test_albums_root_empty_copy_is_query_aware() {
        use riff_backend::app::state::LibrarySection;

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            // The library HAS albums — the query just matches none of them.
            hit_albums: Vec::new(),
            matching_searches: vec!["geo".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "zzz".to_string();
        }
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("No matching albums").is_some(),
            "the Albums column explains a filtered-to-empty list"
        );
        assert!(
            shell
                .harness
                .query_all_by_label("Nothing in your library matches 'zzz'.")
                .next()
                .is_some(),
            "the empty hint names the query"
        );
    }

    /// Clearing the query restores the full Albums column (the general
    /// "clear restores" contract, pinned on the Albums root).
    #[test]
    fn test_clearing_the_query_restores_the_full_albums_column() {
        use riff_backend::app::state::LibrarySection;

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            // The Albums root's flat listing is the paged flat windowed query
            // over every album (paginate-browse-columns issue 03): the mock
            // seeds `paged_albums` in canonical browsing order.
            paged_albums: vec![
                album("Geogaddi", "Boards of Canada", 2002),
                album("Homework", "Boards of Canada", 1997),
            ],
            hit_albums: vec![album("Geogaddi", "Boards of Canada", 2002)],
            matching_searches: vec!["geo".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "geo".to_string();
        }
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Homework (Boards of Canada \u{b7} 1997)")
                .is_none(),
            "under the query only the hit album shows"
        );

        {
            let mut library = shell.library.lock_or_recover();
            library.search_query.clear();
        }
        shell.harness.step();

        assert!(
            shell
                .harness
                .query_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
                .is_some()
                && shell
                    .harness
                    .query_by_label("Homework (Boards of Canada \u{b7} 1997)")
                    .is_some(),
            "clearing the query restores the full Albums column"
        );
    }

    /// The Artists root under a query lists only hit artists, each row
    /// carrying its hit-album count, with the A–Z sort control hidden.
    #[test]
    fn test_artists_root_under_a_query_lists_hit_artists() {
        use riff_backend::app::state::LibrarySection;

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            artists: vec![
                artist("Boards of Canada", &["geogaddi", "campfire"]),
                artist("Portishead", &["dummy"]),
            ],
            hit_artists: vec![
                artist("Boards of Canada", &["geogaddi"]),
                artist("Portishead", &["dummy"]),
            ],
            matching_searches: vec!["geo".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Artists;
            library.search_query = "geo".to_string();
        }
        shell.harness.step();

        // The Artists column stays in place and lists the hit artists.
        assert!(
            shell
                .harness
                .query_by_label("Boards of Canada (1 album)")
                .is_some(),
            "a hit artist's row renders with its hit-album count"
        );
        assert!(
            shell
                .harness
                .query_by_label("Portishead (1 album)")
                .is_some(),
            "every hit artist renders under the query"
        );
        assert!(
            shell.harness.query_by_label("Sort Z to A").is_none()
                && shell.harness.query_by_label("Sort A to Z").is_none(),
            "the Artists root's sort control is hidden under a query"
        );
    }

    /// A name-hit artist drills to ALL its albums — the downward name-hit
    /// expansion — even the albums with no hit of their own.
    #[test]
    fn test_name_hit_artist_drills_to_all_its_albums() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            artists: vec![artist("Boards of Canada", &["geogaddi", "campfire"])],
            albums: vec![
                album("Geogaddi", "Boards of Canada", 2002),
                album("Campfire Headphase", "Boards of Canada", 2005),
            ],
            hit_artists: vec![artist("Boards of Canada", &["geogaddi"])],
            album_name_hits: vec!["Boards of Canada - Geogaddi".to_string()],
            matching_searches: vec!["boards".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Artists;
            library.search_query = "boards".to_string();
            // Drill into the artist.
            library.browser_path = vec![BrowserSelection::Artist("Boards of Canada".to_string())];
        }
        shell.harness.step();

        assert!(
            shell
                .harness
                .query_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
                .is_some(),
            "the name-hit artist's hit album drills"
        );
        assert!(
            shell
                .harness
                .query_by_label("Campfire Headphase (Boards of Canada \u{b7} 2005)")
                .is_some(),
            "a name-hit artist expands into ALL its albums, including non-hits"
        );
    }

    /// A track-hit artist's drill shows only its hit albums.
    #[test]
    fn test_track_hit_artist_drill_shows_only_hit_albums() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            artists: vec![artist("Portishead", &["dummy", "third"])],
            albums: vec![
                album("Dummy", "Portishead", 1994),
                album("Third", "Portishead", 2021),
            ],
            hit_artists: vec![artist("Portishead", &["dummy"])],
            // Only Dummy carries the matching member track.
            album_name_hits: vec!["Portishead - Dummy".to_string()],
            matching_searches: vec!["roads".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Artists;
            library.search_query = "roads".to_string();
            library.browser_path = vec![BrowserSelection::Artist("Portishead".to_string())];
        }
        shell.harness.step();

        assert!(
            shell
                .harness
                .query_by_label("Dummy (Portishead \u{b7} 1994)")
                .is_some(),
            "a track-hit artist's hit album drills"
        );
        assert!(
            shell
                .harness
                .query_by_label("Third (Portishead \u{b7} 2021)")
                .is_none(),
            "a track-hit artist's non-hit albums stay out of the drill"
        );
    }

    /// The album drill under a query shows only the album's hit tracks; a
    /// name-hit album opens its full track list instead.
    #[test]
    fn test_album_drill_shows_hit_tracks_unless_the_album_is_a_name_hit() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};

        // First: a track-hit album (a member track matches) drills its hit
        // tracks only.
        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            albums: vec![album("Geogaddi", "Boards of Canada", 2002)],
            album_tracks: vec![
                track("g1.mp3", "Ready Let's Go"),
                track("g2.mp3", "Music Is Math"),
            ],
            album_hit_tracks: vec![track("g2.mp3", "Music Is Math")],
            matching_searches: vec!["math".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "math".to_string();
            library.browser_path = vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }];
        }
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Music Is Math").is_some(),
            "the matching track drills in its album"
        );
        assert!(
            shell.harness.query_by_label("Ready Let's Go").is_none(),
            "a track-hit album's non-matching tracks stay out"
        );

        // Second: the same album as a NAME hit opens its full track list.
        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            albums: vec![album("Geogaddi", "Boards of Canada", 2002)],
            album_tracks: vec![
                track("g1.mp3", "Ready Let's Go"),
                track("g2.mp3", "Music Is Math"),
            ],
            album_hit_tracks: vec![track("g2.mp3", "Music Is Math")],
            album_name_hits: vec!["Boards of Canada - Geogaddi".to_string()],
            matching_searches: vec!["geogaddi".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "geogaddi".to_string();
            library.browser_path = vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }];
        }
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Ready Let's Go").is_some()
                && shell.harness.query_by_label("Music Is Math").is_some(),
            "a name-hit album opens its full track list (never a dead-end)"
        );
    }

    /// The Albums root renders no genre chip row — the search filters the
    /// hit albums without any genre-chip surface (issue 04 keeps search
    /// display-independent).
    #[test]
    fn test_albums_root_renders_no_genre_chips_under_a_query() {
        use riff_backend::app::state::LibrarySection;
        use riff_backend::domain::GenreCount;

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            artists: vec![artist("Boards of Canada", &["geogaddi"])],
            albums: vec![album("Geogaddi", "Boards of Canada", 2002)],
            hit_albums: vec![album("Geogaddi", "Boards of Canada", 2002)],
            genre_counts: vec![GenreCount {
                genre: "Electronic".to_string(),
                tracks: 42,
            }],
            matching_searches: vec!["geo".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "geo".to_string();
        }
        shell.harness.step();

        // The genre chip row does not render, yet the hit listing works.
        assert!(
            shell
                .harness
                .query_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
                .is_some(),
            "the search still lists hit albums"
        );
        assert!(
            shell.harness.query_by_label("All genres").is_none(),
            "no genre chip row renders on the Albums root"
        );
    }

    /// The Tracks column's Play all under a query starts exactly the shown
    /// (filtered) rows — pinned through the recorded transport.
    #[test]
    fn test_album_header_play_all_starts_the_shown_filtered_rows() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};

        let (mut shell, transport) = recording_transport_shell(MockLibraryQueryStore {
            albums: vec![album("Geogaddi", "Boards of Canada", 2002)],
            album_tracks: vec![
                track("g1.mp3", "Ready Let's Go"),
                track("g2.mp3", "Music Is Math"),
            ],
            album_hit_tracks: vec![track("g2.mp3", "Music Is Math")],
            matching_searches: vec!["math".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            library.search_query = "math".to_string();
            library.browser_path = vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }];
        }
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Music Is Math").is_some(),
            "the filtered Tracks column shows only the hit track"
        );
        assert!(
            shell.harness.query_by_label("Ready Let's Go").is_none(),
            "the filtered Tracks column hides the non-hit track"
        );

        shell.harness.get_by_label("Play all").click();
        shell.harness.step();
        shell.harness.step();

        let intents = transport.recorded();
        let batches: Vec<_> = intents
            .iter()
            .filter(|i| matches!(i, crate::mocks::TransportIntent::PlayMany(..)))
            .collect();
        assert_eq!(
            batches.len(),
            1,
            "Play all emits exactly one batch: {intents:?}"
        );
        assert!(
            matches!(
                batches[0],
                crate::mocks::TransportIntent::PlayMany(first, rest)
                    if first == &riff_backend::domain::TrackId("g2.mp3".to_string())
                        && rest.is_empty()
            ),
            "the batch starts the FIRST shown (filtered) row and nothing else: {intents:?}"
        );
    }

    /// The Folders tree stays browsable under a query: it shows the pruned
    /// tree, never the flat track list.
    #[test]
    fn test_folders_stay_pruned_under_a_query() {
        use riff_backend::app::state::BrowseMode;
        use std::path::PathBuf;

        let root = PathBuf::from("/music");
        let child = root.join("boards");
        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            folder_has_audio: true,
            folder_search_match: true,
            folder_children: vec![child.clone()],
            folder_direct_tracks: vec![track("t1.mp3", "Ready Let's Go")],
            matching_searches: vec!["geo".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.browse_mode = BrowseMode::Folders;
            library
                .library_paths
                .hydrate(&riff_backend::app::store::Settings {
                    library_paths: vec![root.clone()],
                    ..Default::default()
                });
            library.search_query = "geo".to_string();
        }
        shell.harness.step();

        // Open the root node: folder nodes collapse by default, so the
        // child only renders after the root node opens.
        shell.harness.get_by_label("music").click();
        shell.harness.step();

        // The Folders tree renders (its roots), not a flat search listing.
        assert!(
            shell.harness.query_by_label("music").is_some(),
            "the folder root stays in the tree under a query"
        );
        assert!(
            shell.harness.query_by_label("boards").is_some(),
            "a matching child folder stays pruned INTO the tree"
        );
    }

    /// The folder rows' cover wiring: each visible node asks the cover seam for
    /// its OWN directory at the shared thumbnail box, and nothing else asks on
    /// its behalf.
    #[test]
    fn test_folder_rows_request_their_own_directory_cover() {
        use riff_backend::app::state::BrowseMode;
        use riff_gui::ui::app::COVER_THUMB;

        let root = PathBuf::from("/music");
        let child = root.join("boards");
        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            folder_has_audio: true,
            folder_children: vec![child.clone()],
            folder_direct_tracks: vec![track("t1.mp3", "Ready Let's Go")],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.browse_mode = BrowseMode::Folders;
            library
                .library_paths
                .hydrate(&riff_backend::app::store::Settings {
                    library_paths: vec![root.clone()],
                    ..Default::default()
                });
        }

        /// Drain the recording and return the distinct folder identities that
        /// asked, in first-seen order, asserting each ask is at the thumbnail box.
        ///
        /// Under the in-flight marker this reports *what became visible since the
        /// last drain* rather than everything on screen: a row that already has an
        /// outstanding answer is not asked for again, which is the whole point.
        fn folder_nodes(shell: &Shell) -> Vec<String> {
            let mut asks = shell.folder_covers.lock().unwrap();
            let mut seen: Vec<String> = Vec::new();
            for (id, size) in asks.drain(..) {
                assert_eq!(
                    size, COVER_THUMB,
                    "a folder row asks at the existing thumbnail box, not a box of its own"
                );
                if !seen.contains(&id.0) {
                    seen.push(id.0);
                }
            }
            seen
        }

        shell.harness.step();
        assert_eq!(
            folder_nodes(&shell),
            vec![root.to_string_lossy().to_string()],
            "a collapsed tree asks for the root's own cover and nothing else"
        );

        // Opening the root makes its child a visible node, so the child asks.
        // The root does **not** ask again: its first answer is still outstanding
        // (`ShellCovers` never serves one), and a repaint is no reason to
        // re-enqueue a request onto the unbounded channel.
        shell.harness.get_by_label("music").click();
        shell.harness.step();
        assert_eq!(
            folder_nodes(&shell),
            vec![child.to_string_lossy().to_string()],
            "only the newly visible folder asks; the root's outstanding request is not re-sent"
        );

        // And a frame that changes nothing sends nothing.
        shell.harness.step();
        shell.harness.step();
        assert_eq!(
            folder_nodes(&shell),
            Vec::<String>::new(),
            "two more repaints of an unchanged tree issue no new cover requests at all"
        );
    }

    /// All Tracks under a query filters the flat list exactly as today — the
    /// query-aware flat path, not a section takeover.
    #[test]
    fn test_all_tracks_filters_the_flat_list_under_a_query() {
        use riff_backend::app::state::LibrarySection;

        let (mut shell, _transport) = recording_transport_shell(MockLibraryQueryStore {
            flat: vec![track("a.mp3", "Alpha"), track("b.mp3", "Beta")],
            search: vec![track("a.mp3", "Alpha")],
            matching_searches: vec!["alp".to_string()],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::AllTracks;
            library.search_query = "alp".to_string();
        }
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Artist - Alpha").is_some(),
            "the matching track shows in the flat list under the query"
        );
        assert!(
            shell.harness.query_by_label("Artist - Beta").is_none(),
            "non-matching tracks stay out of the flat list under the query"
        );
    }

    // --- Browse pagination (paginate-browse-columns 02) --------------------
    //
    // The browse columns render one window in hand: only the visible window
    // reaches the store, and a refetch happens only when a visible row walks
    // past it. The shared query handle lets a test assert exactly which
    // windows the pane fetched.

    use riff_backend::app::errors::StoreError;
    use riff_backend::app::store::SortDirection;
    use riff_backend::domain::{Album, Artist, GenreCount, SmartPlaylistKind, Track};

    /// [`LibraryQueryStore`] view over one shared [`MockLibraryQueryStore`]
    /// behind a mutex: the shell takes ownership of the port, the test keeps
    /// the handle for recordings and post-wire configuration changes.
    #[derive(Clone)]
    struct SharedQueries(Arc<Mutex<MockLibraryQueryStore>>);

    impl LibraryQueryStore for SharedQueries {
        fn get_track(
            &self,
            id: &riff_backend::domain::TrackId,
        ) -> Result<Option<Track>, StoreError> {
            self.0.lock().unwrap().get_track(id)
        }
        fn metadata_version(&self) -> Result<u32, StoreError> {
            Ok(riff_persistence::track::METADATA_VERSION)
        }
        fn tracks_page(
            &self,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0.lock().unwrap().tracks_page(offset, limit)
        }
        fn library_counts(&self) -> Result<riff_backend::app::store::LibraryCounts, StoreError> {
            self.0.lock().unwrap().library_counts()
        }
        fn all_track_ids(&self) -> Result<Vec<riff_backend::domain::TrackId>, StoreError> {
            self.0.lock().unwrap().all_track_ids()
        }
        fn search_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0.lock().unwrap().search_page(query, offset, limit)
        }
        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            self.0.lock().unwrap().all_artists()
        }
        fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError> {
            self.0.lock().unwrap().artist_albums(artist)
        }
        fn album_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_tracks(album_artist, album_title)
        }
        fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, StoreError> {
            self.0.lock().unwrap().folder_has_audio(folder)
        }
        fn folder_has_search_match(
            &self,
            folder: &std::path::Path,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.0
                .lock()
                .unwrap()
                .folder_has_search_match(folder, query)
        }
        fn track_ids_in_folder_tree(
            &self,
            folder: &std::path::Path,
        ) -> Result<Vec<riff_backend::domain::TrackId>, StoreError> {
            self.0.lock().unwrap().track_ids_in_folder_tree(folder)
        }
        fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().tracks_in_folder(folder)
        }
        fn folder_track_count(&self, folder: &std::path::Path) -> Result<usize, StoreError> {
            self.0.lock().unwrap().folder_track_count(folder)
        }
        fn last_full_scan(
            &self,
        ) -> Result<Option<riff_backend::app::store::FullScanSummary>, StoreError> {
            self.0.lock().unwrap().last_full_scan()
        }
        fn subdirs_with_audio(
            &self,
            folder: &std::path::Path,
        ) -> Result<Vec<std::path::PathBuf>, StoreError> {
            self.0.lock().unwrap().subdirs_with_audio(folder)
        }
        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().smart_playlist(kind, limit)
        }
        fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
            self.0.lock().unwrap().smart_list_counts()
        }
        fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
            self.0.lock().unwrap().genre_counts()
        }
        fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError> {
            self.0.lock().unwrap().artists_in_genre(genre)
        }
        fn artist_albums_in_genre(
            &self,
            artist: &str,
            genre: &str,
        ) -> Result<Vec<Album>, StoreError> {
            self.0.lock().unwrap().artist_albums_in_genre(artist, genre)
        }
        fn album_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_tracks_in_genre(album_artist, album_title, genre)
        }
        fn hit_albums_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.0.lock().unwrap().hit_albums_page(query, offset, limit)
        }
        fn hit_artists_page(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .hit_artists_page(query, offset, limit)
        }
        fn album_hit_tracks(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_hit_tracks(album_artist, album_title, query)
        }
        fn album_is_name_hit(
            &self,
            album_artist: &str,
            album_title: &str,
            query: &str,
        ) -> Result<bool, StoreError> {
            self.0
                .lock()
                .unwrap()
                .album_is_name_hit(album_artist, album_title, query)
        }
        fn hit_albums_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Album>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .hit_albums_in_genre(genre, query, offset, limit)
        }
        fn hit_artists_in_genre(
            &self,
            genre: &str,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .hit_artists_in_genre(genre, query, offset, limit)
        }
        fn album_hit_tracks_in_genre(
            &self,
            album_artist: &str,
            album_title: &str,
            genre: &str,
            query: &str,
        ) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().album_hit_tracks_in_genre(
                album_artist,
                album_title,
                genre,
                query,
            )
        }
        fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError> {
            self.0.lock().unwrap().hit_genre_counts(query)
        }
        fn artists_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .artists_page(direction, offset, limit)
        }
        fn albums_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.0.lock().unwrap().albums_page(direction, offset, limit)
        }
        fn genres_page(
            &self,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<GenreCount>, StoreError> {
            self.0.lock().unwrap().genres_page(direction, offset, limit)
        }
        fn artists_in_genre_page(
            &self,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Artist>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .artists_in_genre_page(genre, direction, offset, limit)
        }
        fn artist_albums_in_genre_page(
            &self,
            artist: &str,
            genre: &str,
            direction: SortDirection,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Album>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .artist_albums_in_genre_page(artist, genre, direction, offset, limit)
        }
    }

    /// A shell over a shared query mock, returning the shared handle so the
    /// test can assert exactly which store reads the pane performed.
    fn shared_query_shell(
        mock: MockLibraryQueryStore,
    ) -> (Shell, Arc<Mutex<MockLibraryQueryStore>>) {
        let shared = Arc::new(Mutex::new(mock));
        let settings_calls = Arc::new(Mutex::new(Vec::new()));
        let shell = build(
            Box::new(MockTransport::new()),
            MockScans::default(),
            Box::new(MockSettingsStore::with_shared_calls(Arc::clone(
                &settings_calls,
            ))),
            settings_calls,
            Box::new(MockPlaylistStore::default()),
            Box::new(MockLibraryMutationStore::new()),
            SessionViews::new(
                Box::new(SharedQueries(Arc::clone(&shared))),
                Box::new(MockPlaylistStore::default()),
                StoreGeneration::new(),
                StoreGeneration::new(),
            ),
        );
        (shell, shared)
    }

    /// The Artists column renders one window in hand: the first frame fetches
    /// only the visible window (never the whole 120-row library), and a
    /// refetch happens only when a visible row walks past the window in hand.
    #[test]
    fn test_artists_column_fetches_only_the_window_in_hand() {
        use egui_kittest::kittest::Queryable;
        use riff_backend::app::state::LibrarySection;

        let artists: Vec<riff_backend::domain::Artist> = (0..120)
            .map(|i| artist(&format!("Artist {i:03}"), &["debut"]))
            .collect();
        let (mut shell, shared) = shared_query_shell(MockLibraryQueryStore {
            artists,
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Artists;
        }
        shell.harness.step();

        // The pane served the window in hand: no window beyond 0 reached the
        // store (the whole 120-row list never materializes), whatever the
        // shell's warm-up frames already cached.
        let windows = shared.lock().unwrap().window_calls();
        assert!(
            windows.iter().all(|(o, l)| *o == 0 && *l == 50),
            "the first frame fetches only the visible window, never beyond it: {windows:?}"
        );
        assert!(
            shell
                .harness
                .query_by_label("Artist 000 (1 album)")
                .is_some(),
            "the first window's rows render"
        );
        assert!(
            shell
                .harness
                .query_by_label("Artist 099 (1 album)")
                .is_none(),
            "rows beyond the window in hand never materialize on first view"
        );

        // Scrolling a visible row down walks it past the window in hand: the
        // pane refetches exactly the next window and renders its rows. Each
        // AccessKit scroll step re-queries a row still in the rendered tree
        // (rows cull as they leave the viewport) until a row of the next
        // window becomes visible.
        let target = "Artist 060 (1 album)";
        let probes = [
            "Artist 000 (1 album)",
            "Artist 004 (1 album)",
            "Artist 008 (1 album)",
            "Artist 012 (1 album)",
            "Artist 016 (1 album)",
            "Artist 020 (1 album)",
            "Artist 024 (1 album)",
            "Artist 028 (1 album)",
            "Artist 032 (1 album)",
            "Artist 036 (1 album)",
            "Artist 040 (1 album)",
            "Artist 044 (1 album)",
            "Artist 048 (1 album)",
            "Artist 052 (1 album)",
            "Artist 056 (1 album)",
        ];
        let mut guard = 0;
        while shell.harness.query_by_label(target).is_none() && guard < 80 {
            let row = probes
                .iter()
                .find_map(|label| shell.harness.query_by_label(label))
                .expect("a rendered artist row to scroll");
            row.scroll_down();
            shell.harness.step();
            guard += 1;
        }
        assert!(
            guard < 80,
            "scrolling must reach the next window within the step budget"
        );

        // Sanity: the scroll moved — the first row is culled from the tree.
        assert!(
            shell
                .harness
                .query_by_label("Artist 000 (1 album)")
                .is_none(),
            "scrolling must cull the first row from the rendered tree"
        );
        let windows = shared.lock().unwrap().window_calls();
        assert!(
            windows.contains(&(50, 50)),
            "scrolling past the window in hand refetches exactly the next window: {windows:?}"
        );
        assert!(
            shell
                .harness
                .query_by_label("Artist 060 (1 album)")
                .is_some(),
            "the refetched window's rows render after scrolling"
        );
    }

    // --- Scroll Memory (scroll-memory spec) --------------------------------
    //
    // The UI seam for per-Section scroll restore: scroll a Section's root
    // list, switch Sections, and assert the same rows are visible on return
    // (user story 22). Assertions observe visible rows and labels only —
    // never egui scroll state or the Scroll Memory internals.
    //
    // Exactly two tests live here, one per protocol shape, because only a real
    // frame proves egui APPLIED the offset the module handed out. The decisions
    // themselves — per-slot independence, staleness, the drill reset epoch, and
    // which slots carry selection bookkeeping — are asserted directly at the
    // module's own interface in `crates/riff-gui/tests/scroll_memory_tests.rs`.
    // The three further frame tests that used to live here restated those
    // decisions through painted row positions and were replaced, not layered.

    /// A mock library whose flat list holds `count` rows, so the All Tracks
    /// list actually scrolls.
    fn flat_mock(count: usize) -> MockLibraryQueryStore {
        let flat: Vec<_> = (0..count)
            .map(|i| {
                create_test_track_with_metadata(
                    &format!("path/{i:03}.mp3"),
                    &format!("path/{i:03}.mp3"),
                    "One",
                    &format!("Track {i:03}"),
                    "Album",
                )
            })
            .collect();
        MockLibraryQueryStore {
            // The flat list serves from `flat`; a search query serves from
            // `search` — keep them in step for the search-reset tests.
            search: flat.clone(),
            flat,
            ..Default::default()
        }
    }

    #[test]
    fn test_all_tracks_restores_its_scroll_position_across_section_switches() {
        use riff_backend::app::state::LibrarySection;

        let mut mock = flat_mock(500);
        // The sidebar's counts come from `library_counts` (a separate read
        // from the flat list's own count), so seed them so the section rows
        // speak honest labels: "All Tracks (500)" / "Artists (0)".
        mock.library_counts = riff_backend::app::store::LibraryCounts {
            tracks: 500,
            artists: 0,
            albums: 0,
            genres: 0,
        };
        let (mut shell, _transport) = recording_transport_shell(mock);
        shell.harness.step();

        // A fresh run starts the All Tracks list at its first row.
        assert!(
            shell.harness.query_by_label("One - Track 000").is_some(),
            "a fresh app starts every list at the top"
        );

        // Scroll until a deep row holds the viewport (AccessKit scroll steps
        // of ~100px each; the anchor re-probes rows as earlier ones cull).
        let mut guard = 0;
        while shell.harness.query_by_label("One - Track 020").is_none() && guard < 40 {
            let anchor = [
                "One - Track 005",
                "One - Track 010",
                "One - Track 015",
                "One - Track 020",
                "One - Track 025",
                "One - Track 030",
            ]
            .into_iter()
            .find_map(|label| shell.harness.query_by_label(label))
            .expect("a rendered row to scroll");
            anchor.scroll_down();
            shell.harness.step();
            guard += 1;
        }
        assert!(
            shell.harness.query_by_label("One - Track 020").is_some(),
            "scrolling must reach a row deep in the list (guard {guard})"
        );
        assert!(
            shell.harness.query_by_label("One - Track 000").is_none(),
            "scrolling leaves the first row off-screen"
        );
        for _ in 0..3 {
            shell.harness.step();
        }
        let deep_y = y_of(&shell.harness, "One - Track 020");

        // Switch to another Section — the Artists root (empty) renders while
        // away — then return to All Tracks. (The sidebar rows carry their
        // counts in the accessible label, e.g. "Artists (0)".)
        shell.harness.get_by_label("Artists (0)").click();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("No artists yet").is_some(),
            "the Artists root renders while away from All Tracks"
        );
        let library = shell.library.lock_or_recover();
        assert_eq!(
            library.library_section,
            LibrarySection::Artists,
            "the sidebar click lands on the Artists Section"
        );
        drop(library);
        shell.harness.get_by_label("All Tracks (500)").click();
        shell.harness.step();
        shell.harness.step();

        // The same rows are visible again, at the same place — exact, not an
        // approximation (user story 18).
        assert!(
            shell.harness.query_by_label("One - Track 020").is_some(),
            "the remembered rows are visible again on return"
        );
        assert!(
            (y_of(&shell.harness, "One - Track 020") - deep_y).abs() < 1.0,
            "the restored position is exact: the row sits where it was left \
             (deep_y {deep_y:.1}, now {:.1})",
            y_of(&shell.harness, "One - Track 020")
        );
        assert!(
            shell.harness.query_by_label("One - Track 000").is_none(),
            "returning restores the saved offset, not the top"
        );

        // The scrollbar still spans the full list after a restore: scrolling
        // keeps moving until a row far below the restored window appears
        // (user story 19) — culled rows prove the full content is intact.
        let mut guard = 0;
        while shell.harness.query_by_label("One - Track 120").is_none() && guard < 60 {
            let anchor = [
                "One - Track 020",
                "One - Track 030",
                "One - Track 040",
                "One - Track 050",
                "One - Track 060",
                "One - Track 070",
                "One - Track 080",
                "One - Track 090",
                "One - Track 100",
                "One - Track 110",
            ]
            .into_iter()
            .find_map(|label| shell.harness.query_by_label(label))
            .expect("a rendered row to scroll");
            anchor.scroll_down();
            shell.harness.step();
            guard += 1;
        }
        assert!(
            shell.harness.query_by_label("One - Track 120").is_some(),
            "the full list length is reachable after a restore (guard {guard})"
        );
    }

    /// A mock library with populated Artists and Genres roots, so the three
    /// browser root lists actually scroll and restore independently. Rows
    /// serve in canonical order from the canned arrays, with honest sidebar
    /// counts.
    fn browse_mock() -> MockLibraryQueryStore {
        use riff_backend::domain::GenreCount;
        MockLibraryQueryStore {
            artists: (0..200)
                .map(|i| riff_backend::domain::Artist {
                    name: format!("Artist {i:03}"),
                    albums: vec![format!("album-{i:03}")],
                })
                .collect(),
            paged_genres: (0..200)
                .map(|i| GenreCount {
                    genre: format!("Genre {i:03}"),
                    tracks: 1,
                })
                .collect(),
            library_counts: riff_backend::app::store::LibraryCounts {
                tracks: 0,
                artists: 200,
                albums: 0,
                genres: 200,
            },
            ..Default::default()
        }
    }

    /// Scroll the harness's visible list downward until `target` renders or
    /// the step budget runs out. The anchor re-probes rows every 10 slots,
    /// so one is always in the ~15-row visible window (the concurrent
    /// artists-root test precedent).
    fn scroll_list_until(
        harness: &mut egui_kittest::Harness<'static, RiffApp>,
        labels: &[&str],
        target: &str,
        budget: usize,
    ) {
        let mut guard = 0;
        while harness.query_by_label(target).is_none() && guard < budget {
            let anchor = labels
                .iter()
                .find_map(|label| harness.query_by_label(label))
                .expect("a rendered row to scroll");
            anchor.scroll_down();
            harness.step();
            guard += 1;
        }
        assert!(
            harness.query_by_label(target).is_some(),
            "scrolling must reach {target} within the step budget (guard {guard})"
        );
    }

    /// A mock with 200 albums (kept for the artist we drill into) and 200
    /// tracks inside each album, so the drill Albums and the Tracks column
    /// actually scroll.
    fn drill_mock() -> MockLibraryQueryStore {
        let mut mock = browse_mock();
        mock.albums = (0..200)
            .map(|i| riff_backend::domain::Album {
                artist: "Artist 040".to_string(),
                title: format!("Album {i:03}"),
                year: None,
                genre: None,
                tracks: vec![riff_backend::domain::TrackId(format!("t-{i:03}"))],
            })
            .collect();
        mock.album_tracks = (0..200)
            .map(|i| {
                create_test_track_with_metadata(
                    &format!("t-{i:03}"),
                    &format!("t-{i:03}.mp3"),
                    "Artist 040",
                    &format!("T-{i:03}"),
                    "Album",
                )
            })
            .collect();
        mock
    }

    #[test]
    fn test_drill_columns_start_at_the_top_on_selection_change() {
        let (mut shell, _transport) = recording_transport_shell(drill_mock());
        shell.harness.step();
        for _ in 0..3 {
            shell.harness.step();
        }

        // Drill into a row near the top of the root (no root scrolling, so
        // the neighbouring rows stay comfortably on screen for re-selection).
        shell.harness.get_by_label("Artists (200)").click();
        shell.harness.step();
        shell.harness.get_by_label("Artist 005 (1 album)").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Album 000 (Artist 040)")
                .is_some(),
            "a freshly drilled Albums column starts at its first album"
        );

        // Scroll the Albums drill deep, so a stale offset would be visible.
        scroll_list_until(
            &mut shell.harness,
            &[
                "Album 010 (Artist 040)",
                "Album 020 (Artist 040)",
                "Album 030 (Artist 040)",
                "Album 040 (Artist 040)",
                "Album 050 (Artist 040)",
                "Album 060 (Artist 040)",
            ],
            "Album 030 (Artist 040)",
            40,
        );
        for _ in 0..3 {
            shell.harness.step();
        }
        assert!(
            shell
                .harness
                .query_by_label("Album 000 (Artist 040)")
                .is_none(),
            "scrolling the Albums drill culls its first album"
        );

        // Selecting a DIFFERENT artist starts that artist's Albums at its
        // first album — the previous offset is gone.
        shell.harness.get_by_label("Artist 010 (1 album)").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Album 000 (Artist 040)")
                .is_some(),
            "selecting a different artist starts the new Albums column at the top"
        );
        assert!(
            shell
                .harness
                .query_by_label("Album 030 (Artist 040)")
                .is_none(),
            "the previous artist's offset never leaks into the new selection"
        );

        // Re-selecting the SAME artist also starts the drill at the top
        // (user story 10) — there is no per-selection memory.
        shell.harness.get_by_label("Artist 010 (1 album)").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Album 000 (Artist 040)")
                .is_some(),
            "re-selecting the same artist still starts its Albums column at the top"
        );
        assert!(
            shell
                .harness
                .query_by_label("Album 030 (Artist 040)")
                .is_none(),
            "a re-selection never resumes an old drill offset"
        );
    }

    // --- One prompt and confirmation contract (component-layer issue 14) -----
    //
    // Playlist create/rename and the Clear Library confirmation share one
    // interaction contract: the name field takes keyboard focus when the prompt
    // opens, Enter confirms, and Cancel, Escape, or a click outside dismisses —
    // with every dismissal route leaving the durable store untouched. Prompt
    // state and the store write stay where they are: in `RiffApp`.

    /// Open the inline "New Playlist" name prompt through a real frame.
    fn open_playlist_create_prompt(shell: &mut Shell) {
        shell.harness.get_by_label("New Playlist").click();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Create").is_some(),
            "the + control opens the inline name prompt"
        );
    }

    /// Send characters to the prompt's name field and prove it is the widget
    /// that holds the first focus: Tab from wherever the pointer left the
    /// focus walks to the confirm control, so Shift+Tab returns to the field.
    fn focus_and_type(shell: &mut Shell, text: &str) {
        let harness = &mut shell.harness;
        harness.key_press(egui::Key::Tab);
        harness.step();
        assert!(
            harness.get_by_label("Create").is_focused(),
            "Tab from the prompt's first focus lands on Create"
        );
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
        harness.step();
        harness
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .find(|node| node.is_focused())
            .expect("Shift+Tab returns to the prompt's name field, so the draft types there")
            .type_text(text);
        harness.step();
    }

    fn playlist_names(store: &riff_infra::store::SqliteStore) -> Vec<String> {
        store
            .load_playlists()
            .expect("the playlist list reads")
            .into_iter()
            .map(|playlist| playlist.name)
            .collect()
    }

    #[test]
    fn test_playlist_create_prompt_takes_focus_and_enter_creates_through_a_real_frame() {
        let (mut shell, _dir, _pid, store) = store_shell();
        shell.harness.step();
        open_playlist_create_prompt(&mut shell);

        focus_and_type(&mut shell, "Roadwork");
        shell.harness.key_press(egui::Key::Enter);
        shell.harness.step();
        shell.harness.step();

        assert!(
            playlist_names(&store).contains(&"Roadwork".to_string()),
            "the focused field took the typed name and Enter committed it, got {:?}",
            playlist_names(&store)
        );
        assert!(
            shell.harness.query_by_label("Create").is_none(),
            "confirming closes the prompt"
        );
    }

    #[test]
    fn test_escape_dismisses_the_prompt_and_writes_nothing() {
        let (mut shell, _dir, _pid, store) = store_shell();
        shell.harness.step();
        open_playlist_create_prompt(&mut shell);

        focus_and_type(&mut shell, "Doomed");
        shell.harness.key_press(egui::Key::Escape);
        shell.harness.step();
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Create").is_none(),
            "Escape dismisses the prompt"
        );
        assert_eq!(
            playlist_names(&store),
            vec!["Gym".to_string()],
            "dismissal discards the draft: the store gained nothing"
        );
    }

    #[test]
    fn test_cancel_and_an_outside_click_dismiss_the_prompt_and_write_nothing() {
        let (mut shell, _dir, _pid, store) = store_shell();
        shell.harness.step();

        open_playlist_create_prompt(&mut shell);
        focus_and_type(&mut shell, "Doomed");
        shell.harness.get_by_label("Cancel").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Create").is_none(),
            "Cancel dismisses the prompt"
        );

        open_playlist_create_prompt(&mut shell);
        focus_and_type(&mut shell, "AlsoDoomed");
        shell.harness.get_by_label("Gym").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Create").is_none(),
            "interacting outside the prompt dismisses it"
        );

        assert_eq!(
            playlist_names(&store),
            vec!["Gym".to_string()],
            "neither dismissal route wrote to the Playlist Store"
        );
    }

    #[test]
    fn test_asking_to_clear_the_library_opens_the_confirmation_and_writes_nothing() {
        use riff_gui::ui::prompts::CLEAR_LIBRARY_CONFIRM_COPY;

        let (mut shell, _dir, _pid, store) = store_shell();
        shell.harness.step();
        shell.library.lock_or_recover().view_mode = ViewMode::Settings;
        shell.harness.step();

        shell.harness.get_by_label("Clear Library").click();
        shell.harness.step();
        shell.harness.step();

        assert!(
            shell
                .harness
                .query_by_label(CLEAR_LIBRARY_CONFIRM_COPY)
                .is_some(),
            "the destructive action asks first, and its copy says what the wipe keeps"
        );
        assert_eq!(
            store.all_track_ids().expect("the listing reads").len(),
            3,
            "asking for confirmation commits nothing to the collection"
        );
        assert_eq!(
            playlist_names(&store),
            vec!["Gym".to_string()],
            "and it touches no Playlist"
        );
    }

    /// The confirmation reports both outcomes and applies neither: the host
    /// owns the wipe. Its composed Confirm/Cancel path is the ticket-14
    /// follow-up — at the shell's 800px height the confirmation's own row falls
    /// outside the Settings panel's visible area.
    #[test]
    fn test_clear_library_confirmation_reports_confirm_and_cancel() {
        use riff_gui::ui::prompts::{PromptOutcome, clear_library_confirm};

        let palette = riff_gui::ui::theme::Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 88.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, outcomes: &mut Vec<PromptOutcome>| {
                    let mut cache = riff_gui::ui::icons::IconCache::new();
                    outcomes.extend(clear_library_confirm(ui, &mut cache, &palette));
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Cancel").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![PromptOutcome::Cancel],
            "Cancel is reported, not applied"
        );

        harness.state_mut().clear();
        harness.get_by_label("Confirm").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![PromptOutcome::Confirm],
            "the destructive Confirm is reported, not applied"
        );
    }

    /// The Thumbnail clear is destructive and irreversible *in the moment* — there
    /// is no eviction, so this is the only reclaim — so it asks before it acts, and
    /// the widget itself never deletes: it only reports what the listener chose.
    /// Same contract as the Clear Library row above, and the reason the Settings
    /// action can set a flag rather than touch the disk.
    #[test]
    fn test_clear_thumbnail_cache_confirmation_reports_confirm_and_cancel() {
        use riff_gui::ui::prompts::{
            CLEAR_THUMBNAIL_CACHE_CONFIRM_COPY, PromptOutcome, clear_thumbnail_cache_confirm,
        };

        let palette = riff_gui::ui::theme::Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(420.0, 88.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, outcomes: &mut Vec<PromptOutcome>| {
                    let mut cache = riff_gui::ui::icons::IconCache::new();
                    outcomes.extend(clear_thumbnail_cache_confirm(ui, &mut cache, &palette));
                },
                Vec::new(),
            );
        harness.run();

        assert_ne!(
            CLEAR_THUMBNAIL_CACHE_CONFIRM_COPY,
            riff_gui::ui::prompts::CLEAR_LIBRARY_CONFIRM_COPY,
            "the two wipes sit one gap apart in the same footer, so their copy must not read alike"
        );

        harness.get_by_label("Cancel").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![PromptOutcome::Cancel],
            "Cancel is reported, not applied"
        );

        harness.state_mut().clear();
        harness.get_by_label("Confirm").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![PromptOutcome::Confirm],
            "the destructive Confirm is reported, not applied"
        );
    }

    /// The Settings frame still composes the Library Path flow's entry point —
    /// and reaching it is not itself an indexing request: the native dialog
    /// (macOS/Windows) or the text row (Linux) only *offers* a root, and
    /// registering one starts nothing.
    #[test]
    fn test_the_settings_frame_offers_the_library_picker_without_indexing() {
        let mut shell = mock_shell();
        shell.harness.step();
        shell.library.lock_or_recover().view_mode = ViewMode::Settings;
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Add Library").is_some(),
            "the Library section still presents the picker's entry point"
        );
        assert!(
            shell.scans.requested_paths().is_empty(),
            "rendering the Settings frame requests no scan"
        );
        assert!(
            shell
                .library
                .lock_or_recover()
                .library_paths
                .paths()
                .is_empty(),
            "and registers no root"
        );
    }

    /// The Inline Tag Editor stays the host's: a draft opened through `RiffApp`'s
    /// own entry renders the shared field and save-bar treatment inside the
    /// composed inspector, and a changed selection discards it — the widget never
    /// outlives the readout it was opened on (component-layer issue 17).
    #[test]
    fn test_the_host_owns_the_inline_editors_life_inside_the_composed_inspector() {
        use riff_gui::ui::app::{InspectorContent, InspectorKind};
        use riff_gui::ui::selection::{TagField, TagRow, TagRowState};

        let (mut shell, _dir, _pid, store) = store_shell();
        shell.harness.step();

        let ids = store.all_track_ids().expect("the listing reads");
        let (track_id, other_id) = (ids[0].clone(), ids[1].clone());
        shell.library.lock().unwrap().selected_track = Some(track_id.clone());
        shell.harness.step();

        let content = InspectorContent {
            visible: true,
            kind: InspectorKind::Track,
            title: Some("Alpha".to_string()),
            subtitle: Some("Artist".to_string()),
            art_track: Some(track_id.clone()),
            track_ids: vec![track_id.clone()],
            details: Vec::new(),
            tags: vec![TagRow {
                field: TagField::Title,
                state: TagRowState::Value,
                text: "Alpha".to_string(),
                originals: vec![Some("Alpha".to_string())],
            }],
        };
        shell.harness.state_mut().open_inline_draft(&content);
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Save").is_some(),
            "the host's draft renders the shared Save bar in the composed inspector"
        );
        assert!(
            shell.harness.query_by_label("Cancel").is_some(),
            "and the bar carries its neutral partner"
        );

        // A changed selection: the controller drops the draft this frame, so a
        // half-typed edit can never leak onto the next Track's readout. The
        // inspector itself stays — the editor going is the lifecycle, not the
        // panel disappearing.
        shell.library.lock().unwrap().selected_track = Some(other_id);
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Save").is_none(),
            "the draft died with the selection it was opened on"
        );
        assert!(
            shell.harness.query_by_label("SELECTION").is_some(),
            "while the inspector keeps rendering the new readout"
        );
    }
}

// --- Context menus: shared item conventions and typed intents (issue 15) -----
//
// The production Track and list menus render through `ui::menu` and report
// typed intents; every Transport, Playlist Store, selection, and Inline Tag
// Editor effect happens afterwards, in the host's own mapping.

#[cfg(test)]
mod context_menu_ui_tests {
    use egui_kittest::kittest::Queryable;
    use riff_backend::app::store::PlaylistStore;
    use riff_backend::domain::{PlaylistId, TrackId};
    use riff_gui::ui::app::{
        InlineTagEditor, TrackMenuEffects, apply_list_menu_intent, apply_track_menu_intent,
    };
    use riff_gui::ui::menu::{self, Item, ItemState, ListMenuIntent, TrackMenu, TrackMenuIntent};
    use riff_gui::ui::selection::DraftKind;
    use riff_gui::ui::theme::Palette;

    use crate::mocks::{MockTagEdits, MockTransport, TransportIntent};
    use crate::test_utils::create_test_track_with_metadata;

    fn track(file: &str) -> riff_backend::domain::Track {
        create_test_track_with_metadata(file, file, "Artist", "Title", "Album")
    }

    /// Render the production track menu and click `labels` in order, returning
    /// the intents it emitted.
    fn track_menu_intents(props: &TrackMenu<'_>, clicks: &[&str]) -> Vec<TrackMenuIntent> {
        let palette = Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 240.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, intents: &mut Vec<TrackMenuIntent>| {
                    menu::track_menu(ui, &palette, props, intents);
                },
                Vec::new(),
            );
        harness.run();
        for label in clicks {
            harness.get_by_label(label).click();
            harness.run();
        }
        std::mem::take(harness.state_mut())
    }

    fn list_menu_intents(clicks: &[&str]) -> Vec<ListMenuIntent> {
        let palette = Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 120.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, intents: &mut Vec<ListMenuIntent>| {
                    menu::list_menu(ui, &palette, intents);
                },
                Vec::new(),
            );
        harness.run();
        for label in clicks {
            harness.get_by_label(label).click();
            harness.run();
        }
        std::mem::take(harness.state_mut())
    }

    #[test]
    fn test_track_menu_reports_one_typed_intent_per_action() {
        let pid = PlaylistId::new("pl-1");
        let options = [(pid.clone(), "Gym".to_string())];
        let props = TrackMenu {
            playable: true,
            editable: true,
            playlists: &options,
            remove_from_playlist: true,
        };

        assert_eq!(
            track_menu_intents(&props, &["Play"]),
            vec![TrackMenuIntent::Play],
            "Play reports the choice and nothing else"
        );
        assert_eq!(
            track_menu_intents(&props, &["Play Next"]),
            vec![TrackMenuIntent::PlayNext]
        );
        assert_eq!(
            track_menu_intents(&props, &["Add to Queue"]),
            vec![TrackMenuIntent::AddToQueue]
        );
        assert_eq!(
            track_menu_intents(&props, &["Remove from Playlist"]),
            vec![TrackMenuIntent::RemoveFromPlaylist]
        );
        assert_eq!(
            track_menu_intents(&props, &["Edit Tags"]),
            vec![TrackMenuIntent::EditTags]
        );
        assert!(
            track_menu_intents(&props, &[]).is_empty(),
            "rendering a menu reports no intent at all"
        );
    }

    /// The menu's content policy is unchanged by the extraction: a Track whose
    /// file is gone offers neither playback actions nor the tag editor.
    #[test]
    fn test_unplayable_track_menu_keeps_only_the_playlist_actions() {
        let pid = PlaylistId::new("pl-1");
        let options = [(pid.clone(), "Gym".to_string())];
        let props = TrackMenu {
            playable: false,
            editable: false,
            playlists: &options,
            remove_from_playlist: true,
        };
        let palette = Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 120.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, intents: &mut Vec<TrackMenuIntent>| {
                    menu::track_menu(ui, &palette, &props, intents);
                },
                Vec::new(),
            );
        harness.run();

        for absent in ["Play", "Play Next", "Add to Queue", "Edit Tags"] {
            assert!(
                harness.query_by_label(absent).is_none(),
                "a Track with no file offers no {absent}"
            );
        }
        assert!(
            harness.query_by_label("Remove from Playlist").is_some(),
            "the entry is still there to take out"
        );
    }

    #[test]
    fn test_list_menu_reports_its_three_intents() {
        assert_eq!(list_menu_intents(&["Play"]), vec![ListMenuIntent::Play]);
        assert_eq!(
            list_menu_intents(&["Play Next"]),
            vec![ListMenuIntent::PlayNext]
        );
        assert_eq!(
            list_menu_intents(&["Append to Queue"]),
            vec![ListMenuIntent::AppendToQueue]
        );
    }

    /// Enabled, disabled, and destructive rows come from one owner: a disabled
    /// row answers no click, and only the destructive row carries the error ink.
    #[test]
    fn test_menu_item_states_share_one_treatment() {
        let palette = Palette::dark();
        let render = |state: ItemState| {
            egui_kittest::Harness::builder()
                .with_size(egui::vec2(220.0, 64.0))
                .with_pixels_per_point(1.0)
                .build_ui_state(
                    move |ui, activated: &mut bool| {
                        *activated = menu::item(
                            ui,
                            &palette,
                            &Item {
                                label: "Remove from Playlist",
                                state,
                                tooltip: None,
                            },
                        );
                    },
                    false,
                )
        };

        let count = |image: &image::RgbaImage, color: egui::Color32| {
            image
                .pixels()
                .filter(|p| {
                    p.0[0].abs_diff(color.r()) <= 4
                        && p.0[1].abs_diff(color.g()) <= 4
                        && p.0[2].abs_diff(color.b()) <= 4
                })
                .count()
        };

        let mut enabled = render(ItemState::Normal);
        enabled.run();
        let enabled_frame = enabled.render().expect("the row renders");
        assert_eq!(
            count(&enabled_frame, palette.error),
            0,
            "an ordinary row never borrows the destructive ink"
        );

        let mut destructive = render(ItemState::Destructive);
        destructive.run();
        let destructive_frame = destructive.render().expect("the row renders");
        assert!(
            count(&destructive_frame, palette.error) > 0,
            "a destructive row carries the error ink"
        );

        let mut disabled = render(ItemState::Disabled);
        disabled.run();
        disabled.get_by_label("Remove from Playlist").click();
        disabled.run();
        assert!(
            !*disabled.state(),
            "a disabled row answers no click, so it cannot emit an intent"
        );
    }

    /// The Playlist Store side of the menu: the submenu groups the playlists it
    /// was handed under one heading, an empty list is an inert row rather than a
    /// hole, and a chosen playlist rides back on the intent.
    #[test]
    fn test_playlist_rows_emit_the_chosen_playlist_or_nothing() {
        let palette = Palette::dark();
        let first = PlaylistId::new("pl-1");
        let second = PlaylistId::new("pl-2");
        let options = vec![
            (first.clone(), "Gym".to_string()),
            (second.clone(), "Focus".to_string()),
        ];

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(220.0, 160.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, intents: &mut Vec<TrackMenuIntent>| {
                    menu::playlist_items(ui, &palette, &options, intents);
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness.query_by_label("Playlists").is_some(),
            "the submenu groups its rows under one section heading"
        );
        harness.get_by_label("Focus").click();
        harness.run();
        assert_eq!(
            *harness.state(),
            vec![TrackMenuIntent::AddToPlaylist(second.clone())],
            "the chosen playlist rides back on the intent"
        );

        let empty: Vec<(PlaylistId, String)> = Vec::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(220.0, 64.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, intents: &mut Vec<TrackMenuIntent>| {
                    menu::playlist_items(ui, &palette, &empty, intents);
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness.query_by_label("No playlists yet").is_some(),
            "an empty list explains itself instead of leaving a hole"
        );
        harness.get_by_label("No playlists yet").click();
        harness.run();
        assert!(
            harness.state().is_empty(),
            "the explanation is inert: it emits no intent"
        );
    }

    // --- The host's half: an effect happens only in answer to an intent -------

    /// The app's store, plus one playlist seeded before any frame runs.
    fn seeded_store() -> (
        tempfile::TempDir,
        riff_infra::store::SqliteStore,
        PlaylistId,
    ) {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(
            &dir.path().join("riff.sqlite3"),
            changes_tx,
        )
        .expect("opening a fresh store must work");
        let pid = store
            .create_playlist("Gym", &[])
            .expect("the playlist commits");
        (dir, store, pid)
    }

    /// Wire the host slots one menu intent answers to. A fresh call reborrows
    /// everything, so a test can apply an intent, drop the host, and read the
    /// store back.
    fn host<'a>(
        item: &'a riff_backend::domain::Track,
        pid: Option<&'a PlaylistId>,
        store: &'a mut riff_infra::store::SqliteStore,
        transport: &'a MockTransport,
        selected: &'a mut Option<TrackId>,
        editor: &'a mut InlineTagEditor,
    ) -> TrackMenuEffects<'a> {
        let playlist_store: &'a mut dyn PlaylistStore = store;
        TrackMenuEffects {
            track_id: &item.id,
            track: Some(item),
            remove_from_playlist: pid,
            transport,
            playlist_store,
            selected_track: selected,
            tag_editor: editor,
        }
    }

    #[test]
    fn test_playlist_intents_commit_only_when_emitted() {
        let (_dir, mut store, pid) = seeded_store();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let item = track("/music/a.mp3");

        let entries = |store: &riff_infra::store::SqliteStore| {
            store
                .load_playlist_entries(&pid)
                .expect("the entries read")
                .len()
        };
        assert_eq!(
            entries(&store),
            0,
            "the menu a Track carries writes nothing until it is used"
        );

        {
            let mut slots = host(
                &item,
                Some(&pid),
                &mut store,
                &transport,
                &mut selected,
                &mut editor,
            );
            apply_track_menu_intent(TrackMenuIntent::AddToPlaylist(pid.clone()), &mut slots);
        }
        assert_eq!(
            entries(&store),
            1,
            "the emitted Add to Playlist committed through the store"
        );

        {
            let mut slots = host(
                &item,
                Some(&pid),
                &mut store,
                &transport,
                &mut selected,
                &mut editor,
            );
            apply_track_menu_intent(TrackMenuIntent::RemoveFromPlaylist, &mut slots);
        }
        assert_eq!(entries(&store), 0, "the emitted Remove committed its half");
        assert!(
            transport.recorded().is_empty(),
            "a playlist intent sends no transport command"
        );
    }

    #[test]
    fn test_edit_tags_intent_opens_the_detail_editor_and_nothing_else() {
        let (_dir, mut store, pid) = seeded_store();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let item = track("/music/a.mp3");

        {
            let mut slots = host(
                &item,
                Some(&pid),
                &mut store,
                &transport,
                &mut selected,
                &mut editor,
            );
            apply_track_menu_intent(TrackMenuIntent::EditTags, &mut slots);
        }

        assert_eq!(selected, Some(item.id.clone()));
        let draft = editor.draft().expect("the editor opened");
        assert_eq!(draft.track_id, item.id);
        assert_eq!(draft.kind, DraftKind::Track);
        assert!(
            draft.focus_first,
            "the entry point asks the editor to focus its first field"
        );
        assert!(
            transport.recorded().is_empty(),
            "opening the editor sends no transport command"
        );
        assert_eq!(
            store
                .load_playlist_entries(&pid)
                .expect("the entries read")
                .len(),
            0,
            "and it writes no playlist entry"
        );
    }

    #[test]
    fn test_playback_intents_reach_the_transport_only_when_emitted() {
        let (_dir, mut store, _pid) = seeded_store();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let item = track("/music/a.mp3");

        {
            let mut slots = host(
                &item,
                None,
                &mut store,
                &transport,
                &mut selected,
                &mut editor,
            );
            apply_track_menu_intent(TrackMenuIntent::Play, &mut slots);
            apply_track_menu_intent(TrackMenuIntent::PlayNext, &mut slots);
            apply_track_menu_intent(TrackMenuIntent::AddToQueue, &mut slots);
        }

        assert_eq!(
            transport.recorded(),
            vec![
                TransportIntent::Play(item.id.clone()),
                TransportIntent::PlayNext(item.id.clone()),
                TransportIntent::AddToQueue(item.id.clone()),
            ],
            "each playback intent maps to exactly its own command"
        );
        assert!(
            editor.draft().is_none() && selected.is_none(),
            "playback intents neither open the editor nor move the selection"
        );

        let second = track("/music/b.mp3");
        let ids = vec![item.id.clone(), second.id.clone()];
        apply_list_menu_intent(ListMenuIntent::Play, &ids, &transport);
        assert_eq!(
            &transport.recorded()[3..],
            &[
                TransportIntent::Play(item.id.clone()),
                TransportIntent::AddToQueue(second.id.clone()),
            ],
            "the list's Play keeps its current shape: the first Track, then the rest"
        );

        apply_list_menu_intent(ListMenuIntent::PlayNext, &ids, &transport);
        assert_eq!(
            &transport.recorded()[5..],
            &[
                TransportIntent::PlayNext(second.id.clone()),
                TransportIntent::PlayNext(item.id.clone()),
            ],
            "Play Next preserves the current reverse-order sends so the list lands in order"
        );

        apply_list_menu_intent(ListMenuIntent::AppendToQueue, &ids, &transport);
        assert_eq!(
            &transport.recorded()[7..],
            &[
                TransportIntent::AddToQueue(item.id.clone()),
                TransportIntent::AddToQueue(second.id.clone()),
            ],
            "Append to Queue adds the whole list at the end, in order"
        );
    }

    /// The menu's accessible contract: every row is a real button, so the items
    /// are reachable by keyboard once the menu is open, and hovering one emits
    /// nothing.
    #[test]
    fn test_a_row_renders_as_a_real_button() {
        let palette = Palette::dark();
        let options: [(PlaylistId, String); 0] = [];
        let props = TrackMenu {
            playable: true,
            editable: true,
            playlists: &options,
            remove_from_playlist: false,
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 200.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, intents: &mut Vec<TrackMenuIntent>| {
                    menu::track_menu(ui, &palette, &props, intents);
                },
                Vec::new(),
            );
        harness.run();
        harness.get_by_label("Edit Tags").hover();
        harness.run();
        assert_eq!(
            std::mem::take(harness.state_mut()).len(),
            0,
            "hovering an item emits nothing"
        );

        harness.get_by_label("Edit Tags").focus();
        harness.run();
        assert!(
            harness.get_by_label("Edit Tags").is_focused(),
            "every row is a real focusable button, so the menu works by keyboard"
        );
    }
}

// --- Library Path: pure Settings input + host-owned picker (issue 16) --------
//
// The Settings surface's Library Path input renders from data the host resolved
// and reports typed actions; the filesystem check and the session registration
// live in `RiffApp`'s Library Path adapter, which the Settings surface and the
// sidebar's Add Library control share.

#[cfg(test)]
mod library_path_ui_tests {
    use egui_kittest::kittest::Queryable;
    use riff_backend::app::state::{LibrarySession, LibraryStatus};
    use riff_gui::ui::app::register_library_path;
    use riff_gui::ui::settings::{PathInput, PathInputAction, path_input};
    use riff_gui::ui::theme::Palette;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use crate::mocks::{MockSettingsStore, SettingsCall};

    /// Render the input row over `error`/`suggestions` and click `labels`,
    /// returning the actions it reported.
    fn input_actions(
        error: Option<&str>,
        suggestions: &[PathBuf],
        clicks: &[&str],
    ) -> Vec<PathInputAction> {
        let palette = Palette::dark();
        let suggestions = suggestions.to_vec();
        let error = error.map(str::to_owned);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(520.0, 160.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, actions: &mut Vec<PathInputAction>| {
                    let mut text = String::from("/music");
                    let mut input = PathInput {
                        text: &mut text,
                        error: error.as_deref(),
                        suggestions: &suggestions,
                    };
                    actions.extend(path_input(ui, &palette, &mut input));
                },
                Vec::new(),
            );
        harness.run();
        for label in clicks {
            harness.get_by_label(label).click();
            harness.run();
        }
        std::mem::take(harness.state_mut())
    }

    #[test]
    fn test_path_input_reports_only_what_the_listener_chose() {
        assert!(
            input_actions(Some("Path does not exist: /music"), &[], &[]).is_empty(),
            "rendering the input reports nothing"
        );
        assert_eq!(
            input_actions(None, &[], &["Confirm"]),
            vec![PathInputAction::Confirm],
            "Confirm reports the choice; validating it is the host's job"
        );
        assert_eq!(
            input_actions(None, &[], &["Cancel"]),
            vec![PathInputAction::Cancel],
            "Cancel reports the dismissal; clearing the draft is the host's"
        );

        let suggestion = PathBuf::from("/home/listener/Music");
        let row = format!("\u{1F4C1} {}", suggestion.to_string_lossy());
        assert_eq!(
            input_actions(None, std::slice::from_ref(&suggestion), &[&row]),
            vec![PathInputAction::Complete(suggestion)],
            "a suggestion the host resolved comes back as the path it names"
        );
    }

    /// The rejection the host produced is what the row shows — the input reads
    /// its error, it never derives one.
    #[test]
    fn test_path_input_shows_the_hosts_resolved_error() {
        let palette = Palette::dark();
        let mut text = String::from("/music");
        let suggestions: Vec<PathBuf> = Vec::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(520.0, 96.0))
            .with_pixels_per_point(1.0)
            .build_ui(|ui| {
                let mut input = PathInput {
                    text: &mut text,
                    error: Some("Not a directory: /music"),
                    suggestions: &suggestions,
                };
                let _ = path_input(ui, &palette, &mut input);
            });
        harness.run();
        assert!(
            harness.query_by_label("Not a directory: /music").is_some(),
            "the row carries the host's rejection verbatim"
        );
    }

    fn scratch() -> (
        LibrarySession,
        MockSettingsStore,
        Arc<Mutex<Vec<SettingsCall>>>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            LibrarySession::default(),
            MockSettingsStore::with_shared_calls(Arc::clone(&calls)),
            calls,
        )
    }

    #[test]
    fn test_a_valid_path_registers_once_writes_once_and_starts_nothing() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let root = dir.path().join("music");
        std::fs::create_dir(&root).expect("a scratch library directory");
        let canonical = root.canonicalize().expect("the canonical root");
        let (mut library, mut store, calls) = scratch();

        let registered =
            register_library_path(root.to_str().expect("utf-8"), &mut library, &mut store)
                .expect("a real directory is accepted");
        assert_eq!(registered, canonical);
        assert_eq!(
            library.library_paths.paths(),
            std::slice::from_ref(&canonical),
            "the root becomes a fact of the session"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec![SettingsCall::LibraryPaths],
            "the typed settings row is the only durable write"
        );
        assert!(
            library.scan_status.is_none(),
            "registering a root announces no scan"
        );
        assert_eq!(
            library.library_paths.readiness(&canonical),
            LibraryStatus::Idle,
            "and the fresh root claims no readiness until the listener indexes it"
        );

        let again = register_library_path(root.to_str().expect("utf-8"), &mut library, &mut store)
            .expect("a repeat selection is still a valid directory");
        assert_eq!(again, canonical, "the duplicate answers with the same root");
        assert_eq!(library.library_paths.paths().len(), 1, "and registers once");
        assert_eq!(
            calls.lock().unwrap().len(),
            1,
            "a duplicate selection writes nothing twice"
        );
    }

    #[test]
    fn test_a_missing_path_or_a_plain_file_is_refused_without_writing() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let file = dir.path().join("song.mp3");
        std::fs::write(&file, b"not a directory").expect("a scratch file");
        let (mut library, mut store, calls) = scratch();

        let missing = dir.path().join("nowhere");
        let err = register_library_path(missing.to_str().expect("utf-8"), &mut library, &mut store)
            .expect_err("a path that is not there is refused");
        assert!(
            err.starts_with("Path does not exist"),
            "the refusal explains itself: {err}"
        );

        let err = register_library_path(file.to_str().expect("utf-8"), &mut library, &mut store)
            .expect_err("a plain file is not a library root");
        assert!(
            err.starts_with("Not a directory"),
            "the refusal names the real problem: {err}"
        );

        assert!(
            library.library_paths.paths().is_empty(),
            "nothing was registered"
        );
        assert!(
            calls.lock().unwrap().is_empty(),
            "a refused candidate writes no settings"
        );
    }
}

// --- The component boundary, as a standing gate (issue 18) --------------------
//
// The two-tier layer is only real if the rules stay enforced. These sweeps are
// the enforcement: what a primitive may not own, that every component module is
// inside the token sweeps' view, and that the historical frontend paths the
// extraction promised to keep on resolving actually still resolve.

#[cfg(test)]
mod component_boundary_tests {
    use eframe::egui;
    use riff_backend::app::Transport;
    use riff_backend::domain::TrackId;
    use std::path::{Path, PathBuf};

    /// Every module the component-layer tickets added or reshaped: primitives
    /// and the neutral presentation owners. Feature composites (`app`,
    /// `browser_pane`, `sidebar`, `settings`, `detail`, `playerbar`,
    /// `now_playing`, `chrome`, `selection`) are deliberately absent — they are
    /// allowed to hold application state, and the sweeps below would be wrong
    /// to demand otherwise.
    const PRIMITIVES: [&str; 13] = [
        "artwork.rs",
        "button.rs",
        "empty_state.rs",
        "feedback.rs",
        "icons.rs",
        "linear.rs",
        "menu.rs",
        "prompts.rs",
        "row.rs",
        "stage.rs",
        "text_field.rs",
        "toggle_switch.rs",
        "up_next.rs",
    ];

    fn ui_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("crates")
            .join("riff-gui")
            .join("src")
            .join("ui")
    }

    /// `(path, line)` for every code line of one primitive module, comments
    /// dropped — prose is allowed to name the owners a primitive must not hold.
    fn primitive_lines(name: &str) -> Vec<(PathBuf, String)> {
        let path = ui_dir().join(name);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{name} is part of the component layer"));
        source
            .lines()
            .map(str::trim_start)
            .filter(|line| !line.starts_with("//"))
            .map(|line| (path.clone(), line.to_owned()))
            .collect()
    }

    /// A primitive receives presentation values and returns responses or typed
    /// intents. It must not reach for the application: not `RiffApp`, the two
    /// sessions, the read model, a port, a service front end, a generation, or a
    /// native integration. Those are the host's, and the sweep is what stops a
    /// future "just this one call" from putting them back.
    #[test]
    fn test_no_primitive_names_an_application_owner() {
        const FORBIDDEN: [&str; 17] = [
            "RiffApp",
            "SessionViews",
            "LibrarySession",
            "PlaybackSession",
            "dyn Transport",
            "Transport::",
            "PlaylistStore",
            "SettingsStore",
            "LibraryMutationStore",
            "Scans",
            "Covers",
            "TagEdits",
            "WatcherManager",
            "rfd::",
            // A primitive depends on the theme and its own props — never on the
            // composite that hosts it. (Both arrows used to point the wrong
            // way: ui::app <-> ui::cover_placeholder, ui::stage -> ui::app.)
            "ui::app::",
            "super::app::",
            "crate::app::",
        ];
        let mut offenders = Vec::new();
        for name in PRIMITIVES {
            for (path, line) in primitive_lines(name) {
                if FORBIDDEN.iter().any(|owner| line.contains(owner)) {
                    offenders.push(format!("{}: {line}", path.display()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "the component layer must stay free of application ownership:\n{}",
            offenders.join("\n")
        );
    }

    /// A primitive may hold its own presentation state — the feedback board
    /// folds notices into slots, an icon cache holds textures — but it never
    /// reaches a *shared* handle: no `Arc<Mutex<…>>` session, no lock-recovery,
    /// no clone of an application handle. The two sessions, the event inbox, and
    /// the watcher live behind `Arc<Mutex<…>>` in the host, and that is where
    /// they stay.
    #[test]
    fn test_no_primitive_reaches_a_shared_application_handle() {
        const SHARED_HANDLES: [&str; 4] = ["Arc<Mutex", "Arc<", "lock_or_recover", "RiffApp"];
        let mut offenders = Vec::new();
        for name in PRIMITIVES {
            for (path, line) in primitive_lines(name) {
                if SHARED_HANDLES.iter().any(|handle| line.contains(handle)) {
                    offenders.push(format!("{}: {line}", path.display()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "a primitive takes caller-owned values, never a shared application handle:\n{}",
            offenders.join("\n")
        );
    }

    /// The token sweeps (ADR 0004) walk `src/ui` recursively, so a new component
    /// module is covered the moment it lands — but only if it is really there.
    /// Asserted against the same directory the sweeps scan, so a module renamed
    /// out of the list is caught rather than silently unscanned.
    #[test]
    fn test_every_component_module_is_inside_the_token_sweeps() {
        fn scan(dir: &Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("src/ui must be readable") {
                let path = entry.expect("a readable entry").path();
                if path.is_dir() {
                    scan(&path, found);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    found.push(
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .expect("a utf-8 file name")
                            .to_owned(),
                    );
                }
            }
        }

        // The sweeps exclude exactly one file — `theme.rs`, the token store
        // itself — so any module that lives under `src/ui` is judged by them.
        let mut found = Vec::new();
        scan(&ui_dir(), &mut found);
        let missing: Vec<&str> = PRIMITIVES
            .iter()
            .copied()
            .filter(|name| !found.iter().any(|file| file == name))
            .collect();
        assert!(
            missing.is_empty(),
            "component modules outside the swept tree would escape the token \
             sweeps (ADR 0004): {missing:?}"
        );
    }
    /// Story 40: the extraction is an internal ownership change, not an
    /// application-wide migration. Every historical frontend path feature code
    /// and the suite were already writing has to keep resolving — these are the
    /// spellings the tickets promised to keep alive through re-exports, named
    /// here so a cleanup that deletes one is a test failure, not a quiet break.
    #[test]
    fn test_historical_frontend_paths_still_resolve() {
        // Moved out of `ui::app` in ticket 13; still callable from it.
        let _boxes = [
            riff_gui::ui::app::COVER_THUMB,
            riff_gui::ui::app::COVER_CARD,
            riff_gui::ui::app::COVER_HERO,
        ];
        let _key = riff_gui::ui::app::cover_cache_key("a.mp3", riff_gui::ui::app::COVER_THUMB);
        let _: Option<riff_gui::ui::app::CoverCacheKey> = None;
        let _cap = riff_gui::ui::app::COVER_CACHE_CAP;
        let _evicted: Vec<String> = riff_gui::ui::app::lru_insert(&mut Vec::new(), "k".into(), 1);

        // The artwork placeholder's old home.
        let _tile = riff_gui::ui::cover_placeholder::placeholder_cache_key;

        // Moved out of `ui::browser` (ticket 12) and `ui::now_playing` (ticket 05).
        let _empty: fn(&mut egui::Ui, &riff_gui::ui::theme::Palette, &str, &str) =
            riff_gui::ui::browser::empty_state;
        let _entries = riff_gui::ui::now_playing::up_next_entries(&[], 5);
        let _entry: Option<riff_gui::ui::now_playing::UpNextEntry> = None;

        // The host-adapter paths the suite drives menus and prompts through.
        let _intent: fn(riff_gui::ui::menu::ListMenuIntent, &[TrackId], &dyn Transport) =
            riff_gui::ui::app::apply_list_menu_intent;
        let _duration: fn(std::time::Duration) -> String = riff_gui::ui::app::format_duration;
        let _expand: fn(&str) -> std::path::PathBuf = riff_gui::ui::settings::expand_tilde;
    }
}
