use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Unique identifier for a track in the library.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

impl TrackId {
    pub fn from_path(path: &Path) -> Self {
        TrackId(path.to_string_lossy().to_string())
    }
}

/// A track in the music library.
#[derive(Debug, Clone)]
pub struct Track {
    pub id: TrackId,
    pub file_path: PathBuf,
    pub metadata: TrackMetadata,
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    /// How many times this track has finished playing. Persisted in the
    /// Application Store's tracks table; incremented when playback reaches
    /// `TrackEnded`.
    pub play_count: u32,
    /// When this track last finished playing (`None` = never played).
    pub last_played: Option<SystemTime>,
    /// When this track was first added to the library. Set once at scan time
    /// and never refreshed — it drives the "Recently Added" smart playlist.
    /// Deliberately NOT the filesystem mtime, which changes on tag edits.
    pub date_added: Option<SystemTime>,
    /// The track's precomputed lowercase search blob — the same value the
    /// Application Store derives into its `search_text` column at write
    /// time (`title artist album album_artist`, lowercased). Surfaced on
    /// every read so view-side filtering matches against the stored column
    /// without re-formatting per frame. Empty for freshly scanned (not yet
    /// committed) tracks; the store recomputes it on write.
    pub search_text: String,
}

/// The four auto-generated, read-only smart playlists (REQ-ML-009). Each is
/// computed on demand from local library metadata and play history — purely
/// offline, no network access, and never persisted as its own entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmartPlaylistKind {
    RecentlyAdded,
    MostPlayed,
    NeverPlayed,
    LostGems,
}

impl SmartPlaylistKind {
    /// Every smart playlist in stable display order, for the UI to enumerate.
    pub const ALL: [SmartPlaylistKind; 4] = [
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::NeverPlayed,
        SmartPlaylistKind::LostGems,
    ];

    /// Human-readable name shown in the library explorer.
    pub fn display_name(self) -> &'static str {
        match self {
            SmartPlaylistKind::RecentlyAdded => "Recently Added",
            SmartPlaylistKind::MostPlayed => "Most Played",
            SmartPlaylistKind::NeverPlayed => "Never Played",
            SmartPlaylistKind::LostGems => "Lost Gems",
        }
    }
}

/// Metadata extracted from an audio file.
///
/// Note: derives `PartialEq` but not `Eq` — the `ReplayGain` fields are
/// `f32`, which has no total equality.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub composer: Option<String>,
    pub comment: Option<String>,
    /// `ReplayGain` track gain in dB (e.g. `-6.54`), from the
    /// `REPLAYGAIN_TRACK_GAIN` tag. `None` when the file carries no tag.
    pub replaygain_track_gain: Option<f32>,
    /// `ReplayGain` track peak as a linear ratio (0..1), from the
    /// `REPLAYGAIN_TRACK_PEAK` tag. Used to cap applied gain so amplified
    /// samples cannot clip.
    pub replaygain_track_peak: Option<f32>,
}

impl TrackMetadata {
    pub fn display_title(&self, fallback_path: &Path) -> String {
        self.title.clone().unwrap_or_else(|| {
            fallback_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .replace('_', " ")
        })
    }

    pub fn display_artist(&self) -> String {
        self.artist
            .clone()
            .unwrap_or_else(|| "Unknown Artist".to_string())
    }

    pub fn display_album(&self) -> Cow<'_, str> {
        match &self.album {
            Some(album) => Cow::Borrowed(album),
            None => Cow::Borrowed("Unknown Album"),
        }
    }

    pub fn display_album_artist(&self) -> Cow<'_, str> {
        match &self.album_artist {
            Some(album_artist) => Cow::Borrowed(album_artist),
            None => Cow::Owned(self.display_artist()),
        }
    }

    /// The lowercased `title artist album album_artist` blob the store
    /// derives into its `search_text` column. Built with an exact-capacity
    /// buffer; the separator layout (three spaces, always present) matches
    /// the former `format!` byte-for-byte so stored and recomputed values
    /// agree.
    pub fn search_text(&self) -> String {
        let title = self.title.as_deref().unwrap_or("");
        let artist = self.artist.as_deref().unwrap_or("");
        let album = self.album.as_deref().unwrap_or("");
        let album_artist = self.album_artist.as_deref().unwrap_or("");
        let mut text = String::with_capacity(
            title.len() + artist.len() + album.len() + album_artist.len() + 3,
        );
        text.push_str(title);
        text.push(' ');
        text.push_str(artist);
        text.push(' ');
        text.push_str(album);
        text.push(' ');
        text.push_str(album_artist);
        text.to_lowercase()
    }
}

/// An album in the music library (aggregate of tracks).
#[derive(Debug, Clone)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub tracks: Vec<TrackId>,
    pub year: Option<u32>,
    pub genre: Option<String>,
}

/// An artist in the music library.
#[derive(Debug, Clone)]
pub struct Artist {
    pub name: String,
    pub albums: Vec<String>,
}

/// Cover art source for a track. Embedded art shares its bytes behind an
/// `Arc`, so handing the source between threads (reader → cover service →
/// loader) bumps a refcount instead of copying megapixel payloads.
#[derive(Debug, Clone)]
pub enum CoverSource {
    Embedded(Arc<[u8]>),
    Filesystem(PathBuf),
    None,
}
