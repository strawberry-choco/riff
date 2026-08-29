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
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
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
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
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
        let store = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
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
        let store = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
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
        riff_gui::ui::app::load_persisted_state(
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

    // --- "Edit Tags" modal state (REQ-ML-008) ---------------------------------

    /// A track carrying every supported tag value.
    fn fully_tagged_track() -> Track {
        let mut track = crate::test_utils::create_test_track(
            "music/artist/album/01.flac",
            "music/artist/album/01.flac",
        );
        track.metadata = TrackMetadata {
            title: Some("Original Title".to_string()),
            artist: Some("Original Artist".to_string()),
            album: Some("Original Album".to_string()),
            album_artist: Some("Original Album Artist".to_string()),
            genre: Some("Jazz".to_string()),
            year: Some(1959),
            track_number: Some(3),
            ..Default::default()
        };
        track
    }

    #[test]
    fn test_tag_edit_modal_opens_prefilled_with_current_track_tags() {
        let track = fully_tagged_track();

        let state = TagEditState::from_track(&track);

        // Every supported tag is pre-filled with the track's current value,
        // so the user edits in place instead of re-typing everything.
        assert_eq!(state.title, "Original Title");
        assert_eq!(state.artist, "Original Artist");
        assert_eq!(state.album, "Original Album");
        assert_eq!(state.album_artist, "Original Album Artist");
        assert_eq!(state.genre, "Jazz");
        assert_eq!(state.year, "1959");
        assert_eq!(state.track_number, "3");
        // The modal targets the clicked track's file.
        assert_eq!(state.track_id, track.id);
        assert_eq!(state.path, track.file_path);
        // Freshly opened: no error, no save in flight.
        assert!(state.error.is_none());
        assert!(!state.saving);
    }

    #[test]
    fn test_tag_edit_modal_prefill_starts_blank_for_untagged_track() {
        let track = crate::test_utils::create_test_track("plain.mp3", "plain.mp3");

        let state = TagEditState::from_track(&track);

        // Missing tags surface as empty fields, never as errors or
        // leftover placeholders.
        assert_eq!(state.title, "");
        assert_eq!(state.artist, "");
        assert_eq!(state.album, "");
        assert_eq!(state.album_artist, "");
        assert_eq!(state.genre, "");
        assert_eq!(state.year, "");
        assert_eq!(state.track_number, "");
        assert!(state.error.is_none());
        assert!(!state.saving);
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
                riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                    .unwrap();
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
        riff_gui::ui::app::load_persisted_state(
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
        // Surfaces: --riff-bg / --riff-surface / --riff-surface-2 / --riff-surface-3.
        assert_eq!(theme::SURFACE_BG, egui::Color32::from_rgb(0x0c, 0x0c, 0x10));
        assert_eq!(theme::SURFACE, egui::Color32::from_rgb(0x13, 0x13, 0x1a));
        assert_eq!(theme::SURFACE_2, egui::Color32::from_rgb(0x1b, 0x1b, 0x24));
        assert_eq!(theme::SURFACE_3, egui::Color32::from_rgb(0x23, 0x23, 0x2e));

        // Ink ladder: --riff-ink / --riff-ink-2 / --riff-ink-3.
        assert_eq!(theme::INK, egui::Color32::from_rgb(0xf4, 0xf4, 0xf5));
        assert_eq!(theme::INK_2, egui::Color32::from_rgb(0xa1, 0xa1, 0xaa));
        assert_eq!(theme::INK_3, egui::Color32::from_rgb(0x71, 0x71, 0x7a));
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
        assert_eq!(theme::BRAND_500, egui::Color32::from_rgb(0xf5, 0xa6, 0x23));
        assert_eq!(theme::BRAND_600, egui::Color32::from_rgb(0xd9, 0x8a, 0x0d));
        assert_eq!(theme::BRAND_700, egui::Color32::from_rgb(0xa6, 0x67, 0x09));
    }

    #[test]
    fn test_status_color_tokens_match_the_mockup() {
        // --riff-state-success/warning/error/info; warning aliases brand-500.
        assert_eq!(
            theme::STATE_SUCCESS,
            egui::Color32::from_rgb(0x22, 0xc5, 0x5e)
        );
        assert_eq!(theme::STATE_WARNING, theme::BRAND_500);
        assert_eq!(
            theme::STATE_ERROR,
            egui::Color32::from_rgb(0xef, 0x44, 0x44)
        );
        assert_eq!(theme::STATE_INFO, egui::Color32::from_rgb(0x3b, 0x82, 0xf6));
    }

    #[test]
    fn test_radius_scale_constants_match_the_mockup() {
        // --riff-radius-sm/md/lg/xl/full.
        assert!((theme::RADIUS_SM - 4.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_MD - 8.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_LG - 12.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_XL - 16.0).abs() < f32::EPSILON);
        assert!((theme::RADIUS_FULL - 999.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_chrome_dimension_constants_match_the_mockup() {
        // --riff-titlebar-h / --riff-sidebar-w / --riff-playerbar-h.
        assert!((theme::TITLEBAR_H - 56.0).abs() < f32::EPSILON);
        assert!((theme::SIDEBAR_W - 280.0).abs() < f32::EPSILON);
        assert!((theme::PLAYERBAR_H - 88.0).abs() < f32::EPSILON);
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
        // --riff-primary / --riff-ring alias brand-500; --riff-primary-foreground
        // is the deep ink text painted on top of amber fills.
        assert_eq!(p.brand_primary, theme::BRAND_500);
        assert_eq!(p.focus_ring, theme::BRAND_500);
        assert_eq!(p.on_brand, egui::Color32::from_rgb(0x0c, 0x0c, 0x10));
        assert_eq!(p.success, theme::STATE_SUCCESS);
        assert_eq!(p.warning, theme::STATE_WARNING);
        assert_eq!(p.error, theme::STATE_ERROR);
        assert_eq!(p.info, theme::STATE_INFO);
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

        // Surfaces invert: worked example first (#0c0c10 → #f3f3ef), then the
        // rule for the remaining ramp.
        assert_eq!(light.background, egui::Color32::from_rgb(0xf3, 0xf3, 0xef));
        assert_eq!(light.surface, mirror(dark.surface));
        assert_eq!(light.surface_2, mirror(dark.surface_2));
        assert_eq!(light.surface_3, mirror(dark.surface_3));
        // The ramp order flips: dark bg is the darkest step, light bg the
        // lightest.
        assert!(
            lum(dark.background) < lum(dark.surface)
                && lum(dark.surface) < lum(dark.surface_2)
                && lum(dark.surface_2) < lum(dark.surface_3)
        );
        assert!(
            lum(light.background) > lum(light.surface)
                && lum(light.surface) > lum(light.surface_2)
                && lum(light.surface_2) > lum(light.surface_3)
        );

        // Ink flips while preserving the faintness hierarchy: on dark the
        // primary ink is brightest; on light it is darkest.
        assert_eq!(light.ink, egui::Color32::from_rgb(0x0b, 0x0b, 0x0a));
        assert_eq!(light.ink_2, mirror(dark.ink_2));
        assert_eq!(light.ink_3, mirror(dark.ink_3));
        assert!(lum(dark.ink) > lum(dark.ink_2) && lum(dark.ink_2) > lum(dark.ink_3));
        assert!(lum(light.ink) < lum(light.ink_2) && lum(light.ink_2) < lum(light.ink_3));

        // Lines flip their base white→black but keep the same alphas.
        assert_eq!((light.line.r(), light.line.g(), light.line.b()), (0, 0, 0));
        assert_eq!(light.line.a(), dark.line.a());
        assert_eq!(light.border.a(), dark.border.a());

        // Brand amber and status colors are identical across palettes.
        assert_eq!(light.brand_primary, dark.brand_primary);
        assert_eq!(light.brand_primary, theme::BRAND_500);
        assert_eq!(light.success, dark.success);
        assert_eq!(light.warning, dark.warning);
        assert_eq!(light.error, dark.error);
        assert_eq!(light.info, dark.info);
        // Dark text stays correct on the unchanged amber fill.
        assert_eq!(light.on_brand, dark.on_brand);
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
        // (4/8/12 px transcribed from --riff-radius-*).
        assert_eq!(
            v.widgets.inactive.corner_radius,
            egui::CornerRadius::same(4)
        );
        assert_eq!(v.menu_corner_radius, egui::CornerRadius::same(8));
        assert_eq!(v.window_corner_radius, egui::CornerRadius::same(12));

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
    fn test_window_controls_minimize_and_route_close_through_the_vetoable_path() {
        use riff_gui::ui::chrome::WindowControl;

        // Minimize collapses the window.
        assert_eq!(
            WindowControl::Minimize.viewport_command(),
            egui::ViewportCommand::Minimized(true)
        );
        // Close must go through ViewportCommand::Close — the same path as the
        // OS close button — so close-to-tray (REQ-SI-001) keeps vetoing it on
        // macOS/Windows. A hard exit here would silently kill playback.
        assert_eq!(
            WindowControl::Close.viewport_command(),
            egui::ViewportCommand::Close
        );
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

    // --- Hardcoded-color sweep (Issue 03) --------------------------------------
    //
    // ADR 0004: every color in view code must come from the active palette's
    // tokens, never from a flat literal. The sweep is mechanical, so its
    // "grep-verifiable" acceptance is encoded here as a permanent regression
    // guard: the UI layer's source is scanned for hardcoded egui color
    // constructors and named constants, keeping this ticket and every later
    // restyle ticket (07–12) token-pure by construction.

    /// True when a code line constructs an egui color from scratch: any
    /// `Color32`/`Rgba` `from_*` constructor or associated constant. Values
    /// already derived from tokens never appear in the `Type::` path form.
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
        false
    }

    /// Collect `path:line` pairs where UI-layer source hardcodes an egui
    /// color. `theme.rs` is exempt: it is the sanctioned home of the token
    /// literals themselves. Comment lines are skipped so prose may name the
    /// APIs it bans.
    fn hardcoded_color_violations() -> Vec<String> {
        fn scan_dir(dir: &std::path::Path, violations: &mut Vec<String>) {
            let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
                .expect("the src/ui directory must be readable")
                .map(|entry| entry.expect("directory entries must resolve").path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    scan_dir(&path, violations);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    if path.file_name().and_then(|name| name.to_str()) == Some("theme.rs") {
                        continue;
                    }
                    let source =
                        std::fs::read_to_string(&path).expect("source files must be UTF-8");
                    for (idx, line) in source.lines().enumerate() {
                        let trimmed = line.trim_start();
                        if trimmed.starts_with("//") {
                            continue;
                        }
                        if hardcoded_color_literal(trimmed) {
                            violations.push(format!("{}:{}: {}", path.display(), idx + 1, trimmed));
                        }
                    }
                }
            }
        }

        let mut violations = Vec::new();
        scan_dir(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("riff-gui")
                .join("src")
                .join("ui"),
            &mut violations,
        );
        violations
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
        // The chrome-fitting minimum must leave room for the fixed panels
        // PLUS a usable main stage: sidebar + stage across, titlebar +
        // playerbar + stage down. A window below this would collapse the
        // fixed chrome.
        let min = chrome::MIN_WINDOW_SIZE;
        assert!(min.x >= theme::SIDEBAR_W + chrome::MIN_STAGE_SIZE.x);
        assert!(min.y >= theme::TITLEBAR_H + theme::PLAYERBAR_H + chrome::MIN_STAGE_SIZE.y);

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
                    show_titlebar(ui, &mut cache, &palette, &content, &mut widget_actions);
                    actions.append(&mut widget_actions);
                },
                Vec::new(),
            );
        harness.run();

        // Custom window controls keep their issue-04 contract, now surfaced
        // as actions the app applies through the vetoable viewport commands.
        harness.get_by_label("Close").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::Close));

        harness.get_by_label("Minimize").click();
        harness.run();
        assert!(harness.state().contains(&TitleBarAction::Minimize));

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

    // --- Sidebar restyle (Issue 07) --------------------------------------------
    //
    // The sidebar matches the mockup: a search box with a focus-ring border,
    // a segmented Library/Folders control, 40px tree rows on the three-level
    // indent scale with hover states, an animated equalizer-bars indicator on
    // the now-playing row, styled smart playlists ×4, and playlist rows whose
    // hover-revealed edit/delete drive the EXISTING rename/delete Store flows
    // (ADR 0002 projection refresh). Restyle only — behavior untouched.
    //
    // The widgets live behind headless seams in `riff_gui::ui::sidebar`; the
    // pixels are pinned by the `sidebar_dark` golden image.

    use riff_gui::ui::sidebar;

    #[test]
    fn test_sidebar_tree_rows_use_the_mockup_40px_height_and_indent_scale() {
        // Mockup: tree rows are exactly 40px tall...
        assert!((sidebar::ROW_H - 40.0).abs() < f32::EPSILON);
        // ...on the three-level indent scale 12/44/80px.
        assert!((sidebar::indent_px(0) - 12.0).abs() < f32::EPSILON);
        assert!((sidebar::indent_px(1) - 44.0).abs() < f32::EPSILON);
        assert!((sidebar::indent_px(2) - 80.0).abs() < f32::EPSILON);
        // Deeper nesting keeps stepping so deep trees never fold into one edge.
        assert!(sidebar::indent_px(3) > sidebar::indent_px(2));
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
    fn test_segmented_control_reports_the_clicked_destination() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 40.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, clicks: &mut Vec<sidebar::SidebarNav>| {
                    // ACCUMULATE across frames: a click fires its action on
                    // exactly one frame; harness.run() settles afterwards.
                    if let Some(dest) =
                        sidebar::segmented_nav(ui, &palette, Some(sidebar::SidebarNav::Library))
                    {
                        clicks.push(dest);
                    }
                },
                Vec::new(),
            );
        harness.run();

        harness.get_by_label("Folders").click();
        harness.run();
        assert_eq!(
            harness.state(),
            &vec![sidebar::SidebarNav::Folders],
            "clicking the Folders segment reports the Folders destination"
        );

        harness.get_by_label("Library").click();
        harness.run();
        assert!(
            harness.state().contains(&sidebar::SidebarNav::Library),
            "clicking the Library segment reports the Library destination"
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
                    let response = sidebar::tree_row(
                        ui,
                        &mut cache,
                        &palette,
                        sidebar::TreeRow {
                            indent_level: 1,
                            icon: Some(icons::Icon::Music),
                            label: "All Tracks",
                            selected: false,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
                        },
                    );
                    if response.clicked() {
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

    #[test]
    fn test_search_box_clear_button_clears_the_query() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(theme::SIDEBAR_W - 24.0, 40.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                |ui, query: &mut String| {
                    sidebar::search_box(ui, &mut cache, &palette, query);
                },
                "beethoven".to_string(),
            );
        harness.run();

        harness.get_by_label("Clear search").click();
        harness.run();
        assert_eq!(
            harness.state().as_str(),
            "",
            "the clear affordance empties the search query"
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
    // Headless seams (`riff_gui::ui::playerbar`): the mockup dimension tokens,
    // the monospace readout font, the seek-fraction math, and the
    // control→action contract. The pixels are pinned by the `playerbar_dark`
    // golden image; the action→command wiring is covered further below.

    use riff_gui::ui::playerbar;

    #[test]
    fn test_playerbar_dimensions_match_the_mockup() {
        // Mockup: a 56×56 cover...
        assert!((playerbar::COVER - 56.0).abs() < f32::EPSILON);
        // ...a 40px primary-filled play button among circular ghost
        // transport buttons...
        assert!((playerbar::PLAY_BTN - 40.0).abs() < f32::EPSILON);
        assert!((playerbar::GHOST_BTN - 32.0).abs() < f32::EPSILON);
        // ...and 4px tracks for both the seek row and the volume slider.
        assert!((playerbar::TRACK_H - 4.0).abs() < f32::EPSILON);
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
    /// into a 245s track, mid volume, shuffle on, nothing muted.
    fn playing_content() -> playerbar::PlayerBarContent<'static> {
        playerbar::PlayerBarContent {
            cover: None,
            playback: PlaybackState::Playing,
            position: std::time::Duration::from_mins(2),
            total: Some(std::time::Duration::from_secs(245)),
            volume: 0.65,
            muted: false,
            shuffle: true,
            repeat: RepeatMode::None,
            queue_position: "3/12",
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
    /// recording transport + mock store, returning all four for inspection.
    /// The two session values are the type the [`apply_player_bar_action`]
    /// function takes after the two-session split.
    #[allow(clippy::type_complexity)]
    fn applied(
        action: PlayerBarAction,
    ) -> (
        PlaybackSession,
        LibrarySession,
        crate::mocks::MockTransport,
        crate::mocks::MockSettingsStore,
    ) {
        let mut playback = PlaybackSession::default();
        let mut library = LibrarySession::default();
        let transport = crate::mocks::MockTransport::new();
        let mut store = crate::mocks::MockSettingsStore::default();
        apply_player_bar_action(action, &mut library, &mut playback, &transport, &mut store);
        (playback, library, transport, store)
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
            let (_, _, transport, _) = applied(action);
            assert_eq!(
                transport.recorded(),
                vec![expected.clone()],
                "{expected:?} must still be issued to the engine"
            );
        }
    }

    #[test]
    fn test_play_selected_plays_the_selected_track() {
        let (mut playback, mut library, transport, mut store) =
            applied(PlayerBarAction::PlaySelected); // no selection yet
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
            &mut store,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::Play(TrackId("song.flac".to_string()))]
        );
    }
    #[test]
    fn test_seek_action_is_clamped_against_the_live_total() {
        let (mut playback, _, _, mut store) = applied(PlayerBarAction::Pause);
        playback.current_position.total = Some(std::time::Duration::from_secs(245));

        let transport = crate::mocks::MockTransport::new();
        apply_player_bar_action(
            PlayerBarAction::Seek(std::time::Duration::from_secs_f32(999.0)),
            &mut LibrarySession::default(),
            &mut playback,
            &transport,
            &mut store,
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::Seek(std::time::Duration::from_secs(245))],
            "an out-of-range seek target clamps to the track duration"
        );
    }

    #[test]
    fn test_volume_action_updates_state_persists_and_sends_effective_volume() {
        let (playback, _, transport, store) = applied(PlayerBarAction::SetVolume(0.7));

        assert!(
            (playback.current_volume - 0.7).abs() < 1e-6,
            "slider value lands"
        );
        assert_eq!(
            transport.recorded(),
            vec![TransportIntent::ApplyVolume(0.7)],
            "the engine hears the new volume"
        );
        assert!(
            store.calls.contains(&crate::mocks::SettingsCall::Scalars),
            "volume changes persist through the settings store"
        );
    }

    #[test]
    fn test_mute_toggle_sends_zero_and_keeps_slider_value() {
        let (mut playback, _, _, mut store) = applied(PlayerBarAction::SetVolume(0.7));
        let transport = crate::mocks::MockTransport::new();
        let mut library = LibrarySession::default();

        // Muting sends the muted (zero) volume to the engine...
        apply_player_bar_action(
            PlayerBarAction::ToggleMute,
            &mut library,
            &mut playback,
            &transport,
            &mut store,
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
            &mut store,
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
            &mut store,
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
        let (mut playback, _, _, mut store) = applied(PlayerBarAction::Pause);
        let transport = crate::mocks::MockTransport::new();
        let mut library = LibrarySession::default();

        let was = playback.queue.shuffle;
        apply_player_bar_action(
            PlayerBarAction::ToggleShuffle,
            &mut library,
            &mut playback,
            &transport,
            &mut store,
        );
        assert_ne!(playback.queue.shuffle, was, "shuffle flips");

        assert_eq!(playback.queue.repeat, RepeatMode::None);
        apply_player_bar_action(
            PlayerBarAction::ToggleRepeat,
            &mut library,
            &mut playback,
            &transport,
            &mut store,
        );
        assert_eq!(
            playback.queue.repeat,
            RepeatMode::All,
            "repeat cycles off → all"
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
        // w-40 h-40 disc circle with an 80px (w-20 h-20) glyph.
        assert!((library::HERO_DISC_SIZE - 160.0).abs() < f32::EPSILON);
        assert!((library::HERO_DISC_ICON_SIZE - 80.0).abs() < f32::EPSILON);
        // mb-6 below the circle, mb-1 between title and subtitle, p-8 inset.
        assert!((library::HERO_TITLE_GAP - 24.0).abs() < f32::EPSILON);
        assert!((library::HERO_SUBTITLE_GAP - 4.0).abs() < f32::EPSILON);
        assert!((library::HERO_STAGE_INSET - 32.0).abs() < f32::EPSILON);
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
        let layers = library::GLOW_LAYERS;
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
        // the palette's brand primary scaled by the layer's alpha fraction.
        let palette = riff_gui::ui::theme::Palette::dark();
        for layer in &library::GLOW_LAYERS {
            assert_eq!(
                library::glow_color(&palette, *layer),
                riff_gui::ui::theme::BRAND_500.gamma_multiply(layer.alpha),
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
            (now_playing::COVER_SIZE - 240.0).abs() < f32::EPSILON,
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
        // w-9 h-5 pill with a w-4 h-4 knob inset by 0.5 (2px).
        assert!((toggle_switch::TOGGLE_W - 36.0).abs() < f32::EPSILON);
        assert!((toggle_switch::TOGGLE_H - 20.0).abs() < f32::EPSILON);
        assert!((toggle_switch::KNOB_SIZE - 16.0).abs() < f32::EPSILON);
        assert!((toggle_switch::KNOB_INSET - 2.0).abs() < f32::EPSILON);
        // peer-checked:translate-x-4 — the knob slides exactly 16px.
        assert!((toggle_switch::KNOB_TRAVEL - 16.0).abs() < f32::EPSILON);
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
        assert_eq!(settings::SECTION_PREFERENCES, "PREFERENCES");
        assert_eq!(settings::SECTION_ADVANCED_INFO, "ADVANCED & PLATFORM INFO");
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
        assert_eq!(
            settings::CLEAR_LIBRARY_NOTE,
            "Clear the indexed collection and rebuild it on the next scan."
        );
        // Ghost styling: transparent until hover, then destructive @ 10%.
        let dark = theme::Palette::dark();
        assert_eq!(
            settings::destructive_ghost_fill(&dark, false),
            egui::Color32::TRANSPARENT
        );
        assert_eq!(
            settings::destructive_ghost_fill(&dark, true),
            dark.error.gamma_multiply(0.1)
        );
    }

    /// Render the restyled stage headlessly with one representative library
    /// row, collecting reported actions like the Now Playing harness does.
    fn settings_harness(
        content: &SettingsContent,
    ) -> egui_kittest::Harness<'_, Vec<SettingsAction>> {
        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(672.0, 720.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, actions: &mut Vec<SettingsAction>| {
                    actions.extend(settings::show_settings_stage(
                        ui, &mut cache, &palette, content,
                    ));
                },
                Vec::new(),
            )
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
        }
    }

    #[test]
    fn test_toggle_switch_click_reports_the_preference_action() {
        use egui_kittest::kittest::Queryable;

        let content = sample_content();
        let mut harness = settings_harness(&content);
        harness.run();

        // Advanced mode starts ON (mockup shows it checked); clicking the
        // switch must report turning it OFF.
        harness.get_by_label("Advanced mode").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetAdvanced(false)),
            "the reusable ToggleSwitch drives the Advanced mode preference"
        );

        // ReplayGain starts OFF; its switch reports turning it ON.
        harness.get_by_label("ReplayGain").click();
        harness.run();
        assert!(
            harness
                .state()
                .contains(&SettingsAction::SetReplayGain(true)),
            "the same widget drives the ReplayGain preference"
        );
    }

    #[test]
    fn test_back_button_and_library_actions_report_actions() {
        use egui_kittest::kittest::Queryable;

        let content = sample_content();
        let mut harness = settings_harness(&content);
        harness.run();

        harness.get_by_label("Back to Library").click();
        harness.run();
        assert!(
            harness.state().contains(&SettingsAction::Back),
            "the top bar's back button leaves the Settings View"
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

    /// Harness state for
    /// [`test_playlist_reorder_render_reflects_new_order_without_explicit_invalidation`]:
    /// the production seam/store pairing plus the row ids in the order the
    /// LAST frame rendered them.
    struct ReorderRenderState {
        views: riff_backend::app::views::SessionViews,
        store: riff_backend::infra::store::SqliteStore,
        pid: PlaylistId,
        rendered: Vec<String>,
        cache: icons::IconCache,
    }

    /// The production playlist-view data path, replicated frame-for-frame:
    /// rows come from `views.playlist_view` as `Arc` clones, each valid
    /// entry renders through [`sidebar::reorderable_row`], and a drop
    /// commits through the store port — nothing else.
    fn render_reorder_state_ui(ui: &mut egui::Ui, s: &mut ReorderRenderState) {
        let palette = theme::Palette::dark();
        // Exactly `render_playlist_view`'s read: ready-to-render rows from
        // the seam, Arc'd out before any widget call.
        let Some(view) = s.views.playlist_view(&s.pid) else {
            s.rendered.clear();
            return;
        };
        s.rendered = view
            .rows
            .iter()
            .map(|(id, _, _)| {
                PathBuf::from(&id.0)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(&id.0)
                    .to_string()
            })
            .collect();

        for (index, (_tid, track, valid)) in view.rows.iter().enumerate() {
            if !*valid {
                continue;
            }
            let Some(track) = track else { continue };
            let label = format!(
                "Artist - {}",
                track.metadata.display_title(&track.file_path)
            );
            let outcome = sidebar::reorderable_row(
                ui,
                &mut s.cache,
                &palette,
                egui::Id::new(("riff_stale_fixture", index)),
                index,
                sidebar::TreeRow {
                    indent_level: 0,
                    icon: None,
                    label: &label,
                    selected: false,
                    now_playing: false,
                    playing: false,
                    disclosure: None,
                },
            );
            if let Some(from) = outcome.drop_from {
                // The UI action path: commit and nothing else — no reload,
                // no cache clear, no patch.
                riff_gui::ui::app::commit_playlist_reorder(
                    &mut s.views,
                    &mut s.store,
                    &s.pid,
                    from,
                    index,
                );
            }
        }
    }

    /// The core Phase 3 property, pinned at the render level: a drag-reorder
    /// committed through the UI action path (`commit_playlist_reorder`, the
    /// same call `render_reorderable_playlist_row` makes on a drop) is
    /// reflected by the NEXT rendered frame's rows with NO explicit
    /// invalidation, cache clear, or reload anywhere in the flow — the
    /// committed mutation bumps the playlist generation and the seam's
    /// projection refetches on its own (ADR 0002).
    #[test]
    fn test_playlist_reorder_render_reflects_new_order_without_explicit_invalidation() {
        use egui_kittest::kittest::Queryable;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("opening a fresh store must work");

        // Three real audio files indexed into the Library, so every entry
        // resolves valid and renders as a reorderable row.
        let mut track_ids = Vec::new();
        for (file, title) in [
            ("one.mp3", "Alpha"),
            ("two.mp3", "Beta"),
            ("three.mp3", "Gamma"),
        ] {
            let path = dir.path().join(file);
            std::fs::write(&path, b"fake audio bytes").expect("scratch file writes");
            let track = crate::test_utils::create_test_track_with_metadata(
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

        let mut store_for_views = store.clone();
        let pid = store_for_views
            .create_playlist("Gym", &track_ids)
            .expect("create works");
        let mut views = riff_backend::app::views::SessionViews::new(
            Box::new(store.clone()),
            Box::new(store_for_views),
            store.library_generation(),
            store.playlist_generation(),
        );

        // Warm the projection the way an open playlist view would, then hand
        // the SAME seam instance to the harness — one instance across the
        // commit boundary is what makes staleness observable.
        assert_eq!(views.playlists().len(), 1);
        assert_eq!(views.playlist_view(&pid).expect("view").rows.len(), 3);

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(320.0, sidebar::ROW_H * 3.0))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                render_reorder_state_ui,
                ReorderRenderState {
                    views,
                    store,
                    pid,
                    rendered: Vec::new(),
                    cache: icons::IconCache::new(),
                },
            );
        harness.run();
        assert_eq!(
            harness.state().rendered,
            vec!["one", "two", "three"],
            "the first frame renders the seeded order"
        );

        // Drag row 0 ("one") onto row 2 ("three"): press, move, release.
        let src = harness.get_by_label("Artist - Alpha").rect();
        let dst = harness.get_by_label("Artist - Gamma").rect();
        harness.drag_at(src.center());
        harness.run();
        harness.hover_at(dst.center());
        harness.run();
        harness.drop_at(dst.center());
        harness.run();

        // The next rendered frame reflects the committed reorder — with no
        // explicit invalidation call anywhere in the flow.
        assert_eq!(
            harness.state().rendered,
            vec!["two", "three", "one"],
            "rendered rows reflect the committed reorder without explicit invalidation"
        );
        // The store committed the same order.
        assert_eq!(
            harness.state().store.load_playlists().unwrap()[0].tracks,
            vec![
                track_ids[1].clone(),
                track_ids[2].clone(),
                track_ids[0].clone()
            ],
            "the drag persisted through the PlaylistStore"
        );
    }

    #[test]
    fn test_playlist_entry_rows_drag_reorder_reports_the_move() {
        use egui_kittest::kittest::Queryable;

        let palette = theme::Palette::dark();
        let mut cache = icons::IconCache::new();
        let labels = ["Alpha", "Beta", "Gamma"];
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(256.0, sidebar::ROW_H * 3.0))
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
                                label,
                                selected: false,
                                now_playing: false,
                                playing: false,
                                disclosure: None,
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
            .with_size(egui::vec2(256.0, sidebar::ROW_H * 2.0))
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
                            label: "Beta",
                            selected: false,
                            now_playing: false,
                            playing: false,
                            disclosure: None,
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
            .with_size(egui::vec2(64.0, sidebar::ROW_H))
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
        let view_h = VIEW_ROWS as f32 * sidebar::ROW_H;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(280.0, view_h))
            .with_pixels_per_point(1.0)
            .build_ui_state(
                move |ui, _seen: &mut Vec<()>| {
                    let mut rendered_this_frame = 0usize;
                    egui::ScrollArea::vertical()
                        .id_salt("virtualization_fixture")
                        .auto_shrink(false)
                        .show_rows(ui, sidebar::ROW_H, TOTAL_ROWS, |ui, range| {
                            for i in range {
                                rendered_this_frame += 1;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), sidebar::ROW_H),
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
                        });
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

// --- Background service seams in the UI ----------------------------------------
//
// The UI no longer owns worker threads or channel protocols (ADR 0006): it
// submits intent and polls outcomes through the boxed `TagEdits`/`Covers`
// handles. These tests drive the exact production code paths — the free
// functions `submit_tag_edit_fields`, `apply_tag_edit_outcome`,
// `request_cover_intent`, and `cache_polled_covers` that the RiffApp
// methods delegate to — over recording fakes, with no threads and no disk
// I/O.
#[cfg(test)]
mod background_service_ui_tests {
    use super::*;
    use riff_backend::app::cover_service::Covers;
    use riff_backend::app::tag_edit_service::{TagEditOutcome, TagEditRequest, TagEdits};
    use riff_gui::ui::app::{
        apply_tag_edit_outcome, cache_polled_covers, request_cover_intent, submit_tag_edit_fields,
    };
    use riff_library::app::traits::CoverImage;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Recording [`TagEdits`] fake: captures every submitted request and
    /// never yields outcomes (outcomes are injected through
    /// [`apply_tag_edit_outcome`] directly).
    struct RecordingTagEdits {
        submitted: Mutex<Vec<TagEditRequest>>,
    }

    impl RecordingTagEdits {
        fn new() -> Self {
            Self {
                submitted: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<TagEditRequest> {
            self.submitted.lock().unwrap().clone()
        }
    }

    impl TagEdits for RecordingTagEdits {
        fn submit(&self, request: TagEditRequest) {
            self.submitted.lock().unwrap().push(request);
        }

        fn poll(&self) -> Option<TagEditOutcome> {
            None
        }
    }

    /// Recording [`Covers`] fake: counts request intent, serves nothing.
    struct RecordingCovers {
        requested: Mutex<Vec<(TrackId, PathBuf)>>,
    }

    impl RecordingCovers {
        fn new() -> Self {
            Self {
                requested: Mutex::new(Vec::new()),
            }
        }

        fn requested(&self) -> Vec<(TrackId, PathBuf)> {
            self.requested.lock().unwrap().clone()
        }
    }

    impl Covers for RecordingCovers {
        fn request(&self, track_id: TrackId, path: PathBuf) {
            self.requested.lock().unwrap().push((track_id, path));
        }

        fn poll(&self) -> Vec<(TrackId, Option<CoverImage>)> {
            Vec::new()
        }
    }

    /// Canned [`Covers`] fake whose single poll drains scripted results.
    struct CannedCovers(Vec<(TrackId, Option<CoverImage>)>);

    impl Covers for CannedCovers {
        fn request(&self, _track_id: TrackId, _path: PathBuf) {}

        fn poll(&self) -> Vec<(TrackId, Option<CoverImage>)> {
            self.0.clone()
        }
    }

    /// An open "Edit Tags" modal mid-save for `/music/t1.mp3`.
    fn saving_modal() -> TagEditState {
        TagEditState {
            track_id: TrackId("/music/t1.mp3".to_string()),
            path: PathBuf::from("/music/t1.mp3"),
            title: "Old Title".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            album_artist: "Album Artist".to_string(),
            genre: "Genre".to_string(),
            year: "1999".to_string(),
            track_number: "7".to_string(),
            error: None,
            saving: true,
        }
    }

    #[test]
    fn test_saved_outcome_closes_dialog_and_sets_status_line() {
        let mut modal = Some(saving_modal());
        let mut in_flight = Some((
            TrackId("/music/t1.mp3".to_string()),
            PathBuf::from("/music/t1.mp3"),
        ));
        let mut status = None;

        apply_tag_edit_outcome(
            TagEditOutcome::Saved,
            &mut modal,
            &mut in_flight,
            &mut status,
        );

        assert!(modal.is_none(), "a saved edit closes the dialog");
        assert_eq!(
            status.as_deref(),
            Some("Tags saved for t1.mp3"),
            "the status line names the saved file"
        );
        assert!(in_flight.is_none(), "the outstanding record is consumed");
    }

    #[test]
    fn test_failed_outcome_keeps_dialog_open_with_reason() {
        let mut modal = Some(saving_modal());
        let mut in_flight = Some((
            TrackId("/music/t1.mp3".to_string()),
            PathBuf::from("/music/t1.mp3"),
        ));
        let mut status = Some("earlier message".to_string());

        apply_tag_edit_outcome(
            TagEditOutcome::Failed {
                reason: "permission denied".to_string(),
            },
            &mut modal,
            &mut in_flight,
            &mut status,
        );

        let modal = modal.expect("a failed edit keeps the dialog open");
        assert_eq!(modal.error.as_deref(), Some("permission denied"));
        assert!(!modal.saving, "the save spinner stops");
        // No silent success: the previous status line is left untouched.
        assert_eq!(status.as_deref(), Some("earlier message"));
    }

    #[test]
    fn test_outcome_for_another_track_leaves_modal_untouched() {
        // The user opened a different track's editor while a save was in
        // flight: its outcome must not close or alter the new dialog.
        let mut modal = Some(saving_modal());
        let mut in_flight = Some((
            TrackId("/music/other.mp3".to_string()),
            PathBuf::from("/music/other.mp3"),
        ));
        let mut status = None;

        apply_tag_edit_outcome(
            TagEditOutcome::Failed {
                reason: "stale failure".to_string(),
            },
            &mut modal,
            &mut in_flight,
            &mut status,
        );

        let untouched = modal
            .as_ref()
            .expect("an unrelated outcome leaves the dialog open");
        assert!(untouched.error.is_none());
        assert!(untouched.saving);

        apply_tag_edit_outcome(
            TagEditOutcome::Saved,
            &mut modal,
            &mut in_flight,
            &mut status,
        );
        assert!(
            modal.is_some(),
            "an unrelated save must not close the open dialog"
        );
    }

    #[test]
    fn test_outcome_without_outstanding_request_is_ignored() {
        let mut modal = Some(saving_modal());
        let mut in_flight = None;
        let mut status = None;

        apply_tag_edit_outcome(
            TagEditOutcome::Saved,
            &mut modal,
            &mut in_flight,
            &mut status,
        );

        assert!(
            modal.is_some(),
            "nothing outstanding means nothing to close"
        );
        assert!(status.is_none());
    }

    #[test]
    fn test_parse_invalid_submit_keeps_dialog_open_without_submitting() {
        let mut modal = saving_modal();
        modal.year = "not a number".to_string();
        modal.saving = false;
        let edits = RecordingTagEdits::new();
        let mut in_flight = None;

        submit_tag_edit_fields(&mut modal, &edits, &mut in_flight);

        assert!(
            edits.requests().is_empty(),
            "invalid fields must not reach the service"
        );
        assert!(modal.error.is_some(), "the parse error surfaces");
        assert!(!modal.saving, "the modal stays open, not saving");
        assert!(in_flight.is_none());
    }

    #[test]
    fn test_valid_submit_sends_request_and_marks_saving() {
        let mut modal = saving_modal();
        modal.year = "2001".to_string();
        modal.track_number = String::new(); // empty means "leave unset"
        let edits = RecordingTagEdits::new();
        let mut in_flight = None;

        submit_tag_edit_fields(&mut modal, &edits, &mut in_flight);

        let requests = edits.requests();
        assert_eq!(requests.len(), 1, "exactly one request submitted");
        let request = &requests[0];
        assert_eq!(request.track_id.0, "/music/t1.mp3");
        assert_eq!(request.path, PathBuf::from("/music/t1.mp3"));
        assert_eq!(
            request.edit.title.as_deref(),
            Some("Old Title"),
            "the edited field values travel with the request"
        );
        assert_eq!(request.edit.year, Some(2001));
        assert_eq!(request.edit.track_number, None);
        assert!(modal.saving, "the modal flips into its saving state");
        assert!(modal.error.is_none());
        assert_eq!(
            in_flight,
            Some((
                TrackId("/music/t1.mp3".to_string()),
                PathBuf::from("/music/t1.mp3")
            )),
            "the outstanding record enables outcome matching"
        );
    }

    #[test]
    fn test_cover_intent_skips_cached_texture_but_requests_uncached() {
        let covers = RecordingCovers::new();
        let id = TrackId("/music/t1.mp3".to_string());
        let path = PathBuf::from("/music/t1.mp3");

        request_cover_intent(true, &covers, id.clone(), path.clone());
        assert!(
            covers.requested().is_empty(),
            "a cached texture suppresses the request"
        );

        request_cover_intent(false, &covers, id.clone(), path.clone());
        assert_eq!(
            covers.requested(),
            vec![(id.clone(), path)],
            "an uncached track sends intent to the service"
        );
    }

    #[test]
    fn test_cache_polled_covers_inserts_textures_and_skips_artless() {
        let ctx = egui::Context::default();
        // A real 2x2 PNG: cache_polled_covers decodes the port's encoded
        // bytes on the UI thread before building the texture.
        let png: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
            image::ImageBuffer::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]));
        let mut png_bytes = std::io::Cursor::new(Vec::new());
        png.write_to(&mut png_bytes, image::ImageFormat::Png)
            .unwrap();
        let image = CoverImage {
            data: png_bytes.into_inner(),
            format: image::ImageFormat::Png,
        };
        let covers = CannedCovers(vec![
            (
                TrackId("/music/art.mp3".to_string()),
                Some(CoverImage {
                    data: image.data.clone(),
                    format: image.format,
                }),
            ),
            (TrackId("/music/artless.mp3".to_string()), None),
        ]);
        let mut textures = std::collections::HashMap::new();
        let mut lru_keys = Vec::new();

        cache_polled_covers(&covers, &mut textures, &mut lru_keys, &ctx);

        assert!(
            textures.contains_key("/music/art.mp3"),
            "resolved art becomes a texture keyed by track id"
        );
        assert!(
            !textures.contains_key("/music/artless.mp3"),
            "artless results create no texture (the service negative-caches them)"
        );
        assert_eq!(lru_keys, vec!["/music/art.mp3".to_string()]);
    }
}

/// Issue 01 seam fix: playback errors no longer write the library session's
/// scan-status slot from the coordinator. They arrive as typed notices with
/// playback source, and the UI routes them to the titlebar status line via
/// [`apply_backend_events`], preserving the exact visible string.
#[cfg(test)]
mod playback_notice_ui_tests {
    use riff_backend::app::facade::{BackendEvent, NoticePayload, NoticeSeverity, NoticeSource};
    use riff_gui::ui::app::apply_backend_events;

    fn playback_notice(message: &str) -> BackendEvent {
        BackendEvent::TypedNotice(NoticePayload {
            severity: NoticeSeverity::Error,
            source: NoticeSource::Playback,
            message: message.to_string(),
        })
    }

    #[test]
    fn test_playback_typed_notice_routes_message_to_status_line() {
        let mut status: Option<String> = None;

        apply_backend_events(vec![playback_notice("Playback error: boom")], &mut status);

        assert_eq!(
            status.as_deref(),
            Some("Playback error: boom"),
            "the exact user-facing string reaches the status line"
        );
    }

    #[test]
    fn test_non_playback_notice_leaves_status_line_untouched() {
        let mut status: Option<String> = Some("existing".to_string());

        apply_backend_events(
            vec![BackendEvent::TypedNotice(NoticePayload {
                severity: NoticeSeverity::Error,
                source: NoticeSource::Scan,
                message: "scan failed".to_string(),
            })],
            &mut status,
        );

        assert_eq!(
            status.as_deref(),
            Some("existing"),
            "only playback-sourced notices write the status line here"
        );
    }

    #[test]
    fn test_empty_event_batch_leaves_status_line_untouched() {
        let mut status: Option<String> = Some("existing".to_string());

        apply_backend_events(Vec::new(), &mut status);

        assert_eq!(status.as_deref(), Some("existing"));
    }
}
