//! The **Frame**: one UI frame, from the input read to the write-back.
//!
//! `RiffApp::ui()` used to be a ~180-line ordered sequence that cloned the two
//! session `Arc`s, snapshotted playback, took the library guard, resolved the
//! native close, installed the theme, drained the event inbox and three
//! background services, composed the status line, uploaded cover textures,
//! polled the watcher, read the keyboard, pushed the OS title, ran three panel
//! closures that each mutated *and* drained, and then wrote six fields back.
//! Every one of those steps was load-bearing — `app.rs` said so, in a comment
//! nobody was failing a test against — and the sequence itself was the part
//! most likely to be quietly broken.
//!
//! This module owns that order. The seam sits **above** the panels, so the
//! panels' only job is to paint content and report what the user did; the
//! Frame decides what those reports mean and writes it to the sessions.
//!
//! # The order this module guarantees
//!
//! [`Frame::advance`] is the whole pre-panel half, in this order:
//!
//! 1. resolve the native close request (macOS) — **first**, because eframe
//!    quits unless *that* frame's viewport output carries the cancel;
//! 2. resolve and install the theme (REQ-UI-007);
//! 3. drain the Backend Events inbox — typed notices to the feedback board,
//!    and any Library generation move into the Scroll Memory;
//! 4. drain the three background services (scan, Tag Edit, Thumbnail-cache
//!    clear) into the feedback board;
//! 5. compose the titlebar status line from the board — **after** the
//!    drains, because *a slot filled after the compose is a frame late*;
//! 6. settle the Cover Cache and hand the arrivals to the View half — after
//!    the compose, because a clear that lands in step 4 must reach the line
//!    this frame paints;
//! 7. poll the watcher;
//! 8. read the keyboard (Ctrl+K, Space);
//! 9. push the window title / tray tooltip.
//!
//! Then the driver draws and reports, one slot per panel, and the Frame
//! answers each report at its own slot — the titlebar's before the sidebar
//! highlights the navigation it changed, the control bar's before the stage
//! picks the view it switched, the stage's last:
//!
//! 10. titlebar report → 11. sidebar report → 12. control-bar report →
//! 13. stage report → 14. [`Frame::finish`], which owns the six-field
//!     write-back and the frame-end Preferences commit.
//!
//! # Why the seam is here, and the two alternatives that were rejected
//!
//! *Cutting **below** the services* — around the three drains and the compose
//! only — is smaller and would have left the two hard parts untestable: the
//! panel closures' mutation (a titlebar click changing `view_mode` between the
//! sidebar's highlight and the stage's branch) and the drain order relative to
//! it. Nothing above that cut can assert an order.
//!
//! *Cutting **on** the panel actions* — so each panel keeps its own
//! `apply_*_action` — leaves the ordering itself in the view, which is exactly
//! where it was invisible. The order has to be owned by something a test can
//! call.
//!
//! If someone later "simplifies" toward either, this paragraph is why not.
//!
//! # Why this module is assertable with no egui at all
//!
//! [`Frame`] names no egui type, [`FrameInput`] is pure data, and
//! [`FrameOutput`] is decisions only — a palette, a viewport intent, a
//! tooltip, a Cover arrival. Everything egui-shaped (installing the palette,
//! uploading arrivals, sending viewport commands, requesting a repaint) is a
//! leaf the draw half performs on the way past, which is why
//! `crates/riff-gui/tests/frame_tests.rs` can drive the whole protocol and
//! assert its order with no `Context`, no `build_eframe` and no kittest.
//!
//! `FrameOutput` staying decisions-only is a discipline, not a convenience: an
//! output that accreted widget-ready values would be a shallower module than
//! the function it replaced, and **nothing would fail**. Keep the draw half
//! out of it.

use std::sync::Mutex;

use riff_backend::app::cover_service::{ClearCacheOutcome, Covers};
use riff_backend::app::preferences::Preferences;
use riff_backend::app::scan_service::{ScanOutcome, Scans};
use riff_backend::app::state::{
    BrowseMode, LibrarySection, LibrarySession, LibraryStatus, PlaybackSession, ViewMode,
};
use riff_backend::app::store::{PlaylistStore, SettingsStore};
use riff_backend::app::traits::RequestedSize;
use riff_backend::app::views::SessionViews;
use riff_backend::app::watcher_manager::WatcherManager;
use riff_backend::app::{MutexExt, Transport};
use riff_backend::domain::{PlaylistId, SmartPlaylistKind};

use super::app::{InlineTagEditor, ThemeInputs, ThemeState};
// Gated with the one caller: nothing on Windows or Linux resolves a native
// close, and an ungated import would be an `unused_imports` there.
#[cfg(target_os = "macos")]
use super::app::{CloseIntent, close_resolution};
use super::chrome::TitleBarAction;
use super::cover_cache::{CoverArrival, CoverCache};
use super::feedback::{Feedback, FeedbackBoard};
use super::now_playing::NowPlayingAction;
use super::playerbar::PlayerBarAction;
use super::prompts::PromptOutcome;
use super::scroll_memory::ScrollMemory;
use super::sidebar::PlaylistRowAction;
use super::theme::{self, Palette};
use riff_backend::app::pass_service::Passes;
use riff_backend::app::replaygain_pass::PassCommand;
use riff_backend::app::replaygain_pass::PassReport;

/// What one frame's egui input said.
///
/// The **only** thing the Frame reads from the draw half, and it is pure data:
/// every fact here is something egui owns (a viewport flag, a key press, a
/// window metric), read once by `RiffApp::read_frame_input` before the frame
/// starts. That is what lets [`Frame::advance`] run with no egui context at
/// all — the decision half never reaches back for one.
///
/// Field order mirrors the order the frame consumes it in.
///
/// Four of the six fields are genuinely two-state — a request is either
/// raised this frame or it is not — so the struct's bool count is a property of
/// egui's input vocabulary rather than of this type, and the lint that counts
/// them fires here.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameInput {
    /// Whether eframe reported a close request this frame (the macOS red
    /// traffic light).
    pub native_close_requested: bool,
    /// Whether the viewport reports itself maximized, for the titlebar's
    /// maximize toggle.
    pub viewport_maximized: bool,
    /// Whether Ctrl+K landed this frame.
    pub search_focus_requested: bool,
    /// Whether Space landed this frame **and** no text field owns the
    /// keyboard. The second half is part of the fact, not a later filter: the
    /// pre-frame code short-circuited on `egui_wants_keyboard_input` *before*
    /// consuming the key, and consuming a key a widget wanted is observable.
    pub toggle_playback: bool,
    /// The measured native traffic-light strip width, where the platform can
    /// measure one (`None` everywhere but macOS). The Frame applies the pure
    /// clearance policy to it; only the measurement needs the window.
    pub traffic_lights_width: Option<f32>,
    /// The context's zoom factor, the other half of that policy.
    pub zoom_factor: f32,
}

