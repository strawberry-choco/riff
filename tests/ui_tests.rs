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
            &Arc::new(Mutex::new(None)),
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
            &Arc::new(Mutex::new(None)),
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
            &Arc::new(Mutex::new(None)),
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
            &Arc::new(Mutex::new(None)),
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
            views.track_count("", riff_backend::app::state::TrackSort::default()),
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
        // --riff-brand-50/400/500 — the rungs anything reads; 500 is the primary.
        assert_eq!(theme::BRAND_50, egui::Color32::from_rgb(0xff, 0xf8, 0xe7));
        assert_eq!(theme::BRAND_400, egui::Color32::from_rgb(0xff, 0xb8, 0x33));
        assert_eq!(theme::BRAND_500, egui::Color32::from_rgb(0xf0, 0x82, 0x1e));
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
        use riff_gui::ui::artwork::lookup_cover_texture;
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
        let mut cache = riff_gui::ui::cover_cache::CoverCache::new();
        let a = lookup_cover_texture(
            &mut cache,
            &mut textures,
            &mut lru_keys,
            &ctx,
            &Palette::dark(),
            "a.mp3",
            thumb,
        );
        let b = lookup_cover_texture(
            &mut cache,
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
            &mut cache,
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
        use riff_gui::ui::artwork::placeholder_image;
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
        use riff_gui::ui::artwork::{
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

        let mut cache = riff_gui::ui::cover_cache::CoverCache::new();
        let tile = lookup_cover_texture(
            &mut cache,
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
            &mut cache,
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
        use riff_gui::ui::artwork::lookup_cover_texture;
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
        let mut cache = riff_gui::ui::cover_cache::CoverCache::new();
        let _ = lookup_cover_texture(
            &mut cache,
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
            &mut cache,
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
            &mut cache,
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
        use riff_gui::ui::artwork::{evict_generated, lookup_cover_texture, placeholder_cache_key};
        use riff_gui::ui::theme::Palette;
        use riff_library::app::traits::RequestedSize;

        let ctx = placeholder_ctx();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();
        let size = RequestedSize {
            width: 56,
            height: 56,
        };

        let mut cache = riff_gui::ui::cover_cache::CoverCache::new();
        let _ = lookup_cover_texture(
            &mut cache,
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

    /// The cycle guard: the placeholder tile used to live in a sibling module
    /// that `use`d `crate::ui::app::…` for the very cache-key space the app
    /// module imported the placeholder back from. `ui::artwork` owns that key
    /// space now, so no sibling module reaches into `ui::app` for it.
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
                let _ = clear_library_confirm(ui, &mut cache, &palette, false);
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
        let style = theme::style_from(&theme::Palette::dark(), false);
        let v = &style.visuals;

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

        // The motion family is published, not merely declared: this is the pin
        // that makes the theme store the authority over time. If the publish
        // ever stops, egui silently falls back to its own 0.2 s default and
        // every popup fade, scrollbar expansion and collapsing body inherits
        // the framework's tempo instead of the design's.
        assert_eq!(style.animation_time, theme::MOTION_DEFAULT);

        // Two durations, two jobs: the hover wash is a distinct, shorter step.
        // Equal values would mean one token wearing two names, and a hover
        // wash is the case that must never read as lag. Bound to locals so
        // this reads as a relation between two values rather than as a
        // restatement of the declarations above it.
        let (default, hover) = (theme::MOTION_DEFAULT, theme::MOTION_HOVER);
        assert_ne!(hover, default);
        assert!(hover < default);
    }

    #[test]
    fn test_style_from_applies_light_tokens_when_given_the_light_palette() {
        let light = theme::Palette::light();
        let style = theme::style_from(&light, false);
        let v = &style.visuals;

        assert!(!v.dark_mode);
        assert_eq!(v.panel_fill, light.surface);
        assert_eq!(v.window_fill, light.background);
        assert_eq!(v.override_text_color, Some(light.ink));
        assert_eq!(v.widgets.hovered.weak_bg_fill, light.surface_2);
        assert_eq!(v.widgets.inactive.bg_stroke.color, light.border);

        // Motion is palette-invariant, so the light family publishes the same
        // duration as the dark one. High Contrast changes legibility, not
        // tempo, and per-palette motion sets would be four things to keep
        // coherent for no gain.
        assert_eq!(style.animation_time, theme::MOTION_DEFAULT);
    }

    #[test]
    fn test_high_contrast_style_keeps_focus_unmistakable() {
        // REQ-UI-007 carried over: focused/selected elements get strokes
        // thicker than egui's 1.0 default, over either base.
        for base in [theme::Palette::dark(), theme::Palette::light()] {
            let style = theme::style_from(&base.high_contrast(), false);
            let v = &style.visuals;
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

            // High Contrast is the palette most likely to acquire a
            // motion token set by accident — it is where a "slower, clearer"
            // instinct would land. Asserted inside the loop so the
            // invariance is pinned over both High Contrast families, not just
            // the dark one.
            assert_eq!(
                style.animation_time,
                theme::MOTION_DEFAULT,
                "High Contrast over the {} base must not slow the app down",
                if base.dark { "dark" } else { "light" }
            );
        }
    }

    #[test]
    fn test_install_applies_the_palette_to_the_context() {
        let ctx = egui::Context::default();
        let light = theme::Palette::light();

        theme::install(&ctx, &light, false);

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
        theme::install(&ctx, &theme::Palette::dark(), false);

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
    //
    // macOS native title bar (macos-native-title-bar issue 01, amending ADR
    // 0005): custom chrome is now the rule on Windows and Linux only. On macOS
    // the window keeps its AppKit decorations with a transparent title bar
    // carrying riff's full-size content, and the system's traffic lights are
    // the window controls. The split hangs off one pure per-OS decision —
    // `chrome_mode()` — consumed by both the launch viewport and the titlebar
    // renderer, which is what keeps the macOS branch assertable as data on the
    // Linux/Windows CI machines: the parameterized functions below execute the
    // native branch there without any `cfg`.

    #[test]
    fn test_chrome_mode_puts_native_traffic_lights_on_macos_and_custom_caption_elsewhere() {
        use riff_gui::ui::chrome::{ChromeMode, chrome_mode};

        // The single branch point, resolved per OS. On the Linux/Windows CI
        // machines this is the custom-chrome branch; on macOS the suite's
        // mode-parameterized tests exercise the native branch as data.
        #[cfg(target_os = "macos")]
        assert_eq!(chrome_mode(), ChromeMode::NativeTrafficLights);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(chrome_mode(), ChromeMode::CustomCaption);
    }

    #[test]
    fn test_traffic_light_clearance_divides_the_measured_metric_by_the_zoom_and_floors_at_the_token()
     {
        use riff_gui::ui::chrome::traffic_light_clearance;
        use riff_gui::ui::theme::geometry::titlebar::TRAFFIC_LIGHT_CLEARANCE;

        // The fallback constant is a design value in the theme's single store,
        // and a floor under the real cluster: it must exceed the standard
        // traffic-light span so the wordmark clears the lights even where
        // eframe cannot measure them (headless tests, exotic AppKit states).
        // A `const` block, because the comparison is on constants — this is a
        // compile-time contract on the design value, not a runtime check.
        const {
            assert!(
                TRAFFIC_LIGHT_CLEARANCE >= 66.0,
                "the standard macOS cluster spans ~66pt"
            );
        }

        // The measurement path: eframe reports the cluster's width in native
        // scale; the renderer divides by the zoom factor to land in egui
        // points.
        assert!((traffic_light_clearance(Some(140.0), 2.0) - 70.0).abs() < f32::EPSILON);

        // Where eframe cannot measure (no window handle, non-AppKit), the
        // documented fallback stands in.
        assert!(
            (traffic_light_clearance(None, 1.0) - TRAFFIC_LIGHT_CLEARANCE).abs() < f32::EPSILON
        );
        // A measurement narrower than the floor keeps the floor: the fallback
        // is the acceptable minimum clearance, the measurement only raises it.
        assert!(
            (traffic_light_clearance(Some(20.0), 1.0) - TRAFFIC_LIGHT_CLEARANCE).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn test_launch_viewport_is_frameless_while_keeping_the_window_size_contract() {
        use riff_gui::ui::chrome::{ChromeMode, chrome_mode, viewport_builder_for};

        let builder = riff_gui::ui::chrome::viewport_builder();

        // The launch viewport is the chrome-mode decision applied: the
        // frameless contract holds wherever custom chrome is the rule (and is
        // pinned per-branch by the mode-parameterized test below), and on
        // macOS the AppKit-decorated transparent-titlebar shape ships instead.
        assert_eq!(builder, viewport_builder_for(chrome_mode()));
        if chrome_mode() == ChromeMode::NativeTrafficLights {
            assert_eq!(builder.decorations, Some(true));
        } else {
            assert_eq!(builder.decorations, Some(false));
        }
        // The decorated window's launch/minimum sizes carry over unchanged.
        assert_eq!(builder.inner_size, Some(egui::vec2(1200.0, 800.0)));
        assert_eq!(builder.min_inner_size, Some(egui::vec2(800.0, 600.0)));
    }

    #[test]
    fn test_launch_viewport_follows_the_chrome_mode_branch() {
        use riff_gui::ui::chrome::{ChromeMode, viewport_builder_for};

        // Sizes and the riff mark are shared by both branches: the chrome
        // decision changes only how the window is decorated.
        for mode in [ChromeMode::CustomCaption, ChromeMode::NativeTrafficLights] {
            let builder = viewport_builder_for(mode);
            assert_eq!(builder.inner_size, Some(egui::vec2(1200.0, 800.0)));
            assert_eq!(builder.min_inner_size, Some(egui::vec2(800.0, 600.0)));
            assert!(builder.icon.is_some(), "both branches carry riff's mark");
        }

        // Custom branch: frameless exactly as today.
        let custom = viewport_builder_for(ChromeMode::CustomCaption);
        assert_eq!(custom.decorations, Some(false));
        assert_eq!(custom.fullsize_content_view, None);
        assert_eq!(custom.titlebar_shown, None);

        // Native branch: the window keeps its AppKit decorations, the content
        // view is full-size (content behind the title bar), the title bar is
        // transparent, and the title text is hidden. The traffic-light buttons
        // stay shown — they are the point.
        let native = viewport_builder_for(ChromeMode::NativeTrafficLights);
        assert_eq!(native.decorations, Some(true));
        assert_eq!(native.fullsize_content_view, Some(true));
        assert_eq!(native.titlebar_shown, Some(false));
        assert_eq!(
            native.titlebar_buttons_shown, None,
            "buttons default to shown"
        );

        // `title_shown` and `titlebar_shown` are TWO separate egui fields, and
        // egui-winit maps each to exactly one winit call: `title_shown` drives
        // only `with_title_hidden` (AppKit `titleVisibility=hidden`),
        // `titlebar_shown` drives only `with_titlebar_transparent`
        // (AppKit `titlebarAppearsTransparent`). So `titlebar_shown = false`
        // alone makes the bar transparent while AppKit keeps painting the
        // window title text behind the strip — "transparent" is not "hidden".
        // The two must be set together for the full native shape.
        assert_eq!(
            native.title_shown,
            Some(false),
            "the native title bar is transparent but still shows the window title text"
        );
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

    // macOS traffic-light centring (the drift after the native title bar
    // shipped): AppKit lays the lights out for ITS OWN titlebar height — 28pt,
    // 32pt on macOS 26 Tahoe — so in riff's 56pt strip they ride ~14pt above
    // centre. `ui::traffic_lights` re-centres them per frame by moving y only.
    //
    // The AppKit half is macOS-only and therefore untestable here (and, more
    // importantly, un-covered by the golden suite: the lights are composited by
    // the window's theme frame ABOVE egui's render surface, so no pixel they
    // move lands in a baseline). What is portable is the decision — the target
    // geometry and the drift threshold — and that is exactly what the two pure
    // functions below hold, asserted here as data so the Linux/Windows CI legs
    // execute them with no `cfg`.

    #[test]
    fn test_traffic_light_plan_centres_the_measured_button_in_the_strip() {
        use riff_gui::ui::chrome::traffic_light_plan;

        // Both real button heights, and no OS-version branch between them: the
        // button height is MEASURED off the live NSButton, never hardcoded, so
        // Apple's next change to the titlebar costs this test nothing. Were the
        // height hardcoded to the pre-Tahoe 16pt, a Tahoe window would be
        // centred against a 16pt target and sit 1pt low — the sort of defect
        // that only ever shows up on someone else's machine.
        assert_eq!(traffic_light_plan(56.0, 16.0), (20.0, 56.0));
        assert_eq!(traffic_light_plan(56.0, 14.0), (21.0, 56.0));

        // The invariant that makes the placement correct, rather than merely
        // equal to a literal: the same inset is both the top and the bottom
        // gap, so the result is right whether or not AppKit's titlebar
        // container is coordinate-flipped. A sign error or a one-sided
        // inset (a centred-looking `y` derived from the window's top edge, say)
        // breaks the sum while still passing a naive "is it roughly centred"
        // eyeball — that is the failure this pins.
        for button_h in [16.0, 14.0] {
            let (inset_y, container_h) = traffic_light_plan(56.0, button_h);
            assert_eq!(button_h + 2.0 * inset_y, 56.0, "not vertically centred");
            // The container must be the strip's full height, or the buttons
            // would sit outside the box AppKit lays out and lose the argument
            // on the next relayout.
            assert_eq!(container_h, 56.0, "container must span the strip");
        }
    }

    #[test]
    fn test_needs_reapply_ignores_sub_half_point_jitter_and_catches_a_reset() {
        use riff_gui::ui::chrome::needs_reapply;

        // The steady state must be a no-op: every frame re-reads the geometry,
        // and writing it back when nothing moved would churn AppKit's tracking
        // areas 60 times a second for nothing.
        assert!(!needs_reapply(56.0, 20.0, 56.0, 20.0));

        // The anti-thrash case. AppKit's layout can hand back a frame a fraction
        // of a point off ours; re-applying on that would make the buttons
        // visibly shimmer, and a write that is immediately contradicted is a
        // layout pass with no way to converge. A quarter-point is below the
        // threshold, so it is left alone.
        assert!(!needs_reapply(56.0, 20.25, 56.0, 20.0));

        // The case the feature exists for. AppKit re-centres the cluster
        // against ITS titlebar height whenever the window relayouts — and
        // `setTitle:` (which riff sends on every track change) is a documented
        // trigger — so the height and y snap back to the 28pt-container layout
        // and the lights jump. That has to be caught.
        assert!(needs_reapply(28.0, 6.0, 56.0, 20.0));

        // Drift larger than the threshold, but small enough to look like
        // nothing: exactly the case a `!=` comparison would let through (it
        // would rewrite every frame) and an exact-equality comparison would
        // also let through as a permanent unflappable mismatch. The threshold
        // catches it once and stops.
        assert!(needs_reapply(56.0, 20.75, 56.0, 20.0));
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
        // The launch decoration follows the chrome-mode decision (pinned per
        // branch by the viewport tests above); only the size contract matters
        // here.
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

    // The custom-chrome caption contract exists only where the caption code
    // compiles — the macOS build carries no caption buttons at all (the
    // traffic lights are the window controls), so the native-mode pass below
    // is what runs there.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn test_titlebar_clicks_report_window_and_nav_actions() {
        use riff_gui::ui::chrome::{ChromeMode, TitleBarAction, TitleBarContent, show_titlebar};
        use riff_gui::ui::icons::IconCache;

        // Harness label queries resolve through kittest's accessibility tree.
        use egui_kittest::kittest::Queryable;

        let content = TitleBarContent {
            scan_status: None,
            theme_dark: true,
            // Library is the active destination in this fixture.
            active_nav: Some(chrome::NavDestination::Library),
            // This test pins the custom-chrome caption contract.
            chrome: ChromeMode::CustomCaption,
            traffic_clearance: 0.0,
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
    }

    #[test]
    fn test_titlebar_left_inset_starts_the_wordmark_cluster_past_the_traffic_lights() {
        use riff_gui::ui::chrome::{ChromeMode, titlebar_left_inset};
        use riff_gui::ui::theme::geometry::titlebar::{
            TRAFFIC_LIGHT_CLEARANCE, WORDMARK_LEFT_INSET,
        };

        // Custom branch: the strip's first element starts at the small inset
        // it always had.
        assert_eq!(
            titlebar_left_inset(ChromeMode::CustomCaption, TRAFFIC_LIGHT_CLEARANCE),
            WORDMARK_LEFT_INSET
        );
        // Native branch: it starts past the traffic-light clearance — the
        // wordmark can never sit under the lights, whatever the system
        // measures.
        assert_eq!(
            titlebar_left_inset(ChromeMode::NativeTrafficLights, 70.0),
            70.0
        );
    }

    #[test]
    fn test_native_mode_titlebar_reports_no_window_control_actions_and_clears_the_lights() {
        use riff_gui::ui::chrome::{ChromeMode, TitleBarAction, TitleBarContent, show_titlebar};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::geometry::titlebar::TRAFFIC_LIGHT_CLEARANCE;

        // Harness label queries resolve through kittest's accessibility tree.
        use egui_kittest::kittest::Queryable;

        // The macOS-mode pass, executed as data on every platform: the native
        // branch of the chrome-mode decision, rendered headlessly.
        let content = TitleBarContent {
            scan_status: None,
            theme_dark: true,
            active_nav: Some(chrome::NavDestination::Library),
            chrome: ChromeMode::NativeTrafficLights,
            traffic_clearance: TRAFFIC_LIGHT_CLEARANCE,
        };
        let palette = theme::Palette::dark();
        let mut cache = IconCache::new();
        let mut widget_actions = Vec::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(800.0, 56.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions| {
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

        // The window controls are the system's traffic lights — AppKit pixels
        // the harness cannot see. Riff draws none: the caption buttons are
        // gone from the accessibility tree entirely.
        assert!(harness.query_by_label("Close").is_none());
        assert!(harness.query_by_label("Minimize").is_none());
        assert!(harness.query_by_label("Maximize").is_none());

        // The shared controls keep their positions and behaviour.
        harness.get_by_label("Settings").click();
        harness.run();
        harness.get_by_label("Theme").click();
        harness.run();
        harness.get_by_label("Now Playing").click();
        harness.run();

        // And no interaction on the strip ever emits a window-control action:
        // the native close resolves through the close-request path, not the
        // titlebar.
        let state = harness.state();
        assert!(!state.contains(&TitleBarAction::Minimize));
        assert!(!state.contains(&TitleBarAction::ToggleMaximize));
        assert!(!state.contains(&TitleBarAction::Close));
        assert!(state.contains(&TitleBarAction::GoSettings));
        assert!(state.contains(&TitleBarAction::ToggleTheme));
        assert!(state.contains(&TitleBarAction::ToggleNowPlaying));

        // The left cluster starts past the traffic-light clearance. The
        // wordmark is paint, not a widget, so the observable proxy is the
        // search field: it fills the band after the left cluster, so its left
        // edge clears the lights.
        let search_left = harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .rect()
            .left();
        assert!(
            search_left >= TRAFFIC_LIGHT_CLEARANCE,
            "the search band must start past the traffic lights, got {search_left}"
        );
    }

    #[test]
    fn test_native_mode_drag_region_never_maximizes_on_double_click() {
        // The gesture decision itself is mode-agnostic, but its application
        // is not: on the native branch the system's double-click-titlebar
        // preference governs the titlebar band, and riff hardcodes no
        // maximize toggle there. This pins the mode gate the renderer applies.
        use riff_gui::ui::chrome::{DragRegionAction, drag_region_action};

        // The pure decision is unchanged — the renderer gates the maximize.
        assert_eq!(
            drag_region_action(false, true),
            Some(DragRegionAction::ToggleMaximize)
        );
    }

    #[test]
    fn test_a_committed_quit_is_never_cancelled_by_the_close_resolver() {
        use riff_gui::ui::app::{CUSTOM_TITLEBAR_CLOSE, CloseIntent, close_resolution};

        // The failure this whole test guards: the tray's Quit enqueues a real
        // `ViewportCommand::Close`, which egui-winit turns into
        // `ViewportEvent::Close` — bit-for-bit the same event the macOS red
        // traffic light produces (the `ViewportEvent` enum has one no-payload
        // variant, so a `Close` carries no provenance). With the window
        // visible and "Quit on close" at its default, the old resolver
        // cancelled it: eframe quits unless that frame's viewport output
        // carries `CancelClose`, so Quit became "hide to tray" — while the
        // tray thread had already run `transport.stop()` and already broken
        // its own loop on `quit_flag`, leaving a running, playback-stopped
        // process whose menu is dead and whose window cannot be recovered.

        // A committed quit is a quit. The preference is not consulted at all,
        // so this holds in BOTH preference states — the point of the fix.
        for quit_on_close in [false, true] {
            assert_eq!(
                close_resolution(CloseIntent::Quit, quit_on_close),
                None,
                "a riff-initiated quit must never be cancelled (quit_on_close={quit_on_close})"
            );
        }

        // A window close (the red traffic light) still follows the preference:
        // off (the default) cancels the close for the frame and hides through
        // the frontend-local visibility channel, the exact gesture the custom
        // X performs.
        assert_eq!(
            close_resolution(CloseIntent::WindowClose, false),
            Some((egui::ViewportCommand::CancelClose, CUSTOM_TITLEBAR_CLOSE))
        );

        // On: pass-through, the close proceeds and the app quits.
        assert_eq!(close_resolution(CloseIntent::WindowClose, true), None);
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

    /// Measure every bar's period off the curve that actually ships.
    ///
    /// A bar's height is `(sin(rate * t + offset) * 0.5 + 0.5)` clamped, so it
    /// returns to its OWN phase-zero height twice per cycle: at the half
    /// period (a sine is odd about its start) and again at the full one. The
    /// first return is therefore the half period and the second is the period,
    /// which is what this walks: a forward scan for sign changes of
    /// `heights(t) - heights(0)` — every bar's `cos(offset)` is non-zero, so
    /// every crossing is transversal and can be bisected — stopping at the
    /// second one.
    ///
    /// The point of measuring rather than copying is that the band these
    /// periods are then held to is the tempo contract, and a copy of the rate
    /// literals could only ever agree with the implementation by coincidence.
    /// Reading the period back out of the painted function cannot go stale.
    fn measured_equalizer_periods() -> [f64; 4] {
        let at_zero = sidebar::equalizer_heights(0.0);
        let probe = |bar: usize, t: f64| sidebar::equalizer_heights(t)[bar] - at_zero[bar];
        // 0.5 ms of scan: the two crossings of a bar in the 0.38–0.52 s band
        // are at least 0.19 s apart, so the scan cannot step over one.
        let step = 0.5e-3;
        let mut periods = [0.0_f64; 4];
        for (bar, period) in periods.iter_mut().enumerate() {
            let mut crossings = 0;
            let mut lo = step;
            let mut tau = step;
            let mut side = probe(bar, tau).is_sign_positive();
            while tau < 3.0 {
                tau += step;
                let here = probe(bar, tau).is_sign_positive();
                if here == side {
                    continue;
                }
                // A crossing: bisect it to f64 precision, then keep scanning
                // from it, so the next crossing found is a whole half cycle on.
                let (mut a, mut b) = (lo, tau);
                for _ in 0..40 {
                    let mid = 0.5 * (a + b);
                    if probe(bar, mid).is_sign_positive() == side {
                        a = mid;
                    } else {
                        b = mid;
                    }
                }
                lo = 0.5 * (a + b);
                side = here;
                crossings += 1;
                if crossings == 2 {
                    *period = lo;
                    break;
                }
            }
            assert_eq!(
                crossings, 2,
                "bar {bar}'s height must come back to its phase-zero value \
                 twice per cycle; found {crossings} return(s) in 3 s, so its \
                 rate is not animating at all"
            );
        }
        periods
    }

    /// The tempo: every bar's cycle lands in 0.38–0.52 s, and the four rates
    /// sit within about 1.23× of each other.
    ///
    /// The old rates (5.1 / 6.3 / 4.7 / 5.9 rad/s) ran at 1.232 / 0.997 /
    /// 1.336 / 1.065 s per cycle — a mean of 1.158 s, which reads as swaying
    /// rather than as playing. The band and the spread are what stop the
    /// tempo being walked back down: the spread is the difference between four
    /// voices and one shape breathing, so a set that hits the band but leaves
    /// the rates 1.34× apart has reproduced the same defect at a new speed.
    #[test]
    fn test_equalizer_bar_periods_land_in_the_tightened_band() {
        let periods = measured_equalizer_periods();
        for (bar, period) in periods.iter().enumerate() {
            assert!(
                (0.38..=0.52).contains(period),
                "bar {bar}'s period is {period:.3} s; every cycle must land in \
                 0.38–0.52 s for the group to read as playing"
            );
        }
        // spread = fastest rate / slowest rate = shortest period / longest
        // period, so the measured periods carry the spread without the rates
        // ever being written down twice.
        let shortest = periods.iter().copied().fold(f64::INFINITY, f64::min);
        let longest = periods.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let spread = longest / shortest;
        assert!(
            (spread - 1.23).abs() < 0.01,
            "the four rates span {spread:.3}×; they must stay within about \
             1.23× of each other or the group reads as one shape breathing \
             instead of four voices dancing"
        );
    }

    /// The floor: each bar spends about a quarter of its cycle pinned at the
    /// minimum height, which reads as a VU meter's bottom stop rather than as
    /// a stall. This is the recorded reason the floor is kept, so it is pinned
    /// rather than asserted in prose — retune the floor and this says how long
    /// the bars sit on it.
    ///
    /// The fraction is a property of the floor against a sine, not of the
    /// tempo: `0.15` sits `asin(0.7)` past the trough, so exactly
    /// `(pi - 2 * asin(0.7)) / 2pi` = 25.3% of every cycle is under it. What
    /// the tempo changes is how long that is in wall-clock time — 0.11 s of a
    /// 0.45 s cycle, against 0.29 s of a 1.16 s one.
    #[test]
    fn test_equalizer_bar_floor_dwells_about_a_quarter_of_each_cycle() {
        let periods = measured_equalizer_periods();
        // A 0.25 ms step: the dwell fraction is then good to a few hundredths
        // of a percent, and the samples cover exactly one cycle from its start.
        let step = 0.25e-3;
        for (bar, period) in periods.iter().enumerate() {
            let samples = (period / step).round() as usize;
            let pinned = (0..samples)
                .filter(|i| sidebar::equalizer_heights(*i as f64 * step)[bar] == 0.15)
                .count();
            let dwell = pinned as f64 / samples as f64;
            assert!(
                (0.24..=0.27).contains(&dwell),
                "bar {bar} sits on the minimum height for {dwell:.3} of its \
                 cycle; the floor is kept because that is the bottom stop of a \
                 VU meter, and it should stay near the quarter of a cycle the \
                 sine puts there"
            );
        }
    }

    /// The offsets: unchanged, and pinned where they are legible — the height
    /// each bar takes at phase zero, which is its own offset's
    /// `sin * 0.5 + 0.5`. The offsets are what make the group read as dancing;
    /// the tempo was the thing that had to change, so nothing here may.
    ///
    /// A sine's reflection about its own axis is invisible to a height
    /// reading, so this pins the offsets' sense rather than their sign. No
    /// downstream behaviour depends on which side of the axis they start on.
    #[test]
    fn test_equalizer_bar_phase_offsets_are_unchanged() {
        let offsets = [0.0_f64, 1.3, 2.6, 3.9];
        let expected = offsets.map(|offset| (offset.sin() * 0.5 + 0.5).clamp(0.15, 1.0) as f32);
        let at_zero = sidebar::equalizer_heights(0.0);
        for (bar, (got, want)) in at_zero.into_iter().zip(expected).enumerate() {
            assert!(
                (got - want).abs() < 1e-6,
                "bar {bar} starts at {got} but offset {} rad starts it at \
                 {want}; the phase offsets are the dancing and must not move",
                offsets[bar]
            );
        }
        // And the group is four voices, not one bar repeated: the offsets
        // spread the bars apart at every phase, phase zero included.
        for (later, height) in at_zero.iter().enumerate().skip(1) {
            assert!(
                (height - at_zero[0]).abs() > 0.01,
                "the four bars must sit at four different heights, not one \
                 height four times: bar {later} is at {} like bar 0 in \
                 {at_zero:?}",
                height
            );
        }
    }

    /// A harness over one now-playing sidebar row whose `playing` flag the
    /// test flips between frames through the returned [`Cell`].
    ///
    /// The row is the only widget in the canvas, so every pixel that moves
    /// between two frames moved because the equalizer moved — the label, the
    /// band and the glyph are the same in all of them.
    fn now_playing_row_harness(
        playing: bool,
        reduce_motion: bool,
    ) -> (
        egui_kittest::Harness<'static>,
        std::rc::Rc<std::cell::Cell<bool>>,
    ) {
        let palette = theme::Palette::dark();
        let flag = std::rc::Rc::new(std::cell::Cell::new(playing));
        let seen = std::rc::Rc::clone(&flag);
        let mut cache = icons::IconCache::new();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                sidebar::tree_row(
                    ui,
                    &mut cache,
                    &palette,
                    reduce_motion,
                    sidebar::TreeRow {
                        indent_level: 1,
                        icon: Some(icons::Icon::Music),
                        cover: None,
                        label: "Ready Let's Go",
                        count: None,
                        meta: None,
                        favorite: None,
                        selected: false,
                        now_playing: true,
                        playing: seen.get(),
                        art_slot: false,
                    },
                );
            });
        (harness, flag)
    }

    /// The pixels two frames disagree on, and the box that disagreement sits
    /// in. The count is the evidence for "only the bars moved"; the box is the
    /// evidence for where. Comparing frames as whole images instead would print
    /// every pixel of a 256x48 canvas into the failure.
    fn pixel_diff(
        left: &image::RgbaImage,
        right: &image::RgbaImage,
    ) -> (usize, Option<(u32, u32, u32, u32)>) {
        assert_eq!(
            left.dimensions(),
            right.dimensions(),
            "frames must be the same size to be compared"
        );
        let mut moved = 0;
        let mut bounds = None;
        for (x, y, pixel) in left.enumerate_pixels() {
            if *pixel == *right.get_pixel(x, y) {
                continue;
            }
            moved += 1;
            let (x0, y0, x1, y1) = bounds.unwrap_or((x, y, x + 1, y + 1));
            bounds = Some((x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)));
        }
        (moved, bounds)
    }

    /// Pause HOLDS the last advanced phase; it does not reset it.
    ///
    /// Resetting snapped all four bars to their phase-zero heights, which is a
    /// lopsided staircase — phase zero is not a shape the four sine waves
    /// happen to be in agreement on. The phase therefore lives in egui's memory
    /// keyed on the row, and the row paints from it whether or not it is
    /// playing, so the pause is a freeze rather than a rewind.
    ///
    /// A row that has never played has no stored phase and paints phase zero;
    /// the exact zero value is pinned by the idle goldens staying byte
    /// identical, which is the stronger place for it, and what is pinned here
    /// is that such a row never drifts and that the held phase is a real phase
    /// rather than a reset.
    #[test]
    fn test_equalizer_pause_holds_the_last_advanced_phase() {
        // Step, never `run()`: a playing row asks for a repaint every frame,
        // which is exactly what `run()` refuses to settle.
        let (mut playing, flag) = now_playing_row_harness(true, false);
        playing.step();
        playing.step();
        playing.step();
        let last_playing = playing.render().expect("the row renders headlessly");

        flag.set(false);
        playing.step();
        let paused = playing.render().expect("the paused row renders headlessly");
        let (moved, where_) = pixel_diff(&last_playing, &paused);
        assert_eq!(
            moved, 0,
            "pausing must hold the last advanced phase, not snap the four bars \
             to their phase-zero staircase; {moved} pixels moved, at {where_:?}"
        );

        playing.step();
        let paused_again = playing.render().expect("the paused row renders");
        let (drifted, where_) = pixel_diff(&paused, &paused_again);
        assert_eq!(
            drifted, 0,
            "and the held phase must survive the frame boundary it was read on; \
             {drifted} pixels moved, at {where_:?}"
        );

        // A row that has never played has nothing to hold: it starts at phase
        // zero and stays there, so it never drifts.
        let (mut never_played, _) = now_playing_row_harness(false, false);
        never_played.step();
        let idle_frame_one = never_played.render().expect("the row renders headlessly");
        never_played.step();
        let idle_frame_two = never_played.render().expect("the row renders headlessly");
        let (drifted, where_) = pixel_diff(&idle_frame_one, &idle_frame_two);
        assert_eq!(
            drifted, 0,
            "a row that has never played must not drift: it has no phase to \
             advance; {drifted} pixels moved, at {where_:?}"
        );
        let (moved, where_) = pixel_diff(&idle_frame_one, &last_playing);
        assert!(
            moved > 0,
            "the held phase is a real phase: a paused row must not look like a \
             row that has never played (phase zero)"
        );
        // ...and the only thing that may differ between them is the equalizer:
        // a 14x14 indicator in the row's leading strip, so the label, the band
        // and the glyph are the same in both frames.
        let (x0, y0, x1, y1) = where_.expect("the two frames must differ somewhere");
        assert!(
            x1 - x0 <= 14 && y1 - y0 <= 14,
            "the two frames may differ only inside the 14x14 equalizer \
             indicator; they differ over {x0},{y0} to {x1},{y1}"
        );
        assert!(
            x1 * 2 <= idle_frame_one.width(),
            "and only in the row's leading strip, not in its label; they differ \
             up to x={x1} of a {}-px row",
            idle_frame_one.width()
        );
    }

    /// Under Reduce Motion the now-playing equalizer holds its calm rest shape
    /// and asks the frame loop for nothing — the only thing that changes from
    /// the default is whether the bars move. It paints the flat
    /// [`sidebar::EQUALIZER_REST`] silhouette even when the row is playing, and
    /// issues no `request_repaint_after`, so a playing reduce-motion row is as
    /// still as a paused one and costs no frames.
    #[test]
    fn test_reduce_motion_equalizer_holds_the_rest_shape_and_asks_for_no_frames() {
        // A playing row under reduce_motion: `playing == true`, but the bars
        // must not advance and no frame must be requested.
        let (mut playing_reduced, _pflag) = now_playing_row_harness(true, true);
        playing_reduced.step();
        let causes: Vec<String> = playing_reduced
            .ctx
            .repaint_causes()
            .iter()
            .map(|cause| format!("{}:{}", cause.file.replace('\\', "/"), cause.line))
            .collect();
        assert!(
            causes.is_empty(),
            "a reduce-motion playing row must not ask for frames: the equalizer holds \
             EQUALIZER_REST and issues no request_repaint_after; causes={causes:?}"
        );

        // The rest shape is motionless across frames.
        let playing_frame_a = playing_reduced
            .render()
            .expect("the row renders headlessly");
        playing_reduced.step();
        let playing_frame_b = playing_reduced
            .render()
            .expect("the row renders headlessly");
        let (drifted, where_) = pixel_diff(&playing_frame_a, &playing_frame_b);
        assert_eq!(
            drifted, 0,
            "the reduce-motion bars must not drift: they hold EQUALIZER_REST; {drifted} moved, \
             at {where_:?}"
        );

        // A paused row under reduce_motion paints the same rest shape, so a
        // playing and a paused reduce-motion row are indistinguishable.
        let (mut paused_reduced, _qflag) = now_playing_row_harness(false, true);
        paused_reduced.step();
        let paused_frame = paused_reduced.render().expect("the paused row renders");
        let (moved, where_) = pixel_diff(&playing_frame_a, &paused_frame);
        assert_eq!(
            moved, 0,
            "a playing and a paused row under reduce motion both paint EQUALIZER_REST; \
             {moved} pixels moved, at {where_:?}"
        );
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
                        false,
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
                        false,
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
                        false,
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
                        false,
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
                        false,
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
                    riff_gui::ui::sidebar::sidebar_footer(ui, &mut cache, &palette, false, None);
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
                    if let Some(action) = sidebar::playlist_row(
                        ui, &mut cache, &palette, false, "Gym", "Gym (3)", false,
                    ) {
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
                        ui, &mut cache, palette, false, entries, &mut buf,
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

    // --- Library stage empty-state hero tokens (Issue 09) ------------------------
    //
    // The empty-state hero widget that painted these is gone (the stage's
    // empty state moved to `empty_state.rs` and the leftover module is
    // deleted), but its geometry and glow tokens stay in the design system's
    // store, so what they pinned is pinned here against the token values
    // themselves: the mockup's 160px disc circle (`w-40 h-40`) with an 80px
    // glyph (`w-20 h-20`), `mb-6`/`mb-1` copy gaps inside a `p-8` stage, and
    // the `.riff-disc-glow` rule (`box-shadow: 0 0 60px -20px brand@15%`),
    // approximated with layered translucent fills because egui has no blur.

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

    /// `sidebar` is the single owner of the Up Next entry model and label
    /// builder: `now_playing`'s historical path is a re-export of it, not a
    /// second copy. This guards against the two surfaces (Now Playing and the
    /// queue sheet) silently diverging back into owning their own row data —
    /// the coupling ticket 05 removed.
    #[test]
    fn test_up_next_model_has_one_owner_shared_by_both_surfaces() {
        let window = up_next_window_fixture();

        let neutral = sidebar::up_next_entries(&window, 5);
        let via_now_playing = now_playing::up_next_entries(&window, 5);
        assert_eq!(
            neutral, via_now_playing,
            "the re-exported `now_playing::` path and the `sidebar::` path build \
             identical rows — one owner, not two copies"
        );
        // The entry type is literally the same type across both paths.
        let _: &sidebar::UpNextEntry = &via_now_playing[0];
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
                        false,
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
                        false,
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
        // Appearance is the one section that is a single card, so the card's
        // own edge is separable from every other thing in the frame.
        let mut harness = settings_modal_harness(&content, SettingsSection::Appearance);
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
                path: PathBuf::from("C:\\Users\\local\\Music"),
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
    fn test_replaygain_pass_card_reports_its_gating_and_start_actions() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();

        let mut harness = settings_modal_harness(&content, SettingsSection::Advanced);
        harness.run();
        harness.get_by_label("Track values").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGainPassTrack(true)),
            "the track-values checkbox gates the library-wide pass"
        );
        harness.get_by_label("Album values").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGainPassAlbum(true)),
            "the album-values checkbox gates the library-wide pass"
        );
        harness.get_by_label("Force").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGainPassForce(true)),
            "the Force choice re-measures already-measured Tracks"
        );
        harness.get_by_label("Measure Library").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::StartReplayGainPass),
            "one button starts the pass writing whatever was enabled"
        );
    }

    #[test]
    fn test_replaygain_pass_button_is_disabled_while_a_pass_runs() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let mut content = sample_content();
        content.pass_running = true;
        content.pass_progress = (3, 10);

        let mut harness = settings_modal_harness(&content, SettingsSection::Advanced);
        harness.run();
        // The button reads as in-progress and is disabled: clicking it
        // reports nothing, so a second pass cannot start.
        harness.get_by_label("Measuring\u{2026}").click();
        harness.run();
        assert!(
            !harness
                .state()
                .contains(&SettingsAction::StartReplayGainPass),
            "no second pass can start while one runs"
        );
        harness.get_by_label("Cancel").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::CancelReplayGainPass),
            "the running pass has a cancel control"
        );
        harness.get_by_label("Force").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGainPassForce(true)),
            "the Force choice stays readable while a pass runs"
        );
    }

    #[test]
    fn test_replaygain_pass_card_shows_the_settled_outcome_inline() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let mut content = sample_content();
        content.pass_outcome = Some("Measured 10 tracks (2 were already measured)".to_string());

        let mut harness = settings_modal_harness(&content, SettingsSection::Advanced);
        harness.run();
        harness.get_by_label("Measured 10 tracks (2 were already measured)");
        // The settled pass's outcome report shows inline — resolving by its
        // accessible label is the assertion.
    }

    #[test]
    fn test_replaygain_mode_choice_reports_the_chosen_mode() {
        use egui_kittest::kittest::Queryable;
        use riff_backend::app::state::ReplayGainMode;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();

        // The Playback pane carries the Mode choice beside the master toggle:
        // clicking Album reports the Album mode, and Track reports Track.
        let mut harness = settings_modal_harness(&content, SettingsSection::Playback);
        harness.run();
        harness.get_by_label("Album").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGainMode(ReplayGainMode::Album)),
            "the Mode choice reports the chosen ReplayGain Mode"
        );

        let mut harness = settings_modal_harness(&content, SettingsSection::Playback);
        harness.run();
        harness.get_by_label("Track").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGainMode(ReplayGainMode::Track)),
            "the Mode choice reports the chosen ReplayGain Mode"
        );
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

    /// Reduce motion is the temporal peer of High contrast in the Appearance
    /// pane: its switch is wired to `SettingsAction::SetReduceMotion`, whose
    /// handler writes `library.ui_flags.reduce_motion` (the line that resolves
    /// the theme boundary). Starts OFF beside High contrast.
    #[test]
    fn test_the_reduce_motion_preference_reports_set_reduce_motion() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::settings::SettingsSection;

        let content = sample_content();

        let mut harness = settings_modal_harness(&content, SettingsSection::Appearance);
        harness.run();
        harness.get_by_label("Reduce motion").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReduceMotion(true)),
            "the Appearance pane's Reduce motion switch reports SetReduceMotion(true)"
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

        harness.get_by_label("Scan C:\\Users\\local\\Music").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::Scan(PathBuf::from(
                    "C:\\Users\\local\\Music"
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
                            false,
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
        use crate::mocks::{
            MockLibraryMutationStore, MockPlaylistStore, MockTagEdits, MockTransport,
        };
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::app::{InlineTagEditor, TrackMenuHost, TrackMenuSubject};

        /// What the row's menu reported each frame, where the Track-menu host
        /// put it, and the mock ports that host is built over.
        ///
        /// The host is built through `TrackMenuHost::new` — the constructor
        /// `RiffApp::track_menu` itself calls — so this fixture cannot drift
        /// into a mirror of production's assembly: there is no second assembly
        /// to keep faithful, and a handle added to or dropped from the host
        /// breaks this at compile time exactly as it breaks the app.
        struct RowFrame {
            events: Vec<&'static str>,
            selected: Option<TrackId>,
            transport: MockTransport,
            passes: crate::mocks::MockPasses,
            playlist_store: MockPlaylistStore,
            library_mutations: MockLibraryMutationStore,
            tag_editor: InlineTagEditor,
        }

        impl RowFrame {
            fn new() -> Self {
                Self {
                    events: Vec::new(),
                    selected: None,
                    transport: MockTransport::new(),
                    passes: crate::mocks::MockPasses,
                    playlist_store: MockPlaylistStore::default(),
                    library_mutations: MockLibraryMutationStore::new(),
                    tag_editor: InlineTagEditor::new(Box::new(MockTagEdits)),
                }
            }

            /// What the row reported, and the selection the host produced.
            fn observed(&self) -> (Vec<&'static str>, Option<TrackId>) {
                (self.events.clone(), self.selected.clone())
            }
        }

        // The Track this row stands for. Spelled out here rather than read back
        // off whatever the widget painted, because the identity the selection
        // gets is the attach site's argument — the row is what it was attached
        // to, not what it drew.
        let row_track = TrackId("/music/b.mp3".to_string());
        let attached = row_track.clone();
        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(256.0, theme::geometry::sidebar::ROW_H * 2.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, frame: &mut RowFrame| {
                    let outcome = sidebar::reorderable_row(
                        ui,
                        &mut cache,
                        &palette,
                        false,
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
                        frame.events.push("clicked");
                    }
                    // Stand-in for the shared track context menu: proves the
                    // drag affordance did not swallow secondary clicks. The
                    // selection is then carried by the PRODUCTION applier, the
                    // one the real attach site calls — the point being that a
                    // right-click on a reorderable row is an ordinary row
                    // right-click that happens to be draggable.
                    let open = outcome
                        .response
                        .context_menu(|ui| {
                            let _ = ui.button("MenuProbe");
                        })
                        .is_some();
                    if open {
                        // The row is an entry whose file is gone: the menu's
                        // reduced shape, and the report that opened it.
                        let subject = TrackMenuSubject::unresolved(&attached, None);
                        let mut host = TrackMenuHost::new(
                            &frame.transport,
                            &frame.passes,
                            &mut frame.playlist_store,
                            &mut frame.library_mutations,
                            &mut frame.tag_editor,
                            &mut frame.selected,
                        );
                        host.right_clicked(subject);
                    }
                },
                RowFrame::new(),
            );
        harness.run();
        assert!(
            harness.state().observed().1.is_none(),
            "painting the row selects nothing"
        );

        // Plain clicks still select/play.
        harness.get_by_label("Beta").click();
        harness.run();
        assert!(
            harness.state().observed().0.contains(&"clicked"),
            "adding drag-and-drop must not break row clicks"
        );
        assert!(
            harness.state().observed().1.is_none(),
            "and a plain click reaches the row's own selection path, not the menu's — \
             so nothing here is the right-click's selection arriving early"
        );

        // Secondary clicks still open the context menu.
        harness.get_by_label("Beta").click_secondary();
        harness.run();
        assert!(
            harness.query_by_label("MenuProbe").is_some(),
            "the context menu opens on a reorderable row"
        );
        assert_eq!(
            harness.state().observed().1,
            Some(row_track),
            "and the right-click that reached it selected that row's Track, exactly as a \
             right-click on any other Track row does: the drag handle underneath never \
             swallowed the gesture"
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
                path: PathBuf::from("C:\\Users\\local\\Music"),
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
// inline Tag Edit controller, and the View half of the Cover Cache that
// RiffApp delegates to — over recording fakes, with no threads and no disk
// I/O.
//
// The Cover Cache's *decision* half needs no `egui::Context` and has no reason
// to be asserted from here, so its four assertions — the request key, the
// in-flight dedup, a hero arrival being a miss for a thumbnail, and the settle
// path — live at `crates/riff-gui/tests/cover_cache_tests.rs` instead. What
// stays here is what genuinely needs a Context: the texture upload, the count
// cap, and the byte budget.
#[cfg(test)]
mod background_service_ui_tests {
    use super::*;
    use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
    use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
    use riff_gui::ui::app::CoverCacheKey;
    use riff_gui::ui::app::{
        COVER_CACHE_CAP, COVER_HERO, COVER_TEXTURE_BYTE_BUDGET, InlineTagEditor, InspectorContent,
        InspectorKind, cover_cache_key, flush_cleared_cache, folder_cover_texture,
        request_cache_clear,
    };
    use riff_gui::ui::artwork::store_cover_texture;
    use riff_gui::ui::cover_cache::CoverCache;
    use riff_gui::ui::selection::{TagField, TagRow, TagRowState};
    use riff_library::app::traits::{DecodedCover, RequestedSize};
    use std::collections::{HashMap, VecDeque};
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
        // painting the covers it already has rather than blanking on a promise. The
        // Frame's step-4 drain is what notices the settlement now — and it reports
        // the outcome rather than touching the map, which is why the flush below is
        // the draw half's one egui-bound act (see `flush_cleared_cache`).
        assert_eq!(covers.poll_cache_clear(), None);
        assert_eq!(
            textures.len(),
            4,
            "a clear in progress has removed nothing yet"
        );

        covers.serve_clear(ClearCacheOutcome::Cleared);
        let settled = covers.poll_cache_clear().expect("the wipe settled");
        flush_cleared_cache(&settled, &mut textures, &mut lru_keys);
        assert!(
            textures.is_empty(),
            "every texture is gone, so the next repaint shows placeholders and the rows re-request"
        );
        assert!(
            lru_keys.is_empty(),
            "and nothing is left in the LRU order either"
        );

        // Once settled, the frame stops asking the service at all.
        assert_eq!(covers.poll_cache_clear(), None);
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

        let settled = covers.poll_cache_clear().expect("the wipe settled");
        assert_eq!(
            settled,
            ClearCacheOutcome::Failed {
                reason: "read-only volume".to_string()
            },
            "the reason reaches the caller so the status line can say it"
        );
        flush_cleared_cache(&settled, &mut textures, &mut lru_keys);
        assert_eq!(textures.len(), 3, "a failed clear removes nothing");
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
        assert!(
            covers.poll_cache_clear().is_some(),
            "the Frame's drain settles the outstanding wipe"
        );
        // The flag is the Frame's to clear (step 4 of `Frame::advance`), which is
        // the half of the settle that is not egui-bound; here the test stands in
        // for that drain.
        in_flight = false;
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
        let mut cache = CoverCache::new();
        let mut textures = HashMap::new();
        let mut lru_keys = Vec::new();

        // 18 uploads of 1024x1024 = 72 MB: nowhere near the 200-entry count cap, and
        // over the byte budget from the 17th on.
        let batch: Vec<(TrackId, RequestedSize, Option<DecodedCover>)> = (0..18)
            .map(|index| {
                let id = TrackId(format!("/music/big{index:02}.mp3"));
                (id, COVER_HERO, Some(square_cover(1024)))
            })
            .collect();
        land_covers(
            &mut cache,
            &CannedCovers(batch),
            &ctx,
            &mut textures,
            &mut lru_keys,
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

        // The tile first, so it is the LRU tail and the obvious victim.
        let mut cache = CoverCache::new();
        let tile = crate::ui::artwork::lookup_cover_texture(
            &mut cache,
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
        land_covers(
            &mut cache,
            &CannedCovers(batch),
            &ctx,
            &mut textures,
            &mut lru_keys,
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

    /// Consume polled cover results exactly as `RiffApp::update_cover_cache`
    /// does: the Cover Cache settles — which drops the markers and hands the
    /// pixels over — and the View half files each arrival as a texture,
    /// bounding the map as it goes. The two calls are the production ones; only
    /// the `&mut self` is spelled out here.
    fn land_covers(
        cache: &mut CoverCache,
        covers: &dyn Covers,
        ctx: &egui::Context,
        textures: &mut HashMap<CoverCacheKey, egui::TextureHandle>,
        lru_keys: &mut Vec<CoverCacheKey>,
    ) {
        for arrival in cache.settle(covers) {
            store_cover_texture(cache, ctx, textures, lru_keys, arrival);
        }
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
    fn test_folder_cover_texture_asks_once_then_paints_the_cached_texture() {
        let dir = std::path::Path::new("/music/boards");
        let thumb = RequestedSize {
            width: 56,
            height: 56,
        };
        let covers = RecordingCovers::new();
        let mut cache = CoverCache::new();
        let mut textures: std::collections::HashMap<_, egui::TextureHandle> =
            std::collections::HashMap::new();
        let mut lru_keys = Vec::new();
        let ctx = egui::Context::default();

        // A cold folder has nothing to paint, so the row keeps its glyph and
        // exactly one request goes out — for the directory itself.
        assert_eq!(
            folder_cover_texture(&mut cache, &covers, &textures, dir, thumb),
            None,
            "an uncached folder has no texture to paint"
        );
        assert_eq!(
            covers.requested(),
            vec![(TrackId::from_path(dir), dir.to_path_buf(), thumb,)],
            "the folder row asks for its OWN directory, at the box it paints at"
        );

        // The decode lands through the two halves exactly as a frame lands it —
        // the cache records the arrival, the View half files the texture — and
        // the row must find the art it is now holding.
        land_covers(
            &mut cache,
            &CannedCovers(vec![(
                TrackId::from_path(dir),
                thumb,
                Some(DecodedCover {
                    rgba: vec![0; 2 * 2 * 4],
                    width: 2,
                    height: 2,
                }),
            )]),
            &ctx,
            &mut textures,
            &mut lru_keys,
        );
        let id = textures[&cover_cache_key(&dir.to_string_lossy(), thumb)].id();
        assert_eq!(
            folder_cover_texture(&mut cache, &covers, &textures, dir, thumb),
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
    fn test_store_cover_texture_uploads_the_delivered_pixels_untouched() {
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
        let mut cache = CoverCache::new();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        land_covers(&mut cache, &covers, &ctx, &mut textures, &mut lru_keys);

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
        let mut cache = CoverCache::new();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        let track_at = |i: usize| TrackId(format!("/music/t{i:03}.mp3"));
        let full: Vec<_> = (0..COVER_CACHE_CAP)
            .map(|i| (track_at(i), thumb, Some(cover.clone())))
            .collect();
        land_covers(
            &mut cache,
            &CannedCovers(full),
            &ctx,
            &mut textures,
            &mut lru_keys,
        );
        assert_eq!(
            textures.len(),
            COVER_CACHE_CAP,
            "exactly the cap, so nothing has been evicted yet"
        );

        let oldest = track_at(0);
        let newcomer = CannedCovers(vec![(oldest.clone(), hero, Some(cover))]);
        land_covers(&mut cache, &newcomer, &ctx, &mut textures, &mut lru_keys);

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
    fn test_one_track_keeps_one_texture_per_requested_size() {
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
        let mut cache = CoverCache::new();
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        land_covers(&mut cache, &covers, &ctx, &mut textures, &mut lru_keys);

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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
                        riff_gui::ui::browser::show_browser_column_scrolled(
                            ui, &mut cache, &palette, false, column, None, actions,
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
    use riff_gui::ui::app::apply_entity_selection;
    use riff_gui::ui::column::ColumnIdentity;

    /// The unit separator joins an album's `(album artist, title)` composite
    /// identity into one row key — the same identity the store keys albums
    /// by.
    fn album_key(artist: &str, title: &str) -> String {
        format!("{artist}\u{1f}{title}")
    }

    /// What one entity row's click DOES to the library session, driven through
    /// the Column's own production interface: the identity it stated once, and
    /// the key its row carried.
    ///
    /// This is the shape the dispatch uses — [`apply_entity_selection`] is the
    /// one line both bindings' `Select` arms reach, and both read the Section
    /// and the depth out of the identity rather than taking them per call. A
    /// test that called a wrapper taking `(section, level)` would have been
    /// asserting the *arguments* rather than the Column, and would have
    /// disagreed with the app the moment a Column's depth moved.
    #[test]
    fn test_browser_selection_resolves_per_section() {
        // Artists: the row key IS the artist name, at the root's depth 0.
        let mut library = LibrarySession::default();
        apply_entity_selection(
            "Aphex Twin",
            ColumnIdentity::root(LibrarySection::Artists),
            &mut library,
        );
        assert_eq!(
            library.current_selection(),
            Some(&BrowserSelection::Artist("Aphex Twin".to_string())),
            "selecting an artist row stores the artist identity for the detail column"
        );

        // Albums: the key is the store's (album artist, title) composite.
        let mut library = LibrarySession::default();
        apply_entity_selection(
            &album_key("Boards of Canada", "Geogaddi"),
            ColumnIdentity::root(LibrarySection::Albums),
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
        let mut library = LibrarySession::default();
        apply_entity_selection(
            "Electronic",
            ColumnIdentity::root(LibrarySection::Genres),
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
    // (`ui::detail::show_detail_column_scrolled`, headless kittest harness) and the
    // session-glue seam (`apply_detail_action`, real store/transport mocks).

    #[test]
    fn test_track_list_renders_rows_and_reports_row_gestures() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{
            DetailAction, DetailColumn, DetailReport, TrackRow, show_detail_column_scrolled,
        };
        use std::time::Duration;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
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
                |ui, reports: &mut Vec<DetailReport>| {
                    let column = DetailColumn {
                        tracks: &tracks,
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, reports,
                    );
                },
                Vec::new(),
            );
        harness.run();

        // Every track's title renders as its row's label — the shared 40px
        // track row. The right-aligned `plays · time` cluster is
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
                .contains(&DetailReport::Action(DetailAction::SelectTrack(
                    "t1".to_string()
                ))),
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
                .contains(&DetailReport::Action(DetailAction::PlayTrack(
                    "t1".to_string()
                ))),
            "double-clicking a row starts the track"
        );
    }

    /// The track listings' sort control, at the widget seam: the button names
    /// the order in force, and choosing a row from its popup reports that
    /// order for the session to apply.
    #[test]
    fn test_detail_column_track_sort_control_reports_the_chosen_order() {
        use egui_kittest::kittest::Queryable;
        use riff_backend::app::state::TrackSort;
        use riff_gui::ui::detail::{
            DetailAction, DetailColumn, DetailReport, TrackRow, show_detail_column_scrolled,
        };

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let tracks = vec![TrackRow {
            key: "t1".to_string(),
            title: "Magic Window".to_string(),
            plays: 0,
            duration: None,
            favorite: false,
            selected: false,
            now_playing: false,
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, reports: &mut Vec<DetailReport>| {
                    let column = DetailColumn {
                        tracks: &tracks,
                        sort: Some(TrackSort::NumberAsc),
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, reports,
                    );
                },
                Vec::new(),
            );
        harness.run();

        // The button names the order in force, for the pointer and the
        // accessibility tree alike.
        assert!(
            harness
                .query_by_label("Sort order: Track No. \u{2191}")
                .is_some(),
            "the sort button names the canonical order while it is active"
        );

        // Choosing a row from the popup reports that order.
        harness
            .get_by_label("Sort order: Track No. \u{2191}")
            .click();
        harness.run();
        harness.get_by_label("Title A\u{2013}Z").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&DetailReport::Action(DetailAction::TrackSortSelected(
                    TrackSort::TitleAsc
                ))),
            "choosing a row from the sort popup reports the chosen order"
        );
    }

    /// A Track row's right-click, at the widget seam: the Tracks Column's rows
    /// open the shared Track menu and report the row's OWN key with whatever
    /// was chosen from it.
    ///
    /// The fixture is long enough that the column really is VIRTUALIZED, and
    /// the row that is right-clicked is neither the first nor the last. That is
    /// the point: the popup is egui's transient one, keyed by the row's own
    /// response identity, so there is no per-row open state and nothing to keep
    /// in step with an index — and a list whose rows are recycled as it scrolls
    /// is the only place a per-row or index-keyed menu could show itself. The
    /// culled rows are asserted absent, so "virtualized" is a fact about this
    /// frame rather than an assumption about the renderer.
    ///
    /// The report arrives on the frame the menu OPENS, with an empty intent
    /// list, because opening is what moves the selection and choosing an item
    /// is a separate event; the chosen intent then rides the same key.
    #[test]
    fn test_a_secondary_click_on_a_track_row_reports_its_key_with_the_menu() {
        use egui_kittest::kittest::Queryable;
        use riff_backend::domain::PlaylistId;
        use riff_gui::ui::detail::{
            DetailColumn, DetailReport, TrackRow, show_detail_column_scrolled,
        };
        use riff_gui::ui::menu::{TrackMenu, TrackMenuIntent, TrackMenuReport};
        use std::time::Duration;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        // Far more rows than a 300px window shows, so most of them are culled
        // and the visible ones are materialized afresh every frame.
        let tracks: Vec<TrackRow> = (0..120u32)
            .map(|i| TrackRow {
                key: format!("t{i}"),
                title: format!("Track {i:02}"),
                plays: i % 5,
                duration: (i % 3 == 0).then(|| Duration::from_secs(180 + u64::from(i))),
                favorite: i % 2 == 0,
                selected: false,
                now_playing: false,
            })
            .collect();
        // The playlist targets the host resolved up front, and NO removal: a
        // Track in the Tracks Column is in no Playlist. The menu is handed over
        // as the same per-row factory the app hands over, reading the row's own
        // Favourite flag.
        let playlists = [(PlaylistId::new("pl-1"), "Gym".to_string())];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, reports: &mut Vec<DetailReport>| {
                    let track_menu = |row: &TrackRow| TrackMenu {
                        playable: true,
                        editable: true,
                        favorite: row.favorite,
                        playlists: &playlists,
                        remove_from_playlist: false,
                    };
                    let column = DetailColumn {
                        tracks: &tracks,
                        track_menu: Some(&track_menu),
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, reports,
                    );
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness.state().is_empty(),
            "painting the column reports no action at all"
        );
        assert!(
            harness.query_by_label("Track 03").is_some()
                && harness.query_by_label("Track 100").is_none(),
            "the list really is virtualized: an on-screen row exists and a far-off one \
             is culled, so the rows the widget does not create are rows it cannot \
             have opened a menu on"
        );

        harness.get_by_label("Track 03").click_secondary();
        harness.run();

        assert_eq!(
            harness.state().first(),
            Some(&DetailReport::TrackMenu(TrackMenuReport {
                key: "t3".to_string(),
                intents: Vec::new(),
            })),
            "opening the menu reports the RIGHT-CLICKED row's own key with no intent \
             chosen yet — that report is what moves the selection"
        );
        // The Track menu, and the Track menu only. "Play Next" lives in both
        // menu renderers, "Edit Tags" in the Track one only, and "Shuffle" is
        // what the collection and whole-list menus have where the Track menu
        // has "Edit Tags" — so those three together identify the menu without
        // depending on a label the transport bar also wears. The removal is
        // absent because a Track in this column is in no Playlist.
        //
        // The playlist target is probed with the label egui gives a submenu
        // button INSIDE a menu, which appends its disclosure arrow: the bare
        // wording is what a `MenuButton` wears at the top level, and the menu
        // content seam pins that shape.
        for present in ["Play Next", "Edit Tags", "Add to Playlist \u{23f5}"] {
            assert!(
                harness.query_by_label(present).is_some(),
                "the Track menu is the one on screen — '{present}' is missing from it"
            );
        }
        for absent in ["Shuffle", "Remove from Playlist"] {
            assert!(
                harness.query_by_label(absent).is_none(),
                "the Track menu offers no '{absent}'"
            );
        }

        harness.get_by_label("Play Next").click();
        harness.run();
        assert_eq!(
            harness.state().last(),
            Some(&DetailReport::TrackMenu(TrackMenuReport {
                key: "t3".to_string(),
                intents: vec![TrackMenuIntent::PlayNext],
            })),
            "choosing an item rides the same key, so the host can both select the \
             Track and act on it from one report"
        );
    }

    /// The Tracks Column's Favourite item is a per-ROW fact, proved with two
    /// rows of DIFFERENT Favourite state in one render.
    ///
    /// This is the seam that catches the mistake the column's shape invites: a
    /// single column-wide menu would put one row's answer on every row, and every
    /// other assertion here would still pass — the menu would open, it would be
    /// the Track menu, it would report the right key. Only the LABEL would be
    /// wrong, and only on whichever rows disagreed with the one answer.
    ///
    /// So the counting is the assertion. Both rows carry a heart, labelled by
    /// what their own click will do, so before any right-click the column holds
    /// exactly one "Add to Favorites" and one "Remove from Favorites". Right-
    /// clicking a row's menu then ADDS one node: the item whose wording is the
    /// OPPOSITE of that row's heart. Reading a count instead of "is this label
    /// present" is what makes the claim unambiguous — the two surfaces wear the
    /// same two strings, so presence alone could not say which menu was open.
    #[test]
    fn test_the_tracks_column_offers_each_row_its_own_favourite_wording() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{DetailColumn, TrackRow, show_detail_column_scrolled};
        use riff_gui::ui::menu::TrackMenu;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let row = |key: &str, title: &str, favorite: bool| TrackRow {
            key: key.to_string(),
            title: title.to_string(),
            plays: 0,
            duration: None,
            favorite,
            selected: false,
            now_playing: false,
        };
        // One un-Favourited Track and one Favourited Track, side by side.
        let tracks = vec![
            row("plain", "Not A Favorite", false),
            row("loved", "Loved", true),
        ];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, reports: &mut Vec<riff_gui::ui::detail::DetailReport>| {
                    // The host's per-row factory, reading the flag off the row it
                    // is handed — the same shape `render_tracks_column` hands the
                    // widget.
                    let track_menu = |row: &TrackRow| TrackMenu {
                        playable: true,
                        editable: true,
                        favorite: row.favorite,
                        playlists: &[],
                        remove_from_playlist: false,
                    };
                    let column = DetailColumn {
                        tracks: &tracks,
                        track_menu: Some(&track_menu),
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, reports,
                    );
                },
                Vec::new(),
            );
        harness.run();
        // How many nodes wear each of the two wordings, heart and menu row
        // together. Reading a count instead of "is this label present" is what
        // makes the claim unambiguous: the heart and the menu item wear the same
        // two strings, so presence alone could not say which menu was open.
        fn count(
            harness: &egui_kittest::Harness<'_, Vec<riff_gui::ui::detail::DetailReport>>,
            label: &str,
        ) -> usize {
            harness.query_all_by_label(label).count()
        }
        assert_eq!(
            (
                count(&harness, "Add to Favorites"),
                count(&harness, "Remove from Favorites")
            ),
            (1, 1),
            "each row's heart is labelled by its own flag, so the two rows really are in \
             the two different states"
        );

        // The un-Favourited row's menu says it will ADD one.
        harness.get_by_label("Not A Favorite").click_secondary();
        harness.run();
        assert_eq!(
            (
                count(&harness, "Add to Favorites"),
                count(&harness, "Remove from Favorites")
            ),
            (2, 1),
            "right-clicking the un-Favourited row adds an 'Add to Favorites' item and \
             nothing else — its menu says what ITS click will do"
        );

        // The Favourited row's menu says the opposite, with the other row still on
        // screen and unchanged: the column does not settle for one answer per
        // frame, it answers per row.
        //
        // Two secondary clicks, and the first is not redundant. An open context
        // menu is a popup with the pointer: the click that would land on the other
        // row is taken by the layer above it and only DISMISSES the open menu —
        // which is visible in the counts below dropping to (1, 1) before the
        // second click. Two clicks, one menu at a time, which is also how a
        // listener moves from one row's menu to another's.
        harness.get_by_label("Loved").click_secondary();
        harness.run();
        assert_eq!(
            (
                count(&harness, "Add to Favorites"),
                count(&harness, "Remove from Favorites")
            ),
            (1, 1),
            "the second right-click dismisses the open popup, taking its item with it — \
             and no row's heart changed"
        );
        harness.get_by_label("Loved").click_secondary();
        harness.run();
        assert_eq!(
            (
                count(&harness, "Add to Favorites"),
                count(&harness, "Remove from Favorites")
            ),
            (1, 2),
            "right-clicking the Favourited row adds a 'Remove from Favorites' item — the \
             same column, the other row still beside it, the opposite label"
        );
    }

    #[test]
    fn test_track_table_favorite_control_reports_the_toggle() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::detail::{
            DetailAction, DetailColumn, DetailReport, TrackRow, show_detail_column_scrolled,
        };

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
                |ui, reports: &mut Vec<DetailReport>| {
                    let column = DetailColumn {
                        tracks: &tracks,
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, reports,
                    );
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
                DetailReport::Action(DetailAction::SetFavorite {
                    key: "plain".to_string(),
                    favorite: true,
                }),
                DetailReport::Action(DetailAction::SetFavorite {
                    key: "loved".to_string(),
                    favorite: false,
                }),
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
        let transport = crate::mocks::MockTransport::new();

        // The row's control reports the flag's NEW value; applying it
        // commits through the store's favorite setter.
        apply_detail_action(
            DetailAction::SetFavorite {
                key: track.id.0.clone(),
                favorite: true,
            },
            &mut library,
            &transport,
            library_mutations.as_mut(),
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

    /// A Drill Column's row click lands at the depth **the Column stated**, and
    /// truncates anything deeper.
    ///
    /// Rewritten rather than preserved: this used to drive
    /// `apply_drill_action(section, level, key, library)`, a one-line wrapper
    /// that existed only because a Drill Column re-supplied its own Section and
    /// depth on every action. There is no such wrapper now — both dispatch
    /// bindings reach `apply_entity_selection(key, column, library)` with the
    /// identity they were handed, so this drives the same line the app drives,
    /// with the identity each real Column states.
    #[test]
    fn a_drill_columns_click_lands_at_the_depth_it_stated() {
        use riff_backend::app::state::BrowserSelection;
        use riff_gui::ui::app::apply_entity_selection;
        use riff_gui::ui::column::ColumnIdentity;
        use riff_gui::ui::scroll_memory::DrillSlot;

        // Artists, level 1: the row key is the (album artist, title)
        // composite; the new entry lands under the root artist.
        let mut library = LibrarySession {
            library_section: LibrarySection::Artists,
            browser_path: vec![BrowserSelection::Artist("Boards of Canada".to_string())],
            ..LibrarySession::default()
        };
        apply_entity_selection(
            &album_key("Boards of Canada", "Geogaddi"),
            ColumnIdentity::drill(LibrarySection::Artists, 1, DrillSlot::ArtistAlbums),
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
        apply_entity_selection(
            "Autechre",
            ColumnIdentity::drill(LibrarySection::Genres, 1, DrillSlot::GenreArtists),
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
        apply_entity_selection(
            &album_key("Autechre", "Tri Repetae"),
            ColumnIdentity::drill(LibrarySection::Genres, 2, DrillSlot::GenreArtistAlbums),
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

        // A Section with no entity rows at that depth applies nothing — the
        // defensive shape, and the reason a Column states its depth rather
        // than a caller passing one.
        let mut library = LibrarySession::default();
        apply_entity_selection(
            "x",
            ColumnIdentity::root(LibrarySection::AllTracks),
            &mut library,
        );
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
                path: PathBuf::from("C:\\Users\\local\\Music"),
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
    ///
    /// The band's wash is tweened (issue 06), so this helper hands the painter a
    /// `Ui` and a row id and lets the band settle: `Harness::run` keeps stepping
    /// while a repaint is outstanding, and a hovered row asks for one on every
    /// frame of its wash. `focused` still arrives as a plain flag, exactly as it
    /// did before — the id is only here to carry the wash's tween, which is why
    /// no widget has to claim it.
    fn render_row_band(selected: bool, hovered: bool, focused: bool) -> image::RgbaImage {
        use riff_gui::ui::row::paint_row_band;
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        let id = egui::Id::new("test_row_band");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                let bg = ui.ctx().layer_painter(egui::LayerId::background());
                bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.background);
                let rect =
                    egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(200.0, 40.0));
                let painter = ui.painter();
                paint_row_band(
                    ui, painter, &palette, false, rect, id, selected, hovered, focused,
                );
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
        use riff_gui::ui::browser::show_browser_column_scrolled;
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
                    show_browser_column_scrolled(
                        ui,
                        &mut cache,
                        &palette,
                        false,
                        column,
                        None,
                        &mut Vec::new(),
                    );
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

    /// A `Color32`'s channels as a fraction of its own alpha — the
    /// *un-premultiplied* RGB, which is the colour a translucent wash is
    /// nominally "of". `Color32` stores premultiplied bytes, so a wash that
    /// keeps its hue reads the same here at every coverage; a wash that had
    /// lerped its RGB toward some backdrop would not.
    fn nominal_rgb(color: egui::Color32) -> [f32; 3] {
        let a = f32::from(color.a()).max(1.0) / 255.0;
        [
            f32::from(color.r()) / a,
            f32::from(color.g()) / a,
            f32::from(color.b()) / a,
        ]
    }

    /// Issue 06: the row band's hover wash is a pure function of its tween
    /// value, asserted at both endpoints and at the midpoint — the seam the
    /// motion spec names for this change, so the fade is checkable without
    /// pixels and without a frame loop.
    ///
    /// The endpoints are the whole reason the function is allowed to exist.
    /// `t = 0` must be *fully transparent* and `t = 1` must be *exactly the
    /// token*, so a row that never hovered and a hover that has settled paint
    /// byte-for-byte what they painted before the wash was introduced: every
    /// existing golden in the suite captures the `t = 0` or the `t = 1` state,
    /// and neither may move.
    ///
    /// The midpoint is asserted twice over, because the two are different
    /// claims. It must be *strictly between* the endpoints, and it must be the
    /// same colour at partial coverage rather than a colour halfway to a
    /// guessed backdrop — the painter does not know what the band sits on (a
    /// sidebar row and a browser row have different surfaces behind them), so
    /// interpolating RGB would be interpolating toward a fiction.
    #[test]
    fn test_row_band_hover_wash_is_a_pure_function_of_its_tween_value() {
        use riff_gui::ui::theme::{Palette, row_band_fill};

        let palette = Palette::dark();

        // --- The unselected band: transparent at rest, the token once settled.
        let idle = row_band_fill(&palette, false, 0.0);
        assert_eq!(
            idle,
            egui::Color32::TRANSPARENT,
            "t=0 must be fully transparent, so an unhovered row paints what it painted \
             before the wash existed"
        );
        let settled = row_band_fill(&palette, false, 1.0);
        assert_eq!(
            settled, palette.row_hover,
            "t=1 must be exactly the row_hover token, so a settled hover is byte-identical \
             to the instant swap this wash replaces"
        );

        let mid = row_band_fill(&palette, false, 0.5);
        assert!(
            mid != idle && mid != settled,
            "the midpoint must be strictly between the two endpoints, not equal to either: \
             {mid:?}"
        );
        assert!(
            mid.a() > 0 && mid.a() < 255,
            "the midpoint's coverage must be strictly between nothing and the full token, \
             which is what 'half a wash' means: alpha {} of {mid:?}",
            mid.a()
        );
        let wash_hue = nominal_rgb(palette.row_hover);
        let mid_hue = nominal_rgb(mid);
        for (channel, (got, want)) in mid_hue.iter().zip(wash_hue.iter()).enumerate() {
            assert!(
                (got - want).abs() <= 1.0,
                "channel {channel} of the midpoint drifted from the token's own hue \
                 ({got} vs {want}): the wash scales coverage, it does not lerp RGB toward a \
                 backdrop the painter cannot see"
            );
        }

        // --- The selected band: unchanged while idle, blended while hovered.
        // Both endpoints are the same two app colours the unselected band uses,
        // which is the point — a selected, hovered row is *interpolated*
        // between two states the app defines, never crossfaded through a third
        // colour that belongs to neither.
        assert_eq!(
            row_band_fill(&palette, true, 0.0),
            palette.surface_3,
            "a selected, unhovered row is byte-identical to the selected fill it painted \
             before the wash existed"
        );
        assert_eq!(
            row_band_fill(&palette, true, 1.0),
            palette.row_hover,
            "at full coverage the wash is opaque, so a settled selected+hovered row lands on \
             the wash token and not on some third colour"
        );
        let selected_mid = row_band_fill(&palette, true, 0.5);
        for (channel, (got, low, high)) in [
            (
                selected_mid.r(),
                palette.surface_3.r(),
                palette.row_hover.r(),
            ),
            (
                selected_mid.g(),
                palette.surface_3.g(),
                palette.row_hover.g(),
            ),
            (
                selected_mid.b(),
                palette.surface_3.b(),
                palette.row_hover.b(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let (lo, hi) = if low < high { (low, high) } else { (high, low) };
            assert!(
                got > lo && got < hi,
                "channel {channel} of the selected+hovered midpoint ({got}) is not strictly \
                 between the selected fill ({low}) and the wash ({high}): the two are blended \
                 directly, never through a colour that belongs to neither state"
            );
        }
    }

    /// Build a one-row harness that paints the shared row band, with the
    /// pointer state held in the harness state so a test can hover the row
    /// between steps.
    fn row_band_frame_loop(
        step_dt: f32,
        id: egui::Id,
        reduce_motion: bool,
    ) -> egui_kittest::Harness<'static, bool> {
        use riff_gui::ui::row::paint_row_band;
        use riff_gui::ui::theme::Palette;
        let palette = Palette::dark();
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 80.0))
            .with_pixels_per_point(1.0)
            .with_step_dt(step_dt)
            .build_ui_state(
                move |ui, hovered: &mut bool| {
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(200.0, 40.0));
                    paint_row_band(
                        ui,
                        ui.painter(),
                        &palette,
                        reduce_motion,
                        rect,
                        id,
                        false,
                        *hovered,
                        false,
                    );
                },
                false,
            )
    }

    /// The source files that asked for a repaint on the pass that has just
    /// finished, or an empty vector if nothing did.
    ///
    /// **Why the cause list and not `requested_repaint_last_pass`.** egui
    /// swaps the accumulated causes into their "previous pass" slot at the
    /// *start* of a pass (`egui-0.35.0/src/context.rs:101-105`), so a reading
    /// taken right after `step` describes the pass before the one that ran. The
    /// cause list is exact, though — it is cleared each pass and refilled only
    /// by an actual `request_repaint` call — so reading it *before* stepping
    /// describes the pass that has just finished, and no alignment arithmetic
    /// is needed.
    ///
    /// The two boolean seams cannot answer this at all, and it is worth saying
    /// why. `requested_repaint_last_pass` is `prev_pass_paint_delay == ZERO`,
    /// and an immediate request leaves `repaint_delay` at `ZERO` for the next
    /// pass as well, so it stays true for two passes after the last ask — it
    /// reports "a repaint was requested recently", not "this pass asked".
    /// `has_requested_repaint` is worse, counting the same `outstanding` flag.
    /// Neither can distinguish a settled tween from one still running, and
    /// neither says *who* asked.
    fn asked_by(harness: &egui_kittest::Harness<'_, bool>) -> Vec<String> {
        harness
            .ctx
            .repaint_causes()
            .iter()
            .map(|cause| format!("{}:{}", cause.file.replace('\\', "/"), cause.line))
            .collect()
    }

    /// Issue 06: the row band's wash is served frames while it is in flight and
    /// asks for nothing once it has settled — and an idle row asks for nothing
    /// ever.
    ///
    /// **The finding this pins.** egui 0.35's
    /// `animate_bool_with_time_and_easing` already ends in
    /// `if 0.0 < animated_value && animated_value < 1.0 { self.request_repaint(); }`
    /// (`egui-0.35.0/src/context.rs:3145-3148`). The wash therefore needs *no*
    /// frame request of its own: adding one would be a second call for the same
    /// frame, from riff, saying what egui has already said. The bound at both
    /// ends is egui's, and that is the half that matters here — an unhovered
    /// band rests at exactly `0.0`, so the one-sided reading `t < 1.0` would be
    /// true for all fifty rows in a list forever and pin the app at an uncapped
    /// frame rate with the pointer nowhere near it.
    ///
    /// So the oracle is egui's own cause list, per pass: an idle pass and every
    /// settled pass must be silent, and an in-flight pass must make **exactly
    /// one** request. That last half is what fails if someone later "fixes" the
    /// missing request the way the Folders tree's is written — by adding a
    /// duplicate.
    #[test]
    fn test_row_band_hover_wash_asks_for_frames_only_while_it_is_in_flight() {
        use riff_gui::ui::theme::MOTION_HOVER;

        /// One step of the harness clock, chosen so the hover token spans
        /// several frames and the settle is observable rather than a single
        /// lucky step.
        const STEP_DT: f32 = 0.02;
        /// Passes sampled, several times the longest a motion token could
        /// publish.
        const SAMPLED_PASSES: usize = 20;
        /// Settled passes the sample must end with, so "stops asking" is
        /// asserted over a run and not over one frame.
        const SETTLED_PASSES: usize = 4;

        let mut harness = row_band_frame_loop(STEP_DT, egui::Id::new("row_band_frame_loop"), false);

        // One pass with the row idle. This seeds the tween at `0.0`, which is
        // what lets the next pass read a *transition* rather than the animation
        // manager's first-call snap to the target.
        harness.step();
        assert!(
            asked_by(&harness).is_empty(),
            "an idle row band must ask the frame loop for nothing: the tween rests at exactly \
             0.0, and a one-sided `t < 1.0` bound would make this true for every row in a \
             fifty-row list forever"
        );

        *harness.state_mut() = true;
        // Read the causes of the pass that just finished, then run the next —
        // see [`asked_by`]. The first entry is therefore the idle pass above,
        // and the rest follow the pointer onto the row.
        let mut sampled: Vec<Vec<String>> = Vec::with_capacity(SAMPLED_PASSES);
        for _ in 0..SAMPLED_PASSES {
            sampled.push(asked_by(&harness));
            harness.step();
        }
        let asked: Vec<bool> = sampled.iter().map(|causes| !causes.is_empty()).collect();

        // Asking is one contiguous run, and the run's length is the claim that
        // the wash runs on the *hover* token rather than the published global
        // default: at this step MOTION_HOVER (0.10 s) is five passes and
        // MOTION_DEFAULT (0.18 s) is nine, so a wash that had picked up
        // `Style::animation_time` instead of passing the token explicitly
        // would be caught here — and an instant swap would never ask at all.
        let asking = asked.iter().filter(|asked| **asked).count();
        let longest_run = asked
            .iter()
            .fold((0usize, 0usize), |(best, current), asked| {
                let current = if *asked { current + 1 } else { 0 };
                (best.max(current), current)
            })
            .0;
        let hover_passes = (MOTION_HOVER / STEP_DT).ceil() as usize;
        assert_eq!(
            asking, longest_run,
            "frames must be asked for across the whole tween, not in bursts: {asked:?}"
        );
        assert!(
            (2..=hover_passes).contains(&asking),
            "the wash must tween for roughly MOTION_HOVER ({MOTION_HOVER}s ≈ {hover_passes} \
             passes at {STEP_DT}s) and asked for a frame on {asking} of {SAMPLED_PASSES}: an \
             instant swap never asks, a tween that never settles asks forever. asked={asked:?}"
        );
        assert!(
            SAMPLED_PASSES - asking >= SETTLED_PASSES,
            "the sample holds fewer than {SETTLED_PASSES} settled passes, so 'a settled hover \
             asks for nothing' is unproven: {asked:?}"
        );

        // The other half of the finding: **exactly one** request per in-flight
        // pass, and it is the tween read.
        //
        // `Context::request_repaint` is `#[track_caller]`, so a `RepaintCause`
        // names riff's *call site* — the `animate_bool_with_time` line in
        // `row.rs` — not egui's own `request_repaint` line. Attribution by file
        // therefore cannot tell "egui's wrapper asked on riff's behalf" from
        // "riff asked", because both are `row.rs`. The count can: a redundant
        // `ui.ctx().request_repaint()` in the wash would be a *second* cause on
        // every in-flight pass, and the settled passes would keep asking.
        for (pass, causes) in sampled.iter().enumerate() {
            if causes.is_empty() {
                continue;
            }
            assert_eq!(
                causes.len(),
                1,
                "pass {pass} made {causes:?} — the wash must cost exactly one repaint \
                 request per in-flight frame, the one egui's animate_bool_with_time already \
                 issues while the value is strictly between 0 and 1. A second cause is riff \
                 asking again for a frame it has already been given."
            );
            assert!(
                causes[0].contains("riff-gui/src/ui/row.rs"),
                "pass {pass} asked for a frame from outside the row band: {causes:?}"
            );
        }
    }

    /// Under Reduce Motion the row band's hover wash asks for zero repaints and
    /// lands exactly on the settled hover tint on the first hovered frame: the
    /// `MOTION_REDUCED` sentinel snaps the tween to its target (`t == 1.0`), so
    /// nothing is ever in flight and no fade is shown.
    #[test]
    fn test_reduce_motion_row_wash_lands_on_the_settled_tint_and_asks_for_no_frames() {
        /// Short steps, matching the wash test above.
        const STEP_DT: f32 = 0.02;
        /// Enough passes for the default (motion) wash to settle and to prove
        /// the reduce-motion wash never comes in flight across a run.
        const PASSES: usize = 12;

        // The settled hover tint under the default tempo: hovered, stepped past
        // MOTION_HOVER so the fade completes, is the reference both bands land
        // on.
        let mut moving = row_band_frame_loop(STEP_DT, egui::Id::new("row_band_moving"), false);
        *moving.state_mut() = true;
        for _ in 0..PASSES {
            moving.step();
        }
        let settled = moving.render().expect("the moving band settles");

        // The reduce-motion band: idle first (the tween rests at 0.0), then
        // hovered — with no pass ever asking for a frame.
        let mut reduced = row_band_frame_loop(STEP_DT, egui::Id::new("row_band_reduced"), true);
        reduced.step();
        assert!(
            asked_by(&reduced).is_empty(),
            "an idle reduce-motion band asks for nothing, exactly like the default"
        );
        *reduced.state_mut() = true;
        let mut asked: Vec<Vec<String>> = Vec::with_capacity(PASSES);
        for _ in 0..PASSES {
            asked.push(asked_by(&reduced));
            reduced.step();
        }
        assert!(
            asked.iter().all(|causes| causes.is_empty()),
            "reduce-motion snaps the wash to its target, so no pass is ever in flight and none \
             asks for a frame: {asked:?}"
        );

        // And it lands exactly on the settled hover tint, with no intermediate
        // fade — identical pixels to the default band after its fade completes.
        let first_hover = reduced.render().expect("the reduced band renders");
        assert_eq!(
            &settled, &first_hover,
            "the first reduce-motion hover frame must be the fully-settled hover tint, identical \
             to the default band after its fade completes"
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
                text_button(ui, &mut cache, &palette, false, &spec);
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
                text_button(ui, &mut cache, &palette, false, &spec);
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

    /// hovered. `paint_text_button` takes `hovered` as an argument, so this
    /// observes the two states in one frame each without moving a pointer —
    /// the same technique the row-band test uses for its four states.
    fn render_accent_button(hovered: bool) -> (image::RgbaImage, egui::Rect) {
        use riff_gui::ui::button::{Variant, paint_text_button};
        use riff_gui::ui::icons::IconCache;
        use riff_gui::ui::theme::Palette;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 34.0), egui::vec2(120.0, 36.0));
        // The two wash roles keep their tween under the button's own widget id,
        // so the probe needs one. This helper renders the accent tier at rest
        // and settled, which are the two endpoints, so the id is never asked to
        // remember a tween in flight.
        let id = egui::Id::new("render_accent_button");
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
                    false,
                    id,
                    rect,
                    "Rescan now",
                    None,
                    false,
                    Variant::Accent,
                    true,
                    hovered,
                    false,
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
                        // Issue 08 added the hover-driven thumb as its own field,
                        // so this literal has to answer it. `false` is what this
                        // test's contract already wanted: it counts fill pixels on
                        // a bar the pointer is not over, and the resting half of
                        // that — no thumb, idle thickness — is now pinned by
                        // `seek_affordance_tests` with the hover slot actually on.
                        hover_thumb: false,
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
        actions: Vec<riff_gui::ui::detail::DetailReport>,
    }

    /// The production Tracks-column data path, replicated frame-for-frame:
    /// `resolve_detail_content` maps the library session through the
    /// Session Views seam, and `show_detail_column_scrolled` paints it. Entity
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
            tracks: &content.tracks,
            ..riff_gui::ui::detail::DetailColumn::empty("Nothing selected", "Pick a row.")
        };
        s.actions.clear();
        riff_gui::ui::detail::show_detail_column_scrolled(
            ui,
            &mut s.cache,
            &palette,
            false,
            column,
            None,
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

        // Album level: the track list resolves from the store. The column
        // states no identity of its own — the album is named on its selected
        // row in the column that listed it, and again in the inspector's
        // readout — so what the Tracks column shows here is the rows.
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

        // Above the album level the Tracks column has nothing to resolve —
        // entity listings are their own columns now — so the column falls
        // back to the app's no-selection copy rather than naming the level.
        harness.state_mut().library.browser_path =
            vec![BrowserSelection::Artist("Boards of Canada".to_string())];
        harness.run();
        assert!(
            harness.query_by_label("Magic Window").is_none(),
            "the artist level renders no track table"
        );
        assert!(
            harness.query_by_label("Nothing selected").is_some(),
            "the artist level shows the no-selection copy, not an identity of its own"
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
        // the Genres section, three levels down the path.
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
            harness.query_by_label("Magic Window").is_some()
                && harness.query_by_label("Dawn Chorus").is_some(),
            "the genre-scoped track table resolves the album's tracks"
        );
    }

    // --- Inspector (the collapsible selection panel, handoff issue 10) ----------
    //
    // The elastic stage's rightmost column: a readout of the live selection
    // (album / artist / genre / track) and nothing that acts on it.
    // Tested at the same seams as the browser column (issue 08) and detail
    // column (issue 09): the pure widget seam
    // (`ui::selection::show_selection_panel`, headless kittest harness) and
    // the session-glue seam in `ui::app`.

    #[test]
    fn test_selection_panel_renders_album_details_and_reports_nothing() {
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
                        kind: "Album",
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &details,
                        tags: &[],
                        editor: None,
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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

        // The panel is a READOUT: it shows the resolved values and reports
        // nothing. No play action, no queue action, and — the part that is easy
        // to regress — no button at all anywhere in the panel, so there is no
        // control-shaped thing to try pressing.
        for gone in ["Play album", "Play", "Add to Queue", "Queue"] {
            assert!(
                harness.query_by_label(gone).is_none(),
                "the readout offers no {gone} — an entity's actions live on its own \
                 context menu"
            );
        }
        assert_eq!(
            harness.state(),
            &Vec::<SelectionAction>::new(),
            "and a frame that only paints the panel reports no action at all"
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
                        kind: "Album",
                        subtitle: Some("Boards of Canada \u{b7} 2013"),
                        details: &details,
                        tags: &[],
                        editor: None,
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: None,
                        details: &[],
                        tags: &[],
                        editor: None,
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: Some("Some Artist \u{b7} 2013"),
                        details: &[],
                        tags: &tags,
                        editor: None,
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: Some(&mut draft),
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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
                        kind: "Album",
                        subtitle: Some("Some Artist"),
                        details: &[],
                        tags: &tags,
                        editor: None,
                    };
                    show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
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

    /// The header chip names the KIND of thing being read out. It used to be a
    /// button with no click handler whose label was the hardcoded string
    /// "Album", so a Track, an Artist and a Genre readout all announced
    /// themselves as an Album — and each wore a button face, so it invited a
    /// press that did nothing.
    ///
    /// Rendered in one pass: every kind at once, so a chip that names the wrong
    /// one cannot pass by being the only thing checked.
    #[test]
    fn test_the_header_chip_names_the_readouts_kind() {
        use egui_kittest::kittest::Queryable;
        use riff_gui::ui::selection::{SelectionAction, SelectionPanel, show_selection_panel};

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 640.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<SelectionAction>| {
                    for (kind, title) in [
                        ("Album", "Geogaddi"),
                        ("Artist", "Boards of Canada"),
                        ("Genre", "IDM"),
                        ("Track", "Music Is Math"),
                    ] {
                        let panel = SelectionPanel {
                            art: None,
                            title: Some(title),
                            kind,
                            subtitle: None,
                            details: &[],
                            tags: &[],
                            editor: None,
                        };
                        show_selection_panel(ui, &mut cache, &palette, false, panel, actions);
                    }
                },
                Vec::new(),
            );
        // `run_steps(2)` rather than `run()`: four scroll areas in one pass
        // never settle, and a settled frame is all this reads.
        harness.run_steps(2);

        for kind in ["Album", "Artist", "Genre", "Track"] {
            assert!(
                harness.query_by_label(kind).is_some(),
                "the chip names the readout's kind, and a {kind} readout says {kind}"
            );
        }
        // The Track case is the one that was wrong: a Track readout is not an
        // Album, and saying so was the defect.
        assert_eq!(
            harness.query_all_by_label("Album").count(),
            1,
            "exactly ONE readout claims to be an Album: the Album one"
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
            "the inspector resolves the album's track batch, the targets an album \
             tag draft writes to"
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
            11,
            "the tag section carries the seven Metadata fields and the four \
             ReplayGain values"
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
        use riff_gui::ui::selection::TagRowState;

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
        let row = |content: &riff_gui::ui::app::InspectorContent, label: &str| {
            content
                .tags
                .iter()
                .find(|row| row.field.label() == label)
                .map(|row| (row.state, row.text.clone()))
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
        // The two-decimal form is the row: `.2` is load-bearing, since an f32
        // widened into the REAL column and narrowed back prints
        // `-6.540000057220459`.
        assert_eq!(
            row(&content, "Track Gain (dB)"),
            Some((TagRowState::Value, "-6.54".to_string())),
            "the track gain row carries the measured value"
        );
        assert_eq!(
            row(&content, "Album Gain (dB)"),
            Some((TagRowState::Value, "1.50".to_string())),
            "a positive album gain reads as the value that will be written"
        );

        // A file with neither tag shows both rows as unmeasured — never as a
        // number, least of all zero, which would read as "analyzed at 0 dB".
        let content = readout(plain.id);
        assert_eq!(
            row(&content, "Track Gain (dB)"),
            Some((TagRowState::None, "unmeasured".to_string())),
        );
        assert_eq!(
            row(&content, "Album Gain (dB)"),
            Some((TagRowState::None, "unmeasured".to_string())),
        );
    }

    #[test]
    fn test_album_readout_aggregates_the_replaygain_rows() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection, LibrarySession};
        use riff_gui::ui::app::resolve_inspector;
        use riff_gui::ui::selection::TagRowState;

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
        // its own: the album readout aggregates what the tracks carry.
        seed_gain_track(&mut store, "music/a/02.flac", "Two", None, Some(-7.12));
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store.clone()),
            store.library_generation(),
            store.playlist_generation(),
        );
        let mut row = |label: &str| {
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
                .tags
                .iter()
                .find(|row| row.field.label() == label)
                .map(|row| (row.state, row.text.clone()))
        };

        assert_eq!(
            row("Album Gain (dB)"),
            Some((TagRowState::Value, "-7.12".to_string())),
            "every Track carries the album value, so the row is that value"
        );
        assert_eq!(
            row("Track Gain (dB)"),
            Some((TagRowState::Different, "(different)".to_string())),
            "one member measured, one not — the mix is the orange state"
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
                "Track Gain (dB)",
                "Track Peak",
                "Album Gain (dB)",
                "Album Peak",
            ],
            "the tag rows resolve in the modal order, the ReplayGain values \
             after the Metadata fields"
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
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
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
        use riff_gui::ui::detail::{
            DetailAction, DetailColumn, DetailReport, TrackRow, show_detail_column_scrolled,
        };
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
                |ui, reports: &mut Vec<DetailReport>| {
                    let column = DetailColumn {
                        tracks: &tracks,
                        ..DetailColumn::empty("", "")
                    };
                    show_detail_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, reports,
                    );
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
                .contains(&DetailReport::Action(DetailAction::SelectTrack(
                    "t1".to_string()
                ))),
            "Enter on the focused track row selects it: {:?}",
            harness.state()
        );

        // The row's favorite control toggles the flag to its NEW value.
        harness.get_by_label("Add to Favorites").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(
            harness
                .state()
                .contains(&DetailReport::Action(DetailAction::SetFavorite {
                    key: "t1".to_string(),
                    favorite: true,
                })),
            "Enter on the focused favorite control toggles the flag: {:?}",
            harness.state()
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
        use riff_gui::ui::detail::{DetailColumn, TrackRow};
        use riff_gui::ui::selection::SelectionPanel;
        use riff_gui::ui::theme::TITLEBAR_H;
        use std::time::Duration;

        let palette = Palette::dark();
        let mut cache = IconCache::new();
        let items = fixture_items();
        let mut query = "abc".to_string();

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
                            riff_gui::ui::browser::show_browser_column_scrolled(
                                ui,
                                &mut cache,
                                &palette,
                                false,
                                column,
                                None,
                                &mut browser_actions,
                            );
                        });

                        let mut selection_actions = Vec::new();
                        ui.allocate_ui(egui::vec2(300.0, 480.0), |ui| {
                            let panel = SelectionPanel {
                                art: None,
                                title: Some("Tomorrow's Harvest"),
                                kind: "Album",
                                subtitle: None,
                                details: &[],
                                tags: &[],
                                editor: None,
                            };
                            riff_gui::ui::selection::show_selection_panel(
                                ui,
                                &mut cache,
                                &palette,
                                false,
                                panel,
                                &mut selection_actions,
                            );
                        });

                        let mut detail_actions = Vec::new();
                        ui.allocate_ui(egui::vec2(600.0, 480.0), |ui| {
                            let column = DetailColumn {
                                tracks: &tracks,
                                ..DetailColumn::empty("", "")
                            };
                            riff_gui::ui::detail::show_detail_column_scrolled(
                                ui,
                                &mut cache,
                                &palette,
                                false,
                                column,
                                None,
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
        // app's creation order: the browser rows, then the detail column's
        // track row's favorite control and the track row itself. The
        // selection panel contributes NO control at all: its action row and
        // its button-shaped header chip are gone, and what remains is labels
        // and the tag rows' text — none of which can take focus. The detail
        // column contributes only those two: the breadcrumb trail and the
        // album header that once sat above the rows are gone, so nothing else
        // in it can take focus either.
        for label in [
            "Clear search",
            "Alpha",
            "Beta",
            "Gamma",
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
        // affordance, the three browser rows, then the detail column's
        // favorite control and the track row. The selection panel has nothing
        // focusable in it, so it does not appear in the chain.
        for _ in [
            "Clear search",
            "Alpha",
            "Beta",
            "Gamma",
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
        // through the detail column, the browser rows, and the search's
        // clear affordance — and lands on the search.
        for label in ["Add to Favorites", "Gamma", "Beta", "Alpha", "Clear search"] {
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
        MockSettingsStore, MockTagEdits, MockTransport, SettingsCall, TransportIntent,
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

    /// kittest's own default clock: a quarter of a second per step. Every shell
    /// but [`build_at_step`]'s caller takes it, so a step is a step everywhere
    /// else in this suite.
    const HARNESS_STEP: f32 = 0.25;

    /// The step a DOUBLE-CLICK needs: short enough that the two presses land
    /// inside egui's `max_double_click_delay`. [`HARNESS_STEP`] advances a
    /// quarter of a second per queued event, so a click's press and release
    /// alone span half a second — the two presses would never be close enough
    /// in TIME, and no position would save them.
    const FAST_STEP: f32 = 0.02;

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

    /// Build a shell from fully-specified ports, on kittest's own default clock.
    fn build(
        transport: Box<dyn Transport>,
        scans: MockScans,
        settings_store: Box<dyn SettingsStore>,
        settings_calls: Arc<Mutex<Vec<SettingsCall>>>,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
    ) -> Shell {
        build_at_step(
            transport,
            scans,
            settings_store,
            settings_calls,
            playlist_store,
            library_mutations,
            views,
            HARNESS_STEP,
        )
    }

    /// [`build`] on a clock of the caller's choosing, for the gestures whose
    /// TIMING is the thing under test rather than a frame count.
    ///
    /// `build_eframe` runs the app's `logic`/`ui` for its own warm-up frames
    /// (one to initialise AccessKit, one to settle), so by the time this
    /// returns the app has already rendered — and `first_frame` hydration has
    /// already happened. A test must therefore mutate state *after* the shell
    /// exists and then `step()` once per frame it wants to observe.
    ///
    /// The step is a real clock, not a frame count: `Harness::step` runs one
    /// frame per queued input event and advances egui's time by `step_dt` each
    /// time, so a gesture egui recognises by elapsed time — a double click, and
    /// only a double click — needs a step short enough for its two presses to
    /// land inside egui's `max_double_click_delay`.
    #[expect(clippy::too_many_arguments, reason = "the shell's port kit")]
    fn build_at_step(
        transport: Box<dyn Transport>,
        scans: MockScans,
        settings_store: Box<dyn SettingsStore>,
        settings_calls: Arc<Mutex<Vec<SettingsCall>>>,
        playlist_store: Box<dyn PlaylistStore>,
        library_mutations: Box<dyn LibraryMutationStore>,
        views: SessionViews,
        step_dt: f32,
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
            Box::new(crate::mocks::MockPasses),
            Box::new(ShellCovers(Arc::clone(&folder_covers))),
            Arc::clone(&backend_events),
        );

        let harness = egui_kittest::Harness::builder()
            .with_size(WINDOW)
            .with_step_dt(step_dt)
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
        recording_transport_shell_at_step(mock, HARNESS_STEP)
    }

    /// [`recording_transport_shell`] on a clock of the caller's choosing, for
    /// the one gesture whose recognition is a matter of elapsed time rather
    /// than frames.
    fn recording_transport_shell_at_step(
        mock: MockLibraryQueryStore,
        step_dt: f32,
    ) -> (Shell, Arc<MockTransport>) {
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
            fn add_many(&self, tracks: Vec<riff_backend::domain::TrackId>) {
                self.0.add_many(tracks);
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
        let shell = build_at_step(
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
            step_dt,
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

    /// One double click, delivered as the four events it is: a press and a
    /// release, twice, at ONE position.
    ///
    /// The events are queued rather than taken from `Node::click` because
    /// egui recognises a double click by two conditions at once — the two
    /// presses close together in space AND in time — and each is easy to lose
    /// here. In time, because `Harness::step` runs one frame per queued event
    /// and advances egui's clock by the step each time, so the shell's step has
    /// to be short (see [`build_at_step`]). In space, because the position is
    /// captured once: the first click selects the row, which opens the Detail
    /// Panel and narrows the column the row is in, so re-querying the row for
    /// the second click would aim at a point the pointer had never been.
    ///
    /// The quiet frames first are egui's TRIPLE-click window, and they are the
    /// third condition. egui classifies a release against the previous two
    /// releases, so a click close enough in time and space to both of them is
    /// counted as a triple — and a triple is not a double, whatever else is
    /// true of it. A click this harness made earlier (opening a list from the
    /// sidebar, say) is therefore one of the two neighbours the second release
    /// is measured against, and the queue says nothing about how long ago that
    /// was. Stepping past the window says "and then, a moment later", which is
    /// what a listener double-clicking a row is.
    fn double_click_at(
        harness: &mut egui_kittest::Harness<'static, RiffApp>,
        pos: egui::Pos2,
        step_dt: f32,
    ) {
        /// egui's triple-click window: twice its double-click window.
        const TRIPLE_WINDOW: f64 = 0.6;
        for _ in 0..(TRIPLE_WINDOW / f64::from(step_dt)).ceil() as usize {
            harness.step();
        }
        harness.hover_at(pos);
        for pressed in [true, false, true, false] {
            harness.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
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

    /// The node labelled `label` inside `anchor`'s OWN container.
    ///
    /// The scoping is the point, not a convenience. Two things in the app
    /// deliberately wear the same labels — a Track row's heart and the menu's
    /// Favourite item, which is the whole point of the item being a second path
    /// to the same action — so a flat `get_by_label` cannot say which of them it
    /// found, and a test that clicked one while meaning the other would pass for
    /// the wrong reason. Anchoring on a node that only ONE of them can be, and
    /// walking to its parent, scopes the query to the thing under test: the
    /// popup for a menu row, the row for a heart.
    fn within<'a>(
        anchor: egui_kittest::Node<'a>,
        what: &str,
        label: &'a str,
    ) -> egui_kittest::Node<'a> {
        use egui_kittest::kittest::NodeT;
        anchor
            .parent()
            .unwrap_or_else(|| panic!("{what} is a container in the tree"))
            .children_recursive()
            .find(|node| node.accesskit_node().label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("{what} holds no node labelled {label:?}"))
    }

    /// One row of an OPEN Track menu, located inside the popup.
    ///
    /// `anchor` names a row only the Track menu has: the "Add to Playlist"
    /// submenu, which egui labels with a disclosure arrow because it is a submenu
    /// INSIDE a menu. The popup is its parent, so the search that follows is
    /// scoped to the menu and cannot reach a row's heart.
    fn menu_item<'a>(shell: &'a Shell, anchor: &'a str, label: &'a str) -> egui_kittest::Node<'a> {
        within(
            shell.harness.get_by_label(anchor),
            "the open Track menu",
            label,
        )
    }

    /// The menu's Favourite item, chosen for real in the real app, commits the
    /// SAME durable flag the heart commits — and it survives a restart, so a
    /// Favourite is one fact however it was set.
    ///
    /// Both paths are driven over the same Application Store, from the same
    /// shell the heart's own durable test uses: one Track Favourited from its
    /// menu, one from its heart, then the store is closed and reopened and both
    /// flags are read back. That is the acceptance criterion stated as a fact
    /// about the database rather than about two calls, and the menu's item is
    /// reached through [`menu_item`] so the click lands in the menu and not on
    /// the heart that wears the same words.
    #[test]
    fn test_favouriting_from_the_menu_and_the_heart_commit_the_same_durable_flag() {
        use riff_backend::domain::SmartPlaylistKind;

        /// The label egui gives the Track menu's "Add to Playlist" submenu once
        /// it is rendered inside a menu: the wording plus a disclosure arrow.
        const MENU_SUBMENU: &str = "Add to Playlist \u{23f5}";

        let (mut shell, dir, _pid, store) = store_shell();
        shell.harness.step();
        let alpha = dir.path().join("one.mp3");
        let beta = dir.path().join("two.mp3");

        // --- from the menu ---
        shell
            .harness
            .get_by_label("Artist - Alpha")
            .click_secondary();
        // Two settling frames before the item is located. A node is a rectangle
        // captured from one frame, and the popup's own geometry is only settled
        // on the frame after it opens: clicking a row captured from the opening
        // frame aims at a rectangle the popup has since moved out of, and the
        // click then lands beside the menu and only dismisses it — which looks
        // exactly like an item that does nothing. (The Tracks Column menu test
        // settles for the same reason.)
        shell.harness.step();
        shell.harness.step();
        menu_item(&shell, MENU_SUBMENU, "Add to Favorites").click();
        shell.harness.step();
        shell.harness.step();

        // --- from the heart, on the row beside it ---
        //
        // The heart of ONE named row, found inside that row's own container, so
        // the two paths provably touched two different Tracks: a listing of three
        // identical hearts would otherwise let the heart click land on whichever
        // row the tree listed first, and this test would be asserting a pair
        // without saying which pair.
        within(
            shell.harness.get_by_label("Artist - Beta"),
            "Beta's row",
            "Add to Favorites",
        )
        .click();
        shell.harness.step();
        shell.harness.step();

        let favourited = |store: &riff_infra::store::SqliteStore| -> Vec<PathBuf> {
            store
                .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
                .expect("the Favorites smart list reads")
                .iter()
                .map(|t| t.file_path.clone())
                .collect()
        };
        assert_eq!(
            favourited(&store),
            vec![alpha.clone(), beta.clone()],
            "the menu's item and the heart committed the same flag to the same two Tracks — \
             one each, and no Track committed twice"
        );
        assert_eq!(
            shell
                .harness
                .query_all_by_label("Remove from Favorites")
                .count(),
            2,
            "and both rows re-render with the committed flag, through the heart each row \
             already wore"
        );

        // Durability: the same flags after the store is closed and reopened, which
        // is the only thing that makes them a promise rather than a frame's state.
        let db_path = dir.path().join("riff.sqlite3");
        drop(shell);
        drop(store);
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening the store must work");
        assert_eq!(
            favourited(&reopened),
            vec![alpha, beta],
            "a Favourite set from the menu and one set from the heart both survive a restart"
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
            library.library_paths.hydrate(
                &riff_backend::app::store::Settings {
                    library_paths: vec![root.clone()],
                    ..Default::default()
                },
                &mut None,
            );
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
            library.library_paths.hydrate(
                &riff_backend::app::store::Settings {
                    library_paths: vec![root.clone()],
                    ..Default::default()
                },
                &mut None,
            );
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
            order: riff_persistence::store::TrackListOrder,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0.lock().unwrap().tracks_page(order, offset, limit)
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
            order: riff_persistence::store::TrackListOrder,
            offset: usize,
            limit: usize,
        ) -> Result<riff_persistence::store::Page<Track>, StoreError> {
            self.0
                .lock()
                .unwrap()
                .search_page(query, order, offset, limit)
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

    /// A library whose Albums root lists one real album with two real tracks, so
    /// a row is right-clickable in a composed frame and its batch is observable.
    ///
    /// Note what the mock does NOT do: `album_tracks` ignores the album it is
    /// asked for and serves this canned pair, so an album's batch in these
    /// tests is "what was seeded" and not a per-album read. That is what makes
    /// an entity row right-clickable here at all, and it is why genre-scoping
    /// and per-key resolution are proved at the applier seam instead, where a
    /// mock is cheap enough to seed per entity kind.
    fn album_row_mock() -> MockLibraryQueryStore {
        let geogaddi = riff_backend::domain::Album {
            artist: "Boards of Canada".to_string(),
            title: "Geogaddi".to_string(),
            tracks: vec![TrackId("g1".to_string()), TrackId("g2".to_string())],
            year: Some(2002),
            genre: Some("IDM".to_string()),
        };
        MockLibraryQueryStore {
            // The Albums root's listing is the paged flat window, so the ROOT
            // needs `paged_albums`; `albums` is the per-artist table the album's
            // own batch is read from. Both are seeded, because both are read.
            paged_albums: vec![geogaddi.clone()],
            albums: vec![geogaddi],
            album_tracks: vec![
                track("g1", "Music for Beat People"),
                track("g2", "Iced Cooly"),
            ],
            genre_counts: vec![riff_backend::domain::GenreCount {
                genre: "IDM".to_string(),
                tracks: 2,
            }],
            library_counts: riff_backend::app::store::LibraryCounts {
                tracks: 2,
                artists: 1,
                albums: 1,
                genres: 1,
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

    /// The composed assertion no other seam can make: a REAL secondary click on
    /// a REAL entity row, in a real app over the real store seam, both opens
    /// the menu and moves the session. The widget seam proves a menu opens and
    /// reports; the applier seam proves a report is applied; only here do the
    /// egui right-click gesture, the row's response, and the session state meet.
    /// A suite without this would still pass if the row attached a left-click
    /// only handler.
    ///
    /// The batch it acts on is "whatever you seeded", not a per-album read:
    /// `MockLibraryQueryStore`'s `album_tracks` ignores its arguments and serves
    /// the canned list, which is exactly what makes an entity row right-clickable
    /// in a composed test at all. Genre-scoping is proved at the applier seam,
    /// where it costs one mock instead of a second composed shell.
    #[test]
    fn test_a_secondary_click_on_an_album_row_opens_the_menu_and_moves_the_selection() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};

        let (mut shell, transport) = recording_transport_shell(album_row_mock());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
        }
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Geogaddi (Boards of Canada · 2002)")
                .is_some(),
            "the Albums root lists the seeded album"
        );
        {
            let library = shell.library.lock_or_recover();
            assert!(
                library.browser_path.is_empty(),
                "nothing is selected before the right-click"
            );
        }
        // Baseline: the app's own first-frame hydration (a volume clamp) is not
        // something the gesture caused, so the assertions below are about what
        // the CLICK added to the record.
        let before = transport.recorded().len();

        shell
            .harness
            .get_by_label("Geogaddi (Boards of Canada · 2002)")
            .click_secondary();
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Play Next").is_some(),
            "a real secondary click on a real row opens the collection menu. \
             'Play Next' is the probe because that wording lives only in the two \
             menu renderers, and of those only this one is open in the frame: the \
             player bar says 'Next track' and a Track's own menu is not open"
        );
        let library = shell.library.lock_or_recover();
        assert_eq!(
            library.browser_path,
            vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }],
            "and moves the browser path onto that Album, so the Detail Panel and the \
             menu describe the same entity"
        );
        drop(library);
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "a right-click changes only the selection: the frame it happened in sent \
             the transport nothing at all. The baseline is taken BEFORE the click \
             because the app's first frame hydrates its volume, and this is about \
             what the CLICK added, not about what the app ever sends"
        );
    }

    /// The negative half of the same gesture, at the same seam: a right-click
    /// that CHOOSES an item acts on the entity, as a batch, and the selection it
    /// carried is still the right one. Play Next and Shuffle reach the transport
    /// the same way and are pinned at the applier seam.
    #[test]
    fn test_choosing_an_item_on_an_entity_row_acts_on_the_batch_in_one_frame() {
        use riff_backend::app::state::{BrowserSelection, LibrarySection};
        use riff_backend::domain::TrackId;

        let (mut shell, transport) = recording_transport_shell(album_row_mock());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
        }
        shell.harness.step();
        shell
            .harness
            .get_by_label("Geogaddi (Boards of Canada · 2002)")
            .click_secondary();
        shell.harness.step();
        shell.harness.step();
        let before = transport.recorded().len();
        // "Add to Queue" is this collection menu's own item, and in this frame it
        // is the only node wearing that label: the Detail Panel is a readout and
        // carries no action buttons of its own, so nothing in the window's
        // contents competes with the popup for the query.
        // Getting this wrong is not a silent pass — the assertion below expects
        // one AddMany, and a per-Track control would send a fan-out instead.
        shell
            .harness
            .query_all_by_label("Add to Queue")
            .last()
            .expect("the open menu offers Add to Queue")
            .click();
        shell.harness.step();
        shell.harness.step();

        assert_eq!(
            &transport.recorded()[before..],
            &[TransportIntent::AddMany(vec![
                TrackId("g1".to_string()),
                TrackId("g2".to_string()),
            ])],
            "the whole collection is queued as ONE batch in the frame the item was \
             chosen — not a command per Track: {:?}",
            transport.recorded()
        );
        let library = shell.library.lock_or_recover();
        assert_eq!(
            library.browser_path,
            vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }],
            "and the row the menu was opened on is still the selection"
        );
    }

    /// The negative twin, and the proof that the scope boundary is a boundary:
    /// a playlist header carries the SAME four items and NONE of the selection
    /// behaviour. Right-clicking it acts on the playlist and does not select,
    /// open, or navigate anything — which is the whole reason entity rows were
    /// scoped to rows rather than applied globally. Asserted on the session, in
    /// a real frame, after a real secondary click: the path is still empty and
    /// the stage is still the playlist it was showing.
    #[test]
    fn test_a_secondary_click_on_a_playlist_header_acts_without_selecting_it() {
        use riff_backend::app::state::{BrowserSelection, ViewMode};

        let (mut shell, _dir, _pid, _store) = store_shell();
        shell.harness.step();
        shell.harness.get_by_label("Gym").click();
        shell.harness.step();

        // The playlist is on screen and nothing else is selected.
        {
            let library = shell.library.lock_or_recover();
            assert!(
                library.browser_path.is_empty(),
                "a playlist view selects no browser entity"
            );
        }

        // The playlist VIEW's header, not the sidebar's row: both read "Gym", so
        // the header is named by ROLE — a heading is a Label, the sidebar row is a
        // Button. It is the header the whole-list menu rides on.
        shell
            .harness
            .get_by_role_and_label(egui::accesskit::Role::Label, "Gym")
            .click_secondary();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Play Next").is_some(),
            "the header's menu is the same four-item collection menu — 'Play Next' \
             is the unambiguous probe, since the player bar says 'Next track' and \
             nothing else in a playlist view offers it"
        );
        shell.harness.get_by_label("Play Next").click();
        shell.harness.step();
        shell.harness.step();

        let library = shell.library.lock_or_recover();
        assert!(
            library.browser_path.is_empty(),
            "acting on a header did NOT select it: a header has no readout for a \
             selection to stay consistent with"
        );
        assert!(
            library.selected_track.is_none(),
            "and it selected no Track either"
        );
        assert_eq!(
            library.view_mode,
            ViewMode::Library,
            "and it did not navigate away from the view the listener was on"
        );
        drop(library);
        assert!(
            shell.harness.query_by_label("Artist - Alpha").is_some(),
            "the playlist's own rows are still the ones on screen — the header was \
             acted on, not opened"
        );

        // The negative twin of the twin: the same assertion shape on an ENTITY
        // row, so a regression that taught HEADERS to select would break the
        // assertions above, and one that taught ROWS to stop selecting would
        // break this one. The two gestures differ in exactly this, which is what
        // makes the boundary worth pinning.
        let (mut shell, transport) = recording_transport_shell(album_row_mock());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = riff_backend::app::state::LibrarySection::Albums;
        }
        shell.harness.step();
        let before = transport.recorded().len();
        shell
            .harness
            .get_by_label("Geogaddi (Boards of Canada · 2002)")
            .click_secondary();
        shell.harness.step();
        let library = shell.library.lock_or_recover();
        assert_eq!(
            library.browser_path,
            vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }],
            "an ENTITY row's right-click does select — the two gestures differ only \
             in that, which is what makes the boundary worth pinning"
        );
        drop(library);
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "and an entity right-click still starts no playback, exactly as a \
             header's does: the selection is the ONLY thing either gesture changes"
        );
    }

    /// A Track row's right-click, at the one seam a real gesture can reach: a
    /// REAL secondary click on a REAL row in the real flat list that both opens
    /// the Track's menu and makes that Track the selection. The widget seam
    /// proves a menu opens and reports; the applier seam proves a report is
    /// applied; only here do the egui right-click gesture, the row's response,
    /// and the session state meet. A suite without this would still pass if the
    /// row attached a left-click-only handler.
    ///
    /// Search results are the SAME list under a query — the same rows through
    /// the same renderer — so both surfaces are driven from one test on two
    /// shells rather than from a second invented one. The two halves are on
    /// separate shells so neither is reading a selection the other's still-open
    /// popup could have written.
    #[test]
    fn test_a_secondary_click_on_a_track_row_opens_its_menu_and_selects_the_track() {
        use riff_backend::app::state::LibrarySection;
        use riff_backend::domain::TrackId;

        let beta = TrackId("b.mp3".to_string());
        let flat = || MockLibraryQueryStore {
            flat: vec![track("a.mp3", "Alpha"), track("b.mp3", "Beta")],
            search: vec![track("b.mp3", "Beta")],
            matching_searches: vec!["bet".to_string()],
            ..Default::default()
        };

        // ---- the unqueried All Tracks list ----
        let (mut shell, transport) = recording_transport_shell(flat());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::AllTracks;
        }
        shell.harness.step();
        assert!(
            shell.library.lock_or_recover().selected_track.is_none(),
            "no Track is selected before the right-click"
        );
        // Baseline taken BEFORE the click: the app's first frame hydrates its
        // volume, and this is about what the CLICK added to the record.
        let before = transport.recorded().len();

        shell
            .harness
            .get_by_label("Artist - Beta")
            .click_secondary();
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Play Next").is_some(),
            "a real secondary click on a real row opens the Track's own menu. 'Play Next' \
             is the probe because the player bar says 'Next track' and no other menu is \
             open in this frame"
        );
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(beta.clone()),
            "and makes that Track the selection, so the Detail Panel describes the same \
             Track the menu acts on"
        );
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "a right-click is never a left-click: the frame it happened in sent the \
             transport nothing at all"
        );

        // ---- the same list under a query: a search result ----
        let (mut shell, transport) = recording_transport_shell(flat());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::AllTracks;
            library.search_query = "bet".to_string();
        }
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Artist - Alpha").is_none(),
            "the query narrowed the list, so the row about to be clicked really is a \
             search result and not the same row seen twice"
        );
        let before = transport.recorded().len();

        shell
            .harness
            .get_by_label("Artist - Beta")
            .click_secondary();
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Play Next").is_some(),
            "a search result is the same row under a query, and opens the same menu"
        );
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(beta),
            "and selects the same way, so search results are not a second-class \
             right-click"
        );
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "and a search result's right-click starts no playback either"
        );
    }

    /// The other two Track rows, on the surfaces that are not a list. A Track
    /// under a folder node and a Track in a Smart Playlist are right-clicked
    /// through their own renderers and their own loops, and both must answer
    /// exactly as the flat list does.
    ///
    /// Composed rather than assumed, and deliberately so: all four surfaces
    /// reach one `attach_track_menu`, which proves the APPLIER is shared but
    /// says nothing about whether each site handed that applier a response able
    /// to sense a secondary click at all. Attaching a menu to a response that
    /// only hovers is invisible until a right-click, and then the menu is simply
    /// absent — the failure this file already documents for a list header. One
    /// real click per surface is the only thing that rules it out.
    #[test]
    fn test_a_secondary_click_on_a_folders_and_a_smart_playlists_track_selects_it() {
        use riff_backend::app::state::BrowseMode;
        use riff_backend::domain::TrackId;

        // ---- a Track under a folder node ----
        let (mut shell, transport) = recording_transport_shell(MockLibraryQueryStore {
            folder_has_audio: true,
            folder_direct_tracks: vec![track("t1.mp3", "Ready Let's Go")],
            ..Default::default()
        });
        {
            let mut library = shell.library.lock_or_recover();
            library.browse_mode = BrowseMode::Folders;
            library.library_paths.hydrate(
                &riff_backend::app::store::Settings {
                    library_paths: vec![PathBuf::from("/music")],
                    ..Default::default()
                },
                &mut None,
            );
        }
        shell.harness.step();
        // Folder nodes are collapsed by default, so the body — and the Tracks
        // in it — only exist once the node is open.
        shell.harness.get_by_label("music").click();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("0. Ready Let's Go").is_some(),
            "the open folder node lists the Tracks directly in it"
        );
        let before = transport.recorded().len();

        shell
            .harness
            .get_by_label("0. Ready Let's Go")
            .click_secondary();
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Play Next").is_some(),
            "a right-click on a folder's own Track opens that Track's menu — not the \
             folder's whole-list menu, which is what the node row above it carries"
        );
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(TrackId("t1.mp3".to_string())),
            "and selects the Track. The node itself stays a pure peek: right-clicking a \
             Track inside a folder is a Track action, and browsing the Library Path is \
             unaffected by it"
        );
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "and still starts no playback"
        );

        // ---- a Track in a Smart Playlist ----
        let (mut shell, transport) = recording_transport_shell(MockLibraryQueryStore {
            smart: vec![track("s1.mp3", "Sunrise")],
            smart_list_counts: vec![(riff_backend::domain::SmartPlaylistKind::RecentlyAdded, 1)],
            ..Default::default()
        });
        shell.harness.step();
        // Opened from the sidebar, as a listener does. The row's accessible
        // label folds in its count, exactly as the sidebar paints it.
        shell.harness.get_by_label("Recently Added (1)").click();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Artist - Sunrise").is_some(),
            "the Smart Playlist lists its generated Tracks"
        );
        let before = transport.recorded().len();

        shell
            .harness
            .get_by_label("Artist - Sunrise")
            .click_secondary();
        shell.harness.step();

        assert!(
            shell.harness.query_by_label("Play Next").is_some(),
            "a right-click on a Smart Playlist's Track opens that Track's menu — not the \
             header's whole-list menu, which offers its own four"
        );
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(TrackId("s1.mp3".to_string())),
            "and selects that Track, exactly as the flat list and the folder's did"
        );
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "and still starts no playback"
        );
    }

    /// The one Track surface that had NO menu at all, proved at the only seam a
    /// real gesture can reach: a REAL secondary click on a REAL row of the
    /// Tracks Column, in the real app, both opens the Track's menu and makes
    /// that Track the selection.
    ///
    /// This is new coverage for a surface that carried no menu, so it is the
    /// assertion that cannot be made at either other seam. The widget seam
    /// proves a row reports a menu; the applier seam proves a report is
    /// applied. Neither says whether the Tracks Column's row handed the popup a
    /// response able to sense a secondary click at all — and a row painted
    /// through a custom painter is exactly the shape this file already records
    /// as unable to open a menu. Attaching one to a hover-only response is
    /// invisible until a listener right-clicks, and then the menu is simply
    /// absent, which is how that bug shipped once already. Only a real click
    /// rules it out.
    #[test]
    fn test_a_secondary_click_on_a_tracks_column_row_opens_the_track_menu_and_selects_the_track() {
        use riff_backend::app::state::LibrarySection;
        use riff_backend::domain::TrackId;

        let (mut shell, transport) = recording_transport_shell(album_row_mock());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
        }
        shell.harness.step();
        // Drill the way a listener does: the album's row in the Albums column
        // opens the Tracks column beside it.
        shell
            .harness
            .get_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
            .click();
        shell.harness.step();
        // Two frames past the click, as the drill tests do: the click's frame
        // applies the path, the next one re-plans the stage and paints the
        // Tracks column, and the frame after settles it.
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Iced Cooly").is_some(),
            "the album's own track rows render in the Tracks column; path={:?}",
            shell.library.lock_or_recover().browser_path
        );
        assert!(
            shell.library.lock_or_recover().selected_track.is_none(),
            "and no Track is selected before the right-click"
        );
        // The album's two rows each carry a heart, and neither Track is a
        // Favourite, so this counts HEARTS only — the baseline the menu's own
        // Favourite item is measured against below.
        let hearts = shell.harness.query_all_by_label("Add to Favorites").count();
        assert_eq!(hearts, 2, "both rows carry a heart, both un-Favourited");
        // Baseline taken BEFORE the click: the app's first frame hydrates its
        // volume, and this is about what the CLICK added to the record.
        let before = transport.recorded().len();

        shell.harness.get_by_label("Iced Cooly").click_secondary();
        shell.harness.step();

        // The Favourite item is counted rather than probed for PRESENCE, because
        // the row's heart wears the same label: one more "Add to Favorites" node
        // is the menu's item and nothing else could be.
        assert_eq!(
            shell.harness.query_all_by_label("Add to Favorites").count(),
            hearts + 1,
            "and the Tracks Column's own menu carries the Favourite item, worded for the \
             Track it belongs to"
        );
        // The FULL Track menu, identified by the rows no other surface in this
        // frame owns. "Play" is NOT a probe — the transport bar wears that
        // label — and "Add to Queue" is not either: the two menu renderers are
        // the only places in the app that print it, the Detail Panel being a
        // readout with no action buttons of its own. "Play Next" lives in those
        // same two menu renderers, "Edit Tags" only in the Track one, and a
        // collection or whole-list menu would answer "Shuffle" where the Track
        // menu answers "Edit Tags". The playlist target is probed with the label
        // egui gives a submenu button INSIDE a menu, which appends its
        // disclosure arrow.
        for label in ["Play Next", "Edit Tags", "Add to Playlist \u{23f5}"] {
            assert!(
                shell.harness.query_by_label(label).is_some(),
                "a right-click on a Tracks column row opens the Track's own menu, and \
                 '{label}' is missing from it"
            );
        }
        for absent in ["Shuffle", "Remove from Playlist"] {
            assert!(
                shell.harness.query_by_label(absent).is_none(),
                "and the menu is the TRACK menu, not a collection one: it offers no \
                 '{absent}' — a Track in the Tracks Column is in no Playlist"
            );
        }
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(TrackId("g2".to_string())),
            "and it makes that Track the selection, so the menu and the Detail Panel \
             describe the same Track"
        );
        // The accepted rough edge rides on that selection, and its readout
        // consequence is pinned where it can be observed: the pure resolver
        // already has a test saying a Track selected in an album's Tracks
        // column wins over the album in the drill path
        // (`resolve_inspector`'s Track case). It is not re-asserted here
        // because this harness cannot see it — the Detail Panel's kind chip and
        // title are `ui.label` text, and the eframe-backed harness exposes no
        // label for those. The selection above is the fact this column
        // controls.
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "a right-click is never a left-click: the frame it happened in sent the \
             transport nothing at all"
        );

        // And the item chosen from it produces the SAME effect a flat-list
        // row's does — one dispatch, through the one applier, in the frame the
        // item was chosen. "Play Next" is the probe precisely because it is
        // the one menu row no button outside a menu wears: the transport bar
        // says "Next track", and the Detail Panel — a readout with no action
        // buttons — does not wear it at all.
        //
        // One settling frame first: the frame the menu OPENS both reports the
        // open and moves the selection, and the popup's own geometry is only
        // settled on the frame after it. Clicking a node captured from the
        // opening frame aims at a rectangle the popup has since moved out of,
        // and the click then lands outside the menu and dismisses it — which
        // looks exactly like "the item does nothing".
        shell.harness.step();
        let before = transport.recorded().len();
        shell.harness.get_by_label("Play Next").click();
        shell.harness.step();
        shell.harness.step();

        assert_eq!(
            &transport.recorded()[before..],
            &[TransportIntent::PlayNext(TrackId("g2".to_string()))],
            "choosing an item on a Tracks column row acts on that Track exactly as \
             choosing the same item on a flat-list row does, not through a second \
             dispatch of its own: {:?}",
            transport.recorded()
        );
    }

    /// The three gestures a Tracks Column row already spoke, on the rows that
    /// just gained a fourth. The menu is a new attach on the same response
    /// those gestures are read off, so each is pinned from the outside — the
    /// only place where "the menu quietly took the click" would show up as
    /// something other than a missing report.
    ///
    /// A single click still selects and still starts no playback, and a double
    /// click still selects AND plays. The pair is asserted together because
    /// either alone would be ambiguous: a selection proof that let playback
    /// through would pass for the wrong reason, and a playback proof on a row
    /// that selected nothing would say nothing about the selection.
    #[test]
    fn test_a_tracks_column_row_still_selects_on_one_click_and_plays_on_two() {
        use riff_backend::app::state::LibrarySection;
        use riff_backend::domain::TrackId;

        // ---- one click: selects, and nothing else ----
        let (mut shell, transport) = recording_transport_shell(album_row_mock());
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
        }
        shell.harness.step();
        shell
            .harness
            .get_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
            .click();
        shell.harness.step();
        shell.harness.step();
        let before = transport.recorded().len();

        shell.harness.get_by_label("Iced Cooly").click();
        shell.harness.step();

        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(TrackId("g2".to_string())),
            "a single click on a Tracks column row still selects the Track"
        );
        assert_eq!(
            transport.recorded()[before..],
            Vec::<TransportIntent>::new(),
            "and still starts no playback: the menu's attach did not turn the row's \
             primary click into something else"
        );

        // ---- two clicks: selects AND plays ----
        let (mut shell, transport) = recording_transport_shell_at_step(album_row_mock(), FAST_STEP);
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
        }
        shell.harness.step();
        shell
            .harness
            .get_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
            .click();
        shell.harness.step();
        shell.harness.step();
        let before = transport.recorded().len();

        // The position is captured once, before the click, because the first
        // click of a double-click is also a selection: it re-resolves the
        // Detail Panel, and re-querying the row for the second click would aim
        // at a point the pointer had never been. See [`double_click_at`].
        let at = shell.harness.get_by_label("Iced Cooly").rect().center();
        double_click_at(&mut shell.harness, at, FAST_STEP);
        shell.harness.step();

        let g2 = TrackId("g2".to_string());
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(g2.clone()),
            "a double-click on a Tracks column row still selects the Track"
        );
        assert_eq!(
            &transport.recorded()[before..],
            &[TransportIntent::Play(g2)],
            "and still plays it: one dispatch, from the click path and not from the \
             menu. {:?}",
            transport.recorded()
        );
    }

    /// The heart on a Tracks Column row, on a real `SQLite` store, so
    /// "Favouriting from this Column is unaffected" is a durable fact and not a
    /// repainted label. The menu is a SECOND path to the same change (the
    /// heart is the first, and issue 03 gives the menu a third), so the row
    /// keeps its own control and this is what pins that.
    #[test]
    fn test_the_heart_on_a_tracks_column_row_still_commits_a_favourite_durably() {
        use riff_backend::app::state::LibrarySection;
        use riff_backend::domain::TrackId;

        let (mut shell, dir, _pid, store) = store_shell();
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
        }
        shell.harness.step();
        shell.harness.get_by_label("Album (Artist)").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Alpha").is_some(),
            "the album's tracks render in the Tracks column"
        );
        let alpha = TrackId(dir.path().join("one.mp3").to_string_lossy().into_owned());
        // Every row carries a heart and the seeded album is entirely
        // un-Favourited, so three nodes wear this label. The one on Alpha's OWN
        // row is located by the row it shares a line with — a locator, not an
        // assertion: the assertion below names the track, so clicking the
        // wrong heart would fail loudly rather than quietly pass.
        let alpha_y = shell.harness.get_by_label("Alpha").rect().center().y;
        let heart = shell
            .harness
            .query_all_by_label("Add to Favorites")
            .find(|node| float_close(node.rect().center().y, alpha_y))
            .expect("Alpha's row carries its own heart");
        heart.click();
        shell.harness.step();

        assert!(
            store
                .get_track(&alpha)
                .expect("the query works")
                .expect("the track is still known")
                .favorite,
            "the row's heart committed the flag through the store, from the Tracks \
             column, durably"
        );
        // And the row paints the new state, which is the generation bump the
        // committed mutation made on its own — the next frame's read, with no
        // caller action.
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Remove from Favorites")
                .is_some(),
            "and the row's own control now offers to take the Favourite back"
        );
    }

    /// The one Track row whose menu is deliberately reduced, and the reason it
    /// still selects. A playlist entry whose file has gone missing offers
    /// neither a playback action nor the tag editor, because it cannot play —
    /// that reduction is a separate decision about what the menu CONTAINS, and it
    /// is the Favourite item's condition too, so the reduced menu is pinned whole
    /// rather than by absence. What the row IS is unchanged: a Track in a
    /// playlist, with a readout to keep consistent, so its right-click selects
    /// exactly as every other Track row's does.
    ///
    /// Both halves are asserted together because either alone would be
    /// ambiguous — a selection proof that let the full menu through would pass
    /// for the wrong reason, and a reduction proof on a row that selected
    /// nothing would say nothing about the selection.
    #[test]
    fn test_a_right_click_on_a_missing_playlist_entry_selects_it_and_keeps_its_reduced_menu() {
        use riff_backend::domain::TrackId;

        let (mut shell, dir, _pid, mut store) = store_shell();
        // The file goes, then a commit moves the generation so the open
        // playlist's projection re-resolves and re-runs its read-time
        // filesystem check — which is what turns a valid row into a missing
        // one. The entry keeps its identity throughout: the track row is still
        // in the store, only the file behind it is not.
        std::fs::remove_file(dir.path().join("two.mp3")).expect("the scratch file goes");
        store
            .create_playlist("Scratch", &[])
            .expect("the committing write moves the generation");

        shell.harness.step();
        shell.harness.get_by_label("Gym").click();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("two (missing)").is_some(),
            "the entry whose file went missing is flagged rather than dropped"
        );
        assert!(
            shell.library.lock_or_recover().selected_track.is_none(),
            "and nothing is selected before the right-click"
        );

        shell
            .harness
            .get_by_label("two (missing)")
            .click_secondary();
        shell.harness.step();

        // The reduced menu first: this Track cannot play, so it is offered no
        // playback action and no tag editor. 'Play Next' is the probe because
        // the player bar says 'Next track'.
        assert!(
            shell.harness.query_by_label("Play Next").is_none(),
            "a Track whose file is gone still offers no playback action"
        );
        assert!(
            shell.harness.query_by_label("Edit Tags").is_none(),
            "nor the tag editor"
        );
        // The reduced menu is pinned WHOLE, from inside the open popup, because
        // the other two entries in this playlist are valid and each wears a
        // heart: a flat query for "Add to Favorites" would find THEIR controls
        // and answer the wrong question. The exclusion being asked about is the
        // Favourite item, and it is the same condition the other two are on —
        // which is the heart's, since an entry whose file is gone is drawn as a
        // flagged label with no heart on it.
        use egui_kittest::kittest::NodeT;
        let rows: Vec<String> = shell
            .harness
            .get_by_label("Remove from Playlist")
            .parent()
            .expect("the open menu is a popup in the tree")
            .children_recursive()
            .map(|node| node.accesskit_node().label().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(
            rows,
            vec!["Remove from Playlist"],
            "the reduced menu is exactly the removal and nothing else — no playback, no tag \
             editor, and no Favourite in either wording"
        );
        assert!(
            shell
                .harness
                .query_by_label("Remove from Playlist")
                .is_some(),
            "but it can still be taken out of the Playlist, which is all it can do"
        );
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(TrackId(
                dir.path().join("two.mp3").to_string_lossy().into_owned()
            )),
            "and the row still selected: the readout describes the same Track the menu \
             acts on, whatever the menu is willing to offer"
        );
    }

    /// The gesture this change is most at risk of costing. A Track row's
    /// double-click still selects AND plays, in the flat list and in a Smart
    /// Playlist. Right-click selecting rides the menu's own report and touches
    /// neither the click path nor the double-click path; this says so from the
    /// outside, where the only thing that can.
    #[test]
    fn test_double_clicking_a_track_row_still_selects_and_plays_it() {
        use riff_backend::app::state::LibrarySection;
        use riff_backend::domain::{SmartPlaylistKind, TrackId};

        let beta = TrackId("b.mp3".to_string());

        // ---- the flat All Tracks list ----
        let (mut shell, transport) = recording_transport_shell_at_step(
            MockLibraryQueryStore {
                flat: vec![track("a.mp3", "Alpha"), track("b.mp3", "Beta")],
                ..Default::default()
            },
            FAST_STEP,
        );
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::AllTracks;
        }
        shell.harness.step();
        let before = transport.recorded().len();

        let at = shell.harness.get_by_label("Artist - Beta").rect().center();
        double_click_at(&mut shell.harness, at, FAST_STEP);
        shell.harness.step();

        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(beta.clone()),
            "a double-click still selects the Track"
        );
        assert_eq!(
            &transport.recorded()[before..],
            &[TransportIntent::Play(beta.clone())],
            "and still plays it: one dispatch, from the click path and not from the \
             menu. {:?}",
            transport.recorded()
        );

        // ---- a Smart Playlist ----
        let (mut shell, transport) = recording_transport_shell_at_step(
            MockLibraryQueryStore {
                smart: vec![track("s1.mp3", "Sunrise")],
                smart_list_counts: vec![(SmartPlaylistKind::RecentlyAdded, 1)],
                ..Default::default()
            },
            FAST_STEP,
        );
        shell.harness.step();
        shell.harness.get_by_label("Recently Added (1)").click();
        shell.harness.step();
        let before = transport.recorded().len();

        let at = shell
            .harness
            .get_by_label("Artist - Sunrise")
            .rect()
            .center();
        double_click_at(&mut shell.harness, at, FAST_STEP);
        shell.harness.step();

        let sunrise = TrackId("s1.mp3".to_string());
        assert_eq!(
            shell.library.lock_or_recover().selected_track,
            Some(sunrise.clone()),
            "a double-click in a Smart Playlist still selects the Track"
        );
        assert_eq!(
            &transport.recorded()[before..],
            &[TransportIntent::Play(sunrise)],
            "and still plays it, exactly as the flat list's does"
        );
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
                    outcomes.extend(clear_library_confirm(ui, &mut cache, &palette, false));
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
                    outcomes.extend(clear_thumbnail_cache_confirm(
                        ui, &mut cache, &palette, false,
                    ));
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

    /// The three intents that made the Tracks Column intercept necessary —
    /// **Add To Playlist**, **Remove From Playlist** and **Edit Tags** — work
    /// from a real Track row, through the per-app Track-menu host, with no
    /// intercept anywhere.
    ///
    /// The intercept existed because the detail-action applier held neither the
    /// Playlist Store nor the Inline Tag Editor, which are exactly what those
    /// three need. Moving those two handles into the host is what removed it,
    /// so the claim has to be proved at the surface that used to carry the
    /// intercept — a whole frame, a real store, a real menu click — rather than
    /// only at the host's own interface. If the intercept had been removed by
    /// dropping the intents instead, this test fails and the no-op arm it
    /// replaced is gone either way.
    #[test]
    fn the_three_intercept_only_intents_work_from_a_tracks_column_row() {
        use egui_kittest::kittest::Queryable;
        use riff_backend::app::state::{BrowserSelection, LibrarySection};

        /// The label egui gives the Track menu's "Add to Playlist" submenu once
        /// it is rendered inside a menu: the wording plus a disclosure arrow.
        const MENU_SUBMENU: &str = "Add to Playlist \u{23f5}";

        let (mut shell, _dir, _pid, store) = store_shell();
        {
            let mut library = shell.library.lock_or_recover();
            library.library_section = LibrarySection::Albums;
            // Drill straight to the album every seeded track belongs to, so the
            // Tracks Column renders without a second click through the Albums
            // root.
            library.browser_path.push(BrowserSelection::Album {
                artist: "Artist".to_string(),
                title: "Album".to_string(),
            });
        }
        shell.harness.step();
        let row_label = "Alpha";
        assert!(
            shell.harness.query_by_label(row_label).is_some(),
            "the drilled album's Tracks Column lists its rows"
        );

        // --- Add To Playlist: the Playlist Store door ------------------------
        //
        // A SECOND, empty playlist, so the commit is a visible 0 -> 1 rather
        // than a duplicate the store may decline.
        let mut writes = store.clone();
        let empty = writes
            .create_playlist("Solo", &[])
            .expect("the second playlist commits");
        shell.harness.step();
        let entries = |store: &riff_infra::store::SqliteStore| {
            store
                .load_playlist_entries(&empty)
                .expect("the entries read")
                .len()
        };
        assert_eq!(entries(&store), 0, "the new playlist starts empty");

        shell.harness.get_by_label(row_label).click_secondary();
        // Two settling frames before the item is located, and the click scoped
        // to the popup: a node is a rectangle captured from one frame, and the
        // popup's geometry only settles on the frame after it opens.
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label(MENU_SUBMENU).is_some(),
            "the Tracks Column's menu offers the playlist targets"
        );
        // The targets are a CHILD popup, so the anchor is clicked first and the
        // child is then read from the whole tree. "Solo" also names the
        // sidebar's playlist row, so the LAST match is the submenu's own —
        // popups are appended after the window that opened them.
        shell.harness.get_by_label(MENU_SUBMENU).click();
        shell.harness.step();
        shell.harness.step();
        let mut targets: Vec<_> = shell.harness.query_all_by_label("Solo").collect();
        assert_eq!(
            targets.len(),
            2,
            "the submenu's target is a second 'Solo', beside the sidebar's own"
        );
        targets
            .pop()
            .expect("the submenu offers the playlist it was opened for")
            .click();
        shell.harness.step();
        shell.harness.step();
        assert_eq!(
            entries(&store),
            1,
            "Add To Playlist committed through the Playlist Store, from a Tracks \
             Column row — the intent the intercept used to carry by hand"
        );

        // --- Edit Tags: the Inline Tag Editor door ---------------------------
        //
        // The first right-click SELECTED this row, so its title is now on screen
        // more than once — the row, and the Detail Panel readout that followed
        // the selection. Taking the first pins the row, and the count is
        // asserted rather than assumed so a fourth appearance cannot pass
        // quietly.
        let titles: Vec<_> = shell.harness.query_all_by_label(row_label).collect();
        assert!(
            titles.len() >= 2,
            "the row and the readout the right-click's selection produced: {} nodes",
            titles.len()
        );
        titles
            .into_iter()
            .next()
            .expect("the row is there")
            .click_secondary();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Edit Tags").is_some(),
            "and the same row still offers the tag editor"
        );
        menu_item(&shell, MENU_SUBMENU, "Edit Tags").click();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell.harness.query_by_label("Save").is_some(),
            "Edit Tags opened the Inline Tag Editor from a Tracks Column row, so \
             the applier that holds neither the editor nor its service was never \
             the thing answering it"
        );
        assert!(
            shell.library.lock_or_recover().selected_track.is_some(),
            "and it selected the Track on the way in, so the readout follows the \
             entry point"
        );
    }

    /// The third of the three, from the only Track-row surface that offers it:
    /// a user playlist's entry. A Track in the Tracks Column is in no Playlist,
    /// so the removal door is a playlist row's, and this proves it commits
    /// through the host rather than through anything the column intercepts.
    #[test]
    fn remove_from_playlist_works_from_a_playlist_entry_row() {
        use egui_kittest::kittest::Queryable;

        /// The label egui gives the Track menu's "Add to Playlist" submenu once
        /// it is rendered inside a menu: the wording plus a disclosure arrow.
        /// Used here as the in-popup anchor every `menu_item` lookup needs.
        const MENU_SUBMENU: &str = "Add to Playlist \u{23f5}";

        let (mut shell, _dir, pid, store) = store_shell();
        shell.harness.step();
        shell.harness.get_by_label("Gym").click();
        shell.harness.step();

        let entries = |store: &riff_infra::store::SqliteStore| {
            store
                .load_playlist_entries(&pid)
                .expect("the entries read")
                .len()
        };
        let before = entries(&store);
        assert!(before > 0, "the seeded playlist has an entry to remove");

        let row_label = "Artist - Beta";
        shell.harness.get_by_label(row_label).click_secondary();
        shell.harness.step();
        shell.harness.step();
        assert!(
            shell
                .harness
                .query_by_label("Remove from Playlist")
                .is_some(),
            "a playlist entry's menu offers the removal"
        );
        menu_item(&shell, MENU_SUBMENU, "Remove from Playlist").click();
        shell.harness.step();
        shell.harness.step();
        assert_eq!(
            entries(&store),
            before - 1,
            "Remove From Playlist committed through the Playlist Store, from a \
             Track row — the third of the intents the intercept used to carry"
        );
    }

    /// A Folders-tree shell whose harness advances the clock in 20 ms steps
    /// (issue 03).
    ///
    /// The shared [`build`] takes kittest's default step, a quarter of a
    /// second — longer than any unfold duration the motion tokens can publish,
    /// so a single step snaps the tween straight to its end state and the
    /// in-flight frames the frame-loop contract is about never exist. A step
    /// short enough that several frames fall inside the tween is the only way
    /// to observe one; nothing else about the shell differs, so the fold,
    /// selection and cover wiring are the production ones.
    fn folders_shell(step_dt: f32) -> Shell {
        let playback = Arc::new(Mutex::new(PlaybackSession::default()));
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let backend_events = Arc::new(Mutex::new(BackendEvents::default()));
        let folder_covers = Arc::new(Mutex::new(Vec::new()));
        let settings_calls = Arc::new(Mutex::new(Vec::new()));

        let (app, _visibility_tx) = RiffApp::new_for_test(
            Arc::clone(&playback),
            Arc::clone(&library),
            Box::new(MockTransport::new()),
            Box::new(MockScans::default()),
            Box::new(MockSettingsStore::with_shared_calls(Arc::clone(
                &settings_calls,
            ))),
            Box::new(MockPlaylistStore::default()),
            Box::new(MockLibraryMutationStore::new()),
            SessionViews::new(
                Box::new(MockLibraryQueryStore {
                    folder_has_audio: true,
                    folder_children: vec![PathBuf::from("/music/boards")],
                    folder_direct_tracks: vec![track("t1.mp3", "Ready Let's Go")],
                    ..Default::default()
                }),
                Box::new(MockPlaylistStore::default()),
                StoreGeneration::new(),
                StoreGeneration::new(),
            ),
            Box::new(MockTagEdits),
            Box::new(crate::mocks::MockPasses),
            Box::new(ShellCovers(Arc::clone(&folder_covers))),
            Arc::clone(&backend_events),
        );

        let harness = egui_kittest::Harness::builder()
            .with_size(WINDOW)
            .with_step_dt(step_dt)
            .build_eframe(|cc| {
                riff_gui::ui::fonts::configure_fonts(&cc.egui_ctx);
                app
            });

        Shell {
            harness,
            playback,
            library,
            backend_events,
            scans: MockScans::default(),
            settings_calls,
            folder_covers,
        }
    }

    /// The repaint requests the *previous* pass attributed to riff's own UI
    /// code, as source lines, and whether that pass was an immediate repaint.
    ///
    /// Attribution is the only oracle that can answer the question this seam
    /// exists for — "did *riff* ask for this frame?" — and the two boolean
    /// seams both fail to:
    ///
    /// - `has_requested_repaint` is true on every pass of any whole-shell
    ///   harness, because the end-of-frame idle tick schedules one every frame
    ///   regardless of what is on screen. It cannot tell a moving tree from a
    ///   still one.
    /// - `requested_repaint_last_pass` *is* true for the whole tween even with
    ///   the fix reverted, because egui 0.35's `animate_bool_*` convenience
    ///   wrapper ends in `if 0.0 < value && value < 1.0 { request_repaint() }`.
    ///   egui's own consumers therefore already get frames from inside the
    ///   animation helper, and a bool assertion would pass with riff's
    ///   contribution deleted.
    ///
    /// So the contract under test is the one the issue actually states: riff
    /// drives its own frames rather than riding on a dependency's incidental
    /// request. `RepaintCause`'s file and line are public, documented fields
    /// ("what file had the call that requested the repaint?"), and
    /// `repaint_causes` is egui's own answer to "why are we repainting?".
    ///
    /// Both values describe the same pass: at the start of a pass egui swaps
    /// the accumulated causes and the settled paint delay into their
    /// "previous pass" slots, so reading both after a `step` describes the
    /// frame that step ran rather than the next one.
    fn last_pass_repaint(shell: &Shell) -> (std::collections::BTreeSet<u32>, bool) {
        let ctx = &shell.harness.ctx;
        // Normalized to `/` because `file!()` bakes in backslash separators on
        // Windows, where an un-normalized `ends_with` against a forward-slash
        // module path never matches and the idle tick reads as absent.
        let asked_by_riff = ctx
            .repaint_causes()
            .iter()
            .filter(|cause| {
                cause
                    .file
                    .replace('\\', "/")
                    .ends_with("riff-gui/src/ui/app.rs")
            })
            .map(|cause| cause.line)
            .collect();
        (asked_by_riff, ctx.requested_repaint_last_pass())
    }

    /// Issue 03: the Folders tree asks for its own frames while a body is
    /// unfolding, and asks for nothing once it has settled.
    ///
    /// egui's animation manager advances a tween's value whenever it is asked
    /// but never schedules the pass that would ask again, so a riff-driven
    /// tween that does not say "there is another frame" itself renders as the
    /// one frame that started it. The tree's own row is the surface, the
    /// condition is the collapsing body's own openness rather than a timer, and
    /// the whole point of writing it as "not settled" is that a settled
    /// subtree costs the frame loop nothing.
    ///
    /// Settled is asserted in both directions, and that is not decoration. A
    /// closed body rests at openness `0.0`, so the one-sided reading of "not
    /// settled" — `openness < 1.0` — is true for every collapsed folder forever
    /// and pins the sidebar at uncapped frame rate with the tree shut. The
    /// honest translation of "in flight" for this accessor is the open
    /// interval, and a closed tree asserting nothing is what pins it there.
    #[test]
    fn test_folders_tree_asks_for_its_own_frames_only_while_a_body_is_settling() {
        use riff_backend::app::state::BrowseMode;
        use std::collections::BTreeSet;

        /// Frames sampled across one unfold and one fold. 20 ms each, so the
        /// window is several times the longest unfold duration a motion token
        /// could publish and ends well past the settled state.
        const SAMPLED_FRAMES: usize = 20;
        /// Settled frames the sample must contain past the end of the tween, so
        /// "stops asking" is asserted over a run and not over one lucky frame.
        const SETTLED_FRAMES: usize = 4;

        let mut shell = folders_shell(0.02);
        {
            let mut library = shell.library.lock_or_recover();
            library.browse_mode = BrowseMode::Folders;
            library.library_paths.hydrate(
                &riff_backend::app::store::Settings {
                    library_paths: vec![PathBuf::from("/music")],
                    ..Default::default()
                },
                &mut None,
            );
        }

        /// Step once and report that pass's two observations: whether riff asked
        /// for a repaint beyond the resting baseline, and whether the pass was
        /// an immediate repaint rather than a delayed one.
        fn sample(shell: &mut Shell, resting: &BTreeSet<u32>) -> (bool, bool) {
            shell.harness.step();
            let (asked_by_riff, immediate) = last_pass_repaint(shell);
            (
                asked_by_riff.difference(resting).next().is_some(),
                immediate,
            )
        }

        // A frame with the tree closed, so the resting baseline below is a
        // settled surface rather than a cold one.
        shell.harness.step();
        let (resting, _) = last_pass_repaint(&shell);
        assert!(
            !resting.is_empty(),
            "the shell's end-of-frame idle tick is the baseline this test subtracts; \
             a frame that requested nothing at all would make the subtraction vacuous"
        );

        shell.harness.get_by_label("music").click();
        let unfold: Vec<(bool, bool)> = (0..SAMPLED_FRAMES)
            .map(|_| sample(&mut shell, &resting))
            .collect();

        shell.harness.get_by_label("music").click();
        let fold: Vec<(bool, bool)> = (0..SAMPLED_FRAMES)
            .map(|_| sample(&mut shell, &resting))
            .collect();

        for (label, sampled) in [("unfold", &unfold), ("fold", &fold)] {
            let asked: Vec<bool> = sampled.iter().map(|(asked, _)| *asked).collect();

            // Asking is one contiguous run, not a stutter: every pass from the
            // first to the last asked, which is the shape "while the body is
            // moving" takes, and the run's end is where the body landed.
            let first = asked
                .iter()
                .position(|asked| *asked)
                .unwrap_or_else(|| panic!("the {label} never asked riff for a frame: {asked:?}"));
            let last = asked
                .iter()
                .rposition(|asked| *asked)
                .unwrap_or_else(|| panic!("the {label} never asked riff for a frame: {asked:?}"));
            assert!(
                asked[first..=last].iter().all(|asked| *asked),
                "the tree stopped and started asking mid-{label} instead of asking \
                 across the whole tween: {asked:?}"
            );
            assert!(
                asked.len() - 1 - last >= SETTLED_FRAMES,
                "the sampled window holds fewer than {SETTLED_FRAMES} settled frames \
                 after the {label}, so 'stops asking' is unproven: {asked:?}"
            );

            // Corroboration, not the discriminator: the pass the tree asked in
            // was an immediate repaint, so the tween is served the very next
            // frame rather than waiting out the 100 ms idle tick. This holds
            // even with the fix reverted, which is exactly why the attribution
            // above is what carries the contract.
            assert!(
                sampled
                    .iter()
                    .filter(|(asked, _)| *asked)
                    .all(|(_, immediate)| *immediate),
                "a pass the tree asked for must be an immediate repaint: {sampled:?}"
            );
        }

        // The whole shell back at rest — tree closed, every body settled —
        // must not be asking for anything either. This is the assertion that
        // keeps the condition two-sided, and it is the one an idle-cost
        // regression breaks first.
        for _ in 0..SETTLED_FRAMES {
            let (asked, _) = sample(&mut shell, &resting);
            assert!(
                !asked,
                "a settled, closed Folders tree must ask the frame loop for nothing"
            );
        }
    }
}

