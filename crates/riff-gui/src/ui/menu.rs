//! The shared context-menu surface (component-layer issue 15).
//!
//! One owner for how a menu row reads — ordinary, destructive, or inert — and
//! one owner for what a click MEANS: the Track and whole-list menus report
//! [`TrackMenuIntent`]s and [`ListMenuIntent`]s and stop there. The host maps an
//! intent onto its Transport, Playlist Store, selection, or Inline Tag Editor
//! afterwards, so no effect can happen while a menu is merely being painted.
//!
//! The menus' *content policy* stays with the menus: which rows exist for a
//! Track whose file is gone, and which playlist names to offer, are facts the
//! caller resolves and hands over as props.

use eframe::egui;

use super::theme::Palette;
use riff_backend::domain::PlaylistId;

/// The one explanation the "Edit Tags" entry point carries — what saving a
/// track's tags actually touches.
pub const EDIT_TAGS_TOOLTIP: &str = "Edit this track's tags (title, artist, album, and more). \
     Changes are written to the file on Save.";

// --- The shared item conventions --------------------------------------------------

/// How a menu row presents and behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemState {
    /// An ordinary actionable row.
    Normal,
    /// A row that removes something: the palette's error ink, so the dangerous
    /// choice in a menu is recognisable in the same way everywhere.
    Destructive,
    /// A row that is present to be read but answers no click — the inert line
    /// that explains why there is nothing to choose.
    Disabled,
}

/// One menu row's props.
pub struct Item<'a> {
    pub label: &'a str,
    pub state: ItemState,
    pub tooltip: Option<&'a str>,
}

impl<'a> Item<'a> {
    /// An ordinary actionable row.
    #[must_use]
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            state: ItemState::Normal,
            tooltip: None,
        }
    }

    /// A row that removes something.
    #[must_use]
    pub fn destructive(label: &'a str) -> Self {
        Self {
            label,
            state: ItemState::Destructive,
            tooltip: None,
        }
    }

    /// The inert line that explains an empty group.
    #[must_use]
    pub fn disabled(label: &'a str) -> Self {
        Self {
            label,
            state: ItemState::Disabled,
            tooltip: None,
        }
    }
}

/// Render one menu row and report whether the listener activated it — by
/// pointer or keyboard, since every row is a real focusable button. An
/// activated row closes the menu; a disabled one answers nothing at all.
pub fn item(ui: &mut egui::Ui, palette: &Palette, row: &Item<'_>) -> bool {
    let mut text = egui::RichText::new(row.label);
    if row.state == ItemState::Destructive {
        text = text.color(palette.error);
    }
    let enabled = row.state != ItemState::Disabled;
    let response = ui.add_enabled(enabled, egui::Button::new(text));
    if let Some(tooltip) = row.tooltip {
        response.clone().on_hover_text(tooltip);
    }
    if enabled && response.clicked() {
        ui.close();
        return true;
    }
    false
}

/// A group heading inside a menu: the muted label that names the rows below it.
pub fn section(ui: &mut egui::Ui, palette: &Palette, heading: &str) {
    ui.label(egui::RichText::new(heading).color(palette.ink_3).small());
}

/// A nested group — the menu's disclosure affordance.
pub fn submenu(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.menu_button(label, add);
}

// --- The typed intents ------------------------------------------------------------

/// One choice in a Track's context menu. It names what was chosen, not what to
/// do about it: the Track, the playlist to remove from, and the ports to act
/// through all belong to the host that attached the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackMenuIntent {
    /// Play this Track now.
    Play,
    /// Insert this Track next in the queue.
    PlayNext,
    /// Append this Track to the end of the queue.
    AddToQueue,
    /// Add this Track to the named Playlist.
    AddToPlaylist(PlaylistId),
    /// Take this Track out of the Playlist the menu was opened from.
    RemoveFromPlaylist,
    /// Select this Track and open the Detail Panel's Inline Tag Editor.
    EditTags,
}

/// One choice in a whole-list context menu, over the list the host attached it
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMenuIntent {
    /// Play the list: its first Track, then the rest behind it.
    Play,
    /// Queue the whole list next, preserving its order.
    PlayNext,
    /// Append the whole list to the end of the queue.
    AppendToQueue,
}

/// The props one Track's menu renders from.
#[derive(Debug, Clone, Copy)]
pub struct TrackMenu<'a> {
    /// Whether the file still exists: a Track whose file is gone offers no
    /// playback actions and no tag editor.
    pub playable: bool,
    /// Whether the "Edit Tags" entry point is offered (same fact as
    /// `playable`, named separately because the menu treats them apart).
    pub editable: bool,
    /// The "Add to Playlist" targets, already resolved to `(id, name)`.
    pub playlists: &'a [(PlaylistId, String)],
    /// Whether this row belongs to a Playlist, which offers the removal.
    pub remove_from_playlist: bool,
}

/// Render a Track's context menu, appending one [`TrackMenuIntent`] per
/// activation.
pub fn track_menu(
    ui: &mut egui::Ui,
    palette: &Palette,
    menu: &TrackMenu<'_>,
    intents: &mut Vec<TrackMenuIntent>,
) {
    if menu.playable {
        if item(ui, palette, &Item::new("Play")) {
            intents.push(TrackMenuIntent::Play);
        }
        if item(ui, palette, &Item::new("Play Next")) {
            intents.push(TrackMenuIntent::PlayNext);
        }
        if item(ui, palette, &Item::new("Add to Queue")) {
            intents.push(TrackMenuIntent::AddToQueue);
        }
        submenu(ui, "Add to Playlist", |ui| {
            playlist_items(ui, palette, menu.playlists, intents);
        });
    }
    if menu.remove_from_playlist && item(ui, palette, &Item::destructive("Remove from Playlist")) {
        intents.push(TrackMenuIntent::RemoveFromPlaylist);
    }
    if menu.editable
        && item(
            ui,
            palette,
            &Item {
                label: "Edit Tags",
                state: ItemState::Normal,
                tooltip: Some(EDIT_TAGS_TOOLTIP),
            },
        )
    {
        intents.push(TrackMenuIntent::EditTags);
    }
}

/// The "Add to Playlist" group: one row per playlist, or the inert line that
/// says there are none to pick.
pub fn playlist_items(
    ui: &mut egui::Ui,
    palette: &Palette,
    playlists: &[(PlaylistId, String)],
    intents: &mut Vec<TrackMenuIntent>,
) {
    if playlists.is_empty() {
        let _ = item(ui, palette, &Item::disabled("No playlists yet"));
        return;
    }
    section(ui, palette, "Playlists");
    for (id, name) in playlists {
        if item(ui, palette, &Item::new(name)) {
            intents.push(TrackMenuIntent::AddToPlaylist(id.clone()));
        }
    }
}

/// Render a whole list's context menu, appending one [`ListMenuIntent`] per
/// activation.
pub fn list_menu(ui: &mut egui::Ui, palette: &Palette, intents: &mut Vec<ListMenuIntent>) {
    if item(ui, palette, &Item::new("Play")) {
        intents.push(ListMenuIntent::Play);
    }
    if item(ui, palette, &Item::new("Play Next")) {
        intents.push(ListMenuIntent::PlayNext);
    }
    if item(ui, palette, &Item::new("Append to Queue")) {
        intents.push(ListMenuIntent::AppendToQueue);
    }
}
