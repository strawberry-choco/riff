//! The [`UiState`]: the signals the bridge owns.
//!
//! Every signal here is written on the engine thread and nowhere else — at
//! runtime through the pump (and [`crate::shell::sidebar`]'s selection
//! handlers, which own the nav signal), and in debug builds through the
//! fixture's variant switcher and the readout's toggles. The worker threads
//! hand results over by channel and never write (the plan's "Worker threads
//! never touch a signal"). nsh signals are `!Send`
//! (P0 correction 9), so this bundle lives and dies with the engine thread:
//! [`UiState`] is `Copy` because every field is a `Signal` handle, and the
//! app copies the handles it renders from once.

use nsh_core::element::{Signal, signal};
use riff_backend::app::events::NoticePayload;
use riff_backend::domain::TrackId;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::model::{
    AppearanceMode, CustomListRow, EntityView, LibraryPathView, NavView, PlaybackView,
    PlaylistDetail, SettingsPage, SettingsSnapshot, SidebarView, SmartListDetail, TagDraft,
};
use crate::screens::smart_playlists::rules::SmartDraft;
use crate::theme::Palette;

/// The number of typed notices the signal's history keeps. The notices
/// signal carries the recent history (the plan's `Signal<Vec<NoticePayload>>`);
/// N5's toast surface reads the newest entries off it.
pub(crate) const NOTICE_HISTORY: usize = 50;