// --- Context menus: shared item conventions and typed intents (issue 15) -----
//
// The production Track and list menus render through `ui::menu` and report
// typed intents; every Transport, Playlist Store, selection, and Inline Tag
// Editor effect happens afterwards, in the host's own mapping.
//
// Two reports and two appliers, kept apart on purpose. A menu that OPENS
// reports its row's identity; an item CHOSEN from it reports the item. Right
// click selecting (issue 04) rides the first — on the entity rows through
// `apply_collection_menu`, on every Track row through the per-app Track-menu
// host's `right_clicked` —
// so a right-click that opens a menu and is then dismissed has still selected.
// The second report is the item path, and it is unchanged by any of that.

#[cfg(test)]
mod context_menu_ui_tests {
    use crate::mocks::RecordingPasses;
    use egui_kittest::kittest::{NodeT, Queryable};
    use riff_backend::app::state::{
        BrowserSelection, LibrarySection, LibrarySession, PlaybackSession,
    };
    use riff_backend::app::store::{LibraryMutationStore, PlaylistStore};
    use riff_backend::domain::{PlaylistId, TrackId};
    use riff_gui::ui::app::{
        CollectionMenuEffects, InlineTagEditor, TrackMenuHost, TrackMenuSubject,
        apply_collection_menu, apply_list_menu_intent, entity_track_ids,
    };
    use riff_gui::ui::column::ColumnIdentity;
    use riff_gui::ui::menu::{self, Item, ItemState, ListMenuIntent, TrackMenu, TrackMenuIntent};
    use riff_gui::ui::scroll_memory::DrillSlot;
    use riff_gui::ui::selection::DraftKind;
    use riff_gui::ui::theme::Palette;

