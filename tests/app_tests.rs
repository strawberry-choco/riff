// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::CoverSource;
    use riff_backend::app::errors::StoreError;
    use riff_backend::app::playlist_manager;
    use riff_backend::app::traits::{MetadataWriter, TagEdit};
    use riff_library::app::errors::LibraryError;
    use riff_library::app::traits::{AudioFormatInfo, MetadataReader};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

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
    fn test_app_state_advanced_mode_defaults_to_false() {
        // Progressive disclosure (REQ-UI-006): the UI starts minimal; power
        // features stay hidden until the user opts in via the toggle.
        let state = LibrarySession::default();
        assert!(!state.ui_flags.advanced_mode);
    }

    #[test]
    fn test_app_state_high_contrast_defaults_to_false() {
        // Accessibility (REQ-UI-007): the high-contrast theme is opt-in; the
        // app starts with the regular light/dark palette.
        let state = LibrarySession::default();
        assert!(!state.ui_flags.high_contrast);
    }

    #[test]
    fn test_app_state_muted_defaults_to_false() {
        // Mute (REQ-UI-003-08): the app starts unmuted.
        let state = PlaybackSession::default();
        assert!(!state.muted);
    }

    #[test]
    fn test_app_state_replaygain_defaults_to_disabled() {
        // ReplayGain (Task 4.3) is opt-in.
        let state = PlaybackSession::default();
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
    // The engine's gapless DECISIONS are delegated to these pure helpers —
    // testing them IS testing that decision logic. The engine loop itself is
    // exercised through its ports in the `audio_engine_tests` module below.

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
            shuffle: queue.shuffle,
            repeat_one: queue.repeat_one,
            format_compatible: formats_compatible,
            has_successor,
        }
    }

    /// Shorthand for the queue part of [`gapless_conditions`].
    fn queue_conditions(shuffle: bool, repeat_one: bool) -> QueueConditions {
        QueueConditions {
            shuffle,
            repeat_one,
            has_successor: true,
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
        let mut state = PlaybackSession {
            current_volume: 0.7,
            ..PlaybackSession::default()
        };

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

    // --- playlists: entry validity (read-time, app layer) -------------------------
    //
    // Playlist persistence and CRUD moved behind the `PlaylistStore` port;
    // their behavior is tested against real SQL in `infra_tests.rs`. What
    // stays here is the read-time validity rule the UI plays by, applied on
    // top of the store's LEFT JOIN entries-with-validity query.

    #[test]
    fn test_track_is_valid_and_valid_tracks_filter_invalid_entries() {
        use riff_backend::app::store::PlaylistEntry;

        let root = tempfile::tempdir().unwrap();
        let real_file = root.path().join("real.mp3");
        std::fs::write(&real_file, b"not really audio").unwrap();
        let missing_file = root.path().join("missing.mp3");

        let make_entry = |path: &std::path::Path| PlaylistEntry {
            id: TrackId(path.to_string_lossy().into_owned()),
            track: Some(crate::test_utils::create_test_track(
                &path.to_string_lossy(),
                &path.to_string_lossy(),
            )),
            valid: true,
        };
        let real_entry = make_entry(&real_file);
        let missing_entry = make_entry(&missing_file);
        let dangling_entry = PlaylistEntry {
            id: TrackId("never scanned.flac".to_string()),
            track: None,
            valid: false,
        };

        // Valid: in the Library AND the file exists on disk.
        assert!(playlist_manager::track_is_valid(&real_entry));
        // Invalid: file gone on disk (though the Library still knows it), or
        // track never scanned (dangling reference).
        assert!(!playlist_manager::track_is_valid(&missing_entry));
        assert!(!playlist_manager::track_is_valid(&dangling_entry));

        // Loading a playlist keeps only playable entries, in playlist order.
        assert_eq!(
            playlist_manager::valid_tracks(&[missing_entry, real_entry, dangling_entry,]),
            vec![TrackId(real_file.to_string_lossy().into_owned())]
        );
    }

    // --- build_tracks with a mock MetadataReader ---------------------------------

    /// Minimal [`MetadataReader`](riff_backend::app::traits::MetadataReader) for
    /// exercising scan-side Track construction without real audio files.
    /// `fail` simulates unreadable/corrupt input.
    struct MockMetadataReader {
        fail: bool,
    }

    impl MetadataReader for MockMetadataReader {
        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, LibraryError> {
            Ok(CoverSource::None)
        }

        fn read_all(
            &self,
            path: &Path,
        ) -> Result<(TrackMetadata, Duration, CoverSource, AudioFormatInfo), LibraryError> {
            if self.fail {
                return Err(LibraryError::MetadataRead(format!(
                    "mock failure: {path:?}"
                )));
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
                Duration::from_secs(90),
                CoverSource::None,
                AudioFormatInfo {
                    sample_rate: 44_100,
                    channels: 2,
                },
            ))
        }
    }

    // --- build_tracks over a mock MetadataReader (scan-side Track construction) ----
    //
    // The former mirror's `scan_and_add_tracks` is gone; what survives is the
    // app-layer Track construction the store-backed scan flow commits.

    #[test]
    fn test_build_tracks_reads_metadata_and_audio_format() {
        let reader = MockMetadataReader { fail: false };
        let tracks = riff_backend::app::scan::build_tracks(
            vec![PathBuf::from("scan/a.mp3"), PathBuf::from("scan/b.mp3")],
            &reader,
        );

        assert_eq!(tracks.len(), 2, "every readable path becomes a Track");
        let scanned = &tracks[0];
        assert_eq!(scanned.id.0, "scan/a.mp3", "the id is the full file path");
        assert_eq!(scanned.file_path, PathBuf::from("scan/a.mp3"));
        // Metadata and audio-format info from the reader land on the track.
        assert_eq!(scanned.metadata.title.as_deref(), Some("a"));
        assert_eq!(scanned.metadata.artist.as_deref(), Some("Mock Artist"));
        assert_eq!(scanned.duration, Some(Duration::from_secs(90)));
        assert_eq!(scanned.sample_rate, Some(44_100));
        assert_eq!(scanned.channels, Some(2));
    }

    #[test]
    fn test_build_tracks_skips_unreadable_files() {
        let reader = MockMetadataReader { fail: true };
        let tracks = riff_backend::app::scan::build_tracks(
            vec![PathBuf::from("bad/a.mp3"), PathBuf::from("bad/b.mp3")],
            &reader,
        );
        assert!(
            tracks.is_empty(),
            "unreadable files are skipped, never aborting the scan"
        );
    }

    #[test]
    fn test_build_tracks_stamps_date_added_and_zero_plays() {
        let reader = MockMetadataReader { fail: false };
        let tracks =
            riff_backend::app::scan::build_tracks(vec![PathBuf::from("scan/fresh.mp3")], &reader);

        let track = &tracks[0];
        assert_eq!(track.play_count, 0);
        assert!(track.last_played.is_none());
        assert!(
            track.date_added.is_some(),
            "first-add time is stamped once at scan time (drives Recently Added)"
        );
    }

    #[test]
    fn test_scanner_yields_nothing_for_an_unavailable_path() {
        // An unavailable (missing or removed) library path must degrade to
        // "nothing scanned", not an error: the scanner skips the unreadable
        // root entry and yields no files for build_tracks to read.
        let temp_dir = tempfile::tempdir().unwrap();
        let missing = temp_dir.path().join("does-not-exist");
        assert!(!missing.exists());

        let scanner = AudioFileScanner::new(Arc::new(AtomicBool::new(false)));
        assert!(scanner.scan(&missing).is_empty());
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
        assert!(matches!(
            err,
            riff_backend::app::errors::LibraryError::MetadataWrite(_)
        ));
        assert!(err.to_string().contains("Failed to write tags"));
        assert!(writer.recorded().is_empty());
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

    // --- SettingsStore port orchestration (mock-driven) -------------------------

    #[test]
    fn test_mock_settings_store_drives_hydration_and_persistence() {
        use riff_backend::app::store::SettingsStore;
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
        mock.save_scalars(&riff_backend::app::state::ScalarSettings {
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
        assert!(
            mock.save_scalars(&riff_backend::app::state::ScalarSettings::default())
                .is_err()
        );
        assert!(mock.save_library_paths(&[]).is_err());
        assert!(mock.save_watch_states(&HashMap::new()).is_err());
    }

    // --- Session Views facade: bounded views with generation invalidation ---
    //
    // The UI's single read seam over the Application Store (ADR 0002). Every
    // scenario drives `SessionViews` through its public interface: a shared
    // [`MockLibraryQueryStore`] stands in for the store port and a real
    // `StoreGeneration` handle plays the mutation adapter's bumps.

    use crate::mocks::{LibraryQueryCall, MockLibraryQueryStore};
    use riff_backend::app::store::{LibraryQueryStore, StoreGeneration};
    use riff_backend::app::views::SessionViews;

    /// Deterministic fixture row for a window slot.
    fn projection_track(n: usize) -> Track {
        crate::test_utils::create_test_track(&n.to_string(), &format!("f:\\{n}.mp3"))
    }

    /// A flat fixture library of `rows` deterministic tracks.
    fn flat_library(rows: usize) -> Vec<Track> {
        (0..rows).map(projection_track).collect()
    }

    /// [`LibraryQueryStore`] view over one shared [`MockLibraryQueryStore`]
    /// behind a mutex: the facade takes ownership of the port, so the test
    /// keeps the other handle for assertions and post-wire configuration
    /// changes (failure flags, mutated canned data).
    #[derive(Clone)]
    struct SharedMock(Arc<Mutex<MockLibraryQueryStore>>);

    impl LibraryQueryStore for SharedMock {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            self.0.lock().unwrap().get_track(id)
        }

        fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().tracks_window(offset, limit)
        }

        fn track_count(&self) -> Result<usize, StoreError> {
            self.0.lock().unwrap().track_count()
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            self.0.lock().unwrap().all_track_ids()
        }

        fn search_window(
            &self,
            query: &str,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().search_window(query, offset, limit)
        }

        fn search_count(&self, query: &str) -> Result<usize, StoreError> {
            self.0.lock().unwrap().search_count(query)
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
        ) -> Result<Vec<TrackId>, StoreError> {
            self.0.lock().unwrap().track_ids_in_folder_tree(folder)
        }

        fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().tracks_in_folder(folder)
        }

        fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, StoreError> {
            self.0.lock().unwrap().subdirs_with_audio(folder)
        }

        fn smart_playlist(
            &self,
            kind: SmartPlaylistKind,
            limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            self.0.lock().unwrap().smart_playlist(kind, limit)
        }
    }

    /// Test-side handle to the shared mock: locks on every access so
    /// assertions read recordings and configuration tweaks mutate the canned
    /// data behind the facade's back.
    struct MockHandle(Arc<Mutex<MockLibraryQueryStore>>);

    impl MockHandle {
        fn lock(&self) -> std::sync::MutexGuard<'_, MockLibraryQueryStore> {
            self.0.lock().unwrap()
        }

        /// Every bounded-window fetch as `(offset, limit)` pairs.
        fn window_calls(&self) -> Vec<(usize, usize)> {
            self.lock().window_calls()
        }

        /// Every `get_track` id, in call order.
        fn get_track_calls(&self) -> Vec<TrackId> {
            self.lock().get_track_calls()
        }

        /// How often one exact query shape fired.
        fn count_of(&self, call: &LibraryQueryCall) -> usize {
            self.lock().count_of(call)
        }

        /// Snapshot of every recorded query, in call order.
        fn calls(&self) -> Vec<LibraryQueryCall> {
            self.lock().calls()
        }
    }

    /// Wire a facade to `mock`; returns the facade, the shared mock handle
    /// for assertions and configuration, and the generation handle that
    /// stands in for the mutation adapter's bumps.
    fn wire(mock: MockLibraryQueryStore) -> (SessionViews, MockHandle, StoreGeneration) {
        let mock = Arc::new(Mutex::new(mock));
        let generation = StoreGeneration::new();
        // The playlist ports are wired but unused by these Library-view
        // tests; an empty stub stands in for the Playlists section.
        let views = SessionViews::new(
            Box::new(SharedMock(Arc::clone(&mock))),
            Box::new(crate::mocks::MockPlaylistStore::default()),
            generation.clone(),
            StoreGeneration::new(),
        );
        (views, MockHandle(mock), generation)
    }

    #[test]
    fn test_first_track_list_load_fetches_the_visible_windows_and_total() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            flat: flat_library(120),
            ..Default::default()
        });

        let page = views.track_list("", 0);
        assert_eq!(page.total, 120);
        assert_eq!(page.start, 0);
        assert_eq!(page.rows.len(), 50);
        assert_eq!(page.rows[0].id.0, "0");

        let page = views.track_list("", 50);
        assert_eq!(page.start, 50);
        assert_eq!(page.rows.len(), 50);
        assert_eq!(page.rows[49].id.0, "99");

        assert_eq!(
            mock.window_calls(),
            vec![(0, 50), (50, 50)],
            "each requested window is fetched once at the projection's window size"
        );
    }

    #[test]
    fn test_cache_is_bounded_fifo() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            flat: flat_library(500),
            ..Default::default()
        });

        for offset in [0, 50, 100, 150, 200, 250, 300, 350, 400] {
            views.track_list("", offset);
        }

        assert_eq!(mock.window_calls().len(), 9, "all requested windows load");

        // Asking for the oldest window again must fetch it anew: it was
        // evicted once the bound was exceeded.
        views.track_list("", 0);
        assert_eq!(
            mock.window_calls().last(),
            Some(&(0, 50)),
            "the oldest window was evicted once the bound was exceeded"
        );

        // The newest window remains cached.
        let calls_before = mock.window_calls().len();
        views.track_list("", 400);
        assert_eq!(
            mock.window_calls().len(),
            calls_before,
            "the newest window remains cached"
        );
    }

    #[test]
    fn test_scrolling_fetches_only_the_missing_window_when_fresh() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            flat: flat_library(200),
            ..Default::default()
        });
        views.track_list("", 0);
        let calls_after_load = mock.window_calls().len();

        // Scrolling brings one new window into view while staying fresh.
        views.track_list("", 100);

        assert_eq!(
            &mock.window_calls()[calls_after_load..],
            &[(100, 50)],
            "only the missing window is fetched when fresh"
        );
    }

    #[test]
    fn test_mutation_recounts_so_total_and_rows_agree() {
        let (mut views, mock, generation) = wire(MockLibraryQueryStore {
            flat: flat_library(120),
            ..Default::default()
        });
        assert_eq!(views.track_list("", 0).total, 120);

        // A committed mutation adds ten tracks and bumps the generation.
        // The invalidated frame must recount instead of trusting the stale
        // count, so the returned total agrees with the refreshed rows (the
        // torn-count guarantee the facade owns).
        {
            let mut mock = mock.lock();
            for n in 120..130 {
                mock.flat.push(projection_track(n));
            }
        }
        generation.bump();

        let page = views.track_list("", 0);
        assert_eq!(page.total, 130, "the invalidated frame recounts");
        assert_eq!(page.rows.len(), 50);
        assert_eq!(
            page.rows[49].id.0, "49",
            "rows come back refreshed alongside the recounted total"
        );
        assert!(
            mock.count_of(&LibraryQueryCall::TrackCount) >= 2,
            "the recount issued a fresh COUNT query"
        );
    }

    #[test]
    fn test_search_gate_reports_whether_a_query_matches_anything() {
        let (views, mock, _gen) = wire(MockLibraryQueryStore {
            search: flat_library(2),
            matching_searches: vec!["q".to_string()],
            ..Default::default()
        });

        assert!(views.search_has_matches("q"));
        assert!(
            !views.search_has_matches("nothing"),
            "an empty result set gates rendering off"
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::SearchCount),
            2,
            "one store count per gate check"
        );
    }

    // --- Browsing views: artist/album levels over store queries --------------

    /// Canned browsing fixture: one artist with one album.
    fn browsing_fixtures() -> (Vec<Artist>, Vec<Album>, Vec<Track>) {
        (
            vec![Artist {
                name: "Alpha".to_string(),
                albums: vec!["Alpha - One".to_string()],
            }],
            vec![Album {
                title: "One".to_string(),
                artist: "Alpha".to_string(),
                tracks: vec![TrackId("f:\\a\\1.mp3".to_string())],
                year: Some(1999),
                genre: None,
            }],
            vec![crate::test_utils::create_test_track_with_metadata(
                "f:\\a\\1.mp3",
                "f:\\a\\1.mp3",
                "Alpha",
                "One",
                "Song",
            )],
        )
    }

    #[test]
    fn test_browsing_views_fetch_each_level_once_per_generation() {
        let (artists, albums, tracks) = browsing_fixtures();
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            artists,
            albums,
            album_tracks: tracks,
            ..Default::default()
        });

        // Every level queried twice at the same generation serves the cache.
        assert_eq!(views.artists()[0].name, "Alpha");
        assert_eq!(views.artists().len(), 1);
        assert_eq!(views.artist_albums("Alpha")[0].title, "One");
        assert_eq!(views.artist_albums("Alpha").len(), 1);
        assert_eq!(views.album_tracks("Alpha", "One").len(), 1);
        assert_eq!(views.album_tracks("Alpha", "One").len(), 1);

        assert_eq!(
            mock.count_of(&LibraryQueryCall::AllArtists),
            1,
            "artists fetch once per generation"
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::ArtistAlbums("Alpha".to_string())),
            1,
            "one artist's albums fetch once per generation"
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::AlbumTracks(
                "Alpha".to_string(),
                "One".to_string()
            )),
            1,
            "one album's tracks fetch once per generation"
        );
    }

    // --- Folder views: five folder query shapes over store queries -----------

    #[test]
    fn test_folder_views_fetch_each_level_once_per_generation() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            folder_tree_ids: vec![TrackId("f:\\lib\\a\\1.mp3".to_string())],
            folder_direct_tracks: vec![crate::test_utils::create_test_track(
                "f:\\lib\\1.mp3",
                "f:\\lib\\1.mp3",
            )],
            folder_children: vec![PathBuf::from("f:\\lib\\child")],
            ..Default::default()
        });
        let folder = Path::new("f:\\lib");

        for _ in 0..2 {
            assert!(views.folder_has_audio(folder));
            assert!(views.folder_search_match(folder, "q"));
            assert_eq!(views.folder_subtree_ids(folder).len(), 1);
            assert_eq!(views.folder_direct_tracks(folder).len(), 1);
            assert_eq!(
                views.folder_children(folder).as_ref(),
                [PathBuf::from("f:\\lib\\child")]
            );
        }

        assert_eq!(
            mock.count_of(&LibraryQueryCall::FolderHasAudio(folder.to_path_buf())),
            1
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::FolderHasSearchMatch(
                folder.to_path_buf(),
                "q".to_string()
            )),
            1
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::TrackIdsInFolderTree(
                folder.to_path_buf()
            )),
            1
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::TracksInFolder(folder.to_path_buf())),
            1
        );
        assert_eq!(
            mock.count_of(&LibraryQueryCall::SubdirsWithAudio(folder.to_path_buf())),
            1
        );
    }

    // --- Smart playlist views: read-only lists over store queries ------------

    #[test]
    fn test_smart_list_refetches_after_generation_bump_and_limit_increase() {
        let (mut views, mock, generation) = wire(MockLibraryQueryStore {
            smart: vec![crate::test_utils::create_test_track(
                "f:\\sm\\1.mp3",
                "f:\\sm\\1.mp3",
            )],
            ..Default::default()
        });
        let _ = views.smart_list(SmartPlaylistKind::MostPlayed, 50);

        // A committed mutation bumps the generation: refetch.
        generation.bump();
        let _ = views.smart_list(SmartPlaylistKind::MostPlayed, 50);

        // A larger limit than the cache holds also refetches even when fresh.
        let _ = views.smart_list(SmartPlaylistKind::MostPlayed, 100);

        assert_eq!(
            mock.calls(),
            vec![
                LibraryQueryCall::SmartPlaylist(SmartPlaylistKind::MostPlayed, 50),
                LibraryQueryCall::SmartPlaylist(SmartPlaylistKind::MostPlayed, 50),
                LibraryQueryCall::SmartPlaylist(SmartPlaylistKind::MostPlayed, 100),
            ],
            "bumps and limit growth refetch; equal limits do not"
        );
    }

    // --- Playback-side reads: current track + Up Next window -----------------
    //
    // Serves the window title, the playerbar cover, the Now Playing stage,
    // and the track-details panel off cached store rows. Invalidated by
    // generation bumps AND Playback Queue changes (a TrackChanged advance,
    // Next/Previous/PlayNext/AddToQueue).

    /// A four-track queue playing the first entry.
    fn playback_queue() -> PlaybackQueue {
        let mut queue = PlaybackQueue::default();
        for i in 1..=4 {
            queue.append(TrackId(format!("t{i}.mp3")));
        }
        queue.current_index = Some(0);
        queue
    }

    /// The matching canned library: four tagged tracks keyed by id.
    fn playback_library() -> HashMap<TrackId, Track> {
        (1..=4)
            .map(|i| {
                let track = crate::test_utils::create_test_track_with_metadata(
                    &format!("t{i}.mp3"),
                    &format!("music/t{i}.mp3"),
                    "Artist",
                    &format!("Song {i}"),
                    "Album",
                );
                (track.id.clone(), track)
            })
            .collect()
    }

    #[test]
    fn test_playback_slots_load_current_and_up_next_once_per_inputs() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            library: playback_library(),
            ..Default::default()
        });
        let queue = playback_queue();

        for _ in 0..2 {
            views.sync_playback(&queue, 5);
        }

        assert_eq!(
            views.playback_current().map(|t| t.id.0.as_str()),
            Some("t1.mp3")
        );
        let up_next: Vec<&str> = views
            .playback_up_next()
            .iter()
            .map(|t| t.id.0.as_str())
            .collect();
        assert_eq!(
            up_next,
            vec!["t2.mp3", "t3.mp3", "t4.mp3"],
            "Up Next lists the tracks after the current one, in queue order"
        );
        assert_eq!(
            mock.get_track_calls().len(),
            4,
            "one load per distinct id (current + window); the fresh frame refetches nothing"
        );
    }

    #[test]
    fn test_playback_slots_refetch_when_the_queue_advances() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            library: playback_library(),
            ..Default::default()
        });
        let mut queue = playback_queue();
        views.sync_playback(&queue, 5);
        let calls_after_load = mock.get_track_calls().len();

        // A TrackChanged advance moves the queue: same generation, but the
        // stamp moved, so the slots reload.
        queue.advance();
        views.sync_playback(&queue, 5);

        assert!(
            mock.get_track_calls().len() > calls_after_load,
            "a queue change invalidates the playback slots even at the same generation"
        );
        assert_eq!(
            views.playback_current().map(|t| t.id.0.as_str()),
            Some("t2.mp3")
        );
    }

    #[test]
    fn test_playback_slots_follow_the_queue_shuffle_order() {
        let (mut views, _mock, _gen) = wire(MockLibraryQueryStore {
            library: playback_library(),
            ..Default::default()
        });
        let mut queue = playback_queue();
        queue.shuffle = true;
        // A hand-seeded shuffle order (indices into tracks): t4 then t2 —
        // the window must mirror the QUEUE's order, not the append order.
        queue.shuffled_indices = std::collections::VecDeque::from(vec![3, 1]);

        views.sync_playback(&queue, 5);

        let up_next: Vec<&str> = views
            .playback_up_next()
            .iter()
            .map(|t| t.id.0.as_str())
            .collect();
        assert_eq!(up_next, vec!["t4.mp3", "t2.mp3"]);
    }

    #[test]
    fn test_playback_slots_skip_ids_missing_from_the_store() {
        let mut library = playback_library();
        library.remove(&TrackId("t3.mp3".to_string()));
        let (mut views, _mock, _gen) = wire(MockLibraryQueryStore {
            library,
            ..Default::default()
        });
        let queue = playback_queue();

        views.sync_playback(&queue, 5);

        let up_next: Vec<&str> = views
            .playback_up_next()
            .iter()
            .map(|t| t.id.0.as_str())
            .collect();
        assert_eq!(
            up_next,
            vec!["t2.mp3", "t4.mp3"],
            "entries whose files left the library are skipped, not rendered blank"
        );
    }

    #[test]
    fn test_playback_slots_empty_queue_load_nothing() {
        let (mut views, mock, _gen) = wire(MockLibraryQueryStore {
            library: playback_library(),
            ..Default::default()
        });
        let queue = PlaybackQueue::default();

        views.sync_playback(&queue, 5);

        assert!(views.playback_current().is_none());
        assert!(views.playback_up_next().is_empty());
        assert!(
            mock.get_track_calls().is_empty(),
            "nothing to resolve, nothing asked"
        );
    }

    #[test]
    fn test_selected_track_caches_until_selection_or_generation_moves() {
        let (mut views, mock, generation) = wire(MockLibraryQueryStore {
            library: playback_library(),
            ..Default::default()
        });
        let t1 = TrackId("t1.mp3".to_string());

        for _ in 0..2 {
            assert!(views.selected_track(&t1).is_some(), "selection resolves");
        }
        assert_eq!(
            mock.get_track_calls().len(),
            1,
            "an unchanged selection at an unchanged generation never requeries"
        );

        // A different selection refetches…
        let other = TrackId("t2.mp3".to_string());
        let _ = views.selected_track(&other);
        assert_eq!(mock.get_track_calls().len(), 2);

        // …and so does the same selection after a committed mutation.
        generation.bump();
        let _ = views.selected_track(&other);
        assert_eq!(mock.get_track_calls().len(), 3);

        // An absent track caches its negative result too.
        let missing = TrackId("gone.mp3".to_string());
        for _ in 0..2 {
            assert!(views.selected_track(&missing).is_none(), "absence resolves");
        }
        assert_eq!(
            mock.get_track_calls().len(),
            4,
            "a dangling selection is asked once"
        );
    }

    // --- Playlist drag-reorder math (Issue 12) ---------------------------------
    //
    // The pure move semantics behind the playlist view's drag-and-drop:
    // removing the dragged entry and reinserting it at the drop index, with
    // everything else shifting to close/open the gaps.

    use riff_backend::app::playlist_manager::reorder_tracks;

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

