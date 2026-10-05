//! The shared context-menu surface (component-layer issue 15).
//!
//! One owner for how a menu row reads — ordinary, destructive, or inert — and
//! one owner for what a click MEANS: the Track and whole-list menus report
//! [`TrackMenuIntent`]s and [`ListMenuIntent`]s and stop there. The host maps an
//! intent onto its Transport, Playlist Store, Application Store, selection, or
//! Inline Tag Editor afterwards, so no effect can happen while a menu is merely
//! being painted.
//!
//! The menus' *content policy* stays with the menus: which rows exist for a
//! Track whose file is gone, and which playlist names to offer, are facts the
//! caller resolves and hands over as props.
//!
//! # One menu, one set of words
//!
//! The Track menu and the whole-list menu open with the same three rows — Play,
//! Play Next, Add to Queue — so queueing a Track and queueing a whole
//! collection are not one action wearing two names, and a listener who has
//! learned one menu has learned both. The two set-of-Tracks menus — the
//! whole-list one on headers and nodes, the collection one on an entity row —
//! then add Shuffle, and both take their rows from one private helper, so the
//! order cannot drift between them.
//!
//! [`ItemState`] stays the three states it has always been. A menu announces a
//! condition through its label and its ink — a destructive row is how "Remove
//! from Playlist" says what it does — and adding a checked state would give the
//! same idea a second convention.
//!
//! # The Favourite row, and why its label flips instead of ticking
//!
//! The Favourite item is the one row whose *state* the listener would most
//! expect to see marked, and it deliberately is not marked. It says what the
//! click will do — "Add to Favorites" or "Remove from Favorites", read from
//! [`FAVORITE_ADD_LABEL`] and [`FAVORITE_REMOVE_LABEL`].
//!
//! A tick would be the second convention for one idea. The menu already says
//! "this row is a choice that will remove something" with ink rather than with a
//! mark, and "this Track IS a Favourite" with a word. A mark beside the word
//! would restate the word in a vocabulary nothing else in the app speaks, and it
//! would have to be a fourth [`ItemState`] to carry — the one thing the enum's
//! shape says it is not for. The label is also the strictly more informative
//! half: it distinguishes *what the click will do* from *what is true now*, and
//! only a listener who cannot read the row's heart needs the former.
//!
//! So the item carries the flag's NEW value rather than the bare fact of a
//! toggle, and the caller — which already knows the row's current state, and
//! already commits this exact flag for the row's heart — does the same one
//! durable write either way.

use eframe::egui;

use super::theme::Palette;
use riff_backend::app::state::TrackSort;
use riff_backend::domain::PlaylistId;

/// The one explanation the "Edit Tags" entry point carries — what saving a
/// track's tags actually touches.
pub const EDIT_TAGS_TOOLTIP: &str = "Edit this track's tags (title, artist, album, and more). \
     Changes are written to the file on Save.";

/// The Favourite action's wording when the Track is NOT a Favourite: the click
/// ADDS one. And [`FAVORITE_REMOVE_LABEL`] is its opposite.
///
/// The two wordings live here, with the rest of the menu's action vocabulary —
/// "Play", "Add to Queue", "Add to Playlist", "Remove from Playlist", "Edit
/// Tags" — because this module is the one that decides how an action is
/// spelled. A Track's menu offers the Favourite as a second path alongside the
/// heart on its row, and the two must not be spelled independently: a listener
/// who Favourites from the heart and then from the menu should not have to
/// learn that the app has two names for one thing. `sidebar` reads these
/// constants for the heart rather than declaring its own, which is also the
/// direction the component layer's dependency rule requires.
///
/// The spelling is the app's long-standing one, "Favorites" rather than the
/// domain glossary's "Favourite". That divergence predates the menu, and it is
/// recorded rather than fixed here: re-spelling either surface would move
/// pixels and re-record goldens for a wording nobody asked about.
pub const FAVORITE_ADD_LABEL: &str = "Add to Favorites";

/// The Favourite action's wording when the Track IS a Favourite: the click
/// REMOVES it. See [`FAVORITE_ADD_LABEL`].
pub const FAVORITE_REMOVE_LABEL: &str = "Remove from Favorites";

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