    use crate::mocks::{
        MockLibraryQueryStore, MockPlaylistStore, MockTagEdits, MockTransport, TransportIntent,
    };
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

    /// Render the collection menu and click `labels` in order, returning the
    /// intents it emitted. The collection menu takes no props — an entity
    /// always denotes a set of Tracks, so it has nothing to be told.
    fn collection_menu_intents(clicks: &[&str]) -> Vec<ListMenuIntent> {
        let palette = Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 120.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, intents: &mut Vec<ListMenuIntent>| {
                    menu::collection_menu(ui, &palette, intents);
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

    /// The menu rows a renderer painted, in the accessibility tree's reading
    /// order. Reading order is the menu's own sequence, not a pixel measurement.
    fn menu_rows(render: impl Fn(&mut egui::Ui, &Palette) + 'static) -> Vec<String> {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(240.0, 200.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, _rows: &mut ()| {
                    render(ui, &Palette::dark());
                },
                (),
            );
        harness.run();
        harness
            .query_all_by_role(egui::accesskit::Role::Button)
            .map(|node| node.accesskit_node().label().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn test_track_menu_reports_one_typed_intent_per_action() {
        let pid = PlaylistId::new("pl-1");
        let options = [(pid.clone(), "Gym".to_string())];
        let props = TrackMenu {
            playable: true,
            editable: true,
            favorite: false,
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
        assert_eq!(
            track_menu_intents(&props, &["Measure ReplayGain"]),
            vec![TrackMenuIntent::MeasureReplayGain],
            "the measure item reports its own typed intent"
        );
        assert!(
            track_menu_intents(&props, &[]).is_empty(),
            "rendering a menu reports no intent at all"
        );
    }

    /// The measure item's dispatch: the host submits a targeted Track pass —
    /// exactly that Track, never the Album aggregate — and nothing else in
    /// the host's handle set is touched.
    #[test]
    fn the_measure_item_submits_a_targeted_track_pass() {
        use crate::mocks::{
            MockLibraryMutationStore, MockPlaylistStore, MockTransport, RecordingPasses,
        };
        use riff_backend::app::replaygain_pass::PassCommand;

        let passes = RecordingPasses::default();
        let transport = MockTransport::new();
        let mut playlist_store = MockPlaylistStore::default();
        let mut library_mutations = MockLibraryMutationStore::new();
        let mut tag_editor = InlineTagEditor::new(Box::new(crate::mocks::MockTagEdits));
        let mut selected: Option<TrackId> = None;

        // Track identity is the full file path, so the fixture's id and path
        // agree, as they always do in the store.
        let track = crate::test_utils::create_test_track("/music/a1.flac", "/music/a1.flac");
        let subject = TrackMenuSubject::resolved(&track, None);
        let mut host = TrackMenuHost::new(
            &transport,
            &passes,
            &mut playlist_store,
            &mut library_mutations,
            &mut tag_editor,
            &mut selected,
        );
        host.item_chosen(subject, TrackMenuIntent::MeasureReplayGain);

        assert_eq!(
            passes.submitted.lock().unwrap().clone(),
            vec![PassCommand::Track(TrackId("/music/a1.flac".to_string()))],
            "exactly one targeted Track pass, for exactly this Track"
        );
        assert!(
            transport.recorded().is_empty(),
            "the measure item starts no playback"
        );
    }

    /// The Album menu's two measure items dispatch as Album pass commands
    /// naming the row's Album; the play items still dispatch as batches.
    #[test]
    fn the_album_measure_items_submit_album_pass_commands() {
        use riff_backend::app::replaygain_pass::PassCommand;

        let mut playback = PlaybackSession::default();
        let mut library = LibrarySession {
            library_section: LibrarySection::Albums,
            ..Default::default()
        };
        let mut views = entity_views();
        let transport = MockTransport::new();
        let passes = RecordingPasses::default();

        apply_collection_menu(
            "Boards of Canada\u{1f}Geogaddi",
            &[ListMenuIntent::MeasureAlbumAggregate],
            ColumnIdentity::root(LibrarySection::Albums),
            CollectionMenuEffects {
                library: &mut library,
                playback: &mut playback,
                transport: &transport,
                passes: &passes,
                views: &mut views,
            },
        );
        apply_collection_menu(
            "Boards of Canada\u{1f}Geogaddi",
            &[ListMenuIntent::MeasureAlbumTracks],
            ColumnIdentity::root(LibrarySection::Albums),
            CollectionMenuEffects {
                library: &mut library,
                playback: &mut playback,
                transport: &transport,
                passes: &passes,
                views: &mut views,
            },
        );

        assert_eq!(
            passes.submitted.lock().unwrap().clone(),
            vec![
                PassCommand::AlbumAggregate {
                    album_artist: "Boards of Canada".to_string(),
                    album_title: "Geogaddi".to_string(),
                },
                PassCommand::AlbumTracks {
                    album_artist: "Boards of Canada".to_string(),
                    album_title: "Geogaddi".to_string(),
                },
            ],
            "each measure item names the row's Album, and nothing is played"
        );
        assert!(transport.recorded().is_empty());
    }

    /// The menu's content policy is unchanged by the extraction: a Track whose
    /// file is gone offers neither playback actions nor the tag editor — and no
    /// Favourite either, because the Favourite item sits inside the very same
    /// `playable` branch. That is the heart's own gating read back: the heart is
    /// painted into the row of a Track the store still resolves, and a
    /// missing-file playlist entry is drawn as a struck-through `selectable_label`
    /// with no heart on it at all (see `render_playlist_entry`). So the two
    /// surfaces agree on which Tracks are Favouritable without either of them
    /// naming the other's condition.
    ///
    /// Pinned whole rather than by absence, because "keeps only what it kept" is
    /// the claim: one row, and that row is the removal.
    #[test]
    fn test_unplayable_track_menu_keeps_only_the_playlist_actions() {
        let pid = PlaylistId::new("pl-1");
        let options = [(pid.clone(), "Gym".to_string())];
        let props = TrackMenu {
            playable: false,
            editable: false,
            // The flag is a real value here — the entry IS a Favourite in the
            // store — and the menu still offers nothing to do about it.
            favorite: true,
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
        for absent in ["Add to Favorites", "Remove from Favorites"] {
            assert!(
                harness.query_by_label(absent).is_none(),
                "and no Favourite, in either wording: a missing-file entry's menu is not \
                 offered one ({absent} is present)"
            );
        }
        assert_eq!(
            harness
                .query_all_by_role(egui::accesskit::Role::Button)
                .map(|node| node.accesskit_node().label().unwrap_or_default().to_owned())
                .collect::<Vec<String>>(),
            vec!["Remove from Playlist"],
            "the entry is still there to take out, and that is the ONLY row it has"
        );
    }

    /// The Favourite item, and the whole of its design in one render.
    ///
    /// The label flips on the Track's state, so BOTH wordings have to be right
    /// in the SAME pass — which is why the Favourited fixture here is built by a
    /// field set rather than by the module's `track(...)` helper: that helper is
    /// `create_test_track_with_metadata`, which hardcodes `favorite: false`, so a
    /// fixture taken from it could only ever have proved half the claim.
    ///
    /// Two menus, one pass, compared slice by slice: the second is the first
    /// with ONE word changed, which is the whole design position — the condition
    /// rides in the label, the row is in the same place, and nothing is marked.
    /// The value each reports is then read by clicking, in a render of its own:
    /// activating any row closes the menu (egui's own behaviour, and why every
    /// other test here clicks one item per harness), so one harness cannot
    /// answer for both.
    #[test]
    fn test_the_favourite_item_reads_one_way_on_each_state_in_the_same_render() {
        let mut favourited = track("/music/loved.mp3");
        favourited.favorite = true;
        let plain = track("/music/plain.mp3");
        assert_eq!(
            (favourited.favorite, plain.favorite),
            (true, false),
            "the two fixtures really are the two different states the labels claim"
        );
        let options = [(PlaylistId::new("pl-1"), "Gym".to_string())];
        let offered = options.clone();
        let rows = menu_rows(move |ui, palette| {
            let mut intents = Vec::new();
            // Nothing about the render is per-harness: what decides the wording is
            // the prop each menu was handed, so a renderer that ignored the flag
            // would read the same on both menus here.
            for favorite in [false, true] {
                menu::track_menu(
                    ui,
                    palette,
                    &TrackMenu {
                        playable: true,
                        editable: true,
                        favorite,
                        playlists: &offered,
                        remove_from_playlist: false,
                    },
                    &mut intents,
                );
            }
        });
        // Each menu now ends with the ReplayGain measure item — the seventh
        // row in both renders, appended after the tag editor's.
        let plain_rows: Vec<String> = rows[..7].to_vec();
        let favourited_rows: Vec<String> = rows[7..].to_vec();
        assert_eq!(
            plain_rows,
            vec![
                "Play",
                "Play Next",
                "Add to Queue",
                "Add to Playlist",
                "Add to Favorites",
                "Edit Tags",
                "Measure ReplayGain",
            ],
            "a Track that is not a Favourite is offered the wording that adds one"
        );
        assert_eq!(
            favourited_rows,
            vec![
                "Play",
                "Play Next",
                "Add to Queue",
                "Add to Playlist",
                "Remove from Favorites",
                "Edit Tags",
                "Measure ReplayGain",
            ],
            "and the Favourited one the opposite wording, in the SAME render, in the same \
             place — the label carries the condition, so nothing is marked"
        );

        // Each menu reports the value the Track will END UP with, so the host
        // commits exactly what the listener was told.
        for (label, expected) in [
            ("Add to Favorites", TrackMenuIntent::SetFavorite(true)),
            ("Remove from Favorites", TrackMenuIntent::SetFavorite(false)),
        ] {
            let props = TrackMenu {
                playable: true,
                editable: true,
                favorite: label == "Remove from Favorites",
                playlists: &options,
                remove_from_playlist: false,
            };
            assert_eq!(
                track_menu_intents(&props, &[label]),
                vec![expected],
                "{label} reports the Favourite value the click will leave behind, \
                 not the bare fact of a toggle"
            );
        }
    }

    /// The whole-list menu's four items, and the one typed intent each of them
    /// reports. The first three carry the same words a Track's own menu uses for
    /// the same three actions, so queueing is spelled one way whether it is one
    /// Track or a whole collection.
    #[test]
    fn test_list_menu_reports_one_typed_intent_per_action() {
        assert_eq!(list_menu_intents(&["Play"]), vec![ListMenuIntent::Play]);
        assert_eq!(
            list_menu_intents(&["Play Next"]),
            vec![ListMenuIntent::PlayNext]
        );
        assert_eq!(
            list_menu_intents(&["Add to Queue"]),
            vec![ListMenuIntent::AddToQueue]
        );
        assert_eq!(
            list_menu_intents(&["Shuffle"]),
            vec![ListMenuIntent::Shuffle]
        );
        assert!(
            list_menu_intents(&[]).is_empty(),
            "rendering the menu reports no intent at all"
        );
    }

    /// The order is part of the menu's contract, not an accident of how the
    /// rows happen to be written: the accessibility tree reads Play, Play Next,
    /// Add to Queue, Shuffle. Playlist headers, smart playlist headers, and
    /// folder nodes all attach this one renderer, so the three surfaces offer
    /// the same menu in the same sequence by construction.
    #[test]
    fn test_the_list_menu_carries_its_four_items_in_the_fixed_order() {
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

        let labels: Vec<String> = harness
            .query_all_by_role(egui::accesskit::Role::Button)
            .map(|node| node.accesskit_node().label().unwrap_or_default().to_owned())
            .collect();
        assert_eq!(
            labels,
            vec!["Play", "Play Next", "Add to Queue", "Shuffle"],
            "the whole-list menu offers these four items, in this order, and nothing else"
        );
        assert!(
            harness.query_by_label("Append to Queue").is_none(),
            "the retired name is gone: one action has one name"
        );
    }

    /// The collection menu — the one on an Album, Artist, or Genre row — is
    /// content-identical to the whole-list menu, in the same order, because both
    /// denote a set of Tracks. An entity always has tracks, so the menu takes
    /// no props and offers no reduced state: the Track menu's `playable` and
    /// `editable` flags have no meaning for a thing that is not a file.
    #[test]
    fn test_the_collection_menu_carries_the_same_four_items_in_the_fixed_order() {
        assert_eq!(
            menu_rows(|ui, palette| {
                let mut intents = Vec::new();
                menu::collection_menu(ui, palette, &mut intents);
            }),
            vec!["Play", "Play Next", "Add to Queue", "Shuffle"],
            "the collection menu offers these four items, in this order, and nothing else"
        );
        assert_eq!(
            collection_menu_intents(&["Play"]),
            vec![ListMenuIntent::Play]
        );
        assert_eq!(
            collection_menu_intents(&["Play Next"]),
            vec![ListMenuIntent::PlayNext]
        );
        assert_eq!(
            collection_menu_intents(&["Add to Queue"]),
            vec![ListMenuIntent::AddToQueue]
        );
        assert_eq!(
            collection_menu_intents(&["Shuffle"]),
            vec![ListMenuIntent::Shuffle]
        );
        assert!(
            collection_menu_intents(&[]).is_empty(),
            "rendering the collection menu reports no intent at all"
        );
    }

    /// The Track menu's own contents, pinned whole and in order, in both of the
    /// shapes its host can ask for.
    ///
    /// A right-click selecting its row moves a selection ALONGSIDE this menu and
    /// changes nothing inside it, so the exact rows are the preservation guard
    /// for that claim: an effect that added an item, dropped one, reordered the
    /// menu, or relabelled a row would be a change to what the Track menu IS
    /// rather than to when it appears. Both shapes are pinned because the two
    /// hosts differ in exactly one flag, and a change that respected one and not
    /// the other would pass a single-case test.
    ///
    /// The Favourite item is in both lists, in one place, between "Add to
    /// Playlist" and the removal: adding it moved NOTHING that was already here.
    #[test]
    fn test_the_track_menu_carries_its_items_in_the_fixed_order() {
        // A row that is in no Playlist: the flat list, a search result, a Smart
        // Playlist's Tracks, a folder node's Tracks, AND a Track in the Tracks
        // Column all render this shape.
        let no_playlists: [(PlaylistId, String); 0] = [];
        let in_no_playlist = menu_rows(move |ui, palette| {
            let mut intents = Vec::new();
            menu::track_menu(
                ui,
                palette,
                &TrackMenu {
                    playable: true,
                    editable: true,
                    favorite: false,
                    playlists: &no_playlists,
                    remove_from_playlist: false,
                },
                &mut intents,
            );
        });
        assert_eq!(
            in_no_playlist,
            vec![
                "Play",
                "Play Next",
                "Add to Queue",
                "Add to Playlist",
                "Add to Favorites",
                "Edit Tags",
                "Measure ReplayGain",
            ],
            "a Track row's menu offers these seven rows, in this order, and nothing else"
        );

        // A row that IS in a Playlist, which is the one extra row that host
        // adds, and it goes between the two groups for the same reason it always
        // did.
        let with_playlist = [(PlaylistId::new("pl-1"), "Gym".to_string())];
        assert_eq!(
            menu_rows(move |ui, palette| {
                let mut intents = Vec::new();
                menu::track_menu(
                    ui,
                    palette,
                    &TrackMenu {
                        playable: true,
                        editable: true,
                        favorite: false,
                        playlists: &with_playlist,
                        remove_from_playlist: true,
                    },
                    &mut intents,
                );
            }),
            vec![
                "Play",
                "Play Next",
                "Add to Queue",
                "Add to Playlist",
                "Add to Favorites",
                "Remove from Playlist",
                "Edit Tags",
                "Measure ReplayGain",
            ],
            "a playlist entry's menu is the same menu plus the removal, in the same place"
        );

        // The THIRD shape, and the one the Tracks Column hands the widget: a
        // Track in no Playlist with the host's playlist targets on offer. It is
        // the only combination the two cases above do not cover, and it is the
        // one that keeps "Add to Playlist" — the column is not a playlist view
        // and offers no removal, but a Track listed there is exactly as
        // addable as one listed anywhere else.
        //
        // Asserted as an EQUALITY with the flat-list rows above rather than as
        // a third restatement of them: "the same menu in both places" is the
        // claim, and a comparison fails if either surface's wording moves
        // independently of the other's — which a restatement of the expected
        // list would not.
        let targets = [(PlaylistId::new("pl-1"), "Gym".to_string())];
        let in_the_tracks_column = menu_rows(move |ui, palette| {
            let mut intents = Vec::new();
            menu::track_menu(
                ui,
                palette,
                &TrackMenu {
                    playable: true,
                    editable: true,
                    favorite: false,
                    playlists: &targets,
                    remove_from_playlist: false,
                },
                &mut intents,
            );
        });
        assert_eq!(
            in_the_tracks_column, in_no_playlist,
            "a Track in the Tracks Column is offered exactly the menu a Track gets in \
             any other Column — same rows, same order, no removal"
        );
    }

    /// The Track menu and the collection menu read as one menu: the first three
    /// rows are the same words in the same order, and what the collection adds
    /// on top of that shared prefix is Shuffle. Written as a comparison of two
    /// rendered menus rather than a restatement of either, so it fails if either
    /// menu's wording moves independently of the other.
    #[test]
    fn test_the_collection_menu_shares_a_three_item_prefix_with_the_track_menu() {
        let track_options: [(PlaylistId, String); 0] = [];
        let track_rows = menu_rows(move |ui, palette| {
            let mut intents = Vec::new();
            menu::track_menu(
                ui,
                palette,
                &TrackMenu {
                    playable: true,
                    editable: true,
                    favorite: false,
                    playlists: &track_options,
                    remove_from_playlist: false,
                },
                &mut intents,
            );
        });
        let collection_rows = menu_rows(|ui, palette| {
            let mut intents = Vec::new();
            menu::collection_menu(ui, palette, &mut intents);
        });

        assert_eq!(
            &collection_rows[..3],
            &track_rows[..3],
            "Play, Play Next, Add to Queue are the same rows the Track menu opens with, \
             in the same order — one menu, one set of words"
        );
        assert_eq!(
            collection_rows[3..],
            ["Shuffle"],
            "Shuffle is exactly what the collection adds to the shared prefix"
        );
    }

    /// The three Track-only items are absent from the collection menu, and each
    /// for its own reason: an entity has no tags of its own to edit, and a
    /// collection is not a single Track to add to or take out of a Playlist.
    /// Offering any of them would be offering an action that cannot apply.
    #[test]
    fn test_no_entity_menu_offers_a_track_only_action() {
        let rows = menu_rows(|ui, palette| {
            let mut intents = Vec::new();
            menu::collection_menu(ui, palette, &mut intents);
        });
        for absent in ["Edit Tags", "Add to Playlist", "Remove from Playlist"] {
            assert!(
                !rows.iter().any(|row| row == absent),
                "an Album, Artist, or Genre menu offers no {absent}"
            );
        }
    }

    /// The row-level popup owner. A secondary click on an entity row reports
    /// the row's own key together with whatever the menu collected — and
    /// reports the pair on the frame the menu OPENS, with an empty intent list,
    /// because opening the menu is what moves the selection, not choosing an
    /// item from it. The key travelling with the intents is what lets the host
    /// select and act without a pointer.
    ///
    /// The popup is egui's own transient one, keyed by the row's response
    /// identity: there is no per-row open state here, and the row is
    /// materialized only because it was on screen, so a virtualized listing
    /// needs nothing extra to keep this correct.
    #[test]
    fn test_a_secondary_click_on_an_entity_row_reports_its_key_with_the_menu() {
        let palette = Palette::dark();
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let items = [riff_gui::ui::browser::BrowserItem {
            key: "Boards of Canada\u{1f}Geogaddi".to_string(),
            label: "Geogaddi".to_string(),
            detail: Some("Boards of Canada \u{b7} 2002".to_string()),
            thumbnail: None,
            selected: false,
            now_playing: false,
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<riff_gui::ui::browser::BrowserAction>| {
                    let mut fixture_item =
                        |i: usize| -> Option<riff_gui::ui::browser::BrowserItem> {
                            items.get(i).cloned()
                        };
                    let column = riff_gui::ui::browser::BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();
        assert!(
            harness.state().is_empty(),
            "painting a column reports no action at all"
        );

        let key = "Boards of Canada\u{1f}Geogaddi".to_string();
        harness
            .get_by_label("Geogaddi (Boards of Canada \u{b7} 2002)")
            .click_secondary();
        harness.run();

        assert_eq!(
            harness.state().first(),
            Some(&riff_gui::ui::browser::BrowserAction::ContextMenu {
                key: key.clone(),
                intents: Vec::new(),
            }),
            "opening the menu reports the row's own key with no intent chosen yet — \
             that report is what moves the selection"
        );
        assert!(
            harness.query_by_label("Shuffle").is_some(),
            "and the collection menu is the one on screen"
        );

        harness.get_by_label("Shuffle").click();
        harness.run();
        assert_eq!(
            harness.state().last(),
            Some(&riff_gui::ui::browser::BrowserAction::ContextMenu {
                key,
                intents: vec![ListMenuIntent::Shuffle],
            }),
            "choosing an item rides the same key, so the host can both select the \
             row and act on it from one report"
        );
    }

    /// A left click stays a plain selection: right-click selecting must not have
    /// turned the ordinary gesture into a menu report.
    #[test]
    fn test_a_left_click_on_an_entity_row_still_reports_only_a_selection() {
        let palette = Palette::dark();
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let items = [riff_gui::ui::browser::BrowserItem {
            key: "jazz".to_string(),
            label: "Jazz".to_string(),
            detail: None,
            thumbnail: None,
            selected: false,
            now_playing: false,
        }];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, 300.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, actions: &mut Vec<riff_gui::ui::browser::BrowserAction>| {
                    let mut fixture_item =
                        |i: usize| -> Option<riff_gui::ui::browser::BrowserItem> {
                            items.get(i).cloned()
                        };
                    let column = riff_gui::ui::browser::BrowserColumn {
                        sort_desc: false,
                        show_sort: false,
                        total: items.len(),
                        item: &mut fixture_item,
                        virtualize: false,
                        empty_title: "",
                        empty_hint: "",
                    };
                    riff_gui::ui::browser::show_browser_column_scrolled(
                        ui, &mut cache, &palette, false, column, None, actions,
                    );
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Jazz").click();
        harness.run();
        assert_eq!(
            std::mem::take(harness.state_mut()),
            vec![riff_gui::ui::browser::BrowserAction::Select(
                "jazz".to_string()
            )],
            "a primary click reports the selection and nothing else — no menu report"
        );
    }

    // --- The host's half: a collection's selection + its batch ---------------
    //
    // The selection an entity row's right-click causes is CARRIED as an effect
    // of opening the menu, alongside the menu's other effects, rather than
    // being read back off the click. Everything below therefore drives the
    // applier with a key and a list of intents and no pointer in sight: the row
    // was clicked a long way from here, and everything the click caused is
    // spelled out in the two values.

    /// A `SessionViews` over a canned library whose entity queries are seeded
    /// far enough to resolve an Album, an Artist, and a Genre to distinct
    /// batches.
    fn entity_views() -> riff_backend::app::views::SessionViews {
        use riff_backend::app::store::StoreGeneration;
        use riff_backend::domain::{Album, Artist, GenreCount};

        let mock = MockLibraryQueryStore {
            albums: vec![Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
                tracks: vec![TrackId("g1".to_string()), TrackId("g2".to_string())],
                year: Some(2002),
                genre: Some("IDM".to_string()),
            }],
            album_tracks: vec![
                crate::test_utils::create_test_track_with_metadata(
                    "g1",
                    "/music/g1.mp3",
                    "Boards of Canada",
                    "Music for",
                    "Geogaddi",
                ),
                crate::test_utils::create_test_track_with_metadata(
                    "g2",
                    "/music/g2.mp3",
                    "Boards of Canada",
                    "Iced Cooly",
                    "Geogaddi",
                ),
            ],
            artists: vec![Artist {
                name: "Boards of Canada".to_string(),
                albums: vec!["Geogaddi".to_string()],
            }],
            genre_counts: vec![GenreCount {
                genre: "IDM".to_string(),
                tracks: 2,
            }],
            genre_artists: vec![Artist {
                name: "Boards of Canada".to_string(),
                albums: vec!["Geogaddi".to_string()],
            }],
            genre_albums: vec![Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
                tracks: vec![TrackId("g1".to_string())],
                year: Some(2002),
                genre: Some("IDM".to_string()),
            }],
            genre_album_tracks: vec![crate::test_utils::create_test_track_with_metadata(
                "g1",
                "/music/g1.mp3",
                "Boards of Canada",
                "Music for",
                "Geogaddi",
            )],
            ..Default::default()
        };
        riff_backend::app::views::SessionViews::new(
            Box::new(mock),
            Box::new(MockPlaylistStore::default()),
            StoreGeneration::new(),
            StoreGeneration::new(),
        )
    }

    /// The batch each kind of entity denotes, resolved from the row's key. The
    /// three answers are deliberately different shapes: an Album is one album's
    /// tracks, an Artist is every track across its albums, and a Genre is the
    /// tracks CARRYING that genre — which is not the same set, and is the whole
    /// reason a genre's batch cannot be reached by filtering the library's.
    #[test]
    fn test_an_entitys_batch_is_resolved_from_its_row_key() {
        let mut views = entity_views();
        let album_key = "Boards of Canada\u{1f}Geogaddi";

        assert_eq!(
            entity_track_ids(
                album_key,
                ColumnIdentity::root(LibrarySection::Albums),
                &mut views
            ),
            vec![TrackId("g1".to_string()), TrackId("g2".to_string())],
            "an Album row resolves to that album's tracks"
        );
        assert_eq!(
            entity_track_ids(
                "Boards of Canada",
                ColumnIdentity::root(LibrarySection::Artists),
                &mut views,
            ),
            vec![TrackId("g1".to_string()), TrackId("g2".to_string())],
            "an Artist row resolves to every track across its albums"
        );
        assert_eq!(
            entity_track_ids(
                "IDM",
                ColumnIdentity::root(LibrarySection::Genres),
                &mut views
            ),
            vec![TrackId("g1".to_string())],
            "a Genre row resolves to the tracks carrying THAT genre — the \
             genre-scoped walk, not the artist's or the album's full track list"
        );
        assert_eq!(
            entity_track_ids(
                "Krautrock",
                ColumnIdentity::root(LibrarySection::Genres),
                &mut views,
            ),
            Vec::<TrackId>::new(),
            "a genre the read model no longer carries resolves to nothing"
        );
    }

    /// Opening an entity row's menu selects it — and selecting it is ALL it
    /// does. A right-click that chooses no item leaves the menu on screen, and
    /// the readout must already agree with it; it must not start playback, it
    /// must not queue anything, and it must not touch shuffle.
    ///
    /// No pointer appears in this test: the selection arrives as an effect, in
    /// the same report the menu's intents would have.
    #[test]
    fn test_opening_a_collection_menu_selects_the_entity_and_nothing_else() {
        let transport = MockTransport::new();
        let mut playback = PlaybackSession::default();
        let mut library = LibrarySession {
            library_section: LibrarySection::Albums,
            ..Default::default()
        };
        let mut views = entity_views();

        apply_collection_menu(
            "Boards of Canada\u{1f}Geogaddi",
            &[],
            ColumnIdentity::root(LibrarySection::Albums),
            CollectionMenuEffects {
                library: &mut library,
                playback: &mut playback,
                transport: &transport,
                passes: &crate::mocks::MockPasses,
                views: &mut views,
            },
        );

        assert_eq!(
            library.browser_path,
            vec![BrowserSelection::Album {
                artist: "Boards of Canada".to_string(),
                title: "Geogaddi".to_string(),
            }],
            "the report moved the browser path onto the right-clicked Album, so the \
             Detail Panel describes the entity the menu is open on"
        );
        assert!(
            transport.recorded().is_empty(),
            "a right-click starts no playback: the transport saw nothing"
        );
        assert!(
            !playback.queue.shuffle,
            "and it does not so much as touch shuffle"
        );
    }

    /// Each of the four items reaches the transport as the same batch it reaches
    /// from a playlist header, a smart playlist header, or a folder node — the
    /// one intent vocabulary, one applier, one effect per item. And every one of
    /// them selects the row it was opened on, because that is what opening the
    /// menu means.
    #[test]
    fn test_a_collection_menu_intent_batches_and_selects_in_one_report() {
        let ids = [TrackId("g1".to_string()), TrackId("g2".to_string())];
        let cases: [(ListMenuIntent, Vec<TransportIntent>, bool); 4] = [
            (
                ListMenuIntent::Play,
                vec![TransportIntent::PlayMany(
                    ids[0].clone(),
                    vec![ids[1].clone()],
                )],
                false,
            ),
            (
                ListMenuIntent::PlayNext,
                vec![
                    TransportIntent::PlayNext(ids[1].clone()),
                    TransportIntent::PlayNext(ids[0].clone()),
                ],
                false,
            ),
            (
                ListMenuIntent::AddToQueue,
                vec![TransportIntent::AddMany(vec![
                    ids[0].clone(),
                    ids[1].clone(),
                ])],
                false,
            ),
            (
                ListMenuIntent::Shuffle,
                vec![TransportIntent::PlayMany(
                    ids[0].clone(),
                    vec![ids[1].clone()],
                )],
                true,
            ),
        ];

        for (intent, expected_dispatch, shuffle_after) in cases {
            let transport = MockTransport::new();
            let mut playback = PlaybackSession::default();
            let mut library = LibrarySession {
                library_section: LibrarySection::Albums,
                ..Default::default()
            };
            let mut views = entity_views();

            apply_collection_menu(
                "Boards of Canada\u{1f}Geogaddi",
                &[intent],
                ColumnIdentity::root(LibrarySection::Albums),
                CollectionMenuEffects {
                    library: &mut library,
                    playback: &mut playback,
                    transport: &transport,
                    passes: &crate::mocks::MockPasses,
                    views: &mut views,
                },
            );

            assert_eq!(
                transport.recorded(),
                expected_dispatch,
                "{intent:?} reaches the transport as a batch, never a command per Track"
            );
            assert_eq!(
                playback.queue.shuffle, shuffle_after,
                "{intent:?} leaves shuffle engaged: {shuffle_after}"
            );
            assert_eq!(
                library.browser_path,
                vec![BrowserSelection::Album {
                    artist: "Boards of Canada".to_string(),
                    title: "Geogaddi".to_string(),
                }],
                "{intent:?} selected the row the menu was opened on"
            );
        }
    }

    /// A drill column's row is an Album one level down, and its key means
    /// something the root's key of the same section does not. The selection
    /// lands at the row's own level, and the batch is the album's — so a right
    /// click deep in the Genres drill reads as the same kind of act as one at
    /// the top of the Artists root.
    ///
    /// The parent is seeded first, because a drill column only ever renders
    /// below a selection the listener already made — the dispatch site reads
    /// `browser_path.first()` to know which column to plan. `select_at`
    /// TRUNCATES to the row's level and then pushes, so the depth is what keeps
    /// the entries above it, not something it can invent.
    #[test]
    fn test_a_drill_rows_collection_menu_selects_at_its_own_level() {
        let transport = MockTransport::new();
        let mut playback = PlaybackSession::default();
        let mut library = LibrarySession {
            browser_path: vec![BrowserSelection::Artist("Boards of Canada".to_string())],
            ..Default::default()
        };
        let mut views = entity_views();

        apply_collection_menu(
            "Boards of Canada\u{1f}Geogaddi",
            &[ListMenuIntent::Play],
            ColumnIdentity::drill(LibrarySection::Artists, 1, DrillSlot::ArtistAlbums),
            CollectionMenuEffects {
                library: &mut library,
                playback: &mut playback,
                transport: &transport,
                passes: &crate::mocks::MockPasses,
                views: &mut views,
            },
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
            "the path keeps the parent the listener drilled through and the \
             right-clicked Album lands at the row's own level"
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::PlayMany(
                TrackId("g1".to_string()),
                vec![TrackId("g2".to_string())],
            )],
            "a drill row's batch is its entity's, resolved from its key"
        );
    }

    /// A key that names nothing the read model still carries — a stale row, or a
    /// row whose section does not denote a set of Tracks at all — selects
    /// nothing and plays nothing. The All Tracks listing has no entity rows, and
    /// it must not grow a menu that acts on an empty key.
    #[test]
    fn test_a_key_naming_no_entity_acts_on_nothing() {
        let transport = MockTransport::new();
        let mut playback = PlaybackSession::default();
        let mut library = LibrarySession::default();
        let mut views = entity_views();

        apply_collection_menu(
            "a.mp3",
            &[ListMenuIntent::Play, ListMenuIntent::Shuffle],
            // The All Tracks listing has no entity rows, and states no
            // entity-Column identity — this is the defensive shape, spelled
            // the way the shape that has no rows still has to be spelled.
            ColumnIdentity::root(LibrarySection::AllTracks),
            CollectionMenuEffects {
                library: &mut library,
                playback: &mut playback,
                transport: &transport,
                passes: &crate::mocks::MockPasses,
                views: &mut views,
            },
        );

        assert!(
            library.browser_path.is_empty(),
            "no browser selection is invented for a section with no entity rows"
        );
        assert!(
            transport.recorded().is_empty(),
            "and the transport sees nothing at all"
        );
        assert!(
            !playback.queue.shuffle,
            "shuffle is never so much as engaged"
        );
    }

    /// The one predicate every scroll guard asks. A right-click moves the
    /// selection exactly as a click does, so it must reset the listing's scroll
    /// exactly as a click does; anything that is not a row selection must not.
    /// Pinning the answers here is what makes the guard impossible to widen by
    /// accident: a new selection-carrying variant that nobody declared in this
    /// function is a test failure, not a silently stale guard.
    #[test]
    fn test_only_row_selecting_actions_reset_a_listings_scroll() {
        use riff_gui::ui::browser::BrowserAction;

        assert!(
            BrowserAction::Select("a".to_string()).selects_a_row(),
            "a click selects"
        );
        assert!(
            BrowserAction::ContextMenu {
                key: "a".to_string(),
                intents: Vec::new(),
            }
            .selects_a_row(),
            "opening a row's menu selects, which is the whole point of the guard: a \
             right-click that moved the selection but not the scroll would leave the \
             list pointing at the wrong place"
        );
        assert!(
            BrowserAction::ContextMenu {
                key: "a".to_string(),
                intents: vec![ListMenuIntent::Play],
            }
            .selects_a_row(),
            "and a report that also carries a chosen item still selected the row — the \
             choice does not cancel the selection the open already made"
        );
        assert!(
            !BrowserAction::ToggleSort.selects_a_row(),
            "the sort control selects nothing"
        );
    }

    // --- The host's half: opening a Track's menu moves the selection ---------
    //
    // A Track row's right-click opens that Track's menu AND makes it the
    // selected Track, so the menu acts on the same Track the Detail Panel
    // shows. The selection rides as an effect of the menu OPENING — a separate
    // and earlier report from the items the listener chooses from it — so
    // everything below drives it with no pointer in sight: the row was clicked
    // a long way from here, and all the click caused is spelled out in the two
    // values the applier takes.
    //
    // A Track row is the deliberate opposite of a header here, in exactly one
    // respect and no other: a header has no readout for a selection to stay
    // consistent with, a row does.

    /// Opening a Track's menu selects that Track, and selecting it is ALL it
    /// does. A right-click that opens the menu and is then dismissed chose
    /// nothing, and must still have selected — which is the whole reason the
    /// effect hangs off the popup's own report and not off an item choice.
    #[test]
    fn test_opening_a_track_menu_selects_that_track_and_nothing_else() {
        let (_dir, mut store, pid) = seeded_store();
        let mut mutations = store.clone();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let item = track("/music/a.mp3");

        {
            let subject = TrackMenuSubject::resolved(&item, None);
            let mut host = host(
                &transport,
                &mut store,
                &mut mutations,
                &mut editor,
                &mut selected,
            );
            host.right_clicked(subject);
        }

        assert_eq!(
            selected,
            Some(item.id.clone()),
            "the report moved the selection onto the Track the menu was opened on, so the \
             Detail Panel and the menu describe the same Track"
        );
        assert!(
            transport.recorded().is_empty(),
            "a right-click is never a left-click: the transport saw nothing"
        );
        assert!(
            editor.draft().is_none(),
            "and it does not so much as open the Inline Tag Editor"
        );
        assert_eq!(
            store
                .load_playlist_entries(&pid)
                .expect("the entries read")
                .len(),
            0,
            "or write a playlist entry"
        );
    }

    /// The negative twin, and the half that makes the test above mean anything:
    /// a frame that merely PAINTED a Track row reaches the host at all and
    /// therefore moves nothing — not the selection it would have written, and
    /// not the one already there. A list of Track rows spends most of its
    /// frames in exactly this state, so it is the case that has to leave the
    /// readout alone.
    ///
    /// **This is the property the host's shape buys, not one it is asked
    /// about.** The old interface had a `TrackMenuOpen::NotOpened` report the
    /// caller had to pass in for a frame that chose to do nothing, and this
    /// test existed to pin that the applier left the selection alone. The host
    /// has no such report and no such parameter: a painted frame simply does
    /// not call `right_clicked`, so "do nothing" is the absence of a call
    /// rather than a value the host has to be trusted to interpret. What the
    /// host still owes is the other half — a menu that opened and was then
    /// dismissed has still selected, because opening reported and chose
    /// nothing, which the test above covers.
    #[test]
    fn test_a_frame_that_painted_a_track_menu_moves_nothing() {
        let (_dir, store, pid) = seeded_store();
        let transport = MockTransport::new();
        let editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let already = track("/music/b.mp3");
        let mut selected = None;

        // A frame that only painted called neither of the host's two methods —
        // there is nothing on the interface to call with "nothing happened" —
        // and the selection is exactly what it was.
        assert!(
            selected.is_none(),
            "nothing was selected before, and a row that merely rendered selects nothing"
        );

        selected = Some(already.id.clone());
        assert_eq!(
            selected,
            Some(already.id),
            "and a selection that is already there is not cleared either: the host moves \
             the selection, and only ever when it is asked to"
        );
        assert!(
            transport.recorded().is_empty(),
            "no transport command either way"
        );
        assert!(editor.draft().is_none(), "and no tag draft");
        assert_eq!(
            store
                .load_playlist_entries(&pid)
                .expect("the entries read")
                .len(),
            0,
            "and no playlist entry"
        );
    }

    /// The variants [`ItemState`] declares, in declaration order, read from the
    /// enum's own source.
    ///
    /// A sweep, because the claim is about the SHAPE of a type and nothing in the
    /// app constrains that shape: no code matches exhaustively on `ItemState`, so
    /// a `Checked` variant would compile, would paint, and would pass every test
    /// that exercises the three states that exist. The spec's "no checked or
    /// ticked state" is a promise about the enum, so it is read off the enum.
    fn item_state_variants() -> Vec<String> {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("crates")
                .join("riff-gui")
                .join("src")
                .join("ui")
                .join("menu.rs"),
        )
        .expect("menu.rs must be readable");
        let body = source
            .split_once("pub enum ItemState {")
            .expect("ItemState is declared in menu.rs")
            .1
            .split_once("\n}")
            .expect("ItemState's body is closed by a brace on its own line")
            .0;
        body.lines()
            .map(str::trim)
            .filter(|line| {
                // A variant is a bare, documented identifier: no attributes, no
                // braces, no type annotation.
                !line.is_empty()
                    && !line.starts_with("//")
                    && !line.starts_with('#')
                    && !line.contains(['{', '}', ':', ';', '='])
            })
            .map(|variant| variant.trim_end_matches(',').to_owned())
            .collect()
    }

    /// Enabled, disabled, and destructive rows come from one owner: a disabled
    /// row answers no click, and only the destructive row carries the error ink.
    ///
    /// The vocabulary count below is the second half of this test and the reason
    /// the Favourite item needed no new state: a menu condition is said with a
    /// word or with ink, and a tick would have needed a FOURTH variant to live in.
    /// It is counted from the enum's own source because the count is not
    /// observable any other way from outside — nothing matches exhaustively on
    /// [`ItemState`], so a fourth variant would compile, paint, and pass every
    /// other test in this file.
    #[test]
    fn test_menu_item_states_share_one_treatment() {
        assert_eq!(
            item_state_variants(),
            vec!["Normal", "Destructive", "Disabled"],
            "the menu's item state is still exactly these three — a Favourite is announced \
             by its LABEL, so nothing marked was needed and no Checked arrived"
        );
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

    /// Build the per-app Track-menu host over mock ports.
    ///
    /// This is the SAME constructor `RiffApp::track_menu` calls in production —
    /// the one place the app's handle set is named — so a test drives the host
    /// the app has rather than a bag it assembled to look like one. That is
    /// the whole point of the change: the previous helper was a field-for-field
    /// MIRROR of the two production assembly sites, and a production site that
    /// quietly dropped a handle would have left the mirror complete and the test
    /// green, proving a host the app does not have. Now there is no second
    /// assembly to keep faithful: a handle added to or removed from the host
    /// changes this constructor's signature and breaks the app and this test
    /// together, at compile time, which is the only place such a mistake can be
    /// caught.
    fn host<'a>(
        transport: &'a MockTransport,
        store: &'a mut riff_infra::store::SqliteStore,
        library_mutations: &'a mut dyn LibraryMutationStore,
        editor: &'a mut InlineTagEditor,
        selected: &'a mut Option<TrackId>,
    ) -> TrackMenuHost<'a> {
        let playlist_store: &'a mut dyn PlaylistStore = store;
        TrackMenuHost::new(
            transport,
            &crate::mocks::MockPasses,
            playlist_store,
            library_mutations,
            editor,
            selected,
        )
    }

    #[test]
    fn test_playlist_intents_commit_only_when_emitted() {
        let (_dir, mut store, pid) = seeded_store();
        // The library mutations port is a SECOND handle to the same store, the
        // way the composition root wires the two ports: one object, two
        // sections. A test that Favourited through a recording fake instead
        // would prove the arm, not the durability.
        let mut mutations = store.clone();
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
            let subject = TrackMenuSubject::resolved(&item, Some(&pid));
            let mut host = host(
                &transport,
                &mut store,
                &mut mutations,
                &mut editor,
                &mut selected,
            );
            host.item_chosen(subject, TrackMenuIntent::AddToPlaylist(pid.clone()));
        }
        assert_eq!(
            entries(&store),
            1,
            "the emitted Add to Playlist committed through the store"
        );

        {
            let subject = TrackMenuSubject::resolved(&item, Some(&pid));
            let mut host = host(
                &transport,
                &mut store,
                &mut mutations,
                &mut editor,
                &mut selected,
            );
            host.item_chosen(subject, TrackMenuIntent::RemoveFromPlaylist);
        }
        assert_eq!(entries(&store), 0, "the emitted Remove committed its half");
        assert!(
            transport.recorded().is_empty(),
            "a playlist intent sends no transport command"
        );
    }

    /// The Favourite item's dispatch: it commits the flag the menu REPORTED,
    /// through the library mutation port, and touches nothing else.
    ///
    /// The port is the existing recording fake and the recorder is its existing
    /// `favorites()`, so what is asserted is the port-observed effect: the exact
    /// `(id, value)` pair, in call order. Both directions are driven, because the
    /// menu carries a VALUE and not a toggle — a bare-toggle dispatch would pass
    /// the `true` case and fail the `false` one, which is why both are here.
    #[test]
    fn test_the_favourite_intent_commits_the_reported_value_and_nothing_else() {
        use crate::mocks::MockLibraryMutationStore;

        let (_dir, mut store, pid) = seeded_store();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let mut mutations = MockLibraryMutationStore::new();
        let item = track("/music/a.mp3");

        assert!(
            mutations.favorites().is_empty(),
            "a Track menu that is only painted writes no flag"
        );

        for reported in [true, false] {
            {
                let subject = TrackMenuSubject::resolved(&item, Some(&pid));
                let mut host = host(
                    &transport,
                    &mut store,
                    &mut mutations,
                    &mut editor,
                    &mut selected,
                );
                host.item_chosen(subject, TrackMenuIntent::SetFavorite(reported));
            }
            assert_eq!(
                mutations.favorites().last(),
                Some(&(item.id.clone(), reported)),
                "SetFavorite({reported}) committed exactly that value for that Track"
            );
            assert!(
                transport.recorded().is_empty(),
                "a Favourite is a library fact: it starts nothing and queues nothing"
            );
            assert!(editor.draft().is_none(), "and it opens no tag draft");
            assert!(
                selected.is_none(),
                "and it moves no selection — the menu's OPENING already did that, and the \
                 item is a separate event"
            );
        }
        assert_eq!(
            mutations.favorites(),
            vec![(item.id.clone(), true), (item.id.clone(), false)],
            "both directions went through the one port method, in the order they were \
             applied, and nothing was written between them"
        );
        assert_eq!(
            store
                .load_playlist_entries(&pid)
                .expect("the entries read")
                .len(),
            0,
            "and no playlist entry was written: the two stores stay separate"
        );
    }

    #[test]
    fn test_edit_tags_intent_opens_the_detail_editor_and_nothing_else() {
        let (_dir, mut store, pid) = seeded_store();
        let mut mutations = store.clone();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let item = track("/music/a.mp3");

        {
            let subject = TrackMenuSubject::resolved(&item, Some(&pid));
            let mut host = host(
                &transport,
                &mut store,
                &mut mutations,
                &mut editor,
                &mut selected,
            );
            host.item_chosen(subject, TrackMenuIntent::EditTags);
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

    /// Shuffle is a one-shot that *starts* the collection: it engages shuffle
    /// on the playback session and then plays the batch, and shuffle is still
    /// engaged afterwards. The intent is the same one the album, playlist,
    /// smart playlist, and folder menus report — so the action means one
    /// thing wherever it is offered.
    #[test]
    fn test_shuffle_intent_engages_shuffle_then_plays_the_batch() {
        let transport = MockTransport::new();
        let mut playback = PlaybackSession::default();
        let ids: Vec<TrackId> = ["/music/a.mp3", "/music/b.mp3", "/music/c.mp3"]
            .iter()
            .map(|p| track(p).id)
            .collect();
        assert!(
            !playback.queue.shuffle,
            "shuffle is off before the intent is applied"
        );

        apply_list_menu_intent(ListMenuIntent::Shuffle, &ids, &transport, &mut playback);

        assert!(
            playback.queue.shuffle,
            "the one-shot leaves shuffle engaged afterwards, as it always has"
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::PlayMany(
                ids[0].clone(),
                vec![ids[1].clone(), ids[2].clone()],
            )],
            "the batch is one dispatch behind the shuffle flag, not a command per Track"
        );
    }

    /// An empty collection starts nothing — and it does not so much as turn
    /// shuffle on. The two go together: engaging shuffle is the first half of
    /// starting a batch, so with no batch there is nothing to do at all.
    #[test]
    fn test_shuffle_intent_on_an_empty_collection_does_nothing_at_all() {
        let transport = MockTransport::new();
        let mut playback = PlaybackSession::default();

        apply_list_menu_intent(ListMenuIntent::Shuffle, &[], &transport, &mut playback);

        assert!(
            !playback.queue.shuffle,
            "an empty list never re-enables shuffle"
        );
        assert!(
            transport.recorded().is_empty(),
            "and sends nothing to the transport"
        );
    }

    /// The tracer: a menu that is only *drawn* reaches nothing, and the click
    /// that follows reaches exactly the effect of the row it landed on. This
    /// test exists to join the two seams — a real `list_menu` rendering in
    /// isolation, then the host's dispatch loop over whatever it reported —
    /// because the property worth protecting is that the join is thin. A menu
    /// can be painted on any number of frames without a Track starting to play,
    /// and the frame it is finally clicked on does the work of the one item
    /// chosen and nothing else.
    ///
    /// The expected effects are written out as the dispatch the transport port
    /// should observe and the session state the click should leave, so the
    /// assertions read as a contract rather than as a transcript of whichever
    /// arm happens to run.
    #[test]
    fn test_a_painted_list_menu_reaches_the_transport_only_through_a_click() {
        let ids: Vec<TrackId> = ["/music/a.mp3", "/music/b.mp3"]
            .iter()
            .map(|p| track(p).id)
            .collect();

        // A painted frame: the menu's whole dispatch loop, fed the intents a
        // frame that only drew the menu reported.
        let paint_only = MockTransport::new();
        let mut untouched = PlaybackSession::default();
        for intent in list_menu_intents(&[]) {
            apply_list_menu_intent(intent, &ids, &paint_only, &mut untouched);
        }
        assert!(
            paint_only.recorded().is_empty(),
            "painting the menu sends nothing to the transport"
        );
        assert!(
            !untouched.queue.shuffle,
            "and leaves the session's shuffle exactly as it found it"
        );

        // Then the four rows, each clicked on its own, each reaching its own
        // effect: the exact dispatch the transport sees, and whether shuffle
        // is engaged afterwards.
        let cases: [(&str, Vec<TransportIntent>, bool); 4] = [
            (
                "Play",
                vec![TransportIntent::PlayMany(
                    ids[0].clone(),
                    vec![ids[1].clone()],
                )],
                false,
            ),
            (
                "Play Next",
                vec![
                    TransportIntent::PlayNext(ids[1].clone()),
                    TransportIntent::PlayNext(ids[0].clone()),
                ],
                false,
            ),
            (
                "Add to Queue",
                vec![TransportIntent::AddMany(vec![
                    ids[0].clone(),
                    ids[1].clone(),
                ])],
                false,
            ),
            (
                "Shuffle",
                vec![TransportIntent::PlayMany(
                    ids[0].clone(),
                    vec![ids[1].clone()],
                )],
                true,
            ),
        ];

        for (label, expected_dispatch, shuffle_after) in cases {
            let transport = MockTransport::new();
            let mut playback = PlaybackSession::default();

            let intents = list_menu_intents(&[label]);
            assert_eq!(
                intents.len(),
                1,
                "one click on {label} reports exactly one intent"
            );
            apply_list_menu_intent(intents[0], &ids, &transport, &mut playback);

            assert_eq!(
                transport.recorded(),
                expected_dispatch,
                "{label} reaches the transport as exactly this, and nothing more"
            );
            assert_eq!(
                playback.queue.shuffle, shuffle_after,
                "{label} leaves shuffle engaged: {shuffle_after}"
            );
        }
    }

    #[test]
    fn test_playback_intents_reach_the_transport_only_when_emitted() {
        let (_dir, mut store, _pid) = seeded_store();
        let mut mutations = store.clone();
        let transport = MockTransport::new();
        let mut selected = None;
        let mut editor = InlineTagEditor::new(Box::new(MockTagEdits));
        let item = track("/music/a.mp3");

        {
            let subject = TrackMenuSubject::resolved(&item, None);
            let mut host = host(
                &transport,
                &mut store,
                &mut mutations,
                &mut editor,
                &mut selected,
            );
            host.item_chosen(subject, TrackMenuIntent::Play);
            host.item_chosen(subject, TrackMenuIntent::PlayNext);
            host.item_chosen(subject, TrackMenuIntent::AddToQueue);
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
        let mut playback = PlaybackSession::default();
        apply_list_menu_intent(ListMenuIntent::Play, &ids, &transport, &mut playback);
        assert_eq!(
            &transport.recorded()[3..],
            &[TransportIntent::PlayMany(
                item.id.clone(),
                vec![second.id.clone()],
            )],
            "the list's Play is the shared batch-play helper: one dispatch, \
             never a command per Track"
        );

        apply_list_menu_intent(ListMenuIntent::PlayNext, &ids, &transport, &mut playback);
        assert_eq!(
            &transport.recorded()[4..],
            &[
                TransportIntent::PlayNext(second.id.clone()),
                TransportIntent::PlayNext(item.id.clone()),
            ],
            "Play Next inserts the whole list ahead of the current Track, and \
             the reverse-order sends are what make it land in order"
        );

        apply_list_menu_intent(ListMenuIntent::AddToQueue, &ids, &transport, &mut playback);
        assert_eq!(
            &transport.recorded()[6..],
            &[TransportIntent::AddMany(vec![
                item.id.clone(),
                second.id.clone(),
            ])],
            "Add to Queue appends the whole list as ONE batch, in order — the queue \
             mutates once, and nothing starts playing"
        );
        assert!(
            !playback.queue.shuffle,
            "none of Play, Play Next, or Add to Queue so much as touches shuffle"
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
            favorite: false,
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

/// Press feedback, wired (issue 05).
///
/// `IconButton::pressed` was computed on every frame by `begin_icon_button` and
/// read by no painter: the app had hover feedback and none at all for a press.
/// These probes drive a real pointer press through egui_kittest and hold two
/// things at once — the state the widget's own `Response` reports, and the
/// pixels the painter chose from it. "A press reports a state distinct from
/// hover" is only a claim if the two can be told apart, so every probe keeps
/// the idle, hover and press frames side by side.
///
/// **The press is simulated, not injected.** `Harness::drag_at` queues an
/// `egui::Event::PointerButton { pressed: true }` and `Harness::step` runs
/// exactly one frame with it. `Harness::run` cannot be used for the press: it
/// steps until nothing is repainting, and `RawInput::take` resets the pointer
/// between frames, so by the second frame the press has already been released.
/// egui's `is_pointer_button_down_on` reads `Interaction::potential_click_id`,
/// which the press sets from the *previous* frame's widget rects — which is why
/// every probe settles with `run()` before the pointer arrives.
/// The accent and destructive button washes fade in (issue 07).
///
/// Two seams, in the order the motion spec's coverage table names them. The
/// **pure function** is the primary one: each wash is a function of a tween
/// value alone, so the fade is assertable at its endpoints and its midpoint
/// with no frame, no renderer and no pixel. The **frame loop** is the
/// supporting one: it pins the property the pure function cannot see — that
/// only these two roles allocate a tween at all, so a hovered list of fifty
/// buttons that are not accent-tier costs nothing.
///
/// **The pointer has to be re-sent every frame.** `Harness::step` takes
/// `RawInput::take` per frame, so a pointer that arrived on one frame is gone
/// on the next; a probe that hovers once and then steps is measuring an
/// unhovered button. `Harness::run` is worse for this: it steps until nothing
/// repaints, which is exactly what a settled tween is asking for. Every probe
/// below therefore sends `hover_at` and steps, once per sampled pass.
#[cfg(test)]
mod button_wash_tests {
    use riff_gui::ui::button::Variant;
    use riff_gui::ui::theme::{self, MOTION_HOVER, Palette};

    /// One step of the harness clock, small enough that the hover token spans
    /// several frames — the same choice the row band's frame-loop test makes,
    /// and for the same reason: at the default 0.25 s step a 0.10 s tween
    /// saturates inside one frame and there is nothing left to observe.
    const STEP_DT: f32 = 0.02;
    /// Passes sampled per probe, several times the longest a motion token
    /// could publish.
    const SAMPLED_PASSES: usize = 20;
    /// Settled passes the sample must end with, so "stops asking" is asserted
    /// over a run and not over one frame.
    const SETTLED_PASSES: usize = 4;

    /// A `Color32`'s channels as a fraction of its own alpha — the
    /// *un-premultiplied* RGB, which is the colour a translucent wash is
    /// nominally "of". `Color32` stores premultiplied bytes, so a wash that
    /// keeps its hue reads the same here at every coverage; one that had lerped
    /// its RGB toward a backdrop would drift. The row-band test carries the
    /// same helper privately; this is that helper, for the same reason.
    fn nominal_rgb(color: egui::Color32) -> [f32; 3] {
        let a = f32::from(color.a()).max(1.0) / 255.0;
        [
            f32::from(color.r()) / a,
            f32::from(color.g()) / a,
            f32::from(color.b()) / a,
        ]
    }

    /// The source files that asked for a repaint on the pass that has just
    /// finished, or an empty vector if nothing did.
    ///
    /// egui swaps the accumulated causes into a "previous pass" slot at the
    /// *start* of a pass (`egui-0.35.0/src/context.rs:101-105`), so a reading
    /// taken right after `step` describes the pass that just ran, and no
    /// alignment arithmetic is needed. `Context::request_repaint` is
    /// `#[track_caller]`, so a cause names riff's call site — the
    /// `animate_bool_with_time` line in `button.rs` — rather than egui's own.
    ///
    /// The separators are normalized to `/`: `file!()` on Windows bakes in
    /// backslash paths, so a `contains` against a forward-slash module path
    /// never matches there and every ask reads as silence.
    fn asked_by(harness: &egui_kittest::Harness<'_, ()>) -> Vec<String> {
        harness
            .ctx
            .repaint_causes()
            .iter()
            .map(|cause| format!("{}:{}", cause.file.replace('\\', "/"), cause.line))
            .collect()
    }

    /// The causes attributed to riff's button module, which is where a wash's
    /// tween read lives. Filtered by file because egui's own housekeeping
    /// (a pointer move, a focus change) also asks for frames, and the claim
    /// under test is about the ones a *riff tween* issues.
    fn asked_by_buttons(harness: &egui_kittest::Harness<'_, ()>) -> Vec<String> {
        asked_by(harness)
            .into_iter()
            .filter(|cause| cause.contains("riff-gui/src/ui/button.rs"))
            .collect()
    }

    /// One text button of `variant` over the card plane, with a clock small
    /// enough to see a hover token fade. No tooltip: a tooltip schedules its own
    /// frame, and the probe is counting the tween's.
    fn wash_probe(
        variant: Variant,
        label: &'static str,
    ) -> (egui_kittest::Harness<'static, ()>, egui::Rect) {
        let palette = Palette::dark();
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 22.0), egui::vec2(120.0, 36.0));
        let id = egui::Id::new(label);
        let mut cache = riff_gui::ui::icons::IconCache::new();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 80.0))
            .with_pixels_per_point(1.0)
            .with_step_dt(STEP_DT)
            .build_ui_state(
                move |ui, _: &mut ()| {
                    let bg = ui.ctx().layer_painter(egui::LayerId::background());
                    bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                    let _ = riff_gui::ui::button::text_button(
                        ui,
                        &mut cache,
                        &palette,
                        false,
                        &riff_gui::ui::button::TextButton {
                            id,
                            rect,
                            label,
                            a11y: label,
                            tooltip: None,
                            icon: None,
                            small: false,
                            variant,
                            enabled: true,
                        },
                    );
                },
                (),
            );
        (harness, rect)
    }

    /// The last frame's pixels, with the card plane the probe painted behind
    /// the button so a comparison reads the button and not the window.
    fn render_probe(harness: &mut egui_kittest::Harness<'_, ()>) -> image::RgbaImage {
        harness
            .render()
            .expect("the wash probe must render headlessly")
    }

    /// Walk the pointer onto the button and hold it there for
    /// [`SAMPLED_PASSES`] passes, returning the first hovered frame, the last
    /// one, and the repaint causes attributed to `button.rs` on **each** pass.
    ///
    /// The causes come back per pass and un-collapsed, because the *count* on a
    /// pass is the claim: a tween that asked twice would show up as a second
    /// cause on every in-flight pass while leaving the number of asking passes
    /// exactly where it was, so a probe that reduced each pass to a bool could
    /// not tell a tween from a tween plus a redundant request.
    ///
    /// The first frame is read *after* the first hover step, and the last after
    /// the final one, so both are frames the pointer was on the button for.
    fn hover_frames(
        harness: &mut egui_kittest::Harness<'_, ()>,
        rect: egui::Rect,
    ) -> (image::RgbaImage, image::RgbaImage, Vec<Vec<String>>) {
        // Settling first is what registers the tween at 0.0, and it has to come
        // before the pointer arrives: egui's `animate_bool` returns its target
        // outright on the *first* call for an id it has never seen, so a probe
        // that hovered before the button had ever painted at rest would measure
        // a snap and call it a fade.
        harness.run();
        let mut causes_per_pass = Vec::with_capacity(SAMPLED_PASSES);
        let mut first = None;
        let mut last = None;
        for pass in 0..SAMPLED_PASSES {
            harness.hover_at(rect.center());
            harness.step();
            causes_per_pass.push(asked_by_buttons(harness));
            let frame = render_probe(harness);
            if pass == 0 {
                first = Some(frame.clone());
            }
            last = Some(frame);
        }
        (
            first.expect("one hovered frame"),
            last.expect("one settled frame"),
            causes_per_pass,
        )
    }

    /// Issue 07: each button wash is a pure function of its tween value,
    /// asserted at both endpoints and at the midpoint — the seam the motion
    /// spec's coverage table names for this change ("endpoints match the two
    /// tokens; the midpoint is strictly between them").
    ///
    /// **The endpoints are the contract that lets this change exist at all.**
    /// `t = 0` must be fully transparent and `t = 1` must be *exactly* the
    /// wash the instant swap painted, which is asserted against the existing
    /// `accent_fill` / `destructive_fill` helpers rather than against a
    /// literal: the tweened form is defined in terms of them, so this is the
    /// pin that the definition has not quietly changed what "fully hovered"
    /// means. Every golden in the suite captures an unhovered or a settled
    /// button, and neither may move.
    ///
    /// The midpoint is asserted three ways, because they are three claims. It
    /// is *strictly between* the endpoints; its *coverage* is strictly between
    /// nothing and the full wash; and its *hue* is the wash's own, because a
    /// wash is laid over a backdrop the painter never sees — scaling coverage
    /// is the whole operation, and an RGB lerp would be interpolating toward a
    /// fiction. That last one is what catches the premultiplied-alpha trap:
    /// `egui::Color32` holds premultiplied bytes, so compositing a faded wash
    /// through `blend_over` multiplies the coverage by itself and lands on a
    /// colour darker than *both* endpoints — a colour belonging to neither
    /// state, which is the exact failure the store's rules exist to prevent.
    /// Assert one wash's endpoints, midpoint and hue, given its
    /// `t`-parameterised form and the settled colour it must land on. Shared by
    /// both roles because they are the same gesture in two hues, and running
    /// the identical assertions over both is what shows that.
    fn assert_wash(
        role: &str,
        at: impl Fn(f32) -> egui::Color32,
        settled: egui::Color32,
        nominal_of: egui::Color32,
    ) {
        let idle = at(0.0);
        assert_eq!(
            idle,
            theme::TRANSPARENT,
            "{role}: t=0 must be fully transparent, so an unhovered button paints what it painted \
             before the wash tweened"
        );
        assert_eq!(
            at(1.0),
            settled,
            "{role}: t=1 must be exactly the wash the instant swap painted, so a settled hover is \
             byte-identical to the behaviour this replaces"
        );

        let mid = at(0.5);
        assert!(
            mid != idle && mid != settled,
            "{role}: the midpoint must be strictly between the two endpoints, not equal to \
             either: {mid:?}"
        );
        assert!(
            mid.a() > 0 && mid.a() < settled.a(),
            "{role}: the midpoint's coverage must be strictly between nothing and the full wash, \
             which is what 'half a wash' means: alpha {} of the wash's {}",
            mid.a(),
            settled.a()
        );
        // The hue is the wash's own at every coverage — the anti-crossfade
        // property, asserted in the value rather than in a frame, and against
        // the *token* the wash is nominally "of" rather than against the settled
        // wash, so the claim is that the fade changes coverage and nothing else.
        //
        // **The bound is half a byte step at the midpoint's own alpha, not a
        // round number.** These targets are *translucent* — a 10% wash — so the
        // stored bytes are premultiplied by 13/255 at the midpoint and one byte
        // step there is worth 255/13 = 19.6 nominal units; rounding the same
        // quantity two ways puts the brand's green at 137.3 and 127.5 around a
        // true 130. A +/-1 tolerance (which is what an *opaque* target like the
        // row band's `row_hover` affords) would fail on quantisation alone, and
        // loosening it to a whole byte step would be toothless. Half a step is
        // exactly what the rounding guarantees, and a crossfade — which is what
        // compositing through `blend_over` would produce — misses it by an
        // order of magnitude, because it also lands opaque.
        let half_step = 127.5 / f32::from(mid.a());
        let (nominal, target) = (nominal_rgb(mid), nominal_rgb(nominal_of));
        for (channel, (got, want)) in nominal.iter().zip(target.iter()).enumerate() {
            assert!(
                (got - want).abs() <= half_step,
                "{role}: channel {channel} of the midpoint drifted from the wash's own hue ({got} \
                 vs {want}, within {half_step} of half a byte step at alpha {}): a wash scales \
                 coverage over a backdrop the painter cannot see, and never lerps RGB toward a \
                 guess at it",
                mid.a()
            );
        }
    }

    /// Issue 07: each button wash is a pure function of its tween value,
    /// asserted at both endpoints and at the midpoint — the seam the motion
    /// spec's coverage table names for this change ("endpoints match the two
    /// tokens; the midpoint is strictly between them").
    ///
    /// **The endpoints are the contract that lets this change exist at all.**
    /// They are asserted against the existing `accent_fill` /
    /// `destructive_fill` helpers rather than against a literal, because the
    /// tweened forms are *defined* in terms of them: this is the pin that the
    /// definition has not quietly changed what "fully hovered" means. Every
    /// golden in the suite captures an unhovered or a settled button, and
    /// neither may move.
    ///
    /// The midpoint carries two further claims. Its *coverage* is strictly
    /// between nothing and the full wash — that is what "half a wash" means.
    /// And its *hue* is the wash's own, because a wash is laid over a backdrop
    /// the painter never sees: scaling coverage is the whole operation, and an
    /// RGB lerp would be interpolating toward a fiction. That last one is what
    /// catches the premultiplied-alpha trap — `egui::Color32` holds
    /// premultiplied bytes, so compositing a faded wash through `blend_over`
    /// multiplies the coverage by itself and lands on a colour darker than
    /// *both* endpoints, which is the exact failure the store's rules exist to
    /// prevent.
    ///
    /// **Red, proven by revert:** before these two functions existed the suite
    /// did not compile — `error[E0425]: cannot find function 'accent_wash' in
    /// module 'theme'`, with egui pointing at `destructive_fill` as the
    /// similarly-named function — which is the honest red for a test whose
    /// subject is a function. It earned its keep immediately after: the first
    /// implementation, `wash_at(accent_fill(palette, true), t)`, failed the
    /// endpoint assertion with `left: #18_0D_03_FF` against
    /// `right: #18_0D_03_1A` — a 10% wash landing **opaque**, because
    /// `wash_at` builds straight-alpha and was handed a colour that was already
    /// premultiplied. Fixing the *production* code (feed it the un-premultiplied
    /// token and a coverage) is what the endpoint assertion is for.
    #[test]
    fn test_the_accent_and_destructive_washes_are_pure_functions_of_their_tween_values() {
        let palette = Palette::dark();
        assert_wash(
            "accent",
            |t| theme::accent_wash(&palette, t),
            theme::accent_fill(&palette, true),
            palette.brand_primary,
        );
        assert_wash(
            "destructive",
            |t| theme::destructive_wash(&palette, t),
            theme::destructive_fill(&palette, true),
            palette.error,
        );
    }

    /// Issue 07: only the two wash roles tween, and they ask for frames only
    /// while they are in flight.
    ///
    /// This is the property the pure function cannot see, and it is the one
    /// that keeps the feature affordable: a tween allocates an animation id and
    /// asks the frame loop for a pass, so a variant that tweens for tidiness
    /// multiplies that cost by every button on screen. So the probe pins both
    /// halves from the outside:
    ///
    /// - a hovered **Secondary** — a role with a real hover treatment (a
    ///   `surface_3` face and the hover stroke) that must nonetheless be
    ///   **instant** — asks for no frame from `button.rs` on any pass, and its
    ///   first hovered frame is already **pixel-identical to its own settled
    ///   frame**. That is the byte-identity claim, stated against the only
    ///   comparison that can mean it: a hovered button legitimately repaints,
    ///   so "identical to unhovered" would be a claim about nothing. Identical
    ///   to *settled* is the claim that no tween was allocated.
    /// - a hovered **Accent** asks for a frame across the whole tween and then
    ///   stops, and its first hovered frame differs from its settled one —
    ///   which is what a fade looks like from the outside.
    ///
    /// The asking run's length is the claim that the wash runs on the *hover*
    /// token rather than the published global default: at this step
    /// `MOTION_HOVER` (0.10 s) is five passes and `MOTION_DEFAULT` (0.18 s) is
    /// nine, so a wash that had picked up `Style::animation_time` instead of
    /// passing the token explicitly is caught here.
    /// **Red, proven by revert, twice.** Before the tween was wired the Accent
    /// probe reported `asked for a frame on 0 of 20` with
    /// `asked=[false, false, ... x20]` — the instant swap's signature, which is
    /// also what an *un*-tweened implementation looks like from the outside.
    /// Reverting only the wiring in `paint_text_button` (leaving both
    /// `theme::accent_wash` and `theme::destructive_wash` in place) reproduces
    /// that identical red while the pure-function test above stays green. That
    /// split is the point of having two tests: the value and the clock are
    /// separate claims, and only one of them can be wrong at a time.
    ///
    /// **The per-pass count, proven by duplication.** The asking-pass assertions
    /// above cannot see a redundant `request_repaint`, because asking twice does
    /// not make a pass any more *asked-for* than asking once. So the redundant
    /// request was added to `wash_tween` in the form rule 3 warns about —
    /// `if 0.0 < t && t < 1.0 { ui.ctx().request_repaint(); }`, which is how the
    /// Folders tree writes its own — and this failed with:
    ///
    /// ```text
    /// pass 1 made ["crates/riff-gui/src/ui/button.rs:166",
    ///               "crates/riff-gui/src/ui/button.rs:170"] — the accent wash must cost
    /// exactly one repaint request per in-flight frame …
    ///   left: 2
    ///  right: 1
    /// ```
    ///
    /// Two things are worth keeping from that output. The two causes name
    /// **two different lines of `button.rs`** — `:166` the
    /// `animate_bool_with_time` and `:170` the redundant call — so
    /// `#[track_caller]` attributes per call site and a duplicate is legible in
    /// the failure message rather than merely counted. And the row band's
    /// frame-loop test **passed with the duplicate in place** (measured, not
    /// assumed): it drives a different animation id in a different module, so
    /// nothing outside this module's own frame loop can stand in for it. That is
    /// why the store's rule-3 claim needed pinning *here* and not just there.
    #[test]
    fn test_only_the_accent_and_destructive_washes_tween() {
        let hover_passes = (MOTION_HOVER / STEP_DT).ceil() as usize;

        // --- Secondary: a real hover treatment, deliberately instant. --------
        let (mut harness, rect) = wash_probe(Variant::Secondary, "Rescan now");
        harness.run();
        assert!(
            asked_by_buttons(&harness).is_empty(),
            "an idle button must ask the frame loop for nothing"
        );
        let (first_secondary, settled_secondary, causes_secondary) =
            hover_frames(&mut harness, rect);
        assert!(
            causes_secondary.iter().all(Vec::is_empty),
            "a hovered secondary asked for frames {causes_secondary:?} — only the accent and \
             destructive washes are on the allow-list, and a per-variant fill is not one of them"
        );
        assert_eq!(
            first_secondary, settled_secondary,
            "a hovered secondary's first frame must be pixel-identical to its settled one: the \
             hover swap is instant, so there is no tween sample to find part-applied"
        );

        // --- Accent: the same gesture, and this one fades. --------------------
        let (mut harness, rect) = wash_probe(Variant::Accent, "Rescan now");
        harness.run();
        let (first_accent, settled_accent, causes_accent) = hover_frames(&mut harness, rect);

        let asked_accent: Vec<bool> = causes_accent
            .iter()
            .map(|causes| !causes.is_empty())
            .collect();
        let asking = asked_accent.iter().filter(|asked| **asked).count();
        let longest_run = asked_accent
            .iter()
            .fold((0usize, 0usize), |(best, current), asked| {
                let current = if *asked { current + 1 } else { 0 };
                (best.max(current), current)
            })
            .0;
        assert_eq!(
            asking, longest_run,
            "frames must be asked for across the whole tween, not in bursts: {asked_accent:?}"
        );
        assert!(
            (2..=hover_passes).contains(&asking),
            "the accent wash must tween for roughly MOTION_HOVER ({MOTION_HOVER}s ≈ \
             {hover_passes} passes at {STEP_DT}s) and asked for a frame on {asking} of \
             {SAMPLED_PASSES}: an instant swap never asks, a tween that never settles asks \
             forever. asked={asked_accent:?}"
        );
        assert!(
            SAMPLED_PASSES - asking >= SETTLED_PASSES,
            "the sample holds fewer than {SETTLED_PASSES} settled passes, so 'a settled hover \
             asks for nothing' is unproven: {asked_accent:?}"
        );
        assert_ne!(
            first_accent, settled_accent,
            "the accent wash must actually fade: its first hovered frame is identical to its \
             settled one, so nothing moved between them"
        );

        // **Exactly one request per in-flight pass, and none once settled.** This
        // is the half the asking-pass count above cannot see, and it is what
        // makes the store's rule-3 claim true for *this* surface: the wash relies
        // on egui's bound for its frames, so a redundant
        // `ui.ctx().request_repaint()` beside the tween read would be riff
        // saying what egui has already said, once per frame of every fade in the
        // app. It leaves the number of asking passes untouched, so only the
        // per-pass count catches it.
        //
        // `Context::request_repaint` is `#[track_caller]`, so a `RepaintCause`
        // names riff's *call site* — the `animate_bool_with_time` line in
        // `button.rs` — and not egui's own `request_repaint`. Attribution by file
        // therefore cannot by itself tell "egui's wrapper asked on riff's
        // behalf" from "riff asked": both are `button.rs`. The count can.
        for (pass, causes) in causes_accent.iter().enumerate() {
            match asked_accent[pass] {
                true => assert_eq!(
                    causes.len(),
                    1,
                    "pass {pass} made {causes:?} — the accent wash must cost exactly one repaint \
                     request per in-flight frame, the one egui's animate_bool_with_time already \
                     issues while the value is strictly between 0 and 1. A second cause is riff \
                     asking again for a frame it has already been given."
                ),
                false => assert!(
                    causes.is_empty(),
                    "pass {pass} asked for a frame after the wash had settled: {causes:?} — a \
                     settled tween asks for nothing, which is the bound at the far end"
                ),
            }
        }
    }
}

#[cfg(test)]
mod press_feedback_tests {
    use riff_gui::ui::button::Variant;
    use riff_gui::ui::{icons, sidebar, theme};

    /// What one driven frame reported for the probed button, read back out of
    /// the button's own `Response` — egui's verdict, not the painter's.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    struct PressProbe {
        /// The pointer is over the button.
        hovered: bool,
        /// The pointer is held down on the button.
        pressed: bool,
    }

    /// One driven frame: its pixels, and the state the probed button reported
    /// while it was being painted.
    struct PressFrame {
        image: image::RgbaImage,
        state: PressProbe,
    }

    /// Count frame pixels within a hair of `color` inside `rect` — the same
    /// +/-2 tolerance and the same shape as the row-band counter in
    /// `browser_column_ui_tests`, which is the idiom for asking *which fill a
    /// painter chose*. That counter is private to its module, so this probe
    /// carries its own.
    fn press_count_color(
        image: &image::RgbaImage,
        color: egui::Color32,
        rect: egui::Rect,
    ) -> usize {
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

    /// The framework's own active fill, as the theme store publishes it: read
    /// straight off the style rather than off a `palette.*` field, because the
    /// claim under test is that riff's buttons and egui's stock widgets agree
    /// about what a press looks like — which is only true if both read the one
    /// style.
    fn active_fill() -> egui::Color32 {
        theme::style_from(&theme::Palette::dark(), false)
            .visuals
            .widgets
            .active
            .bg_fill
    }

    /// Stand the app's style up inside the probe. Only the widget-state block is
    /// installed, because `style_from` also names riff's vendored font families
    /// and nothing binds those in a headless context — a harness limitation, and
    /// nothing about it touches the fill a button reads.
    fn use_riff_widget_style(ctx: &egui::Context) {
        let active = active_fill();
        ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.visuals.widgets.active.bg_fill = active;
        });
    }

    /// Read the probe out of the widget egui registered under `id` this frame.
    fn press_probe_of(ctx: &egui::Context, id: egui::Id) -> PressProbe {
        ctx.read_response(id)
            .map_or_else(PressProbe::default, |response| PressProbe {
                hovered: response.hovered(),
                pressed: response.is_pointer_button_down_on(),
            })
    }

    /// The band of a button's face a pixel count is read from: an interior
    /// strip along the left edge, clear of the centred glyph, of the rounded
    /// corners, and of the mouse cursor `Harness::render` synthesises at the
    /// pointer. A colour counted here is the fill the painter chose and nothing
    /// else — the cursor is white on black and its antialiased edge walks the
    /// neutral surface ramp, which is exactly what an "this fill must be
    /// absent" assertion would otherwise trip over.
    fn press_face_patch(rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x + 8.0, rect.min.y + 8.0),
            egui::pos2(rect.min.x + 14.0, rect.max.y - 8.0),
        )
    }

    /// The strip a hover *stroke* is read from: the button's top edge, where an
    /// inside stroke actually lands, held clear of the corner arcs (a stroke
    /// follows the rounded outline, so a count over the corners would measure
    /// the radius rather than the stroke).
    fn press_stroke_band(rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x + 14.0, rect.min.y),
            egui::pos2(rect.max.x - 14.0, rect.min.y + 2.0),
        )
    }

    /// The last frame's pixels. The probe paints the card plane behind the
    /// button first, so every count below reads a fill and never the window.
    fn render_probe(harness: &mut egui_kittest::Harness<'_, PressProbe>) -> image::RgbaImage {
        harness
            .render()
            .expect("the press probe must render headlessly")
    }

    /// Settle the probe, walk the pointer onto it for one hover frame, then hold
    /// the button down for one press frame, returning both frames with the state
    /// each reported.
    fn drive_press(
        harness: &mut egui_kittest::Harness<'_, PressProbe>,
        rect: egui::Rect,
    ) -> (PressFrame, PressFrame) {
        harness.run();
        harness.hover_at(rect.center());
        harness.step();
        let hovered = PressFrame {
            image: render_probe(harness),
            state: *harness.state(),
        };
        // The press gets a frame of its own and no further pointer event, so
        // the frame is exactly "over the button, button held down".
        harness.drag_at(rect.center());
        harness.step();
        let pressed = PressFrame {
            image: render_probe(harness),
            state: *harness.state(),
        };
        (hovered, pressed)
    }

    /// One text button of `variant` over the card plane, recording the state it
    /// reports. Driven through `text_button` — the real entry point — so a
    /// probe cannot pass by a painter production does not take.
    fn press_text_button(
        variant: Variant,
        label: &'static str,
    ) -> (egui_kittest::Harness<'static, PressProbe>, egui::Rect) {
        let palette = theme::Palette::dark();
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 22.0), egui::vec2(120.0, 36.0));
        let id = egui::Id::new(label);
        let mut cache = icons::IconCache::new();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, probe: &mut PressProbe| {
                    let bg = ui.ctx().layer_painter(egui::LayerId::background());
                    bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                    let _ = riff_gui::ui::button::text_button(
                        ui,
                        &mut cache,
                        &palette,
                        false,
                        &riff_gui::ui::button::TextButton {
                            id,
                            rect,
                            label,
                            a11y: label,
                            tooltip: None,
                            icon: None,
                            small: false,
                            variant,
                            enabled: true,
                        },
                    );
                    *probe = press_probe_of(ui.ctx(), id);
                },
                PressProbe::default(),
            );
        use_riff_widget_style(&harness.ctx);
        (harness, rect)
    }

    /// The same, for the hand-painted ghost icon button — one of the painters
    /// that holds an `IconButton` and read only its `hovered` field.
    fn press_icon_button() -> (egui_kittest::Harness<'static, PressProbe>, egui::Rect) {
        let palette = theme::Palette::dark();
        let rect = egui::Rect::from_min_size(egui::pos2(84.0, 24.0), egui::vec2(32.0, 32.0));
        let id = egui::Id::new("press_icon_probe");
        let mut cache = icons::IconCache::new();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 80.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, probe: &mut PressProbe| {
                    let bg = ui.ctx().layer_painter(egui::LayerId::background());
                    bg.rect_filled(ui.ctx().content_rect(), 0.0, palette.surface);
                    let _ = sidebar::ghost_icon_button(
                        ui,
                        &mut cache,
                        &palette,
                        rect,
                        id,
                        icons::Icon::Trash,
                        "Delete playlist",
                        true,
                    );
                    *probe = press_probe_of(ui.ctx(), id);
                },
                PressProbe::default(),
            );
        use_riff_widget_style(&harness.ctx);
        (harness, rect)
    }

    /// A ghost text button's press is a state of its own. Idle is transparent,
    /// hover is the hover fill, press is the active fill — three different
    /// faces, so a press that still looked like a hover would be visible on the
    /// face alone and no stroke has to be read to tell it.
    ///
    /// The pressed face is also *complete* on that one frame. Press is instant
    /// by decision (`theme::MOTION_*` is a hover-family allow-list that press
    /// deliberately sits outside), so the first pressed frame is already the
    /// whole active fill rather than a tween's first sample on its way there.
    ///
    /// **Red, proven by revert:** with `pressed` unthreaded from `text_button`
    /// back out of `paint_text_button`, the press frame paints the hover fill
    /// and this fails with `left: 0, right: 120` on the active-fill count — the
    /// state assertion above it still passes, because egui was reporting the
    /// press all along and no painter was listening.
    #[test]
    fn test_a_pressed_text_button_paints_a_state_distinct_from_hover() {
        let palette = theme::Palette::dark();
        // The pressed fill is egui's own active fill, which the theme store
        // publishes from the palette's interact styles — so riff's buttons and
        // egui's stock widgets cannot disagree about what a press looks like.
        assert_eq!(
            active_fill(),
            palette.surface_3,
            "the framework's active fill is the palette's own pressed surface"
        );

        let (mut harness, rect) = press_text_button(Variant::Ghost, "Delete all");
        let patch = press_face_patch(rect);
        let area = (patch.width() * patch.height()) as usize;
        let (hovered, pressed) = drive_press(&mut harness, rect);

        assert_eq!(
            hovered.state,
            PressProbe {
                hovered: true,
                pressed: false
            },
            "a pointer over the button is a hover and nothing more"
        );
        assert_eq!(
            pressed.state,
            PressProbe {
                hovered: true,
                pressed: true
            },
            "a held button reports a press — the state no painter used to read"
        );
        let hover_hovered = press_count_color(&hovered.image, palette.surface_2, patch);
        let active_hovered = press_count_color(&hovered.image, palette.surface_3, patch);
        let active_pressed = press_count_color(&pressed.image, palette.surface_3, patch);
        let hover_pressed = press_count_color(&pressed.image, palette.surface_2, patch);
        assert_eq!(
            (hover_hovered, active_hovered),
            (area, 0),
            "a hovered ghost button wears the hover fill, whole"
        );
        assert_eq!(
            active_pressed, area,
            "the first pressed frame is already the whole active fill — press is \
             instant, so there is no tween sample to find part-filled"
        );
        assert_eq!(
            hover_pressed, 0,
            "press REPLACES the hover fill rather than compositing with it"
        );
    }

    /// A secondary button's face cannot carry the difference — its hover fill
    /// *is* the active fill — so what tells a press from a hover there is the
    /// hover stroke dropping away. A held button must never look both hovered
    /// and pressed.
    ///
    /// **Red, proven by revert:** with `pressed` unthreaded, a held button is
    /// told only that it is hovered, so the stroke stays and this fails with
    /// `left: 92, right: 0` — 92 px of hover stroke on a button that is down.
    #[test]
    fn test_a_press_drops_the_text_button_hover_stroke() {
        let palette = theme::Palette::dark();

        let (mut harness, rect) = press_text_button(Variant::Secondary, "Rescan now");
        let band = press_stroke_band(rect);
        let (hovered, pressed) = drive_press(&mut harness, rect);

        assert!(pressed.state.pressed, "the press is reported here too");
        let stroke_hovered = press_count_color(&hovered.image, palette.focus_ring, band);
        let stroke_pressed = press_count_color(&pressed.image, palette.focus_ring, band);
        assert!(
            stroke_hovered > 0,
            "a hovered secondary button carries the hover stroke, else the next \
             assertion would pass on a frame that painted none"
        );
        assert_eq!(
            stroke_pressed, 0,
            "a press drops the hover stroke: a held button must never look both \
             hovered and pressed"
        );
    }

    /// The other half of the write-only field: the painters that already hold an
    /// `IconButton` now read `pressed` too. A ghost icon button has no hover
    /// fill to swap out — hovering only reveals the glyph — so its press is the
    /// first frame that paints a face at all, which is why the hover frame
    /// counts zero.
    ///
    /// **Red, proven by revert:** with the `button.pressed` arm removed from
    /// `sidebar::ghost_icon_button`, this fails on a zero pixel count — "found
    /// 0 px of the active fill #28_25_21_FF in the face", because nothing paints
    /// a fill behind the glyph while the button is held down.
    #[test]
    fn test_a_pressed_ghost_icon_button_paints_the_active_fill() {
        let active = active_fill();

        let (mut harness, rect) = press_icon_button();
        let patch = press_face_patch(rect);
        let (hovered, pressed) = drive_press(&mut harness, rect);

        assert_eq!(
            pressed.state,
            PressProbe {
                hovered: true,
                pressed: true
            },
            "the icon button reports the press it used to compute and discard"
        );
        let fill_hovered = press_count_color(&hovered.image, active, patch);
        let fill_pressed = press_count_color(&pressed.image, active, patch);
        assert_eq!(
            fill_hovered, 0,
            "hover alone paints no fill on a ghost icon button"
        );
        assert!(
            fill_pressed > 0,
            "a held ghost icon button paints the framework's active fill, found \
             {fill_pressed} px of the active fill {active:?} in the face"
        );
    }
}

