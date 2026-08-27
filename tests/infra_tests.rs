// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocks::{MockAudioDecoder, MockAudioOutput, MockCoverLoader, MockMetadataReader};
    use riff_backend::app::errors::AppError;
    use riff_backend::app::traits::{
        AudioDecoder, AudioFormatInfo, AudioOutput, CoverImage, CoverLoader, MetadataReader,
        MetadataWriter, TagEdit,
    };
    use riff_backend::domain::CoverSource;
    use std::path::PathBuf;
    use std::time::Duration;

    fn test_format() -> AudioFormatInfo {
        AudioFormatInfo {
            sample_rate: 44_100,
            channels: 2,
            duration: Some(Duration::from_secs(1)),
        }
    }

    // --- ReplayGain tag parsing (pure) -----------------------------------------

    #[test]
    fn test_parse_replaygain_gain_strips_db_suffix() {
        assert_eq!(parse_replaygain_gain("-6.54 dB"), Some(-6.54));
        assert_eq!(parse_replaygain_gain("+3.21 dB"), Some(3.21));
        // Case-insensitive unit, with or without the space.
        assert_eq!(parse_replaygain_gain("3.21db"), Some(3.21));
        assert_eq!(parse_replaygain_gain("-1.5DB"), Some(-1.5));
    }

    #[test]
    fn test_parse_replaygain_gain_accepts_bare_number() {
        assert_eq!(parse_replaygain_gain("3.21"), Some(3.21));
        assert_eq!(parse_replaygain_gain(" 0.0 "), Some(0.0));
    }

    #[test]
    fn test_parse_replaygain_gain_rejects_garbage() {
        assert_eq!(parse_replaygain_gain("garbage"), None);
        assert_eq!(parse_replaygain_gain(""), None);
        assert_eq!(parse_replaygain_gain("dB"), None);
    }

    // --- AudioDecoder boundary behavior (via MockAudioDecoder) ----------------

    #[test]
    fn test_decoder_open_yields_scripted_samples_then_eof() {
        let mut decoder = MockAudioDecoder::new(test_format())
            .with_batches(vec![vec![0.1, 0.2], vec![0.3, 0.4, 0.5]]);
        decoder.duration = Some(Duration::from_secs(1));
        let path = PathBuf::from("music/song.flac");

        let info = decoder.open(&path).expect("open should succeed");
        assert_eq!(info.sample_rate, 44_100);
        assert_eq!(info.channels, 2);
        assert_eq!(decoder.opened, vec![path]);
        assert_eq!(decoder.duration(), Some(Duration::from_secs(1)));

        // Scripted batches come out in order, then EOF (`Ok(0)`).
        let mut out = [0.0f32; 4];
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 2);
        assert_eq!(out[..2], [0.1, 0.2]);
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 3);
        assert_eq!(out[..3], [0.3, 0.4, 0.5]);
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 0);
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 0);

        decoder.close();
        assert!(decoder.closed);
    }

    #[test]
    fn test_decoder_seek_is_recorded_and_resets_the_stream() {
        let mut decoder =
            MockAudioDecoder::new(test_format()).with_batches(vec![vec![1.0], vec![2.0]]);
        let path = PathBuf::from("song.mp3");
        decoder.open(&path).unwrap();

        // Drain the first batch, then seek: the stream restarts from the
        // beginning of the script and the seek position is recorded.
        let mut out = [0.0f32; 1];
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 1);
        assert!(crate::test_utils::float_close(out[0], 1.0));
        decoder.seek(Duration::from_secs(5)).unwrap();
        assert_eq!(decoder.seeks, vec![Duration::from_secs(5)]);
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 1);
        assert!(crate::test_utils::float_close(out[0], 1.0));
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 1);
        assert!(crate::test_utils::float_close(out[0], 2.0));
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 0);
    }

    #[test]
    fn test_decoder_injected_open_and_decode_errors_surface_as_app_error() {
        let mut decoder = MockAudioDecoder::new(test_format());
        decoder.open_error = Some("unsupported codec".to_string());
        let err = decoder
            .open(&PathBuf::from("bad.ogg"))
            .expect_err("open must fail");
        assert!(matches!(err, AppError::Decode(_)));
        assert!(err.to_string().contains("unsupported codec"));
        assert!(decoder.opened.is_empty());

        // Decode errors surface mid-stream without losing earlier samples.
        let mut decoder = MockAudioDecoder::new(test_format()).with_batches(vec![vec![0.5]]);
        decoder.open(&PathBuf::from("ok.ogg")).unwrap();
        let mut out = [0.0f32; 1];
        assert_eq!(decoder.next_frames(&mut out).unwrap(), 1);
        assert!(crate::test_utils::float_close(out[0], 0.5));
        decoder.decode_error = Some("corrupt frame".to_string());
        assert!(matches!(
            decoder.next_frames(&mut out).unwrap_err(),
            AppError::Decode(_)
        ));
    }

    // --- AudioOutput boundary behavior (via MockAudioOutput) ------------------

    #[test]
    fn test_output_write_accumulates_buffer_and_controls_are_recorded() {
        let mut output = MockAudioOutput::new();
        output.initialize(48_000, 2).unwrap();
        output.start().unwrap();

        assert_eq!(output.write_samples(&[0.1, 0.2, 0.3]).unwrap(), 3);
        assert_eq!(output.write_samples(&[0.4]).unwrap(), 1);
        assert_eq!(output.buffer_len(), 4);
        assert_eq!(output.written.len(), 2);

        output.clear_buffer();
        assert_eq!(output.buffer_len(), 0);

        output.set_volume(0.5);
        output.stop().unwrap();

        assert_eq!(output.initialized, vec![(48_000, 2)]);
        assert_eq!(output.start_count, 1);
        assert_eq!(output.stop_count, 1);
        assert_eq!(output.clear_count, 1);
        assert_eq!(output.volumes, vec![0.5]);
    }

    #[test]
    fn test_output_injected_errors_propagate_and_skip_side_effects() {
        let mut output = MockAudioOutput::new();
        output.initialize_error = Some("no device".to_string());
        assert!(matches!(
            output.initialize(44_100, 2).unwrap_err(),
            AppError::AudioOutput(_)
        ));
        assert!(output.initialized.is_empty());

        output.initialize_error = None;
        output.initialize(44_100, 2).unwrap();
        output.write_error = Some("device lost".to_string());
        assert!(matches!(
            output.write_samples(&[0.1, 0.2]).unwrap_err(),
            AppError::AudioOutput(_)
        ));
        // A failed write must not land in the buffer.
        assert_eq!(output.buffer_len(), 0);
        assert!(output.written.is_empty());
    }

    // --- MetadataReader boundary behavior (via MockMetadataReader) ------------

    #[test]
    fn test_metadata_reader_read_all_aggregates_canned_values() {
        let reader = MockMetadataReader {
            metadata: TrackMetadata {
                title: Some("Song".to_string()),
                artist: Some("Artist".to_string()),
                ..Default::default()
            },
            cover_source: CoverSource::Embedded(vec![1, 2, 3].into()),
            ..Default::default()
        };

        let (metadata, duration, cover, format) = reader
            .read_all(&PathBuf::from("a.mp3"))
            .expect("read_all should succeed");
        assert_eq!(metadata.title.as_deref(), Some("Song"));
        assert_eq!(metadata.artist.as_deref(), Some("Artist"));
        assert_eq!(duration, Some(Duration::from_secs(90)));
        assert!(matches!(cover, CoverSource::Embedded(ref data) if data.as_ref() == [1, 2, 3]));
        assert_eq!(format.sample_rate, 44_100);

        // Individual accessors agree with read_all.
        assert_eq!(
            reader.read_metadata(&PathBuf::from("a.mp3")).unwrap(),
            reader.metadata
        );
        assert_eq!(
            reader.read_duration(&PathBuf::from("a.mp3")).unwrap(),
            Some(Duration::from_secs(90))
        );
    }

    #[test]
    fn test_metadata_reader_injected_error_propagates_from_every_method() {
        let reader = MockMetadataReader {
            fail: true,
            ..Default::default()
        };
        let path = PathBuf::from("corrupt.flac");

        assert!(matches!(
            reader.read_metadata(&path).unwrap_err(),
            AppError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_duration(&path).unwrap_err(),
            AppError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_cover_source(&path).unwrap_err(),
            AppError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_audio_format(&path).unwrap_err(),
            AppError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_all(&path).unwrap_err(),
            AppError::MetadataRead(_)
        ));
    }

    // --- CoverLoader boundary behavior (via MockCoverLoader) ------------------

    #[test]
    fn test_cover_loader_returns_image_none_or_error_per_configuration() {
        let image = CoverImage {
            width: 2,
            height: 2,
            rgba: vec![255; 16],
        };
        let with_image = MockCoverLoader {
            result: Ok(Some(image.clone())),
        };
        let loaded = with_image
            .load_cover(&CoverSource::None)
            .unwrap()
            .expect("image expected");
        assert_eq!(loaded.width, 2);
        assert_eq!(loaded.rgba.len(), 16);

        let empty = MockCoverLoader { result: Ok(None) };
        assert!(empty.load_cover(&CoverSource::None).unwrap().is_none());

        let failing = MockCoverLoader {
            result: Err("decode failed".to_string()),
        };
        let err = failing
            .load_cover(&CoverSource::Embedded(vec![9].into()))
            .unwrap_err();
        assert!(matches!(err, AppError::CoverLoad(_)));
        assert!(err.to_string().contains("decode failed"));
    }

    // --- Metadata tag writing on real files (REQ-ML-008) ------------------------
    //
    // The file on disk is the source of truth; these tests write tags with the
    // real `LoftyMetadataWriter`, then re-read the same file (through the real
    // `LoftyMetadataReader` in one case, raw lofty in another) to prove the new
    // tags actually landed.

    /// Write a tiny but fully valid PCM WAV file (0.1 s of mono 8 kHz audio)
    /// so the tag writer/reader have a real audio file to work on.
    fn write_minimal_wav(path: &std::path::Path) {
        const SAMPLES: u32 = 800; // 0.1 s at 8 kHz
        let data_size = SAMPLES * 2; // 16-bit mono
        let mut bytes = Vec::with_capacity(44 + data_size as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&8000u32.to_le_bytes()); // sample rate
        bytes.extend_from_slice(&16000u32.to_le_bytes()); // byte rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for i in 0..SAMPLES {
            let sample = ((i % 100) as i16).wrapping_mul(64);
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, bytes).expect("temp WAV fixture must be writable");
    }

    #[test]
    fn test_lofty_writer_full_edit_lands_on_disk_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.wav");
        write_minimal_wav(&path);

        let edit = TagEdit {
            title: Some("Kind of Blue".to_string()),
            artist: Some("Miles Davis".to_string()),
            album: Some("Kind of Blue (Legacy Edition)".to_string()),
            album_artist: Some("Miles Davis Sextet".to_string()),
            genre: Some("Jazz".to_string()),
            year: Some(1959),
            track_number: Some(1),
        };
        LoftyMetadataWriter::new()
            .write_metadata(&path, &edit)
            .expect("writing tags to a valid audio file must succeed");

        // Re-reading the file confirms the on-disk tags are the source of
        // truth and carry exactly what was written.
        let metadata = LoftyMetadataReader::new()
            .read_metadata(&path)
            .expect("re-reading the written file must succeed");
        assert_eq!(metadata.title.as_deref(), Some("Kind of Blue"));
        assert_eq!(metadata.artist.as_deref(), Some("Miles Davis"));
        assert_eq!(
            metadata.album.as_deref(),
            Some("Kind of Blue (Legacy Edition)")
        );
        assert_eq!(metadata.album_artist.as_deref(), Some("Miles Davis Sextet"));
        assert_eq!(metadata.genre.as_deref(), Some("Jazz"));
        assert_eq!(metadata.year, Some(1959));
        assert_eq!(metadata.track_number, Some(1));
    }

    #[test]
    fn test_lofty_writer_partial_edit_preserves_untouched_tags_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.wav");
        write_minimal_wav(&path);
        let writer = LoftyMetadataWriter::new();

        writer
            .write_metadata(
                &path,
                &TagEdit {
                    title: Some("Flamenco Sketches".to_string()),
                    artist: Some("Miles Davis".to_string()),
                    album: Some("Kind of Blue".to_string()),
                    album_artist: None,
                    genre: Some("Jazz".to_string()),
                    year: Some(1959),
                    track_number: Some(5),
                },
            )
            .expect("initial tag write must succeed");

        // A later edit touching only the title must not disturb the rest.
        writer
            .write_metadata(
                &path,
                &TagEdit {
                    title: Some("So What".to_string()),
                    ..Default::default()
                },
            )
            .expect("partial tag write must succeed");

        let metadata = LoftyMetadataReader::new().read_metadata(&path).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("So What"));
        // Untouched fields survive on disk.
        assert_eq!(metadata.artist.as_deref(), Some("Miles Davis"));
        assert_eq!(metadata.album.as_deref(), Some("Kind of Blue"));
        assert_eq!(metadata.genre.as_deref(), Some("Jazz"));
        assert_eq!(metadata.year, Some(1959));
        assert_eq!(metadata.track_number, Some(5));
    }

    #[test]
    fn test_lofty_writer_unsupported_file_returns_graceful_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "this is definitely not an audio file").unwrap();

        let result = LoftyMetadataWriter::new().write_metadata(
            &path,
            &TagEdit {
                title: Some("Nope".to_string()),
                ..Default::default()
            },
        );

        // Unsupported/corrupt input is reported as a normal write error —
        // no panic, no crash.
        assert!(matches!(
            result.expect_err("unsupported format must fail the write"),
            AppError::MetadataWrite(_)
        ));
    }

    #[test]
    fn test_lofty_writer_missing_file_returns_graceful_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("never-existed.wav");

        let result = LoftyMetadataWriter::new().write_metadata(&path, &TagEdit::default());

        assert!(matches!(
            result.expect_err("missing file must fail the write"),
            AppError::MetadataWrite(_)
        ));
    }

    // --- Construction smoke tests (kept: verify real infra types build) -------

    #[test]
    fn test_symphonia_decoder_new() {
        let mut codec_registry = symphonia::core::codecs::registry::CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut codec_registry);
        let _decoder = SymphoniaDecoder::new(codec_registry);
        // Decoder creation test - we can't test much more without actual audio files
    }

    #[test]
    fn test_lofty_metadata_reader_new() {
        let _reader = LoftyMetadataReader::new();
        // Reader creation test
    }

    #[test]
    fn test_image_cover_loader_new() {
        let _loader = ImageCoverLoader::new();
        // Loader creation test
    }

    #[test]
    fn test_audio_file_scanner_new() {
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let _scanner = AudioFileScanner::new(cancel_flag);
        // Scanner creation test
    }

    // --- Filesystem watcher: debounced batch shaping (pure) -------------------

    /// Build one synthetic debounced event carrying a single path.
    fn debounced_event(
        kind: notify::EventKind,
        path: &std::path::Path,
    ) -> notify_debouncer_full::DebouncedEvent {
        let mut event = notify::Event::new(kind);
        event.paths.push(path.to_path_buf());
        notify_debouncer_full::DebouncedEvent::new(event, std::time::Instant::now())
    }

    #[test]
    fn test_debounced_audio_paths_filters_and_dedupes_a_batch() {
        use notify::event::{CreateKind, DataChange, EventKind, ModifyKind};
        use std::path::Path;

        // One burst: an .mp3 created then modified (two raw events for the
        // same file), a non-audio file, and a case-insensitive extension hit.
        let events = vec![
            debounced_event(EventKind::Create(CreateKind::File), Path::new("/lib/a.mp3")),
            debounced_event(
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                Path::new("/lib/a.mp3"),
            ),
            debounced_event(
                EventKind::Create(CreateKind::File),
                Path::new("/lib/notes.txt"),
            ),
            debounced_event(
                EventKind::Create(CreateKind::File),
                Path::new("/lib/b.FLAC"),
            ),
        ];

        // Expected straight from the audio-extension contract: audio paths
        // only, deduplicated, first-seen order preserved.
        assert_eq!(
            riff_backend::infra::watcher::debounced_audio_paths(&events),
            vec![PathBuf::from("/lib/a.mp3"), PathBuf::from("/lib/b.FLAC")]
        );
    }

    #[test]
    fn test_filesystem_watcher_forwards_debounced_audio_batches() {
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<PathBuf>>();
        let mut watcher = FilesystemWatcher::new(tx).expect("watcher must build");
        watcher.watch(dir.path()).expect("watch must register");

        // One burst: an audio file and a non-audio file, written back to
        // back so the debouncer coalesces them into one flush.
        let song = dir.path().join("song.mp3");
        std::fs::write(&song, b"audio").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"text").unwrap();

        // The debouncer flushes once the filesystem has been quiet for its
        // 2s window; bound the wait so a wedged backend fails instead of
        // hanging the suite.
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut seen: Vec<PathBuf> = Vec::new();
        while !seen.contains(&song) {
            assert!(
                Instant::now() < deadline,
                "no batch containing {song:?} arrived; seen {seen:?}"
            );
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(batch) => seen.extend(batch),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    panic!("debouncer never flushed; seen {seen:?}")
                }
                Err(err) => panic!("event channel failed: {err}"),
            }
        }

        // Contract: which paths changed — audio only, non-audio filtered.
        assert!(
            !seen.iter().any(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
            }),
            "non-audio paths must not be forwarded; seen {seen:?}"
        );
    }

    #[test]
    fn test_filesystem_watcher_new() {
        // This test might fail if there are filesystem permissions issues
        let (tx, _) = crossbeam_channel::unbounded::<Vec<PathBuf>>();
        let _result = FilesystemWatcher::new(tx);
        // The result could be Ok or Err depending on the system
    }
}

