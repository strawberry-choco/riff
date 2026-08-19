use crate::domain::track::TrackId;
use std::time::SystemTime;

/// Unique identifier for a user playlist.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PlaylistId(pub String);

impl PlaylistId {
    /// Generate a new id from a slug of `name` plus a millisecond timestamp.
    /// Callers that need global uniqueness (playlist creation) dedupe against
    /// existing ids — see `playlist_manager::create_playlist`.
    pub fn new(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        PlaylistId(format!("{}-{stamp}", slugify(name)))
    }
}

/// Lowercase alphanumerics only; everything else collapses to `-`. Falls back
/// to `playlist` when the name has no alphanumeric characters at all.
fn slugify(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "playlist".to_string()
    } else {
        trimmed
    }
}

/// A user-managed playlist: a named, ordered list of track references.
///
/// Persisted to its own `playlists.json` — deliberately NOT inside the
/// library cache, which can be cleared and rebuilt (Task 3.2). User
/// playlists must survive that. Entries may become invalid when files are
/// moved or deleted; validity is checked on use, never assumed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Playlist {
    pub id: PlaylistId,
    pub name: String,
    /// Ordered track references. `Vec` preserves user ordering; exact
    /// duplicates are prevented at insertion time.
    pub tracks: Vec<TrackId>,
    /// When the playlist was created (`None` only for legacy data).
    #[serde(default)]
    pub created: Option<SystemTime>,
}

impl Playlist {
    /// Create an empty playlist with the given id and name, stamped now.
    pub fn new(id: PlaylistId, name: String) -> Self {
        Self {
            id,
            name,
            tracks: Vec::new(),
            created: Some(SystemTime::now()),
        }
    }
}