/// One OS-level intent the draw half enacts on the frame's behalf.
///
/// egui's own `ViewportCommand` would say the same things, but naming it here
/// would put an egui type in [`FrameOutput`] and cost this module its
/// headless assertability. The mapping is one `match` in `RiffApp`, on the
/// way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameViewport {
    /// Collapse the window to the taskbar (the titlebar's minimize).
    Minimize,
    /// Set the maximized flag to this value (the titlebar's maximize toggle).
    Maximized(bool),
    /// Quit: really close the window.
    Close,
    /// Take the close back for this frame — eframe quits unless the frame that
    /// reported the request carries this.
    CancelClose,
    /// Set the OS window title.
    Title(String),
}

/// What a native close request resolved to.
///
/// The signal carries no provenance (see [`CloseIntent`]): the tray's Quit
/// enqueues bit-for-bit the same `Close` the red light produces, and only the
/// app-wide quit flag separates them. The Frame owns that decision now, in
/// this vocabulary; the draw half just enacts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeClose {
    /// Let it through — riff is quitting, or the preference says a window
    /// close quits.
    Proceed,
    /// Take it back for this frame and hide the window instead.
    CancelAndHide,
}

/// What the frame decided, for the draw half to enact.
///
/// Decisions only: a palette to install, viewport intents, a tray tooltip, the
/// Cover arrivals to upload, whether to ask for another frame. No
/// `Context`, no `TextureHandle`, no `Response`, no `Ui`, no `Painter`, no
/// `Rect`, no `TextureId` — see this module's docs for why that is the
/// discipline worth keeping.
///
/// `Default` only, not `Debug`/`Clone`: the Cover arrivals carry decoded
/// pixels, and a frame's decisions are consumed once, on the way past.
#[derive(Default)]
pub struct FrameOutput {
    /// Install this palette on the context. `None` on every frame where the
    /// selection did not move: installation happens once at init and again
    /// only when the user switches, not per frame.
    pub palette: Option<Palette>,
    /// The palette-family flip invalidates the shared placeholder tile — its
    /// well and glyph colours were derived for the old family's tokens. The
    /// View half does the eviction; the Frame decides it.
    pub evict_generated: bool,
    /// What the native close request resolved to, when there was one. Always
    /// `None` off macOS, where a close is not resolved against the preference.
    pub native_close: Option<NativeClose>,
    /// OS-level intents, in the order the frame decided them.
    pub viewport: Vec<FrameViewport>,
    /// Hide the window through the frontend-local visibility channel — the
    /// custom titlebar X's gesture, and what a cancelled native close does.
    pub hide_window: bool,
    /// The tooltip to push to the tray icon, when the playing track's identity
    /// moved. `None` on a frame where it did not: pure command suppression,
    /// never staleness.
    pub tray_tooltip: Option<String>,
    /// A Thumbnail-cache clear the worker settled this frame, when one was
    /// outstanding. The View half flushes its texture map on `Cleared`.
    pub cache_clear: Option<ClearCacheOutcome>,
    /// The Covers that arrived this frame. The Cover Cache has already dropped
    /// their in-flight markers; wrapping and uploading them is the View
    /// half's work.
    pub cover_arrivals: Vec<CoverArrival>,
    /// Open the platform's folder picker for a library root. Set by the
    /// sidebar's Add-folder control; performed by the draw half, because the
    /// dialog belongs to the platform rather than to a decision. Nothing in
    /// this frame depends on its result, so it rides the tail.
    pub pick_folder: bool,
}

impl FrameOutput {
    /// Whether a native close request resolved to "take it back and hide".
    ///
    /// A named predicate rather than a field the draw half matches on, so the
    /// only place that knows what [`NativeClose`] means is this module — the
    /// draw half enacts the answer without ever naming the type.
    #[must_use]
    pub fn cancels_native_close(&self) -> bool {
        self.native_close == Some(NativeClose::CancelAndHide)
    }
}

/// Last identity pushed to the window title / tray tooltip. `Unset`
/// distinguishes "nothing pushed yet" from "pushed while nothing plays" so the
/// very first frame always pushes once.
#[derive(Default)]
pub enum TitleKey {
    #[default]
    Unset,
    Set(Option<riff_backend::domain::TrackId>),
}

