// Bring the crate-root prelude (re-exported types) into this module so the
// inner `use super::*` can see the bare type names used in the tests.
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mocks::{MockAudioDecoder, MockAudioOutput, MockCoverLoader, MockMetadataReader};
    use riff_backend::app::errors::{LibraryError, PlaybackError};
    use riff_backend::app::traits::{
        AudioDecoder, AudioFormatInfo, AudioOutput, CoverImage, CoverLoader, MetadataReader,
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
        assert!(matches!(err, PlaybackError::Decode(_)));
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
            PlaybackError::Decode(_)
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
            PlaybackError::AudioOutput(_)
        ));
        assert!(output.initialized.is_empty());

        output.initialize_error = None;
        output.initialize(44_100, 2).unwrap();
        output.write_error = Some("device lost".to_string());
        assert!(matches!(
            output.write_samples(&[0.1, 0.2]).unwrap_err(),
            PlaybackError::AudioOutput(_)
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
            LibraryError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_duration(&path).unwrap_err(),
            LibraryError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_cover_source(&path).unwrap_err(),
            LibraryError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_audio_format(&path).unwrap_err(),
            LibraryError::MetadataRead(_)
        ));
        assert!(matches!(
            reader.read_all(&path).unwrap_err(),
            LibraryError::MetadataRead(_)
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
        assert!(matches!(err, LibraryError::CoverLoad(_)));
        assert!(err.to_string().contains("decode failed"));
    }
}
