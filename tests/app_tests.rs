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

    // --- playlists: entry validity (read-time, app layer) -------------------------
    //
    // Playlist persistence and CRUD moved behind the `PlaylistStore` port;
    // their behavior is tested against real SQL in `infra_tests.rs`. What
    // stays here is the read-time validity rule the UI plays by, applied on
    // top of the store's LEFT JOIN entries-with-validity query.

    #[test]
    fn test_track_is_valid_and_valid_tracks_filter_invalid_entries() {
        use riff::app::store::PlaylistEntry;

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

    /// Minimal [`MetadataReader`](riff::app::traits::MetadataReader) for
    /// exercising scan-side Track construction without real audio files.
    /// `fail` simulates unreadable/corrupt input.
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

    // --- build_tracks over a mock MetadataReader (scan-side Track construction) ----
    //
    // The former mirror's `scan_and_add_tracks` is gone; what survives is the
    // app-layer Track construction the store-backed scan flow commits.

    #[test]
    fn test_build_tracks_reads_metadata_and_audio_format() {
        let reader = MockMetadataReader { fail: false };
        let tracks = riff::app::scan::build_tracks(
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
        let tracks = riff::app::scan::build_tracks(
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
        let tracks = riff::app::scan::build_tracks(vec![PathBuf::from("scan/fresh.mp3")], &reader);

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
        assert!(matches!(err, AppError::MetadataWrite(_)));
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

    // --- Playback Projection: current track + Up Next window ----------------
    //
    // Serves the window title, the playerbar cover, the Now Playing stage,
    // and the track-details panel off cached store rows. Invalidated by
    // generation bumps AND Playback Queue changes (a TrackChanged advance,
    // Next/Previous/PlayNext/AddToQueue).

    use riff::app::projection::PlaybackProjection;

    /// Counting fake resolver standing in for the store's `get_track`
    /// query: serves a canned library, records every call, fails on demand.
    struct FakePlaybackStore {
        tracks: HashMap<TrackId, Track>,
        calls: Vec<TrackId>,
        fail_next: bool,
    }

    impl FakePlaybackStore {
        fn new(tracks: HashMap<TrackId, Track>) -> Self {
            Self {
                tracks,
                calls: Vec::new(),
                fail_next: false,
            }
        }

        fn get(&mut self, id: &TrackId) -> Result<Option<Track>, AppError> {
            self.calls.push(id.clone());
            if std::mem::take(&mut self.fail_next) {
                return Err(AppError::InvalidOperation("store boom".to_string()));
            }
            Ok(self.tracks.get(id).cloned())
        }
    }

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
    fn test_playback_projection_loads_current_and_up_next_once_per_inputs() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let queue = playback_queue();

        for _ in 0..2 {
            projection
                .refresh(1, &queue, 5, &mut |id| store.get(id))
                .expect("first refresh loads");
        }

        assert_eq!(
            projection.current().map(|t| t.id.0.as_str()),
            Some("t1.mp3")
        );
        let up_next: Vec<&str> = projection
            .up_next()
            .iter()
            .map(|t| t.id.0.as_str())
            .collect();
        assert_eq!(
            up_next,
            vec!["t2.mp3", "t3.mp3", "t4.mp3"],
            "Up Next lists the tracks after the current one, in queue order"
        );
        assert_eq!(
            store.calls.len(),
            4,
            "one load per distinct id (current + window); the fresh frame refetches nothing"
        );
    }

    #[test]
    fn test_playback_projection_refetches_when_the_queue_advances() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let mut queue = playback_queue();
        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .unwrap();
        let calls_after_load = store.calls.len();

        // A TrackChanged advance moves the queue: same generation, but the
        // stamp moved, so the projection reloads.
        queue.advance();
        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .expect("reload after advance");

        assert!(
            store.calls.len() > calls_after_load,
            "a queue change invalidates the playback slots even at the same generation"
        );
        assert_eq!(
            projection.current().map(|t| t.id.0.as_str()),
            Some("t2.mp3")
        );
    }

    #[test]
    fn test_playback_projection_refetches_on_generation_bump() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let queue = playback_queue();
        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .unwrap();
        let calls_after_load = store.calls.len();

        projection
            .refresh(2, &queue, 5, &mut |id| store.get(id))
            .expect("reload after bump");

        assert!(
            store.calls.len() > calls_after_load,
            "a committed mutation invalidates the playback slots"
        );
    }

    #[test]
    fn test_playback_projection_follows_the_queue_shuffle_order() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let mut queue = playback_queue();
        queue.shuffle = true;
        // A hand-seeded shuffle order (indices into tracks): t4 then t2 —
        // the window must mirror the QUEUE's order, not the append order.
        queue.shuffled_indices = vec![3, 1];

        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .unwrap();

        let up_next: Vec<&str> = projection
            .up_next()
            .iter()
            .map(|t| t.id.0.as_str())
            .collect();
        assert_eq!(up_next, vec!["t4.mp3", "t2.mp3"]);
    }

    #[test]
    fn test_playback_projection_skips_ids_missing_from_the_store() {
        let mut library = playback_library();
        library.remove(&TrackId("t3.mp3".to_string()));
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(library);
        let queue = playback_queue();

        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .unwrap();

        let up_next: Vec<&str> = projection
            .up_next()
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
    fn test_playback_projection_empty_queue_loads_nothing() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let queue = PlaybackQueue::default();

        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .unwrap();

        assert!(projection.current().is_none());
        assert!(projection.up_next().is_empty());
        assert_eq!(store.calls, Vec::new(), "nothing to resolve, nothing asked");
    }

    #[test]
    fn test_playback_projection_error_keeps_stale_cache_and_retries() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let mut queue = playback_queue();
        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .unwrap();

        // A failing reload (mid-scan store hiccup) propagates and leaves the
        // previous cache completely untouched.
        queue.advance();
        store.fail_next = true;
        let err = projection.refresh(1, &queue, 5, &mut |id| store.get(id));
        assert!(err.is_err(), "loader errors propagate");
        assert_eq!(
            projection.current().map(|t| t.id.0.as_str()),
            Some("t1.mp3"),
            "stale-but-present beats blank while the UI retries"
        );

        projection
            .refresh(1, &queue, 5, &mut |id| store.get(id))
            .expect("the next frame retries");
        assert_eq!(
            projection.current().map(|t| t.id.0.as_str()),
            Some("t2.mp3")
        );
    }

    #[test]
    fn test_playback_projection_selected_track_caches_until_selection_or_generation_moves() {
        let mut projection = PlaybackProjection::new();
        let mut store = FakePlaybackStore::new(playback_library());
        let t9 = TrackId("t9.mp3".to_string());

        for _ in 0..2 {
            let _ = projection
                .selected_track(1, &t9, &mut |id| store.get(id))
                .expect("selection resolves");
        }
        assert_eq!(
            store.calls.len(),
            1,
            "an unchanged selection at an unchanged generation never requeries"
        );

        // A different selection refetches…
        let other = TrackId("t2.mp3".to_string());
        let _ = projection
            .selected_track(1, &other, &mut |id| store.get(id))
            .unwrap();
        assert_eq!(store.calls.len(), 2);

        // …and so does the same selection after a committed mutation.
        let _ = projection
            .selected_track(2, &other, &mut |id| store.get(id))
            .unwrap();
        assert_eq!(store.calls.len(), 3);

        // An absent track caches its negative result too.
        let missing = TrackId("gone.mp3".to_string());
        for _ in 0..2 {
            let resolved = projection
                .selected_track(2, &missing, &mut |id| store.get(id))
                .expect("absence resolves");
            assert!(resolved.is_none());
        }
        assert_eq!(store.calls.len(), 4, "a dangling selection is asked once");
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
