// ReplayGain tag-writing tests (ReplayGain seam 1's write side).
//
// The write path's contract: the string encoding the reader's gain-string
// parser accepts is the same one the writer emits, so a file riff has
// measured parses back to the values riff wrote. Opus is the one per-format
// exception — RFC 7845 R128-convention gains in their Q7.8 unit encoding.
//
// The Opus fixture is a hand-built Ogg Opus stream (OpusHead + OpusTags
// pages, byte-by-byte, CRC'd), the same no-binary-fixtures precedent as the
// WAV fixtures: lofty seeds it directly so the test does not depend on
// riff's own writer, and riff's writer is proven against it in both
// directions.

use super::adapter_tests::{seed_file_with_items, write_minimal_wav};
use super::*;
use lofty::file::TaggedFileExt;
use lofty::tag::ItemKey;
use riff_infra::media::LoftyMetadataWriter;
use riff_library::app::traits::{ReplayGainTags, ReplayGainWriter};

// --- Ogg Opus fixture --------------------------------------------------------

/// The Ogg page CRC: polynomial 0x04c11db7, init 0, no reflection, no final
/// XOR — not a standard CRC-32.
fn ogg_crc32(data: &[u8]) -> u32 {
    let mut table = [0_u32; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        #[allow(clippy::cast_possible_truncation)] // i < 256, so the cast is lossless
        let mut r = i as u32;
        for _ in 0..8 {
            r = if r & 1 != 0 {
                0x04c1_1db7 ^ (r >> 1)
            } else {
                r >> 1
            };
        }
        *entry = r;
    }
    let mut crc = 0_u32;
    for &byte in data {
        crc = (crc << 8) ^ table[((crc >> 24) as usize ^ usize::from(byte)) & 0xff];
    }
    crc
}

/// One Ogg page carrying `payload` (must fit 255-block lacing — every packet
/// here does).
fn ogg_page(header_type: u8, granule: u64, serial: u32, sequence: u32, payload: &[u8]) -> Vec<u8> {
    let mut segment_table = Vec::new();
    let mut remaining = payload.len();
    while remaining >= 255 {
        segment_table.push(255_u8);
        remaining -= 255;
    }
    #[allow(clippy::cast_possible_truncation)] // the remainder is < 255
    segment_table.push(remaining as u8);

    let mut page = Vec::with_capacity(27 + segment_table.len() + payload.len());
    page.extend_from_slice(b"OggS");
    page.push(0); // stream structure version
    page.push(header_type);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&serial.to_le_bytes());
    page.extend_from_slice(&sequence.to_le_bytes());
    page.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
    #[allow(clippy::cast_possible_truncation)] // fixtures fit one page's lacing
    page.push(segment_table.len() as u8);
    page.extend_from_slice(&segment_table);
    page.extend_from_slice(payload);
    let crc = ogg_crc32(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// The 19-byte identification header RFC 7845 mandates as the stream's first
/// packet: version 1, two channels, no pre-skip, 44.1 kHz input rate, no
/// output gain, one stream.
fn opus_head_packet() -> Vec<u8> {
    let mut packet = Vec::with_capacity(19);
    packet.extend_from_slice(b"OpusHead");
    packet.push(1); // version
    packet.push(2); // channel count
    packet.extend_from_slice(&0_u16.to_le_bytes()); // pre-skip
    packet.extend_from_slice(&44_100_u32.to_le_bytes()); // input sample rate
    packet.extend_from_slice(&0_i16.to_le_bytes()); // output gain
    packet.push(0); // channel mapping family
    packet
}

/// The comment header: `OpusTags`, a vendor string, then `KEY=value`
/// comments.
fn opus_tags_packet(items: &[(&str, &str)]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(b"OpusTags");
    let vendor = b"riff test suite";
    #[allow(clippy::cast_possible_truncation)]
    packet.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    packet.extend_from_slice(vendor);
    #[allow(clippy::cast_possible_truncation)]
    packet.extend_from_slice(&(items.len() as u32).to_le_bytes());
    for (key, value) in items {
        let comment = format!("{key}={value}");
        #[allow(clippy::cast_possible_truncation)] // fixture comments are small
        packet.extend_from_slice(&(comment.len() as u32).to_le_bytes());
        packet.extend_from_slice(comment.as_bytes());
    }
    packet
}

/// Write a minimal valid Ogg Opus stream carrying `items` as its `OpusTags`
/// comments. No audio packets — the tags reader is the only consumer.
fn write_minimal_opus(path: &std::path::Path, items: &[(&str, &str)]) {
    let serial = 0x1234_5678;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&ogg_page(0x02, 0, serial, 0, &opus_head_packet()));
    bytes.extend_from_slice(&ogg_page(0x00, 0, serial, 1, &opus_tags_packet(items)));
    bytes.extend_from_slice(&ogg_page(0x04, 0, serial, 2, &[]));
    std::fs::write(path, bytes).expect("opus fixture must be writable");
}

