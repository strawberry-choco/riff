# Changelog

All notable changes to riff are documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.0]

### Added

- Metadata tag editing: right-click → Edit Tags opens a modal with the editable fields; writes run on a background thread via lofty, the library updates without a rescan, and failures surface as graceful errors naming the file.
- Smart/discovery playlists generated locally from library data: Recently Added, Most Played, Never Played, and Lost Gems (unheard for 90+ days). Play counts persist in the library cache across restarts.
- Custom playlist management: create, rename, and delete named playlists with ordered tracks, plus an add-to-playlist context menu with deduplication. Playlists persist in `playlists.json`, separate from the rebuildable library cache. Entries whose files are missing render struck-through as "(missing)" and are excluded from playback.
- Gapless playback: the engine pre-decodes the next track starting about two seconds before end-of-file and stages up to four seconds of samples. Same-format handoffs — including repeat-one — are seamless; format mismatches and mid-transition shuffle changes fall back to the gapped path.
- ReplayGain support: REPLAYGAIN_TRACK_GAIN and REPLAYGAIN_TRACK_PEAK tags are read during scans, and the peak-capped track gain is applied at the volume-scaling stage of the audio output callback. Opt-in via settings; takes effect on the next track; untagged tracks play untouched.
- Progressive disclosure: an advanced-mode toggle reveals tag editing, smart playlists, and the transport's stop control; the default UI stays minimal.
- Accessibility: full keyboard navigation, a visible focus indicator, and a high-contrast theme toggle that persists across restarts and fully restores the normal theme when switched off.
- Cache hardening: `library_cache.json` carries a schema version; version mismatches or corruption fall back to an empty library with a warning log and a user-visible notice. Settings gains a "Clear Library Cache" action behind a confirmation dialog; the cache rebuilds on the next scan.
- Linux folder picker improvements: validated text input with autocomplete, clear errors for nonexistent paths and non-directories, and in-settings platform notes documenting the platform's limitations.
- Control bar additions: a mute toggle that restores the exact previous volume when unmuted, and a Stop control available in advanced mode.
- Now Playing view completion: large cover art (300 px), the full metadata set (album artist, year, genre, track number), clickable up-next rows that trigger Play Next, an in-view seek slider with clamping, and a graceful nothing-playing state.

### Changed

- System tray behavior completed on Windows/macOS: closing the window minimizes to the tray with playback continuing; the tooltip shows "Artist - Title"; left-click toggles the window; the right-click menu offers Play/Pause, Next Track, Previous Track, Show Window, and Quit (stops playback, then exits). Linux remains window-only by design.
- Cover art renders through a single LRU-backed pipeline (50-texture cap) with fixed-size clamped rendering — 200 px in the library detail pane, 300 px in the Now Playing view — and a placeholder glyph when no art exists.
- Engineering infrastructure: CI runs on GitHub Actions (ubuntu-latest and windows-latest) executing rustfmt, clippy with warnings denied, and the test suite; the suite grew to ~165 integration tests across domain, app, infra, ui, and integration suites.

### Fixed

- A corrupt or version-mismatched library cache no longer fails silently: riff logs a warning, shows an explanatory notice, and offers Settings → Clear Library Cache for a clean rebuild.

## [0.1.0]

### Added

- Initial release: multi-format decoding (MP3, AAC, Opus, FLAC, OGG Vorbis, WAV) with cpal output, library scanning/search/persistence with folder watching, cover art resolution, a dual-view library explorer, player control bar with click-to-seek, a basic Now Playing view, system tray on macOS/Windows, and cross-platform builds for Linux, Windows, and macOS.