/// Everything the Frame's decision half borrows out of `RiffApp`.
///
/// A bundle of disjoint field borrows rather than a `&mut RiffApp`, so the
/// frame can be driven with no application at all — which is the only reason
/// `crates/riff-gui/tests/frame_tests.rs` needs no egui, no kittest and no
/// eleven-argument constructor. Every field is a port, a session-scoped seam,
/// or a slot of frontend-local state; none of them is egui-shaped.
pub struct FrameParts<'a> {
    /// The theme selection and the palette currently installed.
    pub theme: &'a mut ThemeState,
    /// The structured feedback board the titlebar composes from.
    pub feedback: &'a mut FeedbackBoard,
    /// The per-Section scroll record; a committed mutation makes its slots
    /// stale.
    pub scroll_memory: &'a mut ScrollMemory,
    /// The Session Views seam: every store-backed read the frame renders.
    pub views: &'a mut SessionViews,
    /// The Backend Events inbox the frame drains at its start.
    pub backend_events: &'a Mutex<riff_backend::app::events::BackendEvents>,
    /// The playback command port.
    pub transport: &'a dyn Transport,
    /// The Library Scan Service front end.
    pub scans: &'a dyn Scans,
    /// The Tag Edit Service front end, inside the inline editor's controller.
    pub tag_edits: &'a mut InlineTagEditor,
    /// The Cover Service front end.
    pub covers: &'a dyn Covers,
    /// The `ReplayGain` Pass service front end.
    pub passes: &'a dyn Passes,
    /// The Settings surface's Force choice for the next library-wide pass.
    pub pass_force_choice: &'a mut bool,
    /// The last settled pass's outcome, kept for the Settings card's inline
    /// report; the frame's poll writes it.
    pub last_pass_report: &'a mut Option<PassReport>,
    /// The Cover Cache's decision half: wanted, in flight, arrived.
    pub cover_cache: &'a mut CoverCache,
    /// The watcher manager, polled once per frame.
    pub watchers: &'a Mutex<Option<WatcherManager>>,
    /// The Settings round-trip owner, diff-committed at frame end.
    pub prefs: &'a mut Preferences,
    /// The Application Store's settings section.
    pub settings_store: &'a mut dyn SettingsStore,
    /// The Application Store's playlists section.
    pub playlist_store: &'a mut dyn PlaylistStore,
    /// A Thumbnail-cache clear the worker has not answered yet.
    pub clear_cache_in_flight: &'a mut bool,
    /// Ctrl+K landed: one-shot focus request for the global search field.
    pub global_search_focus: &'a mut bool,
    /// The last identity the OS title and tray tooltip were pushed for.
    pub title_key: &'a mut TitleKey,
    /// Which user playlist is open in the library explorer.
    pub playlist_view: &'a mut Option<PlaylistId>,
    /// Which read-only smart playlist is open.
    pub smart_playlist_view: &'a mut Option<SmartPlaylistKind>,
    /// The inline rename prompt: (playlist id, draft name).
    pub playlist_rename: &'a mut Option<(PlaylistId, String)>,
    /// The inline "New Playlist" name prompt, holding the draft.
    pub playlist_create_name: &'a mut Option<String>,
    /// The live playback session, re-locked by [`Frame::finish`] to write the
    /// six UI-owned fields back.
    pub playback_live: &'a Mutex<PlaybackSession>,
    /// The Linux text-row folder flow's own state (no native dialog there): the
    /// sidebar footer opens it during its apply step, before the stage draws.
    #[cfg(target_os = "linux")]
    pub settings_show_input: &'a mut bool,
    #[cfg(target_os = "linux")]
    pub settings_path_error: &'a mut Option<String>,
    /// The app-wide quit intent, the one fact separating a riff-initiated quit
    /// from an OS window close (macOS only — nothing elsewhere reads it, and
    /// an ungated import would be an `unused_imports` there).
    #[cfg(target_os = "macos")]
    pub quit_flag: &'a std::sync::atomic::AtomicBool,
}

/// One frame, borrowed: the decision half's state and the frame's two sessions.
///
/// `playback` is the frame's **snapshot** (lock → clone → drop, taken by
/// `ui()` before the Frame exists): the engine and coordinator write playback
/// state on their own threads, so the frame renders from a plain clone and
/// writes back only the UI-owned fields in [`Self::finish`] — a whole-session
/// replace would clobber engine-written position and traversal index.
/// `library` is the **live guard**, held for the whole frame because the
/// library session is UI-owned.
///
/// A `Frame` is cheap to build and its borrows are short: the driver builds one
/// per slot (see `RiffApp::ui`), which is what lets the draw half keep its own
/// `&mut self` between the Frame's steps — the reason the slot API is
/// `advance` + one `apply_*` each + `finish` rather than one method that draws.
pub struct Frame<'a> {
    parts: FrameParts<'a>,
    playback: &'a mut PlaybackSession,
    library: &'a mut LibrarySession,
}

impl<'a> Frame<'a> {
    /// Assemble one frame over the app's state and this frame's two sessions.
    #[must_use]
    pub fn new(
        parts: FrameParts<'a>,
        playback: &'a mut PlaybackSession,
        library: &'a mut LibrarySession,
    ) -> Self {
        Self {
            parts,
            playback,
            library,
        }
    }

    /// Steps 1–9: everything the frame decides before any panel draws.
    ///
    /// Consumes [`FrameInput`] and produces decisions for the draw half. The
    /// whole method is egui-free by construction — every egui-owned fact
    /// arrives in `input`, and every egui-bound act is reported in the output
    /// rather than performed.
    pub fn advance(&mut self, input: &FrameInput) -> FrameOutput {
        let mut out = FrameOutput::default();

        self.resolve_native_close(input, &mut out);
        self.apply_theme(&mut out);
        self.drain_backend_events();
        self.drain_background_outcomes(&mut out);
        // Compose the titlebar status line from the independent source slots,
        // so a Library Scan update cannot erase a live playback error or a Tag
        // Edit outcome. **After** the drains: a slot filled after the compose
        // is a frame late, which is what `tests/frame_tests.rs` pins.
        self.library.scan_status = self.parts.feedback.display_message();
        out.cover_arrivals = self.settle_covers();
        self.poll_watchers();
        self.read_keyboard(input);
        self.update_window_title(&mut out);

        out
    }

    /// Step 10: answer the titlebar panel's report.
    ///
    /// The search query the field holds and the Ctrl+K focus request both land
    /// here rather than in the closure, which is why the titlebar panel takes
    /// content and returns a report instead of writing the session.
    pub fn apply_titlebar(&mut self, out: &mut FrameOutput, report: &TitlebarReport) {
        self.library.search_query.clone_from(&report.search_query);
        if report.focus_search {
            *self.parts.global_search_focus = false;
        }
        for action in report.actions.iter().copied() {
            self.apply_titlebar_action(action, report.maximized, out);
        }
    }

    /// Step 11: answer the sidebar panel's report.
    pub fn apply_sidebar(&mut self, out: &mut FrameOutput, report: &SidebarReport) {
        for action in report.actions.iter().cloned() {
            self.apply_sidebar_action(action, out);
        }
    }

    /// Step 12: answer the control-bar panel's report.
    ///
    /// Takes the output like its siblings do, even though nothing here needs it
    /// today: the bar's actions are all session writes and transport intents, so
    /// a slot that might one day decide a viewport command should not have to
    /// change shape when it does.
    pub fn apply_control_bar(&mut self, _out: &mut FrameOutput, report: &ControlBarReport) {
        for action in report.actions.iter().cloned() {
            apply_player_bar_action(action, self.library, self.playback, self.parts.transport);
        }
    }