// --- Application Store: connection setup + checksummed migrations --------

#[test]
fn test_store_fresh_start_creates_file_and_applies_initial_migration_once() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("fresh store must open and migrate");

    let applied: Vec<(i64, String)> = store
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect()
        })
        .expect("reading schema_migrations must work");
    // The shipped initial set: v1 (foundation) + v2 (typed settings tables)
    // + v3 (playlists) + v4 (library collection).
    assert_eq!(
        applied.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn test_store_reopen_reuses_migrated_store_without_reapplying() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    store
        .with_connection(|conn| conn.execute("UPDATE schema_migrations SET applied_at = 12345", []))
        .expect("marking applied_at must work");
    drop(store);

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening a migrated store must succeed");
    let applied_at: i64 = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT applied_at FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get(0),
            )
        })
        .expect("reading applied_at must work");
    assert_eq!(
        applied_at, 12345,
        "reopening must reuse the recorded migration row, not reapply"
    );
}

#[test]
fn test_store_double_apply_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    store
        .with_connection(|conn| conn.execute("UPDATE schema_migrations SET applied_at = 777", []))
        .expect("marking applied_at must work");

    // Applying the full migration set a second time in sequence must be
    // a no-op over the bookkeeping rows.
    store.apply_migrations().expect("second apply must succeed");

    let rows: Vec<(i64, String)> = store
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
            let mapped = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect()
        })
        .expect("reading schema_migrations must work");
    assert_eq!(rows.len(), 4, "no duplicate migration rows allowed");
    assert_eq!(
        rows.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );

    let applied_at: i64 = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT applied_at FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        applied_at, 777,
        "idempotent re-apply must not touch existing rows"
    );
}

#[test]
fn test_store_checksum_tamper_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    // Simulate a corrupted/tampered bookkeeping row: the recorded
    // checksum no longer matches the embedded migration.
    store
        .with_connection(|conn| {
            conn.execute("UPDATE schema_migrations SET checksum = 'deadbeef'", [])
        })
        .expect("tampering with the checksum must work");

    let err = store
        .apply_migrations()
        .expect_err("checksum mismatch must be a fatal startup error");
    assert!(
        err.to_string().contains("tampered"),
        "error must clearly name the checksum tamper: {err}"
    );

    // Nothing partially applied: the bookkeeping row is untouched.
    let rows: i64 = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE checksum = 'deadbeef'",
                [],
                |row| row.get(0),
            )
        })
        .expect("reading schema_migrations must work");
    assert_eq!(
        rows, 4,
        "all shipped migration rows must exist, none re-applied"
    );
}

#[test]
fn test_store_unopenable_path_is_a_clear_fatal_error() {
    // A path whose parent does not exist cannot be opened; the store
    // must report a clear error rather than panic or silently succeed.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("nope/should/fail.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let Err(err) = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
    else {
        panic!("unopenable store must fail, but opened successfully");
    };
    assert!(
        err.to_string().contains("failed to open Application Store"),
        "error must clearly name the open failure: {err}"
    );
}

#[test]
fn test_store_connection_setup_pragmas() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let journal_mode: String = store
        .with_connection(|conn| conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)))
        .expect("reading journal_mode must work");
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let synchronous: i64 = store
        .with_connection(|conn| conn.query_row("PRAGMA synchronous", [], |row| row.get(0)))
        .expect("reading synchronous must work");
    assert_eq!(synchronous, 1, "synchronous=NORMAL");

    let foreign_keys: i64 = store
        .with_connection(|conn| conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0)))
        .expect("reading foreign_keys must work");
    assert_eq!(foreign_keys, 1, "foreign_keys must be ON");
}

#[test]
fn test_store_busy_timeout_is_about_five_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let timeout_ms: i64 = store
        .with_connection(|conn| conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0)))
        .expect("reading busy_timeout must work");
    assert!(
        (4_000..=6_000).contains(&timeout_ms),
        "busy timeout should be roughly five seconds, got {timeout_ms}ms"
    );
}

// --- Application Store: corruption detection + automatic recovery ----------

#[test]
fn test_store_corrupt_db_reopens_as_fresh_store_with_siblings_renamed_aside() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let wal_path = dir.path().join("riff.sqlite3-wal");
    let shm_path = dir.path().join("riff.sqlite3-shm");

    // Build a healthy store with data, then close it so the WAL is
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO store_metadata(key, value) VALUES ('marker', 'pre-corruption')",
                [],
            )
        })
        .unwrap();
    drop(store);

    // Simulate crash leftovers: a stale WAL/SHM pair beside the database.
    std::fs::write(&wal_path, b"stale wal bytes").unwrap();
    std::fs::write(&shm_path, b"stale shm bytes").unwrap();

    // Corrupt the database bytes in place.
    let mut bytes = std::fs::read(&db_path).unwrap();
    assert!(
        bytes.len() > 200,
        "fixture must be large enough to corrupt meaningfully"
    );
    for byte in &mut bytes[100..200] {
        *byte = 0x00;
    }
    std::fs::write(&db_path, &bytes).unwrap();

    // Reopen: corruption must be detected at startup (quick_check) and
    // recovered automatically.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("corrupt store must be detected and replaced by a fresh one automatically");

    // Fresh start: pre-corruption data is intentionally not recovered.
    let marker: Option<String> = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT value FROM store_metadata WHERE key = 'marker'",
                [],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
        .expect("fresh store must be queryable");
    assert!(marker.is_none(), "fresh start must not carry old data");

    // The corrupted database plus its -wal/-shm siblings were preserved
    // beside the originals with Unix-nanosecond suffixed names.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();

    let db_asides: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("riff.sqlite3."))
        .collect();
    assert!(
        !db_asides.is_empty(),
        "corrupt db must be renamed aside: {entries:?}"
    );
    for aside in &db_asides {
        let suffix = aside.trim_start_matches("riff.sqlite3.");
        assert!(
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()),
            "aside suffix must be Unix nanoseconds: {aside}"
        );
    }

    let wal_asides: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("riff.sqlite3-wal."))
        .collect();
    assert_eq!(
        wal_asides.len(),
        1,
        "exactly one renamed-aside wal expected: {entries:?}"
    );
    let wal_suffix = wal_asides[0].trim_start_matches("riff.sqlite3-wal.");
    assert!(
        !wal_suffix.is_empty() && wal_suffix.chars().all(|c| c.is_ascii_digit()),
        "wal aside suffix must be Unix nanoseconds: {}",
        wal_asides[0]
    );

    let shm_asides: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("riff.sqlite3-shm."))
        .collect();
    assert_eq!(
        shm_asides.len(),
        1,
        "exactly one renamed-aside shm expected: {entries:?}"
    );
}

#[test]
fn test_store_recovery_failure_is_a_fatal_startup_error() {
    // A store path inside a missing directory cannot be renamed aside nor
    // recreated; recovery itself must fail with a clear fatal error instead
    // of crashing or silently continuing.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("nope").join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let Err(err) = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
    else {
        panic!("recovery failure must be a fatal startup error, but open succeeded");
    };
    let message = err.to_string();
    assert!(
        message.contains("failed to open Application Store")
            || message.contains("could not be recovered"),
        "error must clearly name the fatal recovery failure: {message}"
    );
}

// --- Application Store: Settings in typed tables -----------------------------

use riff_backend::app::store::SettingsStore;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_store_scalar_settings_roundtrip_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Fresh store: defaults (volume unset, toggles off).
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let store = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .unwrap();
        let settings = store
            .load_settings()
            .expect("loading settings from a fresh store must work");
        assert_eq!(settings.scalars.volume, None);
        assert!(!settings.scalars.advanced_mode);
        assert!(!settings.scalars.high_contrast);
        assert!(!settings.scalars.replaygain_enabled);
    }

    // Change every scalar and drop the connection (the "restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        store
            .save_scalars(&riff_backend::app::state::ScalarSettings {
                volume: Some(0.42),
                advanced_mode: true,
                high_contrast: true,
                replaygain_enabled: true,
            })
            .expect("saving scalars must work");
    }

    // Reopen: every value survives in its typed column.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let settings = reopened
        .load_settings()
        .expect("loading settings must work");
    assert_eq!(settings.scalars.volume, Some(0.42));
    assert!(settings.scalars.advanced_mode);
    assert!(settings.scalars.high_contrast);
    assert!(settings.scalars.replaygain_enabled);
}

