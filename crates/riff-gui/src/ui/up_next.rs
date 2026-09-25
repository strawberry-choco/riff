//! The neutral Up Next presentation.
//!
//! One owner for the Up Next entry model, the label builder, and the row
//! renderer shared by the Now Playing stage and the player bar's queue sheet.
//! Both surfaces consume this module, so neither supplies the other's row data
//! — which is what removes the `playerbar` ↔ `now_playing` module cycle.
//!
//! It stays presentation-only: the queue-to-window ordering and the skip of
//! departed files live in the read model (`SessionViews::playback_up_next`),
//! and each surface keeps its own row limit and maps the returned row click to
//! its own typed Play Next action.

use eframe::egui;
use riff_backend::domain::{Track, TrackId};

use super::icons::IconCache;
use super::sidebar::{self, TreeRow};
use super::theme::Palette;

/// One clickable Up Next row: the queued track plus its display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpNextEntry {
    /// The queued track; rides back on the surface's Play Next intent.
    pub id: TrackId,
    /// Preformatted row label, `"Artist - Title"`.
    pub label: String,
}

/// Build the Up Next rows from the playback projection's resolved window:
/// the tracks after the current one, in the QUEUE's own order (shuffle
/// included), capped at `limit`. The queue-to-window mapping and the skip of
/// entries whose files have left the library live in
/// [`crate::app::views::SessionViews`]; this is the pure label formatting over
/// its result.
#[must_use]
pub fn up_next_entries(up_next: &[Track], limit: usize) -> Vec<UpNextEntry> {
    up_next
        .iter()
        .take(limit)
        .map(|t| UpNextEntry {
            id: t.id.clone(),
            label: format!(
                "{} - {}",
                t.metadata.display_artist(),
                t.metadata.display_title(&t.file_path)
            ),
        })
        .collect()
}

/// Render one Up Next row through the shared neutral tree row and return its
/// response. Both the Now Playing stage and the queue sheet paint the same
/// Track row this way, then map a click to their own Play Next action.
pub fn up_next_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    entry: &UpNextEntry,
) -> egui::Response {
    sidebar::tree_row(
        ui,
        cache,
        palette,
        TreeRow {
            indent_level: 0,
            icon: None,
            cover: None,
            label: &entry.label,
            count: None,
            meta: None,
            favorite: None,
            selected: false,
            now_playing: false,
            playing: false,
            art_slot: false,
        },
    )
    .response
}