// --- The track-listing sort control -------------------------------------------------

/// The short label the sort button paints for each mode — the active order,
/// read at a glance, the same vocabulary the popup's rows speak.
fn track_sort_button_label(sort: TrackSort) -> &'static str {
    match sort {
        TrackSort::NumberAsc => "# \u{2191}",
        TrackSort::NumberDesc => "# \u{2193}",
        TrackSort::TitleAsc => "A\u{2013}Z",
        TrackSort::TitleDesc => "Z\u{2013}A",
    }
}

/// The popup row for each mode — what the click selects.
fn track_sort_item_label(sort: TrackSort) -> &'static str {
    match sort {
        TrackSort::NumberAsc => "Track No. \u{2191}",
        TrackSort::NumberDesc => "Track No. \u{2193}",
        TrackSort::TitleAsc => "Title A\u{2013}Z",
        TrackSort::TitleDesc => "Title Z\u{2013}A",
    }
}

/// Every track listing's sort control, in listing order: the four modes a
/// track listing can take, offered by one small ghost button that opens a
/// popup menu on click — the same button shape the entity columns' A–Z / Z–A
/// toggle paints, with a menu instead of a toggle because there are four
/// modes to move between, not two. The button names the order in force; the
/// rows name what a click selects, and the current one is present to be read
/// (`Disabled`, marked "(current)") rather than clicked again. Returns the
/// newly chosen mode, if any — the caller owns the session state.
///
/// A track listing's canonical order differs by surface (track number within
/// an album, playlist order, path order in the flat list), which is why the
/// two canonical rows say "Track No." and not a per-surface fact: the button
/// is one widget, and the session's [`TrackSort`] is what the host resolves
/// the label's promise against.
pub fn track_sort_control(
    ui: &mut egui::Ui,
    palette: &Palette,
    current: TrackSort,
) -> Option<TrackSort> {
    let label = track_sort_button_label(current);
    let button = egui::Button::new(
        egui::RichText::new(label)
            .text_style(egui::TextStyle::Small)
            .color(palette.ink_2),
    )
    .fill(palette.surface_2)
    .corner_radius(super::theme::RADIUS_SM);
    let response = ui
        .add(button)
        .on_hover_text("Change how this list is ordered");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Sort order: {}", track_sort_item_label(current)),
        )
    });

    let mut chosen = None;
    egui::Popup::menu(&response).show(|ui| {
        for sort in [
            TrackSort::NumberAsc,
            TrackSort::NumberDesc,
            TrackSort::TitleAsc,
            TrackSort::TitleDesc,
        ] {
            let active = sort == current;
            if active {
                let marked = concat_current(track_sort_item_label(sort));
                let _ = item(ui, palette, &Item::disabled(&marked));
            } else if item(ui, palette, &Item::new(track_sort_item_label(sort))) {
                chosen = Some(sort);
            }
        }
    });
    chosen
}

/// The active row's static label plus its "(current)" mark. The labels are
/// `'static`, so the marked form has to be composed — one `format!` per
/// open frame, never per row of any list.
fn concat_current(label: &'static str) -> String {
    format!("{label} (current)")
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
    /// Set this Track's Favourite flag to the value carried here.
    ///
    /// The NEW value, not "toggle this": the row's heart already reports the
    /// flag it is about to set, so the two paths to the same durable change
    /// name the same change and the host writes it the same way.
    SetFavorite(bool),
    /// Select this Track and open the Detail Panel's Inline Tag Editor.
    EditTags,
    /// Measure this Track's `ReplayGain` and write the values to its file
    /// tags and the Store. Exactly this Track: the Album aggregate is
    /// untouched.
    MeasureReplayGain,
}

