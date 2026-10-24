//! The application layer's half of the sidebar name rule: a candidate name
//! against the six Built-in Smart Playlists' display names.
//!
//! The store's mutation path holds the cross-table half of the rule — no name
//! may be held by two sidebar rows, compared trimmed and Rust-lowercased
//! (`SqliteStore::reject_taken_sidebar_name`). It deliberately cannot hold this
//! half: the Built-in Smart Playlists exist only as code ([`SmartPlaylistKind`]
//! and its `display_name`), never as rows in either table, so no query can see
//! them and no constraint can name them. Without this check a listener could
//! create a Playlist or a Custom Smart Playlist called "Most Played" beside the
//! built-in row of the same name, and the sidebar would show two rows reading
//! the same thing (user story 8).
//!
//! Every gesture that names a sidebar row asks this one function — the
//! editor's Save asks it on its own refusal ladder, and each of the three
//! sidebar writes (the "New Playlist" create, the playlist rename, and the
//! Custom Smart Playlist rename the sidebar menu opens) asks it inside its
//! own commit helper, so any future caller of those paths inherits the
//! check — so the check and its wording exist once. The
//! one name no gesture picks is Duplicate's: the store mints `Copy of …` in
//! its own transaction, and no such name can be a Built-in display name. The
//! comparison trims and folds exactly the way the store's rule does (Rust's
//! full-Unicode `to_lowercase`, never SQLite's ASCII-only `lower()`): a name
//! is identity, and the two halves of one rule must not fold differently.
//!
//! [`SmartPlaylistKind`]: crate::domain::SmartPlaylistKind

use crate::domain::SmartPlaylistKind;

/// Why `name` may not be given to a sidebar row because one of the six
/// Built-in Smart Playlists already answers to it, or `None` when none does.
///
/// The name is trimmed and folded the way the store's cross-table rule folds
/// it, so `"  most played "` collides with the built-in `Most Played` exactly
/// as `"most played"` does. The refusal quotes the built-in's display name —
/// the spelling the sidebar actually shows — rather than the candidate, so
/// the reason reads the same whatever case the listener typed.
#[must_use]
pub fn builtin_smart_list_name_conflict(name: &str) -> Option<String> {
    let wanted = name.trim().to_lowercase();
    SmartPlaylistKind::ALL
        .into_iter()
        .find(|kind| kind.display_name().to_lowercase() == wanted)
        .map(|kind| {
            format!(
                "a Built-in Smart Playlist is already named {:?}",
                kind.display_name()
            )
        })
}
