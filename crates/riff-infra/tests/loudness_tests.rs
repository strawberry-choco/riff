// Loudness-analyzer adapter tests (ReplayGain 2.0 seam).
//
// The analyzer port is exercised over real decode of real files whose
// contents are synthetic PCM at known levels, so the expected dB values come
// from reference behavior — the BS.1770 calibration anchor: a 997 Hz full-
// scale sine measures −3.01 LUFS (mono), and loudness scales exactly with
// 20·log10(amplitude) away from it. The album aggregate asserts the worked
// example of the energy-weighted formula recorded with the port.

use riff_infra::audio::Ebur128LoudnessAnalyzer;
use riff_library::app::replaygain::album_aggregate;
use riff_library::app::traits::{LoudnessAnalyzer, TrackLoudness};

/// Write a PCM WAV carrying `seconds` of a mono 997 Hz sine at `amplitude`
/// (linear, full scale = 1.0), 16-bit at 44.1 kHz — the BS.1770 calibration
/// frequency. Written byte-by-byte so the fixture depends on nothing but the
/// WAV format itself.
fn write_sine_wav(path: &std::path::Path, seconds: f32, amplitude: f32) {
    const SAMPLE_RATE: u32 = 44_100;
    const FREQUENCY: f32 = 997.0;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let samples = (seconds * SAMPLE_RATE as f32) as u32;
    let data_size = samples * 2; // 16-bit mono
    let mut bytes = Vec::with_capacity(44 + data_size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..samples {
        let t = f64::from(i) / f64::from(SAMPLE_RATE);
        let value =
            f64::from(amplitude) * (2.0 * std::f64::consts::PI * f64::from(FREQUENCY) * t).sin();
        #[allow(clippy::cast_possible_truncation)]
        let sample = (value * 32_767.0).round() as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("sine WAV fixture must be writable");
}

fn analyzer() -> Ebur128LoudnessAnalyzer {
    Ebur128LoudnessAnalyzer::new()
}

/// The full-scale mono sine anchor: −3.01 LUFS against the −18 LUFS
/// reference → −14.99 dB of track gain (the Track is louder than the
/// reference, so playback attenuates). The true peak of a band-limited
/// full-scale sine is the sample peak itself, 1.0.
#[test]
fn test_analyzer_measures_a_full_scale_sines_gain_and_true_peak() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("full_scale.wav");
    write_sine_wav(&path, 2.0, 1.0);

    let loudness = analyzer()
        .measure_track(&path)
        .expect("the sine fixture must measure");

    assert!(
        (loudness.track_gain_db - (-14.99)).abs() < 0.2,
        "full-scale 997 Hz sine is −3.01 LUFS, so gain = −18 − (−3.01), got {}",
        loudness.track_gain_db
    );
    assert!(
        (loudness.track_peak - 1.0).abs() < 0.02,
        "a full-scale sine's true peak is 1.0, got {}",
        loudness.track_peak
    );
}

/// Loudness scales with 20·log10(amplitude): halving the amplitude lowers
/// the loudness by exactly 6.02 dB and the peak by exactly half.
#[test]
fn test_analyzer_measures_a_quieter_sine_at_the_reference_offset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half.wav");
    write_sine_wav(&path, 2.0, 0.5);

    let loudness = analyzer()
        .measure_track(&path)
        .expect("the sine fixture must measure");

    // −3.01 − 6.02 = −9.03 LUFS → gain = −8.97 dB.
    assert!(
        (loudness.track_gain_db - (-8.97)).abs() < 0.2,
        "half-amplitude sine is −9.03 LUFS, got gain {}",
        loudness.track_gain_db
    );
    assert!(
        (loudness.track_peak - 0.5).abs() < 0.02,
        "half-amplitude sine's true peak is 0.5, got {}",
        loudness.track_peak
    );
}