/// The signals the bridge owns. One bundle, copied out of the model once per
/// consumer — never passed by reference across a `stored` borrow (P0
/// correction 20: a signal write inside the borrow re-borrows the `RefCell`
/// and panics).
#[derive(Clone, Copy)]
pub struct UiState {
    /// The playback snapshot, rebuilt and written only when the pump's cheap
    /// comparison moved.
    pub playback: Signal<PlaybackView>,
    /// The Library generation, as the event inbox last reported it. Moves
    /// with scans and other committed Library mutations.
    pub library_revision: Signal<u64>,
    /// Bumped when the playlist projection's cached list is observed to have
    /// changed. The playlist generation has no event (the inbox carries
    /// Library generations only), so the pump observes the projection's
    /// cache through its read.
    pub playlist_revision: Signal<u64>,
    /// The sidebar's counts and last-scan freshness.
    pub sidebar: Signal<SidebarView>,
    /// Where the app's navigation stands: the selection handlers' own
    /// record of what they wrote. The pump never touches it — a selection
    /// write mutates no store, so the pump's sidebar rebuild cannot see it.
    pub nav: Signal<NavView>,
    /// The selected track, the Library session's `selected_track` as the
    /// row-press handlers last wrote it — the same mirror the nav signal
    /// is for the view fields (a selection write mutates no store, the
    /// render bindings lock no session). The list's row bands and the
    /// inspector read it; the pump never touches it.
    pub selection: Signal<Option<TrackId>>,
    /// The All Tracks listing's total, as the pump's cached `track_count`
    /// read last answered. The virtual list sizes itself with it.
    pub list_total: Signal<usize>,
    /// Bumped when the pump resets the list's warm row window — a
    /// committed Library mutation moved the rows behind the bindings. A
    /// row binding reads it beside the window so refreshed rows repaint
    /// (P0 correction 21: reading a signal is what re-runs a binding).
    pub list_revision: Signal<u64>,
    /// The scalar settings as the hydrated sessions carry them.
    pub settings: Signal<SettingsSnapshot>,
    /// The design system's runtime store (ADR 0004's N2 amendment). The
    /// bridge constructs it now that it owns the signals; every riff surface
    /// still reads its colors from it, and `theme.json` still only restyles
    /// `nsh_base` defaults.
    pub palette: Signal<Palette>,
    /// The typed notices' recent history, oldest first.
    pub notices: Signal<Vec<NoticePayload>>,
    /// The cover map's revision count: bumped on every arrival batch. A
    /// screen's `dynamic` block reads this signal so its closure re-runs
    /// when covers land (P0 correction 21); the pixels live in the model's
    /// [`crate::model::CoverMap`].
    pub covers: Signal<u64>,
    /// The titlebar search field's needle, as the field's handler last
    /// wrote it — the session's `search_query` mirror (that write mutates no
    /// store, so the pump cannot carry it; the pump reads the session
    /// directly and retargets the listing, this signal is what the render
    /// bindings subscribe to). The stage empty copies and the folder tree's
    /// pruning read it; the pump never touches it.
    pub search: Signal<String>,
    /// The search field's own text, the controlled value — in the bundle
    /// (not the field's local signal) so the shell's Escape, the one
    /// `When::Always` binding, can clear it: a native input's raw key path
    /// serves custom views only, so the restore cannot ride a listener on
    /// the field itself.
    pub search_text: Signal<String>,
    /// The active entity stage's rows (the artists/albums/genres listings),
    /// read and shaped by the pump — one batched read per stage build.
    /// `Flat`/`Tracks` render track rows out of the row window instead and
    /// leave this signal untouched.
    pub browse: Signal<Arc<[EntityView]>>,
    /// The Library session's folder roots, as the pump last read them —
    /// the Folders tree's top level. A session read a render binding cannot
    /// take (standing order 6); the pump already reads the paths every
    /// sidebar build and mirrors them here.
    pub folder_roots: Signal<Arc<[PathBuf]>>,
    /// The Folders tree's expanded directories — the row presses' own
    /// record, so the tree's dynamic rebuilds on a toggle and survives a
    /// stage switch (the egui `CollapsingState`'s persistence, as a signal).
    /// The children behind an expansion are read at expand time (lazy), so
    /// a deep tree only ever reads the levels the walk opened.
    pub folder_open: Signal<Arc<HashSet<PathBuf>>>,
    /// The Folders tree's selected directory, the session's
    /// `selected_folder` as the row presses last wrote it — the same mirror
    /// the nav signal is for the view fields (a folder select mutates no
    /// store). The row bands read it.
    pub folder_selected: Signal<Option<PathBuf>>,
    /// The track whose tag editor the inspector shows; `None` = read-only.
    /// Set by the track menu's Edit Tags and the inspector's edit control,
    /// cleared by Cancel and by a successful save. The draft itself lives in
    /// the editor's own stored state.
    pub tag_edit: Signal<Option<TrackId>>,
    /// Whether the Up Next panel shows above the player bar, mirrored from
    /// the session's `queue_open` so the toggle's write repaints the shell.
    pub queue_open: Signal<bool>,
    /// The custom smart playlists the editor and the sidebar's SMART LISTS
    /// section read; written by the pump when the playlist generation or the
    /// library moves, and after a save.
    pub custom_lists: Signal<Arc<[CustomListRow]>>,
    /// Why the custom-list read failed, if it did — the honest reason the
    /// sidebar and editor show instead of an empty list.
    pub custom_lists_error: Signal<Option<String>>,
    /// The opened user playlist's detail, refreshed by the pump when the
    /// playlist or library generation moves.
    pub playlist_detail: Signal<Option<PlaylistDetail>>,
    /// The sidebar's inline create-playlist row is open.
    pub playlist_creating: Signal<bool>,
    /// The inline name draft (create and rename share it).
    pub playlist_draft: Signal<String>,
    /// The playlist header's inline rename field is open.
    pub playlist_renaming: Signal<bool>,
    /// The playlist header's delete confirmation is showing.
    pub playlist_delete_confirm: Signal<bool>,
    /// The name refusal the last create or rename produced, shown verbatim.
    pub playlist_error: Signal<Option<String>>,
    /// The opened built-in smart list's detail, refreshed by the pump.
    pub smart_list_detail: Signal<Option<SmartListDetail>>,
    /// The tag editor's draft, its album mode, and its save status.
    pub tag_draft: Signal<TagDraft>,
    pub tag_album_mode: Signal<bool>,
    pub tag_saving: Signal<bool>,
    pub tag_status: Signal<Option<String>>,
    /// The files still waiting for a tag-edit outcome, oldest first — the
    /// poller pairs each outcome with the file it named.
    pub tag_pending: Signal<Vec<PathBuf>>,
    /// How many tag edits saved since the last save began, and the failures
    /// with their file names.
    pub tag_saved: Signal<usize>,
    pub tag_failures: Signal<Vec<String>>,
    /// The custom smart playlist editor: whether it is open, its draft, the
    /// saved baseline the unsaved guard compares against, a revision the
    /// tree's structural rebuild subscribes to, and its status line.
    pub smart_editing: Signal<bool>,
    pub smart_draft: Signal<Option<SmartDraft>>,
    pub smart_saved: Signal<Option<SmartDraft>>,
    pub smart_draft_rev: Signal<u64>,
    pub smart_status: Signal<Option<String>>,
    pub smart_confirm_discard: Signal<bool>,
    /// The custom list header's delete confirmation.
    pub smart_delete_confirm: Signal<bool>,
    /// The open custom list's live match total, built by the pump.
    pub custom_list_total: Signal<usize>,
    /// The Settings screen: the selected page, the appearance choice, the
    /// registered paths as the page lists them, the typed add-path draft,
    /// and the status lines the page's actions report.
    pub settings_page: Signal<SettingsPage>,
    pub appearance: Signal<AppearanceMode>,
    pub library_paths: Signal<Arc<[LibraryPathView]>>,
    pub library_add_draft: Signal<String>,
    pub settings_status: Signal<Option<String>>,
    pub clear_confirm: Signal<bool>,
    /// The thumbnail cache clear's settled outcome, and the ReplayGain
    /// pass's progress and settled report.
    pub cache_status: Signal<Option<String>>,
    pub pass_progress: Signal<(usize, usize)>,
    pub pass_status: Signal<Option<String>>,
    /// The ReplayGain pass's force switch (session state, like the egui
    /// Settings surface's).
    pub pass_force: Signal<bool>,
    /// The newest handler-reported line (a drop's outcome, a picker's
    /// refusal); the titlebar's status slot prefers it over the notice log.
    pub action_status: Signal<Option<String>>,
}