/// Read the raw tag items of a file back through lofty directly, so the
/// tests can pin the exact on-disk convention independent of riff's reader.
fn raw_item(path: &std::path::Path, key: ItemKey) -> Option<String> {
    let tagged_file = lofty::read_from_path(path).ok()?;
    let tag = tagged_file.primary_tag()?;
    tag.get_string(key).map(str::to_string)
}

// --- Writer round-trip (non-Opus) ---------------------------------------------

/// A file riff has measured parses back to the values riff wrote, through
/// riff's own reader — the write contract is the read contract.
#[test]
fn test_writer_replaygain_round_trips_through_riffs_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measured.wav");
    write_minimal_wav(&path);

    LoftyMetadataWriter::new()
        .write_replaygain(
            &path,
            &ReplayGainTags {
                track_gain: Some(-6.54),
                track_peak: Some(0.98),
                album_gain: Some(-7.12),
                album_peak: Some(0.81),
            },
        )
        .expect("the measurement must write");

    let metadata = LoftyMetadataReader::new()
        .read_metadata(&path)
        .expect("the measured file must read");
    assert_eq!(metadata.replaygain_track_gain, Some(-6.54));
    assert_eq!(metadata.replaygain_track_peak, Some(0.98));
    assert_eq!(metadata.replaygain_album_gain, Some(-7.12));
    assert_eq!(metadata.replaygain_album_peak, Some(0.81));
}

/// Non-Opus formats keep the RG-convention item keys with the ` dB` suffix —
/// the exact strings every other player's parser expects.
#[test]
fn test_writer_writes_rg_convention_strings_on_non_opus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measured.wav");
    write_minimal_wav(&path);

    LoftyMetadataWriter::new()
        .write_replaygain(
            &path,
            &ReplayGainTags {
                track_gain: Some(-6.54),
                track_peak: Some(0.98),
                album_gain: Some(-7.12),
                album_peak: Some(0.81),
            },
        )
        .expect("the measurement must write");

    assert_eq!(
        raw_item(&path, ItemKey::ReplayGainTrackGain).as_deref(),
        Some("-6.54 dB")
    );
    assert_eq!(
        raw_item(&path, ItemKey::ReplayGainTrackPeak).as_deref(),
        Some("0.980000")
    );
    assert_eq!(
        raw_item(&path, ItemKey::ReplayGainAlbumGain).as_deref(),
        Some("-7.12 dB")
    );
    assert_eq!(
        raw_item(&path, ItemKey::ReplayGainAlbumPeak).as_deref(),
        Some("0.810000")
    );
}

/// A write names only what it sets: an album-pair write leaves an existing
/// track pair and unrelated tags exactly as it found them.
#[test]
fn test_writer_replaygain_write_does_not_clobber_untouched_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seeded.wav");
    seed_file_with_items(
        &path,
        &[
            (lofty::tag::ItemKey::TrackTitle, "Keep Me"),
            (lofty::tag::ItemKey::ReplayGainTrackGain, "-6.54 dB"),
            (lofty::tag::ItemKey::ReplayGainTrackPeak, "0.98"),
        ],
    );

    LoftyMetadataWriter::new()
        .write_replaygain(&path, &ReplayGainTags::album_pair(-7.12, 0.81))
        .expect("the album pair must write");

    let metadata = LoftyMetadataReader::new()
        .read_metadata(&path)
        .expect("the seeded file must read");
    assert_eq!(metadata.title.as_deref(), Some("Keep Me"));
    assert_eq!(metadata.replaygain_track_gain, Some(-6.54), "untouched");
    assert_eq!(metadata.replaygain_track_peak, Some(0.98), "untouched");
    assert_eq!(metadata.replaygain_album_gain, Some(-7.12), "newly written");
    assert_eq!(metadata.replaygain_album_peak, Some(0.81), "newly written");
}

