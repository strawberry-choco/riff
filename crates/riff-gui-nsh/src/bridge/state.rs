//! The [`UiState`]: the signals the bridge owns.
//!
//! Every signal here is written on the engine thread and nowhere else — at
//! runtime through the pump, and in debug builds through the fixture's
//! variant switcher and the readout's toggles. The worker threads hand
//! results over by channel and never write (the plan's "Worker threads
//! never touch a signal"). nsh signals are `!Send`
//! (P0 correction 9), so this bundle lives and dies with the engine thread:
//! [`UiState`] is `Copy` because every field is a `Signal` handle, and the
//! app copies the handles it renders from once.

use nsh_core::element::{Signal, signal};
use riff_backend::app::events::NoticePayload;

use crate::model::{PlaybackView, SettingsSnapshot, SidebarView};
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
            settings: signal(SettingsSnapshot::default()),
            palette: signal(Palette::dark()),
            notices: signal(Vec::new()),
            covers: signal(0),
        }
    }
}
