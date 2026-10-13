// riff-persistence — the audio-extension list's one house (deepen-iter1 02).
//
// `AUDIO_EXTENSIONS` is the single declared list of formats the Library Scan
// indexes. Three sites used to spell their own copy of the seven values;
// each now derives them from this one declaration, so a format added here
// reaches every consumer: the persisted scan defaults and the scan's own
// default options here, `ScanPrefs::default()` in `riff-backend`, and the
// watcher's batch filter in `riff-infra`.
//
// These tests lock the values those sites resolve. A reintroduced private
// copy cannot drift silently: it would have to resolve these seven too, or
// these assertions fail.

use riff_persistence::store::{AUDIO_EXTENSIONS, ScalarSettings, ScanOptions};

/// The canonical seven as the `Vec<String>` every consumer stores — the
/// shape the persisted settings, the scan options, and the backend's
/// `ScanPrefs` all carry.
fn canonical_formats() -> Vec<String> {
    AUDIO_EXTENSIONS
        .iter()
        .map(|extension| (*extension).to_string())
        .collect()
}

/// The one house itself: seven formats, lowercase, no dots.
#[test]
fn the_canonical_list_is_seven_lowercase_extensions() {
    assert_eq!(
        AUDIO_EXTENSIONS.to_vec(),
        vec!["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"],
        "the canonical list is exactly the seven formats"
    );
    for extension in AUDIO_EXTENSIONS {
        assert_eq!(
            extension.to_lowercase(),
            *extension,
            "every entry is already lowercase"
        );
        assert!(
            !extension.starts_with('.'),
            "no entry carries a leading dot"
        );
    }
}

/// The persisted scan default resolves the canonical seven, in the
/// canonical order.
#[test]
fn scalar_settings_default_resolves_the_canonical_extensions() {
    assert_eq!(
        ScalarSettings::default().scan_formats,
        canonical_formats(),
        "the persisted scan default derives from the one canonical list"
    );
}

/// The Library Scan's own default options resolve the canonical seven, in
/// the canonical order.
#[test]
fn scan_options_default_resolves_the_canonical_extensions() {
    assert_eq!(
        ScanOptions::default().formats,
        canonical_formats(),
        "the scan's default options derive from the one canonical list"
    );
}