    /// Step 13: answer the main stage's report. Same uniform shape as
    /// [`Self::apply_control_bar`], for the same reason.
    pub fn apply_stage(&mut self, _out: &mut FrameOutput, report: &StageReport) {
        for action in report.actions.iter().cloned() {
            apply_now_playing_action(action, self.library, self.playback, self.parts.transport);
        }
    }

    /// Step 14: the frame's write-back, with one owner.
    ///
    /// The Preferences commit runs **while the library guard is still live**,
    /// so it sees the frame's playback snapshot (volume, mute, replay-gain,
    /// shuffle, repeat) together with the library session's preference fields
    /// and lands any drift in the store — durability by construction, no
    /// per-handler call sites. Only then are the seven UI-owned playback
    /// fields written back into the live session; `playback_state`,
    /// `current_position`, and the queue's traversal state are the engine's and
    /// the coordinator's, and a whole-session replace here would clobber their
    /// work between frames.
    pub fn finish(self) {
        let parts = self.parts;
        parts
            .prefs
            .commit_if_changed(self.playback, self.library, parts.settings_store);
        let mut live = parts.playback_live.lock_or_recover();
        live.current_volume = self.playback.current_volume;
        live.muted = self.playback.muted;
        live.replaygain_enabled = self.playback.replaygain_enabled;
        live.replaygain_mode = self.playback.replaygain_mode;
        // What is NOT written back is deliberate: the engine and coordinator own
        // `playback_state`, `current_position`, and the queue's traversal index,
        // so a whole-session replace here would clobber them mid-frame.
        live.queue.set_shuffle(self.playback.queue.shuffle);
        live.queue.repeat = self.playback.queue.repeat;
    }

    // --- Step 1: the native close ------------------------------------------

    /// Resolve the macOS native close request (the red traffic light) for the
    /// frame being built, and record what it means.
    ///
    /// Resolved as the frame's FIRST decision, because eframe reads
    /// `close_requested()` at the top of the frame and quits unless THAT
    /// frame's viewport output carries [`FrameViewport::CancelClose`] — so the
    /// decision cannot be deferred, and nothing else in the frame may get to
    /// queue a competing close first.
    ///
    /// A no-op when eframe reported no close request, and on Windows/Linux
    /// (not compiled) — their frameless OS close always quits.
    #[cfg(target_os = "macos")]
    fn resolve_native_close(&mut self, input: &FrameInput, out: &mut FrameOutput) {
        if !input.native_close_requested {
            return;
        }
        // The load is `Acquire` to pair with the tray's `Release` store; the
        // correctness argument rests on that pair, not on any third-party
        // crate's internal mutex.
        let intent = if self
            .parts
            .quit_flag
            .load(std::sync::atomic::Ordering::Acquire)
        {
            CloseIntent::Quit
        } else {
            CloseIntent::WindowClose
        };
        // `close_resolution` is the one applier both close paths go through, so
        // the native red light and the custom X cannot drift; its answer is
        // mapped onto the frame's own vocabulary and the draw half turns THAT
        // into the two viewport acts.
        out.native_close = Some(
            match close_resolution(intent, self.library.ui_flags.close_quits_app) {
                Some(_) => NativeClose::CancelAndHide,
                None => NativeClose::Proceed,
            },
        );
    }

    /// No native close request to resolve anywhere but macOS.
    #[cfg(not(target_os = "macos"))]
    #[allow(clippy::unused_self)]
    fn resolve_native_close(&mut self, _input: &FrameInput, _out: &mut FrameOutput) {}

    // --- Step 2: the theme -------------------------------------------------

    /// Resolve the active palette and record that it is due for installation.
    ///
    /// The palette is resolved from the token module — dark (mockup) or light
    /// (derived per ADR 0004), with High Contrast as a token-set variant over
    /// the base. Installation happens once at init and again only when the
    /// selection changes, not every frame: the draw half performs it. The
    /// resolved palette is kept on [`ThemeState`] either way, so view code
    /// styles itself from the active tokens (ADR 0004) on the same frame the
    /// selection moved.
    ///
    /// A persisted high-contrast choice is already in the session — the runtime
    /// hydrated it before this app existed — so it takes effect on the very
    /// first frame.
    fn apply_theme(&mut self, out: &mut FrameOutput) {
        let inputs = ThemeInputs {
            dark: self.parts.theme.dark,
            high_contrast: self.library.ui_flags.high_contrast,
            reduce_motion: self.library.ui_flags.reduce_motion,
        };
        if self.parts.theme.last_applied == Some(inputs) {
            return;
        }

        let palette = theme::resolve(inputs.dark, inputs.high_contrast);
        // A palette-family flip invalidates the placeholder tile: its well and
        // glyph colours were derived for the old family's tokens, so it
        // re-renders under the new one on its next lookup. Keyed on the dark
        // axis ONLY: a reduce-motion-only flip re-installs the style above but
        // must not evict any real cover, so `evict_generated` never fires for it.
        out.evict_generated = self.parts.theme.active.dark != palette.dark;
        out.palette = Some(palette);
        self.parts.theme.active = palette;
        self.parts.theme.last_applied = Some(inputs);
    }

    // --- Step 3: the event inbox -------------------------------------------

    /// Drain the backend event inbox for this frame.
    ///
    /// Called at the start of the frame so any dispatch recorded by the tray
    /// thread or by a transport between frames is observable before the UI
    /// renders. The frontend renders from the real engine updates on the
    /// playback session; this seam's events are the observability surface that
    /// proves every dispatch path (mouse/keyboard/tray) flows through one
    /// recorded Transport.
    fn drain_backend_events(&mut self) {
        let events = {
            use std::sync::PoisonError;
            self.parts
                .backend_events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .events()
        };
        // Fold any Library-generation move into the Scroll Memory, so a
        // committed rescan turns every Section slot's fingerprint stale.
        self.parts.scroll_memory.note_backend_events(&events);
        apply_backend_events(events, self.parts.feedback);
    }

    // --- Step 4: the three background services -----------------------------

    /// Drain every background service's outstanding results into the feedback
    /// board, in the order the status line is later composed from it. The three
    /// drains are one step because their sequence relative to
    /// `feedback.display_message()` is load-bearing: a slot filled after the
    /// compose is a frame late.
    fn drain_background_outcomes(&mut self, out: &mut FrameOutput) {
        self.poll_library_updates();
        self.parts.tag_edits.poll_outcomes(self.parts.feedback);
        self.poll_cache_clear_outcome(out);
        self.poll_pass_outcome();
    }