/// The silent file's verdict: BS.1770 reports no integrated loudness for a
/// signal that is exactly zero, so its gain would come back infinite — a value
/// no tag form carries, no other player honors, and no Album aggregate can
/// weigh. The measurement falls back instead: 0 dB of gain (play it exactly as
/// decoded, the one value that provably neither amplifies nor attenuates), and
/// its true peak, which silence does measure — a real 0.0, kept as measured.
#[test]
fn test_analyzer_falls_back_to_unity_gain_for_a_silent_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("silent.wav");
    // Amplitude 0.0 is digital silence: every sample the fixture writes is 0.
    write_sine_wav(&path, 2.0, 0.0);

    let loudness = analyzer()
        .measure_track(&path)
        .expect("a silent file still measures, with the no-verdict pair");

    assert!(
        loudness.track_gain_db.abs() < f32::EPSILON,
        "no verdict falls back to 0 dB of gain, got {}",
        loudness.track_gain_db
    );
    assert!(
        loudness.track_peak.abs() < f32::EPSILON,
        "silence measures a real peak of 0.0, which the fallback must not overwrite, got {}",
        loudness.track_peak
    );
}

/// The album aggregate over two measured members: the energy-weighted mean
/// of their loudnesses, weighted by duration, against the reference — and
/// the peak is the maximum of the member peaks. Worked example, independent
/// of the implementation:
///
/// - member A: 2.0 s at −9.03 LUFS (gain −8.97 dB, peak 0.5)
/// - member B: 1.0 s at −15.03 LUFS (gain −2.97 dB, peak 0.25)
/// - album loudness = −0.691 + 10·log10((2·10^((−9.03+0.691)/10)
///   + 1·10^((−15.03+0.691)/10)) / 3) = −10.28 LUFS
/// - album gain = −18 − (−10.28) = −7.72 dB; album peak = 0.5.
#[test]
fn test_analyzer_album_aggregate_is_energy_weighted_with_max_peak() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_sine_wav(&a, 2.0, 0.5);
    write_sine_wav(&b, 1.0, 0.25);

    let analyzer = analyzer();
    let loud_a = analyzer.measure_track(&a).expect("member A must measure");
    let loud_b = analyzer.measure_track(&b).expect("member B must measure");

    // The members land where the calibration anchor says they must, so the
    // aggregate's inputs are the worked example's.
    assert!((loud_a.track_gain_db - (-8.97)).abs() < 0.2);
    assert!((loud_b.track_gain_db - (-2.97)).abs() < 0.2);

    let album =
        album_aggregate(&[(loud_a, 2.0), (loud_b, 1.0)]).expect("two measured members aggregate");
    assert!(
        (album.album_gain_db - (-7.72)).abs() < 0.3,
        "energy-weighted album gain over the two members, got {}",
        album.album_gain_db
    );
    assert!(
        (album.album_peak - 0.5).abs() < 0.02,
        "album peak is the maximum of the member peaks, got {}",
        album.album_peak
    );
}

/// A file that is not audio reports a normal measurement error — no panic,
/// no crash.
#[test]
fn test_analyzer_unreadable_file_returns_a_graceful_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_audio.txt");
    std::fs::write(&path, "definitely not audio").unwrap();

    let result = analyzer().measure_track(&path);
    assert!(result.is_err(), "a non-audio file must fail to measure");
}

/// The analyzer is used through the port object in production, so the port
/// object must be usable as one (trait-object safety smoke check).
#[test]
fn test_analyzer_satisfies_the_port_through_a_trait_object() {
    let port: Box<dyn LoudnessAnalyzer> = Box::new(analyzer());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sine.wav");
    write_sine_wav(&path, 1.0, 0.5);
    let loudness = port.measure_track(&path).expect("port object must measure");
    assert!(loudness.track_gain_db.is_finite());
    assert!(loudness.track_peak.is_finite());
}

/// The loudness type crosses the port, so it must be shareable.
#[test]
fn test_track_loudness_is_copy_and_comparable() {
    let a = TrackLoudness {
        track_gain_db: -8.97,
        track_peak: 0.5,
    };
    let b = a;
    assert_eq!(a, b);
}