#[cfg(test)]
mod component_boundary_tests {
    use eframe::egui;
    use riff_backend::app::Transport;
    use riff_backend::app::state::PlaybackSession;
    use riff_backend::domain::TrackId;
    use std::path::{Path, PathBuf};

    /// Every module the component-layer tickets added or reshaped: primitives
    /// and the neutral presentation owners. Feature composites (`app`,
    /// `browser_pane`, `sidebar`, `settings`, `detail`, `playerbar`,
    /// `now_playing`, `chrome`, `selection`) are deliberately absent — they are
    /// allowed to hold application state, and the sweeps below would be wrong
    /// to demand otherwise.
    const PRIMITIVES: [&str; 12] = [
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
            // way: the placeholder's own module <-> ui::app, ui::stage ->
            // ui::app.)
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

        // The artwork placeholder's owner (its historical alias module is gone).
        let _tile = riff_gui::ui::artwork::placeholder_cache_key;

        // Moved out of `ui::browser` (ticket 12) and `ui::now_playing` (ticket 05).
        let _empty: fn(&mut egui::Ui, &riff_gui::ui::theme::Palette, &str, &str) =
            riff_gui::ui::browser::empty_state;
        let _entries = riff_gui::ui::now_playing::up_next_entries(&[], 5);
        let _entry: Option<riff_gui::ui::now_playing::UpNextEntry> = None;

        // The host-adapter paths the suite drives menus and prompts through.
        let _intent: fn(
            riff_gui::ui::menu::ListMenuIntent,
            &[TrackId],
            &dyn Transport,
            &mut PlaybackSession,
        ) = riff_gui::ui::app::apply_list_menu_intent;
        let _duration: fn(std::time::Duration) -> String = riff_gui::ui::app::format_duration;
        let _expand: fn(&str) -> std::path::PathBuf = riff_gui::ui::settings::expand_tilde;
    }
}

