//! The per-app **Collection-menu host**: the one place a right-click on an
//! entity row is turned into an effect.
//!
//! One user gesture — a right-click on an entity row of a Section root or a
//! Drill Column — is answered here, and only here. Before this module the
//! gesture was answered through a five-field effects bag that each of the two
//! dispatch bindings in `app/browser_pane.rs` assembled by hand, and the test
//! suite carried a field-for-field **mirror** of that bag at six sites — so a
//! site that quietly dropped a handle left the mirror complete and the test
//! green, proving a host the app did not have.
//!
//! The host replaces that with one declaration: [`CollectionMenuHost::new`] is
//! the only place the handle set is written down. `RiffApp::collection_menu`
//! calls it in production and the test suite calls it too, so there is no
//! second assembly for the two to drift apart in, and the fields are private
//! so no site can re-assemble the bag by hand.
//!
//! **Borrowed per gesture, not stored on the app.** The host borrows the five
//! handles it owns rather than holding them: the library and playback sessions
//! are the frame's own borrows, threaded into the stage per gesture, and
//! storing them would move both out of that threading for no gain. The scope
//! that matters is where the handles are *named*, and that is one function.

use riff_backend::app::Transport;
use riff_backend::app::pass_service::Passes;
use riff_backend::app::replaygain_pass::PassCommand;
use riff_backend::app::state::{LibrarySession, PlaybackSession};
use riff_backend::app::views::SessionViews;

use crate::ui::column::ColumnIdentity;

use super::{
    EntityRow, apply_entity_selection, apply_list_menu_intent, entity_row, entity_track_ids,
    play_batch,
};

/// The host slots one collection menu's effects answer to — the five handles a
/// right-click on an entity row reaches, and nothing else.
///
/// Nothing in here is reachable while the menu is being painted: the renderer
/// only reports a choice, or the fact that the menu was opened at all. The
/// fields are private, so the set is declared once in [`Self::new`] and
/// [`apply_collection_menu`] reads it off the host rather than being handed
/// the five handles separately at every call.
pub struct CollectionMenuHost<'a> {
    /// The library session: opening an entity row's menu selects it, so the
    /// menu just opened and the Detail Panel describe the same entity.
    library: &'a mut LibrarySession,
    /// The playback session: the Shuffle item's flag lives on its queue, and
    /// the batch intents go to the engine from there.
    playback: &'a mut PlaybackSession,
    /// The playback command port the batch intents go through.
    transport: &'a dyn Transport,
    /// The `ReplayGain` Pass service: the Album menu's two measure items are
    /// submissions here, never runs — the pass owns its worker thread.
    passes: &'a dyn Passes,
    /// The Session Views seam, which resolves the row's Track batch at
    /// dispatch time.
    views: &'a mut SessionViews,
}

impl<'a> CollectionMenuHost<'a> {
    /// Take the handles one collection-menu answer needs. This is the app's
    /// single declaration of that set — `RiffApp::collection_menu` calls it in
    /// production and the test suite calls it too, so there is no second
    /// assembly for the two to drift apart in.
    pub fn new(
        library: &'a mut LibrarySession,
        playback: &'a mut PlaybackSession,
        transport: &'a dyn Transport,
        passes: &'a dyn Passes,
        views: &'a mut SessionViews,
    ) -> Self {
        Self {
            library,
            playback,
            transport,
            passes,
            views,
        }
    }
}

/// Apply one entity row's context-menu report: the selection, then whatever the
/// listener chose.
///
/// `column` is the identity of the Column the row was clicked in — its Section,
/// its depth, its Scroll Memory slot — which the Column stated once and this
/// function reads. A Section root and a Drill Column therefore cannot disagree
/// with a plain click on the same row: it is the same identity, reached through
/// the same two calls.
///
/// The selection is applied FIRST and unconditionally, before the intents are
/// looked at: OPENING the menu is what moves it, so a right-click that chooses
/// nothing still leaves the readout agreeing with the menu on screen. That is
/// also why the selection is an argument here rather than something read back
/// off the click — this whole function is reachable, and provable, without a
/// pointer.
///
/// Each intent then goes through [`apply_list_menu_intent`], the same applier a
/// playlist header, a smart playlist header, and a folder node use, so the four
/// items mean one thing on all six set-of-Tracks surfaces and their effect logic
/// is written once rather than twice.
pub fn apply_collection_menu(
    key: &str,
    intents: &[crate::ui::menu::ListMenuIntent],
    column: ColumnIdentity,
    host: CollectionMenuHost<'_>,
) {
    apply_entity_selection(key, column, host.library);
    for intent in intents {
        // The Album menu's two measure items are not batch actions over a
        // Track set — they name the Album itself. The other four go through
        // the shared list-intent applier.
        // An Album row's play/shuffle is a **whole-Album play** — the
        // `ReplayGain` queue provenance — while an Artist or Genre row is a
        // batch of Albums and a folder or playlist is neither.
        let whole_album = matches!(
            entity_row(key, column.section(), column.level()),
            Some(EntityRow::Album { .. })
        );
        match intent {
            crate::ui::menu::ListMenuIntent::MeasureAlbumAggregate => {
                if let Some(EntityRow::Album { artist, title }) =
                    entity_row(key, column.section(), column.level())
                {
                    host.passes.submit(PassCommand::AlbumAggregate {
                        album_artist: artist,
                        album_title: title,
                    });
                }
            }
            crate::ui::menu::ListMenuIntent::MeasureAlbumTracks => {
                if let Some(EntityRow::Album { artist, title }) =
                    entity_row(key, column.section(), column.level())
                {
                    host.passes.submit(PassCommand::AlbumTracks {
                        album_artist: artist,
                        album_title: title,
                    });
                }
            }
            crate::ui::menu::ListMenuIntent::Play => {
                let batch = entity_track_ids(key, column, host.views);
                play_batch(&batch, whole_album, host.transport);
            }
            crate::ui::menu::ListMenuIntent::Shuffle => {
                let batch = entity_track_ids(key, column, host.views);
                if batch.is_empty() {
                    continue;
                }
                host.playback.queue.set_shuffle(true);
                play_batch(&batch, whole_album, host.transport);
            }
            intent => {
                let batch = entity_track_ids(key, column, host.views);
                apply_list_menu_intent(*intent, &batch, host.transport, host.playback);
            }
        }
    }
}