#[test]
fn test_store_library_paths_and_watch_states_roundtrip_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Change paths + watch states, then drop the connection ("restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        let mut watch_states = HashMap::new();
        watch_states.insert(PathBuf::from("music/a"), WatchState::Enabled);
        watch_states.insert(
            PathBuf::from("music/b"),
            WatchState::Warning("network mount".to_string()),
        );
        store
            .save_watch_states(&watch_states)
            .expect("saving watch states must work");
        store
            .save_library_paths(&[PathBuf::from("music/b"), PathBuf::from("music/a")])
            .expect("saving library paths must work");
    }

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening must succeed");
    let settings = reopened
        .load_settings()
        .expect("loading settings must work");

    assert_eq!(
        settings.library_paths,
        vec![PathBuf::from("music/b"), PathBuf::from("music/a")],
        "registration order must survive the round trip"
    );
    assert_eq!(settings.watch_states.len(), 2);
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("music/a")),
        Some(&WatchState::Enabled)
    );
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("music/b")),
        Some(&WatchState::Warning("network mount".to_string()))
    );

    // Replacing the map removes stale entries: a path deleted from settings
    // must not keep its old watch row.
    let mut replacement = std::collections::HashMap::new();
    replacement.insert(PathBuf::from("music/c"), WatchState::Disabled);
    reopened.save_watch_states(&replacement).unwrap();
    reopened
        .save_library_paths(&[PathBuf::from("music/c")])
        .unwrap();

    let after = reopened.load_settings().unwrap();
    assert_eq!(after.library_paths, vec![PathBuf::from("music/c")]);
    assert_eq!(
        after.watch_states,
        std::collections::HashMap::from([(PathBuf::from("music/c"), WatchState::Disabled)])
    );
}

// --- Application Store: Playlists, atomic per-mutation durability ------------

use riff_backend::app::store::PlaylistStore;
use riff_backend::domain::{PlaylistId, TrackId};

#[test]
fn test_store_fresh_playlists_are_empty() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let playlists = store
        .load_playlists()
        .expect("loading playlists from a fresh store must work");
    assert!(playlists.is_empty(), "fresh store has no playlists");
}

#[test]
fn test_store_created_playlist_survives_immediate_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Create a playlist with initial entries; the mutation commits before
    // the connection is dropped (the "restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        let id = store
            .create_playlist(
                "  Chill Mix  ",
                &[TrackId("a.mp3".to_string()), TrackId("b.mp3".to_string())],
            )
            .expect("creating a playlist must work");
        assert_eq!(
            id.0,
            format!("chill-mix-{}", id.0.rsplit('-').next().unwrap())
        );
    }

    // Reopen: the playlist and its entry order are intact.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let playlists = reopened
        .load_playlists()
        .expect("loading playlists must work");
    assert_eq!(playlists.len(), 1, "the created playlist survives restart");
    assert_eq!(playlists[0].name, "Chill Mix", "name is trimmed");
    assert_eq!(
        playlists[0].tracks,
        vec![TrackId("a.mp3".to_string()), TrackId("b.mp3".to_string())],
        "initial entries keep their order"
    );
    assert!(playlists[0].created.is_some(), "creation time is stamped");
}

#[test]
fn test_store_rename_and_delete_commit_instantly_without_touching_unrelated_data() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        let keep = store.create_playlist("Keep", &[]).unwrap();
        let gone = store
            .create_playlist("Gone", &[TrackId("g1.mp3".to_string())])
            .unwrap();

        // Rename commits instantly (no later save step exists to make it
        // durable) and reports the known id.
        let renamed = store
            .rename_playlist(&keep, "  Kept Forever  ")
            .expect("renaming must not error");
        assert!(renamed, "renaming a known playlist returns true");

        // Delete cascades the playlist's entries and reports the removal.
        let deleted = store
            .delete_playlist(&gone)
            .expect("deleting must not error");
        assert!(deleted, "deleting a known playlist returns true");

        // Unknown ids are no-ops that report false.
        let unknown = PlaylistId("never-created".to_string());
        assert!(!store.rename_playlist(&unknown, "X").unwrap());
        assert!(!store.delete_playlist(&unknown).unwrap());

        drop(store);

        // Simulated crash right after the actions: reopen and verify.
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let reopened =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        let playlists = reopened.load_playlists().unwrap();
        assert_eq!(
            playlists
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Kept Forever"],
            "rename survived the restart; the deleted playlist is gone"
        );
        // Unrelated data was untouched: the surviving playlist has no stray
        // entries from the deleted one.
        assert!(
            playlists[0].tracks.is_empty(),
            "unrelated playlist keeps its own (empty) entry list"
        );
    }

    // The cascade removed the deleted playlist's entries at the SQL level.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let orphan_entries: i64 = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM playlist_entries WHERE track_id = 'g1.mp3'",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        orphan_entries, 0,
        "delete must remove the playlist's entries"
    );
}

#[test]
fn test_store_entry_add_remove_semantics_and_dangling_survival() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let pid;

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        pid = store.create_playlist("P", &[]).unwrap();

        // Appends land at the end, in call order.
        assert!(
            store
                .add_playlist_entry(&pid, &TrackId("a.mp3".to_string()))
                .unwrap()
        );
        assert!(
            store
                .add_playlist_entry(&pid, &TrackId("b.mp3".to_string()))
                .unwrap()
        );
        // Exact duplicates are rejected.
        assert!(
            !store
                .add_playlist_entry(&pid, &TrackId("a.mp3".to_string()))
                .unwrap()
        );
        // Unknown playlist ids are no-ops.
        assert!(
            !store
                .add_playlist_entry(&PlaylistId("x".to_string()), &TrackId("c.mp3".to_string()))
                .unwrap()
        );

        // Removal reports whether anything was removed.
        assert!(
            store
                .remove_playlist_entries(&pid, &TrackId("a.mp3".to_string()))
                .unwrap()
        );
        assert!(
            !store
                .remove_playlist_entries(&pid, &TrackId("zzz.mp3".to_string()))
                .unwrap()
        );

        // A dangling reference (no tracks table exists at all) commits like
        // any other entry.
        assert!(
            store
                .add_playlist_entry(&pid, &TrackId("vanished\\file.flac".to_string()))
                .unwrap()
        );
        drop(store);
    }

    // Restart: ordering and the dangling entry both survive; the entry stays
    // listed so it resolves again once the referenced file returns.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let playlists = reopened.load_playlists().unwrap();
    assert_eq!(playlists.len(), 1);
    assert_eq!(
        playlists[0].tracks,
        vec![
            TrackId("b.mp3".to_string()),
            TrackId("vanished\\file.flac".to_string())
        ],
        "entry order preserved; dangling reference kept visible"
    );
}

#[test]
fn test_store_duplicate_names_allowed_with_unique_ids() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    // Same-millisecond creation of same-named playlists must not collide:
    // ids dedupe, names may repeat (today's behavior).
    let a = store.create_playlist("Mix", &[]).unwrap();
    let b = store.create_playlist("Mix", &[]).unwrap();
    assert_ne!(a, b, "ids must be unique even for identical names");
    let playlists = store.load_playlists().unwrap();
    assert_eq!(playlists.len(), 2);
    assert!(playlists.iter().all(|p| p.name == "Mix"));
}

// --- Application Store: playlist drag-reorder persistence (Issue 12) --------

#[test]
fn test_store_reorder_playlist_entries_persists_the_new_order_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let pid;

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        pid = store
            .create_playlist(
                "Gym",
                &[
                    TrackId("a.mp3".to_string()),
                    TrackId("b.mp3".to_string()),
                    TrackId("c.mp3".to_string()),
                ],
            )
            .unwrap();

        // Drag the first entry down between the others: A,B,C → B,A,C.
        // One immediate durable transaction rewrites the positions.
        let reordered = store
            .reorder_playlist_entries(
                &pid,
                &[
                    TrackId("b.mp3".to_string()),
                    TrackId("a.mp3".to_string()),
                    TrackId("c.mp3".to_string()),
                ],
            )
            .expect("reordering a known playlist must work");
        assert!(reordered, "reordering a known playlist returns true");
        drop(store);
    }

    // Reopen: the new order survived the restart.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let playlists = reopened.load_playlists().unwrap();
    assert_eq!(
        playlists[0].tracks,
        vec![
            TrackId("b.mp3".to_string()),
            TrackId("a.mp3".to_string()),
            TrackId("c.mp3".to_string())
        ],
        "the dragged order persisted through the PlaylistStore"
    );

    // Reordering again (C first) keeps working on the persisted data.
    let mut store = reopened;
    assert!(
        store
            .reorder_playlist_entries(
                &pid,
                &[
                    TrackId("c.mp3".to_string()),
                    TrackId("b.mp3".to_string()),
                    TrackId("a.mp3".to_string())
                ]
            )
            .unwrap()
    );
    assert_eq!(
        store.load_playlists().unwrap()[0].tracks[0],
        TrackId("c.mp3".to_string()),
        "re-reordering rewrites the persisted order"
    );
}

#[test]
fn test_store_reorder_unknown_playlist_is_a_noop_and_other_playlists_are_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let keep = store
        .create_playlist(
            "Keep",
            &[TrackId("k1.mp3".to_string()), TrackId("k2.mp3".to_string())],
        )
        .unwrap();

    let unknown = PlaylistId("never-created".to_string());
    let reordered = store
        .reorder_playlist_entries(&unknown, &[TrackId("x.mp3".to_string())])
        .expect("reordering an unknown playlist must not error");
    assert!(!reordered, "unknown ids report false");

    // The unrelated playlist's entries are untouched.
    let playlists = store.load_playlists().unwrap();
    assert_eq!(
        playlists[0].tracks,
        vec![TrackId("k1.mp3".to_string()), TrackId("k2.mp3".to_string())],
        "an unknown-id reorder never touches other playlists"
    );
    let _ = keep;
}

#[test]
fn test_store_playlist_entries_report_library_validity_via_left_join() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Two scanned tracks plus a dangling reference that was never scanned.
    store
        .apply_scan_batch(&[
            library_track("m:\\lib\\a.mp3", "A", None, "One", None),
            library_track("m:\\lib\\b.mp3", "B", None, "One", None),
        ])
        .expect("batch applies");
    let pid = store
        .create_playlist(
            "Mix",
            &[
                TrackId("m:\\lib\\a.mp3".to_string()),
                TrackId("m:\\gone\\deleted.mp3".to_string()),
                TrackId("m:\\lib\\b.mp3".to_string()),
            ],
        )
        .unwrap();

    // Entries come back in playlist order; known tracks ride their full
    // row (valid=true), the dangling reference stays listed with its track
    // unset and valid=false — product behavior, never dropped.
    let entries = store.load_playlist_entries(&pid).expect("entries load");
    assert_eq!(entries.len(), 3, "dangling references stay listed");
    assert_eq!(entries[0].id.0, "m:\\lib\\a.mp3");
    assert!(entries[0].valid, "a scanned track is Library-valid");
    let track = entries[0].track.as_ref().expect("known track resolves");
    assert_eq!(track.metadata.title.as_deref(), Some("A"));
    assert_eq!(track.file_path, PathBuf::from("m:\\lib\\a.mp3"));
    assert!(!entries[1].valid, "dangling reference is flagged invalid");
    assert!(
        entries[1].track.is_none(),
        "dangling reference has no track"
    );
    assert_eq!(entries[1].id.0, "m:\\gone\\deleted.mp3");
    assert!(entries[2].valid);
    assert_eq!(entries[2].id.0, "m:\\lib\\b.mp3");

    // An unknown playlist id yields an empty entry list.
    let unknown = PlaylistId("never-created".to_string());
    assert!(
        store
            .load_playlist_entries(&unknown)
            .expect("unknown playlist loads")
            .is_empty()
    );

    // Validity is read-time: removing a track from the Library flips its
    // entry to invalid on the next query while the entry itself survives.
    store
        .remove_library_path(std::path::Path::new("m:\\lib\\a.mp3"))
        .expect("removal works");
    let entries = store.load_playlist_entries(&pid).expect("reload works");
    assert_eq!(entries.len(), 3, "the entry is not silently dropped");
    assert!(
        !entries[0].valid && entries[0].track.is_none(),
        "a track removed from the Library turns its entry dangling"
    );
}

// --- Application Store: Library collection (ticket 05) ---------------------

use riff_backend::app::store::{LibraryMutationStore, LibraryQueryStore};
use std::time::Duration;

/// Build a Track fixture with full metadata control (compilation cases need
/// track-level `artist` distinct from `album_artist`).
fn library_track(
    path: &str,
    title: &str,
    artist: Option<&str>,
    album: &str,
    album_artist: Option<&str>,
) -> Track {
    Track {
        id: TrackId::from_path(std::path::Path::new(path)),
        file_path: PathBuf::from(path),
        metadata: TrackMetadata {
            title: Some(title.to_string()),
            artist: artist.map(str::to_string),
            album: Some(album.to_string()),
            album_artist: album_artist.map(str::to_string),
            ..TrackMetadata::default()
        },
        duration: None,
        sample_rate: None,
        channels: None,
        play_count: 0,
        last_played: None,
        date_added: None,
        search_text: String::new(),
    }
}

#[test]
fn test_store_library_migration_004_applies_and_reopens_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("fresh store must open and migrate");
    let tables: Vec<String> = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name IN ('artists', 'albums', 'tracks') ORDER BY name",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("reading table list must work");
    assert_eq!(
        tables,
        ["albums", "artists", "tracks"],
        "library collection tables must exist after migration"
    );
    drop(store);

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening a store with the library schema must succeed");
}

