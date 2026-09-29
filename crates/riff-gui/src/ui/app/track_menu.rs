//! The per-app **Track-menu host**: the one place a right-click on a Track
//! is turned into an effect.
//!
//! One user gesture — a right-click on a Track row — is answered here, and
//! only here. Before this module the gesture was answered through an
//! eight-field effects bag that each surface assembled by hand: the flat All
//! Tracks list, the search results, a smart playlist, a folder node and a
//! user playlist's entries all went through one attach point, and the Tracks
//! Column assembled a second, differently-populated copy of the same bag to
//! intercept a report its own applier could not answer. A fifth Track-row
//! surface meant writing those eight fields again, and the test suite carried
//! a field-for-field **mirror** of them — so a production site that quietly
//! dropped a handle left the mirror complete and the test green, proving a
//! host the app did not have.
//!
//! The host replaces that with a small interface and a small subject:
//!
//! * [`TrackMenuSubject`] is the per-ROW data — which Track the right-click
//!   landed on, and the one fact about where that row sits. It names no
//!   handle, and its fields are private, so a surface picks one of two named
//!   constructors instead of enumerating anything.
//! * [`TrackMenuHost`] owns the shared handles and answers exactly two
//!   questions: [`TrackMenuHost::right_clicked`] ("a right-click happened on
//!   this Track") and [`TrackMenuHost::item_chosen`] ("this item was chosen").
//!
//! # Why per-app, and why not per-row
//!
//! **Not per-row.** A per-row host would be the bag again, spelled with a
//! constructor: every surface would still have to name what the host needs, and
//! a surface that named it wrong would fail the same way and go unnoticed the
//! same way. The point of this change is that a new Track-row surface supplies
//! *a Track and a row's context* and nothing else, so the set of handles is
//! declared in one place and cannot be assembled differently at another.
//!
//! **Per-app, by construction.** There is one host per app because
//! [`TrackMenuHost::new`] — the constructor `RiffApp::track_menu` calls — is
//! the only place the handle set is written down, and the test suite calls
//! that same constructor. A handle added to or removed from the host changes
//! one signature and breaks the app and its tests together, at compile time.
//!
//! **Borrowed per gesture, not stored on the app.** The host borrows the
//! handles it owns rather than holding them, and that is deliberate. Storing
//! it would mean moving four handles (`transport`, `playlist_store`,
//! `library_mutations`, `tag_editor`) out of `RiffApp` and re-reaching them at
//! every other site in a 4,000-line file — the playerbar, Settings, the
//! collection menus — to save a borrow that costs nothing. The scope that
//! matters is where the handles are *named*, and that is one function.

use crate::ui::menu::{TrackMenu, TrackMenuIntent};
use crate::ui::theme::Palette;
use riff_backend::app::Transport;
use riff_backend::app::store::{LibraryMutationStore, PlaylistStore};
use riff_backend::domain::{PlaylistId, Track, TrackId};

use super::InlineTagEditor;
use super::tag_rows;

/// Which Track a right-click landed on, and the one fact about where that row
/// sits that its menu needs.
///
/// This is per-ROW data and it names no handle: the two constructors are the
/// only way to build one, and they say which of the two shapes the row is.
#[derive(Debug, Clone, Copy)]
pub struct TrackMenuSubject<'a> {
    /// The Track this right-click is about. It is the row's own identity, and
    /// it is what the selection becomes.
    track_id: &'a TrackId,
    /// The Track itself, when the listing resolved one. `None` for a row
    /// whose file is gone — a user playlist's missing entry — and that is what
    /// reduces the menu: no playback items, no "Edit Tags", no Favourite,
    /// because there is no Track to act on.
    track: Option<&'a Track>,
    /// When the row belongs to that Playlist, the menu offers the removal and
    /// removes from this one. `None` everywhere else, which is why the
    /// Tracks Column — where a Track is in no Playlist — needs no more.
    remove_from_playlist: Option<&'a PlaylistId>,
}

impl<'a> TrackMenuSubject<'a> {
    /// A row that resolved its Track: every listing the Library owns, and
    /// every entry of a user playlist whose file is still there.
    ///
    /// `remove_from_playlist` is `Some` only for a row that is a user
    /// playlist's entry; a Track in the Tracks Column, the flat list, a smart
    /// playlist or a folder node is in no Playlist and passes `None`.
    #[must_use]
    pub fn resolved(track: &'a Track, remove_from_playlist: Option<&'a PlaylistId>) -> Self {
        Self {
            track_id: &track.id,
            track: Some(track),
            remove_from_playlist,
        }
    }