    /// Poll the `ReplayGain` Pass service: while a pass runs, its progress
    /// line lives on the pass's own feedback slot; when one settles, its
    /// outcome report lands there — measured, skipped, failed, cancelled —
    /// the way Tag Edit outcomes are surfaced. Nothing else in the frame is
    /// touched: the values themselves are Store facts the read models pick
    /// up through the session generation.
    fn poll_pass_outcome(&mut self) {
        use riff_backend::app::events::NoticeSeverity;
        let passes = self.parts.passes;
        if passes.is_running() {
            let (done, total) = passes.poll_progress();
            if total > 0 {
                self.parts.feedback.set_replaygain(
                    format!("Measuring ReplayGain ({done}/{total})\u{2026}"),
                    NoticeSeverity::Info,
                );
            }
        }
        while let Some(report) = passes.poll() {
            let failed = report.failed > 0;
            *self.parts.last_pass_report = Some(report.clone());
            let message = pass_outcome_line(&report);
            let severity = if failed {
                NoticeSeverity::Error
            } else {
                NoticeSeverity::Info
            };
            self.parts.feedback.set_replaygain(message, severity);
        }
    }

    /// Drain polled Library Scan outcomes and report each root's Readiness
    /// through the [`LibraryPaths`] slot — the scan worker writes *through* the
    /// module instead of reaching into the session — plus the titlebar
    /// scan-status line. The service NEVER touches `LibrarySession` (ADR
    /// 0006). The watcher observes a scan's end itself via `is_scanning`, so no
    /// relay fires here.
    fn poll_library_updates(&mut self) {
        use riff_backend::app::events::{NoticeSeverity, NoticeSource};
        for outcome in self.parts.scans.poll() {
            match outcome {
                ScanOutcome::Progress { path, files_found } => {
                    self.library
                        .library_paths
                        .report_readiness(&path, LibraryStatus::Scanning { files_found });
                    self.parts
                        .feedback
                        .set_scan(format!("{files_found} files"), NoticeSeverity::Info);
                }
                ScanOutcome::Complete { path, total_files } => {
                    self.library
                        .library_paths
                        .report_readiness(&path, LibraryStatus::Scanned(total_files));
                    self.parts.feedback.set_scan(
                        format!("Scan complete: {total_files} tracks"),
                        NoticeSeverity::Info,
                    );
                    // When the pass's Settings gating enables it, a
                    // library-wide pass follows every completed scan —
                    // watcher-triggered included — filling in only unmeasured
                    // Tracks, so scans never redo finished work. Force is
                    // never set here. The checkboxes never affect the menu
                    // commands: those are targeted passes that ignore this
                    // state entirely.
                    let prefs = &self.library.pass_prefs;
                    if prefs.track_values || prefs.album_values {
                        self.parts.passes.submit(PassCommand::LibraryWide {
                            track_values: prefs.track_values,
                            album_values: prefs.album_values,
                            force: false,
                        });
                    }
                    // Scan batches already committed through the store as they
                    // progressed; nothing whole-file remains to save.
                }
                ScanOutcome::Failed { path, reason } => {
                    self.library
                        .library_paths
                        .report_readiness(&path, LibraryStatus::Idle);
                    // A failed scan carries an Error severity and a Rescan
                    // recovery intent through to the paint boundary.
                    self.parts.feedback.put(Feedback {
                        severity: NoticeSeverity::Error,
                        source: NoticeSource::Scan,
                        message: format!("Error: {reason}"),
                        recovery: Some(super::feedback::Recovery::Rescan),
                    });
                }
            }
        }
    }

    /// Drain the settled outcome of a Thumbnail-cache clear and report it on
    /// the status line the rest of the Library pane already uses. A cache that
    /// cannot be cleared is an inconvenience, not a data-loss event, so this is
    /// one line in the feedback board — never a modal.
    ///
    /// The View half's texture-map flush travels in the output rather than
    /// happening here, because the map is egui state. Its ORDER does not: the
    /// draw half enacts the flush before the step-6 uploads, which is the order
    /// the pre-split frame ran — at confirm time every visible row would
    /// re-request while the wipe was still queued behind those very requests,
    /// rebuilding the entries the user had just asked to delete. Settled is the
    /// moment the disk is genuinely empty, so it is the moment the screen can
    /// be emptied with it. A `Failed` clear leaves every texture in place.
    fn poll_cache_clear_outcome(&mut self, out: &mut FrameOutput) {
        if !*self.parts.clear_cache_in_flight {
            return;
        }
        let Some(outcome) = self.parts.covers.poll_cache_clear() else {
            return;
        };
        *self.parts.clear_cache_in_flight = false;
        if outcome == ClearCacheOutcome::Cleared {
            // The View half is empty, so nothing is arrived any more. Without
            // this the cache would go on believing in entries that were just
            // dropped and no row would ask again.
            self.parts.cover_cache.forget_arrivals();
        }
        out.cache_clear = Some(outcome.clone());
        match &outcome {
            ClearCacheOutcome::Cleared => {
                self.parts.feedback.set_library(
                    "Thumbnail cache cleared. Covers rebuild as you browse.".to_string(),
                    riff_backend::app::events::NoticeSeverity::Info,
                );
            }
            ClearCacheOutcome::Failed { reason } => {
                tracing::warn!("Failed to clear the Thumbnail cache: {reason}");
                self.parts.feedback.set_library(
                    "Failed to clear the Thumbnail cache \u{2014} nothing was changed.".to_string(),
                    riff_backend::app::events::NoticeSeverity::Error,
                );
            }
        }
    }

    // --- Step 6: covers ----------------------------------------------------

    /// Settle the Cover Cache: which Covers arrived, which markers drop, and
    /// which textures the View half must upload.
    ///
    /// The rgba→texture conversion is the egui-bound half and stays on the
    /// main thread with the draw; every other caching concern lives in one of
    /// the two halves.
    fn settle_covers(&mut self) -> Vec<CoverArrival> {
        self.parts.cover_cache.settle(self.parts.covers)
    }

    // --- Step 7: the watcher -----------------------------------------------

    /// Let the watcher observe what the frame's drains settled.
    fn poll_watchers(&mut self) {
        if let Some(ref mut mgr) = *self.parts.watchers.lock_or_recover() {
            mgr.poll();
        }
    }

