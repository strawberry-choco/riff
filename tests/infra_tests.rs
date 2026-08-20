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