#[test]
fn test_scan_batch_populates_collection_including_compilations() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Compilation: one album credited to "Various Artists", two tracks with
    // their own track-level artists.
    let batch = vec![
        library_track(
            "m:\\music\\comp\\01.mp3",
            "Song One",
            Some("Artist A"),
            "Comp",
            Some("Various Artists"),
        ),
        library_track(
            "m:\\music\\comp\\02.mp3",
            "Song Two",
            Some("Artist B"),
            "Comp",
            Some("Various Artists"),
        ),
    ];
    let written = store.apply_scan_batch(&batch).expect("batch must apply");
    assert_eq!(written, 2, "both tracks of the batch are written");

    let (artists, albums): (i64, i64) = store
        .with_connection(|conn| {
            let artists: i64 =
                conn.query_row("SELECT COUNT(*) FROM artists", [], |row| row.get(0))?;
            let albums: i64 =
                conn.query_row("SELECT COUNT(*) FROM albums", [], |row| row.get(0))?;
            Ok((artists, albums))
        })
        .expect("counting parents must work");
    assert_eq!(artists, 1, "one album-artist grouping for the compilation");
    assert_eq!(
        albums, 1,
        "one album row identified by (album_artist, title)"
    );

    let rows: Vec<(String, Option<String>, String, String, String)> = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, artist, album_artist_key, album_title_key, search_text
                 FROM tracks ORDER BY path",
            )?;
            let mapped = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            mapped.collect()
        })
        .expect("reading track rows must work");

    assert_eq!(rows.len(), 2);
    // Track-level artists stay distinct per track...
    assert_eq!(rows[0].1.as_deref(), Some("Artist A"));
    assert_eq!(rows[1].1.as_deref(), Some("Artist B"));
    // ...while the album identity keys use the resolved display fallbacks.
    for row in &rows {
        assert_eq!(row.2, "Various Artists");
        assert_eq!(row.3, "Comp");
    }
    // Derived search text is the Rust-lowercased concatenation, exactly like
    // TrackMetadata::search_text().
    assert_eq!(
        rows[0].4,
        batch[0].metadata.search_text(),
        "search_text must be derived in Rust at write time"
    );
}

#[test]
fn test_interrupted_scan_keeps_committed_batches() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Batch 1 commits; then the process "dies" before batch 2 is ever sent
    // (the store handle is dropped without applying more batches).
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        let batch1 = vec![library_track(
            "m:\\music\\a.mp3",
            "A",
            Some("Artist A"),
            "Album A",
            None,
        )];
        store
            .apply_scan_batch(&batch1)
            .expect("batch 1 must commit");
    }

    // After reopening, everything committed so far is present.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let count: i64 = reopened
        .with_connection(|conn| conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0)))
        .expect("counting tracks must work");
    assert_eq!(count, 1, "committed scan progress survives an interruption");
}

#[test]
fn test_rescan_is_idempotent_and_preserves_history() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let first = vec![library_track(
        "m:\\music\\old.mp3",
        "Old Title",
        Some("Artist A"),
        "Album A",
        None,
    )];
    store
        .apply_scan_batch(&first)
        .expect("initial scan applies");

    // Simulate accumulated play history on the stored row.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 3,
                 last_played_nanos = 1000, date_added_nanos = 500
                 WHERE path = 'm:\\music\\old.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    // Rescan the same path with refreshed metadata.
    let rescanned = vec![library_track(
        "m:\\music\\old.mp3",
        "New Title",
        Some("Artist A"),
        "Album A",
        None,
    )];
    store.apply_scan_batch(&rescanned).expect("rescan applies");

    let (tracks, albums, artists): (i64, i64, i64) = store
        .with_connection(|conn| {
            let t: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
            let al: i64 = conn.query_row("SELECT COUNT(*) FROM albums", [], |r| r.get(0))?;
            let ar: i64 = conn.query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0))?;
            Ok((t, al, ar))
        })
        .expect("counting rows must work");
    assert_eq!(
        (tracks, albums, artists),
        (1, 1, 1),
        "rescan must not duplicate rows"
    );

    let (title, play_count, last_played, date_added): (String, i64, Option<i64>, Option<i64>) =
        store
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT title, play_count, last_played_nanos, date_added_nanos
                     FROM tracks WHERE path = 'm:\\music\\old.mp3'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .expect("reading the upserted row must work");
    assert_eq!(title, "New Title", "metadata refreshes on rescan");
    assert_eq!(play_count, 3, "play history must survive a rescan");
    assert_eq!(last_played, Some(1000), "last-played must survive a rescan");
    assert_eq!(date_added, Some(500), "date-added must survive a rescan");
}

#[test]
fn test_foreign_keys_reject_orphan_tracks() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let err = store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO tracks(path, search_text, album_artist_key, album_title_key)
                 VALUES ('x.mp3', 'x', 'Ghost Artist', 'Ghost Album')",
                [],
            )
        })
        .expect_err("a track without its album parent must violate the foreign key");
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "error must name the FK violation: {err}"
    );
}

#[test]
fn test_flat_list_windows_are_path_ordered_and_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Shuffled insertion order with byte-ordering traps: uppercase sorts
    // before lowercase ('B' 0x42 < 'a' 0x61), digits before letters.
    let paths = [
        "m:\\z.mp3",
        "M:\\A\\B.mp3",
        "m:\\a.mp3",
        "m:\\1.mp3",
        "M:\\A\\a.mp3",
    ];
    let batch: Vec<Track> = paths
        .iter()
        .map(|p| library_track(p, "T", Some("Artist"), "Album", None))
        .collect();
    store.apply_scan_batch(&batch).expect("batch applies");

    let mut expected: Vec<String> = paths.iter().map(std::string::ToString::to_string).collect();
    expected.sort(); // Rust byte-wise sort = SQLite BINARY collation

    assert_eq!(store.track_count().expect("count works"), 5);

    let first_page = store.tracks_window(0, 3).expect("window works");
    assert_eq!(
        first_page
            .iter()
            .map(|t| t.id.0.clone())
            .collect::<Vec<_>>(),
        expected[..3],
        "flat list must be deterministically path-ordered"
    );
    let tail = store.tracks_window(3, 10).expect("window works");
    assert_eq!(
        tail.iter().map(|t| t.id.0.clone()).collect::<Vec<_>>(),
        expected[3..],
        "window past the page must return the remaining rows"
    );
    assert!(
        store
            .tracks_window(100, 5)
            .expect("window works")
            .is_empty(),
        "offset past the end yields an empty window"
    );
}

#[test]
fn test_all_track_ids_are_canonically_path_ordered() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // A fresh store has no ids to fill a queue with.
    assert!(
        store
            .all_track_ids()
            .expect("empty store lists no ids")
            .is_empty()
    );

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    // Shuffled insertion order with byte-ordering traps: Queue Fill must see
    // the canonical flat ordering (path ascending, ADR 0003), not insertion
    // or HashMap luck.
    let paths = [
        "m:\\z.mp3",
        "M:\\A\\B.mp3",
        "m:\\a.mp3",
        "m:\\1.mp3",
        "M:\\A\\a.mp3",
    ];
    let batch: Vec<Track> = paths
        .iter()
        .map(|p| library_track(p, "T", Some("Artist"), "Album", None))
        .collect();
    store.apply_scan_batch(&batch).expect("batch applies");

    let mut expected: Vec<String> = paths.iter().map(std::string::ToString::to_string).collect();
    expected.sort(); // Rust byte-wise sort = SQLite BINARY collation

    assert_eq!(
        store
            .all_track_ids()
            .expect("ids list works")
            .iter()
            .map(|id| id.0.clone())
            .collect::<Vec<_>>(),
        expected,
        "Queue Fill's data source is deterministically path-ordered"
    );
}

#[test]
fn test_search_parity_with_legacy_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Fixture library including non-Latin titles and literal wildcard
    // characters in metadata.
    let fixtures = vec![
        library_track(
            "f:\\s1.mp3",
            "Summer Breeze",
            Some("The Beach Boys"),
            "Sun",
            None,
        ),
        library_track(
            "f:\\s2.mp3",
            "Лебединое озеро",
            Some("Чайковский"),
            "Балеты",
            None,
        ),
        library_track("f:\\s3.mp3", "東京物語", None, "小津安二郎", None),
        library_track(
            "f:\\s4.mp3",
            "100% Live_Recording",
            Some("Idol"),
            "Stage",
            Some("Various Artists"),
        ),
    ];
    store.apply_scan_batch(&fixtures).expect("batch applies");

    // The legacy in-memory semantics are the oracle: case-insensitive
    // substring over the derived search text.
    let expected_for = |query: &str| -> Vec<String> {
        let q = query.to_lowercase();
        let mut hits: Vec<String> = fixtures
            .iter()
            .filter(|t| t.metadata.search_text().contains(&q))
            .map(|t| t.id.0.clone())
            .collect();
        hits.sort();
        hits
    };

    for query in [
        "summer",
        "SUMMER",
        "чайковский",
        "ЧАЙКОВСКИЙ",
        "東京",
        "100%",
        "_rec",
        "",
        "various artists",
        "zzz-no-match",
    ] {
        let expected = expected_for(query);
        let got_count = store.search_count(query).expect("search count works");
        assert_eq!(got_count, expected.len(), "count parity for {query:?}");
        let got = store.search_window(query, 0, 100).expect("search works");
        assert_eq!(
            got.iter().map(|t| t.id.0.clone()).collect::<Vec<_>>(),
            expected,
            "window content parity for {query:?}"
        );
    }

    // Bounded windows slice the match set deterministically.
    let all = store.search_window("", 0, 2).expect("search works");
    assert_eq!(all.len(), 2, "limit bounds the window");
    let rest = store.search_window("", 2, 50).expect("search works");
    assert_eq!(rest.len(), fixtures.len() - 2);
}

#[test]
fn test_get_track_roundtrips_all_fields() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let full = Track {
        id: TrackId::from_path(std::path::Path::new("f:\\full.flac")),
        file_path: PathBuf::from("f:\\full.flac"),
        metadata: TrackMetadata {
            title: Some("Full".to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: Some("Album Artist".to_string()),
            track_number: Some(7),
            disc_number: Some(2),
            genre: Some("Rock".to_string()),
            year: Some(1999),
            composer: Some("Composer".to_string()),
            comment: Some("Comment".to_string()),
            replaygain_track_gain: Some(-6.54),
            replaygain_track_peak: Some(0.75),
        },
        duration: Some(Duration::from_secs_f32(12.5)),
        sample_rate: Some(44_100),
        channels: Some(2),
        play_count: 4,
        last_played: Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
        date_added: Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(500)),
        search_text: String::new(),
    };
    store
        .apply_scan_batch(std::slice::from_ref(&full))
        .expect("batch applies");
    // History columns are insert-only for scans; seed them directly to
    // exercise the read path.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 4,
                 last_played_nanos = 1000000000000, date_added_nanos = 500000000000
                 WHERE path = 'f:\\full.flac'",
                [],
            )
        })
        .expect("seeding history works");

    let got = store
        .get_track(&full.id)
        .expect("get_track works")
        .expect("known id resolves");
    assert_eq!(got.id, full.id);
    assert_eq!(got.file_path, full.file_path);
    assert_eq!(got.metadata, full.metadata, "metadata round-trips exactly");
    assert_eq!(got.duration, full.duration);
    assert_eq!(got.sample_rate, full.sample_rate);
    assert_eq!(got.channels, full.channels);
    assert_eq!(got.play_count, 4);
    assert_eq!(
        got.last_played,
        Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000))
    );
    assert_eq!(
        got.date_added,
        Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(500))
    );

    // Minimal track: every optional field absent.
    let minimal = library_track("f:\\min.mp3", "Min", None, "Album", None);
    let minimal = Track {
        metadata: TrackMetadata {
            title: None,
            artist: None,
            album: None,
            ..minimal.metadata
        },
        ..minimal
    };
    store
        .apply_scan_batch(std::slice::from_ref(&minimal))
        .expect("batch applies");
    let got_min = store
        .get_track(&minimal.id)
        .expect("get_track works")
        .expect("known id resolves");
    assert_eq!(got_min.metadata.title, None);
    assert_eq!(got_min.metadata.artist, None);
    assert_eq!(got_min.metadata.album, None);
    assert_eq!(got_min.duration, None);
    assert_eq!(got_min.last_played, None);

    let unknown = store
        .get_track(&TrackId("f:\\missing.mp3".to_string()))
        .expect("get_track works");
    assert!(unknown.is_none(), "unknown ids resolve to None");
}

// --- Application Store: collection event transactions (ticket 06) -----------

use std::time::SystemTime;

#[test]
fn test_record_track_played_persists_immediately_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let batch = vec![
        library_track("m:\\music\\a.mp3", "A", Some("Artist A"), "Album A", None),
        library_track("m:\\music\\b.mp3", "B", Some("Artist A"), "Album A", None),
    ];
    store.apply_scan_batch(&batch).expect("batch applies");

    let id_a = TrackId("m:\\music\\a.mp3".to_string());
    let first_play = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let second_play = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
    assert!(
        store
            .record_track_played(&id_a, first_play)
            .expect("recording a known track works"),
        "known track must report true"
    );
    assert!(
        store
            .record_track_played(&id_a, second_play)
            .expect("recording a known track works"),
        "second play must also record"
    );

    // Unknown ids are not an error but change nothing.
    assert!(
        !store
            .record_track_played(&TrackId("m:\\missing.mp3".to_string()), first_play)
            .expect("unknown id is not an error"),
        "unknown track must report false"
    );

    // Durability: the plays survive closing and reopening the store.
    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let a = reopened
        .get_track(&id_a)
        .expect("get_track works")
        .expect("track still known after reopen");
    assert_eq!(a.play_count, 2, "both finished plays persisted");
    assert_eq!(a.last_played, Some(second_play), "the last play wins");

    let b = reopened
        .get_track(&TrackId("m:\\music\\b.mp3".to_string()))
        .expect("get_track works")
        .expect("track still known after reopen");
    assert_eq!(b.play_count, 0, "other tracks are untouched");
    assert_eq!(b.last_played, None);
}