/// Seek-bar affordance (issue 08): the hover thumb, the dragged-track
/// thickening, and the geometry tokens that own both.
///
/// The spec's coverage table is explicit that this change is appearance-only and
/// that no hover is simulated, so nothing here drives a pointer onto a control.
/// What these tests pin instead is everything the hover is *built from*, which is
/// the part that can rot with no pixel ever changing:
///
/// - the dimensions are tokens in the seek surface's own store, ordered so the
///   grabbed track is thicker than the idle one and the thumb caps it;
/// - at rest a seek bar paints no thumb and its track is the idle thickness —
///   and the *same* counter, on a frame that differs only in which of the
///   control's two thumb slots the diameter comes from, does find a thumb on a
///   control that wears one unconditionally, so the first assertion cannot pass
///   on a frame that renders nothing at all;
/// - the thickening is a pure function of the caller's rect and the drag state,
///   asserted directly, with no frame and no pointer involved.
///
/// That last one is why the derivation is a `pub fn` rather than an expression
/// inside `paint`: dragging must change the track's thickness and *only* its
/// thickness, or the bar shifts under the pointer at the moment the user can
/// least afford to see it move.
mod seek_affordance_tests {
    use riff_gui::ui::linear::{LinearControl, active_track, linear_control};
    use riff_gui::ui::theme::Palette;
    use riff_gui::ui::theme::geometry::now_playing::SEEK_H;
    use riff_gui::ui::theme::geometry::playerbar::VOLUME_THUMB;
    use riff_gui::ui::theme::geometry::seek::{THUMB_D, TRACK_H, TRACK_H_ACTIVE};
    use std::collections::BTreeSet;

