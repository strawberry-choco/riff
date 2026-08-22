// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocks::{MockAudioDecoder, MockAudioOutput, MockCoverLoader, MockMetadataReader};
    use riff::app::errors::AppError;
    use riff::app::traits::{
        AudioDecoder, AudioFormatInfo, AudioOutput, CoverImage, CoverLoader, MetadataReader,
        MetadataWriter, TagEdit,
    };
    use riff::domain::CoverSource;
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

        // Scripted batches come out in order, then EOF (`None`).
        assert_eq!(decoder.next_frames(2).unwrap(), Some(vec![0.1, 0.2]));
        assert_eq!(decoder.next_frames(3).unwrap(), Some(vec![0.3, 0.4, 0.5]));
        assert_eq!(decoder.next_frames(2).unwrap(), None);
        assert_eq!(decoder.next_frames(2).unwrap(), None);

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
        assert_eq!(decoder.next_frames(1).unwrap(), Some(vec![1.0]));
        decoder.seek(Duration::from_secs(5)).unwrap();
        assert_eq!(decoder.seeks, vec![Duration::from_secs(5)]);
        assert_eq!(decoder.next_frames(1).unwrap(), Some(vec![1.0]));
        assert_eq!(decoder.next_frames(1).unwrap(), Some(vec![2.0]));
        assert_eq!(decoder.next_frames(1).unwrap(), None);
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
        assert_eq!(decoder.next_frames(1).unwrap(), Some(vec![0.5]));
        decoder.decode_error = Some("corrupt frame".to_string());
        assert!(matches!(
            decoder.next_frames(1).unwrap_err(),
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
            cover_source: CoverSource::Embedded(vec![1, 2, 3]),
            ..Default::default()
        };

        let (metadata, duration, cover, format) = reader
            .read_all(&PathBuf::from("a.mp3"))
            .expect("read_all should succeed");
        assert_eq!(metadata.title.as_deref(), Some("Song"));
        assert_eq!(metadata.artist.as_deref(), Some("Artist"));
        assert_eq!(duration, Some(Duration::from_secs(90)));
        assert!(matches!(cover, CoverSource::Embedded(ref data) if data == &vec![1, 2, 3]));
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
            .load_cover(&CoverSource::Embedded(vec![9]))
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
        let mut codec_registry = symphonia::core::codecs::CodecRegistry::new();
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

    #[test]
    fn test_filesystem_watcher_new() {
        // This test might fail if there are filesystem permissions issues
        let (tx, _) = crossbeam_channel::unbounded();
        let _result = FilesystemWatcher::new(tx);
        // The result could be Ok or Err depending on the system
    }
}

// --- Application Store: connection setup + checksummed migrations --------

#[test]
fn test_store_fresh_start_creates_file_and_applies_initial_migration_once() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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

    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
    store
        .with_connection(|conn| conn.execute("UPDATE schema_migrations SET applied_at = 12345", []))
        .expect("marking applied_at must work");
    drop(store);

    let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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

    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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

    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
    let db_path = dir.path().join("nope").join("riff.sqlite3");

    let Err(err) = riff::infra::store::SqliteStore::open_and_migrate(&db_path) else {
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

    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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

    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
    // checkpointed and the connection released.
    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
    let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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

    let Err(err) = riff::infra::store::SqliteStore::open_and_migrate(&db_path) else {
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

use riff::app::store::SettingsStore;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_store_scalar_settings_roundtrip_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Fresh store: defaults (volume unset, toggles off).
    {
        let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
        let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
        store
            .save_scalars(&riff::app::state::ScalarSettings {
                volume: Some(0.42),
                advanced_mode: true,
                high_contrast: true,
                replaygain_enabled: true,
            })
            .expect("saving scalars must work");
    }

    // Reopen: every value survives in its typed column.
    let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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
        let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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

    let mut reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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

use riff::app::store::PlaylistStore;
use riff::domain::{PlaylistId, TrackId};

#[test]
fn test_store_fresh_playlists_are_empty() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
        let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
    let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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
        let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
        let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
        let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
        pid = store.create_playlist("P", &[]).unwrap();

        // Appends land at the end, in call order.
        assert!(store
            .add_playlist_entry(&pid, &TrackId("a.mp3".to_string()))
            .unwrap());
        assert!(store
            .add_playlist_entry(&pid, &TrackId("b.mp3".to_string()))
            .unwrap());
        // Exact duplicates are rejected.
        assert!(!store
            .add_playlist_entry(&pid, &TrackId("a.mp3".to_string()))
            .unwrap());
        // Unknown playlist ids are no-ops.
        assert!(!store
            .add_playlist_entry(&PlaylistId("x".to_string()), &TrackId("c.mp3".to_string()))
            .unwrap());

        // Removal reports whether anything was removed.
        assert!(store
            .remove_playlist_entries(&pid, &TrackId("a.mp3".to_string()))
            .unwrap());
        assert!(!store
            .remove_playlist_entries(&pid, &TrackId("zzz.mp3".to_string()))
            .unwrap());

        // A dangling reference (no tracks table exists at all) commits like
        // any other entry.
        assert!(store
            .add_playlist_entry(&pid, &TrackId("vanished\\file.flac".to_string()))
            .unwrap());
        drop(store);
    }

    // Restart: ordering and the dangling entry both survive; the entry stays
    // listed so it resolves again once the referenced file returns.
    let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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

    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
    // Same-millisecond creation of same-named playlists must not collide:
    // ids dedupe, names may repeat (today's behavior).
    let a = store.create_playlist("Mix", &[]).unwrap();
    let b = store.create_playlist("Mix", &[]).unwrap();
    assert_ne!(a, b, "ids must be unique even for identical names");
    let playlists = store.load_playlists().unwrap();
    assert_eq!(playlists.len(), 2);
    assert!(playlists.iter().all(|p| p.name == "Mix"));
}