#[test]
fn test_tag_refresh_preserves_history_and_updates_album_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Track A carries the album's derivation (first-added); B is newer.
    let mut track_a = library_track(
        "f:\\al\\a.mp3",
        "Old Title",
        Some("Artist A"),
        "Album A",
        None,
    );
    track_a.metadata.year = Some(2001);
    track_a.metadata.genre = Some("Rock".to_string());
    let mut track_b = library_track("f:\\al\\b.mp3", "B", Some("Artist A"), "Album A", None);
    track_b.metadata.year = Some(1999);
    track_b.metadata.genre = Some("Jazz".to_string());
    store
        .apply_scan_batch(&[track_a.clone(), track_b.clone()])
        .expect("batch applies");

    // Accumulated history on A (and its earlier date_added makes it the
    // album's first-added track).
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 3,
                 last_played_nanos = 1000, date_added_nanos = 500
                 WHERE path = 'f:\\al\\a.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    // Tag edit: title/year/genre change, everything else carried over —
    // exactly what `TagEdit::apply_to` produces before handing us the Track.
    let mut refreshed = track_a;
    refreshed.metadata.title = Some("New Title".to_string());
    refreshed.metadata.year = Some(2010);
    refreshed.metadata.genre = Some("Metal".to_string());
    store
        .apply_tag_refresh(&refreshed)
        .expect("refresh applies");

    // Durability: assert against a freshly opened store.
    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let a = reopened
        .get_track(&refreshed.id)
        .expect("get_track works")
        .expect("track known after reopen");
    assert_eq!(a.metadata.title.as_deref(), Some("New Title"));
    assert_eq!(a.metadata.year, Some(2010), "edited year lands");
    assert_eq!(a.metadata.genre.as_deref(), Some("Metal"));
    assert_eq!(
        a.metadata.artist.as_deref(),
        Some("Artist A"),
        "unchanged field stays"
    );
    assert_eq!(a.play_count, 3, "play history survives the tag refresh");
    assert_eq!(
        a.last_played,
        Some(SystemTime::UNIX_EPOCH + Duration::from_micros(1))
    );
    assert_eq!(
        a.date_added,
        Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(500))
    );

    let b = reopened
        .get_track(&track_b.id)
        .expect("get_track works")
        .expect("track known after reopen");
    assert_eq!(b.metadata.year, Some(1999), "other tracks untouched");

    // The album's derivation followed its first-added track's edit.
    let (album_year, album_genre): (Option<i64>, Option<String>) = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT year, genre FROM albums WHERE album_artist = 'Artist A' AND title = 'Album A'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("reading the album row must work");
    assert_eq!(
        album_year,
        Some(2010),
        "album year re-derived from the edit"
    );
    assert_eq!(album_genre.as_deref(), Some("Metal"));
}

#[test]
fn test_tag_refresh_moving_between_albums_cleans_orphans() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let original = library_track("f:\\x\\a.mp3", "A", Some("Solo"), "X", None);
    store
        .apply_scan_batch(std::slice::from_ref(&original))
        .expect("batch applies");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 7, date_added_nanos = 100
                 WHERE path = 'f:\\x\\a.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    // Move the track to another album/artist via a tag edit.
    let mut moved = original;
    moved.metadata.album_artist = Some("Duo".to_string());
    moved.metadata.album = Some("Y".to_string());
    moved.metadata.year = Some(1970);
    moved.metadata.genre = Some("Prog".to_string());
    store.apply_tag_refresh(&moved).expect("refresh applies");

    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let a = reopened
        .get_track(&moved.id)
        .expect("get_track works")
        .expect("track known after reopen");
    assert_eq!(a.play_count, 7, "history survives the move");
    assert_eq!(
        a.date_added,
        Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(100))
    );

    let (albums, artists): (Vec<(String, String)>, Vec<String>) = reopened
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT album_artist, title FROM albums ORDER BY album_artist")?;
            let albums = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut stmt = conn.prepare("SELECT name FROM artists ORDER BY name")?;
            let artists = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((albums, artists))
        })
        .expect("reading parents must work");
    assert_eq!(
        albums,
        vec![("Duo".to_string(), "Y".to_string())],
        "vacated album X must be gone; target album Y remains"
    );
    assert_eq!(
        artists,
        vec!["Duo".to_string()],
        "artist Solo had no albums left and must be gone"
    );

    let (year, genre): (Option<i64>, Option<String>) = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT year, genre FROM albums WHERE album_artist = 'Duo' AND title = 'Y'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("reading the album row must work");
    assert_eq!(year, Some(1970));
    assert_eq!(genre.as_deref(), Some("Prog"));
}

#[test]
fn test_remove_library_path_is_atomic_and_keeps_playlists_dangling() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Two roots share artist "Shared"; "m:\one2" guards against prefix traps.
    let batch = vec![
        library_track("m:\\one\\a.mp3", "A", Some("Shared"), "One", None),
        library_track("m:\\one\\b.mp3", "B", Some("Shared"), "One", None),
        library_track("m:\\one2\\c.mp3", "C", Some("Shared"), "Two", None),
        library_track("m:\\other\\d.mp3", "D", Some("Loner"), "Other", None),
    ];
    store.apply_scan_batch(&batch).expect("batch applies");
    for root in ["m:\\one", "m:\\one2"] {
        store
            .with_connection(|conn| {
                conn.execute("INSERT INTO library_paths(path) VALUES (?1)", [root])
            })
            .expect("registering the path must work");
    }

    // A playlist entry pointing into the removed root must survive dangling.
    let removed_id = TrackId("m:\\one\\a.mp3".to_string());
    let playlist_id = store
        .create_playlist("Mix", std::slice::from_ref(&removed_id))
        .expect("playlist creation works");

    let removed = store
        .remove_library_path(std::path::Path::new("m:\\one"))
        .expect("removal works");
    assert_eq!(removed, 2, "exactly that root's two tracks are removed");

    // Unknown roots remove nothing.
    let again = store
        .remove_library_path(std::path::Path::new("m:\\nowhere"))
        .expect("removing an unknown root works");
    assert_eq!(again, 0);

    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let remaining: Vec<String> = reopened
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM tracks ORDER BY path")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("listing tracks must work");
    assert_eq!(
        remaining,
        vec![
            "m:\\one2\\c.mp3".to_string(),
            "m:\\other\\d.mp3".to_string(),
        ],
        "exactly the removed root's tracks are gone; sibling prefixes survive"
    );

    let (albums, artists): (Vec<(String, String)>, Vec<String>) = reopened
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare("SELECT album_artist, title FROM albums ORDER BY album_artist, title")?;
            let albums = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut stmt = conn.prepare("SELECT name FROM artists ORDER BY name")?;
            let artists = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((albums, artists))
        })
        .expect("reading parents must work");
    assert_eq!(
        albums,
        vec![
            ("Loner".to_string(), "Other".to_string()),
            ("Shared".to_string(), "Two".to_string()),
        ],
        "emptied album One is cleaned; albums with surviving tracks stay"
    );
    assert_eq!(
        artists,
        vec!["Loner".to_string(), "Shared".to_string()],
        "orphaned artists go; Shared keeps an album"
    );

    let paths: Vec<String> = reopened
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM library_paths ORDER BY path")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("listing library paths must work");
    assert_eq!(
        paths,
        vec!["m:\\one2".to_string()],
        "the removed root's own record goes; others stay"
    );

    // Playlist entries referencing removed files stay listed so they recover
    // when the files return.
    let playlists = reopened.load_playlists().expect("playlists load");
    let mix = playlists
        .iter()
        .find(|p| p.id == playlist_id)
        .expect("playlist survives");
    assert_eq!(mix.tracks, vec![removed_id], "dangling entry stays listed");
}

// --- Application Store: artist/album browsing queries (ticket 07) ------------

/// A fixture track with full control over number and year, for browsing
/// orderings.
fn browsing_track(
    path: &str,
    title: &str,
    album_artist: &str,
    album: &str,
    track_number: Option<u32>,
    year: Option<u32>,
) -> Track {
    Track {
        id: TrackId::from_path(std::path::Path::new(path)),
        file_path: PathBuf::from(path),
        metadata: TrackMetadata {
            title: Some(title.to_string()),
            artist: Some(album_artist.to_string()),
            album: Some(album.to_string()),
            album_artist: Some(album_artist.to_string()),
            track_number,
            year,
            ..TrackMetadata::default()
        },
        duration: None,
        sample_rate: None,
        channels: None,
        play_count: 0,
        last_played: None,
        date_added: None,
        search_text: String::new(),
    }
}

/// An independent Rust-only restatement of the three browsing orderings,
/// computed straight from the fixture list with the former comparators. The
/// SQL queries must agree with this without going through any shared code.
mod reference_order {
    use super::*;
    use std::collections::BTreeSet;

    pub fn artists_az(fixtures: &[Track]) -> Vec<String> {
        let names: BTreeSet<String> = fixtures
            .iter()
            .map(|t| t.metadata.display_album_artist().into_owned())
            .collect();
        names.into_iter().collect()
    }

    pub fn artist_albums(fixtures: &[Track], artist: &str) -> Vec<(String, Option<u32>)> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut albums: Vec<(String, Option<u32>)> = Vec::new();
        for track in fixtures
            .iter()
            .filter(|t| t.metadata.display_album_artist() == artist)
        {
            let title = track.metadata.display_album().into_owned();
            if seen.insert(title.clone()) {
                albums.push((title, track.metadata.year));
            }
        }
        albums.sort_by(|a, b| {
            b.1.unwrap_or(0)
                .cmp(&a.1.unwrap_or(0))
                .then_with(|| a.0.cmp(&b.0))
        });
        albums
    }

    pub fn album_tracks(fixtures: &[Track], artist: &str, title: &str) -> Vec<TrackId> {
        let mut tracks: Vec<&Track> = fixtures
            .iter()
            .filter(|t| {
                t.metadata.display_album_artist() == artist && t.metadata.display_album() == title
            })
            .collect();
        tracks.sort_by(|a, b| {
            a.metadata
                .track_number
                .unwrap_or(0)
                .cmp(&b.metadata.track_number.unwrap_or(0))
                .then_with(|| a.file_path.cmp(&b.file_path))
        });
        tracks.into_iter().map(|t| t.id.clone()).collect()
    }
}

/// The parity fixture library: multiple artists, missing years, same-year
/// title ties, missing track numbers, and duplicate-number ties, listed in
/// the path-ascending insertion order that scans produce.
fn browsing_fixtures() -> Vec<Track> {
    vec![
        browsing_track(
            "f:\\zeta\\new wave\\01 - Opening.mp3",
            "Opening",
            "Zeta",
            "New Wave",
            Some(1),
            Some(2019),
        ),
        browsing_track(
            "f:\\zeta\\new wave\\02 - Middle.mp3",
            "Middle",
            "Zeta",
            "New Wave",
            Some(2),
            Some(2019),
        ),
        // Duplicate track number 2: a tie the path tiebreak must settle.
        browsing_track(
            "f:\\zeta\\new wave\\02b - Encore.mp3",
            "Encore",
            "Zeta",
            "New Wave",
            Some(2),
            Some(2019),
        ),
        // Missing track number sorts before numbered tracks (legacy 0 slot).
        browsing_track(
            "f:\\zeta\\new wave\\zz - Unnumbered.mp3",
            "Unnumbered",
            "Zeta",
            "New Wave",
            None,
            Some(2019),
        ),
        browsing_track(
            "f:\\zeta\\new wave\\10 - Closer.mp3",
            "Closer",
            "Zeta",
            "New Wave",
            Some(10),
            Some(2019),
        ),
        browsing_track(
            "f:\\zeta\\a sides\\1.mp3",
            "A1",
            "Zeta",
            "A Sides",
            Some(1),
            Some(2010),
        ),
        browsing_track(
            "f:\\zeta\\b sides\\1.mp3",
            "B1",
            "Zeta",
            "B Sides",
            Some(1),
            Some(2010),
        ),
        browsing_track(
            "f:\\zeta\\middle era\\1.mp3",
            "M1",
            "Zeta",
            "Middle Era",
            Some(1),
            Some(2005),
        ),
        // Missing year is oldest in the newest-first album order.
        browsing_track(
            "f:\\zeta\\old hits\\1.mp3",
            "O1",
            "Zeta",
            "Old Hits",
            Some(1),
            None,
        ),
        browsing_track(
            "f:\\alpha\\only\\2.mp3",
            "Alpha Two",
            "Alpha",
            "Only",
            Some(2),
            Some(1999),
        ),
        browsing_track(
            "f:\\alpha\\only\\1.mp3",
            "Alpha One",
            "Alpha",
            "Only",
            Some(1),
            Some(1999),
        ),
    ]
}