// --- Library Scan Service tests ----------------------------------------------
//
// The extracted scan flow (`src/app/scan_service.rs`) is exercised end to end
// through its public seam (ADR 0006): real tempdir fixtures with dummy
// audio-extension files, the canned metadata reader below, and a REAL SQLite
// Application Store in a scratch dir wired through the same infra adapters the
// composition root uses. A helper thread runs `ScanWorker::run()`; every wait
// spins on the public interface only, bounded by TIMEOUT so a wedged worker
// fails an assertion instead of hanging CI.
mod scan_service_tests {
    use super::*;
    use riff_backend::app::errors::StoreError;
    use riff_backend::app::scan_service::{ScanOutcome, ScanService, Scans};
    use riff_backend::app::store::{LibraryMutationStore, LibraryQueryStore};
    use riff_backend::domain::CoverSource;
    use riff_backend::infra::store::SqliteStore;
    use riff_library::app::errors::LibraryError;
    use riff_library::app::traits::{AudioFormatInfo, MetadataReader};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant, SystemTime};

    /// How long any single wait on the worker may take before the test
    /// declares it wedged.
    const TIMEOUT: Duration = Duration::from_secs(10);

    // --- Fixtures --------------------------------------------------------------

    /// Canned [`MetadataReader`] for scan fixtures: derives title/artist/
    /// album from each file's path (bytes on disk are irrelevant). When
    /// `gate` is `Some`, `read_all` calls beyond `permit_first` BLOCK until
    /// the flag flips — lets the cancel test freeze the worker mid-scan
    /// deterministically, after the first batch's Progress is observable.
    struct FixtureReader {
        permit_first: usize,
        served: AtomicUsize,
        gate: Option<std::sync::Arc<AtomicBool>>,
    }

    impl FixtureReader {
        /// A reader that serves every call immediately.
        fn open() -> Self {
            Self {
                permit_first: usize::MAX,
                served: AtomicUsize::new(0),
                gate: None,
            }
        }

        /// A reader whose first `permit_first` reads pass and every later
        /// read blocks until the returned flag is set.
        fn gated_after(permit_first: usize) -> (Self, std::sync::Arc<AtomicBool>) {
            let gate = std::sync::Arc::new(AtomicBool::new(false));
            (
                Self {
                    permit_first,
                    served: AtomicUsize::new(0),
                    gate: Some(std::sync::Arc::clone(&gate)),
                },
                gate,
            )
        }

        fn canned_read(
            &self,
            path: &Path,
        ) -> (TrackMetadata, Duration, CoverSource, AudioFormatInfo) {
            let n = self.served.fetch_add(1, Ordering::SeqCst);
            if n >= self.permit_first {
                let gate = self.gate.as_ref().expect("gated reader has a gate");
                while !gate.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
            (
                TrackMetadata {
                    title: Some(title),
                    artist: Some("Scan Artist".to_string()),
                    album: Some("Scan Album".to_string()),
                    ..Default::default()
                },
                Duration::from_secs(90),
                CoverSource::None,
                AudioFormatInfo {
                    sample_rate: 44_100,
                    channels: 2,
                },
            )
        }
    }

    impl MetadataReader for FixtureReader {
        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, LibraryError> {
            Ok(CoverSource::None)
        }

        fn read_all(
            &self,
            path: &Path,
        ) -> Result<(TrackMetadata, Duration, CoverSource, AudioFormatInfo), LibraryError> {
            Ok(self.canned_read(path))
        }
    }

    /// One scratch Application Store (real `SQLite` in a tempdir) plus the two
    /// Library ports wired over its shared connection, mirroring the
    /// composition root's wiring.
    struct ScratchStore {
        /// Keeps the database file alive for the whole test.
        _dir: tempfile::TempDir,
        mutations: SqliteStore,
        queries: SqliteStore,
    }

    impl ScratchStore {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("riff.sqlite3");
            let (changes_tx, _changes_rx) =
                crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
            let store = SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("fresh store must open and migrate");
            let mutations = store.clone();
            let queries = store;
            Self {
                _dir: dir,
                mutations,
                queries,
            }
        }

        /// Rows currently committed in the Library collection, read through
        /// the query port.
        fn track_count(&self) -> usize {
            self.queries.track_count().expect("count reads")
        }
    }

    /// Create `n` dummy audio-extension files under a fresh tempdir and
    /// return the directory plus the root to scan.
    fn seed_audio_dir(name: &str, n: usize) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..n {
            std::fs::write(root.join(format!("song_{i:03}.mp3")), b"not really audio").unwrap();
        }
        (dir, root)
    }

    /// Wire a real service/worker pair over the given ports and run the
    /// blocking worker on its own thread, exactly like the composition root
    /// (ADR 0006). The walk closure binds a real [`AudioFileScanner`] to the
    /// same cancel flag the service cancels through.
    fn spawn_service(reader: FixtureReader, scratch: &ScratchStore) -> ScanService {
        let cancel_flag = std::sync::Arc::new(AtomicBool::new(false));
        let scanner = AudioFileScanner::new(cancel_flag.clone());
        let (service, worker) = ScanService::new(
            Box::new(reader),
            Box::new(scratch.queries.clone()),
            Box::new(scratch.mutations.clone()),
            cancel_flag,
            move |path| scanner.scan(path),
        );
        std::thread::spawn(move || worker.run());
        service
    }

    /// Poll until `pred` holds over the accumulated outcome stream, or panic
    /// with everything that did arrive. Spins on the public interface only.
    fn poll_until(service: &dyn Scans, pred: impl Fn(&[ScanOutcome]) -> bool) -> Vec<ScanOutcome> {
        let start = Instant::now();
        let mut outcomes = Vec::new();
        while start.elapsed() < TIMEOUT {
            outcomes.extend(service.poll());
            if pred(&outcomes) {
                return outcomes;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("condition not met within {TIMEOUT:?}; outcomes so far: {outcomes:?}");
    }

    /// Poll until the `Complete` outcome for `root` arrives.
    fn poll_until_complete(service: &dyn Scans, root: &Path) -> Vec<ScanOutcome> {
        poll_until(service, |outcomes| {
            outcomes
                .iter()
                .any(|o| matches!(o, ScanOutcome::Complete { path, .. } if path.as_path() == root))
        })
    }

    // --- Failure-injection stubs -------------------------------------------------

    /// [`LibraryQueryStore`] whose `get_track` ALWAYS fails — the fail-open
    /// trigger. Every other query answers empty: the scan flow issues only
    /// `get_track`.
    struct FailingFreshnessQueries;

    impl LibraryQueryStore for FailingFreshnessQueries {
        fn get_track(&self, _id: &TrackId) -> Result<Option<Track>, StoreError> {
            Err(StoreError::InvalidOperation(
                "freshness probe boom".to_string(),
            ))
        }

        fn tracks_window(&self, _offset: usize, _limit: usize) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn track_count(&self) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }

        fn search_window(
            &self,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn search_count(&self, _query: &str) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn all_artists(&self) -> Result<Vec<crate::domain::Artist>, StoreError> {
            Ok(Vec::new())
        }

        fn artist_albums(&self, _artist: &str) -> Result<Vec<crate::domain::Album>, StoreError> {
            Ok(Vec::new())
        }

        fn album_tracks(
            &self,
            _album_artist: &str,
            _album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn folder_has_audio(&self, _folder: &Path) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn folder_has_search_match(
            &self,
            _folder: &Path,
            _query: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn track_ids_in_folder_tree(&self, _folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }

        fn tracks_in_folder(&self, _folder: &Path) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn subdirs_with_audio(&self, _folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            Ok(Vec::new())
        }

        fn smart_playlist(
            &self,
            _kind: crate::domain::SmartPlaylistKind,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }
    }

    /// [`LibraryMutationStore`] whose scan batches always fail to commit —
    /// simulates a broken store so the `Failed` outcome path is exercised.
    struct FailingCommitMutations;

    impl LibraryMutationStore for FailingCommitMutations {
        fn apply_scan_batch(&mut self, _tracks: &[Track]) -> Result<usize, StoreError> {
            Err(StoreError::InvalidOperation(
                "scan batch commit boom".to_string(),
            ))
        }

        fn record_track_played(
            &mut self,
            _id: &TrackId,
            _played_at: SystemTime,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn apply_tag_refresh(&mut self, _track: &Track) -> Result<(), StoreError> {
            Ok(())
        }

        fn remove_library_path(&mut self, _root: &Path) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn clear_library(&mut self) -> Result<usize, StoreError> {
            Ok(0)
        }
    }

    // --- Tests -------------------------------------------------------------------

    #[test]
    fn test_cancel_mid_scan_keeps_committed_batches_and_never_completes() {
        let scratch = ScratchStore::new();
        let (_dir, root) = seed_audio_dir("cancel-me", 25);
        // Reads 1..=10 pass (batch one); read #11 blocks until the gate
        // opens, freezing the worker inside the SECOND batch.
        let (reader, gate) = FixtureReader::gated_after(10);
        let scans = spawn_service(reader, &scratch);

        scans.request(root.clone());

        // Wait until the FIRST batch's Progress is observable: by then batch
        // one is durably committed (the commit precedes the Progress send).
        let first = poll_until(&scans, |outcomes| {
            outcomes.iter().any(|o| {
                matches!(
                    o,
                    ScanOutcome::Progress {
                        files_found: 10,
                        ..
                    }
                )
            })
        });
        assert_eq!(
            first,
            vec![ScanOutcome::Progress {
                path: root.clone(),
                files_found: 10
            }],
            "exactly the first batch's progress before cancel"
        );

        // Cancel while the second batch is blocked mid-read, then release it.
        scans.cancel();
        gate.store(true, Ordering::SeqCst);

        // Quiesce: the worker finishes the in-flight batch, sees the flag
        // BETWEEN batches, and stops without publishing anything further.
        let start = Instant::now();
        while scans.is_scanning(&root) && start.elapsed() < TIMEOUT {
            std::thread::sleep(Duration::from_millis(2));
        }
        let rest = scans.poll();

        assert!(
            !rest
                .iter()
                .any(|o| matches!(o, ScanOutcome::Complete { .. })),
            "a cancelled scan must never publish Complete: {rest:?}"
        );
        assert!(
            !rest.iter().any(|o| matches!(o, ScanOutcome::Failed { .. })),
            "cancellation is not failure: {rest:?}"
        );
        assert!(!scans.is_scanning(&root), "the cancelled scan ended");

        // Committed batches survive the cancel: batch one always, batch two
        // too unless the cancel landed between the batches (either way the
        // third batch never ran).
        let committed = scratch.track_count();
        assert!(
            (10..25).contains(&committed),
            "committed batches survive cancel, got {committed}"
        );
    }

    #[test]
    fn test_freshness_check_failure_fails_open_and_still_commits() {
        let scratch = ScratchStore::new();
        let (_dir, root) = seed_audio_dir("fail-open", 3);

        let cancel_flag = std::sync::Arc::new(AtomicBool::new(false));
        let scanner = AudioFileScanner::new(cancel_flag.clone());
        let (scans, worker) = ScanService::new(
            Box::new(FixtureReader::open()),
            Box::new(FailingFreshnessQueries),
            Box::new(scratch.mutations.clone()),
            cancel_flag,
            move |path| scanner.scan(path),
        );
        std::thread::spawn(move || worker.run());

        scans.request(root.clone());
        let outcomes = poll_until_complete(&scans, &root);

        assert_eq!(
            scratch.track_count(),
            3,
            "the scan committed even though EVERY freshness query failed"
        );
        assert!(
            outcomes
                .iter()
                .all(|o| !matches!(o, ScanOutcome::Failed { .. })),
            "a fail-open scan is not a failure: {outcomes:?}"
        );
    }

    #[test]
    fn test_rescan_through_the_service_is_idempotent_and_keeps_play_history() {
        let mut scratch = ScratchStore::new();
        let (_dir, root) = seed_audio_dir("rescan", 3);
        let scans = spawn_service(FixtureReader::open(), &scratch);

        scans.request(root.clone());
        poll_until_complete(&scans, &root);
        assert_eq!(scratch.track_count(), 3, "first scan indexed every file");

        // Play one track, then rescan the SAME directory through the service.
        let played_id = TrackId::from_path(&root.join("song_000.mp3"));
        scratch
            .mutations
            .record_track_played(&played_id, SystemTime::now())
            .expect("play records");

        scans.request(root.clone());
        poll_until_complete(&scans, &root);

        // No duplicate rows, and the play history survived the upserts.
        assert_eq!(scratch.track_count(), 3, "rescan must not duplicate rows");
        let played = scratch
            .queries
            .get_track(&played_id)
            .expect("read works")
            .expect("track still indexed");
        assert_eq!(played.play_count, 1, "rescan preserves play history");
        assert!(
            played.last_played.is_some(),
            "last_played survives the rescan"
        );
    }

    #[test]
    fn test_outcome_stream_reports_progress_cadence_then_complete_total() {
        let scratch = ScratchStore::new();
        // 23 files over the ~10-track batch size: chunks of 10 / 10 / 3.
        let (_dir, root) = seed_audio_dir("cadence", 23);
        let scans = spawn_service(FixtureReader::open(), &scratch);

        scans.request(root.clone());
        let outcomes = poll_until_complete(&scans, &root);

        let progress: Vec<usize> = outcomes
            .iter()
            .filter_map(|o| match o {
                ScanOutcome::Progress { files_found, .. } => Some(*files_found),
                _ => None,
            })
            .collect();
        assert_eq!(
            progress,
            vec![10, 20, 23],
            "one cumulative Progress per committed batch"
        );
        assert_eq!(
            outcomes.len(),
            4,
            "three Progress outcomes then one Complete"
        );
        assert_eq!(
            outcomes.last(),
            Some(&ScanOutcome::Complete {
                path: root.clone(),
                total_files: 23
            }),
            "Complete carries the exact discovered total"
        );
    }

    #[test]
    fn test_commit_failure_surfaces_as_failed_outcome() {
        let (_dir, root) = seed_audio_dir("broken-commit", 12);

        let cancel_flag = std::sync::Arc::new(AtomicBool::new(false));
        let scanner = AudioFileScanner::new(cancel_flag.clone());
        let (scans, worker) = ScanService::new(
            Box::new(FixtureReader::open()),
            Box::new(FailingFreshnessQueries),
            Box::new(FailingCommitMutations),
            cancel_flag,
            move |path| scanner.scan(path),
        );
        std::thread::spawn(move || worker.run());

        scans.request(root.clone());
        let outcomes = poll_until(&scans, |outcomes| {
            outcomes
                .iter()
                .any(|o| matches!(o, ScanOutcome::Failed { .. }))
        });

        match outcomes.first() {
            Some(ScanOutcome::Failed { path, reason }) => {
                assert_eq!(path, &root, "the failure names the scanned root");
                assert!(
                    reason.contains("scan batch commit boom"),
                    "the reason carries the store error: {reason}"
                );
            }
            other => panic!("expected Failed first, got {other:?}"),
        }
        assert!(
            !outcomes
                .iter()
                .any(|o| matches!(o, ScanOutcome::Complete { .. })),
            "a scan whose commit failed must not report Complete: {outcomes:?}"
        );
    }
}

// --- Audio Engine loop tests -------------------------------------------------
//
// The extracted engine (`src/app/audio_engine.rs`) is exercised through its
// ports only: a scripted [`MockAudioDecoder`] factory, the recording
// [`MockAudioOutput`], and an in-memory `LibraryQueryStore` fake. A helper
// thread runs `AudioEngine::new(..).run()` over unbounded crossbeam channels;
// every receive uses a timeout so a wedged engine fails an assertion instead
// of hanging CI.
//
// The shared-handle adapters below exist because the engine takes ownership
// of its ports: they delegate every call to the real mocks behind a mutex so
// the mocks' recording counters remain inspectable after (and while) the
// engine runs — the counters are what prove the substitutes were driven.
mod audio_engine_tests {
    use super::*;
    use crossbeam_channel::{Receiver, unbounded};
    use riff_backend::app::audio_engine::AudioEngine;
    use riff_backend::app::errors::{PlaybackError, StoreError};
    use riff_backend::app::store::LibraryQueryStore;
    use riff_backend::app::traits::{AudioDecoder, AudioFormatInfo, AudioOutput};
    use riff_playback::app::errors::PlaybackError as EnginePlaybackError;
    use riff_playback::infra::ports::AudioDecoder as EngineAudioDecoder;
    use riff_playback::infra::ports::AudioFormatInfo as EngineAudioFormatInfo;
    use riff_playback::infra::ports::AudioOutput as EngineAudioOutput;
    use riff_playback::infra::ports::DecoderFactory;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    // The shared mocks under test (crate-root mocks module).
    use crate::mocks::{MockAudioDecoder, MockAudioOutput};

    /// How long any single wait on the engine may take before the test
    /// declares it wedged.
    const TIMEOUT: Duration = Duration::from_secs(30);

    /// The one format every scripted decoder in these tests reports.
    const RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;

    fn format_with_duration(duration: Option<Duration>) -> AudioFormatInfo {
        AudioFormatInfo {
            sample_rate: RATE,
            channels: CHANNELS,
            duration,
        }
    }

    /// One decode batch: `frames` stereo frames of interleaved silence.
    fn batch(frames: usize) -> Vec<f32> {
        vec![0.0; frames * usize::from(CHANNELS)]
    }

    // --- Shared-handle port adapters -----------------------------------------

    /// [`AudioDecoder`] view over one scripted [`MockAudioDecoder`] owned by
    /// the engine; the test keeps the other handle for counter checks.
    struct SharedDecoder {
        mock: Arc<Mutex<MockAudioDecoder>>,
        path: PathBuf,
    }

    impl EngineAudioDecoder for SharedDecoder {
        fn source_path(&self) -> &Path {
            &self.path
        }

        fn init(&mut self, path: &Path) -> Result<EngineAudioFormatInfo, EnginePlaybackError> {
            self.path = path.to_path_buf();
            let info = AudioDecoder::open(&mut *self.mock.lock().unwrap(), path)
                .map_err(|e| EnginePlaybackError::Decode(e.to_string()))?;
            Ok(EngineAudioFormatInfo {
                sample_rate: info.sample_rate,
                channels: info.channels,
            })
        }

        fn next_frames(&mut self, buf: &mut [f32]) -> Option<usize> {
            match AudioDecoder::next_frames(&mut *self.mock.lock().unwrap(), buf) {
                Ok(0) | Err(_) => None,
                Ok(n) => Some(n),
            }
        }

        fn seek(&mut self, position: Duration) -> Duration {
            let _ = AudioDecoder::seek(&mut *self.mock.lock().unwrap(), position);
            position
        }

        fn duration(&self) -> Option<Duration> {
            AudioDecoder::duration(&*self.mock.lock().unwrap())
        }
    }

    impl AudioDecoder for SharedDecoder {
        fn open(&mut self, path: &Path) -> Result<AudioFormatInfo, PlaybackError> {
            self.mock.lock().unwrap().open(path)
        }

        fn next_frames(&mut self, out: &mut [f32]) -> Result<usize, PlaybackError> {
            self.mock.lock().unwrap().next_frames(out)
        }

        fn seek(&mut self, position: Duration) -> Result<(), PlaybackError> {
            self.mock.lock().unwrap().seek(position)
        }

        fn duration(&self) -> Option<Duration> {
            self.mock.lock().unwrap().duration()
        }

        fn close(&mut self) {
            self.mock.lock().unwrap().close();
        }
    }

    /// [`AudioOutput`] view over one recording [`MockAudioOutput`] owned by
    /// the engine; the test keeps the other handle for counter checks.
    struct SharedOutput(Arc<Mutex<MockAudioOutput>>);

    impl EngineAudioOutput for SharedOutput {
        fn start(&mut self, format: EngineAudioFormatInfo) -> Result<(), EnginePlaybackError> {
            AudioOutput::initialize(
                &mut *self.0.lock().unwrap(),
                format.sample_rate,
                format.channels,
            )
            .map_err(|e| EnginePlaybackError::AudioOutput(e.to_string()))?;
            AudioOutput::start(&mut *self.0.lock().unwrap())
                .map_err(|e| EnginePlaybackError::AudioOutput(e.to_string()))
        }

        fn write(&mut self, samples: &[f32]) -> usize {
            AudioOutput::write_samples(&mut *self.0.lock().unwrap(), samples).unwrap_or(0)
        }

        fn stop(&mut self) {
            let _ = AudioOutput::stop(&mut *self.0.lock().unwrap());
        }

        fn set_volume(&mut self, volume: f32) {
            AudioOutput::set_volume(&mut *self.0.lock().unwrap(), volume);
        }

        fn latency(&self) -> u32 {
            0
        }
    }

    impl AudioOutput for SharedOutput {
        fn initialize(&mut self, sample_rate: u32, channels: u16) -> Result<(), PlaybackError> {
            self.0.lock().unwrap().initialize(sample_rate, channels)
        }

        fn start(&mut self) -> Result<(), PlaybackError> {
            self.0.lock().unwrap().start()
        }

        fn stop(&mut self) -> Result<(), PlaybackError> {
            self.0.lock().unwrap().stop()
        }

        fn write_samples(&mut self, samples: &[f32]) -> Result<usize, PlaybackError> {
            self.0.lock().unwrap().write_samples(samples)
        }

        fn set_volume(&mut self, volume: f32) {
            self.0.lock().unwrap().set_volume(volume);
        }

        fn buffer_len(&self) -> usize {
            self.0.lock().unwrap().buffer_len()
        }

        fn clear_buffer(&mut self) {
            self.0.lock().unwrap().clear_buffer();
        }

        fn effective_sample_rate(&self) -> u32 {
            self.0.lock().unwrap().effective_sample_rate()
        }
    }

    /// Every decoder the factory mints, in mint order (primary first,
    /// gapless pre-decode second). Shared handles keep the mocks' counters
    /// readable after the engine has taken ownership.
    type DecoderLog = Arc<Mutex<Vec<Arc<Mutex<MockAudioDecoder>>>>>;

    /// Scripted [`DecoderFactory`]: mints fresh [`MockAudioDecoder`]s that
    /// report `format`, registering each in `log`. The first mint (the
    /// primary slot) replays `primary`; every later mint (the gapless
    /// pre-decode slot) replays `successor` — mirroring reality, where the
    /// successor is a different track.
    fn scripted_factory(
        primary: Vec<Vec<f32>>,
        successor: Vec<Vec<f32>>,
        format: AudioFormatInfo,
        log: DecoderLog,
    ) -> DecoderFactory {
        let call_index = AtomicUsize::new(0);
        Box::new(move || {
            let script = if call_index.fetch_add(1, Ordering::Relaxed) == 0 {
                primary.clone()
            } else {
                successor.clone()
            };
            let mock = Arc::new(Mutex::new(
                MockAudioDecoder::new(format.clone()).with_batches(script),
            ));
            log.lock().unwrap().push(Arc::clone(&mock));
            Box::new(SharedDecoder {
                mock,
                path: PathBuf::new(),
            })
        })
    }

    /// In-memory [`LibraryQueryStore`] fake serving a canned track map. The
    /// engine only ever calls `get_track` and `all_track_ids`; the remaining
    /// queries return empty results.
    struct FakeLibraryStore {
        tracks: HashMap<TrackId, Track>,
    }

    impl LibraryQueryStore for FakeLibraryStore {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            Ok(self.tracks.get(id).cloned())
        }

        fn tracks_window(&self, _offset: usize, _limit: usize) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn track_count(&self) -> Result<usize, StoreError> {
            Ok(self.tracks.len())
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            let mut ids: Vec<TrackId> = self.tracks.keys().cloned().collect();
            ids.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(ids)
        }

        fn search_window(
            &self,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn search_count(&self, _query: &str) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            Ok(Vec::new())
        }

        fn artist_albums(&self, _artist: &str) -> Result<Vec<Album>, StoreError> {
            Ok(Vec::new())
        }

        fn album_tracks(
            &self,
            _album_artist: &str,
            _album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn folder_has_audio(&self, _folder: &Path) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn folder_has_search_match(
            &self,
            _folder: &Path,
            _query: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn track_ids_in_folder_tree(&self, _folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }

        fn tracks_in_folder(&self, _folder: &Path) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn subdirs_with_audio(&self, _folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            Ok(Vec::new())
        }

        fn smart_playlist(
            &self,
            _kind: SmartPlaylistKind,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }
    }

    // --- Harness --------------------------------------------------------------

    /// A running engine plus every handle the tests need to observe it.
    struct Harness {
        cmd_tx: crossbeam_channel::Sender<PlaybackCommand>,
        updates: Receiver<PlaybackUpdate>,
        state: Arc<Mutex<PlaybackSession>>,
        output: Arc<Mutex<MockAudioOutput>>,
        decoders: DecoderLog,
    }

    /// Spawn the engine exactly like `main.rs` does (blocking `run()` on its
    /// own thread, ports boxed inside the closure) over mock ports.
    fn spawn_engine(
        state: Arc<Mutex<PlaybackSession>>,
        library: FakeLibraryStore,
        primary_script: Vec<Vec<f32>>,
        successor_script: Vec<Vec<f32>>,
        format: AudioFormatInfo,
    ) -> Harness {
        let (cmd_tx, cmd_rx) = unbounded::<PlaybackCommand>();
        let (update_tx, update_rx) = unbounded::<PlaybackUpdate>();

        let output = Arc::new(Mutex::new(MockAudioOutput::new()));
        output.lock().unwrap().set_effective_sample_rate(RATE);
        let decoders: DecoderLog = Arc::new(Mutex::new(Vec::new()));

        let out_handle = Arc::clone(&output);
        let dec_handle = Arc::clone(&decoders);
        let thread_state = Arc::clone(&state);
        std::thread::spawn(move || {
            let factory = scripted_factory(primary_script, successor_script, format, dec_handle);
            let engine = AudioEngine::new(
                cmd_rx,
                update_tx,
                Box::new(library),
                factory,
                Box::new(SharedOutput(out_handle)),
                thread_state,
            );
            engine.run();
        });
        Harness {
            cmd_tx,
            updates: update_rx,
            state,
            output,
            decoders,
        }
    }

    /// End the test's involvement with the engine. `run()` is a daemon loop
    /// by design — the engine holds its own `cmd_tx` re-dispatch handle, so
    /// its `recv()` never disconnects (in production the process lifetime
    /// bounds the thread) — hence the thread is intentionally left parked
    /// rather than joined.
    fn release(_h: Harness) {}

    /// Receive one update, mirroring main.rs's update processor along the
    /// way: on `TrackEnded` it advances the queue and re-dispatches
    /// `Play(new current)` — the exact duplicate the gapless dedup guard
    /// must swallow — or marks playback stopped when nothing follows.
    fn next_update(h: &Harness) -> PlaybackUpdate {
        let update = h.updates.recv_timeout(TIMEOUT).expect("update in time");
        if matches!(update, PlaybackUpdate::TrackEnded) {
            let next = {
                let mut s = h.state.lock_or_recover();
                if s.queue.repeat == RepeatMode::One {
                    s.queue.current_track().cloned()
                } else {
                    s.queue.advance().cloned()
                }
            };
            match next {
                Some(track_id) => {
                    let _ = h.cmd_tx.send(PlaybackCommand::Play(track_id));
                }
                None => {
                    h.state.lock_or_recover().playback_state = PlaybackState::Stopped;
                }
            }
        }
        update
    }

    /// Collect updates until one satisfies `want` (inclusive).
    fn collect_until(h: &Harness, want: impl Fn(&PlaybackUpdate) -> bool) -> Vec<PlaybackUpdate> {
        let mut collected = Vec::new();
        loop {
            let update = next_update(h);
            let done = want(&update);
            collected.push(update);
            if done {
                return collected;
            }
        }
    }

    fn count(updates: &[PlaybackUpdate], want: impl Fn(&PlaybackUpdate) -> bool) -> usize {
        updates.iter().filter(|u| want(u)).count()
    }

    /// Poll until `pred` holds (commands that produce no update — volume,
    /// seek — need this to prove the engine consumed them).
    fn wait_until(pred: impl Fn() -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !pred() {
            assert!(Instant::now() < deadline, "condition not reached in time");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn is_track_changed(u: &PlaybackUpdate, id: &TrackId) -> bool {
        matches!(u, PlaybackUpdate::TrackChanged(t) if t == id)
    }

    fn is_state(u: &PlaybackUpdate, s: PlaybackState) -> bool {
        matches!(u, PlaybackUpdate::StateChanged(x) if *x == s)
    }

    /// Pre-populate the queue so Queue Fill never fires, and register the
    /// matching canned library entries.
    fn queued_state_and_library(paths: &[&str]) -> (Arc<Mutex<PlaybackSession>>, FakeLibraryStore) {
        let mut state = PlaybackSession::default();
        let mut tracks = HashMap::new();
        for path in paths {
            let track = crate::test_utils::create_test_track(path, path);
            state.queue.append(track.id.clone());
            tracks.insert(track.id.clone(), track);
        }
        state.queue.current_index = Some(0);
        (Arc::new(Mutex::new(state)), FakeLibraryStore { tracks })
    }

    // --- Red 1 tests -----------------------------------------------------------

    /// Play(track) → `TrackChanged` + `StateChanged(Playing)` + position
    /// updates as the script decodes, then the gapped EOF path emits
    /// `TrackEnded`.
    #[test]
    fn engine_plays_track_and_emits_updates() {
        let (state, library) = queued_state_and_library(&["music/t1.wav"]);
        // Two small batches: well under the backpressure threshold, so the
        // decode loop writes both and then hits EOF. No successor is
        // scripted, so the pre-decode slot stays silent.
        let script = vec![batch(2_400), batch(2_400)];
        let format = format_with_duration(Some(Duration::from_secs(1)));
        let h = spawn_engine(state.clone(), library, script, Vec::new(), format);

        h.cmd_tx
            .send(PlaybackCommand::Play(TrackId("music/t1.wav".into())))
            .unwrap();

        let updates = collect_until(&h, |u| matches!(u, PlaybackUpdate::TrackEnded));

        assert!(
            updates
                .iter()
                .any(|u| is_track_changed(u, &TrackId("music/t1.wav".into()))),
            "expected TrackChanged(t1), got {updates:?}"
        );
        assert!(
            updates.iter().any(|u| is_state(u, PlaybackState::Playing)),
            "expected StateChanged(Playing), got {updates:?}"
        );
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, PlaybackUpdate::PositionChanged(_))),
            "expected position updates while decoding, got {updates:?}"
        );

        // Mock substitution proof: the primary decoder was opened for the
        // requested track and the output mock recorded the writes. (Decoders
        // are minted on demand: with no successor queued, the gapless
        // pre-decode slot never mints its decoder.)
        let decoders = h.decoders.lock().unwrap();
        assert_eq!(
            decoders.len(),
            1,
            "primary only - no successor to pre-decode"
        );
        let d = decoders[0].lock().unwrap();
        assert_eq!(d.opened, vec![PathBuf::from("music/t1.wav")]);
        assert_eq!(d.seeks.len(), 0, "no seek without Resume/Seek commands");
        drop(d);
        drop(decoders);
        let out = h.output.lock().unwrap();
        assert_eq!(out.initialized, vec![(RATE, CHANNELS)]);
        assert_eq!(out.start_count, 1, "the stream started exactly once");
        // Each scripted batch holds 4800 samples, so both split across the
        // engine's 4096-sample decode chunk: four writes cover the two
        // batches (4096 + 704 remainder, twice).
        assert_eq!(out.written.len(), 4, "both scripted batches were written");

        // Release the gapped EOF drain (samples sit in the mock buffer): the
        // drain loop swallows this Stop without dispatching it, which is the
        // documented gapped-path behavior.
        h.cmd_tx.send(PlaybackCommand::Stop).unwrap();
        drop(out);
        release(h);
        assert_eq!(
            state.lock_or_recover().playback_state,
            PlaybackState::Stopped
        );
    }

    /// The hardest invariant: when a compatible successor was pre-decoded,
    /// EOF hands off WITHOUT stopping/restarting the stream, and the update
    /// processor's automatic `Play(successor)` is swallowed — exactly one
    /// decode/start of the successor survives the handoff.
    #[test]
    fn engine_survives_gapless_handoff_with_single_play() {
        let (state, library) = queued_state_and_library(&["music/t1.wav", "music/t2.wav"]);
        // Primary script: 30 × 2400-frame batches = 144k interleaved samples
        // (~1.5 s @48 kHz stereo) — enough to trigger pre-encode early and,
        // together with the flushed successor pre-buffer, strictly under the
        // 192k-sample backpressure threshold, so the loop always reaches EOF
        // instead of parking.
        let primary: Vec<Vec<f32>> = (0..30).map(|_| batch(2_400)).collect();
        // Successor script: 5 batches = 24k samples fully pre-buffered at
        // pre-encode time, then flushed at the handoff.
        let successor: Vec<Vec<f32>> = (0..5).map(|_| batch(2_400)).collect();
        let format = format_with_duration(Some(Duration::from_secs(1)));
        let h = spawn_engine(state.clone(), library, primary, successor, format);

        h.cmd_tx
            .send(PlaybackCommand::Play(TrackId("music/t1.wav".into())))
            .unwrap();

        // Through the handoff: TrackEnded #1 triggers the simulated
        // processor's Play(t2) re-dispatch, which the engine must swallow.
        let mut updates =
            collect_until(&h, |u| is_track_changed(u, &TrackId("music/t2.wav".into())));
        // Then ride out the successor's (script-exhausted) tail to EOF #2.
        let tail = collect_until(&h, |u| matches!(u, PlaybackUpdate::TrackEnded));
        updates.extend(tail);

        // Snapshot BEFORE releasing the drain: through the whole handoff the
        // cpal stream must never have been torn down. One start total (a
        // teardown+restart would need a second), and exactly one stop — the
        // pre-play stop every handle_play issues before opening. Any stop
        // across the handoff itself would push this to 2+.
        {
            let out = h.output.lock().unwrap();
            assert_eq!(
                out.start_count, 1,
                "gapless handoff never restarts the stream"
            );
            assert_eq!(
                out.stop_count, 1,
                "only the pre-play stop; none across the handoff"
            );
            let total_written: usize = out.written.iter().map(Vec::len).sum();
            assert_eq!(
                total_written,
                30 * 4_800 + 5 * 4_800,
                "primary script + flushed pre-buffer, nothing re-decoded"
            );
        }

        // Exactly one announcement per track: the auto-Play(t2) duplicate
        // must not produce a second TrackChanged(t2) or a reopen.
        assert_eq!(
            count(&updates, |u| is_track_changed(
                u,
                &TrackId("music/t1.wav".into())
            )),
            1
        );
        assert_eq!(
            count(&updates, |u| is_track_changed(
                u,
                &TrackId("music/t2.wav".into())
            )),
            1,
            "the post-handoff auto-Play duplicate is swallowed"
        );
        assert_eq!(
            count(&updates, |u| matches!(u, PlaybackUpdate::TrackEnded)),
            2,
            "handoff TrackEnded + successor-tail TrackEnded"
        );

        // Exactly two decoders were ever minted: primary + pre-decode slot.
        // A third mint would mean the dup tore down and re-opened the track;
        // the open logs prove which decoder served which track.
        let decoders = h.decoders.lock().unwrap();
        assert_eq!(decoders.len(), 2, "no decoder is minted across the handoff");
        assert_eq!(
            decoders[0].lock().unwrap().opened,
            vec![PathBuf::from("music/t1.wav")]
        );
        assert_eq!(
            decoders[1].lock().unwrap().opened,
            vec![PathBuf::from("music/t2.wav")],
            "the successor opened exactly once (during pre-decode)"
        );
        drop(decoders);

        h.cmd_tx.send(PlaybackCommand::Stop).unwrap();
        release(h);
    }

    /// Command→update contract: Pause/Resume round-trip through the decode
    /// loop (Resume re-opens and seeks back to the pause point), `Seek` and
    /// `SetVolume` apply mid-session and while idle.
    #[test]
    fn engine_pause_resume_seek_volume_contract() {
        let (state, library) = queued_state_and_library(&["music/t1.wav"]);
        // A long script so the first session parks in backpressure (buffer
        // full) instead of hitting EOF — that parked loop is where mid-
        // session commands are consumed deterministically. No successor is
        // scripted.
        let script: Vec<Vec<f32>> = (0..4_000).map(|_| batch(480)).collect();
        let format = format_with_duration(Some(Duration::from_secs(80)));
        let h = spawn_engine(state.clone(), library, script, Vec::new(), format);

        h.cmd_tx
            .send(PlaybackCommand::Play(TrackId("music/t1.wav".into())))
            .unwrap();
        collect_until(&h, |u| is_state(u, PlaybackState::Playing));

        // Mid-session contract, consumed by the decode loop's polling:
        h.cmd_tx.send(PlaybackCommand::SetVolume(0.5)).unwrap();
        h.cmd_tx
            .send(PlaybackCommand::Seek(Duration::from_secs(1)))
            .unwrap();
        h.cmd_tx.send(PlaybackCommand::Pause).unwrap();
        let paused = collect_until(&h, |u| is_state(u, PlaybackState::Paused));
        assert!(
            !paused
                .iter()
                .any(|u| matches!(u, PlaybackUpdate::TrackEnded)),
            "pause itself ends the session — no track-end may precede it"
        );

        // Resume re-dispatches Play(current): a second open+start of the
        // SAME track, seeking back to the recorded pause position.
        h.cmd_tx.send(PlaybackCommand::Resume).unwrap();
        let resumed = collect_until(&h, |u| is_state(u, PlaybackState::Playing));
        assert_eq!(
            count(&resumed, |u| is_track_changed(
                u,
                &TrackId("music/t1.wav".into())
            )),
            1,
            "resume announces the same track again"
        );

        // Idle-command contract after stopping the resumed session. Neither
        // command produces an update, so poll for their observable effects.
        h.cmd_tx.send(PlaybackCommand::Stop).unwrap();
        collect_until(&h, |u| is_state(u, PlaybackState::Stopped));
        h.cmd_tx.send(PlaybackCommand::SetVolume(0.25)).unwrap();
        h.cmd_tx
            .send(PlaybackCommand::Seek(Duration::from_secs(2)))
            .unwrap();
        wait_until(|| {
            let out = h.output.lock().unwrap();
            out.volumes.last() == Some(&0.25)
        });
        wait_until(|| {
            let decoders = h.decoders.lock().unwrap();
            decoders[0]
                .lock()
                .unwrap()
                .seeks
                .contains(&Duration::from_secs(2))
        });

        let out = h.output.lock().unwrap();
        assert_eq!(
            out.initialized,
            vec![(RATE, CHANNELS), (RATE, CHANNELS)],
            "initial start + resume restart"
        );
        assert_eq!(out.start_count, 2);
        assert!(
            out.volumes.contains(&0.5),
            "mid-session SetVolume reached the output"
        );
        assert_eq!(
            out.volumes.last(),
            Some(&0.25),
            "idle SetVolume reached the output"
        );
        drop(out);

        let decoders = h.decoders.lock().unwrap();
        // Decoders are minted on demand: with no successor queued, only the
        // primary exists (the resume re-opens it rather than minting one).
        assert_eq!(decoders.len(), 1, "primary only");
        let d = decoders[0].lock().unwrap();
        assert_eq!(d.opened.len(), 2, "initial open + resume re-open");
        assert!(
            d.seeks.contains(&Duration::from_secs(1)),
            "mid-session Seek reached the decoder"
        );
        assert!(
            d.seeks.contains(&Duration::from_secs(2)),
            "idle Seek reached the decoder"
        );
        assert!(
            d.seeks.len() >= 3,
            "resume also re-seeked to the recorded pause position"
        );
        drop(d);
        drop(decoders);

        release(h);
    }
}

// --- Tag Edit Service tests ---------------------------------------------------
//
// The extracted service (`src/app/tag_edit_service.rs`) is exercised through
// its public interface only: `submit` a [`TagEditRequest`], `poll` for the
// combined [`TagEditOutcome`], and assert on the mocks' recordings. A helper
// thread runs the blocking `TagEditWorker::run()` over the paired channels —
// exactly how the composition root will spawn it (ADR 0006); every wait uses
// a deadline so a wedged worker fails an assertion instead of hanging CI.
//
// The shared-handle adapters below exist because the worker takes ownership
// of its ports: they delegate every call to the real mocks behind a mutex so
// the mocks' recordings remain inspectable after the worker has run — the
// recordings are what prove the substitutes were driven.
mod tag_edit_service_tests {
    use super::*;
    use crate::mocks::{MockLibraryMutationStore, MockLibraryQueryStore, MockMetadataWriter};
    use riff_backend::app::errors::{LibraryError, StoreError};
    use riff_backend::app::store::LibraryQueryStore;
    use riff_backend::app::tag_edit_service::{
        TagEditOutcome, TagEditRequest, TagEditService, TagEdits,
    };
    use riff_backend::app::traits::{MetadataWriter, TagEdit};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// How long any single wait on the worker may take before the test
    /// declares it wedged.
    const TIMEOUT: Duration = Duration::from_secs(10);

    // --- Shared-handle port adapters -----------------------------------------

    /// [`MetadataWriter`] view over one recording [`MockMetadataWriter`]
    /// owned by the worker; the test keeps the other handle.
    struct SharedWriter(Arc<Mutex<MockMetadataWriter>>);

    impl MetadataWriter for SharedWriter {
        fn write_metadata(&self, path: &Path, edit: &TagEdit) -> Result<(), LibraryError> {
            self.0.lock().unwrap().write_metadata(path, edit)
        }
    }

    /// [`LibraryQueryStore`] view over one canned
    /// [`MockLibraryQueryStore`] owned by the worker; the test keeps the
    /// other handle. Only `get_track` is delegated with its real behavior —
    /// it is the only query the save flow issues; every other query answers
    /// empty without touching the mock.
    struct SharedQueries(Arc<Mutex<MockLibraryQueryStore>>);

    impl LibraryQueryStore for SharedQueries {
        fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
            self.0.lock().unwrap().get_track(id)
        }

        fn tracks_window(&self, _offset: usize, _limit: usize) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn track_count(&self) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }

        fn search_window(
            &self,
            _query: &str,
            _offset: usize,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn search_count(&self, _query: &str) -> Result<usize, StoreError> {
            Ok(0)
        }

        fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
            Ok(Vec::new())
        }

        fn artist_albums(&self, _artist: &str) -> Result<Vec<Album>, StoreError> {
            Ok(Vec::new())
        }

        fn album_tracks(
            &self,
            _album_artist: &str,
            _album_title: &str,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn folder_has_audio(&self, _folder: &Path) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn folder_has_search_match(
            &self,
            _folder: &Path,
            _query: &str,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }

        fn track_ids_in_folder_tree(&self, _folder: &Path) -> Result<Vec<TrackId>, StoreError> {
            Ok(Vec::new())
        }

        fn tracks_in_folder(&self, _folder: &Path) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }

        fn subdirs_with_audio(&self, _folder: &Path) -> Result<Vec<PathBuf>, StoreError> {
            Ok(Vec::new())
        }

        fn smart_playlist(
            &self,
            _kind: SmartPlaylistKind,
            _limit: usize,
        ) -> Result<Vec<Track>, StoreError> {
            Ok(Vec::new())
        }
    }

    /// [`riff_backend::app::store::LibraryMutationStore`] view over one recording
    /// [`MockLibraryMutationStore`] owned by the worker; the test keeps the
    /// other handle. Only `apply_tag_refresh` is delegated with its real
    /// behavior — it is the only mutation the save flow issues.
    struct SharedMutations(Arc<Mutex<MockLibraryMutationStore>>);

    impl riff_backend::app::store::LibraryMutationStore for SharedMutations {
        fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError> {
            self.0.lock().unwrap().apply_scan_batch(tracks)
        }

        fn record_track_played(
            &mut self,
            id: &TrackId,
            played_at: std::time::SystemTime,
        ) -> Result<bool, StoreError> {
            self.0.lock().unwrap().record_track_played(id, played_at)
        }

        fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError> {
            self.0.lock().unwrap().apply_tag_refresh(track)
        }

        fn remove_library_path(&mut self, root: &Path) -> Result<usize, StoreError> {
            self.0.lock().unwrap().remove_library_path(root)
        }

        fn clear_library(&mut self) -> Result<usize, StoreError> {
            self.0.lock().unwrap().clear_library()
        }
    }

    // --- Harness ---------------------------------------------------------------

    /// Ports under test plus the front-end handle to drive them with.
    struct Harness {
        service: TagEditService,
        writer: Arc<Mutex<MockMetadataWriter>>,
        queries: Arc<Mutex<MockLibraryQueryStore>>,
        mutations: Arc<Mutex<MockLibraryMutationStore>>,
    }

    /// Wire a real service/worker pair over injected mocks and run the
    /// blocking worker on its own thread, exactly like the composition root
    /// will (ADR 0006). Dropping `Harness.service` ends the thread.
    fn spawn_service(
        writer: MockMetadataWriter,
        library: HashMap<TrackId, Track>,
        mutations: MockLibraryMutationStore,
    ) -> Harness {
        let writer = Arc::new(Mutex::new(writer));
        let queries = Arc::new(Mutex::new(MockLibraryQueryStore {
            library,
            ..Default::default()
        }));
        let mutations = Arc::new(Mutex::new(mutations));
        let (service, worker) = TagEditService::new(
            Box::new(SharedWriter(Arc::clone(&writer))),
            Box::new(SharedQueries(Arc::clone(&queries))),
            Box::new(SharedMutations(Arc::clone(&mutations))),
        );
        std::thread::spawn(move || worker.run());
        Harness {
            service,
            writer,
            queries,
            mutations,
        }
    }

    /// Poll until an outcome arrives or `TIMEOUT` elapses. Spins on the
    /// public interface only — the worker is fast and the deadline exists so
    /// a missing outcome fails instead of hanging.
    fn poll_outcome(service: &dyn TagEdits) -> Option<TagEditOutcome> {
        let start = Instant::now();
        while start.elapsed() < TIMEOUT {
            if let Some(outcome) = service.poll() {
                return Some(outcome);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    /// A store Track at `/music/t1.mp3` with artist/title/album set.
    fn stored_track() -> Track {
        crate::test_utils::create_test_track_with_metadata(
            "/music/t1.mp3",
            "/music/t1.mp3",
            "Artist",
            "Old Title",
            "Album",
        )
    }

    /// An edit that only renames the title.
    fn title_edit(new_title: &str) -> TagEdit {
        TagEdit {
            title: Some(new_title.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_tag_edit_success_reports_saved_and_commits_edited_track() {
        let track = stored_track();
        let h = spawn_service(
            MockMetadataWriter::recording(),
            HashMap::from([(track.id.clone(), track)]),
            MockLibraryMutationStore::new(),
        );

        let path = PathBuf::from("/music/t1.mp3");
        h.service.submit(TagEditRequest {
            track_id: TrackId::from_path(&path),
            path: path.clone(),
            edit: title_edit("New Title"),
        });

        let outcome = poll_outcome(&h.service);
        assert_eq!(outcome, Some(TagEditOutcome::Saved));

        // The file write reached the Metadata Writer port with path + edit.
        let writes = h.writer.lock().unwrap().recorded();
        assert_eq!(writes.len(), 1, "exactly one file write");
        assert_eq!(writes[0].0, path);
        assert_eq!(writes[0].1.title.as_deref(), Some("New Title"));

        // The store commit received a Track whose metadata carries the
        // applied edit; untouched fields stay as resolved from the store.
        let refreshed = h.mutations.lock().unwrap().refreshed();
        assert_eq!(refreshed.len(), 1, "exactly one tag refresh commit");
        assert_eq!(refreshed[0].metadata.title.as_deref(), Some("New Title"));
        assert_eq!(refreshed[0].metadata.artist.as_deref(), Some("Artist"));
        assert_eq!(refreshed[0].metadata.album.as_deref(), Some("Album"));
    }

    #[test]
    fn test_tag_edit_writer_failure_fails_without_store_interaction() {
        let track = stored_track();
        let h = spawn_service(
            MockMetadataWriter::failing(),
            HashMap::from([(track.id.clone(), track)]),
            MockLibraryMutationStore::new(),
        );

        let path = PathBuf::from("/music/t1.mp3");
        h.service.submit(TagEditRequest {
            track_id: TrackId::from_path(&path),
            path,
            edit: title_edit("New Title"),
        });

        match poll_outcome(&h.service) {
            Some(TagEditOutcome::Failed { reason }) => {
                assert!(
                    reason.contains("permission denied"),
                    "unexpected failure reason: {reason}"
                );
            }
            other => panic!("expected Failed with a reason, got {other:?}"),
        }

        // A failed file write must not touch the store at all: no resolve,
        // no commit.
        assert!(
            h.queries.lock().unwrap().get_track_calls().is_empty(),
            "the store was queried despite the write failing"
        );
        assert!(
            h.mutations.lock().unwrap().refreshed().is_empty(),
            "the store was committed despite the write failing"
        );
    }

    #[test]
    fn test_tag_edit_vanished_track_fails_without_commit() {
        // Empty library: the track was removed from the store mid-edit, so
        // the resolve comes back empty even though the file write succeeded.
        let h = spawn_service(
            MockMetadataWriter::recording(),
            HashMap::new(),
            MockLibraryMutationStore::new(),
        );

        let path = PathBuf::from("/music/t1.mp3");
        let track_id = TrackId::from_path(&path);
        h.service.submit(TagEditRequest {
            track_id: track_id.clone(),
            path,
            edit: title_edit("New Title"),
        });

        assert_eq!(
            poll_outcome(&h.service),
            Some(TagEditOutcome::Failed {
                reason: "Track is no longer in the library".to_string()
            }),
            "exact product-wording for an unknown track"
        );

        // The file write happened and the store WAS consulted, but nothing
        // was persisted for a track the store no longer knows.
        assert_eq!(h.writer.lock().unwrap().recorded().len(), 1);
        assert_eq!(h.queries.lock().unwrap().get_track_calls(), vec![track_id]);
        assert!(
            h.mutations.lock().unwrap().refreshed().is_empty(),
            "the store was committed for a vanished track"
        );
    }

    #[test]
    fn test_tag_edit_commit_failure_fails_despite_successful_write() {
        // The file write succeeds and the track resolves, but the store
        // commit fails: the combined outcome must be a failure, never a
        // silent success (spec testing decisions).
        let track = stored_track();
        let mutations = MockLibraryMutationStore::failing_refresh();
        let h = spawn_service(
            MockMetadataWriter::recording(),
            HashMap::from([(track.id.clone(), track)]),
            mutations,
        );

        let path = PathBuf::from("/music/t1.mp3");
        h.service.submit(TagEditRequest {
            track_id: TrackId::from_path(&path),
            path,
            edit: title_edit("New Title"),
        });

        match poll_outcome(&h.service) {
            Some(TagEditOutcome::Failed { reason }) => {
                assert!(
                    reason.contains("mock tag refresh failure"),
                    "unexpected failure reason: {reason}"
                );
            }
            other => panic!("expected Failed despite successful file write, got {other:?}"),
        }

        // The file write DID happen (it succeeded); only the commit failed.
        assert_eq!(h.writer.lock().unwrap().recorded().len(), 1);
    }

    #[test]
    fn test_tag_edit_preserves_play_history_in_committed_track() {
        // The Track handed to the commit must be the store's fresh copy with
        // only metadata edited: play history (play_count, last_played,
        // date_added) survives so Most Played-style smart playlists keep
        // working (spec user story 5).
        let mut track = stored_track();
        track.play_count = 7;
        track.last_played = Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
        track.date_added = Some(std::time::SystemTime::UNIX_EPOCH);
        let h = spawn_service(
            MockMetadataWriter::recording(),
            HashMap::from([(track.id.clone(), track)]),
            MockLibraryMutationStore::new(),
        );

        let path = PathBuf::from("/music/t1.mp3");
        h.service.submit(TagEditRequest {
            track_id: TrackId::from_path(&path),
            path,
            edit: title_edit("New Title"),
        });

        assert_eq!(poll_outcome(&h.service), Some(TagEditOutcome::Saved));

        let refreshed = h.mutations.lock().unwrap().refreshed();
        assert_eq!(refreshed.len(), 1, "exactly one tag refresh commit");
        let committed = &refreshed[0];
        assert_eq!(committed.metadata.title.as_deref(), Some("New Title"));
        assert_eq!(committed.play_count, 7, "play_count must survive the edit");
        assert_eq!(
            committed.last_played,
            Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
            "last_played must survive the edit"
        );
        assert_eq!(
            committed.date_added,
            Some(std::time::SystemTime::UNIX_EPOCH),
            "date_added must survive the edit"
        );
    }

    #[test]
    fn test_tag_edit_poll_returns_none_when_idle_and_delivers_each_outcome_once() {
        // First write succeeds, second fails: the two outcomes are
        // distinguishable, so exactly-once delivery is observable.
        let track = stored_track();
        let h = spawn_service(
            MockMetadataWriter::failing_after(1),
            HashMap::from([(track.id.clone(), track)]),
            MockLibraryMutationStore::new(),
        );

        // Nothing submitted yet: polling is a non-blocking `None`.
        assert!(h.service.poll().is_none(), "idle service must not yield");

        // Two edits with distinguishable fates: the first saves, the second
        // fails its file write. Outcomes come back exactly once each, oldest
        // first.
        let path = PathBuf::from("/music/t1.mp3");
        h.service.submit(TagEditRequest {
            track_id: TrackId::from_path(&path),
            path: path.clone(),
            edit: title_edit("New Title"),
        });
        h.service.submit(TagEditRequest {
            track_id: TrackId::from_path(&path),
            path,
            edit: title_edit("Doomed"),
        });

        let first = poll_outcome(&h.service);
        assert_eq!(first, Some(TagEditOutcome::Saved), "first edit saved");
        let second = poll_outcome(&h.service);
        match second {
            Some(TagEditOutcome::Failed { .. }) => {}
            other => panic!("expected the second outcome to be Failed, got {other:?}"),
        }

        // Both submitted edits were delivered; nothing else exists to poll.
        assert!(
            h.service.poll().is_none(),
            "each completed edit is delivered exactly once"
        );
    }
}

// --- Cover Service tests -------------------------------------------------------
//
// The extracted service (`src/app/cover_service.rs`) is exercised through
// its public interface only: `request` a cover, `poll` for drained results,
// and assert on the resolver mocks' interaction counts. A helper thread runs
// the blocking `CoverWorker::run()` over the paired channels — exactly how
// the composition root will spawn it (ADR 0006); every wait uses a deadline
// so a wedged worker fails an assertion instead of hanging CI.
//
// The counting port fakes below are local to this module (precedent:
// `FakeLibraryStore` in the audio-engine tests) because the shared mocks in
// `tests/mod.rs` carry no call counters; the counters are what prove the
// dedup/negative-cache discipline actually suppressed disk I/O.
mod cover_service_tests {
    use super::*;
    use riff_backend::app::cover_service::{CoverService, Covers};
    use riff_backend::domain::CoverSource;
    use riff_library::app::errors::LibraryError;
    use riff_library::app::traits::{AudioFormatInfo, CoverImage, CoverLoader, MetadataReader};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// How long any single wait on the worker may take before the test
    /// declares it wedged.
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// One decoded cover image, distinct enough to assert identity.
    fn test_image() -> CoverImage {
        CoverImage {
            data: vec![7; 4 * 4 * 4],
            format: image::ImageFormat::Png,
        }
    }

    /// [`MetadataReader`] fake whose only live query is `read_cover_source`
    /// (the only one `CoverResolver` issues): it counts invocations and
    /// serves a canned [`CoverSource`].
    struct SharedReader {
        source: CoverSource,
        calls: Arc<AtomicUsize>,
    }

    impl MetadataReader for SharedReader {
        fn read_cover_source(&self, _path: &Path) -> Result<CoverSource, LibraryError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.source.clone())
        }

        fn read_all(
            &self,
            _path: &Path,
        ) -> Result<(TrackMetadata, Duration, CoverSource, AudioFormatInfo), LibraryError> {
            Err(LibraryError::Io(
                "not exercised by CoverResolver".to_string(),
            ))
        }
    }

    /// [`CoverLoader`] fake serving one canned result and counting every
    /// load. When `gate` is set, each call blocks until a token arrives on
    /// the paired sender — a deterministic in-flight window for the dedup
    /// test (the test observes `calls == 1` before releasing).
    struct SharedLoader {
        result: Result<Option<CoverImage>, String>,
        calls: Arc<AtomicUsize>,
        gate: Option<Mutex<crossbeam_channel::Receiver<()>>>,
    }

    impl CoverLoader for SharedLoader {
        fn load_cover(&self, _source: &CoverSource) -> Result<Option<CoverImage>, LibraryError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(ref gate) = self.gate {
                let _ = gate.lock().unwrap().recv();
            }
            self.result.clone().map_err(LibraryError::CoverLoad)
        }
    }

    // --- Harness ---------------------------------------------------------------

    /// Counters plus the front-end handle to drive them with.
    struct Harness {
        service: CoverService,
        reader_calls: Arc<AtomicUsize>,
        loader_calls: Arc<AtomicUsize>,
    }

    /// Wire a real service/worker pair over the counting fakes and run the
    /// blocking worker on its own thread, exactly like the composition root
    /// will (ADR 0006). Dropping `Harness.service` ends the thread.
    fn spawn_service(
        source: CoverSource,
        loader_result: Result<Option<CoverImage>, String>,
    ) -> Harness {
        spawn_with_gate(source, loader_result, None)
    }

    /// Like [`spawn_service`], but with an optional load gate for tests that
    /// must hold a resolve open mid-flight.
    fn spawn_with_gate(
        source: CoverSource,
        loader_result: Result<Option<CoverImage>, String>,
        gate: Option<crossbeam_channel::Receiver<()>>,
    ) -> Harness {
        let reader_calls = Arc::new(AtomicUsize::new(0));
        let loader_calls = Arc::new(AtomicUsize::new(0));
        let (service, worker) = CoverService::new(
            Box::new(SharedReader {
                source,
                calls: Arc::clone(&reader_calls),
            }),
            Box::new(SharedLoader {
                result: loader_result,
                calls: Arc::clone(&loader_calls),
                gate: gate.map(Mutex::new),
            }),
        );
        std::thread::spawn(move || worker.run());
        Harness {
            service,
            reader_calls,
            loader_calls,
        }
    }

    /// Poll until at least `expected` results have drained or `TIMEOUT`
    /// elapses, accumulating across polls (results are delivered once, so
    /// accumulation cannot double-count). Spins on the public interface
    /// only; returning short of `expected` fails the caller's assertion
    /// instead of hanging CI.
    fn poll_until(service: &dyn Covers, expected: usize) -> Vec<(TrackId, Option<CoverImage>)> {
        let mut collected: Vec<(TrackId, Option<CoverImage>)> = Vec::new();
        let start = Instant::now();
        while collected.len() < expected && start.elapsed() < TIMEOUT {
            collected.extend(service.poll());
            if collected.len() < expected {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        collected
    }

    #[test]
    fn test_cover_request_resolves_and_poll_yields_image() {
        let image = test_image();
        let h = spawn_service(
            CoverSource::Embedded(vec![1, 2, 3].into()),
            Ok(Some(image.clone())),
        );

        let path = PathBuf::from("/music/t1.mp3");
        h.service.request(TrackId::from_path(&path), path);

        let results = poll_until(&h.service, 1);
        assert_eq!(results.len(), 1, "exactly one resolved result");
        assert_eq!(results[0].0, TrackId::from_path(Path::new("/music/t1.mp3")));
        let delivered = results[0].1.as_ref().expect("cover should resolve");
        assert_eq!(
            (delivered.data.as_slice(), delivered.format),
            (image.data.as_slice(), image.format)
        );
        assert_eq!(delivered.data, image.data);

        // The resolver chain drove the real ports: one tag read, one load.
        assert_eq!(h.reader_calls.load(Ordering::SeqCst), 1);
        assert_eq!(h.loader_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_cover_artless_track_negative_cached_until_eviction() {
        // No embedded art, no filesystem fallback hit: the resolve comes
        // back None and the track must be negative-cached — a repeat
        // request triggers no further resolver I/O.
        let h = spawn_service(CoverSource::None, Ok(None));

        let path = PathBuf::from("/music/t1.mp3");
        h.service.request(TrackId::from_path(&path), path.clone());

        let results = poll_until(&h.service, 1);
        assert_eq!(results.len(), 1, "the artless resolve is still delivered");
        assert!(results[0].1.is_none(), "no cover found");
        assert_eq!(h.reader_calls.load(Ordering::SeqCst), 1);
        assert_eq!(h.loader_calls.load(Ordering::SeqCst), 1);

        // Repeat request for the same artless track: suppressed at intake.
        h.service.request(TrackId::from_path(&path), path);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            h.reader_calls.load(Ordering::SeqCst),
            1,
            "negative cache did not suppress the tag read"
        );
        assert_eq!(
            h.loader_calls.load(Ordering::SeqCst),
            1,
            "negative cache did not suppress the cover load"
        );
        assert!(
            h.service.poll().is_empty(),
            "a suppressed request must not produce a second result"
        );
    }

    #[test]
    fn test_cover_duplicate_requests_while_unresolved_yield_one_resolve() {
        // Gate the loader so the first resolve is observably in-flight while
        // the duplicates arrive: this makes the dedup window deterministic
        // instead of racing thread scheduling.
        let (gate_tx, gate_rx) = crossbeam_channel::unbounded();
        let image = test_image();
        let h = spawn_with_gate(
            CoverSource::Embedded(vec![1, 2, 3].into()),
            Ok(Some(image)),
            Some(gate_rx),
        );

        let path = PathBuf::from("/music/t1.mp3");
        h.service.request(TrackId::from_path(&path), path.clone());

        // Wait until the resolve is definitively mid-flight...
        let start = Instant::now();
        while h.loader_calls.load(Ordering::SeqCst) < 1 && start.elapsed() < TIMEOUT {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            h.loader_calls.load(Ordering::SeqCst),
            1,
            "the first resolve never started"
        );

        // ...then fire rapid duplicates for the same track.
        for _ in 0..3 {
            h.service.request(TrackId::from_path(&path), path.clone());
        }

        // Release the gate and let the worker settle completely.
        let _ = gate_tx.send(());
        std::thread::sleep(Duration::from_millis(100));

        let mut results = poll_until(&h.service, 1);
        results.extend(h.service.poll());
        assert_eq!(results.len(), 1, "one in-flight resolve, one polled result");
        assert!(results[0].1.is_some());
        assert_eq!(
            h.reader_calls.load(Ordering::SeqCst),
            1,
            "duplicates of an in-flight resolve must not re-read tags"
        );
        assert_eq!(
            h.loader_calls.load(Ordering::SeqCst),
            1,
            "duplicates of an in-flight resolve must not re-load"
        );
    }

    #[test]
    fn test_cover_negative_cache_evicts_oldest_and_allows_retry() {
        // Fill the bounded negative cache past capacity with distinct
        // artless tracks: the oldest entry must fall out, making THAT track
        // resolvable again while newer entries stay cached.
        let h = spawn_service(CoverSource::None, Ok(None));

        let track_path = |i: usize| PathBuf::from(format!("/music/t{i:02}.mp3"));
        for i in 0..=50 {
            let path = track_path(i);
            h.service.request(TrackId::from_path(&path), path);
        }
        let first_round = poll_until(&h.service, 51);
        assert_eq!(first_round.len(), 51, "every artless resolve is delivered");
        assert_eq!(
            h.reader_calls.load(Ordering::SeqCst),
            51,
            "each distinct track resolved exactly once"
        );

        // The oldest entry (t00) was evicted by the 51st insert: requesting
        // it again must re-resolve.
        let evicted_path = track_path(0);
        h.service
            .request(TrackId::from_path(&evicted_path), evicted_path);
        let retry = poll_until(&h.service, 1);
        assert_eq!(retry.len(), 1, "the evicted track re-resolves");
        assert!(retry[0].1.is_none());
        assert_eq!(
            h.reader_calls.load(Ordering::SeqCst),
            52,
            "exactly one retry after eviction"
        );

        // A still-cached track remains suppressed. (The retry above pushed
        // t00 back in at the MRU end, which evicted the NEXT-oldest entry,
        // t01 — so t02 is the neighbor asserted here.)
        let cached_path = track_path(2);
        h.service
            .request(TrackId::from_path(&cached_path), cached_path);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            h.reader_calls.load(Ordering::SeqCst),
            52,
            "non-evicted entries keep suppressing disk I/O"
        );
        assert!(h.service.poll().is_empty());
    }

    #[test]
    fn test_cover_poll_drains_all_completed_results_then_stays_empty() {
        let h = spawn_service(
            CoverSource::Embedded(vec![9, 9, 9].into()),
            Ok(Some(test_image())),
        );

        // Three distinct tracks, all resolvable.
        for i in 0..3 {
            let path = PathBuf::from(format!("/music/t{i}.mp3"));
            h.service.request(TrackId::from_path(&path), path);
        }

        // Accumulate across polls until all three have drained.
        let results = poll_until(&h.service, 3);
        assert_eq!(results.len(), 3, "all three resolutions delivered");
        let mut ids: Vec<String> = results.iter().map(|(id, _)| id.0.clone()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["/music/t0.mp3", "/music/t1.mp3", "/music/t2.mp3"],
            "one result per track, keyed by TrackId"
        );
        assert!(results.iter().all(|(_, cover)| cover.is_some()));

        // Everything was delivered exactly once; nothing remains to poll.
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            h.service.poll().is_empty(),
            "poll must be empty once everything drained"
        );
    }
}

/// The sixth Session Projection: user playlists read through the seam
/// (ADR 0002). These tests drive `SessionViews` against a real `SQLite`
/// scratch store at the infra seam — the same house pattern as the store
/// tests — because the property under test is that committed store
/// mutations show up on the next view call with zero caller action.
mod playlist_projection_tests {
    use super::*;
    use riff_backend::app::errors::StoreError;
    use riff_backend::app::store::{LibraryMutationStore, PlaylistEntry, PlaylistStore};
    use riff_backend::infra::store::SqliteStore;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counting [`PlaylistStore`] decorator: delegates everything to the
    /// real store handle while counting entry-list reads, so tests can tell
    /// "served from cache" apart from "hit the store".
    struct CountingPlaylistStore {
        inner: SqliteStore,
        entry_loads: Arc<AtomicUsize>,
    }

    impl CountingPlaylistStore {
        fn new(inner: SqliteStore) -> (Self, Arc<AtomicUsize>) {
            let entry_loads = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    inner,
                    entry_loads: Arc::clone(&entry_loads),
                },
                entry_loads,
            )
        }
    }

    impl PlaylistStore for CountingPlaylistStore {
        fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
            self.inner.load_playlists()
        }

        fn load_playlist_entries(&self, id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError> {
            self.entry_loads.fetch_add(1, Ordering::SeqCst);
            self.inner.load_playlist_entries(id)
        }

        fn create_playlist(
            &mut self,
            name: &str,
            initial_tracks: &[TrackId],
        ) -> Result<PlaylistId, StoreError> {
            self.inner.create_playlist(name, initial_tracks)
        }

        fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, StoreError> {
            self.inner.rename_playlist(id, new_name)
        }

        fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError> {
            self.inner.delete_playlist(id)
        }

        fn add_playlist_entry(
            &mut self,
            id: &PlaylistId,
            track: &TrackId,
        ) -> Result<bool, StoreError> {
            self.inner.add_playlist_entry(id, track)
        }

        fn remove_playlist_entries(
            &mut self,
            id: &PlaylistId,
            track: &TrackId,
        ) -> Result<bool, StoreError> {
            self.inner.remove_playlist_entries(id, track)
        }

        fn reorder_playlist_entries(
            &mut self,
            id: &PlaylistId,
            ordered: &[TrackId],
        ) -> Result<bool, StoreError> {
            self.inner.reorder_playlist_entries(id, ordered)
        }
    }

    /// One scratch Application Store plus a `SessionViews` wired to it
    /// through clones of the real store handle, with one clone kept out for
    /// seeding and direct store mutations.
    struct Scratch {
        /// Keeps the scratch database (and the seeded audio files) alive for
        /// the whole test; read by `seed_track` for file placement.
        dir: tempfile::TempDir,
        shared: SqliteStore,
        mutations: SqliteStore,
        views: riff_backend::app::views::SessionViews,
        entry_loads: Arc<AtomicUsize>,
    }

    impl Scratch {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db_path = dir.path().join("riff.sqlite3");
            let (changes_tx, _changes_rx) =
                crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
            let store = SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("fresh store must open and migrate");
            let mutations = store.clone();
            let (playlist_queries, entry_loads) = CountingPlaylistStore::new(mutations.clone());
            let views = riff_backend::app::views::SessionViews::new(
                Box::new(store.clone()),
                Box::new(playlist_queries),
                store.library_generation(),
                store.playlist_generation(),
            );
            Self {
                dir,
                shared: store,
                mutations,
                views,
                entry_loads,
            }
        }

        /// How often the facade read playlist entries through the store.
        fn entry_loads(&self) -> usize {
            self.entry_loads.load(Ordering::SeqCst)
        }

        /// Create a real audio file on disk and index it into the Library,
        /// so the entry resolution's filesystem check passes.
        fn seed_track(&mut self, name: &str) -> Track {
            let path = self.dir.path().join(name);
            std::fs::write(&path, b"fake audio bytes").expect("scratch file writes");
            let track = crate::test_utils::create_test_track_with_metadata(
                &path.to_string_lossy(),
                &path.to_string_lossy(),
                "Artist",
                name,
                "Album",
            );
            self.shared
                .apply_scan_batch(std::slice::from_ref(&track))
                .expect("seed scan commits");
            track
        }
    }

    #[test]
    fn test_initial_fetch_serves_seeded_playlists_and_resolved_rows() {
        let mut scratch = Scratch::new();
        let t1 = scratch.seed_track("one.mp3");
        let dangling = TrackId("f:\\nowhere\\gone.mp3".to_string());
        let pid = scratch
            .mutations
            .create_playlist("Focus Mix", &[t1.id.clone(), dangling.clone()])
            .expect("create works");

        // First call fetches through the store: the seeded playlist list...
        let playlists = scratch.views.playlists();
        assert_eq!(playlists.len(), 1, "the seeded playlist is listed");
        assert_eq!(playlists[0].id, pid);
        assert_eq!(playlists[0].name, "Focus Mix");

        // ...and its ready-to-render rows: the known track resolves with
        // metadata and a valid verdict; the dangling reference stays listed
        // as (id, None, false) per ADR 0001.
        let view = scratch
            .views
            .playlist_view(&pid)
            .expect("known id yields a view");
        assert_eq!(view.rows.len(), 2, "dangling entries stay listed");
        assert_eq!(view.rows[0].0, t1.id);
        assert_eq!(
            view.rows[0].1.as_ref().map(|track| track.id.clone()),
            Some(t1.id.clone()),
            "the Library-known track resolves"
        );
        assert!(view.rows[0].2, "the seeded file exists so the row is valid");
        assert_eq!(view.rows[1].0, dangling);
        assert!(view.rows[1].1.is_none(), "unknown tracks resolve to None");
        assert!(!view.rows[1].2, "a dangling reference is flagged invalid");
        assert_eq!(
            view.valid_ids.as_ref(),
            std::slice::from_ref(&t1.id),
            "only playable ids make valid_ids"
        );
    }

    #[test]
    fn test_store_mutations_appear_on_the_next_call_with_zero_caller_action() {
        let mut scratch = Scratch::new();
        let t1 = scratch.seed_track("one.mp3");
        let t2 = scratch.seed_track("two.mp3");
        let pid = scratch
            .mutations
            .create_playlist("Gym", std::slice::from_ref(&t1.id))
            .expect("create works");

        // Warm both projections at the current generation.
        let first = scratch
            .views
            .playlist_view(&pid)
            .expect("known id yields a view");
        assert_eq!(first.rows.len(), 1);
        let _ = scratch.views.playlists();

        // Mutate through the store directly — no invalidation call, no cache
        // clearing, nothing but the committed mutations.
        assert!(
            scratch.mutations.add_playlist_entry(&pid, &t2.id).unwrap(),
            "the add commits"
        );
        assert!(
            scratch
                .mutations
                .reorder_playlist_entries(&pid, &[t2.id.clone(), t1.id.clone()])
                .unwrap(),
            "the reorder commits"
        );
        assert!(
            scratch.mutations.rename_playlist(&pid, "Workout").unwrap(),
            "the rename commits"
        );

        // The very next reads reflect the committed state.
        let playlists = scratch.views.playlists();
        assert_eq!(playlists[0].name, "Workout", "the rename is visible");
        assert_eq!(
            playlists[0].tracks,
            vec![t2.id.clone(), t1.id.clone()],
            "the reorder is visible"
        );

        let view = scratch
            .views
            .playlist_view(&pid)
            .expect("known id yields a view");
        assert_eq!(view.rows.len(), 2, "the added entry is visible");
        assert_eq!(view.rows[0].0, t2.id, "rows follow the new order");
        assert_eq!(view.rows[1].0, t1.id);
        assert_eq!(
            view.valid_ids.as_ref(),
            &[t2.id.clone(), t1.id.clone()],
            "valid_ids follow the new order too"
        );
    }

    #[test]
    fn test_unknown_playlist_id_yields_none_without_a_third_method() {
        let mut scratch = Scratch::new();
        let t1 = scratch.seed_track("one.mp3");
        let pid = scratch
            .mutations
            .create_playlist("Known", std::slice::from_ref(&t1.id))
            .expect("create works");

        assert!(
            scratch.views.playlist_view(&pid).is_some(),
            "a known id yields a view"
        );

        // An unknown id answers None — and never touches the entry loader,
        // the playlists list alone decides membership.
        let loads_before = scratch.entry_loads();
        let unknown = PlaylistId("playlist-does-not-exist".to_string());
        assert!(scratch.views.playlist_view(&unknown).is_none());
        assert_eq!(
            scratch.entry_loads(),
            loads_before,
            "unknown ids resolve without querying entries"
        );
    }
}

// Playback Coordinator (CONTEXT.md): applies Playback Updates to session
// state and owns playback continuation. These tests drive the synchronous
// core (`PlaybackCoordinator::apply_update`) directly — no threads, no real
// store — over the shared recording mocks.
//
// The shared-handle adapter below exists because the coordinator takes
// ownership of its mutation port: it delegates every call to the real mock
// behind a mutex so the mock's recordings remain inspectable after wiring.
mod playback_coordinator_tests {
    use super::*;
    use crate::mocks::MockLibraryMutationStore;
    use crossbeam_channel::{Receiver, unbounded};
    use riff_backend::app::errors::StoreError;
    use riff_backend::app::playback_coordinator::PlaybackCoordinator;
    use riff_backend::app::store::LibraryMutationStore;
    use std::path::Path;

    /// [`LibraryMutationStore`] view over one recording
    /// [`MockLibraryMutationStore`] owned by the coordinator; the test keeps
    /// the other handle and inspects the recordings afterwards.
    struct SharedMutations(Arc<Mutex<MockLibraryMutationStore>>);

    impl LibraryMutationStore for SharedMutations {
        fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError> {
            self.0.lock().unwrap().apply_scan_batch(tracks)
        }

        fn record_track_played(
            &mut self,
            id: &TrackId,
            played_at: std::time::SystemTime,
        ) -> Result<bool, StoreError> {
            self.0.lock().unwrap().record_track_played(id, played_at)
        }

        fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError> {
            self.0.lock().unwrap().apply_tag_refresh(track)
        }

        fn remove_library_path(&mut self, root: &Path) -> Result<usize, StoreError> {
            self.0.lock().unwrap().remove_library_path(root)
        }

        fn clear_library(&mut self) -> Result<usize, StoreError> {
            self.0.lock().unwrap().clear_library()
        }
    }

    /// One wired coordinator plus every handle a test inspects: the shared
    /// playback and library sessions (held in two separate `Arc<Mutex<>>`s so
    /// the test never holds both locks at once), the command receiver (what
    /// the engine would receive), the playback-error notice receiver (what
    /// the facade would receive), and the mutation recordings.
    struct Harness {
        state: Arc<Mutex<PlaybackSession>>,
        library: Arc<Mutex<LibrarySession>>,
        cmd_rx: Receiver<PlaybackCommand>,
        notice_rx: Receiver<String>,
        mutations: Arc<Mutex<MockLibraryMutationStore>>,
        coordinator: PlaybackCoordinator,
    }

    /// Wire a harness whose queue holds `ids` with `current_index` and
    /// `repeat` preset. No threads: tests call `apply_update` directly. The
    /// update channel stays empty (its sender drops immediately) because the
    /// core is driven by hand.
    fn harness(ids: &[&str], current_index: Option<usize>, repeat: RepeatMode) -> Harness {
        let state = Arc::new(Mutex::new(PlaybackSession::default()));
        {
            let mut locked = state.lock_or_recover();
            locked.queue =
                PlaybackQueue::new(ids.iter().map(|id| TrackId((*id).to_string())).collect());
            locked.queue.current_index = current_index;
            locked.queue.repeat = repeat;
        }
        let library = Arc::new(Mutex::new(LibrarySession::default()));
        let (cmd_tx, cmd_rx) = unbounded();
        let (notice_tx, notice_rx) = unbounded();
        let mutations = Arc::new(Mutex::new(MockLibraryMutationStore::new()));
        let coordinator = PlaybackCoordinator::new(
            state.clone(),
            unbounded().1,
            cmd_tx,
            Box::new(SharedMutations(mutations.clone())),
            notice_tx,
        );
        Harness {
            state,
            library,
            cmd_rx,
            notice_rx,
            mutations,
            coordinator,
        }
    }

    #[test]
    fn test_playback_coordinator_commits_history_before_advancing() {
        let mut h = harness(&["a.mp3", "b.mp3"], Some(0), RepeatMode::None);

        h.coordinator.apply_update(PlaybackUpdate::TrackEnded);

        // The recorded play is the track that WAS current at commit time —
        // proof the store commit happened BEFORE the advance moved the index.
        let played = h.mutations.lock().unwrap().played();
        assert_eq!(played.len(), 1, "exactly one play commits");
        assert_eq!(
            played[0].0,
            TrackId("a.mp3".to_string()),
            "the finished track is committed, not the advanced-to one"
        );

        // ...and only then did the queue advance onto the next track.
        assert_eq!(
            h.state.lock_or_recover().queue.current_index,
            Some(1),
            "the queue advances after the commit"
        );

        // The engine was told to play the advanced-to track.
        assert_eq!(
            h.cmd_rx.recv().expect("a Play command follows"),
            PlaybackCommand::Play(TrackId("b.mp3".to_string()))
        );
    }

    #[test]
    fn test_playback_coordinator_repeat_one_replays_current_without_advancing() {
        let mut h = harness(&["a.mp3", "b.mp3"], Some(0), RepeatMode::One);

        h.coordinator.apply_update(PlaybackUpdate::TrackEnded);

        // History still commits for the finished track...
        let played = h.mutations.lock().unwrap().played();
        assert_eq!(played.len(), 1, "repeat-one still records the play");
        assert_eq!(played[0].0, TrackId("a.mp3".to_string()));

        // ...the queue index deliberately does NOT move...
        assert_eq!(
            h.state.lock_or_recover().queue.current_index,
            Some(0),
            "repeat-one never advances the queue index"
        );

        // ...and the SAME track is re-sent for playback; the engine's dedup
        // guard swallows this no-op if gapless handoff already happened.
        assert_eq!(
            h.cmd_rx
                .recv()
                .expect("repeat-one re-plays the current track"),
            PlaybackCommand::Play(TrackId("a.mp3".to_string()))
        );
    }

    #[test]
    fn test_playback_coordinator_advance_past_last_stops_without_play() {
        let mut h = harness(&["a.mp3"], Some(0), RepeatMode::None);

        h.coordinator.apply_update(PlaybackUpdate::TrackEnded);

        // Nothing follows the last track: no Play command reaches the engine...
        assert!(
            h.cmd_rx.try_recv().is_err(),
            "no Play command when nothing follows"
        );

        // ...and playback stops.
        assert_eq!(
            h.state.lock_or_recover().playback_state,
            PlaybackState::Stopped
        );
    }

    #[test]
    fn test_playback_coordinator_error_sets_stopped_and_emits_typed_notice() {
        let mut h = harness(&["a.mp3"], Some(0), RepeatMode::None);

        h.coordinator
            .apply_update(PlaybackUpdate::Error("boom".to_string()));

        {
            let locked = h.state.lock_or_recover();
            assert_eq!(locked.playback_state, PlaybackState::Stopped);
        }
        // The cross-slice state write is gone: the coordinator no longer
        // touches the library session's scan-status slot. Lock and drop the
        // library session on its own — never both session guards at once.
        {
            let locked = h.library.lock_or_recover();
            assert_eq!(
                locked.scan_status, None,
                "playback errors must not write the library session's scan-status slot"
            );
        }
        // Instead the error surfaces as a notice over the facade's channel,
        // preserving the exact user-facing string format.
        assert_eq!(
            h.notice_rx.try_recv().as_deref(),
            Ok("Playback error: boom"),
            "the exact user-facing string format is preserved"
        );
    }

    #[test]
    fn test_playback_coordinator_track_changed_updates_current_index_by_id() {
        let mut h = harness(&["a.mp3", "b.mp3", "c.mp3"], Some(0), RepeatMode::None);

        h.coordinator
            .apply_update(PlaybackUpdate::TrackChanged(TrackId("c.mp3".to_string())));

        // The index relocates by identity lookup, not by stepping.
        assert_eq!(h.state.lock_or_recover().queue.current_index, Some(2));
    }
}

/// The one generic generation-keyed cache behind every Session Projection
/// (ADR 0002). These property tests pin the primitive ONCE so the
/// per-projection suites stay thin descriptor/mapping smokes: a fresh load
/// stamps the observed epoch, an epoch move (or key change) drops the entry,
/// and a failed load — which by contract never reaches `store`/`slot` —
/// leaves the stale-but-present value readable for the retry.
mod generation_cache_tests {
    use riff_backend::app::store::StoreGeneration;
    use riff_backend::app::views::GenerationCache;

    #[test]
    fn test_fresh_load_stamps_the_observed_epoch() {
        let counter = StoreGeneration::new();
        let mut cache = GenerationCache::<String, i32>::new(counter.clone());

        // A fresh cache holds nothing at any epoch.
        let epoch = counter.current();
        assert!(!cache.loaded_at(epoch), "a new cache starts empty");
        assert!(cache.peek().is_none());

        // Committing a load stamps exactly the observed epoch and key.
        cache.store(epoch, "k".to_string(), 7);
        assert!(cache.loaded_at(epoch));
        assert!(cache.holds(epoch, &"k".to_string()));
        assert_eq!(cache.peek(), Some(&7));
    }

    #[test]
    fn test_epoch_move_drops_the_entry_and_slot_reinitializes() {
        let counter = StoreGeneration::new();
        let mut cache = GenerationCache::<(), i32>::new(counter.clone());
        let epoch = counter.current();
        cache.store(epoch, (), 7);

        // A committed mutation bumps the counter: nothing holds at the NEW
        // observed epoch, while the entry keeps its old stamp readable for
        // the stale-but-present fallback.
        counter.bump();
        let moved = counter.current();
        assert_ne!(moved, epoch);
        assert!(
            !cache.loaded_at(moved),
            "nothing counts as loaded at the new epoch yet"
        );
        assert!(
            cache.loaded_at(epoch),
            "the entry still remembers the epoch it was loaded at"
        );

        // The commit step for the new epoch hands out a FRESH default slot:
        // the previous generation's value must not leak through.
        let slot = cache.slot(moved, &());
        assert_eq!(*slot, 0, "the stale value is dropped, not served");
        *slot = 42;
        assert_eq!(cache.peek(), Some(&42));
        assert!(cache.loaded_at(moved));
    }

    #[test]
    fn test_key_change_at_the_same_epoch_resets_the_slot() {
        let counter = StoreGeneration::new();
        let mut cache = GenerationCache::<String, Vec<i32>>::new(counter.clone());
        let epoch = counter.current();
        cache.store(epoch, "a".to_string(), vec![1]);

        // Same epoch, different key: not a hit, and the slot reinitializes.
        assert!(!cache.holds(epoch, &"b".to_string()));
        let slot = cache.slot(epoch, &"b".to_string());
        assert!(
            slot.is_empty(),
            "entries are keyed: another key never sees the first key's rows"
        );
    }

    #[test]
    fn test_failed_load_keeps_the_stale_value_readable_for_the_retry() {
        let counter = StoreGeneration::new();
        let mut cache = GenerationCache::<(), String>::new(counter.clone());
        let epoch = counter.current();
        cache.store(epoch, (), "good".to_string());

        // A failed load touches nothing (the loader's `?` returns before any
        // store/slot call), so after the bump the stale-but-present value…
        counter.bump();
        let moved = counter.current();
        assert_eq!(
            cache.peek(),
            Some(&"good".to_string()),
            "stale-but-present beats blank while the caller retries"
        );

        // …survives until the successful retry commits over it.
        cache.store(moved, (), "reloaded".to_string());
        assert_eq!(cache.peek(), Some(&"reloaded".to_string()));
    }

    #[test]
    fn test_invalidate_and_take_value_drop_whatever_is_cached() {
        let counter = StoreGeneration::new();
        let mut cache = GenerationCache::<(), i32>::new(counter.clone());
        let epoch = counter.current();
        cache.store(epoch, (), 5);

        // take_value steals regardless of epoch (fetch-then-swap merges).
        assert_eq!(cache.take_value(), Some(5));
        assert_eq!(cache.peek(), None);

        // invalidate clears unconditionally too.
        cache.store(epoch, (), 6);
        cache.invalidate();
        assert_eq!(cache.peek(), None);
        assert!(!cache.loaded_at(epoch));
    }
}