    /// A row whose file is gone: the Track cannot be resolved, so the menu
    /// offers only what needs no Track — no playback, no "Edit Tags", no
    /// Favourite. The row's identity still travels, so opening the menu still
    /// selects it, which is what keeps a right-click on a dead entry consistent
    /// with every other Track row.
    ///
    /// `remove_from_playlist` is still the row's own, and that is deliberate
    /// rather than an oversight: a missing entry can still be REMOVED from its
    /// playlist, because that is a fact about the playlist row and not about
    /// the file behind it. The Tracks Column passes `None` because a Track
    /// there is in no Playlist; a user playlist's dead entry passes
    /// `Some(playlist)`.
    #[must_use]
    pub fn unresolved(track_id: &'a TrackId, remove_from_playlist: Option<&'a PlaylistId>) -> Self {
        Self {
            track_id,
            track: None,
            remove_from_playlist,
        }
    }

    /// The Track this right-click is about.
    #[must_use]
    pub fn track_id(self) -> &'a TrackId {
        self.track_id
    }

    /// The resolved Track, when the row has one.
    #[must_use]
    pub fn track(self) -> Option<&'a Track> {
        self.track
    }

    /// The Playlist a "Remove from Playlist" would remove from.
    #[must_use]
    pub fn remove_from_playlist(self) -> Option<&'a PlaylistId> {
        self.remove_from_playlist
    }

    /// The menu to paint for this row.
    ///
    /// The props are read off the row's own Track, which is what lets one
    /// answer per row: the Favourite item's label flips on `track.favorite`,
    /// and a listing may hold a mix of Favourited and un-Favourited rows in
    /// the same frame.
    fn menu(self, playlists: &[(PlaylistId, String)]) -> TrackMenu<'_> {
        TrackMenu {
            playable: self.track.is_some(),
            editable: self.track.is_some(),
            favorite: self.track.is_some_and(|track| track.favorite),
            playlists,
            remove_from_playlist: self.remove_from_playlist.is_some(),
        }
    }
}

/// The per-app Track-menu host: the shared handles a right-click on a Track
/// answers through, and nothing else.
///
/// See the module docs for why it is per-app, why it is borrowed rather than
/// stored, and why a per-row host was rejected.
pub struct TrackMenuHost<'a> {
    /// The Playback Queue's command port: the playback intents go here.
    transport: &'a dyn Transport,
    /// The Application Store's playlists section: the playlist intents commit
    /// through it as one immediate durable transaction.
    playlist_store: &'a mut dyn PlaylistStore,
    /// The Application Store's library section, and the Favourite flag's only
    /// durable home. The Favourite item writes through the SAME port method the
    /// row's heart does (`set_track_favorite`), so the menu is a second door to
    /// one change rather than a second way of storing it.
    library_mutations: &'a mut dyn LibraryMutationStore,
    /// The Inline Tag Editor: "Edit Tags" opens the per-selection draft here.
    tag_editor: &'a mut InlineTagEditor,
    /// The selection slot. "Edit Tags" selects the Track before the editor
    /// opens, and a right-click selects it on the way in, so the Detail Panel
    /// and the menu always describe the same Track.
    selected_track: &'a mut Option<TrackId>,
}

impl<'a> TrackMenuHost<'a> {
    /// Take the handles one Track-menu answer needs. This is the app's single
    /// declaration of that set — `RiffApp::track_menu` calls it in production
    /// and the test suite calls it too, so there is no second assembly for the
    /// two to drift apart in.
    pub fn new(
        transport: &'a dyn Transport,
        playlist_store: &'a mut dyn PlaylistStore,
        library_mutations: &'a mut dyn LibraryMutationStore,
        tag_editor: &'a mut InlineTagEditor,
        selected_track: &'a mut Option<TrackId>,
    ) -> Self {
        Self {
            transport,
            playlist_store,
            library_mutations,
            tag_editor,
            selected_track,
        }
    }

    /// **"A right-click happened on this Track."** That is the whole effect,
    /// and it is the whole effect because a right-click is not a left-click:
    /// it starts no playback, writes nothing, and opens nothing. The Track
    /// becomes the selection so the menu and the Detail Panel describe the
    /// same Track, and a menu that is opened and then dismissed has still
    /// selected.
    ///
    /// Reported separately from [`Self::item_chosen`] because OPENING and
    /// CHOOSING are different events. A surface that only painted its row asks
    /// for neither, and a listener who opens a menu and closes it gets this
    /// and not the other.
    pub fn right_clicked(&mut self, subject: TrackMenuSubject<'_>) {
        *self.selected_track = Some(subject.track_id.clone());
    }