#[test]
fn test_browsing_queries_match_independent_reference_orderings() {
    // Both sides receive the identical fixtures in the identical insertion
    // order; only the ordering pipelines differ (SQL vs an independent Rust
    // restatement of the former comparators).
    let fixtures = browsing_fixtures();

    // Store side: the whole fixture set through one scan commit.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    store.apply_scan_batch(&fixtures).expect("batch applies");

    // Artists A–Z.
    assert_eq!(
        store
            .all_artists()
            .expect("artists query")
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>(),
        reference_order::artists_az(&fixtures),
        "SQL artist order must match the independent reference ordering"
    );

    // An artist's albums: newest-first (missing year last) then title.
    assert_eq!(
        store
            .artist_albums("Zeta")
            .expect("albums query")
            .iter()
            .map(|a| (a.title.clone(), a.year))
            .collect::<Vec<_>>(),
        reference_order::artist_albums(&fixtures, "Zeta"),
        "SQL album order must match the independent reference ordering"
    );

    // Album tracks: number then filename, missing numbers first, ties by path.
    assert_eq!(
        store
            .album_tracks("Zeta", "New Wave")
            .expect("album tracks query")
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        reference_order::album_tracks(&fixtures, "Zeta", "New Wave"),
        "SQL album-track order must match the independent reference ordering"
    );
}

#[test]
fn test_all_artists_lists_names_az_with_canonical_album_keys() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Fresh store: no artists.
    assert!(
        store.all_artists().expect("empty query").is_empty(),
        "a fresh store lists no artists"
    );

    // Insertion deliberately out of alphabetical order, with years chosen so
    // canonical album order differs from first-added order.
    store
        .apply_scan_batch(&[
            browsing_track("f:\\z\\1.mp3", "Z1", "Zulu", "Late", Some(1), Some(2020)),
            browsing_track("f:\\a\\2.mp3", "A2", "Alpha", "Early", Some(2), Some(1980)),
            browsing_track("f:\\a\\1.mp3", "A1", "Alpha", "Dated", Some(1), None),
            browsing_track("f:\\a\\3.mp3", "A3", "Alpha", "Early", Some(3), Some(1980)),
        ])
        .expect("batch applies");

    let artists = store.all_artists().expect("artists query");
    assert_eq!(
        artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Alpha", "Zulu"],
        "artists list name-ascending"
    );

    let alpha = &artists[0];
    assert_eq!(
        alpha.albums,
        vec!["Alpha - Early".to_string(), "Alpha - Dated".to_string(),],
        "an artist's embedded keys follow the canonical album order \
         (newest-first, missing year last, then title)"
    );
}

#[test]
fn test_artist_albums_orders_newest_first_missing_year_last_title_tiebreak() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    store
        .apply_scan_batch(&[
            browsing_track("f:\\n\\1.mp3", "N1", "Nova", "Undated", Some(1), None),
            browsing_track("f:\\n\\2.mp3", "N2", "Nova", "Twins B", Some(1), Some(2000)),
            browsing_track("f:\\n\\3.mp3", "N3", "Nova", "Old", Some(1), Some(1990)),
            browsing_track("f:\\n\\4.mp3", "N4", "Nova", "Twins A", Some(1), Some(2000)),
            browsing_track("f:\\n\\5.mp3", "N5", "Nova", "Latest", Some(1), Some(2021)),
        ])
        .expect("batch applies");

    let albums = store.artist_albums("Nova").expect("albums query");
    assert_eq!(
        albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["Latest", "Twins A", "Twins B", "Old", "Undated"],
        "year descending with missing year last, then title ascending"
    );

    // Every album carries its own tracks in album-track order.
    let twins_a = albums.iter().find(|a| a.title == "Twins A").unwrap();
    assert_eq!(twins_a.tracks.len(), 1, "membership stays per-album");

    // Unknown artists browse to an empty list, not an error.
    assert!(
        store
            .artist_albums("Nobody")
            .expect("unknown artist")
            .is_empty(),
        "unknown artists yield no albums"
    );
}

#[test]
fn test_album_tracks_orders_missing_numbers_first_then_path() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    // Insertion order deliberately scrambled relative to both number and
    // path; the query must impose the canonical order regardless.
    store
        .apply_scan_batch(&[
            browsing_track("f:\\al\\09_zeta.mp3", "Nine", "Duo", "Y", Some(9), None),
            browsing_track("f:\\al\\zz_unnumbered.mp3", "Ghost", "Duo", "Y", None, None),
            browsing_track("f:\\al\\05_beta.mp3", "Five B", "Duo", "Y", Some(5), None),
            browsing_track("f:\\al\\01_alpha.mp3", "One", "Duo", "Y", Some(1), None),
            browsing_track("f:\\al\\05_alpha.mp3", "Five A", "Duo", "Y", Some(5), None),
        ])
        .expect("batch applies");

    let tracks = store.album_tracks("Duo", "Y").expect("album tracks query");
    assert_eq!(
        tracks.iter().map(|t| t.id.0.as_str()).collect::<Vec<_>>(),
        [
            "f:\\al\\zz_unnumbered.mp3", // missing number: legacy 0 slot, first
            "f:\\al\\01_alpha.mp3",
            "f:\\al\\05_alpha.mp3", // duplicate numbers tie-break by path
            "f:\\al\\05_beta.mp3",
            "f:\\al\\09_zeta.mp3",
        ],
        "track number ascending with missing numbers first, path tiebreak"
    );

    // Unknown albums browse to an empty list, not an error.
    assert!(
        store
            .album_tracks("Duo", "Nonexistent")
            .expect("unknown album")
            .is_empty(),
        "unknown albums yield no tracks"
    );
}

#[test]
fn test_browsing_works_straight_from_a_freshly_reopened_store() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let mut store =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .unwrap();
        store
            .apply_scan_batch(&[
                browsing_track("f:\\x\\2.mp3", "Two", "Ithaca", "LP", Some(2), Some(1977)),
                browsing_track("f:\\x\\1.mp3", "One", "Ithaca", "LP", Some(1), Some(1977)),
            ])
            .expect("batch applies");
    } // store dropped: connection closed like an app restart.

    // Reopened cold — no hydration, no warm-up of any kind.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened =
        riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let artists = reopened.all_artists().expect("artists query");
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name, "Ithaca");
    assert_eq!(artists[0].albums, vec!["Ithaca - LP".to_string()]);

    let albums = reopened.artist_albums("Ithaca").expect("albums query");
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].title, "LP");
    assert_eq!(albums[0].year, Some(1977));
    assert_eq!(
        albums[0].tracks,
        vec![
            TrackId("f:\\x\\1.mp3".to_string()),
            TrackId("f:\\x\\2.mp3".to_string()),
        ],
        "album membership hydrates in browsing order without warm-up"
    );

    let tracks = reopened.album_tracks("Ithaca", "LP").expect("tracks query");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].metadata.title.as_deref(), Some("One"));
    assert_eq!(tracks[1].metadata.title.as_deref(), Some("Two"));
}

// --- Application Store: folder navigation queries (ticket 08) ---------------

/// A folder-tree fixture library under a REAL temporary directory: the
/// former in-memory `subdirs_with_audio` stats candidate children with
/// `is_dir()`, so parity fixtures must exist on disk. Exercises nested
/// paths, the sibling-prefix trap ("a" alongside "ab"), percent/underscore/
/// escape-character names, a directory containing only subdirectories, and
/// direct-track ordering with missing and duplicate numbers.
fn folder_fixtures(base: &std::path::Path) -> Vec<Track> {
    for dir in ["a", "ab", "50% Off", "a_b", "axb", "a#b", "deep/only"] {
        std::fs::create_dir_all(base.join(dir)).expect("fixture dirs must create");
    }

    let track = |rel: &str, title: &str, number: Option<u32>| {
        // Fixture literals use `\` for readability; translate to the platform
        // separator so stored paths match real scan output on every OS
        // (`Path::join` only normalizes separators native to the host).
        let path = base.join(rel.replace('\\', std::path::MAIN_SEPARATOR_STR));
        browsing_track(
            &path.to_string_lossy(),
            title,
            "Artist",
            "Album",
            number,
            Some(2001),
        )
    };

    vec![
        track("a\\zz unnumbered.mp3", "Unnumbered", None),
        track("a\\05 beta.mp3", "Beta", Some(5)),
        track("a\\01 alpha.mp3", "Alpha", Some(1)),
        track("a\\05 alpha.mp3", "Alpha Five", Some(5)),
        track("ab\\three.mp3", "Three", Some(1)),
        track("50% Off\\hit.mp3", "Hit", Some(1)),
        track("a_b\\under.mp3", "Under", Some(1)),
        track("axb\\wildcard victim.mp3", "Victim", Some(1)),
        track("a#b\\hash.mp3", "Hash", Some(1)),
        track("deep\\only\\nested.mp3", "Nested", Some(1)),
    ]
}

fn seeded_folder_store(dir: &tempfile::TempDir) -> riff_backend::infra::store::SqliteStore {
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store = riff_backend::infra::store::SqliteStore::open_and_migrate(
        &dir.path().join("riff.sqlite3"),
        changes_tx,
    )
    .expect("fresh store must open");
    store
        .apply_scan_batch(&folder_fixtures(dir.path()))
        .expect("fixtures apply");
    store
}

/// An independent Rust-only restatement of the former in-memory folder
/// logic, computed straight from the fixture list with pure `Path` ops
/// (including the `is_dir()` stats the original performed). The SQL queries
/// must agree with this without going through any shared code.
mod folder_reference {
    use super::*;
    use std::path::Path;

    pub fn has_audio(tracks: &[Track], folder: &Path) -> bool {
        tracks.iter().any(|t| t.file_path.starts_with(folder))
    }

    pub fn tree_ids(tracks: &[Track], folder: &Path) -> Vec<TrackId> {
        let mut ids: Vec<TrackId> = tracks
            .iter()
            .filter(|t| t.file_path.starts_with(folder))
            .map(|t| t.id.clone())
            .collect();
        ids.sort_by(|a, b| {
            let path_of = |id: &TrackId| tracks.iter().find(|t| &t.id == id).map(|t| &t.file_path);
            path_of(a).cmp(&path_of(b))
        });
        ids
    }

    pub fn direct_ids(tracks: &[Track], folder: &Path) -> Vec<TrackId> {
        let mut direct: Vec<&Track> = tracks
            .iter()
            .filter(|t| t.file_path.parent() == Some(folder))
            .collect();
        direct.sort_by(|a, b| {
            a.metadata
                .track_number
                .unwrap_or(0)
                .cmp(&b.metadata.track_number.unwrap_or(0))
                .then_with(|| a.file_path.file_name().cmp(&b.file_path.file_name()))
        });
        direct.into_iter().map(|t| t.id.clone()).collect()
    }

    pub fn subdirs(tracks: &[Track], folder: &Path) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for track in tracks {
            let track_path = &track.file_path;
            if !track_path.starts_with(folder) {
                continue;
            }
            let Ok(relative) = track_path.strip_prefix(folder) else {
                continue;
            };
            if let Some(first_component) = relative.iter().next() {
                let child_dir = folder.join(first_component);
                if child_dir.is_dir() && seen.insert(child_dir.clone()) {
                    dirs.push(child_dir);
                }
            }
        }
        dirs.sort();
        dirs
    }
}

#[test]
fn test_folder_queries_match_independent_reference_logic() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let fixtures = folder_fixtures(dir.path());

    // Probe folders: roots, mid-level, leaves, special-char names, the
    // sibling-prefix trap pair, a file path used as a folder, and a folder
    // that does not exist anywhere in the library. Relative parts use the
    // platform separator, exactly like every real caller (scan joins and
    // `Path::join` never mix separators).
    let rel_probes = [
        "",
        "a",
        "ab",
        "50% Off",
        "a_b",
        "axb",
        "a#b",
        "deep",
        std::path::MAIN_SEPARATOR_STR,
    ];
    let mut probes: Vec<std::path::PathBuf> =
        rel_probes.iter().map(|rel| dir.path().join(rel)).collect();
    probes.push(dir.path().join("deep").join("only"));
    probes.push(dir.path().join("a").join("01 alpha.mp3"));

    for probe in probes {
        let label = probe.to_string_lossy();

        assert_eq!(
            store.folder_has_audio(&probe).expect("has_audio"),
            folder_reference::has_audio(&fixtures, &probe),
            "folder_has_audio({label}) must match the reference logic"
        );

        assert_eq!(
            store.track_ids_in_folder_tree(&probe).expect("tree ids"),
            folder_reference::tree_ids(&fixtures, &probe),
            "track_ids_in_folder_tree({label}) must match the reference logic"
        );

        assert_eq!(
            store
                .tracks_in_folder(&probe)
                .expect("direct tracks")
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            folder_reference::direct_ids(&fixtures, &probe),
            "tracks_in_folder({label}) must match the reference order and membership"
        );

        assert_eq!(
            store.subdirs_with_audio(&probe).expect("subdirs"),
            folder_reference::subdirs(&fixtures, &probe),
            "subdirs_with_audio({label}) must match the reference logic"
        );
    }
}

#[test]
fn test_folder_prefix_queries_escape_like_wildcards() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    // "%" in a folder name is a literal character: querying it must not
    // degrade into a wildcard match.
    let off = base.join("50% Off");
    assert!(
        store.folder_has_audio(&off).expect("has_audio"),
        "the literal percent folder contains audio"
    );
    assert_eq!(
        store
            .track_ids_in_folder_tree(&off)
            .expect("tree ids")
            .len(),
        1,
        "only the percent folder's own track matches"
    );

    // "_" in a folder name must not match sibling "axb".
    let under = base.join("a_b");
    assert!(store.folder_has_audio(&under).expect("has_audio"));
    assert_eq!(
        store.track_ids_in_folder_tree(&under).expect("tree ids"),
        vec![TrackId(
            base.join("a_b")
                .join("under.mp3")
                .to_string_lossy()
                .into_owned()
        )],
        "underscore must be literal, not a single-char wildcard"
    );

    // The escape character itself appearing in a name stays literal.
    let hash = base.join("a#b");
    assert!(store.folder_has_audio(&hash).expect("has_audio"));
    assert_eq!(
        store.track_ids_in_folder_tree(&hash).expect("tree ids"),
        vec![TrackId(
            base.join("a#b")
                .join("hash.mp3")
                .to_string_lossy()
                .into_owned()
        )],
        "escape characters in names are matched literally"
    );
}