impl UiState {
    /// The bundle a fresh model starts with: everything empty except the
    /// palette, which opens dark like the N1/N2 app did.
    pub(crate) fn new() -> Self {
        Self {
            playback: signal(PlaybackView::default()),
            library_revision: signal(0),
            playlist_revision: signal(0),
            sidebar: signal(SidebarView::default()),
            nav: signal(NavView::default()),
            selection: signal(None),
            list_total: signal(0),
            list_revision: signal(0),
            settings: signal(SettingsSnapshot::default()),
            palette: signal(Palette::dark()),
            notices: signal(Vec::new()),
            covers: signal(0),
            search: signal(String::new()),
            search_text: signal(String::new()),
            browse: signal(Arc::from([])),
            folder_roots: signal(Arc::from([])),
            folder_open: signal(Arc::new(HashSet::new())),
            folder_selected: signal(None),
            tag_edit: signal(None),
            queue_open: signal(false),
            custom_lists: signal(Arc::from([])),
            custom_lists_error: signal(None),
            playlist_detail: signal(None),
            playlist_creating: signal(false),
            playlist_draft: signal(String::new()),
            playlist_renaming: signal(false),
            playlist_delete_confirm: signal(false),
            playlist_error: signal(None),
            smart_list_detail: signal(None),
            tag_draft: signal(TagDraft::default()),
            tag_album_mode: signal(false),
            tag_saving: signal(false),
            tag_status: signal(None),
            tag_pending: signal(Vec::new()),
            tag_saved: signal(0),
            tag_failures: signal(Vec::new()),
            smart_editing: signal(false),
            smart_draft: signal(None),
            smart_saved: signal(None),
            smart_draft_rev: signal(0),
            smart_status: signal(None),
            smart_confirm_discard: signal(false),
            smart_delete_confirm: signal(false),
            custom_list_total: signal(0),
            settings_page: signal(SettingsPage::default()),
            appearance: signal(AppearanceMode::default()),
            library_paths: signal(Arc::from([])),
            library_add_draft: signal(String::new()),
            settings_status: signal(None),
            clear_confirm: signal(false),
            cache_status: signal(None),
            pass_progress: signal((0, 0)),
            pass_status: signal(None),
            pass_force: signal(false),
            action_status: signal(None),
        }
    }
}
