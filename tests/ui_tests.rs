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

    use riff::app::store::{LibraryMutationStore, LibraryQueryStore, PlaylistStore, SettingsStore};
    use std::path::PathBuf;

    /// Open a real store-backed settings port at a fresh temp location,
    /// exactly as the UI receives it: a boxed `SettingsStore`.
    fn boxed_store(dir: &tempfile::TempDir) -> Box<dyn SettingsStore> {
        let db_path = dir.path().join("riff.sqlite3");
        Box::new(
            riff::infra::store::SqliteStore::open_and_migrate(&db_path)
                .expect("opening a fresh store must work"),
        )
    }

    /// Open a real store-backed playlists port at a fresh temp location,
    /// exactly as the UI receives it: a boxed `PlaylistStore`.
    fn boxed_playlist_store(dir: &tempfile::TempDir) -> Box<dyn PlaylistStore> {
        let db_path = dir.path().join("riff.sqlite3");
        Box::new(
            riff::infra::store::SqliteStore::open_and_migrate(&db_path)
                .expect("opening a fresh store must work"),
        )
    }

    /// Open a real store-backed library query port at a fresh temp location,
    /// exactly as the UI receives it: a boxed `LibraryQueryStore`.
    fn boxed_library_query_store(dir: &tempfile::TempDir) -> Box<dyn LibraryQueryStore> {
        let db_path = dir.path().join("riff.sqlite3");
        Box::new(
            riff::infra::store::SqliteStore::open_and_migrate(&db_path)
                .expect("opening a fresh store must work"),
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
        assert!(boxed_playlist_store(&dir)
            .load_playlists()
            .unwrap()
            .is_empty());

        // Create + edit, then drop the connection (the "restart").
        let pid;
        {
            let mut store = boxed_playlist_store(&dir);
            pid = store.create_playlist("Gym", &[]).unwrap();
            assert!(store
                .add_playlist_entry(&pid, &TrackId("hype.mp3".to_string()))
                .unwrap());
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
        assert!(boxed_playlist_store(&dir)
            .load_playlists()
            .unwrap()
            .is_empty());
    }

    // --- Library collection cutover (ticket 05) --------------------------------
    //
    // The library hydrates from the Application Store through the
    // `LibraryQueryStore` port on startup; the legacy JSON cache is never
    // read or written and stays untouched on disk.

    /// Seed a compilation album into the store at `dir`: one album credited
    /// to "Various Artists" with two track-level artists.
    fn seed_compilation(dir: &tempfile::TempDir) {
        let mut store =
            riff::infra::store::SqliteStore::open_and_migrate(&dir.path().join("riff.sqlite3"))
                .expect("opening the store must work");
        let make_track = |path: &str, artist: &str| Track {
            id: TrackId(path.to_string()),
            file_path: PathBuf::from(path),
            metadata: crate::domain::TrackMetadata {
                title: Some("Song".to_string()),
                artist: Some(artist.to_string()),
                album: Some("Comp".to_string()),
                album_artist: Some("Various Artists".to_string()),
                ..crate::domain::TrackMetadata::default()
            },
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
        };
        store
            .apply_scan_batch(&[
                make_track("m:\\comp\\01.mp3", "Artist A"),
                make_track("m:\\comp\\02.mp3", "Artist B"),
            ])
            .expect("seeding the collection must work");
    }

    #[test]
    fn test_startup_hydrates_the_mirror_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        seed_compilation(&dir);

        let mut state = AppState::new();
        riff::ui::app::load_persisted_state(
            &mut state,
            boxed_store(&dir).as_ref(),
            boxed_playlist_store(&dir).as_ref(),
            boxed_library_query_store(&dir).as_ref(),
            None,
        );

        assert_eq!(
            state.library.all_tracks().len(),
            2,
            "both seeded tracks hydrate into the mirror"
        );
        assert!(
            state.library.artists.contains_key("Various Artists"),
            "the album-artist grouping hydrates"
        );
        assert!(
            state.library.albums.contains_key("Various Artists - Comp"),
            "the album hydrates under its composite key"
        );
    }

    #[test]
    fn test_legacy_json_cache_is_never_read_or_written() {
        let dir = tempfile::tempdir().unwrap();
        // A corrupt legacy cache sits next to the store; hydration must
        // ignore it entirely and come from the store instead.
        let legacy_path = dir.path().join("library_cache.json");
        std::fs::write(&legacy_path, "{{{ corrupt legacy json").unwrap();
        let legacy_bytes_before = std::fs::read(&legacy_path).unwrap();

        seed_compilation(&dir);

        let mut state = AppState::new();
        riff::ui::app::load_persisted_state(
            &mut state,
            boxed_store(&dir).as_ref(),
            boxed_playlist_store(&dir).as_ref(),
            boxed_library_query_store(&dir).as_ref(),
            None,
        );

        assert_eq!(
            state.library.all_tracks().len(),
            2,
            "hydration comes from the store despite the corrupt legacy file"
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
                .save_scalars(&riff::app::state::ScalarSettings {
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
                .save_scalars(&riff::app::state::ScalarSettings {
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
                .save_scalars(&riff::app::state::ScalarSettings {
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
                .save_scalars(&riff::app::state::ScalarSettings {
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
                .save_scalars(&riff::app::state::ScalarSettings {
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
                .save_scalars(&riff::app::state::ScalarSettings {
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
                .save_scalars(&riff::app::state::ScalarSettings {
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

    #[test]
    fn test_high_contrast_visuals_is_dark_with_visible_focus() {
        // REQ-UI-007: the high-contrast theme must be dark and expose a focus
        // outline thicker than egui's default (1.0) so focused widgets stand out.
        let visuals = high_contrast_visuals();
        assert!(visuals.dark_mode);
        assert!(
            visuals.selection.stroke.width > 1.0,
            "focus stroke should be thicker than the 1.0 default, got {}",
            visuals.selection.stroke.width
        );
        // The focused widget (rendered with the `active` widget visuals) also
        // carries a visible border.
        assert!(visuals.widgets.active.bg_stroke.width > 1.0);
    }

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
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        body();
        match original {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
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
}