#[test]
fn test_sibling_prefix_trap_is_not_matched() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    // Folder "a" alongside "ab": prefix matching must respect component
    // boundaries, so "ab"'s subtree never leaks into "a".
    let a = base.join("a");
    let mut ids = store.track_ids_in_folder_tree(&a).expect("tree ids");
    ids.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        ids,
        vec![
            TrackId(
                base.join("a")
                    .join("01 alpha.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
            TrackId(
                base.join("a")
                    .join("05 alpha.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
            TrackId(
                base.join("a")
                    .join("05 beta.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
            TrackId(
                base.join("a")
                    .join("zz unnumbered.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
        ],
        "folder a holds exactly its own four tracks"
    );

    let ab = base.join("ab");
    assert_eq!(
        store.track_ids_in_folder_tree(&ab).expect("tree ids"),
        vec![TrackId(
            base.join("ab")
                .join("three.mp3")
                .to_string_lossy()
                .into_owned()
        )],
        "folder ab holds exactly its own track"
    );
}

#[test]
fn test_subdirs_with_audio_lists_only_direct_child_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    let children = store.subdirs_with_audio(base).expect("subdirs");
    let names: Vec<String> = children
        .iter()
        .map(|c| c.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["50% Off", "a", "a#b", "a_b", "ab", "axb", "deep"],
        "every child with audio lists once, name-ascending; direct files never appear"
    );

    // A directory containing only subdirectories still expands.
    let deep = base.join("deep");
    assert_eq!(
        store.subdirs_with_audio(&deep).expect("subdirs"),
        vec![base.join("deep").join("only")],
        "directories holding only subdirectories list their child"
    );
    assert!(
        store
            .tracks_in_folder(&deep)
            .expect("direct tracks")
            .is_empty(),
        "deep has no direct tracks of its own"
    );
}

#[test]
fn test_tracks_in_folder_orders_number_then_filename_missing_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let a = dir.path().join("a");

    let tracks = store.tracks_in_folder(&a).expect("direct tracks");
    assert_eq!(
        tracks.iter().map(|t| t.id.0.as_str()).collect::<Vec<_>>(),
        [
            a.join("zz unnumbered.mp3").to_string_lossy(), // missing number first
            a.join("01 alpha.mp3").to_string_lossy(),
            a.join("05 alpha.mp3").to_string_lossy(), // tie by filename
            a.join("05 beta.mp3").to_string_lossy(),
        ],
        "direct tracks sort by number then filename, missing numbers first"
    );
}

#[test]
fn test_folder_has_search_match_scopes_to_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    // "victim" only exists under axb.
    let a = base.join("a");
    let axb = base.join("axb");
    assert!(
        !store
            .folder_has_search_match(&a, "victim")
            .expect("search match"),
        "sibling subtrees must not satisfy the search filter"
    );
    assert!(
        store
            .folder_has_search_match(&axb, "victim")
            .expect("search match"),
        "the owning subtree satisfies the search filter"
    );
    assert!(
        store
            .folder_has_search_match(&a, "unnumbered")
            .expect("search match"),
        "a real match inside the subtree reports true"
    );
}

// --- Application Store: smart playlists as SQL (ticket 09) ------------------

use riff_backend::domain::SmartPlaylistKind;

/// A fixture track with full control over history and dates.
fn smart_track(
    path: &str,
    title: Option<&str>,
    play_count: u32,
    last_played: Option<SystemTime>,
    date_added: Option<SystemTime>,
) -> Track {
    Track {
        id: TrackId::from_path(std::path::Path::new(path)),
        file_path: PathBuf::from(path),
        metadata: TrackMetadata {
            title: title.map(str::to_string),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: Some("Artist".to_string()),
            ..TrackMetadata::default()
        },
        duration: None,
        sample_rate: None,
        channels: None,
        play_count,
        last_played,
        date_added,
        search_text: String::new(),
    }
}

/// The fixture library, shared by the store seeding and the independent
/// smart-playlist reference oracle.
fn smart_fixtures() -> Vec<Track> {
    let now = SystemTime::now();
    let days_ago = |days: u64| now - Duration::from_secs(days * 86_400);
    let epoch_plus = |secs: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);

    vec![
        smart_track(
            "f:\\sm\\zebra.mp3",
            Some("Zebra"),
            10,
            Some(days_ago(100)),
            Some(epoch_plus(5_000)),
        ),
        // Count tie broken by display-title fallback stems ("alpha" first).
        smart_track(
            "f:\\sm\\alpha.mp3",
            None,
            3,
            Some(days_ago(200)),
            Some(epoch_plus(9_000)),
        ),
        // Stem-vs-path divergence pair: titles "a.b" vs "a" order opposite
        // to raw path bytes, pinning the exact comparator.
        smart_track(
            "f:\\sm\\a.b.mp3",
            None,
            3,
            Some(days_ago(300)),
            Some(epoch_plus(1_000)),
        ),
        smart_track(
            "f:\\sm\\a.mp3",
            None,
            3,
            Some(days_ago(310)),
            Some(epoch_plus(2_000)),
        ),
        // Unplayed pair for NeverPlayed's path ordering.
        smart_track(
            "f:\\sm\\never1.mp3",
            Some("Never One"),
            0,
            None,
            Some(epoch_plus(6_000)),
        ),
        smart_track(
            "f:\\sm\\never0.mp3",
            Some("Never Zero"),
            0,
            None,
            Some(epoch_plus(7_000)),
        ),
        // Missing date_added: excluded from Recently Added entirely.
        smart_track("f:\\sm\\nodate.mp3", Some("No Date"), 0, None, None),
        // Future last-played counts as a Lost Gem ("very old").
        smart_track(
            "f:\\sm\\future.mp3",
            Some("Future"),
            1,
            Some(now + Duration::from_hours(1)),
            Some(epoch_plus(8_000)),
        ),
        // Recently-added tie on identical date_added values.
        smart_track(
            "f:\\sm\\tie_b.mp3",
            Some("Tie B"),
            0,
            None,
            Some(epoch_plus(4_000)),
        ),
        smart_track(
            "f:\\sm\\tie_a.mp3",
            Some("Tie A"),
            0,
            None,
            Some(epoch_plus(4_000)),
        ),
        // Played too recently to be a gem.
        smart_track(
            "f:\\sm\\recent_play.mp3",
            Some("Recent Play"),
            2,
            Some(days_ago(30)),
            Some(epoch_plus(3_000)),
        ),
    ]
}

/// Seed the fixture library: scan batches persist metadata + `date_added`;
/// play history is written through direct updates exactly like the
/// tag-refresh tests do (the scan upsert deliberately never touches
/// history columns).
fn seeded_smart_store(db_path: &std::path::Path) -> riff_backend::infra::store::SqliteStore {
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store = riff_backend::infra::store::SqliteStore::open_and_migrate(db_path, changes_tx)
        .expect("fresh store must open");

    let fixtures = smart_fixtures();

    store.apply_scan_batch(&fixtures).expect("fixtures apply");

    for track in &fixtures {
        let last_nanos =
            track
                .last_played
                .map(|t| match t.duration_since(SystemTime::UNIX_EPOCH) {
                    Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
                    Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
                });
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE tracks SET play_count = ?1, last_played_nanos = ?2 WHERE path = ?3",
                    rusqlite::params![i64::from(track.play_count), last_nanos, track.id.0],
                )
            })
            .expect("seeding history must work");
    }
    store
}

/// An independent Rust-only restatement of the four smart-playlist
/// computations, straight from the fixture list with the former comparators.
/// The SQL implementations must agree with this without going through any
/// shared code. Exercising [`LOST_GEMS_THRESHOLD`] here also pins the
/// relocated constant to the semantics it parameterizes.
mod smart_reference {
    use super::*;
    use riff_backend::app::store::LOST_GEMS_THRESHOLD;

    pub fn playlist(tracks: &[Track], kind: SmartPlaylistKind, limit: usize) -> Vec<TrackId> {
        let mut selected: Vec<&Track> = match kind {
            SmartPlaylistKind::RecentlyAdded => {
                // Sorted by the stored `date_added`, deliberately NOT the
                // filesystem mtime; newest first, path tiebreak.
                let mut dated: Vec<(SystemTime, &Track)> = tracks
                    .iter()
                    .filter_map(|t| t.date_added.map(|added| (added, t)))
                    .collect();
                dated.sort_by(|(added_a, a), (added_b, b)| {
                    added_b
                        .cmp(added_a)
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                dated.into_iter().map(|(_, t)| t).collect()
            }
            SmartPlaylistKind::MostPlayed => {
                let mut played: Vec<&Track> = tracks.iter().filter(|t| t.play_count > 0).collect();
                played.sort_by(|a, b| {
                    b.play_count
                        .cmp(&a.play_count)
                        .then_with(|| {
                            a.metadata
                                .display_title(&a.file_path)
                                .cmp(&b.metadata.display_title(&b.file_path))
                        })
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                played
            }
            SmartPlaylistKind::NeverPlayed => {
                let mut unplayed: Vec<&Track> =
                    tracks.iter().filter(|t| t.play_count == 0).collect();
                unplayed.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                unplayed
            }
            SmartPlaylistKind::LostGems => {
                // Unheard for 90+ days: last played older than the threshold
                // OR never played at all. Once-played gems come first,
                // longest-unheard first; never-played tracks fill the tail in
                // path order.
                let mut gems: Vec<(SystemTime, &Track)> = tracks
                    .iter()
                    .filter_map(|t| t.last_played.map(|last| (last, t)))
                    .filter(|(last, _)| match last.elapsed() {
                        Ok(age) => age > LOST_GEMS_THRESHOLD,
                        // A clock anomaly (last_played in the future) counts
                        // as "very old" rather than excluding the track.
                        Err(_) => true,
                    })
                    .collect();
                gems.sort_by(|(last_a, a), (last_b, b)| {
                    last_a
                        .cmp(last_b)
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                let mut unheard: Vec<&Track> =
                    tracks.iter().filter(|t| t.last_played.is_none()).collect();
                unheard.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                gems.into_iter().map(|(_, t)| t).chain(unheard).collect()
            }
        };
        selected.truncate(limit);
        selected.into_iter().map(|t| t.id.clone()).collect()
    }
}

#[test]
fn test_smart_playlists_match_independent_reference_logic() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let store = seeded_smart_store(&db_path);
    let fixtures = smart_fixtures();

    for kind in [
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::NeverPlayed,
        SmartPlaylistKind::LostGems,
    ] {
        for limit in [usize::MAX, 3] {
            assert_eq!(
                store
                    .smart_playlist(kind, limit)
                    .expect("smart playlist query")
                    .iter()
                    .map(|t| t.id.clone())
                    .collect::<Vec<_>>(),
                smart_reference::playlist(&fixtures, kind, limit),
                "smart_playlist({kind:?}, {limit}) must match the reference logic"
            );
        }
    }

    // Spot-check the interesting shapes so a two-sided bug cannot hide.
    let most_played = store
        .smart_playlist(SmartPlaylistKind::MostPlayed, usize::MAX)
        .expect("most played");
    assert_eq!(
        most_played[0].metadata.title.as_deref(),
        Some("Zebra"),
        "highest play count leads"
    );
    let stems: Vec<Option<&str>> = most_played[1..4]
        .iter()
        .map(|t| t.id.0.split('\\').next_back())
        .collect();
    assert_eq!(
        stems,
        [Some("a.mp3"), Some("a.b.mp3"), Some("alpha.mp3")],
        "count ties break by display-title stem order, not raw path bytes"
    );

    let gems = store
        .smart_playlist(SmartPlaylistKind::LostGems, usize::MAX)
        .expect("lost gems");
    let names: Vec<String> = gems
        .iter()
        .map(|t| t.id.0.split('\\').next_back().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "a.mp3",   // oldest gem leads (310 days unheard)
            "a.b.mp3", // gems sort by last-played ascending, not by path
            "alpha.mp3",
            "zebra.mp3",  // 100 days unheard
            "future.mp3", // future last-played qualifies and sorts last among gems
            "never0.mp3", // unheard tail is path-ordered after all gems
            "never1.mp3",
            "nodate.mp3",
            "tie_a.mp3",
            "tie_b.mp3",
        ],
        "longest-unheard gems first, then never-played in path order; \
         recently played tracks are excluded"
    );
}

#[test]
fn test_smart_playlists_reflect_committed_mutations_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = seeded_smart_store(&db_path);

    let victim = TrackId("f:\\sm\\never0.mp3".to_string());
    let before: Vec<String> = store
        .smart_playlist(SmartPlaylistKind::NeverPlayed, usize::MAX)
        .expect("never played")
        .iter()
        .map(|t| t.id.0.clone())
        .collect();
    assert!(before.contains(&victim.0), "sanity: unplayed before");

    // One committed play through the mutation port...
    let played_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567);
    assert!(
        store
            .record_track_played(&victim, played_at)
            .expect("recording works"),
        "known track records"
    );

    // ...is visible to the very next smart-playlist query, no restart.
    let after_never: Vec<String> = store
        .smart_playlist(SmartPlaylistKind::NeverPlayed, usize::MAX)
        .expect("never played")
        .iter()
        .map(|t| t.id.0.clone())
        .collect();
    assert!(
        !after_never.contains(&victim.0),
        "played tracks leave Never Played immediately"
    );

    let most_played: Vec<(String, u32)> = store
        .smart_playlist(SmartPlaylistKind::MostPlayed, usize::MAX)
        .expect("most played")
        .iter()
        .map(|t| (t.id.0.clone(), t.play_count))
        .collect();
    let entry = most_played
        .iter()
        .find(|(id, _)| id == &victim.0)
        .expect("played track joins Most Played immediately");
    assert_eq!(entry.1, 1, "the recorded count is what queries see");
}

// --- Application Store: Clear Library (ticket 10) ---------------------------

/// Seed a full store: three tracks with play history across two albums and
/// two artists, a playlist with entries (one pointing at a track), and
/// every kind of setting.
fn seeded_clear_store(db_path: &std::path::Path) -> riff_backend::infra::store::SqliteStore {
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let mut store = riff_backend::infra::store::SqliteStore::open_and_migrate(db_path, changes_tx)
        .expect("fresh store must open");

    let mut t1 = browsing_track(
        "f:\\cl\\a\\one.mp3",
        "One",
        "Alpha",
        "First",
        Some(1),
        Some(2001),
    );
    t1.date_added = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
    let mut t2 = browsing_track(
        "f:\\cl\\a\\two.mp3",
        "Two",
        "Alpha",
        "First",
        Some(2),
        Some(2001),
    );
    t2.date_added = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(2_000));
    let t3 = browsing_track(
        "f:\\cl\\b\\three.mp3",
        "Three",
        "Beta",
        "Second",
        Some(1),
        Some(1999),
    );

    store
        .apply_scan_batch(&[t1, t2, t3])
        .expect("fixtures apply");
    // Play history: the wipe must take these derived facts with the tracks.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 4, last_played_nanos = 42
                 WHERE path = 'f:\\cl\\a\\one.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    let pid = store
        .create_playlist(
            "Curation",
            &[
                TrackId("f:\\cl\\a\\one.mp3".to_string()),
                TrackId("f:\\cl\\gone.mp3".to_string()),
            ],
        )
        .expect("playlist creation works");
    let _ = pid;

    store
        .save_scalars(&riff_backend::app::state::ScalarSettings {
            volume: Some(0.75),
            advanced_mode: true,
            high_contrast: false,
            replaygain_enabled: true,
        })
        .expect("scalars save");
    store
        .save_library_paths(&[PathBuf::from("f:\\cl")])
        .expect("paths save");
    let mut watches = std::collections::HashMap::new();
    watches.insert(
        PathBuf::from("f:\\cl"),
        crate::app::state::WatchState::Enabled,
    );
    store.save_watch_states(&watches).expect("watches save");

    store
}

#[test]
fn test_clear_library_wipes_collection_and_preserves_curation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = seeded_clear_store(&db_path);