// --- Application Store: Library collection (ticket 05) ---------------------

use riff::app::store::{LibraryMutationStore, LibraryQueryStore};
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
    }
}

#[test]
fn test_store_library_migration_004_applies_and_reopens_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path)
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

    riff::infra::store::SqliteStore::open_and_migrate(&db_path)
        .expect("reopening a store with the library schema must succeed");
}

#[test]
fn test_scan_batch_populates_collection_including_compilations() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

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
        let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
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
    let reopened = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();
    let count: i64 = reopened
        .with_connection(|conn| conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0)))
        .expect("counting tracks must work");
    assert_eq!(count, 1, "committed scan progress survives an interruption");
}

#[test]
fn test_rescan_is_idempotent_and_preserves_history() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

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
    let store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

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
    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

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
fn test_search_parity_with_legacy_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

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
    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

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

#[test]
fn test_load_collection_rebuilds_mirror_compatible_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = riff::infra::store::SqliteStore::open_and_migrate(&db_path).unwrap();

    let mut t_two = library_track("f:\\first\\c.mp3", "Two", Some("Alpha"), "First", None);
    t_two.metadata.track_number = Some(2);
    let t_no_number = library_track(
        "f:\\first\\b.mp3",
        "No Number",
        Some("Alpha"),
        "First",
        None,
    );
    let mut t_one = library_track("f:\\first\\a.mp3", "One", Some("Alpha"), "First", None);
    t_one.metadata.track_number = Some(1);
    let second = library_track("f:\\second\\x.mp3", "X", Some("Alpha"), "Second", None);
    let beta = library_track("f:\\beta\\y.mp3", "Y", Some("Beta"), "Only", None);

    // Insertion order deliberately differs from track-number order.
    store
        .apply_scan_batch(&[t_two.clone(), t_no_number.clone(), beta.clone()])
        .expect("batch 1 applies");
    store
        .apply_scan_batch(&[second.clone(), t_one.clone()])
        .expect("batch 2 applies");

    let collection = store.load_collection().expect("collection loads");

    assert_eq!(collection.tracks.len(), 5, "all tracks hydrate by path key");
    assert!(collection.tracks.contains_key(&t_one.id));

    // Albums keyed by the legacy composite format; album tracks ordered by
    // number with missing numbers first (legacy unwrap_or(0) behavior),
    // path tiebreak.
    let first = collection
        .albums
        .iter()
        .find(|a| a.artist == "Alpha" && a.title == "First")
        .expect("Alpha - First exists");
    assert_eq!(
        first.tracks,
        vec![t_no_number.id.clone(), t_one.id.clone(), t_two.id.clone()],
        "album tracks ordered by number, missing numbers first, path tiebreak"
    );

    // Artists list their albums as composite keys in first-added order.
    let alpha = collection
        .artists
        .iter()
        .find(|a| a.name == "Alpha")
        .expect("Alpha exists");
    assert_eq!(
        alpha.albums,
        vec!["Alpha - First".to_string(), "Alpha - Second".to_string()],
        "artist's album keys follow first-added order"
    );
    let beta = collection
        .artists
        .iter()
        .find(|a| a.name == "Beta")
        .expect("Beta exists");
    assert_eq!(beta.albums, vec!["Beta - Only".to_string()]);
}