    // --- Step 8: the keyboard ----------------------------------------------

    /// Apply the frame's keyboard facts. Ctrl+K raises the one-shot search
    /// focus request; Space toggles playback. The draw half read both off
    /// egui's input, so this is where they become effects.
    fn read_keyboard(&mut self, input: &FrameInput) {
        apply_keyboard(
            input.search_focus_requested,
            input.toggle_playback,
            self.playback,
            self.parts.global_search_focus,
            self.parts.transport,
        );
    }

    // --- Step 9: the OS title and tray tooltip -----------------------------

    /// Push the window title and tray tooltip for the current track
    /// (REQ-SI-001). Both derive from one identity — the current `TrackId` —
    /// which is compared against the last push FIRST: steady-state frames send
    /// no viewport command and format nothing. The key exists to avoid
    /// repeating OS viewport commands, not for staleness; the current Track
    /// resolves through the Session Views seam over the store's `get_track`
    /// query — never the in-memory mirror.
    fn update_window_title(&mut self, out: &mut FrameOutput) {
        self.parts
            .views
            .sync_playback(&self.playback.queue, super::now_playing::UP_NEXT_LIMIT);
        let current_id = self.parts.views.playback_current().map(|t| &t.id);
        let unchanged = match &self.parts.title_key {
            TitleKey::Set(id) => id.as_ref() == current_id,
            TitleKey::Unset => false,
        };
        if unchanged {
            return;
        }

        // Cold path: the playing track moved — both strings are rebuilt and
        // pushed exactly once per identity change.
        let (tooltip, title) = match self.parts.views.playback_current() {
            Some(track) => {
                let tooltip = format!(
                    "{} - {}",
                    track.metadata.display_artist(),
                    track.metadata.display_title(&track.file_path)
                );
                let title = format!("{tooltip} \u{2014} riff");
                (tooltip, title)
            }
            None => ("riff".to_owned(), "riff".to_owned()),
        };
        out.viewport.push(FrameViewport::Title(title));
        out.tray_tooltip = Some(tooltip);
        *self.parts.title_key = TitleKey::Set(current_id.cloned());
    }

    // --- Step 10: the titlebar's actions -----------------------------------

    /// Apply one titlebar action. Close is resolved here and NOT by the
    /// pre-frame caller, because the Frame owns `library.ui_flags`: on
    /// macOS/Windows the custom X follows the persisted "Quit on close"
    /// preference — by default it hides through the frontend-local visibility
    /// channel (applied by `logic()` one frame later), and only when the
    /// preference is on does it send a real `Close`. On Linux there is no tray,
    /// so the X always really closes.
    ///
    /// This is NOT where a macOS native close is resolved, and it must not
    /// become one: the native branch's window controls are the system's
    /// traffic lights, and `chrome.rs` never calls `draw_caption_controls` on
    /// `ChromeMode::NativeTrafficLights` (the caption code is `cfg`'d out
    /// there), so [`TitleBarAction::Close`] is unreachable on macOS — the red
    /// button's close is resolved by [`Self::resolve_native_close`] instead.
    /// Unifying these two paths would reintroduce the tray-Quit cancellation
    /// bug, because this one resolves a `Close` through the preference alone
    /// and cannot see the quit intent.
    fn apply_titlebar_action(
        &mut self,
        action: TitleBarAction,
        maximized: bool,
        out: &mut FrameOutput,
    ) {
        use crate::ui::chrome::NavDestination;
        match action {
            TitleBarAction::ToggleTheme => self.parts.theme.dark = !self.parts.theme.dark,
            TitleBarAction::ToggleNowPlaying => {
                // Now Playing replaces the active view; leaving it returns to
                // the Library view (resolved navigation gap).
                self.library.view_mode = match self.library.view_mode {
                    ViewMode::Library | ViewMode::Settings => ViewMode::NowPlaying,
                    ViewMode::NowPlaying => ViewMode::Library,
                };
            }
            TitleBarAction::GoSettings => {
                NavDestination::Settings
                    .apply(&mut self.library.view_mode, &mut self.library.browse_mode);
            }
            TitleBarAction::Minimize => out.viewport.push(FrameViewport::Minimize),
            TitleBarAction::ToggleMaximize => {
                out.viewport.push(FrameViewport::Maximized(!maximized));
            }
            TitleBarAction::Close => {
                #[cfg(not(target_os = "linux"))]
                if self.library.ui_flags.close_quits_app {
                    out.viewport.push(FrameViewport::Close);
                } else {
                    out.hide_window = true;
                }
                #[cfg(target_os = "linux")]
                out.viewport.push(FrameViewport::Close);
            }
        }
    }

    // --- Step 11: the sidebar's actions ------------------------------------

    /// The shared "land on the Library view" reset: the Library view in
    /// Library browse mode, drill-down path and search filter cleared. Which
    /// open-list slot (smart list / playlist) a caller then fills — or
    /// clears — stays at the caller, since that is the one fact each row
    /// states differently.
    fn land_on_library(&mut self) {
        self.library.view_mode = ViewMode::Library;
        self.library.browse_mode = BrowseMode::Library;
        // Section (or browse-mode) navigation resets the drill-down path:
        // the new section starts at its root listing.
        self.library.reset_browser_path();
        self.library.search_query.clear();
    }

