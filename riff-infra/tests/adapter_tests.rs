// riff-infra — adapter tests for the media and filesystem adapters.
//
// Moved verbatim from the workspace-root suite's `infra_tests` module by
// backend-crate-split issue 07: the lofty tag round-trips, the construction
// smoke tests, and the filesystem-watcher tests belong to `riff-infra`.
// Import paths were rewired to `riff-library` / `riff-persistence` /
// `riff-infra`; every pre-existing assertion is unchanged.

use super::*;
use riff_library::app::traits::MetadataWriter;

// --- ReplayGain tag parsing (pure) -------------------------------------------

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
        ..Default::default()
    };
    LoftyMetadataWriter::new()
        .write_tags(&path, &edit)
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
        .write_tags(
            &path,
            &TagEdit {
                title: Some("Flamenco Sketches".to_string()),
                artist: Some("Miles Davis".to_string()),
                album: Some("Kind of Blue".to_string()),
                album_artist: None,
                genre: Some("Jazz".to_string()),
                year: Some(1959),
                track_number: Some(5),
                ..Default::default()
            },
        )
        .expect("initial tag write must succeed");

    // A later edit touching only the title must not disturb the rest.
    writer
        .write_tags(
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
fn test_lofty_writer_extended_fields_round_trip() {
    // The tag-edit DTO carries the full metadata surface (disc number,
    // composer, comment, ReplayGain); every `Some` field must reach the
    // file tags and read back through the real reader.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.wav");
    write_minimal_wav(&path);

    let edit = TagEdit {
        title: Some("Blue in Green".to_string()),
        disc_number: Some(2),
        composer: Some("Bill Evans".to_string()),
        comment: Some("take 3".to_string()),
        replaygain_track_gain: Some(-6.54),
        replaygain_track_peak: Some(0.5),
        ..Default::default()
    };
    LoftyMetadataWriter::new()
        .write_tags(&path, &edit)
        .expect("writing extended fields must succeed");

    let metadata = LoftyMetadataReader::new().read_metadata(&path).unwrap();
    assert_eq!(metadata.disc_number, Some(2));
    assert_eq!(metadata.composer.as_deref(), Some("Bill Evans"));
    assert_eq!(metadata.comment.as_deref(), Some("take 3"));
    let gain = metadata
        .replaygain_track_gain
        .expect("gain must round-trip");
    assert!((gain - -6.54).abs() < 1e-4, "gain: {gain}");
    let peak = metadata
        .replaygain_track_peak
        .expect("peak must round-trip");
    assert!((peak - 0.5).abs() < 1e-4, "peak: {peak}");
}

#[test]
fn test_lofty_writer_unsupported_file_returns_graceful_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "this is definitely not an audio file").unwrap();

    let result = LoftyMetadataWriter::new().write_tags(
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
        LibraryError::MetadataWrite(_)
    ));
}

#[test]
fn test_lofty_writer_missing_file_returns_graceful_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("never-existed.wav");

    let result = LoftyMetadataWriter::new().write_tags(&path, &TagEdit::default());

    assert!(matches!(
        result.expect_err("missing file must fail the write"),
        LibraryError::MetadataWrite(_)
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
        riff_infra::filesystem::watcher::debounced_audio_paths(&events),
        vec![PathBuf::from("/lib/a.mp3"), PathBuf::from("/lib/b.FLAC")]
    );
}

#[test]
fn test_filesystem_watcher_forwards_debounced_audio_batches() {
    use riff_library::app::traits::FilesystemWatch;
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
