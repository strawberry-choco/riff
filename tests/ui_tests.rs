// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_save_library_paths() {
        // Create a mock storage
        let mut storage = MockStorage::new();

        // Test saving and loading library paths
        let paths = vec![
            std::path::PathBuf::from("path1"),
            std::path::PathBuf::from("path2"),
        ];

        save_library_paths(&mut storage, &paths);
        let loaded_paths = load_library_paths(Some(&mut storage));

        assert_eq!(loaded_paths, paths);
    }

    #[test]
    fn test_load_save_volume() {
        let mut storage = MockStorage::new();

        // Test saving and loading volume
        let volume = 0.75;
        save_volume(&mut storage, volume);
        let loaded_volume = load_volume(Some(&mut storage));

        assert_eq!(loaded_volume, Some(volume));
    }

    #[test]
    fn test_load_save_advanced_mode_roundtrip() {
        let mut storage = MockStorage::new();

        // true round-trips
        save_advanced_mode(&mut storage, true);
        assert!(load_advanced_mode(Some(&mut storage)));

        // false round-trips
        save_advanced_mode(&mut storage, false);
        assert!(!load_advanced_mode(Some(&mut storage)));
    }

    #[test]
    fn test_load_advanced_mode_defaults_to_false_when_absent() {
        // Empty storage: nothing stored under either key => default off.
        let mut storage = MockStorage::new();
        assert!(!load_advanced_mode(Some(&mut storage)));

        // No storage at all => default off.
        assert!(!load_advanced_mode(None));
    }

    #[test]
    fn test_load_save_high_contrast_roundtrip() {
        let mut storage = MockStorage::new();

        // true round-trips
        save_high_contrast(&mut storage, true);
        assert!(load_high_contrast(Some(&mut storage)));

        // false round-trips
        save_high_contrast(&mut storage, false);
        assert!(!load_high_contrast(Some(&mut storage)));
    }

    #[test]
    fn test_load_high_contrast_defaults_to_false_when_absent() {
        // Empty storage: nothing stored under either key => default off.
        let mut storage = MockStorage::new();
        assert!(!load_high_contrast(Some(&mut storage)));

        // No storage at all => default off.
        assert!(!load_high_contrast(None));
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

    #[test]
    fn test_load_save_replaygain_roundtrip() {
        let mut storage = MockStorage::new();

        // true round-trips
        save_replaygain(&mut storage, true);
        assert!(load_replaygain(Some(&mut storage)));

        // false round-trips
        save_replaygain(&mut storage, false);
        assert!(!load_replaygain(Some(&mut storage)));
    }

    #[test]
    fn test_load_replaygain_defaults_to_false_when_absent() {
        // Empty storage: nothing stored under either key => default off.
        let mut storage = MockStorage::new();
        assert!(!load_replaygain(Some(&mut storage)));

        // No storage at all => default off.
        assert!(!load_replaygain(None));
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

    #[test]
    fn test_load_save_watch_states() {
        let mut storage = MockStorage::new();

        // Test saving and loading watch states
        let mut states = std::collections::HashMap::new();
        states.insert(std::path::PathBuf::from("path1"), WatchState::Enabled);

        save_watch_states(&mut storage, &states);
        let loaded_states = load_watch_states(Some(&mut storage));

        assert_eq!(loaded_states, states);
    }

    #[test]
    fn test_restore_from_backup_if_corrupted() {
        let mut storage = MockStorage::new();

        // Set up corrupted primary storage
        storage
            .data
            .insert("library_paths".to_string(), "invalid json".to_string());

        // Set up valid backup storage
        let valid_paths =
            serde_json::to_string(&vec!["path1".to_string(), "path2".to_string()]).unwrap();
        storage
            .data
            .insert("library_paths_backup".to_string(), valid_paths);

        // Restore from backup
        restore_from_backup_if_corrupted(&mut storage);

        // Verify the primary storage was restored
        let restored_paths = load_library_paths(Some(&mut storage));
        assert_eq!(
            restored_paths,
            vec![
                std::path::PathBuf::from("path1"),
                std::path::PathBuf::from("path2"),
            ]
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

    // Mock storage for testing.
    //
    // The current `eframe::Storage` trait (egui/eframe 0.34) has exactly three
    // required methods: `get_string`, `set_string` (taking an owned `String`)
    // and `flush`. The older `get_bool`/`set_bool`/`remove` methods no longer
    // exist on the trait.
    struct MockStorage {
        data: std::collections::HashMap<String, String>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                data: std::collections::HashMap::new(),
            }
        }
    }

    impl eframe::Storage for MockStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.data.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.data.insert(key.to_string(), value);
        }

        fn flush(&mut self) {
            // In-memory storage: nothing to persist.
        }
    }
}
