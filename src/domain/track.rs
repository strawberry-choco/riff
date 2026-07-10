use std::path::PathBuf;
use std::time::Duration;

/// Unique identifier for a track in the library.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TrackId(pub String);

impl TrackId {
    pub fn from_path(path: &PathBuf) -> Self {
        TrackId(path.to_string_lossy().to_string())
    }
}

/// A track in the music library.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub file_path: PathBuf,
    pub metadata: TrackMetadata,
    pub duration: Option<Duration>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
}

/// Metadata extracted from an audio file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
}

impl TrackMetadata {
    pub fn display_title(&self, fallback_path: &PathBuf) -> String {
        self.title.clone()
            .unwrap_or_else(|| {
                fallback_path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .replace('_', " ")
            })
    }

    pub fn display_artist(&self) -> String {
        self.artist.clone().unwrap_or_else(|| "Unknown Artist".to_string())
    }

    pub fn display_album(&self) -> String {
        self.album.clone().unwrap_or_else(|| "Unknown Album".to_string())
    }

    pub fn display_album_artist(&self) -> String {
        self.album_artist.clone().unwrap_or_else(|| self.display_artist())
    }

    pub fn search_text(&self) -> String {
        format!(
            "{} {} {} {}",
            self.title.as_deref().unwrap_or(""),
            self.artist.as_deref().unwrap_or(""),
            self.album.as_deref().unwrap_or(""),
            self.album_artist.as_deref().unwrap_or(""),
        ).to_lowercase()
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

/// Cover art source for a track.
#[derive(Debug, Clone)]
pub enum CoverSource {
    Embedded(Vec<u8>),
    Filesystem(PathBuf),
    None,
}