    /// Apply one sidebar action.
    fn apply_sidebar_action(&mut self, action: SidebarAction, out: &mut FrameOutput) {
        // On Linux no arm reports an output: the folder footer opens the Settings
        // text row through `parts` instead of the native dialog.
        #[cfg(target_os = "linux")]
        let _ = out;
        match action {
            SidebarAction::Navigate { section } => {
                self.land_on_library();
                self.library.library_section = section;
                *self.parts.smart_playlist_view = None;
                *self.parts.playlist_view = None;
            }
            SidebarAction::NavigateFolders => {
                self.land_on_library();
                self.library.browse_mode = BrowseMode::Folders;
                *self.parts.smart_playlist_view = None;
                *self.parts.playlist_view = None;
            }
            SidebarAction::ToggleSmartListsCollapsed => {
                // Folding the section away also closes any smart list it
                // opened: with the rows gone there is no other way back to
                // that view.
                self.library.ui_flags.smart_lists_collapsed =
                    !self.library.ui_flags.smart_lists_collapsed;
                *self.parts.smart_playlist_view = None;
            }
            SidebarAction::OpenSmartList(kind) => {
                self.land_on_library();
                *self.parts.smart_playlist_view = Some(kind);
                *self.parts.playlist_view = None;
            }
            SidebarAction::NewPlaylist => {
                *self.parts.playlist_create_name = Some(String::new());
                *self.parts.playlist_rename = None;
            }
            SidebarAction::PlaylistRow { id, action } => {
                super::app::apply_playlist_row_action(
                    action,
                    &id,
                    self.parts.playlist_store,
                    self.parts.views,
                    super::app::PlaylistPromptSlots {
                        view: self.parts.playlist_view,
                        smart_view: self.parts.smart_playlist_view,
                        rename: self.parts.playlist_rename,
                        create_name: self.parts.playlist_create_name,
                    },
                );
                // The sidebar is shared chrome on every view: opening a
                // playlist lands on the library view so its listing is on
                // screen, and clears any search filter over the results.
                if action == PlaylistRowAction::Open {
                    self.land_on_library();
                }
            }
            SidebarAction::PlaylistCreate(outcome) => {
                if outcome == PromptOutcome::Cancel {
                    *self.parts.playlist_create_name = None;
                    return;
                }
                let Some(name) = self.parts.playlist_create_name.take() else {
                    return;
                };
                let name = name.trim().to_string();
                if name.is_empty() {
                    return;
                }
                match self.parts.playlist_store.create_playlist(&name, &[]) {
                    Ok(id) => {
                        // The committed create bumps the playlist generation;
                        // the seam's next read lists the new playlist.
                        *self.parts.playlist_view = Some(id);
                    }
                    Err(e) => tracing::warn!("Failed to create playlist: {e}"),
                }
            }
            SidebarAction::PlaylistRename { outcome, .. } => {
                if outcome == PromptOutcome::Cancel {
                    *self.parts.playlist_rename = None;
                    return;
                }
                let Some((rid, draft)) = self.parts.playlist_rename.take() else {
                    return;
                };
                // Same Store flow as before the restyle: trim, rename as one
                // durable transaction. The seam's next read reflects the new
                // name on its own (ADR 0002). The prompt slot is keyed by id,
                // so the slot the row reported IS the slot taken here.
                super::app::commit_playlist_rename(self.parts.playlist_store, &rid, &draft);
            }
            SidebarAction::AddFolderRequested => {
                // The native folder picker is an OS dialog, so it is performed
                // by the draw half (`FrameOutput::pick_folder`) rather than
                // here. Linux has no dialog: the flow is a text row beneath the
                // Settings stage, so the footer lands on the Settings view
                // BEFORE the stage is drawn — which is why this is an apply
                // slot and not an end-of-frame note.
                #[cfg(target_os = "linux")]
                {
                    crate::ui::chrome::NavDestination::Settings
                        .apply(&mut self.library.view_mode, &mut self.library.browse_mode);
                    *self.parts.settings_show_input = true;
                    *self.parts.settings_path_error = None;
                }
                #[cfg(not(target_os = "linux"))]
                {
                    out.pick_folder = true;
                }
            }
        }
    }

    // --- Steps 12 and 13: the bar's and the stage's appliers ----------------
    //
    // The two appliers below are free functions rather than methods because
    // the Frame reaches them through its ordered slots and the test suite has
    // asserted them directly since before the Frame existed; both paths go
    // through the same one, so there is nothing to drift.
}

/// Apply one restyled player-bar action (Issue 08) through the SAME engine
/// intents and state paths the pre-restyle controls used. Transport actions
/// pass straight through to the Transport port; the port's mutators complete
/// the intent on the session themselves — `set_volume` clamps and stores the
/// slider value, `toggle_mute` flips the flag, `toggle_shuffle`/
/// `toggle_repeat` flip the queue state — and send the engine exactly what it
/// needs, so a muted app never emits sound. Seek targets re-clamp against the
/// live track duration inside the adapter. Preference changes are session
/// writes only — the frame-end `Preferences` commit persists them.
pub fn apply_player_bar_action(
    action: PlayerBarAction,
    library: &mut LibrarySession,
    playback: &mut PlaybackSession,
    transport: &dyn Transport,
) {
    use crate::ui::playerbar::PlayerBarAction as Action;
    match action {
        Action::Previous => transport.previous(),
        Action::Pause => transport.pause(),
        Action::Resume => transport.resume(),
        Action::PlaySelected => {
            // Pre-restyle behavior: with nothing selected, play does nothing.
            if let Some(selected) = library.selected_track.clone() {
                transport.play(selected);
            }
        }
        Action::Next => transport.next(),
        Action::Stop => transport.stop(),
        Action::Seek(target) => {
            transport.seek(playback, target.as_secs_f32());
        }
        Action::SetVolume(volume) => {
            // While muted the slider still edits current_volume, but the engine
            // keeps receiving 0 until unmuted.
            transport.set_volume(playback, volume);
        }
        Action::ToggleMute => {
            // Muting never moves the volume slider — it only zeroes the
            // effective volume sent to the engine; unmuting restores it.
            transport.toggle_mute(playback);
        }
        Action::ToggleShuffle => transport.toggle_shuffle(playback),
        Action::ToggleRepeat => transport.toggle_repeat(playback),
        Action::ToggleQueue => {
            // The queue panel is session state, not persisted (issue 13).
            library.queue_open = !library.queue_open;
        }
        Action::ToggleExpanded => {
            // The enlarged player view IS the Now Playing mode (issue 13):
            // same routing as the titlebar's Now Playing toggle, and purely view
            // state — playback keeps running untouched.
            library.view_mode = match library.view_mode {
                ViewMode::Library | ViewMode::Settings => ViewMode::NowPlaying,
                ViewMode::NowPlaying => ViewMode::Library,
            };
        }
        Action::PlayNext(track_id) => transport.play_next(track_id),
    }
}

/// Apply one [`NowPlayingAction`](super::now_playing::NowPlayingAction)
/// (Issue 10). Close ALWAYS lands on the Library View: Now Playing is a mode
/// that replaces the active View (resolved navigation gaps), so there is no
/// prior view to restore — closing from anywhere returns to the Library.
/// Transport actions pass straight through to the Transport port; seek targets
/// re-clamp against the live track duration exactly like the playerbar's.
pub fn apply_now_playing_action(
    action: NowPlayingAction,
    library: &mut LibrarySession,
    playback: &PlaybackSession,
    transport: &dyn Transport,
) {
    match action {
        NowPlayingAction::Close => library.view_mode = ViewMode::Library,
        NowPlayingAction::PlayNext(track_id) => transport.play_next(track_id),
        NowPlayingAction::Seek(duration) => {
            transport.seek(playback, duration.as_secs_f32());
        }
    }
}