    /// **"This item was chosen."** Apply one emitted intent, and only that:
    /// a Track menu that is painted reaches nothing, and each item moves its
    /// own single fact — playback to the Transport, a playlist entry to the
    /// Playlist Store, a Favourite to the library mutation port, and "Edit
    /// Tags" to the selection and the Inline Tag Editor.
    pub fn item_chosen(&mut self, subject: TrackMenuSubject<'_>, intent: TrackMenuIntent) {
        let track_id = subject.track_id.clone();
        match intent {
            TrackMenuIntent::Play => self.transport.play(track_id),
            TrackMenuIntent::PlayNext => self.transport.play_next(track_id),
            TrackMenuIntent::AddToQueue => self.transport.add_to_queue(track_id),
            TrackMenuIntent::AddToPlaylist(playlist) => {
                // One immediate durable transaction; the committed mutation
                // bumps the playlist generation, so the seam's next read
                // reflects it with zero caller action (ADR 0002).
                if let Err(e) = self.playlist_store.add_playlist_entry(&playlist, &track_id) {
                    tracing::warn!("Failed to add playlist entry: {e}");
                }
            }
            TrackMenuIntent::RemoveFromPlaylist => {
                let Some(playlist) = subject.remove_from_playlist() else {
                    return;
                };
                if let Err(e) = self
                    .playlist_store
                    .remove_playlist_entries(playlist, &track_id)
                {
                    tracing::warn!("Failed to remove playlist entry: {e}");
                }
            }
            // The Favourite item commits the flag the menu reported, through
            // the one durable setter the row's heart also uses: one immediate
            // transaction, and the committed mutation bumps the library
            // generation so the projections re-resolve the row on the next
            // frame with zero caller action (ADR 0002). Nothing else moves — no
            // transport command, no playlist entry, no tag draft — because a
            // Favourite is a library fact and nothing else. A failed commit
            // changes nothing and is logged, the same as the heart's.
            TrackMenuIntent::SetFavorite(favorite) => {
                if let Err(e) = self
                    .library_mutations
                    .set_track_favorite(&track_id, favorite)
                {
                    tracing::warn!("Failed to commit the favorite flag for {}: {e}", track_id.0);
                }
            }
            // "Edit Tags" is the inline editor's entry point: it selects the
            // Track (so the Detail Panel shows its readout) and opens the
            // per-selection draft focused on the first tag field (Issue 04).
            TrackMenuIntent::EditTags => {
                let Some(track) = subject.track() else {
                    return;
                };
                *self.selected_track = Some(track_id);
                let rows = tag_rows(std::slice::from_ref(track));
                self.tag_editor
                    .open_track(track.id.clone(), track.file_path.clone(), &rows);
                // The entry point focuses the first tag field (Issue 04): the
                // one-shot flag is consumed the frame it lands.
                if let Some(draft) = self.tag_editor.draft_mut() {
                    draft.focus_first = true;
                }
            }
        }
    }
}

/// What a Track row's right-click reported: whether the popup was open on this
/// frame, and whatever was chosen from it in click order.
///
/// A report in its own right, and not a flag on a row, because opening and
/// choosing are kept apart on purpose — see [`TrackMenuHost::right_clicked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackMenuOpen {
    /// The row's popup was open on this frame — with nothing chosen yet, or
    /// with a choice [`TrackMenuHost::item_chosen`] has already answered.
    Opened,
    /// The row was painted and nothing opened on it: the state a list of Track
    /// rows spends almost all of its frames in, and the one that must move
    /// nothing at all.
    NotOpened,
}

/// Paint one Track row's menu and report what happened: whether the popup
/// opened, and every intent the listener chose from it.
///
/// The rows and their meanings belong to [`crate::ui::menu`]; this decides
/// which rows exist from the subject, renders them inside egui's popup, and
/// hands the choices back. **No Transport command, store write, or editor
/// draft can happen while the menu is being painted** — the host is not even
/// borrowed until this returns, which is the whole point of keeping the
/// painting and the answering apart.
///
/// Returns whether the popup was open, which is the menu's only
/// selection-carrying report: the row's identity, and the fact that its menu
/// opened, apart from anything the listener chose from it. The popup is egui's
/// own transient one, keyed by the row's response identity rather than by any
/// index, so a virtualized listing keeps no per-row open state — and a row
/// materialized only because it was on screen is all the report needs to say.
pub(crate) fn paint_track_menu(
    response: &egui::Response,
    palette: &Palette,
    playlists: &[(PlaylistId, String)],
    subject: TrackMenuSubject<'_>,
) -> (TrackMenuOpen, Vec<TrackMenuIntent>) {
    let menu = subject.menu(playlists);
    let mut intents = Vec::new();
    let open = response
        .context_menu(|ui| {
            crate::ui::menu::track_menu(ui, palette, &menu, &mut intents);
        })
        .is_some();
    let open = if open {
        TrackMenuOpen::Opened
    } else {
        TrackMenuOpen::NotOpened
    };
    (open, intents)
}