/// One choice in a whole-list context menu, over the list the host attached it
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMenuIntent {
    /// Play the list as one batch: its first Track, then the rest behind it.
    Play,
    /// Insert the whole list next, preserving its order.
    PlayNext,
    /// Add the whole list to the end of the queue.
    AddToQueue,
    /// Engage shuffle, then play the list as one batch. Shuffle is a one-shot:
    /// it turns shuffle on and starts the list, and shuffle STAYS on
    /// afterwards — the long-standing behaviour of the album header's Shuffle,
    /// preserved rather than quietly changed.
    Shuffle,
    /// Measure this Album's `ReplayGain` aggregate and write the album pair
    /// to every Track of the Album. Album rows only.
    MeasureAlbumAggregate,
    /// Measure track `ReplayGain` for every Track of this Album individually
    /// and write exactly those. Album rows only.
    MeasureAlbumTracks,
}

/// The props one Track's menu renders from.
// Each flag is one condition the renderer branches on, and a listener's Track
// has several at once: the file may be gone, the row may or may not be a
// playlist entry, the Favourite flag may be either way, and "Edit Tags" is
// offered on a condition that HAPPENS to match the first of those today but is
// not the same question. Folding them into one state machine would be a
// redesign of what the host can know, and the menu is the last place that
// should decide how many facts it is told.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per condition the renderer branches on, not one state"
)]
#[derive(Debug, Clone, Copy)]
pub struct TrackMenu<'a> {
    /// Whether the file still exists: a Track whose file is gone offers no
    /// playback actions and no tag editor — and no Favourite either, because this
    /// is the ONE gate on the Favourite item and it is the gate the heart is
    /// behind too. The heart is painted into the row of a Track the store still
    /// resolves, and a missing-file playlist entry is drawn as a flagged label
    /// with no heart on it, so the two surfaces agree on which Tracks are
    /// Favouritable without either of them naming the other's condition.
    pub playable: bool,
    /// Whether the "Edit Tags" entry point is offered (same fact as
    /// `playable`, named separately because the menu treats them apart).
    pub editable: bool,
    /// Whether this Track is a Favourite right now — which is what decides
    /// whether the Favourite item says it will add one or remove it. It travels
    /// beside `playable` and `editable` because it is the same kind of fact: a
    /// condition the renderer branches on, which only the host can know.
    ///
    /// It is per-TRACK, and that is load-bearing. A Track menu is attached to
    /// rows that share nothing, and a listing may hold a mix of Favourited and
    /// un-Favourited rows, so every surface that builds a menu for more than one
    /// row at a time has to build them per row — see
    /// [`TrackMenuFactory`](crate::ui::detail::TrackMenuFactory), which is what
    /// a virtualized column hands the widget in place of one finished menu.
    pub favorite: bool,
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
        // The Favourite item, last in the same group as the three playback
        // actions and the add target, and on exactly the condition they are.
        // The label says which way the click goes, so there is no tick to read,
        // and the intent carries the value that label promises rather than the
        // bare fact of a toggle.
        let favorite_label = if menu.favorite {
            FAVORITE_REMOVE_LABEL
        } else {
            FAVORITE_ADD_LABEL
        };
        if item(ui, palette, &Item::new(favorite_label)) {
            intents.push(TrackMenuIntent::SetFavorite(!menu.favorite));
        }
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
    if menu.editable && item(ui, palette, &Item::new("Measure ReplayGain")) {
        intents.push(TrackMenuIntent::MeasureReplayGain);
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

/// A Track row's menu report: the Track the right-click landed on, and what was
/// chosen from it in click order.
///
/// It lives here, beside [`TrackMenu`] and [`TrackMenuIntent`], because it is a
/// report about a *Track menu* and nothing else — which is exactly why it is
/// NOT a variant of the detail column's own action type. That applier answers
/// selection, playback and a Favourite; answering a Track menu needs the
/// Playlist Store and the Inline Tag Editor, which are the per-app
/// [`TrackMenuHost`](crate::ui::app::TrackMenuHost)'s to hold. A separate
/// report type makes that a fact the compiler enforces instead of a fact a
/// comment asserts and a no-op arm has to cover.
///
/// The counterpart of [`crate::ui::browser::BrowserAction::ContextMenu`], and it
/// arrives on the frame the menu OPENS — with `intents` empty, because opening
/// is what moves the selection and choosing is a separate event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackMenuReport {
    /// The row's [`TrackId`](riff_backend::domain::TrackId), as its key string:
    /// the identity the selection becomes and every intent acts on.
    pub key: String,
    /// Whatever was chosen from the menu, in click order. Empty on the frame it
    /// opened, and a right-click that opened a menu and was then dismissed
    /// chose nothing and has still selected.
    pub intents: Vec<TrackMenuIntent>,
}