    let removed = store.clear_library().expect("clear works");
    assert_eq!(removed, 3, "every track row is wiped in one action");

    // Collection tables are empty; queries see an empty library immediately.
    assert_eq!(store.track_count().expect("count"), 0);
    assert!(
        store.tracks_window(0, 50).expect("window").is_empty(),
        "flat list is empty without a restart"
    );
    assert!(
        store.all_artists().expect("artists").is_empty(),
        "browse view is empty"
    );
    assert!(
        !store
            .folder_has_audio(std::path::Path::new("f:\\cl"))
            .expect("folder probe"),
        "folder views see no audio"
    );
    for kind in [
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::NeverPlayed,
        SmartPlaylistKind::LostGems,
    ] {
        assert!(
            store
                .smart_playlist(kind, usize::MAX)
                .expect("smart playlist")
                .is_empty(),
            "{kind:?} reflects the cleared library"
        );
    }

    // Playlists survive with their entries listed — dangling references are
    // valid product behavior validated at read time.
    let playlists = store.load_playlists().expect("playlists load");
    assert_eq!(playlists.len(), 1, "curation survives the wipe");
    assert_eq!(playlists[0].name, "Curation");
    assert_eq!(
        playlists[0].tracks,
        vec![
            TrackId("f:\\cl\\a\\one.mp3".to_string()),
            TrackId("f:\\cl\\gone.mp3".to_string()),
        ],
        "entries stay listed even though their tracks are gone"
    );

    // Every setting survives untouched.
    let settings = store.load_settings().expect("settings load");
    assert_eq!(settings.scalars.volume, Some(0.75));
    assert!(settings.scalars.advanced_mode);
    assert!(!settings.scalars.high_contrast);
    assert!(settings.scalars.replaygain_enabled);
    assert_eq!(settings.library_paths, vec![PathBuf::from("f:\\cl")]);
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("f:\\cl")),
        Some(&crate::app::state::WatchState::Enabled),
    );
}

#[test]
fn test_clear_library_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    {
        let mut store = seeded_clear_store(&db_path);
        store.clear_library().expect("clear works");
    } // dropped like an app restart.

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
    let reopened = riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening works");
    assert_eq!(reopened.track_count().expect("count"), 0);
    assert_eq!(
        reopened.load_playlists().expect("playlists").len(),
        1,
        "curation persists across restarts"
    );
    assert_eq!(
        reopened.load_settings().expect("settings").library_paths,
        vec![PathBuf::from("f:\\cl")],
        "settings persist across restarts"
    );
}

#[test]
fn test_clear_library_is_atomic_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = seeded_clear_store(&db_path);

    // Simulate a mid-clear failure at the infra seam: an ABORT trigger on
    // the artists table fires after tracks and albums were already deleted
    // inside the transaction.
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER fail_clear BEFORE DELETE ON artists
                 BEGIN SELECT RAISE(ABORT, 'simulated mid-clear failure'); END;",
            )
        })
        .expect("trigger creation works");

    let outcome = store.clear_library();
    assert!(
        outcome.is_err(),
        "the simulated failure surfaces as an error"
    );

    // The rollback restored every deleted row: nothing partially cleared.
    assert_eq!(
        store.track_count().expect("count"),
        3,
        "tracks are fully restored after the failed wipe"
    );
    let artists: i64 = store
        .with_connection(|conn| conn.query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0)))
        .expect("artist count works");
    assert_eq!(artists, 2, "albums/artists are fully restored too");
    assert_eq!(
        store.load_playlists().expect("playlists").len(),
        1,
        "curation is untouched by the failed attempt"
    );

    // Removing the trigger lets the same call succeed cleanly afterwards.
    store
        .with_connection(|conn| conn.execute_batch("DROP TRIGGER fail_clear;"))
        .expect("trigger removal works");
    assert_eq!(store.clear_library().expect("clear works"), 3);
    assert_eq!(store.track_count().expect("count"), 0);
}

// --- Playlist adapter: session generation bumps only on committed mutations ---

use riff_backend::app::store::StoreGeneration;

/// Scratch store plus a clone wired to the session playlist generation
/// handle; `_dir` keeps the database file alive for the test.
struct PlaylistGenerationFixture {
    _dir: tempfile::TempDir,
    shared: riff_backend::infra::store::SqliteStore,
    generation: StoreGeneration,
    store: riff_backend::infra::store::SqliteStore,
}

impl PlaylistGenerationFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_backend::app::store::StoreChanged>();
        let shared =
            riff_backend::infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
                .expect("fresh store must open and migrate");
        let generation = shared.playlist_generation();
        let store = shared.clone();
        Self {
            _dir: dir,
            shared,
            generation,
            store,
        }
    }
}

/// Run one schema batch against the scratch connection (trigger plumbing).
fn exec_batch(shared: &riff_backend::infra::store::SqliteStore, batch: &str) {
    shared
        .with_connection(|conn| conn.execute_batch(batch))
        .expect("schema batch works");
}

#[test]
fn test_playlist_adapter_bumps_generation_on_committed_mutations() {
    let mut fx = PlaylistGenerationFixture::new();
    assert_eq!(fx.generation.current(), 0, "a fresh session starts at zero");

    let t1 = TrackId("a.mp3".to_string());
    let t2 = TrackId("b.mp3".to_string());

    let pid = fx
        .store
        .create_playlist("Focus Mix", &[])
        .expect("create works");
    assert_eq!(fx.generation.current(), 1, "create_playlist commit bumps");

    assert!(fx.store.rename_playlist(&pid, "Workout").unwrap());
    assert_eq!(fx.generation.current(), 2, "rename_playlist commit bumps");

    assert!(fx.store.add_playlist_entry(&pid, &t1).unwrap());
    assert_eq!(
        fx.generation.current(),
        3,
        "add_playlist_entry commit bumps"
    );

    assert!(fx.store.add_playlist_entry(&pid, &t2).unwrap());
    assert_eq!(fx.generation.current(), 4);

    assert!(
        fx.store
            .reorder_playlist_entries(&pid, &[t2.clone(), t1.clone()])
            .unwrap()
    );
    assert_eq!(
        fx.generation.current(),
        5,
        "reorder_playlist_entries commit bumps"
    );

    assert!(fx.store.remove_playlist_entries(&pid, &t1).unwrap());
    assert_eq!(
        fx.generation.current(),
        6,
        "remove_playlist_entries commit bumps"
    );

    assert!(fx.store.delete_playlist(&pid).unwrap());
    assert_eq!(fx.generation.current(), 7, "delete_playlist commit bumps");
}

#[test]
fn test_playlist_adapter_noop_mutations_do_not_bump_generation() {
    let mut fx = PlaylistGenerationFixture::new();
    let t1 = TrackId("a.mp3".to_string());
    let missing = TrackId("gone.mp3".to_string());
    let unknown = PlaylistId("playlist-does-not-exist".to_string());

    // One committed create seeds one entry and sets the baseline every
    // no-op compares against.
    let pid = fx
        .store
        .create_playlist("Focus Mix", std::slice::from_ref(&t1))
        .expect("create works");
    assert_eq!(fx.generation.current(), 1);

    assert!(!fx.store.add_playlist_entry(&pid, &t1).unwrap());
    assert!(!fx.store.remove_playlist_entries(&pid, &missing).unwrap());
    assert!(!fx.store.rename_playlist(&unknown, "New").unwrap());
    assert!(!fx.store.delete_playlist(&unknown).unwrap());
    assert!(!fx.store.add_playlist_entry(&unknown, &t1).unwrap());
    assert!(
        !fx.store
            .reorder_playlist_entries(&unknown, std::slice::from_ref(&t1))
            .unwrap()
    );
    assert_eq!(
        fx.generation.current(),
        1,
        "no-op Ok(false) mutations must not bump the generation"
    );
}
#[test]
fn test_playlist_adapter_failed_mutations_do_not_bump_generation() {
    let mut fx = PlaylistGenerationFixture::new();
    let t1 = TrackId("a.mp3".to_string());
    let t2 = TrackId("b.mp3".to_string());

    // A playlist holding one entry backs the entry-mutation failure cases.
    let pid = fx
        .store
        .create_playlist("Focus Mix", std::slice::from_ref(&t1))
        .expect("create works");
    assert_eq!(fx.generation.current(), 1);

    // Each case aborts exactly one kind of playlist statement mid-transaction
    // (same seam trick as the clear-library atomicity test above).
    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_create BEFORE INSERT ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated create failure'); END;",
    );
    assert!(
        fx.store.create_playlist("Broken", &[]).is_err(),
        "the simulated failure surfaces as an error"
    );
    assert_eq!(fx.generation.current(), 1, "failed create must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_create;");

    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_rename BEFORE UPDATE ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated rename failure'); END;",
    );
    assert!(fx.store.rename_playlist(&pid, "Broken").is_err());
    assert_eq!(fx.generation.current(), 1, "failed rename must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_rename;");

    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_delete BEFORE DELETE ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated delete failure'); END;",
    );
    assert!(fx.store.delete_playlist(&pid).is_err());
    assert_eq!(fx.generation.current(), 1, "failed delete must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_delete;");

    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_add BEFORE INSERT ON playlist_entries
         BEGIN SELECT RAISE(ABORT, 'simulated add failure'); END;",
    );
    assert!(fx.store.add_playlist_entry(&pid, &t2).is_err());
    assert_eq!(fx.generation.current(), 1, "failed add must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_add;");

    // remove and reorder both start by deleting playlist_entries rows.
    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_remove BEFORE DELETE ON playlist_entries
         BEGIN SELECT RAISE(ABORT, 'simulated remove failure'); END;",
    );
    assert!(fx.store.remove_playlist_entries(&pid, &t1).is_err());
    assert_eq!(fx.generation.current(), 1, "failed remove must not bump");
    assert!(
        fx.store
            .reorder_playlist_entries(&pid, std::slice::from_ref(&t1))
            .is_err()
    );
    assert_eq!(fx.generation.current(), 1, "failed reorder must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_remove;");

    // Sanity: with the trigger gone the same mutation commits and moves.
    assert!(fx.store.delete_playlist(&pid).unwrap());
    assert_eq!(
        fx.generation.current(),
        2,
        "commits after failures still bump"
    );
}