/// What the titlebar panel reports, in the panel's own order.
///
/// The search query rides back rather than being written in place: the field is
/// a widget over a `String`, and the session slot it edits is the Frame's to
/// write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitlebarReport {
    /// The controls the user fired, in paint order.
    pub actions: Vec<TitleBarAction>,
    /// The search field's contents after the frame.
    pub search_query: String,
    /// Whether Ctrl+K was standing when the panel ran, so the panel focused
    /// the field. The Frame consumes it.
    pub focus_search: bool,
    /// Whether the viewport reported itself maximized, for `ToggleMaximize`.
    pub maximized: bool,
}

/// What one sidebar row decided.
///
/// Every variant is a fact the row already knows, so the Frame never has to be
/// told which row it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarAction {
    /// A LIBRARY row: land on the Library view in `section`, closing any
    /// opened list and clearing the search.
    Navigate {
        /// The section whose row was clicked.
        section: LibrarySection,
    },
    /// The Folders row: land on the Library view in the Folders browse mode —
    /// the one LIBRARY row that switches browse mode instead of section.
    NavigateFolders,
    /// The SMART LISTS section header's chevron: fold the section away or open
    /// it again.
    ToggleSmartListsCollapsed,
    /// A smart-list row: open it over the library view.
    OpenSmartList(SmartPlaylistKind),
    /// The Playlists header's "+": open the empty name prompt.
    NewPlaylist,
    /// A playlist row's own hover-revealed action, through the store flows.
    PlaylistRow {
        /// The row's playlist.
        id: PlaylistId,
        /// What the row's control reported.
        action: PlaylistRowAction,
    },
    /// The inline "New Playlist" prompt resolved.
    PlaylistCreate(PromptOutcome),
    /// The inline rename prompt resolved (the slot is keyed by playlist id, so
    /// the reported id and the slot's own are the same fact).
    PlaylistRename {
        /// The prompt's playlist.
        id: PlaylistId,
        /// What the prompt reported.
        outcome: PromptOutcome,
    },
    /// The footer's Add-folder control.
    ///
    /// Reported rather than performed because the native folder picker is an OS
    /// dialog, not a decision: the Frame records that one was asked for and the
    /// draw half opens it. On Linux there is no dialog — the flow is a text row
    /// beneath the Settings stage — so the Frame routes to the Settings view
    /// itself, and this frame's stage already draws the row.
    AddFolderRequested,
}

/// Everything the sidebar panel reported this frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SidebarReport {
    /// The rows' decisions, in paint order.
    pub actions: Vec<SidebarAction>,
}

impl SidebarReport {
    /// Record one row's decision.
    pub fn push(&mut self, action: SidebarAction) {
        self.actions.push(action);
    }
}

/// Everything the control-bar panel reported this frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ControlBarReport {
    /// The bar's and queue panel's actions, in paint order.
    pub actions: Vec<PlayerBarAction>,
}

/// Everything the main stage reported this frame.
///
/// The Library and Settings stages report through their own appliers (they are
/// Views, not shell panels, and the Settings modal owns watcher and store
/// effects the Frame has no business knowing); only the Now Playing stage's
/// actions travel back this way.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StageReport {
    /// The Now Playing stage's actions, in paint order.
    pub actions: Vec<NowPlayingAction>,
}

/// Global keyboard shortcuts: Ctrl+K focuses the global search (issue 06),
/// and Space toggles playback. The one applier the Frame's step 8 calls, so
/// the shortcut contract has exactly one behaviour.
pub fn apply_keyboard(
    search_focus_requested: bool,
    toggle_playback: bool,
    playback: &PlaybackSession,
    global_search_focus: &mut bool,
    transport: &dyn Transport,
) {
    if search_focus_requested {
        *global_search_focus = true;
    }
    if toggle_playback {
        if playback.playback_state == riff_backend::domain::PlaybackState::Playing {
            transport.pause();
        } else {
            transport.resume();
        }
    }
}

/// Apply drained backend events to the structured feedback board (issue 11).
/// Playback errors arrive as typed notices stamped with playback source and
/// error severity; each is folded into its source's persistent slot so it
/// survives alongside — not overwritten by — Library Scan progress. Other event
/// kinds carry no UI feedback yet.
pub fn apply_backend_events(
    events: Vec<riff_backend::app::events::BackendEvent>,
    feedback: &mut FeedbackBoard,
) {
    use riff_backend::app::events::BackendEvent;
    for event in events {
        if let BackendEvent::TypedNotice(payload) = event {
            feedback.put(Feedback::from_notice(&payload, None));
        }
    }
}

/// Ask the Cover worker for `track`'s Cover at `size`, through the Cover
/// Cache — the one place that knows whether the request is due at all.
pub fn request_cover(
    cache: &mut CoverCache,
    covers: &dyn Covers,
    track_id: &riff_backend::domain::TrackId,
    file_path: &std::path::Path,
    size: RequestedSize,
) {
    cache.want_track(covers, track_id.clone(), file_path.to_path_buf(), size);
}

/// The one wording a settled `ReplayGain` Pass gets. Shared because the same
/// report is read in two places — the status line the frame writes on the
/// drain, and the Settings card's inline outcome line — and a report that
/// described itself differently to each would make the two disagree.
pub(crate) fn pass_outcome_line(report: &PassReport) -> String {
    let PassReport {
        measured,
        skipped,
        failed,
        first_failure,
        cancelled,
    } = report;
    if *failed > 0 {
        format!(
            "ReplayGain pass finished: {measured} measured, {failed} failed \u{2014} {}",
            first_failure.as_deref().unwrap_or("unknown reason")
        )
    } else if *cancelled {
        format!("ReplayGain pass stopped: {measured} tracks measured so far")
    } else if *skipped > 0 {
        format!("ReplayGain measured: {measured} tracks ({skipped} already measured)")
    } else {
        format!("ReplayGain measured: {measured} tracks")
    }
}
