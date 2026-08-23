// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::CoverSource;
    use riff::app::errors::AppError;
    use riff::app::playlist_manager;
    use riff::app::traits::{AudioFormatInfo, MetadataReader, MetadataWriter, TagEdit};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    /// Build a track whose id and file path are both derived from `path`,
    /// with a track number so ordering-sensitive queries are testable.
    fn track_at(path: &std::path::Path, track_number: u32) -> Track {
        let path_string = path.to_string_lossy().into_owned();
        let mut track = crate::test_utils::create_test_track_with_metadata(
            &path_string,
            &path_string,
            "Folder Artist",
            "Folder Song",
            "Folder Album",
        );
        track.metadata.track_number = Some(track_number);
        track
    }

    #[test]
    fn test_mutex_ext_lock_or_recover() {
        let mutex = Mutex::new(42);
        let guard = mutex.lock_or_recover();
        assert_eq!(*guard, 42);

        // Test that the guard properly releases the lock when dropped
        drop(guard);
        let guard2 = mutex.lock_or_recover();
        assert_eq!(*guard2, 42);
    }

    #[test]
    fn test_library_manager_new() {
        let library = LibraryManager::new();
        assert!(library.all_tracks().is_empty());
        assert!(library.all_artists().is_empty());
        assert!(library.albums.is_empty());
    }

    #[test]
    fn test_app_state_advanced_mode_defaults_to_false() {
        // Progressive disclosure (REQ-UI-006): the UI starts minimal; power
        // features stay hidden until the user opts in via the toggle.
        let state = AppState::new();
        assert!(!state.ui_flags.advanced_mode);
    }

    #[test]
    fn test_app_state_high_contrast_defaults_to_false() {
        // Accessibility (REQ-UI-007): the high-contrast theme is opt-in; the
        // app starts with the regular light/dark palette.
        let state = AppState::new();
        assert!(!state.ui_flags.high_contrast);
    }

    #[test]
    fn test_app_state_muted_defaults_to_false() {
        // Mute (REQ-UI-003-08): the app starts unmuted.
        let state = AppState::new();
        assert!(!state.muted);
    }

    #[test]
    fn test_app_state_replaygain_defaults_to_disabled() {
        // ReplayGain (Task 4.3) is opt-in.
        let state = AppState::new();
        assert!(!state.replaygain_enabled);
    }

    #[test]
    fn test_replaygain_factor_disabled_is_neutral() {
        let f = replaygain_factor(false, Some(-6.0), Some(0.9));
        assert!((f - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_replaygain_factor_missing_gain_is_neutral() {
        let f = replaygain_factor(true, None, Some(0.9));
        assert!((f - 1.0).abs() < f32::EPSILON);
        let f = replaygain_factor(true, None, None);
        assert!((f - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_replaygain_factor_converts_db_to_linear() {
        // -6 dB ≈ 0.5012, +6 dB ≈ 1.9953, 0 dB → exactly 1.0.
        let f = replaygain_factor(true, Some(-6.0), None);
        assert!((f - 0.5012).abs() < 1e-3);
        let f = replaygain_factor(true, Some(6.0), None);
        assert!((f - 1.9953).abs() < 1e-3);
        let f = replaygain_factor(true, Some(0.0), None);
        assert!((f - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_replaygain_factor_peak_caps_positive_gain() {
        // +6 dB wants ≈1.995, but peak 0.8 caps the factor at 1/0.8 = 1.25 so
        // factor * peak <= 1.0 (no clipping).
        let f = replaygain_factor(true, Some(6.0), Some(0.8));
        assert!((f - 1.25).abs() < 1e-6);
        assert!(f * 0.8 <= 1.0 + 1e-6);
    }

    #[test]
    fn test_replaygain_factor_peak_does_not_raise_attenuation() {
        // Negative gain (≈0.5012) sits below the 1/0.9 ≈ 1.111 cap → uncapped.
        let f = replaygain_factor(true, Some(-6.0), Some(0.9));
        assert!((f - 0.5012).abs() < 1e-3);
    }

    #[test]
    fn test_replaygain_factor_zero_or_missing_peak_no_divide_by_zero() {
        // peak 0.0 is not positive, so it is ignored (no 1/0 division).
        let f = replaygain_factor(true, Some(6.0), Some(0.0));
        assert!((f - 1.9953).abs() < 1e-3);
        let f = replaygain_factor(true, Some(6.0), None);
        assert!((f - 1.9953).abs() < 1e-3);
    }

    // --- Gapless-playback helpers (Task 4.1) ----------------------------------
    //
    // The engine (`run_audio_engine`) cannot run headlessly, so every gapless
    // decision it makes is delegated to these pure helpers — testing them IS
    // testing the engine's decision logic.

    #[test]
    fn test_gapless_formats_compatible_same_rate_and_channels() {
        assert!(formats_gapless_compatible(44_100, 2, 44_100, 2));
        assert!(formats_gapless_compatible(48_000, 2, 48_000, 2));
        assert!(formats_gapless_compatible(96_000, 1, 96_000, 1));
    }

    #[test]
    fn test_gapless_formats_different_rate_is_incompatible() {
        // The canonical mismatch: 44.1 kHz followed by 48 kHz.
        assert!(!formats_gapless_compatible(44_100, 2, 48_000, 2));
        assert!(!formats_gapless_compatible(48_000, 2, 44_100, 2));
    }

    #[test]
    fn test_gapless_formats_different_channels_is_incompatible() {
        assert!(!formats_gapless_compatible(44_100, 2, 44_100, 1));
        assert!(!formats_gapless_compatible(48_000, 1, 48_000, 6));
    }

    #[test]
    fn test_gapless_formats_rate_and_channels_both_differ() {
        assert!(!formats_gapless_compatible(44_100, 2, 48_000, 1));
    }

    #[test]
    fn test_gapless_elapsed_from_samples_exact_math() {
        // 1 s of stereo 44.1 kHz = 88_200 interleaved samples.
        assert_eq!(
            elapsed_from_samples(88_200, 44_100, 2),
            Duration::from_secs(1)
        );
        // Mono: samples == frames.
        assert_eq!(
            elapsed_from_samples(48_000, 48_000, 1),
            Duration::from_secs(1)
        );
        // Half a second at stereo 44.1 kHz.
        assert_eq!(
            elapsed_from_samples(44_100, 44_100, 2),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn test_gapless_elapsed_zero_samples_is_zero() {
        assert_eq!(elapsed_from_samples(0, 44_100, 2), Duration::ZERO);
    }

    #[test]
    fn test_gapless_elapsed_truncates_partial_frame() {
        // 5 interleaved stereo samples = 2 full frames (1 leftover truncated).
        let elapsed = elapsed_from_samples(5, 44_100, 2);
        assert_eq!(elapsed, elapsed_from_samples(4, 44_100, 2));
    }

    #[test]
    fn test_gapless_elapsed_degenerate_rate_or_channels_no_div_by_zero() {
        // Zero rate clamps to 1 Hz: 20 samples / 2 ch = 10 frames / 1 Hz = 10 s.
        assert_eq!(elapsed_from_samples(20, 0, 2), Duration::from_secs(10));
        // Zero channels clamp to 1: frames == samples, 5 frames / 5 Hz = 1 s.
        assert_eq!(elapsed_from_samples(5, 5, 0), Duration::from_secs(1));
        // Both zero: still defined, no panic.
        assert_eq!(elapsed_from_samples(3, 0, 0), Duration::from_secs(3));
    }

    #[test]
    fn test_gapless_pre_buffer_cap_math() {
        // 4 s at 48 kHz stereo = 384_000 interleaved samples.
        assert_eq!(pre_buffer_cap(48_000, 2, 4.0), 384_000);
        // 4 s at 44.1 kHz stereo.
        assert_eq!(pre_buffer_cap(44_100, 2, 4.0), 352_800);
        // Mono halves the cap.
        assert_eq!(pre_buffer_cap(48_000, 1, 4.0), 192_000);
        // Zero seconds buffers nothing.
        assert_eq!(pre_buffer_cap(48_000, 2, 0.0), 0);
    }

    #[test]
    fn test_gapless_pre_buffer_cap_negative_or_nonfinite_is_zero() {
        assert_eq!(pre_buffer_cap(48_000, 2, -1.0), 0);
        assert_eq!(pre_buffer_cap(48_000, 2, f32::INFINITY), 0);
        assert_eq!(pre_buffer_cap(48_000, 2, f32::NEG_INFINITY), 0);
        assert_eq!(pre_buffer_cap(48_000, 2, f32::NAN), 0);
    }

    /// Build the gapless-conditions struct used by [`is_gapless_eligible`].
    fn gapless_conditions(
        queue: QueueConditions,
        formats_compatible: bool,
        has_successor: bool,
    ) -> GaplessConditions {
        GaplessConditions {
            queue,
            formats_compatible,
            has_successor,
        }
    }

    /// Shorthand for the queue part of [`gapless_conditions`].
    fn queue_conditions(shuffle: bool, repeat_one: bool) -> QueueConditions {
        QueueConditions {
            shuffle,
            repeat_one,
        }
    }

    #[test]
    fn test_gapless_eligible_true_only_when_all_conditions_hold() {
        assert!(is_gapless_eligible(gapless_conditions(
            queue_conditions(false, false),
            true,
            true
        )));
    }

    #[test]
    fn test_gapless_eligible_false_when_shuffling() {
        // The successor is not predictable far enough ahead in shuffle mode.
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(true, false),
            true,
            true
        )));
    }

    #[test]
    fn test_gapless_eligible_false_when_repeat_one() {
        // Repeat-one loops via its own engine EOF branch, not this helper.
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(false, true),
            true,
            true
        )));
    }

    #[test]
    fn test_gapless_eligible_false_when_formats_incompatible() {
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(false, false),
            false,
            true
        )));
    }

    #[test]
    fn test_gapless_eligible_false_without_successor() {
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(false, false),
            true,
            false
        )));
    }

    #[test]
    fn test_gapless_eligible_false_when_multiple_conditions_fail() {
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(true, true),
            false,
            false
        )));
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(true, false),
            false,
            true
        )));
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(false, true),
            false,
            false
        )));
        assert!(!is_gapless_eligible(gapless_conditions(
            queue_conditions(true, true),
            true,
            true
        )));
    }

    // --- repeat-one gapless handoff (engine EOF branch) -------------------------

    #[test]
    fn test_repeat_one_handoff_eligible_when_not_shuffled_compatible_with_successor() {
        // The one fully eligible case: repeat-one, sequential order, same
        // format, and a pre-buffered copy of the looping track exists.
        assert!(repeat_one_handoff_eligible(false, true, true, true));
    }

    #[test]
    fn test_repeat_one_handoff_ineligible_when_shuffling() {
        // Shuffled: the looping track is not guaranteed to be up next.
        assert!(!repeat_one_handoff_eligible(true, true, true, true));
    }

    #[test]
    fn test_repeat_one_handoff_ineligible_when_formats_incompatible() {
        // A format change requires the gapped path (stream reinit).
        assert!(!repeat_one_handoff_eligible(false, true, false, true));
    }

    #[test]
    fn test_repeat_one_handoff_ineligible_without_successor() {
        // Nothing pre-buffered: the seamless restart has nothing to flush.
        assert!(!repeat_one_handoff_eligible(false, true, true, false));
    }

    #[test]
    fn test_repeat_one_handoff_ineligible_when_repeat_one_off() {
        // Plain sequential playback is `is_gapless_eligible`'s job, not this.
        assert!(!repeat_one_handoff_eligible(false, false, true, true));
    }

    #[test]
    fn test_effective_volume_zeroed_while_muted_and_restored_on_unmute() {
        let mut state = AppState::new();
        state.current_volume = 0.7;

        // Unmuted: the engine receives the slider value.
        assert!((state.effective_volume() - 0.7).abs() < f32::EPSILON);

        // Muted: silence, without moving the slider.
        state.muted = true;
        assert!((state.effective_volume() - 0.0).abs() < f32::EPSILON);
        assert!((state.current_volume - 0.7).abs() < f32::EPSILON);

        // Unmuted: the slider's value is restored.
        state.muted = false;
        assert!((state.effective_volume() - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn test_library_manager_add_track() {
        let library = LibraryManager::new();
        let _reader = LoftyMetadataReader::new();

        let _test_file = std::path::PathBuf::from("test.mp3");
        let track_id = TrackId("test.mp3".to_string());

        // This test would require an actual MP3 file to work properly
        // For now, we'll just test the structure
        assert!(library.get_track(&track_id).is_none());
    }

    #[test]
    fn test_library_manager_search() {
        let mut library = LibraryManager::new();

        // `LibraryManager::search` matches the query against track *metadata*
        // (title/artist/album/album_artist), so the fixtures carry titles that
        // the query can match.
        let track1 = Track {
            id: TrackId("track1.mp3".to_string()),
            file_path: std::path::PathBuf::from("track1.mp3"),
            metadata: crate::domain::TrackMetadata {
                title: Some("track1".to_string()),
                ..Default::default()
            },
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
        };

        let track2 = Track {
            id: TrackId("track2.mp3".to_string()),
            file_path: std::path::PathBuf::from("track2.mp3"),
            metadata: crate::domain::TrackMetadata {
                title: Some("track2".to_string()),
                ..Default::default()
            },
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
        };

        library.tracks.insert(track1.id.clone(), track1);
        library.tracks.insert(track2.id.clone(), track2);

        let results = library.search("track1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id.0, "track1.mp3");
    }

    // --- add_track indexing ---------------------------------------------------

    #[test]
    fn test_add_track_indexes_track_artist_album() {
        let mut library = LibraryManager::new();
        let mut track = crate::test_utils::create_test_track_with_metadata(
            "t1.mp3",
            "music/artist/album/01.mp3",
            "The Artist",
            "Song One",
            "The Album",
        );
        track.metadata.track_number = Some(1);
        track.metadata.year = Some(2001);
        track.metadata.genre = Some("Rock".to_string());

        library.add_track(track);

        // Track index.
        assert_eq!(library.all_tracks().len(), 1);
        let id = TrackId("t1.mp3".to_string());
        let fetched = library.get_track(&id).expect("track should be indexed");
        assert_eq!(fetched.metadata.title.as_deref(), Some("Song One"));

        // Artist index, with the album key attached.
        assert_eq!(library.all_artists().len(), 1);
        let artist_albums = library.get_artist_albums("The Artist");
        assert_eq!(artist_albums.len(), 1);
        assert_eq!(artist_albums[0].title, "The Album");

        // Album index keyed by "{album_artist} - {album}", with year/genre.
        assert_eq!(library.all_albums().len(), 1);
        let album_tracks = library.get_album_tracks("The Artist - The Album");
        assert_eq!(album_tracks.len(), 1);
        assert_eq!(album_tracks[0].id, id);
        let album = &library.albums["The Artist - The Album"];
        assert_eq!(album.artist, "The Artist");
        assert_eq!(album.year, Some(2001));
        assert_eq!(album.genre.as_deref(), Some("Rock"));
    }

    #[test]
    fn test_add_track_sorts_album_tracks_by_track_number() {
        let mut library = LibraryManager::new();
        // Deliberately inserted out of order (descending track numbers). Note:
        // `add_track` sorts the album's track list against the tracks map
        // *before* inserting the new track, so the newest entry sorts with key
        // 0; descending insertion exercises re-sorting on every add and yields
        // a fully track_number-sorted list.
        for (num, id) in [(3u32, "c.mp3"), (2, "b.mp3"), (1, "a.mp3")] {
            let mut track = crate::test_utils::create_test_track_with_metadata(
                id,
                &format!("music/artist/album/{id}"),
                "Artist",
                "Song",
                "Album",
            );
            track.metadata.track_number = Some(num);
            library.add_track(track);
        }

        let album_tracks = library.get_album_tracks("Artist - Album");
        let ids: Vec<&str> = album_tracks.iter().map(|t| t.id.0.as_str()).collect();
        assert_eq!(ids, vec!["a.mp3", "b.mp3", "c.mp3"]);
    }

    #[test]
    fn test_add_track_uses_album_artist_for_indexing() {
        let mut library = LibraryManager::new();
        let mut track = crate::test_utils::create_test_track_with_metadata(
            "feat.mp3",
            "music/va/album/feat.mp3",
            "Solo Singer",
            "Feature Song",
            "Compilation",
        );
        track.metadata.album_artist = Some("Various Artists".to_string());

        library.add_track(track);

        // Indexed under the album artist, NOT the track artist.
        assert!(library.get_artist_albums("Solo Singer").is_empty());
        let albums = library.get_artist_albums("Various Artists");
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].artist, "Various Artists");
        assert_eq!(
            library
                .get_album_tracks("Various Artists - Compilation")
                .len(),
            1
        );
    }

    #[test]
    fn test_add_track_dedup_same_track_id() {
        let mut library = LibraryManager::new();
        let track = crate::test_utils::create_test_track_with_metadata(
            "same.mp3",
            "music/same.mp3",
            "Artist",
            "Title",
            "Album",
        );

        library.add_track(track.clone());
        library.add_track(track);

        assert_eq!(library.tracks.len(), 1);
        assert_eq!(library.all_albums().len(), 1);
        assert_eq!(library.get_album_tracks("Artist - Album").len(), 1);
        assert_eq!(library.all_artists().len(), 1);
        assert_eq!(library.get_artist_albums("Artist").len(), 1);
    }

    // --- removal ---------------------------------------------------------------

    #[test]
    fn test_remove_track_evicts_and_cleans_orphans() {
        let mut library = LibraryManager::new();
        let mut t1 = crate::test_utils::create_test_track_with_metadata(
            "a1.mp3",
            "music/artist/album/a1.mp3",
            "Artist",
            "One",
            "Album",
        );
        t1.metadata.track_number = Some(1);
        let mut t2 = crate::test_utils::create_test_track_with_metadata(
            "a2.mp3",
            "music/artist/album/a2.mp3",
            "Artist",
            "Two",
            "Album",
        );
        t2.metadata.track_number = Some(2);
        library.add_track(t1);
        library.add_track(t2);

        // Removing one track keeps the album alive with the remaining track.
        library.remove_track(&TrackId("a1.mp3".to_string()));
        assert_eq!(library.all_tracks().len(), 1);
        assert_eq!(library.get_album_tracks("Artist - Album").len(), 1);

        // Removing the last track evicts the album and detaches it from the
        // artist (the artist entry itself lingers with an empty album list).
        library.remove_track(&TrackId("a2.mp3".to_string()));
        assert!(library.all_tracks().is_empty());
        assert!(library.albums.is_empty());
        assert!(library.get_album_tracks("Artist - Album").is_empty());
        assert!(library.get_artist_albums("Artist").is_empty());

        // Removing an unknown id is a no-op.
        library.remove_track(&TrackId("ghost.mp3".to_string()));
        assert!(library.all_tracks().is_empty());
    }

    #[test]
    fn test_remove_tracks_by_root_removes_and_cleans_up() {
        let mut library = LibraryManager::new();
        let entries = [
            ("music/a/1.mp3", "Artist A", "Album A"),
            ("music/a/sub/2.mp3", "Artist A", "Album A"),
            ("music/ab/3.mp3", "Artist AB", "Album AB"),
            ("music/b/4.mp3", "Artist B", "Album B"),
        ];
        for (path, artist, album) in entries {
            library.add_track(crate::test_utils::create_test_track_with_metadata(
                path, path, artist, "Song", album,
            ));
        }

        // Component-wise prefix: "music/a" matches "music/a/..." (including
        // nested) but NOT "music/ab/...".
        let removed = library.remove_tracks_by_root(std::path::Path::new("music/a"));
        assert_eq!(removed, 2);
        assert_eq!(library.all_tracks().len(), 2);
        assert!(library
            .get_track(&TrackId("music/a/1.mp3".to_string()))
            .is_none());
        assert!(library
            .get_track(&TrackId("music/a/sub/2.mp3".to_string()))
            .is_none());
        assert!(library
            .get_track(&TrackId("music/ab/3.mp3".to_string()))
            .is_some());

        // Orphaned album cleaned up; unrelated albums untouched.
        assert!(!library.albums.contains_key("Artist A - Album A"));
        assert_eq!(library.all_albums().len(), 2);
    }

    // --- search -----------------------------------------------------------------

    #[test]
    fn test_search_matches_fields_case_insensitively() {
        let mut library = LibraryManager::new();
        let mut tracks = vec![
            crate::test_utils::create_test_track_with_metadata(
                "t1.mp3",
                "music/t1.mp3",
                "Someone",
                "Bohemian Rhapsody",
                "News",
            ),
            crate::test_utils::create_test_track_with_metadata(
                "t2.mp3",
                "music/t2.mp3",
                "Queen Band",
                "Song",
                "Album",
            ),
            crate::test_utils::create_test_track_with_metadata(
                "t3.mp3",
                "music/t3.mp3",
                "Someone",
                "Song",
                "Night At The Opera",
            ),
            crate::test_utils::create_test_track_with_metadata(
                "t4.mp3",
                "music/t4.mp3",
                "Someone",
                "Song",
                "Album",
            ),
        ];
        tracks[3].metadata.album_artist = Some("Opera Various".to_string());
        for track in tracks {
            library.add_track(track);
        }

        // Title match.
        let by_title = library.search("rhapsody");
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].id.0, "t1.mp3");

        // Artist match.
        assert_eq!(library.search("queen").len(), 1);

        // Album + album_artist matches, case-insensitive query.
        assert_eq!(library.search("OPERA").len(), 2);

        assert!(library.search("no-such-text").is_empty());
        // Empty query matches everything.
        assert_eq!(library.search("").len(), 4);
    }

    // --- folder queries (real tempdir on disk) -----------------------------------

    #[test]
    fn test_tracks_in_folder_exact_parent_sorted() {
        let root = tempfile::tempdir().unwrap();
        let album_dir = root.path().join("Artist").join("Album");
        let nested_dir = album_dir.join("Bonus");
        std::fs::create_dir_all(&nested_dir).unwrap();

        let mut library = LibraryManager::new();
        // Inserted out of order; expect sorting by track_number.
        library.add_track(track_at(&album_dir.join("b - second.mp3"), 2));
        library.add_track(track_at(&album_dir.join("a - first.mp3"), 1));
        // A track in a subfolder must NOT appear for the album-dir query.
        library.add_track(track_at(&nested_dir.join("bonus.mp3"), 3));
        // A track directly under the root must NOT appear either.
        library.add_track(track_at(&root.path().join("loose.mp3"), 9));

        let in_album = library.tracks_in_folder(&album_dir);
        let names: Vec<String> = in_album
            .iter()
            .map(|t| {
                t.file_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["a - first.mp3", "b - second.mp3"]);

        let in_root = library.tracks_in_folder(root.path());
        assert_eq!(in_root.len(), 1);
        assert_eq!(in_root[0].file_path, root.path().join("loose.mp3"));
    }

    #[test]
    fn test_track_ids_in_folder_tree_prefix_match() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("Sub");
        std::fs::create_dir_all(&sub).unwrap();

        let mut library = LibraryManager::new();
        library.add_track(track_at(&root.path().join("loose.mp3"), 1));
        library.add_track(track_at(&sub.join("deep.mp3"), 2));
        library.add_track(track_at(std::path::Path::new("elsewhere/other.mp3"), 3));

        let ids = library.track_ids_in_folder_tree(root.path());
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&TrackId::from_path(&root.path().join("loose.mp3"))));
        assert!(ids.contains(&TrackId::from_path(&sub.join("deep.mp3"))));

        // A nested folder only sees its own subtree.
        let in_sub = library.track_ids_in_folder_tree(&sub);
        assert_eq!(in_sub, vec![TrackId::from_path(&sub.join("deep.mp3"))]);
    }

    #[test]
    fn test_folder_has_audio_true_and_false() {
        let root = tempfile::tempdir().unwrap();
        let mut library = LibraryManager::new();
        library.add_track(track_at(&root.path().join("song.mp3"), 1));

        assert!(library.folder_has_audio(root.path()));
        assert!(!library.folder_has_audio(&root.path().join("NoMusic")));
    }

    #[test]
    fn test_subdirs_with_audio_lists_existing_dirs_only() {
        let root = tempfile::tempdir().unwrap();
        let artist_dir = root.path().join("Artist");
        let other_dir = root.path().join("Other");
        std::fs::create_dir_all(&artist_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        // "Ghost" is referenced by a track below but never created on disk.

        let mut library = LibraryManager::new();
        library.add_track(track_at(&artist_dir.join("a.mp3"), 1));
        library.add_track(track_at(&artist_dir.join("nested").join("b.mp3"), 2));
        library.add_track(track_at(&other_dir.join("c.mp3"), 3));
        library.add_track(track_at(&root.path().join("Ghost").join("g.mp3"), 4));
        library.add_track(track_at(&root.path().join("loose.mp3"), 5));

        // Only real directories with audio underneath are listed, sorted and
        // deduplicated (two Artist tracks collapse to one entry).
        let dirs = library.subdirs_with_audio(root.path());
        assert_eq!(dirs, vec![artist_dir, other_dir]);
    }

    // --- playlists: entry validity (read-time, app layer) -------------------------
    //
    // Playlist persistence and CRUD moved behind the `PlaylistStore` port;
    // their behavior is tested against real SQL in `infra_tests.rs`. What
    // stays here is the read-time validity rule the UI plays by.

    #[test]
    fn test_track_is_valid_and_valid_tracks_filter_invalid_entries() {
        let root = tempfile::tempdir().unwrap();
        let real_file = root.path().join("real.mp3");
        std::fs::write(&real_file, b"not really audio").unwrap();
        let missing_file = root.path().join("missing.mp3");

        let mut library = LibraryManager::new();
        library.add_track(crate::test_utils::create_test_track(
            &real_file.to_string_lossy(),
            &real_file.to_string_lossy(),
        ));
        library.add_track(crate::test_utils::create_test_track(
            &missing_file.to_string_lossy(),
            &missing_file.to_string_lossy(),
        ));

        let real_id = TrackId(real_file.to_string_lossy().into_owned());
        let missing_id = TrackId(missing_file.to_string_lossy().into_owned());
        let unknown_id = TrackId("never scanned.flac".to_string());

        // Valid: in the library AND the file exists on disk.
        assert!(playlist_manager::track_is_valid(&library, &real_id));
        // Invalid: file gone, or track never scanned.
        assert!(!playlist_manager::track_is_valid(&library, &missing_id));
        assert!(!playlist_manager::track_is_valid(&library, &unknown_id));

        // Loading a playlist keeps only valid entries, in playlist order.
        let playlist = Playlist {
            id: PlaylistId("mixed".to_string()),
            name: "Mixed".to_string(),
            tracks: vec![missing_id.clone(), real_id.clone(), unknown_id.clone()],
            created: Some(std::time::SystemTime::now()),
        };
        assert_eq!(
            playlist_manager::valid_tracks(&library, &playlist),
            vec![real_id]
        );
    }

    // --- scan_and_add_tracks with a mock MetadataReader ----------------------------

    /// Minimal `MetadataReader` for exercising `scan_and_add_tracks` without
    /// real audio files. `fail` simulates unreadable/corrupt input.
    struct MockMetadataReader {
        fail: bool,
    }

    impl MetadataReader for MockMetadataReader {
        fn read_metadata(&self, _path: &Path) -> Result<TrackMetadata, AppError> {
            Ok(TrackMetadata::default())
        }

        fn read_duration(&self, _path: &Path) -> Result<Option<Duration>, AppError> {
            Ok(Some(Duration::from_secs(90)))
        }

        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, AppError> {
            Ok(CoverSource::None)
        }

        fn read_audio_format(&self, _path: &Path) -> Result<AudioFormatInfo, AppError> {
            Ok(AudioFormatInfo {
                sample_rate: 44_100,
                channels: 2,
                duration: Some(Duration::from_secs(90)),
            })
        }

        fn read_all(
            &self,
            path: &Path,
        ) -> Result<
            (
                TrackMetadata,
                Option<Duration>,
                CoverSource,
                AudioFormatInfo,
            ),
            AppError,
        > {
            if self.fail {
                return Err(AppError::MetadataRead(format!("mock failure: {path:?}")));
            }
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
            let metadata = TrackMetadata {
                title: Some(title),
                artist: Some("Mock Artist".to_string()),
                album: Some("Mock Album".to_string()),
                ..Default::default()
            };
            Ok((
                metadata,
                Some(Duration::from_secs(90)),
                CoverSource::None,
                AudioFormatInfo {
                    sample_rate: 44_100,
                    channels: 2,
                    duration: Some(Duration::from_secs(90)),
                },
            ))
        }
    }

    #[test]
    fn test_scan_and_add_tracks_with_mock_reader() {
        let mut library = LibraryManager::new();
        let reader = MockMetadataReader { fail: false };
        let paths = vec![PathBuf::from("scan/a.mp3"), PathBuf::from("scan/b.mp3")];

        let added = library.scan_and_add_tracks(paths.clone(), &reader);
        assert_eq!(added, 2);
        assert_eq!(library.all_tracks().len(), 2);

        // Metadata and audio-format info from the reader land on the track.
        let scanned = library
            .get_track(&TrackId::from_path(&PathBuf::from("scan/a.mp3")))
            .expect("scanned track should be indexed");
        assert_eq!(scanned.metadata.title.as_deref(), Some("a"));
        assert_eq!(scanned.duration, Some(Duration::from_secs(90)));
        assert_eq!(scanned.sample_rate, Some(44_100));
        assert_eq!(scanned.channels, Some(2));
        // Indexes are built from the scanned metadata.
        assert_eq!(library.get_artist_albums("Mock Artist").len(), 1);

        // Re-scanning the same paths adds nothing (dedup by TrackId).
        let added_again = library.scan_and_add_tracks(paths, &reader);
        assert_eq!(added_again, 0);
        assert_eq!(library.all_tracks().len(), 2);
    }

    #[test]
    fn test_scan_and_add_tracks_skips_unreadable_files() {
        let mut library = LibraryManager::new();
        let reader = MockMetadataReader { fail: true };

        let added = library.scan_and_add_tracks(
            vec![PathBuf::from("bad/a.mp3"), PathBuf::from("bad/b.mp3")],
            &reader,
        );
        assert_eq!(added, 0);
        assert!(library.all_tracks().is_empty());
    }

    // --- play count + smart playlists (REQ-ML-009) --------------------------------

    /// Build a library track with a stable id and path for smart-playlist
    /// tests; play-history fields start at their defaults (0 / None / None).
    fn playlist_track(id: &str) -> Track {
        crate::test_utils::create_test_track(id, &format!("music/{id}"))
    }

    #[test]
    fn test_increment_play_count_increments_and_sets_last_played() {
        let mut library = LibraryManager::new();
        let id = TrackId("p.mp3".to_string());
        library.add_track(playlist_track("p.mp3"));
        assert!(library.get_track(&id).unwrap().last_played.is_none());

        library.increment_play_count(&id);
        let track = library.get_track(&id).unwrap();
        assert_eq!(track.play_count, 1);
        let last = track.last_played.expect("last_played must be stamped");
        assert!(last.elapsed().unwrap().as_secs() < 5);

        library.increment_play_count(&id);
        assert_eq!(library.get_track(&id).unwrap().play_count, 2);
    }

    #[test]
    fn test_increment_play_count_unknown_id_is_noop() {
        let mut library = LibraryManager::new();
        library.increment_play_count(&TrackId("ghost.mp3".to_string()));
        assert!(library.all_tracks().is_empty());
    }

    #[test]
    fn test_scan_and_add_tracks_stamps_date_added_and_zero_plays() {
        let mut library = LibraryManager::new();
        let reader = MockMetadataReader { fail: false };
        library.scan_and_add_tracks(vec![PathBuf::from("scan/fresh.mp3")], &reader);

        let track = library
            .get_track(&TrackId::from_path(&PathBuf::from("scan/fresh.mp3")))
            .expect("scanned track should be indexed");
        assert_eq!(track.play_count, 0);
        assert!(track.last_played.is_none());
        assert!(track.date_added.is_some());
    }

    #[test]
    fn test_scan_and_add_tracks_dedupes_same_file_across_separate_scans() {
        // Two library paths can overlap: scanning a second root that contains
        // an already-indexed file must not duplicate the track or its index
        // entries. Dedup is by TrackId (the full file path).
        let mut library = LibraryManager::new();
        let reader = MockMetadataReader { fail: false };
        let shared = PathBuf::from("music/shared.mp3");

        let first_added = library.scan_and_add_tracks(
            vec![shared.clone(), PathBuf::from("music/first-only.mp3")],
            &reader,
        );
        assert_eq!(first_added, 2);

        // The second scan overlaps on `shared` and adds one new file.
        let second_added = library.scan_and_add_tracks(
            vec![shared.clone(), PathBuf::from("music/second-only.mp3")],
            &reader,
        );
        assert_eq!(second_added, 1);

        assert_eq!(library.all_tracks().len(), 3);
        // Indexes stay consistent: one artist, one album, three tracks in it.
        assert_eq!(library.all_artists().len(), 1);
        assert_eq!(library.all_albums().len(), 1);
        assert_eq!(
            library.get_album_tracks("Mock Artist - Mock Album").len(),
            3
        );
    }

    #[test]
    fn test_scan_of_unavailable_path_adds_nothing() {
        // An unavailable (missing or removed) library path must degrade to
        // "nothing scanned", not an error: the scanner skips the unreadable
        // root entry and yields no files, and the manager indexes nothing.
        let temp_dir = tempfile::tempdir().unwrap();
        let missing = temp_dir.path().join("does-not-exist");
        assert!(!missing.exists());

        let scanner = AudioFileScanner::new(Arc::new(AtomicBool::new(false)));
        let files = scanner.scan(&missing);
        assert!(files.is_empty());

        let mut library = LibraryManager::new();
        let reader = MockMetadataReader { fail: false };
        let added = library.scan_and_add_tracks(files, &reader);
        assert_eq!(added, 0);
        assert!(library.all_tracks().is_empty());
        assert!(library.all_artists().is_empty());
        assert!(library.all_albums().is_empty());
    }

    #[test]
    fn test_smart_playlist_recently_added_orders_newest_first() {
        let mut library = LibraryManager::new();
        let now = SystemTime::now();
        let day = Duration::from_hours(24);
        for (id, age_days) in [("old.mp3", 30u32), ("mid.mp3", 10), ("new.mp3", 1)] {
            let mut track = playlist_track(id);
            track.date_added = Some(now - day * age_days);
            library.add_track(track);
        }
        // A track without date_added (e.g. legacy cache entry) is excluded.
        library.add_track(playlist_track("nodate.mp3"));

        let ids = library.smart_playlist(SmartPlaylistKind::RecentlyAdded, 10);
        assert_eq!(
            ids,
            vec![
                TrackId("new.mp3".to_string()),
                TrackId("mid.mp3".to_string()),
                TrackId("old.mp3".to_string()),
            ]
        );

        // The limit caps the result.
        let top1 = library.smart_playlist(SmartPlaylistKind::RecentlyAdded, 1);
        assert_eq!(top1, vec![TrackId("new.mp3".to_string())]);
    }

    #[test]
    fn test_smart_playlist_most_played_orders_by_count_and_excludes_unplayed() {
        let mut library = LibraryManager::new();
        for (id, count) in [("a.mp3", 3u32), ("b.mp3", 10), ("c.mp3", 3), ("d.mp3", 0)] {
            let mut track = playlist_track(id);
            track.play_count = count;
            library.add_track(track);
        }

        let ids = library.smart_playlist(SmartPlaylistKind::MostPlayed, 10);
        // Highest count first; the unplayed track (d) is excluded entirely.
        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], TrackId("b.mp3".to_string()));
        // Tie at 3 resolves deterministically by title/path: a before c.
        assert_eq!(ids[1], TrackId("a.mp3".to_string()));
        assert_eq!(ids[2], TrackId("c.mp3".to_string()));
    }

    #[test]
    fn test_smart_playlist_never_played_returns_only_unplayed_sorted_by_path() {
        let mut library = LibraryManager::new();
        for (id, count) in [("z.mp3", 0u32), ("a.mp3", 5), ("m.mp3", 0)] {
            let mut track = playlist_track(id);
            track.play_count = count;
            library.add_track(track);
        }

        let ids = library.smart_playlist(SmartPlaylistKind::NeverPlayed, 10);
        assert_eq!(
            ids,
            vec![TrackId("m.mp3".to_string()), TrackId("z.mp3".to_string())]
        );
    }

    #[test]
    fn test_smart_playlist_lost_gems_uses_90_day_threshold() {
        let mut library = LibraryManager::new();
        let now = SystemTime::now();
        let day = Duration::from_hours(24);

        // Played 100 days ago -> unheard for 90+ days -> qualifies.
        let mut ancient = playlist_track("ancient.mp3");
        ancient.play_count = 5;
        ancient.last_played = Some(now - day * 100);
        library.add_track(ancient);

        // Played 10 days ago -> still fresh, excluded.
        let mut recent = playlist_track("recent.mp3");
        recent.play_count = 5;
        recent.last_played = Some(now - day * 10);
        library.add_track(recent);

        // Never played -> qualifies too: the Lost Gems rule is
        // "last played older than 90 days OR never played".
        library.add_track(playlist_track("never.mp3"));

        let ids = library.smart_playlist(SmartPlaylistKind::LostGems, 10);
        assert_eq!(
            ids,
            vec![
                TrackId("ancient.mp3".to_string()),
                TrackId("never.mp3".to_string()),
            ]
        );
    }

    #[test]
    fn test_smart_playlist_lost_gems_orders_longest_unheard_first_then_never_played() {
        let mut library = LibraryManager::new();
        let now = SystemTime::now();
        let day = Duration::from_hours(24);

        // Two gems with hand-worked play stamps: 200 and 95 days unheard.
        let mut two_hundred = playlist_track("b_two_hundred.mp3");
        two_hundred.play_count = 2;
        two_hundred.last_played = Some(now - day * 200);
        library.add_track(two_hundred);

        let mut ninety_five = playlist_track("a_ninety_five.mp3");
        ninety_five.play_count = 9;
        ninety_five.last_played = Some(now - day * 95);
        library.add_track(ninety_five);

        // Fresh play (2 days ago): excluded regardless of play count.
        let mut fresh = playlist_track("c_fresh.mp3");
        fresh.play_count = 50;
        fresh.last_played = Some(now - day * 2);
        library.add_track(fresh);

        // Never played: included after every once-played gem, in path order.
        library.add_track(playlist_track("z_unplayed.mp3"));
        library.add_track(playlist_track("d_unplayed.mp3"));

        let ids = library.smart_playlist(SmartPlaylistKind::LostGems, 10);
        assert_eq!(
            ids,
            vec![
                TrackId("b_two_hundred.mp3".to_string()),
                TrackId("a_ninety_five.mp3".to_string()),
                TrackId("d_unplayed.mp3".to_string()),
                TrackId("z_unplayed.mp3".to_string()),
            ]
        );
    }

    // --- metadata writing with a mock MetadataWriter --------------------------------

    use crate::mocks::MockMetadataWriter;

    #[test]
    fn test_metadata_writer_success_records_path_and_edit() {
        let writer = MockMetadataWriter::recording();
        let path = PathBuf::from("music/artist/album/song.flac");
        let edit = TagEdit {
            title: Some("New Title".to_string()),
            year: Some(1999),
            ..Default::default()
        };

        assert!(writer.write_metadata(&path, &edit).is_ok());

        let recorded = writer.recorded();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, path);
        assert_eq!(recorded[0].1.title.as_deref(), Some("New Title"));
        assert_eq!(recorded[0].1.year, Some(1999));
        // Unset fields stay `None` (partial-edit contract).
        assert!(recorded[0].1.artist.is_none());
        assert!(recorded[0].1.track_number.is_none());
    }

    #[test]
    fn test_metadata_writer_failure_returns_metadata_write_error() {
        let writer = MockMetadataWriter::failing();

        let result = writer.write_metadata(&PathBuf::from("locked.mp3"), &TagEdit::default());

        let err = result.expect_err("failing writer must return an error");
        assert!(matches!(err, AppError::MetadataWrite(_)));
        assert!(err.to_string().contains("Failed to write tags"));
        assert!(writer.recorded().is_empty());
    }

    /// The edit a user saves from the modal: every text field edited in
    /// place is present, so all fields are `Some`.
    fn saved_edit(title: &str) -> TagEdit {
        TagEdit {
            title: Some(title.to_string()),
            artist: Some("Edited Artist".to_string()),
            album: Some("Edited Album".to_string()),
            album_artist: Some("Edited Album Artist".to_string()),
            genre: Some("Edited Genre".to_string()),
            year: Some(2001),
            track_number: Some(4),
        }
    }

    /// A cached library track with fully-populated tags, so a later edit has
    /// concrete "before" values to be compared against.
    fn cached_track(id: &str, title: &str, artist: &str, year: u32) -> Track {
        let mut track =
            crate::test_utils::create_test_track_with_metadata(id, id, artist, title, "Some Album");
        track.metadata.album_artist = Some(format!("{artist} (AA)"));
        track.metadata.genre = Some("Some Genre".to_string());
        track.metadata.year = Some(year);
        track.metadata.track_number = Some(1);
        track
    }

    /// Successful-write handling as the app performs it
    /// (`RiffApp::poll_tag_write_results`, UI layer, not headless-reachable):
    /// the writer port's result decides, and on a successful write the cached
    /// track's metadata picks up the edit — no rescan. This test drives that
    /// state transition through the public seams: the `MetadataWriter` port
    /// and `TagEdit::apply_to` against the live library cache.
    #[test]
    fn test_successful_write_refreshes_library_cache_immediately() {
        let mut state = AppState::new();
        let track_id = TrackId("music/song.mp3".to_string());
        state.library.add_track(cached_track(
            "music/song.mp3",
            "Old Title",
            "Old Artist",
            1984,
        ));
        state.library.add_track(cached_track(
            "music/other.mp3",
            "Keep Me",
            "Keep Artist",
            1990,
        ));

        // The user saves through the modal; the background writer succeeds.
        let writer = MockMetadataWriter::recording();
        let edit = saved_edit("New Title");
        writer
            .write_metadata(&PathBuf::from("music/song.mp3"), &edit)
            .expect("the write must succeed");
        assert_eq!(writer.recorded().len(), 1);

        // On a successful write the cache is refreshed in place (no rescan):
        // every written field is immediately visible through the library's
        // public queries.
        edit.apply_to(
            &mut state
                .library
                .tracks
                .get_mut(&track_id)
                .expect("edited track must still be cached")
                .metadata,
        );

        let cached = state
            .library
            .get_track(&track_id)
            .expect("track must stay in the library after a tag edit");
        assert_eq!(cached.metadata.title.as_deref(), Some("New Title"));
        assert_eq!(cached.metadata.artist.as_deref(), Some("Edited Artist"));
        assert_eq!(cached.metadata.album.as_deref(), Some("Edited Album"));
        assert_eq!(
            cached.metadata.album_artist.as_deref(),
            Some("Edited Album Artist")
        );
        assert_eq!(cached.metadata.genre.as_deref(), Some("Edited Genre"));
        assert_eq!(cached.metadata.year, Some(2001));
        assert_eq!(cached.metadata.track_number, Some(4));
        // The change is discoverable, not just stored: the new title searches
        // cleanly and the old one is gone, with no rescan.
        assert_eq!(state.library.search("New Title").len(), 1);
        assert!(state.library.search("Old Title").is_empty());

        // Nothing else in the library is disturbed.
        assert_eq!(state.library.all_tracks().len(), 2);
        let untouched = state
            .library
            .get_track(&TrackId("music/other.mp3".to_string()))
            .expect("a neighbouring track must survive the edit");
        assert_eq!(untouched.metadata.title.as_deref(), Some("Keep Me"));
        assert_eq!(untouched.metadata.year, Some(1990));
    }

    /// Failed-write handling as the app performs it
    /// (`RiffApp::poll_tag_write_results`, UI layer, not headless-reachable):
    /// the error must surface as a readable, user-visible message identifying
    /// the file, and the library must be left exactly as it was — a write
    /// that did not land applies nothing.
    #[test]
    fn test_failed_write_keeps_library_intact_and_surfaces_a_user_error() {
        let mut state = AppState::new();
        let track_id = TrackId("music/locked.mp3".to_string());
        state.library.add_track(cached_track(
            "music/locked.mp3",
            "Original Title",
            "Original Artist",
            1999,
        ));

        // The write port fails (unwritable file).
        let writer = MockMetadataWriter::failing();
        let edit = saved_edit("Attempted Title");
        let outcome = writer.write_metadata(&PathBuf::from("music/locked.mp3"), &edit);

        // The failure surfaces as a readable, user-visible error...
        let err = outcome.expect_err("failing writer must return an error");
        assert!(matches!(err, AppError::MetadataWrite(_)));
        let displayed = err.to_string();
        assert!(displayed.starts_with("Failed to write tags"));
        assert!(
            displayed.contains("locked.mp3"),
            "the error should identify the file: {displayed}"
        );

        // ...no write was recorded...
        assert!(writer.recorded().is_empty());

        // ...and the app keeps working with the library intact: the failed
        // write applied nothing, so every field keeps its original value.
        assert_eq!(state.library.all_tracks().len(), 1);
        let cached = state
            .library
            .get_track(&track_id)
            .expect("a failed write must not drop the track");
        assert_eq!(cached.metadata.title.as_deref(), Some("Original Title"));
        assert_eq!(cached.metadata.artist.as_deref(), Some("Original Artist"));
        assert_eq!(cached.metadata.year, Some(1999));
        // Lookup still works afterwards (the app keeps functioning).
        assert_eq!(state.library.search("Original Title").len(), 1);
    }

    #[test]
    fn test_tag_edit_default_is_all_none() {
        let edit = TagEdit::default();
        assert!(edit.title.is_none());
        assert!(edit.artist.is_none());
        assert!(edit.album.is_none());
        assert!(edit.album_artist.is_none());
        assert!(edit.genre.is_none());
        assert!(edit.year.is_none());
        assert!(edit.track_number.is_none());
    }

    #[test]
    fn test_tag_edit_apply_to_updates_only_some_fields() {
        let mut metadata = TrackMetadata {
            title: Some("Old Title".to_string()),
            artist: Some("Keep Me".to_string()),
            ..Default::default()
        };
        let edit = TagEdit {
            title: Some("New Title".to_string()),
            year: Some(2020),
            track_number: Some(7),
            ..Default::default()
        };

        edit.apply_to(&mut metadata);

        // `Some` fields overwrite...
        assert_eq!(metadata.title.as_deref(), Some("New Title"));
        assert_eq!(metadata.year, Some(2020));
        assert_eq!(metadata.track_number, Some(7));
        // ...`None` fields leave existing values untouched.
        assert_eq!(metadata.artist.as_deref(), Some("Keep Me"));
        assert!(metadata.album.is_none());
        assert!(metadata.genre.is_none());
    }

    // --- misc -----------------------------------------------------------------------

    #[test]
    fn test_library_manager_clear_empties_everything() {
        let mut library = crate::integration_helpers::create_mock_library();
        assert!(!library.all_tracks().is_empty());

        library.clear();
        assert!(library.all_tracks().is_empty());
        assert!(library.all_artists().is_empty());
        assert!(library.albums.is_empty());
    }

    // --- SettingsStore port orchestration (mock-driven) -------------------------

    #[test]
    fn test_mock_settings_store_drives_hydration_and_persistence() {
        use riff::app::store::SettingsStore;
        // App-layer orchestration through the port: the same flow
        // `load_persisted_state` + the save call sites use, driven by a mock so
        // no real SQL is involved.
        let mut mock = crate::mocks::MockSettingsStore::default();

        // Hydration: the mock starts with defaults.
        let settings = mock.load_settings().unwrap();
        assert_eq!(settings.scalars.volume, None);
        assert!(settings.library_paths.is_empty());

        // The app saves each change as its own small transaction; the mock
        // records the call sequence.
        mock.save_scalars(&riff::app::state::ScalarSettings {
            volume: Some(0.3),
            ..Default::default()
        })
        .unwrap();
        mock.save_library_paths(&[PathBuf::from("music")]).unwrap();
        let mut states = HashMap::new();
        states.insert(PathBuf::from("music"), WatchState::Enabled);
        mock.save_watch_states(&states).unwrap();

        assert_eq!(
            mock.calls,
            vec![
                crate::mocks::SettingsCall::Scalars,
                crate::mocks::SettingsCall::LibraryPaths,
                crate::mocks::SettingsCall::WatchStates,
            ]
        );

        // A failing port surfaces its error instead of being swallowed.
        mock.fail = true;
        assert!(mock
            .save_scalars(&riff::app::state::ScalarSettings::default())
            .is_err());
        assert!(mock.save_library_paths(&[]).is_err());
        assert!(mock.save_watch_states(&HashMap::new()).is_err());
    }

    // --- Session Projection: bounded views with generation invalidation -----

    use riff::app::projection::{ProjectionKey, TrackListProjection};

    /// Deterministic fixture row for a window slot.
    fn projection_track(n: usize) -> Track {
        crate::test_utils::create_test_track(&n.to_string(), &format!("f:\\{n}.mp3"))
    }

    /// Counting fake loader standing in for the store query port: records
    /// every (offset, limit) call and can fail exactly once on demand.
    struct FakeLoader {
        calls: Vec<(usize, usize)>,
        fail_next: bool,
    }

    impl FakeLoader {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                fail_next: false,
            }
        }

        fn fetch(&mut self, offset: usize, limit: usize) -> Result<Vec<Track>, AppError> {
            self.calls.push((offset, limit));
            if std::mem::take(&mut self.fail_next) {
                return Err(AppError::InvalidOperation("loader boom".to_string()));
            }
            Ok((0..limit).map(|i| projection_track(offset + i)).collect())
        }
    }

    #[test]
    fn test_first_refresh_loads_requested_windows_and_total() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        projection.request_window(0);
        projection.request_window(50);

        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 120, &mut |o, l| loader.fetch(o, l))
            .expect("first refresh loads");

        assert_eq!(projection.total(), 120);
        assert_eq!(projection.window(0).expect("window cached").len(), 50);
        assert_eq!(projection.window(50).expect("window cached").len(), 50);
        assert!(
            projection.window(100).is_none(),
            "unrequested windows stay unloaded"
        );
        assert_eq!(
            loader.calls,
            vec![(0, 50), (50, 50)],
            "each requested window is fetched once at the projection's window size"
        );
    }

    #[test]
    fn test_same_generation_refresh_serves_cache_without_refetching() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        projection.request_window(0);
        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 120, &mut |o, l| loader.fetch(o, l))
            .unwrap();

        projection.request_window(0);
        projection
            .refresh(1, 120, &mut |o, l| loader.fetch(o, l))
            .unwrap();

        assert_eq!(
            loader.calls.len(),
            1,
            "a fresh projection must serve cached rows without refetching"
        );
        assert_eq!(projection.window(0).expect("still cached").len(), 50);
    }

    #[test]
    fn test_bumped_generation_invalidates_all_windows() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        projection.request_window(0);
        projection.request_window(50);
        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 120, &mut |o, l| loader.fetch(o, l))
            .unwrap();
        let calls_after_load = loader.calls.len();

        // A committed mutation bumps the generation; every visible window
        // refetches even though the offsets are unchanged.
        projection.request_window(0);
        projection.request_window(50);
        projection
            .refresh(2, 120, &mut |o, l| loader.fetch(o, l))
            .unwrap();

        assert_eq!(
            loader.calls.len(),
            calls_after_load * 2,
            "generation bump invalidates every cached window"
        );
        assert_eq!(loader.calls.last(), Some(&(50, 50)));
    }

    #[test]
    fn test_key_change_invalidates_even_at_same_generation() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        projection.request_window(0);
        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 120, &mut |o, l| loader.fetch(o, l))
            .unwrap();

        // Retargeting the projection to a different query signature (the
        // search box changed) must drop the cache even at the same
        // generation.
        projection.set_key(ProjectionKey::Search("x".to_string()));
        assert!(
            projection.window(0).is_none(),
            "retargeted projections never serve rows from the old signature"
        );

        projection.request_window(0);
        projection
            .refresh(1, 3, &mut |o, l| loader.fetch(o, l))
            .unwrap();
        assert_eq!(
            loader.calls.last(),
            Some(&(0, 50)),
            "refetch happens despite unchanged generation"
        );
        assert_eq!(projection.total(), 3);
    }

    #[test]
    fn test_cache_is_bounded_fifo() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        for offset in [0, 50, 100, 150, 200, 250, 300, 350, 400] {
            projection.request_window(offset);
        }
        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 500, &mut |o, l| loader.fetch(o, l))
            .unwrap();

        assert_eq!(loader.calls.len(), 9, "all requested windows load");
        assert!(
            projection.window(0).is_none(),
            "the oldest window is evicted once the bound is exceeded"
        );
        assert!(
            projection.window(400).expect("newest window stays").len() == 50,
            "the newest window remains cached"
        );
    }

    #[test]
    fn test_loader_error_preserves_existing_cache() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        projection.request_window(0);
        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 120, &mut |o, l| loader.fetch(o, l))
            .unwrap();
        let cached_rows = projection.window(0).expect("cached").len();

        // The next refresh fails mid-flight; stale-but-present beats blank.
        loader.fail_next = true;
        projection.request_window(0);
        let err = projection.refresh(2, 120, &mut |o, l| loader.fetch(o, l));
        assert!(err.is_err(), "loader errors propagate");

        assert_eq!(
            projection.window(0).expect("prior cache survives").len(),
            cached_rows,
            "a failed refresh leaves the previous windows untouched"
        );
        assert!(
            !projection.is_fresh(2),
            "the failed generation still counts as stale"
        );
        assert!(
            projection.is_fresh(1),
            "the loaded generation is remembered"
        );
    }

    #[test]
    fn test_missing_window_fetches_only_that_window_when_fresh() {
        let mut projection = TrackListProjection::new(ProjectionKey::Flat);
        projection.request_window(0);
        let mut loader = FakeLoader::new();
        projection
            .refresh(1, 200, &mut |o, l| loader.fetch(o, l))
            .unwrap();
        let calls_after_load = loader.calls.len();

        // Scrolling brings one new window into view while staying fresh.
        projection.request_window(100);
        projection
            .refresh(1, 200, &mut |o, l| loader.fetch(o, l))
            .unwrap();

        assert_eq!(
            &loader.calls[calls_after_load..],
            &[(100, 50)],
            "only the missing window is fetched when fresh"
        );
    }

    // --- Browsing Projection: artist/album views over store queries ---------

    use riff::app::projection::BrowsingProjection;

    /// Counting fake loaders standing in for the three browsing queries:
    /// each level records its calls and returns deterministic canned data.
    struct FakeBrowsingStore {
        artists_loaded: usize,
        albums_loaded_for: Vec<String>,
        tracks_loaded_for: Vec<(String, String)>,
    }

    impl FakeBrowsingStore {
        fn new() -> Self {
            Self {
                artists_loaded: 0,
                albums_loaded_for: Vec::new(),
                tracks_loaded_for: Vec::new(),
            }
        }

        // The `Result` returns mirror the port signatures: these fakes are
        // only ever invoked inside loader closures handed to the projection.
        #[allow(clippy::unnecessary_wraps)]
        fn artists(&mut self) -> Result<Vec<Artist>, AppError> {
            self.artists_loaded += 1;
            Ok(vec![Artist {
                name: "Alpha".to_string(),
                albums: vec!["Alpha - One".to_string()],
            }])
        }

        #[allow(clippy::unnecessary_wraps)]
        fn artist_albums(&mut self, artist: &str) -> Result<Vec<Album>, AppError> {
            self.albums_loaded_for.push(artist.to_string());
            Ok(vec![Album {
                title: "One".to_string(),
                artist: artist.to_string(),
                tracks: vec![TrackId("f:\\a\\1.mp3".to_string())],
                year: Some(1999),
                genre: None,
            }])
        }

        #[allow(clippy::unnecessary_wraps)]
        fn album_tracks(&mut self, artist: &str, title: &str) -> Result<Vec<Track>, AppError> {
            self.tracks_loaded_for
                .push((artist.to_string(), title.to_string()));
            Ok(vec![crate::test_utils::create_test_track_with_metadata(
                "f:\\a\\1.mp3",
                "f:\\a\\1.mp3",
                artist,
                "One",
                title,
            )])
        }
    }

    #[test]
    fn test_browsing_projection_fetches_each_level_once_per_generation() {
        let mut projection = BrowsingProjection::new();
        let mut store = FakeBrowsingStore::new();

        // Every level queried twice at the same generation serves the cache.
        let _ = projection
            .artists(1, &mut || store.artists())
            .expect("artists load");
        let _ = projection
            .artists(1, &mut || store.artists())
            .expect("artists cached");
        let _ = projection
            .artist_albums(1, "Alpha", &mut |a| store.artist_albums(a))
            .expect("albums load");
        let _ = projection
            .artist_albums(1, "Alpha", &mut |a| store.artist_albums(a))
            .expect("albums cached");
        let _ = projection
            .album_tracks(1, "Alpha", "One", &mut |a, t| store.album_tracks(a, t))
            .expect("tracks load");
        let _ = projection
            .album_tracks(1, "Alpha", "One", &mut |a, t| store.album_tracks(a, t))
            .expect("tracks cached");

        assert_eq!(store.artists_loaded, 1, "artists fetch once per generation");
        assert_eq!(
            store.albums_loaded_for,
            vec!["Alpha".to_string()],
            "one artist's albums fetch once per generation"
        );
        assert_eq!(
            store.tracks_loaded_for,
            vec![("Alpha".to_string(), "One".to_string())],
            "one album's tracks fetch once per generation"
        );
    }

    #[test]
    fn test_browsing_projection_invalidates_everything_on_generation_bump() {
        let mut projection = BrowsingProjection::new();
        let mut store = FakeBrowsingStore::new();
        let _ = projection.artists(1, &mut || store.artists()).unwrap();
        let _ = projection
            .artist_albums(1, "Alpha", &mut |a| store.artist_albums(a))
            .unwrap();
        let _ = projection
            .album_tracks(1, "Alpha", "One", &mut |a, t| store.album_tracks(a, t))
            .unwrap();

        // A committed mutation bumps the generation; every level refetches.
        let artists = projection
            .artists(2, &mut || store.artists())
            .expect("artists reload");
        let albums = projection
            .artist_albums(2, "Alpha", &mut |a| store.artist_albums(a))
            .expect("albums reload");
        let tracks = projection
            .album_tracks(2, "Alpha", "One", &mut |a, t| store.album_tracks(a, t))
            .expect("tracks reload");

        assert_eq!(store.artists_loaded, 2);
        assert_eq!(store.albums_loaded_for.len(), 2);
        assert_eq!(store.tracks_loaded_for.len(), 2);
        assert_eq!(artists[0].name, "Alpha");
        assert_eq!(albums[0].title, "One");
        assert_eq!(tracks[0].id.0, "f:\\a\\1.mp3");
    }

    #[test]
    fn test_browsing_projection_error_keeps_stale_data_and_retries() {
        let mut projection = BrowsingProjection::new();
        let mut store = FakeBrowsingStore::new();
        let first = projection
            .artists(1, &mut || store.artists())
            .expect("first load");

        // The next load fails; the error propagates and the stale rows stay.
        let err = projection.artists(2, &mut || {
            Err(AppError::InvalidOperation("boom".to_string()))
        });
        assert!(err.is_err(), "loader errors propagate");
        let stale = projection
            .artists(2, &mut || store.artists())
            .expect("retry loads");
        assert_eq!(
            stale.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
            first.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
            "the retry re-fetches and matches the prior shape"
        );
        assert_eq!(store.artists_loaded, 2, "the failed attempt fetched once");
    }

    // --- Folder Projection: folder-tree views over store queries ------------

    use riff::app::projection::FolderProjection;

    /// Counting fake loaders standing in for the five folder queries: each
    /// level records its calls and returns deterministic canned data.
    struct FakeFolderStore {
        has_audio_probes: Vec<String>,
        search_probes: Vec<(String, String)>,
        tree_listings: Vec<String>,
        direct_listings: Vec<String>,
        children_listings: Vec<String>,
    }

    impl FakeFolderStore {
        fn new() -> Self {
            Self {
                has_audio_probes: Vec::new(),
                search_probes: Vec::new(),
                tree_listings: Vec::new(),
                direct_listings: Vec::new(),
                children_listings: Vec::new(),
            }
        }

        #[allow(clippy::unnecessary_wraps)]
        fn has_audio(&mut self, folder: &std::path::Path) -> Result<bool, AppError> {
            self.has_audio_probes
                .push(folder.to_string_lossy().into_owned());
            Ok(true)
        }

        #[allow(clippy::unnecessary_wraps)]
        fn has_search_match(
            &mut self,
            folder: &std::path::Path,
            query: &str,
        ) -> Result<bool, AppError> {
            self.search_probes
                .push((folder.to_string_lossy().into_owned(), query.to_string()));
            Ok(true)
        }

        #[allow(clippy::unnecessary_wraps)]
        fn tree_ids(&mut self, folder: &std::path::Path) -> Result<Vec<TrackId>, AppError> {
            self.tree_listings
                .push(folder.to_string_lossy().into_owned());
            Ok(vec![TrackId("f:\\lib\\a\\1.mp3".to_string())])
        }

        #[allow(clippy::unnecessary_wraps)]
        fn direct_tracks(&mut self, folder: &std::path::Path) -> Result<Vec<Track>, AppError> {
            self.direct_listings
                .push(folder.to_string_lossy().into_owned());
            Ok(vec![crate::test_utils::create_test_track(
                "f:\\lib\\1.mp3",
                "f:\\lib\\1.mp3",
            )])
        }

        #[allow(clippy::unnecessary_wraps)]
        fn children(&mut self, folder: &std::path::Path) -> Result<Vec<PathBuf>, AppError> {
            self.children_listings
                .push(folder.to_string_lossy().into_owned());
            Ok(vec![folder.join("child")])
        }
    }

    #[test]
    fn test_folder_projection_fetches_each_level_once_per_generation() {
        let mut projection = FolderProjection::new();
        let mut store = FakeFolderStore::new();
        let folder = std::path::Path::new("f:\\lib");

        for _ in 0..2 {
            let _ = projection
                .has_audio(1, folder, &mut |f| store.has_audio(f))
                .expect("has_audio");
            let _ = projection
                .has_search_match(1, folder, "q", &mut |f, q| store.has_search_match(f, q))
                .expect("search match");
            let _ = projection
                .subtree_ids(1, folder, &mut |f| store.tree_ids(f))
                .expect("tree ids");
            let _ = projection
                .direct_tracks(1, folder, &mut |f| store.direct_tracks(f))
                .expect("direct tracks");
            let _ = projection
                .children(1, folder, &mut |f| store.children(f))
                .expect("children");
        }

        assert_eq!(store.has_audio_probes.len(), 1);
        assert_eq!(store.search_probes.len(), 1);
        assert_eq!(store.tree_listings.len(), 1);
        assert_eq!(store.direct_listings.len(), 1);
        assert_eq!(store.children_listings.len(), 1);
    }

    #[test]
    fn test_folder_projection_invalidates_everything_on_generation_bump() {
        let mut projection = FolderProjection::new();
        let mut store = FakeFolderStore::new();
        let folder = std::path::Path::new("f:\\lib");
        let _ = projection
            .has_audio(1, folder, &mut |f| store.has_audio(f))
            .unwrap();

        // A committed mutation bumps the generation; every level refetches.
        let fresh = projection
            .has_audio(2, folder, &mut |f| store.has_audio(f))
            .expect("reload");
        assert!(fresh);
        assert_eq!(store.has_audio_probes.len(), 2);

        let ids = projection
            .subtree_ids(2, folder, &mut |f| store.tree_ids(f))
            .expect("tree reload");
        assert_eq!(ids.len(), 1);
        assert_eq!(store.tree_listings.len(), 1, "bump dropped the gen-1 cache");
    }

    // --- Smart Playlists Projection: read-only lists over store queries -----

    use riff::app::projection::SmartPlaylistsProjection;
    use riff::domain::SmartPlaylistKind;

    /// Counting fake loader standing in for the smart-playlist query.
    struct FakeSmartStore {
        calls: Vec<(SmartPlaylistKind, usize)>,
    }

    impl FakeSmartStore {
        fn new() -> Self {
            Self { calls: Vec::new() }
        }

        #[allow(clippy::unnecessary_wraps)]
        fn fetch(&mut self, kind: SmartPlaylistKind, limit: usize) -> Result<Vec<Track>, AppError> {
            self.calls.push((kind, limit));
            Ok(vec![crate::test_utils::create_test_track(
                "f:\\sm\\1.mp3",
                "f:\\sm\\1.mp3",
            )])
        }
    }

    #[test]
    fn test_smart_projection_serves_cache_within_generation() {
        let mut projection = SmartPlaylistsProjection::new();
        let mut store = FakeSmartStore::new();

        for _ in 0..2 {
            let tracks = projection
                .list(1, SmartPlaylistKind::MostPlayed, 50, &mut |k, l| {
                    store.fetch(k, l)
                })
                .expect("most played loads");
            assert_eq!(tracks.len(), 1);
        }
        assert_eq!(
            store.calls.len(),
            1,
            "a fresh generation serves cached rows without refetching"
        );

        // A different kind is an independent cache slot.
        let _ = projection
            .list(1, SmartPlaylistKind::LostGems, usize::MAX, &mut |k, l| {
                store.fetch(k, l)
            })
            .expect("lost gems loads");
        assert_eq!(store.calls.len(), 2);
    }

    #[test]
    fn test_smart_projection_refetches_after_generation_bump_and_limit_change() {
        let mut projection = SmartPlaylistsProjection::new();
        let mut store = FakeSmartStore::new();
        let _ = projection
            .list(1, SmartPlaylistKind::MostPlayed, 50, &mut |k, l| {
                store.fetch(k, l)
            })
            .unwrap();

        // A committed mutation bumps the generation: refetch.
        let _ = projection
            .list(2, SmartPlaylistKind::MostPlayed, 50, &mut |k, l| {
                store.fetch(k, l)
            })
            .unwrap();

        // A larger limit than the cache holds also refetches even when fresh.
        let _ = projection
            .list(2, SmartPlaylistKind::MostPlayed, 100, &mut |k, l| {
                store.fetch(k, l)
            })
            .unwrap();

        assert_eq!(
            store.calls,
            vec![
                (SmartPlaylistKind::MostPlayed, 50),
                (SmartPlaylistKind::MostPlayed, 50),
                (SmartPlaylistKind::MostPlayed, 100),
            ],
            "bumps and limit growth refetch; equal limits do not"
        );
    }

    #[test]
    fn test_smart_projection_error_propagates_and_retries() {
        let mut projection = SmartPlaylistsProjection::new();
        let mut store = FakeSmartStore::new();
        let err = projection.list(1, SmartPlaylistKind::LostGems, 10, &mut |_, _| {
            Err(AppError::InvalidOperation("boom".to_string()))
        });
        assert!(err.is_err(), "loader errors propagate");

        let tracks = projection
            .list(1, SmartPlaylistKind::LostGems, 10, &mut |k, l| {
                store.fetch(k, l)
            })
            .expect("retry loads");
        assert_eq!(tracks.len(), 1);
        assert_eq!(store.calls.len(), 1, "the failed attempt fetched nothing");
    }

    // --- Playlist drag-reorder math (Issue 12) ---------------------------------
    //
    // The pure move semantics behind the playlist view's drag-and-drop:
    // removing the dragged entry and reinserting it at the drop index, with
    // everything else shifting to close/open the gaps.

    use riff::app::playlist_manager::reorder_tracks;

    fn ids<const N: usize>(paths: [&str; N]) -> Vec<TrackId> {
        paths.iter().map(|p| TrackId(p.to_string())).collect()
    }

    #[test]
    fn test_reorder_tracks_moves_an_entry_down_between_others() {
        let tracks = ids(["a", "b", "c", "d"]);
        assert_eq!(
            reorder_tracks(&tracks, 0, 2),
            Some(ids(["b", "c", "a", "d"])),
            "dragging A onto C's slot shifts B and C up"
        );
    }

    #[test]
    fn test_reorder_tracks_moves_an_entry_up() {
        let tracks = ids(["a", "b", "c", "d"]);
        assert_eq!(
            reorder_tracks(&tracks, 3, 0),
            Some(ids(["d", "a", "b", "c"])),
            "dragging the last entry to the top shifts the rest down"
        );
    }

    #[test]
    fn test_reorder_tracks_adjacent_swap_and_noop() {
        let tracks = ids(["a", "b", "c"]);
        assert_eq!(
            reorder_tracks(&tracks, 1, 2),
            Some(ids(["a", "c", "b"])),
            "dropping on the next row swaps the pair"
        );
        assert_eq!(
            reorder_tracks(&tracks, 1, 1),
            None,
            "dropping an entry back onto itself is a no-op"
        );
    }

    #[test]
    fn test_reorder_tracks_rejects_out_of_bounds_indices() {
        let tracks = ids(["a", "b"]);
        assert_eq!(reorder_tracks(&tracks, 2, 0), None, "from out of range");
        assert_eq!(reorder_tracks(&tracks, 0, 2), None, "to out of range");
        assert_eq!(reorder_tracks(&[], 0, 0), None, "empty list has no rows");
    }
}