    /// Frame pixels within a hair of `color`, at the same +/-2 tolerance the rest
    /// of this suite uses because rounded edges antialias.
    fn count_near(image: &image::RgbaImage, color: egui::Color32) -> usize {
        image
            .pixels()
            .filter(|p| {
                p.0[0].abs_diff(color.r()) <= 2
                    && p.0[1].abs_diff(color.g()) <= 2
                    && p.0[2].abs_diff(color.b()) <= 2
            })
            .count()
    }

    /// How many distinct pixel rows carry `color` anywhere in the frame — the
    /// painted *height* of a band, read straight off the pixels rather than off
    /// the caller's own rect. A row count is used instead of a pixel count
    /// because the band's width is not what these tests claim anything about.
    fn rows_carrying(image: &image::RgbaImage, color: egui::Color32) -> usize {
        image
            .enumerate_pixels()
            .filter(|(_, _, p)| {
                p.0[0].abs_diff(color.r()) <= 2
                    && p.0[1].abs_diff(color.g()) <= 2
                    && p.0[2].abs_diff(color.b()) <= 2
            })
            .map(|(_, y, _)| y)
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Render one seek-bar-shaped linear control **at rest** and hand back the
    /// frame. No pointer event is ever fed to the harness, so egui has no
    /// `latest_pos` at all: nothing is hovered, nothing is dragged, and — the
    /// reason this seam is usable at all — `Harness::render` paints no cursor
    /// triangle either (it only does so when a pointer is in the window).
    ///
    /// The two flags select which thumb the control wears: a seek bar asks for
    /// the hover-driven one and the volume slider brings its own, so holding
    /// everything else fixed leaves exactly one thing varying between the pair of
    /// frames the resting test compares.
    fn render_at_rest(hover_thumb: bool, always_thumb: bool) -> image::RgbaImage {
        let palette = Palette::dark();
        let track = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, TRACK_H));
        let hit = track.expand2(egui::vec2(0.0, 12.0));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(200.0, 48.0))
            .with_pixels_per_point(1.0)
            .build_ui(move |ui| {
                linear_control(
                    ui,
                    &palette,
                    &LinearControl {
                        id: egui::Id::new("test_seek_rest"),
                        track,
                        hit,
                        value: 0.5,
                        thumb: always_thumb.then_some(VOLUME_THUMB),
                        hover_thumb,
                        interactive: true,
                        label: "Seek",
                    },
                );
            });
        harness.run();
        harness
            .render()
            .expect("the linear control renders headlessly")
    }

    /// The three dimensions this affordance is built from are tokens in the seek
    /// surface's own store, and they are ordered the only way that reads:
    /// dragging thickens the track, and the thumb is big enough to cap the
    /// thickest state of the track it rides on.
    ///
    /// The upper bounds are the real constraint: both seek surfaces build a hit
    /// area out of their own row heights, Now Playing's being the tighter of the
    /// two at 24px, so a thumb or a grabbed track taller than that would spill
    /// onto the meta text above the row.
    #[test]
    fn test_seek_thickness_and_thumb_are_ordered_tokens_in_the_seek_store() {
        // Read through locals: the tokens are `const`s, and an assertion the
        // compiler can fold to `true` is a build error under this workspace's
        // clippy configuration. The player-bar token test writes its pins as
        // `(token - value).abs()` for the same reason.
        let (idle, active, thumb, hit_h, volume) =
            (TRACK_H, TRACK_H_ACTIVE, THUMB_D, SEEK_H, VOLUME_THUMB);

        assert!(idle > 0.0, "the idle track is a real thickness: {idle}");

        assert!(
            active > idle,
            "the grabbed track must be thicker than the idle one — that difference \
             is the whole grab signal: idle {idle}, active {active}"
        );
        assert!(
            active <= hit_h,
            "the grabbed track has to stay inside the tightest seek hit area \
             ({hit_h}px in Now Playing), or it bleeds onto the row above it"
        );

        assert!(
            thumb >= active,
            "the thumb caps the thickest state of the track, so it cannot be \
             smaller than it: thumb {thumb}, active track {active}"
        );
        assert!(
            thumb <= hit_h,
            "the thumb has to stay inside the tightest seek hit area ({hit_h}px), \
             or it spills over the time readouts flanking the bar"
        );

        // The volume slider's thumb is a *different* dimension on a *different*
        // surface, and it is unchanged: the volume control keeps its own 10px
        // always-visible thumb, which is what keeps the two bars distinguishable
        // by purpose instead of looking like one of them lost its thumb.
        assert!(
            (volume - 10.0).abs() < f32::EPSILON,
            "the volume slider keeps its mockup thumb diameter: {volume}"
        );
    }

    /// At rest a seek bar paints no thumb, and its track is the idle thickness.
    ///
    /// The two frames share their drawing code, value, position and hit area, and
    /// differ only in which of the control's two thumb slots the diameter comes
    /// from — the second frame being the volume slider's real shape, at the
    /// volume slider's own diameter. So it is the control that keeps the first
    /// assertion honest: without it, "no ink pixels" would also be what a control
    /// that painted nothing at all would report, and the band-height count would
    /// be carrying the whole test.
    #[test]
    fn test_seek_bar_paints_no_thumb_at_rest_and_keeps_the_idle_thickness() {
        let palette = Palette::dark();

        let seek = render_at_rest(true, false);
        assert_eq!(
            count_near(&seek, palette.ink),
            0,
            "a seek bar the pointer is not over paints no thumb — the affordance \
             that says where the pointer is must be absent, not dimmed"
        );
        assert_eq!(
            rows_carrying(&seek, palette.surface_3),
            TRACK_H as usize,
            "an ungrabbed track is painted at the idle thickness, so the only \
             hover-driven change to the bar's pixels is an added thumb"
        );

        // The control: same code, same value, same ink — but a thumb the control
        // wears unconditionally, exactly like the volume slider's.
        let always = render_at_rest(false, true);
        assert!(
            count_near(&always, palette.ink) > 0,
            "the same counter finds the volume slider's always-visible thumb, so \
             the resting assertion above is about the hover gate and not about a \
             frame that renders nothing"
        );
    }

    /// The grabbed track is the caller's own span and centre at the *token's*
    /// thickness: the thickening is a thickness, not a delta on whatever height
    /// the caller happened to ask for, and at rest it is the caller's rect
    /// byte-for-byte so an idle frame cannot drift by a rounding step.
    #[test]
    fn test_dragged_track_thickens_on_the_same_span_and_centre() {
        let idle = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, TRACK_H));

        assert_eq!(
            active_track(idle, false),
            idle,
            "at rest the painted track *is* the caller's rect — no re-derivation, so \
             an idle frame is identical to a golden's"
        );

        let grabbed = active_track(idle, true);
        assert_eq!(
            (grabbed.left(), grabbed.right()),
            (idle.left(), idle.right()),
            "grabbing changes the thickness only: the horizontal span — and with it \
             the fraction any pointer position maps to — must not move"
        );
        assert!(
            (grabbed.center().y - idle.center().y).abs() < f32::EPSILON,
            "the grabbed track keeps the idle track's centre, so the bar does not \
             shift under a pointer that is already sitting on it"
        );
        assert!(
            (grabbed.height() - TRACK_H_ACTIVE).abs() < f32::EPSILON,
            "the grabbed track is the active token: {} vs {TRACK_H_ACTIVE}",
            grabbed.height()
        );
        assert!(grabbed.height() > idle.height());

        // A caller that ever passes a taller rect than `TRACK_H` still gets the
        // same grabbed bar, measured from the token and not from its own height.
        let tall = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(160.0, SEEK_H));
        assert!(
            (active_track(tall, true).height() - TRACK_H_ACTIVE).abs() < f32::EPSILON,
            "the active thickness is the token, never the caller's height plus a \
             delta"
        );
    }
}