/// The rows every set-of-Tracks menu offers, in the one order they are offered
/// in everywhere, appending one [`ListMenuIntent`] per activation.
///
/// This is the ONE place those four rows are written. Both public renderers
/// below call it, which is the point: the whole-list menu and the collection
/// menu are distinct because their DISPATCH is bound to different surfaces and
/// carry different effects, not because they happen to name the same actions
/// differently. Two hand-written copies of four `item()` calls would be free to
/// drift, and a listener who learned one menu would find the other lying.
fn set_of_tracks_items(ui: &mut egui::Ui, palette: &Palette, intents: &mut Vec<ListMenuIntent>) {
    if item(ui, palette, &Item::new("Play")) {
        intents.push(ListMenuIntent::Play);
    }
    if item(ui, palette, &Item::new("Play Next")) {
        intents.push(ListMenuIntent::PlayNext);
    }
    if item(ui, palette, &Item::new("Add to Queue")) {
        intents.push(ListMenuIntent::AddToQueue);
    }
    if item(ui, palette, &Item::new("Shuffle")) {
        intents.push(ListMenuIntent::Shuffle);
    }
}

/// Render a whole list's context menu — playlist headers, smart playlist
/// headers, and folder nodes — appending one [`ListMenuIntent`] per
/// activation.
///
/// The four items are the shared set-of-Tracks rows
/// ([`set_of_tracks_items`]), and this renderer is a separate entry point
/// because its dispatch is bound to the three header/node call sites: it acts
/// on a list the caller already resolved and carries no selection, so a
/// right-click on a header is a pure peek that neither selects, opens, nor
/// navigates.
pub fn list_menu(ui: &mut egui::Ui, palette: &Palette, intents: &mut Vec<ListMenuIntent>) {
    set_of_tracks_items(ui, palette, intents);
}

/// Render an Album row's context menu, appending one [`ListMenuIntent`] per
/// activation.
///
/// The shared set-of-Tracks rows, then the two `ReplayGain` items only an
/// Album earns: an Album is a measurable unit — one aggregate over its Track
/// set, or every member's own pair — while an Artist or a Genre is only a
/// set of Tracks with no measurement of its own. The two items ride the same
/// report the play items do; what the host does with them is the host's
/// answer, and a Track command's independence from any Settings state lives
/// there.
pub fn album_menu(ui: &mut egui::Ui, palette: &Palette, intents: &mut Vec<ListMenuIntent>) {
    set_of_tracks_items(ui, palette, intents);
    if item(ui, palette, &Item::new("Measure Album ReplayGain")) {
        intents.push(ListMenuIntent::MeasureAlbumAggregate);
    }
    if item(ui, palette, &Item::new("Measure Track ReplayGain")) {
        intents.push(ListMenuIntent::MeasureAlbumTracks);
    }
}

/// Render a collection's context menu — the menu on an Album, Artist, or Genre
/// row — appending one [`ListMenuIntent`] per activation.
///
/// It offers the same four rows as [`list_menu`], in the same order, and adds
/// none of the Track-only items: an entity has no tags of its own to edit, and
/// a collection is not a single Track to add to or take out of a Playlist. It
/// takes no props, because an entity always denotes a set of Tracks and so has
/// no reduced state to be told about — there is no counterpart here to the
/// Track menu's `playable` and `editable` flags.
///
/// The distinctness is in what the HOST does with the report, not in the
/// report: opening this menu also selects the row, so the menu just opened and
/// the Detail Panel describe the same thing. The renderer cannot select
/// anything — it only names the choice, and the host applies it.
pub fn collection_menu(ui: &mut egui::Ui, palette: &Palette, intents: &mut Vec<ListMenuIntent>) {
    set_of_tracks_items(ui, palette, intents);
}