// --- Opus: RFC 7845 R128 convention ---------------------------------------------

/// Opus files *read* R128-convention items per RFC 7845: an integer in Q7.8
/// (units of 1/256 dB) converts to dB in the reader.
#[test]
fn test_reader_reads_r128_convention_gains_from_opus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r128.opus");
    // -1674/256 = −6.5390625 dB; -2048/256 = exactly −8 dB.
    write_minimal_opus(
        &path,
        &[
            ("R128_TRACK_GAIN", "-1674"),
            ("R128_ALBUM_GAIN", "-2048"),
            ("REPLAYGAIN_TRACK_PEAK", "0.980000"),
            ("REPLAYGAIN_ALBUM_PEAK", "0.810000"),
        ],
    );

    let metadata = LoftyMetadataReader::new()
        .read_metadata(&path)
        .expect("the opus fixture must read");
    assert!((metadata.replaygain_track_gain.unwrap() - (-6.539_062_5)).abs() < 1e-4);
    assert_eq!(metadata.replaygain_album_gain, Some(-8.0));
    // Peaks have no R128 standard: they ride the RG-convention items.
    assert_eq!(metadata.replaygain_track_peak, Some(0.98));
    assert_eq!(metadata.replaygain_album_peak, Some(0.81));
}

/// An Opus file tagged RG-convention (older tooling) still reads: R128 wins
/// when present, RG convention fills in when absent.
#[test]
fn test_reader_opus_falls_back_to_rg_convention_gains() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rg_convention.opus");
    write_minimal_opus(
        &path,
        &[
            ("REPLAYGAIN_TRACK_GAIN", "-6.54 dB"),
            ("REPLAYGAIN_ALBUM_GAIN", "-7.12 dB"),
        ],
    );

    let metadata = LoftyMetadataReader::new()
        .read_metadata(&path)
        .expect("the opus fixture must read");
    assert_eq!(metadata.replaygain_track_gain, Some(-6.54));
    assert_eq!(metadata.replaygain_album_gain, Some(-7.12));
}

/// riff *writes* Opus in the R128 convention: the Q7.8 integer lands on the
/// R128 items, and the same values read back through riff's reader to within
/// the format's own 1/256 dB quantization.
#[test]
fn test_writer_writes_r128_convention_gains_on_opus() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("measured.opus");
    write_minimal_opus(&path, &[]);

    LoftyMetadataWriter::new()
        .write_replaygain(
            &path,
            &ReplayGainTags {
                track_gain: Some(-6.54),
                track_peak: Some(0.98),
                album_gain: Some(-8.0),
                album_peak: Some(0.81),
            },
        )
        .expect("the measurement must write");

    // The on-disk convention: R128 items carry the Q7.8 integers, peaks stay
    // RG-convention bare ratios.
    assert_eq!(
        raw_item(&path, ItemKey::R128TrackGain).as_deref(),
        Some("-1674")
    );
    assert_eq!(
        raw_item(&path, ItemKey::R128AlbumGain).as_deref(),
        Some("-2048")
    );
    assert_eq!(
        raw_item(&path, ItemKey::ReplayGainTrackPeak).as_deref(),
        Some("0.980000")
    );
    assert_eq!(
        raw_item(&path, ItemKey::ReplayGainAlbumPeak).as_deref(),
        Some("0.810000")
    );
    // No RG-convention gain items are written alongside.
    assert_eq!(raw_item(&path, ItemKey::ReplayGainTrackGain), None);

    // And the values read back, to the format's own quantization.
    let metadata = LoftyMetadataReader::new()
        .read_metadata(&path)
        .expect("the measured opus must read");
    assert!((metadata.replaygain_track_gain.unwrap() - (-6.54)).abs() <= 1.0 / 256.0);
    assert_eq!(metadata.replaygain_album_gain, Some(-8.0));
    assert_eq!(metadata.replaygain_track_peak, Some(0.98));
    assert_eq!(metadata.replaygain_album_peak, Some(0.81));
}

/// An empty write changes nothing and need not even open the file.
#[test]
fn test_writer_empty_replaygain_write_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.wav");

    let result = LoftyMetadataWriter::new().write_replaygain(&path, &ReplayGainTags::default());
    assert!(result.is_ok(), "an empty write is a no-op, not an error");
    assert!(!path.exists(), "the no-op must not create the file");
}
