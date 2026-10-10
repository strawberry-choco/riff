use crate::ui::button::{self as button, TextButton, Variant};
use crate::ui::icons::{Icon, IconCache};
use crate::ui::now_playing::styled_font;
use crate::ui::theme::geometry::settings::WATCH_BOX;
use crate::ui::theme::geometry::settings::{
    ACTION_BTN_H, ACTIONS_ROW_GAP, ACTIONS_ROW_H, CARD_BORDER_W, CHIP_GAP, CHIP_H, CHIP_LABEL_PAD,
    CHIP_ROW_NO_WRAP_W, COLUMN_GAP, DOT_SIZE, FOOTER_ACTION_GAP, FOOTER_H, HEADER_GAP,
    LIBRARY_ROW_H, LIBRARY_ROW_STACK_W, LIBRARY_ROW_TEXT_GAP, MIN_TWO_COL_W, NAV_GAP,
    NAV_HAIRLINE_W, NAV_ITEM_H, NAV_TOP_INSET, NAV_W, PAGE_HEADER_H, PAGE_PAD, PANE_PAD,
    PREF_ROW_H, PREF_ROW_PAD, PREF_ROW_TEXT_GAP, PREF_ROW_TEXT_INSET, READINESS_GAP, ROW_BTN_PAD,
    SCAN_CARD_H, SCAN_CARD_PAD, SECTION_GAP, SMALL_BTN_H, SMALL_BTN_LABEL_PAD, TRASH_BTN,
};
use crate::ui::theme::{self, Palette};
use eframe::egui;
use riff_backend::app::MutexExt;
use riff_backend::app::replaygain_pass::PassCommand;
use riff_backend::app::state::{
    LibrarySession, LibraryStatus, PlaybackSession, ReplayGainMode, ViewMode, WatchState,
};
use riff_backend::app::store::{AUDIO_EXTENSIONS, FullScanSummary};
use std::path::PathBuf;
use std::sync::Arc;

/// How the Library pane's lower four sections are arranged at a given width.
///
/// A layout decision only. It never reorders anything: both branches walk the
/// sections in the same reading order (Preferences, Formats, Last Full Scan,
/// Artwork), and the two-column branch simply places Formats beside Last Full
/// Scan instead of below it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LibraryPaneColumns {
    /// One stacked column at the full available width — the narrow fallback.
    Stacked {
        /// The column's width, equal to the available width.
        width: f32,
    },
    /// Two balanced columns separated by [`COLUMN_GAP`].
    TwoColumns {
        /// The left column's width.
        left: f32,
        /// The right column's width, equal to `left`.
        right: f32,
    },
}

/// The Library pane's column arrangement for an `available_width` content
/// column.
///
/// The branch is [`MIN_TWO_COL_W`]: at or above it the lower sections split,
/// below it they stack. The boundary is inclusive — the token is the *minimum*
/// width at which the pane is allowed to split, so exactly [`MIN_TWO_COL_W`]
/// splits.
///
/// Columns are balanced: each takes half the available width less half the
/// gap, so the two plus the gap are exactly the available width. Pure, so the
/// branch and the balance are assertable without pixels; the view allocates the
/// rects this returns inside a scoped builder, the same idiom the elastic stage
/// uses for its own columns.
#[must_use]
pub fn settings_pane_columns(available_width: f32) -> LibraryPaneColumns {
    if available_width >= MIN_TWO_COL_W {
        let column = (available_width - COLUMN_GAP) / 2.0;
        LibraryPaneColumns::TwoColumns {
            left: column,
            right: column,
        }
    } else {
        LibraryPaneColumns::Stacked {
            width: available_width,
        }
    }
}

/// The Settings page frame's rectangles, derived from the stage rect the page
/// is handed.
///
/// Split out of [`show_settings_modal`] so the frame's geometry is a pure
/// function of the stage: the view draws from these rects and the UI tests
/// assert the same function, so the two cannot disagree about whether the page
/// fills the stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SettingsPageRects {
    /// The whole page: the stage inset by [`PAGE_PAD`] on every side.
    pub page: egui::Rect,
    /// The header row — "Settings" and Back — spanning the page's full width.
    pub header: egui::Rect,
    /// What is left between the header and the footer: the fill that pins the
    /// footer to the page's bottom edge.
    pub body: egui::Rect,
    /// The left nav column, [`NAV_W`] wide and the body's full height.
    pub nav: egui::Rect,
    /// The current section's content pane, right of the nav's hairline by
    /// [`NAV_GAP`].
    pub pane: egui::Rect,
    /// The footer row, spanning the page's full width and pinned to its bottom.
    pub footer: egui::Rect,
}

/// The Settings page's frame for a `available` stage rect.
///
/// The page fills the stage: the stage inset by [`PAGE_PAD`] on every side,
/// with no width cap and no centring. Inside it the header takes the top strip
/// at full width, the footer takes the bottom strip at full width, and the body
/// — the fill between them — is what pins the footer down.
#[must_use]
pub fn settings_page_rects(available: egui::Rect) -> SettingsPageRects {
    let page = available.shrink(PAGE_PAD);
    let header = egui::Rect::from_min_max(
        page.left_top(),
        egui::pos2(page.right(), page.top() + PAGE_HEADER_H),
    );
    let footer = egui::Rect::from_min_max(
        egui::pos2(page.left(), page.bottom() - FOOTER_H),
        page.right_bottom(),
    );
    let body = egui::Rect::from_min_max(
        egui::pos2(page.left(), header.bottom()),
        egui::pos2(page.right(), footer.top()),
    );
    let nav = egui::Rect::from_min_max(
        body.left_top(),
        egui::pos2(body.left() + NAV_W, body.bottom()),
    );
    // Named `content` rather than `pane` only because clippy's `similar_names`
    // reads `pane`/`page` as one character apart; the rect is the pane.
    let content = egui::Rect::from_min_max(
        egui::pos2(nav.right() + NAV_HAIRLINE_W + NAV_GAP, body.top()),
        body.right_bottom(),
    );
    SettingsPageRects {
        page,
        header,
        body,
        nav,
        pane: content,
        footer,
    }
}

/// Expand a leading `~/` (or a bare `~`) in `input` against the `HOME`
/// environment variable. Returns the path unchanged when there is no leading
/// `~` or when `HOME` is unset.
pub fn expand_tilde(input: &str) -> PathBuf {
    if input == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = input.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(input)
}

/// Return up to `max` existing subdirectories matching the typed partial
/// path, for directory autocomplete in the Linux folder picker.
///
/// Resolves the parent directory of `input` plus its partial last segment and
/// lists children of the parent whose names start with that segment
/// (case-insensitive). When `input` is empty, `~`, or ends with a separator,
/// the children of that directory itself are listed (empty input completes
/// against the current directory). Results are sorted, deduped, and capped.
/// Non-existent or unreadable parents yield an empty list (never a panic).
pub fn suggest_directories(input: &str, max: usize) -> Vec<PathBuf> {
    let expanded = expand_tilde(input);

    // Which directory to list, and which prefix its children must match.
    let (parent, partial) = if input.is_empty() {
        (PathBuf::from("."), String::new())
    } else if input == "~" || input.ends_with(['/', '\\']) {
        (expanded.clone(), String::new())
    } else {
        let partial = expanded
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        match expanded.parent() {
            // A relative single segment (e.g. "Mus") has an empty parent;
            // complete it against the current directory instead.
            Some(parent) if !parent.as_os_str().is_empty() => (parent.to_path_buf(), partial),
            _ => (PathBuf::from("."), partial),
        }
    };

    if !parent.is_dir() {
        return Vec::new();
    }

    let partial_lower = partial.to_lowercase();
    let mut matches: Vec<PathBuf> = match std::fs::read_dir(&parent) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .to_lowercase()
                        .starts_with(&partial_lower)
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    matches.sort();
    matches.dedup();
    matches.truncate(max);
    matches
}

// --- Readiness (CONTEXT.md): per-path health, independent of Watch State -------

/// The per-Library-Path health the status dot renders: whether the path is
/// present on disk and indexed into the Library. Deliberately carries no
/// watcher information — that is [`WatchState`]'s job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Present on disk and indexed into the Library.
    Ready,
    /// A scan is walking this root right now.
    Scanning,
    /// Present on disk but nothing indexed under it yet.
    NotIndexed,
    /// The path is gone from disk.
    Missing,
}

/// Derive one root's [`Readiness`] from its scan status plus how many tracks
/// the Library currently indexes under it. The count (not just the session
/// scan status) is what makes stores hydrated at startup read as Ready
/// before any scan has run this session.
#[must_use]
pub fn readiness(status: &LibraryStatus, indexed_tracks: usize) -> Readiness {
    match status {
        LibraryStatus::Unavailable => Readiness::Missing,
        LibraryStatus::Scanning { .. } => Readiness::Scanning,
        LibraryStatus::Scanned(n) if *n > 0 => Readiness::Ready,
        _ if indexed_tracks > 0 => Readiness::Ready,
        _ => Readiness::NotIndexed,
    }
}

impl Readiness {
    /// The status-dot fill, straight from the palette's status tokens — the
    /// mockup's `var(--riff-state-success)` dot and its siblings.
    #[must_use]
    pub fn dot_color(self, palette: &Palette) -> egui::Color32 {
        match self {
            Self::Ready => palette.success,
            Self::Scanning => palette.info,
            Self::NotIndexed => palette.warning,
            Self::Missing => palette.error,
        }
    }

    /// The muted label beside the dot.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Scanning => "Scanning",
            Self::NotIndexed => "Not indexed",
            Self::Missing => "Missing",
        }
    }
}

// --- Sectioned modal (design-handoff issue 11) ---------------------------------

/// One Settings section selectable from the modal's left nav. `ALL` is the
/// mockup's nav order and drives focus order, so keep it authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsSection {
    Library,
    Playback,
    Appearance,
    Advanced,
    About,
}

impl SettingsSection {
    /// Every section in left-nav (and focus) order.
    pub const ALL: [SettingsSection; 5] = [
        Self::Library,
        Self::Playback,
        Self::Appearance,
        Self::Advanced,
        Self::About,
    ];

    /// The nav label, verbatim from the mockup.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Library => "Library",
            Self::Playback => "Playback",
            Self::Appearance => "Appearance",
            Self::Advanced => "Advanced",
            Self::About => "About",
        }
    }
}

// --- Mockup copy -----------------------------------------------------------------

/// Section heading, verbatim from the mockup (uppercased for display; egui
/// has no letter-spacing, so the muted ink carries the hierarchy).
pub const SECTION_LIBRARIES: &str = "MUSIC LIBRARIES";
/// Section heading, verbatim from the mockup.
pub const SECTION_ADVANCED_INFO: &str = "ADVANCED & PLATFORM INFO";

/// The destructive ghost button's label. CONTEXT.md retires the mockup's
/// "Clear Library Cache" wording; the action is "Clear Library".
pub const CLEAR_LIBRARY_LABEL: &str = "Clear Library";
/// The muted note beside the destructive ghost button (mockup structure,
/// glossary language).
/// Preference row copy: `(title, description)` verbatim from the mockup.
pub const PREF_ADVANCED: (&str, &str) = (
    "Advanced mode",
    "Expose extra metadata fields and per-track actions.",
);
/// Preference row copy verbatim from the mockup.
pub const PREF_HIGH_CONTRAST: (&str, &str) = (
    "High contrast",
    "Increase contrast for text and focus outlines.",
);
/// Preference row copy for the Reduce Motion toggle, the temporal sibling of
/// High Contrast in the Appearance pane.
pub const PREF_REDUCE_MOTION: (&str, &str) = (
    "Reduce motion",
    "Skip animations and show state changes immediately.",
);
/// Preference row copy verbatim from the mockup.
pub const PREF_REPLAYGAIN: (&str, &str) = (
    "ReplayGain",
    "Normalize loudness across tracks when available.",
);
/// Playback pane copy for the `ReplayGain Mode` row: which of the two pairs
/// playback levels at.
pub const PREF_REPLAYGAIN_MODE: (&str, &str) = (
    "ReplayGain Mode",
    "Which values a Track plays at: its own, or its Album's.",
);

/// Playback pane section heading for the `ReplayGain` Mode choice.
pub const SECTION_REPLAYGAIN_MODE: &str = "REPLAYGAIN MODE";
/// Advanced pane section heading for the library-wide `ReplayGain` Pass.
pub const SECTION_REPLAYGAIN_PASS: &str = "REPLAYGAIN PASS";

/// Library pane section headings (design-handoff issue 12), uppercased for
/// display like [`SECTION_LIBRARIES`].
pub const SECTION_FORMATS: &str = "INDEXED FORMATS";
/// Library pane section heading.
pub const SECTION_SCAN_STATUS: &str = "LAST FULL SCAN";
/// Library pane section heading.
pub const SECTION_ARTWORK: &str = "ARTWORK";

/// Library pane preference copy: `(title, description)`.
pub const PREF_WATCH_CHANGES: (&str, &str) = (
    "Watch for changes",
    "Update the Library automatically when files change on disk.",
);
/// Library pane preference copy.
pub const PREF_SKIP_HIDDEN: (&str, &str) = (
    "Skip hidden files",
    "Leave dot-prefixed files and folders out of scans.",
);
/// Library pane preference copy.
pub const PREF_READ_EMBEDDED: (&str, &str) = (
    "Read embedded artwork",
    "Use artwork stored in track tags before folder images.",
);
/// Advanced pane preference copy: the custom title-bar close button's
/// behavior. Offered only where a tray exists (macOS/Windows); Linux has no
/// tray, so closing always quits there.
pub const PREF_CLOSE_QUITS: (&str, &str) = (
    "Quit on close",
    "Closing the window quits the app instead of minimizing it to the tray.",
);

/// The Library pane footer's note.
pub const FOOTER_NOTE: &str = "Changes apply immediately";
/// The Library pane footer's closing action.
pub const DONE_LABEL: &str = "Done";
/// The Last Full Scan card's action label, named so the card lays its text
/// column out against a width it measures rather than one it assumes.
pub const RESCAN_LABEL: &str = "Rescan now";

// --- Stage content & actions -------------------------------------------------------

/// One library-path row as the stage renders it: identity, scan status, the
/// persisted watcher choice, and how many tracks the Library indexes under
/// the root. Readiness derives from `status` + `indexed_tracks` — never from
/// `watch`.
#[derive(Debug, Clone)]
pub struct LibraryRow {
    /// The library root.
    pub path: PathBuf,
    /// Scan/status snapshot for this root.
    pub status: LibraryStatus,
    /// Persisted watcher choice for this root.
    pub watch: WatchState,
    /// Tracks the Library currently indexes under [`Self::path`] (covers
    /// stores hydrated at startup before any session scan ran).
    pub indexed_tracks: usize,
}

impl LibraryRow {
    /// This row's [`Readiness`]: present on disk + indexed, independent of
    /// [`Self::watch`].
    #[must_use]
    pub fn readiness(&self) -> Readiness {
        readiness(&self.status, self.indexed_tracks)
    }
}

/// Everything the Settings stage needs to render one frame. A plain value
/// struct: the caller reads it out of the session, the widgets never touch
/// state.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each persisted preference is an independent toggle"
)]
#[derive(Default)]
pub struct SettingsContent {
    /// One row per configured library root, in display order.
    pub libraries: Vec<LibraryRow>,
    /// Advanced mode preference (drives the first toggle).
    pub advanced_mode: bool,
    /// High contrast preference (drives the second toggle).
    pub high_contrast: bool,
    /// Reduce Motion preference (drives the Appearance pane's third toggle).
    /// The temporal sibling of `high_contrast`: not a colour, so it never
    /// reaches the resolved palette — it selects the style's tempo at the theme
    /// boundary and the rest shape the now-playing equalizer holds.
    pub reduce_motion: bool,
    /// `ReplayGain` preference (drives the third toggle).
    pub replaygain_enabled: bool,
    /// Which `ReplayGain` pair playback levels at — the Mode card's current
    /// choice, beside the toggle that turns the whole feature off.
    pub replaygain_mode: ReplayGainMode,
    /// The pane-level "Watch for changes" toggle: `true` when at least one
    /// root is being watched. Turning it off stops every watcher; turning
    /// it on starts one per root.
    pub watch_any: bool,
    /// The Library Scan preferences (design-handoff issue 12).
    pub skip_hidden_files: bool,
    /// The enabled audio extensions — the format chips' on/off state.
    pub scan_formats: Vec<String>,
    /// Whether embedded artwork is read before filesystem fallbacks.
    pub read_embedded_artwork: bool,
    /// Whether the title-bar close button quits the app instead of minimizing
    /// to the tray (Advanced pane; rendered only where a tray exists).
    pub close_quits_app: bool,
    /// The library-wide `ReplayGain` Pass's Settings gating: which value kinds
    /// a pass writes (persisted) and whether Force is chosen for this run
    /// (session-local, never persisted — an automatic pass never redoes
    /// finished work).
    pub pass_track: bool,
    pub pass_album: bool,
    pub pass_force: bool,
    /// Whether a `ReplayGain` Pass is running on its worker right now: the
    /// button is disabled while it is, so a second pass cannot start.
    pub pass_running: bool,
    /// The running pass's `(done, total)` measurement progress.
    pub pass_progress: (usize, usize),
    /// The last settled pass's outcome line, `None` until one has run.
    pub pass_outcome: Option<String>,
    /// The last completed full scan's summary, `None` when never scanned.
    pub last_scan: Option<FullScanSummary>,
}

/// What the user did to the Settings stage this frame. The app applies these
/// through its state/command/store paths so every effect stays testable
/// headlessly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    /// Leave the Settings View for the Library.
    Back,
    /// Show the given section in the modal's right pane.
    SelectSection(SettingsSection),
    /// Open the platform folder picker to register a new root.
    AddLibrary,
    /// Scan every configured root.
    ScanAll,
    /// Scan one root.
    Scan(PathBuf),
    /// Remove a root (and its indexed tracks) from the app.
    Remove(PathBuf),
    /// Turn the filesystem watcher for a root on or off.
    SetWatch(PathBuf, bool),
    /// Wipe the indexed collection (playlists and settings kept).
    ClearLibrary,
    /// Delete every cached cover Thumbnail. The only reclaim the Thumbnail cache
    /// has, so it asks first — and it touches nothing the listener made.
    ClearThumbnailCache,
    /// Set the Advanced mode preference.
    SetAdvanced(bool),
    /// Set the High contrast preference.
    SetHighContrast(bool),
    /// Set the Reduce Motion preference (the temporal axis of the theme).
    SetReduceMotion(bool),
    /// Set the `ReplayGain` preference.
    SetReplayGain(bool),
    /// Choose which `ReplayGain` pair playback levels at.
    SetReplayGainMode(ReplayGainMode),
    /// Gate the library-wide `ReplayGain` Pass's value kinds (persisted).
    SetReplayGainPassTrack(bool),
    SetReplayGainPassAlbum(bool),
    /// Choose Force for the next pass (session-local, never persisted).
    SetReplayGainPassForce(bool),
    /// Start one library-wide `ReplayGain` Pass, gated by the checkboxes.
    StartReplayGainPass,
    /// Ask the running pass to stop; everything committed stays.
    CancelReplayGainPass,
    /// Start or stop watching every configured root (the pane's
    /// "Watch for changes" toggle).
    SetWatchAll(bool),
    /// Set the "Skip hidden files" scan preference.
    SetSkipHidden(bool),
    /// Enable or disable one audio format's indexing (the format chips).
    SetFormat(String, bool),
    /// Set the "Read embedded artwork" preference.
    SetReadEmbeddedArtwork(bool),
    /// Set whether the title-bar close button quits instead of minimizing to
    /// the tray.
    SetCloseQuitsApp(bool),
}

/// Full-texture UV rect for [`egui::Painter::image`] (sidebar precedent).
const UV_FULL: egui::Rect = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));

// --- Pure helpers ------------------------------------------------------------------

/// Clip a path string for display, keeping the tail (the distinguishing
/// segment) rather than the head. The cut advances forward to the nearest
/// UTF-8 char boundary: `path.len()` counts bytes, and a multi-byte
/// character straddling the cut would otherwise panic the render loop.
#[must_use]
pub fn truncate_path(path: &str, max_len: usize) -> String {
    if path.len() <= max_len {
        path.to_string()
    } else {
        let mut start = path.len().saturating_sub(max_len.saturating_sub(3));
        while !path.is_char_boundary(start) {
            start += 1;
        }
        format!("...{}", &path[start..])
    }
}

/// A muted uppercase section header at the mockup's `text-sm font-semibold`.
fn section_header(ui: &mut egui::Ui, palette: &Palette, text: &str) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), theme::TEXT_SM),
        egui::Sense::hover(),
    );
    ui.painter_at(rect).text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        styled_font(ui, egui::TextStyle::Heading, theme::TEXT_SM),
        palette.ink_3,
    );
}

/// A shared semantic text button, painted through [`button::text_button`]. The
/// `variant` names the tier the button belongs to — `Primary`, the
/// brand-gradient action; `Accent`, the brand-washed action; or the neutral
/// `Secondary` — because the three are genuinely different roles and a
/// `primary: bool` could only ever say two. `small_text` picks the `text-xs`
/// scale. `a11y` feeds the accessibility tree (per-path controls suffix their
/// root so labels stay unique). Returns whether the button was clicked AND
/// enabled.
#[allow(clippy::too_many_arguments)]
fn filled_button(
    ui: &egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    rect: egui::Rect,
    id: egui::Id,
    label: &str,
    a11y: &str,
    icon: Option<Icon>,
    variant: Variant,
    small_text: bool,
    enabled: bool,
) -> bool {
    button::text_button(
        ui,
        cache,
        palette,
        reduce_motion,
        &TextButton {
            id,
            rect,
            label,
            a11y,
            tooltip: None,
            icon,
            small: small_text,
            variant,
            enabled,
        },
    )
}

/// A hairline separator across the card width (the mockup's
/// `border-b border-border` / `h-px bg-border`).
fn row_separator(ui: &mut egui::Ui, palette: &Palette, inset: f32) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter_at(rect).rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.left() + inset, rect.top()),
            egui::pos2(rect.right() - inset, rect.bottom()),
        ),
        0.0,
        palette.border,
    );
}

/// The Music Libraries card: one readiness row per root plus the Add Library
/// / Scan All actions row.
/// The settings card: the shared surface fill/border/radius, with an
/// optional inner margin, wrapping `body`.
fn settings_card<R>(
    ui: &mut egui::Ui,
    palette: &Palette,
    inner_margin: Option<i8>,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut frame = egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(CARD_BORDER_W, palette.border))
        .corner_radius(theme::RADIUS_LG);
    if let Some(margin) = inner_margin {
        frame = frame.inner_margin(egui::Margin::same(margin));
    }
    frame.show(ui, body)
}

fn libraries_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    settings_card(ui, palette, None, |ui| {
        if content.libraries.is_empty() {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), LIBRARY_ROW_H),
                egui::Sense::hover(),
            );
            ui.painter_at(rect).text(
                egui::pos2(rect.left() + 16.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                "No music libraries configured. Add one to get started.",
                styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
                palette.ink_3,
            );
            row_separator(ui, palette, 0.0);
        }
        for row in &content.libraries {
            library_row(ui, cache, palette, content.reduce_motion, row, actions);
            row_separator(ui, palette, 0.0);
        }
        actions_row(ui, cache, palette, content.reduce_motion, actions);
    });
}

/// The width a preference row's text column has, at `available_width`: the row
/// less its own padding, less the toggle, less the gap between them.
///
/// Pure, so "the description is wrapped to the column rather than truncated at
/// the row's edge" is assertable without pixels.
#[must_use]
pub fn preference_text_width(available_width: f32) -> f32 {
    (available_width
        - PREF_ROW_PAD
        - PREF_ROW_PAD
        - theme::geometry::toggle::TOGGLE_W
        - PREF_ROW_TEXT_GAP)
        .max(0.0)
}

/// The height a preference row takes: its fixed height inline, or whatever the
/// wrapped description needs when it stacks.
///
/// Never below [`PREF_ROW_H`], so a short description at a narrow width still
/// gets the same row it gets everywhere else.
#[must_use]
pub fn preference_row_height(flow: RowFlow, title_h: f32, wrapped_desc_h: f32) -> f32 {
    match flow {
        RowFlow::Inline => PREF_ROW_H,
        RowFlow::Stacked => (PREF_ROW_TEXT_INSET
            + title_h
            + PREF_ROW_TEXT_GAP
            + wrapped_desc_h
            + PREF_ROW_TEXT_INSET)
            .max(PREF_ROW_H),
    }
}

/// Shorten `text` until its galley fits `max_width`, dropping one character
/// at a time.
///
/// Measurement-driven, so it needs no character-width constant: the budget is
/// whatever the line actually has left after the icon, the gap and the track
/// count. Returns the original string unchanged if it already fits.
fn fit_to_width(
    painter: &egui::Painter,
    font: &egui::FontId,
    text: &str,
    max_width: f32,
    color: egui::Color32,
) -> String {
    let mut candidate = text.to_owned();
    if painter
        .layout_no_wrap(candidate.clone(), font.clone(), color)
        .size()
        .x
        <= max_width
    {
        return candidate;
    }
    while !candidate.is_empty() {
        candidate.pop();
        if painter
            .layout_no_wrap(candidate.clone(), font.clone(), color)
            .size()
            .x
            <= max_width
        {
            break;
        }
    }
    candidate
}

/// Whether a row lays its content out on one line or stacks it, at
/// `available_width`.
///
/// The one-line form puts a fixed control cluster on the right and the path on
/// the left; when the two cannot both be legible the row stacks instead — path
/// on the first line, readiness and controls on the second — and grows. Pure,
/// so "the reflow is a no-op at the pinned widths" is assertable without
/// pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowFlow {
    /// One line: cluster right, path left, at today's geometry.
    Inline,
    /// Two lines: path on top, readiness and controls below.
    Stacked,
}

/// The flow a row of `available_width` takes.
///
/// Derived from [`LIBRARY_ROW_STACK_W`] rather than hardcoded, and the token's
/// own value is derived from the measured control cluster plus a legible path
/// (see its doc comment). One token serves every row component because their
/// validated-width bands overlap: the narrowest already-pinned row is 461.5px
/// (a preference row in the two-column Library layout) and the widest
/// unvalidated one is 205px (the min-stage pane), and 440 sits between.
#[must_use]
pub fn row_flow(available_width: f32) -> RowFlow {
    if available_width < LIBRARY_ROW_STACK_W {
        RowFlow::Stacked
    } else {
        RowFlow::Inline
    }
}

/// The height a row takes for `flow`.
///
/// Derived from existing tokens rather than a new constant: the stacked form is
/// one extra line of content beside the row's own height, and the tallest thing
/// on that line is a small button.
#[must_use]
pub fn row_height(flow: RowFlow) -> f32 {
    match flow {
        RowFlow::Inline => LIBRARY_ROW_H,
        RowFlow::Stacked => LIBRARY_ROW_H + SMALL_BTN_H + SMALL_BTN_H,
    }
}

/// How the format chips lay out in a content column of `available_width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipFlow {
    /// Every chip on one line — what every pinned wide width still shows.
    OneLine,
    /// Chips wrap onto as many lines as they need, and the card grows.
    Wrapped,
}

/// The flow a format-chip row of `available_width` takes.
///
/// Derived from [`CHIP_ROW_NO_WRAP_W`], whose own value is the measured
/// single-line cluster against the measured widest single chip (see the token's
/// doc comment). As with [`row_flow`], the token is the *minimum* width at which
/// the one-line form is allowed to stand: below it the row wraps.
#[must_use]
pub fn chip_flow(available_width: f32) -> ChipFlow {
    if available_width < CHIP_ROW_NO_WRAP_W {
        ChipFlow::Wrapped
    } else {
        ChipFlow::OneLine
    }
}

/// Where one chip sits inside the card's content column, on a running cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipPlacement {
    /// Zero-based chip row; also the row's index into the card's height.
    pub row: usize,
    /// Offset from the content column's left edge.
    pub left: f32,
    /// The chip's own width, as measured from its label.
    pub width: f32,
}

/// Lays the chips out on a running `x` cursor, breaking to a new line when the
/// next chip would not fit.
///
/// This is the same hand-allocated running-cursor idiom the chips have always
/// used, with the one thing it lacked: a line break. At or above
/// [`CHIP_ROW_NO_WRAP_W`] every chip lands on row `0` at the offsets the
/// pre-wrap code produced, so wide layouts are byte-identical.
#[must_use]
pub fn chip_placements(chip_widths: &[f32], available_width: f32) -> Vec<ChipPlacement> {
    let mut placements = Vec::with_capacity(chip_widths.len());
    let mut row = 0usize;
    let mut x = 0.0_f32;
    for &width in chip_widths {
        // A chip wider than the column still gets its own line at the left
        // edge: it cannot be made to fit, and truncating it would be worse
        // than letting the card's own clip stop.
        let breaks = !placements.is_empty() && x + width > available_width;
        if breaks {
            row += 1;
            x = 0.0;
        }
        placements.push(ChipPlacement {
            row,
            left: x,
            width,
        });
        // The trailing gap is not part of the line's occupied width, which is
        // what keeps the last chip from wrapping on its own.
        x += width + CHIP_GAP;
    }
    placements
}

/// The height the chip block needs for however many rows `placements` spans.
///
/// One row is exactly [`CHIP_H`], the height the card allocated before wrapping
/// existed.
#[must_use]
pub fn chip_block_height(placements: &[ChipPlacement]) -> f32 {
    // Counted by walking the placements rather than by multiplying a row count:
    // one chip height per new row, one gap per break. No numeric cast involved.
    let mut height = 0.0_f32;
    let mut current_row = None;
    for placement in placements {
        if current_row == Some(placement.row) {
            continue;
        }
        if current_row.is_some() {
            height += CHIP_GAP;
        }
        height += CHIP_H;
        current_row = Some(placement.row);
    }
    height
}

/// One library row: folder glyph, truncated path, then the readiness dot +
/// label, Scan, Watch, and trash controls. Every derived string (the lossy
/// path text, its truncation, and the per-control accessibility/hover
/// labels) is built ONCE here and passed down — allocation plan 2.5 hoists
/// them out of the per-control call tree so idle frames format each string
/// a single time per row.
fn library_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    row: &LibraryRow,
    actions: &mut Vec<SettingsAction>,
) {
    // One line while the row is wide enough for the control cluster *and* a
    // legible path; below [`LIBRARY_ROW_STACK_W`] the row stacks — path on the
    // first line, readiness and controls on the second — so neither is clipped
    // and neither overprints the other. `Inline` reproduces the previous
    // geometry exactly, which is what keeps the pinned widths untouched.
    let flow = row_flow(ui.available_width());
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_height(flow)),
        egui::Sense::hover(),
    );
    // Inline keeps today's single band exactly. Stacked splits the row into
    // three: the path, then the track count beside the readiness pair, then the
    // control strip. Two bands are not enough at the minimum stage — the strip
    // alone is ~180px and the count + readiness pair ~110px, against a ~205px
    // row — so the row grows rather than letting either overprint the other.
    let (path_band, meta_band, strip_band) = match flow {
        RowFlow::Inline => (rect, rect, rect),
        RowFlow::Stacked => {
            let path_band = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.right(), rect.top() + LIBRARY_ROW_H),
            );
            let meta_band = egui::Rect::from_min_max(
                egui::pos2(rect.left(), path_band.bottom()),
                egui::pos2(rect.right(), path_band.bottom() + SMALL_BTN_H),
            );
            (
                path_band,
                meta_band,
                egui::Rect::from_min_max(meta_band.right_bottom(), rect.right_bottom()),
            )
        }
    };

    let path_str = row.path.to_string_lossy();
    let display = truncate_path(&path_str, 48);
    let remove_label = format!("Remove {path_str} from your libraries");
    let watch_label = format!("Watch {path_str}");
    let scan_label = format!("Scan {path_str}");

    library_row_path(ui, cache, palette, path_band, row, &display, flow);
    // Returns where the Scan button starts so the readiness pair can sit
    // gap-4 to its left.
    let layout = library_row_controls(
        ui,
        cache,
        palette,
        reduce_motion,
        strip_band,
        row,
        &path_str,
        &remove_label,
        &watch_label,
        &scan_label,
        actions,
    );
    match flow {
        RowFlow::Inline => library_row_readiness(
            ui,
            palette,
            rect,
            row,
            ReadinessSide::LeftOf(layout.scan_left),
        ),
        RowFlow::Stacked => {
            library_row_meta(ui, palette, meta_band, row);
        }
    }
}

/// The stacked row's second band: the track count, then the readiness pair.
fn library_row_meta(ui: &mut egui::Ui, palette: &Palette, rect: egui::Rect, row: &LibraryRow) {
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;
    let count_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS).clone();
    let count = painter.layout_no_wrap(
        format!("{} tracks", row.indexed_tracks),
        count_font,
        palette.ink_3,
    );
    let count_x = rect.left() + PREF_ROW_PAD;
    painter.galley(
        egui::pos2(count_x, cy - count.size().y / 2.0),
        count.clone(),
        palette.ink_3,
    );
    library_row_readiness(
        ui,
        palette,
        rect,
        row,
        ReadinessSide::RightOf(count_x + count.size().x + LIBRARY_ROW_TEXT_GAP),
    );
}

/// The row's left cluster: folder glyph plus the (possibly struck-through)
/// truncated path. `display` arrives precomputed by [`library_row`].
fn library_row_path(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    row: &LibraryRow,
    display: &str,
    flow: RowFlow,
) {
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;
    let is_unavailable = matches!(row.status, LibraryStatus::Unavailable);

    let folder_tex_id = cache.texture(ui.ctx(), Icon::Folder, 16.0, palette.ink_3);
    let folder_rect =
        egui::Rect::from_center_size(egui::pos2(rect.left() + 24.0, cy), egui::vec2(16.0, 16.0));
    painter.image(folder_tex_id, folder_rect, UV_FULL, palette.ink_3);

    let body_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM).clone();
    let path_color = if is_unavailable {
        palette.warning
    } else {
        palette.ink
    };
    // The live per-folder track count, measured first because the path has to
    // yield room for it.
    let count_galley = painter.layout_no_wrap(
        format!("{} tracks", row.indexed_tracks),
        styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
        palette.ink_3,
    );
    let path_x = folder_rect.right() + 12.0;
    // Inline keeps today's fixed 48-character truncation and paints the track
    // count beside the path. Stacked gives the path its own band: it is fitted
    // to whatever the band has left, and the count moves down to the meta band.
    let stacked = flow == RowFlow::Stacked;
    let path_text = match flow {
        RowFlow::Inline => display.to_owned(),
        RowFlow::Stacked => {
            let budget = (rect.width() - (path_x - rect.left()) - PREF_ROW_PAD).max(0.0);
            fit_to_width(&painter, &body_font, display, budget, path_color)
        }
    };
    let path_galley = painter.layout_no_wrap(path_text, body_font, path_color);
    painter.galley(
        egui::pos2(path_x, cy - path_galley.size().y / 2.0),
        path_galley.clone(),
        path_color,
    );

    // Muted, beside the path, so each row answers "how much lives here". In
    // the stacked form the count has its own band, so it is not painted here.
    if !stacked {
        painter.galley(
            egui::pos2(
                path_x + path_galley.size().x + LIBRARY_ROW_TEXT_GAP,
                cy - count_galley.size().y / 2.0,
            ),
            count_galley,
            palette.ink_3,
        );
    }
    if is_unavailable {
        // Strikethrough for the missing root, as the previous row rendered it.
        painter.line_segment(
            [
                egui::pos2(path_x, cy),
                egui::pos2(path_x + path_galley.size().x, cy),
            ],
            egui::Stroke::new(1.0_f32, palette.warning),
        );
    }
}

/// The geometry `library_row_controls` hands back to `library_row` so the
/// readiness pair can sit gap-4 to the left of the Scan button and the
/// path column can stop short of the right cluster.
#[allow(
    dead_code,
    reason = "row_right is reserved for the path column's right limit"
)]
struct RowLayout {
    /// Screen X of the Scan button's left edge — the readiness dot anchors
    /// to it.
    scan_left: f32,
    /// Screen X of the right cluster's right edge (the trash button's
    /// right padding). Reserved for the path column's right limit.
    row_right: f32,
}

/// The row's right cluster (gap-4): trash | watch | scan. Returns the
/// geometry `library_row_readiness` needs to anchor the readiness dot.
/// All per-path labels arrive precomputed by [`library_row`]; the trash
/// hover text is only built while hovered.
#[allow(clippy::too_many_arguments)]
fn library_row_controls(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    rect: egui::Rect,
    row: &LibraryRow,
    path_str: &str,
    remove_label: &str,
    watch_label: &str,
    scan_label: &str,
    actions: &mut Vec<SettingsAction>,
) -> RowLayout {
    let is_unavailable = matches!(row.status, LibraryStatus::Unavailable);
    let is_scanning = matches!(row.status, LibraryStatus::Scanning { .. });
    let row_right = rect.right();
    let mut scan_left = row_right;

    // Layout-based positioning (plan #3): the right cluster is one
    // right-to-left strip. Insertion order is the visual order from the
    // right edge: 16px padding, trash, 16px gap, watch, 16px gap, scan.
    // Each control grabs its own slot via `allocate_exact_size`; the slot's
    // response drives paint + interact. Item spacing is zeroed inside the
    // closure so the explicit `add_space(16.0)` gaps reproduce the
    // original hand-computed 16px exactly.
    ui.push_id(path_str, |ui| {
        // `new_child` does NOT consume parent space (see egui ui.rs:204);
        // `allocate_ui_with_layout` would re-allocate the row and push
        // everything below it down by `LIBRARY_ROW_H`. The row's rect is
        // already reserved by `library_row`'s `allocate_exact_size`.
        let mut strip_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        strip_ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        // 16px right padding (the original `rect.right() - 16.0`).
        strip_ui.add_space(16.0);

        // --- Trash (rightmost) ---
        let (trash_rect, mut trash_response) =
            strip_ui.allocate_exact_size(egui::vec2(TRASH_BTN, TRASH_BTN), egui::Sense::click());
        if trash_response.hovered() {
            trash_response = trash_response.on_hover_text(remove_label.to_owned());
        }
        let trash_tint = if trash_response.hovered() {
            palette.error
        } else {
            palette.ink_3
        };
        let trash_tex_id = cache.texture(strip_ui.ctx(), Icon::Trash, 16.0, trash_tint);
        strip_ui
            .painter()
            .image(trash_tex_id, trash_rect.shrink(6.0), UV_FULL, trash_tint);
        trash_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Remove library")
        });
        if trash_response.clicked() {
            actions.push(SettingsAction::Remove(row.path.clone()));
        }
        strip_ui.add_space(16.0);

        // --- Watch (label + checkbox) ---
        watch_control(
            &mut strip_ui,
            palette,
            row,
            is_unavailable,
            watch_label,
            actions,
        );
        strip_ui.add_space(16.0);

        // --- Scan (leftmost in the strip) ---
        let scan_font = styled_font(&strip_ui, egui::TextStyle::Button, theme::TEXT_XS);
        let scan_label_w = strip_ui
            .painter()
            .layout_no_wrap("Scan".to_owned(), scan_font, palette.ink)
            .size()
            .x;
        let scan_w = 24.0 + scan_label_w;
        let (scan_rect, _) =
            strip_ui.allocate_exact_size(egui::vec2(scan_w, SMALL_BTN_H), egui::Sense::hover());
        scan_left = scan_rect.left();
        let scan_enabled = !is_scanning && !is_unavailable;
        if button::text_button(
            &strip_ui,
            cache,
            palette,
            reduce_motion,
            &TextButton {
                id: egui::Id::new("settings_scan"),
                rect: scan_rect,
                label: "Scan",
                a11y: scan_label,
                tooltip: None,
                icon: None,
                small: true,
                variant: Variant::Secondary,
                enabled: scan_enabled,
            },
        ) {
            actions.push(SettingsAction::Scan(row.path.clone()));
        }
    });
    RowLayout {
        scan_left,
        row_right,
    }
}

/// The per-root Watch control: a "Watch" label beside the shared checkbox box
/// (painted through [`super::toggle_switch`] with the same focus ring the
/// toggle pill draws), over the whole-slot hit area so clicking the label
/// toggles too. A [`WatchState::Warning`] or an unavailable root disables the
/// box — no focus ring, no click — and the warning's reason shows as a tooltip.
/// It stays a checkbox (not a preference toggle) precisely because that
/// per-Library-Path disabling is what distinguishes Watch State from a
/// plain boolean preference.
fn watch_control(
    ui: &mut egui::Ui,
    palette: &Palette,
    row: &LibraryRow,
    is_unavailable: bool,
    watch_label: &str,
    actions: &mut Vec<SettingsAction>,
) {
    // Slot height spans the full row so the hit area matches the
    // original `expand2(0, (LIBRARY_ROW_H - WATCH_BOX) / 2)`.
    let watch_label_w = 38.0;
    let watch_w = watch_label_w + 6.0 + WATCH_BOX;
    let (watch_rect, watch_response) =
        ui.allocate_exact_size(egui::vec2(watch_w, LIBRARY_ROW_H), egui::Sense::click());
    let watch_warning = matches!(row.watch, WatchState::Warning(_));
    let can_watch = !watch_warning && !is_unavailable;
    let watching = row.watch == WatchState::Enabled;
    let box_rect = egui::Rect::from_center_size(
        egui::pos2(watch_rect.right() - WATCH_BOX / 2.0, watch_rect.center().y),
        egui::vec2(WATCH_BOX, WATCH_BOX),
    );
    ui.painter().text(
        egui::pos2(watch_rect.left(), watch_rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Watch",
        styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
        palette.ink_3,
    );
    let watch_focused = can_watch && ui.memory(|m| m.has_focus(watch_response.id));
    super::toggle_switch::paint_checkbox_with_focus(
        ui.painter(),
        palette,
        box_rect,
        watching && can_watch,
        watch_focused,
    );
    super::toggle_switch::register_checkbox_a11y(&watch_response, can_watch, watching, watch_label);
    if can_watch && watch_response.clicked() {
        actions.push(SettingsAction::SetWatch(row.path.clone(), !watching));
    }
    if let WatchState::Warning(ref reason) = row.watch {
        watch_response.on_hover_text(reason.clone());
    }
}

/// Which side of an anchor the readiness pair sits on.
///
/// Inline, the pair sits to the LEFT of the Scan button (its historical
/// position, reproduced exactly). Stacked, it sits to the RIGHT of the track
/// count on the meta band.
#[derive(Debug, Clone, Copy)]
enum ReadinessSide {
    /// The pair's left edge is this far left of the anchor.
    LeftOf(f32),
    /// The pair's left edge is the anchor.
    RightOf(f32),
}

/// The readiness dot + label at the far left of the control cluster
/// (gap-2 inside the pair, gap-4 before it).
fn library_row_readiness(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: egui::Rect,
    row: &LibraryRow,
    side: ReadinessSide,
) {
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;
    let ready = row.readiness();
    let label_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS);
    let label_galley = painter.layout_no_wrap(ready.label().to_owned(), label_font, palette.ink_3);
    let cluster_left = match side {
        ReadinessSide::LeftOf(anchor) => {
            anchor - PREF_ROW_PAD - (DOT_SIZE + READINESS_GAP + label_galley.size().x)
        }
        ReadinessSide::RightOf(anchor) => anchor,
    };
    painter.circle_filled(
        egui::pos2(cluster_left + DOT_SIZE / 2.0, cy),
        DOT_SIZE / 2.0,
        ready.dot_color(palette),
    );
    painter.galley(
        egui::pos2(
            cluster_left + DOT_SIZE + READINESS_GAP,
            cy - label_galley.size().y / 2.0,
        ),
        label_galley,
        palette.ink_3,
    );
}

/// The Add Library (primary) + Scan All (secondary) actions row.
fn actions_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    actions: &mut Vec<SettingsAction>,
) {
    // The two buttons share a line while both fit beside each other; below
    // [`LIBRARY_ROW_STACK_W`] they take one line each and the row grows, so
    // neither runs off the pane's right edge. The widths are measured once and
    // only the *placement* branches, so the single-line form is unchanged.
    let probe = ui.painter();
    let add_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let add_label_w = probe
        .layout_no_wrap("Add Library".to_owned(), add_font, palette.on_brand)
        .size()
        .x;
    let scan_all_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let scan_all_label_w = probe
        .layout_no_wrap("Scan All".to_owned(), scan_all_font, palette.ink)
        .size()
        .x;
    let add_w = ROW_BTN_PAD + add_label_w;
    let scan_all_w = ROW_BTN_PAD + scan_all_label_w;
    let side_by_side = PREF_ROW_PAD + add_w + ACTIONS_ROW_GAP + scan_all_w;
    let stacked = ui.available_width() < side_by_side;

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            if stacked {
                ACTIONS_ROW_H + ACTION_BTN_H
            } else {
                ACTIONS_ROW_H
            },
        ),
        egui::Sense::hover(),
    );
    let cy = if stacked {
        rect.top() + ACTION_BTN_H / 2.0
    } else {
        rect.center().y
    };
    let add_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + PREF_ROW_PAD, cy - ACTION_BTN_H / 2.0),
        egui::vec2(add_w, ACTION_BTN_H),
    );
    if filled_button(
        ui,
        cache,
        palette,
        reduce_motion,
        add_rect,
        egui::Id::new("settings_add_library"),
        "Add Library",
        "Add Library",
        Some(Icon::Plus),
        Variant::Primary,
        false,
        true,
    ) {
        actions.push(SettingsAction::AddLibrary);
    }

    let scan_all_rect = egui::Rect::from_min_size(
        egui::pos2(
            if stacked {
                add_rect.left()
            } else {
                add_rect.right() + ACTIONS_ROW_GAP
            },
            if stacked {
                add_rect.bottom() + ACTION_BTN_H / 2.0
            } else {
                cy - ACTION_BTN_H / 2.0
            },
        ),
        egui::vec2(scan_all_w, ACTION_BTN_H),
    );
    if filled_button(
        ui,
        cache,
        palette,
        reduce_motion,
        scan_all_rect,
        egui::Id::new("settings_scan_all"),
        "Scan All",
        "Scan All",
        Some(Icon::RefreshCw),
        Variant::Accent,
        false,
        true,
    ) {
        actions.push(SettingsAction::ScanAll);
    }
}

/// The format chips card: one toggle chip per [`AUDIO_EXTENSIONS`] entry;
/// enabled formats are indexed on the next scan (design-handoff issue 12).
///
/// The chips are measured first and then placed by [`chip_placements`], so a
/// content column too narrow for all seven on one line wraps them onto as many
/// lines as they need and the card grows to match. At or above
/// [`CHIP_ROW_NO_WRAP_W`] there is exactly one line and the painted result is
/// unchanged from the pre-wrap version.
fn formats_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    settings_card(ui, palette, Some(12), |ui| {
        let available = ui.available_width();
        let font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
        // Measure every chip before painting any of them: the card's height
        // is the sum over the lines the placements come out on, which is
        // not known until the last chip has been measured.
        let mut labels = Vec::with_capacity(AUDIO_EXTENSIONS.len());
        let mut chip_widths = Vec::with_capacity(AUDIO_EXTENSIONS.len());
        for extension in AUDIO_EXTENSIONS {
            let label = extension.to_uppercase();
            let label_w = ui
                .painter()
                .layout_no_wrap(label.clone(), font.clone(), palette.ink)
                .size()
                .x;
            labels.push(label);
            chip_widths.push(CHIP_LABEL_PAD * 2.0 + label_w);
        }
        let placements = chip_placements(&chip_widths, available);
        // `chip_flow` is the token-backed summary of the same decision, while
        // the cursor is what actually places the chips. They must agree: if
        // the token ever drifts away from the measured cluster it was derived
        // from, the card would size itself for one form and paint the other.
        // An empty list has nothing to wrap and is excluded, since
        // `chip_flow` only reports on the column, not on the contents.
        debug_assert!(
            placements.is_empty()
                || placements.iter().any(|placement| placement.row > 0)
                    == matches!(chip_flow(available), ChipFlow::Wrapped),
            "chip_flow and chip_placements must agree about wrapping"
        );

        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(available, chip_block_height(&placements)),
            egui::Sense::hover(),
        );
        for ((extension, label), placement) in AUDIO_EXTENSIONS.iter().zip(labels).zip(placements) {
            let enabled = content.scan_formats.iter().any(|f| f == extension);
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(
                    rect.left() + placement.left,
                    rect.top() + chip_row_top(placement),
                ),
                egui::vec2(placement.width, CHIP_H),
            );
            let a11y = format!("Index {extension} files");
            if filled_button(
                ui,
                cache,
                palette,
                content.reduce_motion,
                chip_rect,
                egui::Id::new(("settings_format_chip", *extension)),
                &label,
                &a11y,
                None,
                // PRE-EXISTING, deliberately preserved: this call site
                // passed `enabled` into the old `primary` slot, so a chip
                // has been painting as `Primary` — brand-filled — whenever
                // it is enabled, and only falls back to `Secondary` when
                // disabled. The golden shows it: the chip row is a band of
                // solid brand. That is almost certainly a slip (a chip is
                // not a primary action, and a chip that changes tier with
                // its own enabled state is not a tier at all), but fixing it
                // would restyle every idle chip, which is a resting-fill
                // change and therefore not this slice's business. Carried
                // forward verbatim so the wash stays the only difference.
                if enabled {
                    Variant::Primary
                } else {
                    Variant::Secondary
                },
                true,
                true,
            ) {
                actions.push(SettingsAction::SetFormat(
                    (*extension).to_string(),
                    !enabled,
                ));
            }
        }
    });
}

/// The y offset of a chip's row from the top of the chip block. Rows advance by
/// one chip height plus the shared gap.
fn chip_row_top(placement: ChipPlacement) -> f32 {
    // Folded from the row index without a numeric cast: each row below the first
    // adds one chip height and one gap.
    let mut top = 0.0_f32;
    for _ in 1..=placement.row {
        top += CHIP_H + CHIP_GAP;
    }
    top
}

/// One line of the Last Full Scan card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCardLine {
    /// The line's copy.
    pub text: String,
    /// Whether this line wears the error role. Only the error count sets it,
    /// and only when the count is non-zero — a scan that found nothing wrong
    /// must not cry wolf.
    pub is_signal: bool,
}

/// The three lines the Last Full Scan card always paints.
///
/// Three is a fixed count, not a function of the counts: a zero-error scan
/// still occupies its error line, so the card cannot resize as a scan's error
/// count moves. `SCAN_CARD_H` is sized for three lines and the card always
/// allocates exactly that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCardLines {
    /// When the scan finished, in primary ink.
    pub stamp: ScanCardLine,
    /// How many files it indexed.
    pub files: ScanCardLine,
    /// How many errors it hit — the line that reads as a signal.
    pub error: ScanCardLine,
}

impl ScanCardLines {
    /// How many lines the card paints. Always three.
    pub const LINES: usize = 3;

    /// The lines, in paint order. Always [`ScanCardLines::LINES`] of them.
    pub fn iter(&self) -> impl Iterator<Item = &ScanCardLine> {
        [&self.stamp, &self.files, &self.error].into_iter()
    }
}

/// Build the card's three lines from the last full scan summary.
///
/// Pure, so the "zero errors still occupies its line" rule is assertable
/// without rendering: see
/// `test_scan_card_height_is_stable_across_error_counts`.
#[must_use]
pub fn scan_card_lines(summary: Option<&FullScanSummary>) -> ScanCardLines {
    match summary {
        Some(summary) => {
            let elapsed = summary.at.elapsed().unwrap_or_default();
            let errors = summary.errors;
            ScanCardLines {
                stamp: ScanCardLine {
                    text: format!(
                        "Last full scan {}",
                        crate::ui::sidebar::format_last_scan_ago(elapsed)
                    ),
                    is_signal: false,
                },
                files: ScanCardLine {
                    text: format!("{} files indexed", summary.files),
                    is_signal: false,
                },
                error: ScanCardLine {
                    text: format!("{errors} errors"),
                    is_signal: errors > 0,
                },
            }
        }
        None => ScanCardLines {
            stamp: ScanCardLine {
                text: String::from("No full scan recorded yet"),
                is_signal: false,
            },
            files: ScanCardLine {
                text: String::from("Run a scan to index your music folders."),
                is_signal: false,
            },
            error: ScanCardLine {
                text: String::from("0 errors"),
                is_signal: false,
            },
        },
    }
}

/// The colour a scan-card line is painted in.
///
/// A line that `is_signal` wears the error role; every other line wears the
/// muted ink rung, including a zero-error count — so a clean scan does not cry
/// wolf. Pure, so "the error count is painted in the error role when non-zero"
/// is assertable without rendering.
#[must_use]
pub fn scan_line_color(line: &ScanCardLine, palette: &Palette) -> egui::Color32 {
    if line.is_signal {
        palette.error
    } else {
        palette.ink_3
    }
}

/// The height the Last Full Scan card allocates, for `lines`.
///
/// Deliberately a function of the line count and nothing else — not of the
/// files or error totals — so the card cannot resize as a scan's outcome
/// changes. This is the seam `test_scan_card_height_is_stable_across_error_counts`
/// pins.
#[must_use]
pub fn scan_card_height(lines: &ScanCardLines) -> f32 {
    debug_assert_eq!(
        lines.iter().count(),
        ScanCardLines::LINES,
        "the card paints three lines"
    );
    SCAN_CARD_H
}

/// The last-full-scan card: when the scan finished and what it saw, on three
/// lines, with a Rescan now action (design-handoff issue 12).
fn scan_status_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    settings_card(ui, palette, None, |ui| {
        let lines = scan_card_lines(content.last_scan.as_ref());
        // The card always allocates `scan_card_height`, sized for three
        // lines, so a scan whose error count changes never resizes it.
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), scan_card_height(&lines)),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);

        // Action first: its width is what the text column has to yield, so
        // the two never overlap and neither is hand-positioned against the
        // other's arithmetic.
        let btn_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
        let btn_label_w = painter
            .layout_no_wrap(RESCAN_LABEL.to_owned(), btn_font, palette.ink)
            .size()
            .x;
        let btn_w = SMALL_BTN_LABEL_PAD + btn_label_w;
        let btn_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - SCAN_CARD_PAD - btn_w,
                rect.center().y - SMALL_BTN_H / 2.0,
            ),
            egui::vec2(btn_w, SMALL_BTN_H),
        );

        // A real vertical layout: the three lines are measured, then stacked
        // by a child `Ui` that owns the column. No line is placed at an
        // offset from the card's centre — the only arithmetic is halving a
        // measured height so the block sits opposite the action.
        let styles = [
            (egui::TextStyle::Body, theme::TEXT_SM),
            (egui::TextStyle::Small, theme::TEXT_XS),
            (egui::TextStyle::Small, theme::TEXT_XS),
        ];
        let text_left = rect.left() + SCAN_CARD_PAD;
        let text_width = (btn_rect.left() - SCAN_CARD_PAD - text_left).max(0.0);
        let galleys: Vec<_> = lines
            .iter()
            .zip(styles)
            .map(|(line, style)| {
                let color = scan_line_color(line, palette);
                let font = styled_font(ui, style.0, style.1);
                (
                    color,
                    painter.layout_no_wrap(line.text.clone(), font, color),
                )
            })
            .collect();
        let block_h: f32 = galleys.iter().map(|(_, galley)| galley.size().y).sum();
        let text_rect = egui::Rect::from_min_size(
            egui::pos2(text_left, rect.center().y - block_h / 2.0_f32),
            egui::vec2(text_width, block_h),
        );
        let mut text_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        for (color, galley) in &galleys {
            text_ui.painter().galley(
                egui::pos2(text_rect.left(), text_ui.cursor().top()),
                galley.clone(),
                *color,
            );
            // Advance by the line's own height, so the stack needs no
            // leading constant of its own.
            text_ui.add_space(galley.size().y);
        }

        if filled_button(
            ui,
            cache,
            palette,
            content.reduce_motion,
            btn_rect,
            egui::Id::new("settings_rescan_now"),
            RESCAN_LABEL,
            RESCAN_LABEL,
            Some(Icon::RefreshCw),
            Variant::Accent,
            true,
            true,
        ) {
            actions.push(SettingsAction::ScanAll);
        }
    });
}

/// The page footer: the immediate-apply note on the left, and the page's three
/// actions on the right — Clear Thumbnail cache and Clear Library (both
/// destructive ghosts) then Done (primary).
///
/// The Thumbnail clear belongs here rather than in the Library pane's Artwork card
/// beside the toggle it relates to: measured at 920×840, a row at the bottom of
/// that card sits behind this footer and answers no click. This is the one reclaim
/// a cache with no eviction has, so it cannot live below the fold.
///
/// This used to be the Library pane's footer and now belongs to the page frame
/// itself, which is what makes the destructive action reachable from every
/// section rather than only from Library. It sits outside the pane's
/// `ScrollArea`, so the actions never scroll away.
fn library_footer(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), FOOTER_H),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;

    // Done (primary) hugs the right edge; Clear Library sits to its left.
    let done_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let done_w = painter
        .layout_no_wrap(DONE_LABEL.to_owned(), done_font, palette.on_brand)
        .size()
        .x
        + 32.0;
    let done_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - done_w - 4.0, cy - ACTION_BTN_H / 2.0),
        egui::vec2(done_w, ACTION_BTN_H),
    );

    // The two destructive ghosts, chained right-to-left off Done. Each is sized off
    // its own label, so neither carries a hardcoded width.
    let (library_left, cleared_library) = footer_ghost(
        ui,
        cache,
        palette,
        reduce_motion,
        &painter,
        CLEAR_LIBRARY_LABEL,
        done_rect.left(),
        cy,
        egui::Id::new("settings_clear_library"),
    );
    if cleared_library {
        actions.push(SettingsAction::ClearLibrary);
    }
    // The two wipes are told apart by their labels and by entirely different
    // confirm copy: one removes the indexed collection, the other only art that
    // was derived from it and rebuilds on its own.
    let (thumbnails_left, cleared_thumbnails) = footer_ghost(
        ui,
        cache,
        palette,
        reduce_motion,
        &painter,
        crate::ui::prompts::CLEAR_THUMBNAIL_CACHE_LABEL,
        library_left,
        cy,
        egui::Id::new("settings_clear_thumbnail_cache"),
    );
    if cleared_thumbnails {
        actions.push(SettingsAction::ClearThumbnailCache);
    }

    // The immediate-apply note takes only what the action chain leaves. It is
    // generic chrome; the actions are the point of this row. Measured at the
    // minimum supported width, a sentence and three actions do not share one line,
    // and the note is the thing that goes — the alternative is an action that
    // silently stops being clickable, which is how the only reclaim a cache with no
    // eviction becomes impossible to reach.
    let note_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM);
    let note_w = painter
        .layout_no_wrap(FOOTER_NOTE.to_owned(), note_font.clone(), palette.ink_3)
        .size()
        .x;
    if rect.left() + 4.0 + note_w + FOOTER_ACTION_GAP <= thumbnails_left {
        painter.text(
            egui::pos2(rect.left() + 4.0, cy),
            egui::Align2::LEFT_CENTER,
            FOOTER_NOTE,
            note_font,
            palette.ink_3,
        );
    }

    if filled_button(
        ui,
        cache,
        palette,
        reduce_motion,
        done_rect,
        egui::Id::new("settings_done"),
        DONE_LABEL,
        DONE_LABEL,
        None,
        Variant::Primary,
        false,
        true,
    ) {
        actions.push(SettingsAction::Back);
    }
}

/// One destructive footer ghost: sized off its own label and hung to the left of
/// `right_of`, so the footer's actions chain right-to-left without any of them
/// carrying a width the next label change would invalidate. Returns the rect's
/// left edge with the click verdict, which is where the next ghost in the chain
/// measures from.
#[allow(clippy::too_many_arguments)]
fn footer_ghost(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    painter: &egui::Painter,
    label: &str,
    right_of: f32,
    cy: f32,
    id: egui::Id,
) -> (f32, bool) {
    let width = painter
        .layout_no_wrap(
            label.to_owned(),
            styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS),
            palette.error,
        )
        .size()
        .x
        + SMALL_BTN_LABEL_PAD;
    let left = right_of - width - FOOTER_ACTION_GAP;
    let clicked = button::text_button(
        ui,
        cache,
        palette,
        reduce_motion,
        &TextButton {
            id,
            rect: egui::Rect::from_min_size(
                egui::pos2(left, cy - SMALL_BTN_H / 2.0),
                egui::vec2(width, SMALL_BTN_H),
            ),
            label,
            a11y: label,
            tooltip: None,
            icon: None,
            small: true,
            variant: Variant::Destructive,
            enabled: true,
        },
    );
    (left, clicked)
}

/// The boolean preferences the stage drives through the reusable toggle
/// switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preference {
    Advanced,
    HighContrast,
    ReduceMotion,
    ReplayGain,
    WatchChanges,
    SkipHidden,
    ReadEmbedded,
    // Only ever constructed inside a `#[cfg(not(target_os = "linux"))]` block,
    // because the tray quit menu item is what offers this preference. The
    // variant itself must stay ungated: the `impl Preference` match arms below
    // are compiled on every platform and would not be exhaustive without it. On
    // Linux it is therefore read but never built, which is a `dead_code` error
    // under CI's `-D warnings` and is invisible from a macOS or Windows machine.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    CloseQuitsApp,
}

impl Preference {
    /// `(title, description)` copy verbatim from the mockup.
    fn copy(self) -> (&'static str, &'static str) {
        match self {
            Self::Advanced => PREF_ADVANCED,
            Self::HighContrast => PREF_HIGH_CONTRAST,
            Self::ReduceMotion => PREF_REDUCE_MOTION,
            Self::ReplayGain => PREF_REPLAYGAIN,
            Self::WatchChanges => PREF_WATCH_CHANGES,
            Self::SkipHidden => PREF_SKIP_HIDDEN,
            Self::ReadEmbedded => PREF_READ_EMBEDDED,
            Self::CloseQuitsApp => PREF_CLOSE_QUITS,
        }
    }

    /// The action reporting `value` for this preference.
    fn action(self, value: bool) -> SettingsAction {
        match self {
            Self::Advanced => SettingsAction::SetAdvanced(value),
            Self::HighContrast => SettingsAction::SetHighContrast(value),
            Self::ReduceMotion => SettingsAction::SetReduceMotion(value),
            Self::ReplayGain => SettingsAction::SetReplayGain(value),
            Self::WatchChanges => SettingsAction::SetWatchAll(value),
            Self::SkipHidden => SettingsAction::SetSkipHidden(value),
            Self::ReadEmbedded => SettingsAction::SetReadEmbeddedArtwork(value),
            Self::CloseQuitsApp => SettingsAction::SetCloseQuitsApp(value),
        }
    }

    /// Stable widget-id stem.
    fn id(self) -> &'static str {
        match self {
            Self::Advanced => "pref_advanced",
            Self::HighContrast => "pref_high_contrast",
            Self::ReduceMotion => "pref_reduce_motion",
            Self::ReplayGain => "pref_replaygain",
            Self::WatchChanges => "pref_watch_changes",
            Self::SkipHidden => "pref_skip_hidden",
            Self::ReadEmbedded => "pref_read_embedded",
            Self::CloseQuitsApp => "pref_close_quits",
        }
    }

    /// The preference's current persisted value.
    fn checked(self, content: &SettingsContent) -> bool {
        match self {
            Self::Advanced => content.advanced_mode,
            Self::HighContrast => content.high_contrast,
            Self::ReduceMotion => content.reduce_motion,
            Self::ReplayGain => content.replaygain_enabled,
            Self::WatchChanges => content.watch_any,
            Self::SkipHidden => content.skip_hidden_files,
            Self::ReadEmbedded => content.read_embedded_artwork,
            Self::CloseQuitsApp => content.close_quits_app,
        }
    }
}

/// A card of preference rows, each driven by the reusable
/// [`super::toggle_switch::toggle_switch`]. `prefs` selects which rows the
/// caller's section shows (one card can hold any subset).
fn preferences_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
    prefs: &[Preference],
) {
    settings_card(ui, palette, Some(4), |ui| {
        for (i, pref) in prefs.iter().enumerate() {
            if i > 0 {
                row_separator(ui, palette, 16.0);
            }
            preference_row(ui, palette, *pref, pref.checked(content), actions);
        }
    });
}

fn artwork_card(
    ui: &mut egui::Ui,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    settings_card(ui, palette, Some(4), |ui| {
        preference_row(
            ui,
            palette,
            Preference::ReadEmbedded,
            content.read_embedded_artwork,
            actions,
        );
    });
}
fn info_lines(ui: &mut egui::Ui, palette: &Palette) {
    let mut lines =
        vec!["Smart playlists update automatically from play history and date added.".to_owned()];
    #[cfg(not(target_os = "linux"))]
    lines.push(
        "The system tray icon keeps the player reachable when the window is closed.".to_owned(),
    );
    lines.push(
        "Folder pickers use the native dialog on macOS and Windows; a text input is used on \
         Linux."
            .to_owned(),
    );

    for line in lines {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), theme::TEXT_SM + 8.0),
            egui::Sense::hover(),
        );
        ui.painter_at(rect).text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            line,
            styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
            palette.ink_3,
        );
        ui.add_space(8.0);
    }
}
fn library_lower_sections(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    preferences_card(
        ui,
        palette,
        content,
        actions,
        &[Preference::WatchChanges, Preference::SkipHidden],
    );

    ui.add_space(SECTION_GAP);
    section_header(ui, palette, SECTION_FORMATS);
    ui.add_space(HEADER_GAP);
    formats_card(ui, cache, palette, content, actions);

    ui.add_space(SECTION_GAP);
    section_header(ui, palette, SECTION_SCAN_STATUS);
    ui.add_space(HEADER_GAP);
    scan_status_card(ui, cache, palette, content, actions);

    ui.add_space(SECTION_GAP);
    section_header(ui, palette, SECTION_ARTWORK);
    ui.add_space(HEADER_GAP);
    artwork_card(ui, palette, content, actions);
}
fn library_pane(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    // The Libraries card keeps the full width: its rows and
    // its Add Library / Scan All actions simply get more
    // room than they had in the single stack.
    section_header(ui, palette, SECTION_LIBRARIES);
    ui.add_space(HEADER_GAP);
    libraries_card(ui, cache, palette, content, actions);

    ui.add_space(SECTION_GAP);
    // The lower four sections settle into two balanced
    // columns above MIN_TWO_COL_W and one stack below it.
    // `settings_pane_columns` owns the branch; this only
    // allocates the rects it hands back, the same idiom the
    // elastic stage uses for its columns.
    match settings_pane_columns(ui.available_width()) {
        LibraryPaneColumns::Stacked { width } => {
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(egui::Rect::from_min_size(
                        ui.cursor().min,
                        egui::vec2(width, ui.available_height()),
                    ))
                    .id_salt("settings-pane-column"),
                |ui| library_lower_sections(ui, cache, palette, content, actions),
            );
        }
        LibraryPaneColumns::TwoColumns { left, right } => {
            ui.horizontal_top(|ui| {
                // Left column: Preferences, then Formats.
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(left, ui.available_height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(("settings-pane-column", 0)),
                    |ui| {
                        preferences_card(
                            ui,
                            palette,
                            content,
                            actions,
                            &[Preference::WatchChanges, Preference::SkipHidden],
                        );

                        ui.add_space(SECTION_GAP);
                        section_header(ui, palette, SECTION_FORMATS);
                        ui.add_space(HEADER_GAP);
                        formats_card(ui, cache, palette, content, actions);
                    },
                );

                // Right column: Last Full Scan, then Artwork.
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(right, ui.available_height()),
                        ))
                        .layout(egui::Layout::top_down(egui::Align::Min))
                        .id_salt(("settings-pane-column", 1)),
                    |ui| {
                        section_header(ui, palette, SECTION_SCAN_STATUS);
                        ui.add_space(HEADER_GAP);
                        scan_status_card(ui, cache, palette, content, actions);

                        ui.add_space(SECTION_GAP);
                        section_header(ui, palette, SECTION_ARTWORK);
                        ui.add_space(HEADER_GAP);
                        artwork_card(ui, palette, content, actions);
                    },
                );
            });
        }
    }
}
fn modal_header(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    reduce_motion: bool,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), PAGE_HEADER_H),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);

    let heading_font = styled_font(ui, egui::TextStyle::Heading, theme::TEXT_XL);
    let galley = painter.layout_no_wrap("Settings".to_owned(), heading_font, palette.ink);
    painter.galley(
        egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0),
        galley,
        palette.ink,
    );

    // Close control at the header's right edge, hugging its content, painted
    // through the shared [`Variant::Caption`] primitive.
    let body_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let label_galley = painter.layout_no_wrap("Back".to_owned(), body_font, palette.ink_2);
    let btn_w = 12.0 + 16.0 + 8.0 + label_galley.size().x + 12.0;
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - btn_w - 12.0, rect.center().y - 16.0),
        egui::vec2(btn_w, 32.0),
    );
    if button::text_button(
        ui,
        cache,
        palette,
        reduce_motion,
        &TextButton {
            id: egui::Id::new("settings_back"),
            rect: btn_rect,
            label: "Back",
            a11y: "Back to Library",
            tooltip: None,
            icon: Some(Icon::ArrowLeft),
            small: false,
            variant: Variant::Caption,
            enabled: true,
        },
    ) {
        actions.push(SettingsAction::Back);
    }
}
fn nav_item(
    ui: &mut egui::Ui,
    palette: &Palette,
    section: SettingsSection,
    selected: bool,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), NAV_ITEM_H),
        egui::Sense::hover(),
    );
    let response = ui.interact(
        rect,
        egui::Id::new(("settings_nav", section.label())),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(
            rect.shrink2(egui::vec2(8.0, 0.0)),
            theme::RADIUS_MD,
            palette.surface_2,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 8.0, rect.top() + 6.0),
                egui::pos2(rect.left() + 11.0, rect.bottom() - 6.0),
            ),
            theme::RADIUS_FULL,
            palette.brand_primary,
        );
    } else if response.hovered() {
        painter.rect_filled(
            rect.shrink2(egui::vec2(8.0, 0.0)),
            theme::RADIUS_MD,
            palette.row_hover,
        );
    }
    painter.text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        section.label(),
        styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM),
        if selected { palette.ink } else { palette.ink_2 },
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, section.label()));
    if response.clicked() {
        actions.push(SettingsAction::SelectSection(section));
    }
}
#[allow(clippy::too_many_arguments, reason = "one preference row's paint call")]
fn paint_preference_text(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: egui::Rect,
    copy: &(&str, &str),
    title_font: egui::FontId,
    desc_font: egui::FontId,
    desc_galley: Option<Arc<egui::Galley>>,
    title_h: f32,
) {
    let painter = ui.painter_at(rect);
    if let Some(wrapped) = desc_galley {
        let top = rect.top() + PREF_ROW_TEXT_INSET;
        painter.galley(
            egui::pos2(rect.left() + PREF_ROW_PAD, top),
            painter.layout_no_wrap(copy.0.to_owned(), title_font, palette.ink),
            palette.ink,
        );
        painter.galley(
            egui::pos2(
                rect.left() + PREF_ROW_PAD,
                top + title_h + PREF_ROW_TEXT_GAP,
            ),
            wrapped,
            palette.ink_3,
        );
    } else {
        painter.text(
            egui::pos2(rect.left() + PREF_ROW_PAD, rect.top() + PREF_ROW_TEXT_INSET),
            egui::Align2::LEFT_TOP,
            copy.0,
            title_font,
            palette.ink,
        );
        painter.text(
            egui::pos2(
                rect.left() + PREF_ROW_PAD,
                rect.bottom() - PREF_ROW_TEXT_INSET,
            ),
            egui::Align2::LEFT_BOTTOM,
            copy.1,
            desc_font,
            palette.ink_3,
        );
    }
}

// --- Library Path input (Linux text flow) -----------------------------------------
//
// What the input draws and what the listener chose; the filesystem probe, the
// session registration, and the durable write are the host's Library Path
// adapter (`ui::app::library_picker`).

/// What the Library Path input shows, all of it resolved by the host: the draft
/// being typed, the rejection the last attempt earned, and the directory
/// suggestions for what is typed so far.
pub struct PathInput<'a> {
    pub text: &'a mut String,
    pub error: Option<&'a str>,
    pub suggestions: &'a [PathBuf],
}

/// What the listener did to the input. The host decides what each one means
/// for the Library Path facts and the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathInputAction {
    /// The typed path should become a Library Path.
    Confirm,
    /// The flow is abandoned; the draft is the host's to clear.
    Cancel,
    /// A suggestion was taken into the field to keep drilling with.
    Complete(PathBuf),
}

/// Render the text-based folder picker (no native dialog on Linux), reported as
/// typed actions. Pure presentation: no filesystem read, no session, no store —
/// whether a candidate is a root is the host's decision.
pub fn path_input(
    ui: &mut egui::Ui,
    palette: &Palette,
    input: &mut PathInput<'_>,
) -> Vec<PathInputAction> {
    let mut actions = Vec::new();
    if let Some(error) = input.error {
        ui.colored_label(palette.error, error);
    }
    ui.horizontal(|ui| {
        ui.label("Path:");
        ui.text_edit_singleline(input.text);
        if ui.button("Confirm").clicked() {
            actions.push(PathInputAction::Confirm);
        }
        if ui.button("Cancel").clicked() {
            actions.push(PathInputAction::Cancel);
        }
    });
    for suggestion in input.suggestions {
        let label = suggestion.to_string_lossy().to_string();
        if ui
            .selectable_label(false, format!("\u{1F4C1} {label}"))
            .clicked()
        {
            actions.push(PathInputAction::Complete(suggestion.clone()));
        }
    }
    actions
}
fn preference_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    pref: Preference,
    checked: bool,
    actions: &mut Vec<SettingsAction>,
) {
    let copy = pref.copy();
    let flow = row_flow(ui.available_width());
    let title_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM).clone();
    let desc_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS).clone();

    // The text column is whatever the row has left once the toggle is placed.
    // In the single-line form the description is painted whole and the row
    // keeps its fixed height; in the stacked form the description wraps to this
    // width and the row grows to whatever the wrapped text needs, so it is
    // never truncated mid-word.
    let text_width = preference_text_width(ui.available_width());
    // The title's own measured height drives the stacked layout, so the row
    // grows to fit its text rather than to a guessed line count.
    let title_galley =
        ui.painter()
            .layout_no_wrap(copy.0.to_owned(), title_font.clone(), palette.ink);
    let title_h = title_galley.size().y;
    let (row_h, desc_galley) = match flow {
        RowFlow::Inline => (PREF_ROW_H, None),
        RowFlow::Stacked => {
            let wrapped = ui.painter().layout(
                copy.1.to_owned(),
                desc_font.clone(),
                palette.ink_3,
                text_width,
            );
            (
                preference_row_height(flow, title_h, wrapped.size().y),
                Some(wrapped),
            )
        }
    };

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);
    if rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
        painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_2);
    }

    paint_preference_text(
        ui,
        palette,
        rect,
        &copy,
        title_font,
        desc_font,
        desc_galley,
        title_h,
    );

    let pill_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.right() - 16.0 - theme::geometry::toggle::TOGGLE_W / 2.0,
            rect.center().y,
        ),
        egui::vec2(
            theme::geometry::toggle::TOGGLE_W,
            theme::geometry::toggle::TOGGLE_H,
        ),
    );
    let toggled = super::toggle_switch::toggle_switch_at(
        ui,
        palette,
        egui::Id::new(pref.id()),
        copy.0,
        pill_rect,
        checked,
    );
    let row_clicked = ui
        .interact(
            rect,
            egui::Id::new((pref.id(), "row")),
            egui::Sense::click(),
        )
        .clicked();
    if toggled || row_clicked {
        actions.push(pref.action(!checked));
    }
}
fn section_pane(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    current: SettingsSection,
    actions: &mut Vec<SettingsAction>,
) {
    egui::ScrollArea::vertical()
        .id_salt("settings_pane")
        .auto_shrink(false)
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::from(PANE_PAD))
                .show(ui, |ui| match current {
                    SettingsSection::Library => {
                        library_pane(ui, cache, palette, content, actions);
                    }
                    SettingsSection::Advanced => {
                        // "Quit on close" is offered only where a tray exists:
                        // on Linux there is no tray, so closing always quits
                        // and the choice would be inert (decision 002).
                        #[cfg(not(target_os = "linux"))]
                        const ADVANCED_PREFS: &[Preference] =
                            &[Preference::Advanced, Preference::CloseQuitsApp];
                        #[cfg(target_os = "linux")]
                        const ADVANCED_PREFS: &[Preference] = &[Preference::Advanced];
                        preferences_card(ui, palette, content, actions, ADVANCED_PREFS);
                        ui.add_space(SECTION_GAP);
                        section_header(ui, palette, SECTION_REPLAYGAIN_PASS);
                        ui.add_space(HEADER_GAP);
                        replaygain_pass_card(ui, cache, palette, content, actions);
                        ui.add_space(SECTION_GAP);
                        section_header(ui, palette, SECTION_ADVANCED_INFO);
                        ui.add_space(HEADER_GAP);
                        info_lines(ui, palette);
                    }
                    SettingsSection::Playback => {
                        preferences_card(ui, palette, content, actions, &[Preference::ReplayGain]);
                        ui.add_space(SECTION_GAP);
                        section_header(ui, palette, SECTION_REPLAYGAIN_MODE);
                        ui.add_space(HEADER_GAP);
                        replaygain_mode_card(ui, cache, palette, content, actions);
                    }
                    SettingsSection::Appearance => {
                        preferences_card(
                            ui,
                            palette,
                            content,
                            actions,
                            &[Preference::HighContrast, Preference::ReduceMotion],
                        );
                    }
                    SettingsSection::About => {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} settings are not implemented yet.",
                                current.label()
                            ))
                            .color(palette.ink_3),
                        );
                    }
                });
        });
}
pub fn show_settings_modal(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    current: SettingsSection,
) -> Vec<SettingsAction> {
    let mut actions = Vec::new();

    // The page fills the stage: no floating card, no backdrop margin, no
    // width cap. `settings_page_rects` owns the frame so the UI tests and this
    // view cannot disagree about it.
    let rects = settings_page_rects(ui.available_rect_before_wrap());

    let painter = ui.painter_at(rects.page);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rects.page), |ui| {
        // Header first: it owns the top strip at full page width, and its Back
        // control is the first focusable widget on the page.
        ui.scope_builder(egui::UiBuilder::new().max_rect(rects.header), |ui| {
            modal_header(ui, cache, palette, content.reduce_motion, &mut actions);
        });

        // Body takes the fill between header and footer; that fill is what
        // pins the footer to the page's bottom edge.
        ui.scope_builder(egui::UiBuilder::new().max_rect(rects.body), |ui| {
            // The nav scope must be built BEFORE the pane scope: focus order
            // falls out of widget-creation order, and nothing calls
            // `request_focus` to pin it. Reordering these two silently
            // retargets Tab. `test_settings_focus_order_visits_every_nav_item_before_the_pane`
            // guards it.
            let nav_inner = egui::Rect::from_min_max(
                egui::pos2(rects.nav.left(), rects.nav.top() + NAV_TOP_INSET),
                rects.nav.right_bottom(),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(nav_inner), |ui| {
                for section in SettingsSection::ALL {
                    nav_item(ui, palette, section, section == current, &mut actions);
                }
            });

            // Hairline between nav and pane, immediately right of the nav so
            // it keeps touching the column it belongs to.
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rects.nav.right(), rects.body.top()),
                    egui::pos2(rects.nav.right() + NAV_HAIRLINE_W, rects.body.bottom()),
                ),
                0.0,
                palette.border,
            );

            // Current section's pane (dispatch added with the pane slices).
            ui.scope_builder(egui::UiBuilder::new().max_rect(rects.pane), |ui| {
                section_pane(ui, cache, palette, content, current, &mut actions);
            });
        });

        // Footer last, outside the scroll area, so the page's one destructive
        // action is offered by every section rather than only by Library.
        ui.scope_builder(egui::UiBuilder::new().max_rect(rects.footer), |ui| {
            library_footer(ui, cache, palette, content.reduce_motion, &mut actions);
        });
    });

    actions
}

/// The `ReplayGain Mode` card (Settings → Playback): the row's copy beside one
/// chip per Mode, the chosen one filled.
///
/// The chips are the whole control and carry no Off of their own — the
/// `ReplayGain` toggle above this card is what turns the feature off, and the
/// Mode only says which pair it applies when it is on.
fn replaygain_mode_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    settings_card(ui, palette, Some(4), |ui| {
        let copy = PREF_REPLAYGAIN_MODE;
        let modes = [
            ("Track", ReplayGainMode::Track),
            ("Album", ReplayGainMode::Album),
        ];
        let flow = row_flow(ui.available_width());
        let title_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM).clone();
        let desc_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS).clone();
        // Measured rather than assumed, as the format chips are: the text
        // column is whatever the two chips leave, so a wider font stack
        // narrows the copy instead of overwriting the choice. The measure is
        // the row's title size while `filled_button` paints the label at the
        // small button size — that difference is the air around each label.
        let chip_widths: Vec<f32> = modes
            .iter()
            .map(|(label, _)| {
                let label_w = ui
                    .painter()
                    .layout_no_wrap((*label).to_owned(), title_font.clone(), palette.ink)
                    .size()
                    .x;
                CHIP_LABEL_PAD * 2.0 + label_w
            })
            .collect();
        // Two chips, so exactly one gap between them — no index arithmetic.
        let chips_w = chip_widths.iter().sum::<f32>() + CHIP_GAP;

        let text_width = ui.available_width() - 2.0 * PREF_ROW_PAD - chips_w - PREF_ROW_TEXT_GAP;
        let title_galley =
            ui.painter()
                .layout_no_wrap(copy.0.to_owned(), title_font.clone(), palette.ink);
        let title_h = title_galley.size().y;
        let (row_h, desc_galley) = match flow {
            // The copy runs top-down in both flows — the description sits
            // under the title rather than pinned to the row's bottom edge,
            // the way the pass card's rows lay theirs out — and only the
            // stacked form trades row height for a wrapped column.
            RowFlow::Inline => (
                PREF_ROW_H,
                Some(ui.painter().layout_no_wrap(
                    copy.1.to_owned(),
                    desc_font.clone(),
                    palette.ink_3,
                )),
            ),
            RowFlow::Stacked => {
                let wrapped = ui.painter().layout(
                    copy.1.to_owned(),
                    desc_font.clone(),
                    palette.ink_3,
                    text_width,
                );
                (
                    preference_row_height(flow, title_h, wrapped.size().y),
                    Some(wrapped),
                )
            }
        };

        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_h),
            egui::Sense::hover(),
        );
        paint_preference_text(
            ui,
            palette,
            rect,
            &copy,
            title_font,
            desc_font,
            desc_galley,
            title_h,
        );

        // Right-aligned as a pair, so the row's right edge is the chips' even
        // as the labels change width.
        let mut chip_left = rect.right() - PREF_ROW_PAD - chips_w;
        for (i, (label, mode)) in modes.iter().enumerate() {
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(chip_left, rect.center().y - CHIP_H / 2.0),
                egui::vec2(chip_widths[i], CHIP_H),
            );
            let selected = content.replaygain_mode == *mode;
            if filled_button(
                ui,
                cache,
                palette,
                content.reduce_motion,
                chip_rect,
                egui::Id::new(("settings_replaygain_mode", *label)),
                label,
                label,
                None,
                if selected {
                    Variant::Primary
                } else {
                    Variant::Secondary
                },
                true,
                true,
            ) {
                actions.push(SettingsAction::SetReplayGainMode(*mode));
            }
            chip_left += chip_widths[i] + CHIP_GAP;
        }
    });
}

/// The library-wide `ReplayGain` Pass card (Settings → Advanced): two
/// persisted checkboxes choosing which value kinds a pass writes, one
/// session-local Force choice, and one button that starts the pass — disabled
/// while a pass runs, so a second one cannot start. While it runs the card
/// shows the polled progress and a cancel control; when one settles, the
/// outcome line stays until the next pass.
///
/// The checkboxes gate ONLY the library-wide pass — the automatic one after a
/// Library Scan included, and the on-demand one here. The Track-menu and
/// Album-menu commands are targeted passes that never read this state, so a
/// menu command always does exactly what its label says.
fn replaygain_pass_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    settings_card(ui, palette, Some(4), |ui| {
        for (label, desc, checked, action) in [
            (
                "Track values",
                "Measure every unmeasured Track's own gain and peak.",
                content.pass_track,
                SettingsAction::SetReplayGainPassTrack(!content.pass_track),
            ),
            (
                "Album values",
                "Also write each Album's aggregate to every Track of it.",
                content.pass_album,
                SettingsAction::SetReplayGainPassAlbum(!content.pass_album),
            ),
            (
                "Force",
                "Re-measure Tracks that are already measured. Not remembered \
                 between runs.",
                content.pass_force,
                SettingsAction::SetReplayGainPassForce(!content.pass_force),
            ),
        ] {
            replaygain_pass_check_row(ui, palette, label, desc, checked, action, actions);
        }
        // The rule about who wins, stated where the measurement happens
        // rather than tracked with a flag: the next measurement simply
        // overwrites whatever a hand edit held.
        let note_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS).clone();
        let (note_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), note_font.size * 1.6),
            egui::Sense::hover(),
        );
        ui.painter_at(note_rect).galley(
            egui::pos2(note_rect.left() + PREF_ROW_PAD, note_rect.top()),
            ui.painter().layout_no_wrap(
                "Measuring overwrites any manual edit.".to_owned(),
                note_font,
                palette.ink_3,
            ),
            palette.ink_3,
        );

        ui.add_space(8.0);
        row_separator(ui, palette, 8.0);
        ui.add_space(8.0);
        replaygain_pass_action_row(ui, cache, palette, content, actions);
    });
}

/// One checkbox row of the pass card: the shared checkbox box (with the
/// focus ring every other boolean control wears) beside a label and its
/// description, the whole row clickable like the Watch control.
fn replaygain_pass_check_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    label: &str,
    desc: &str,
    checked: bool,
    action: SettingsAction,
    actions: &mut Vec<SettingsAction>,
) {
    let check_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM).clone();
    let desc_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS).clone();

    let row_h = PREF_ROW_H / 2.0 + 8.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    let box_side = WATCH_BOX;
    let box_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + PREF_ROW_PAD + box_side / 2.0, rect.center().y),
        egui::vec2(box_side, box_side),
    );
    let text_left = box_rect.right() + PREF_ROW_TEXT_GAP;
    let painter = ui.painter_at(rect);
    painter.galley(
        egui::pos2(text_left, rect.top() + 4.0),
        painter.layout_no_wrap(label.to_owned(), check_font.clone(), palette.ink),
        palette.ink,
    );
    painter.galley(
        egui::pos2(text_left, rect.top() + 4.0 + check_font.size + 2.0),
        painter.layout_no_wrap(desc.to_owned(), desc_font, palette.ink_3),
        palette.ink_3,
    );
    let focused = ui.memory(|m| m.has_focus(response.id));
    super::toggle_switch::paint_square_checkbox_with_focus(
        ui.painter(),
        palette,
        box_rect,
        checked,
        focused,
    );
    super::toggle_switch::register_checkbox_a11y(&response, true, checked, label);
    if response.clicked() {
        actions.push(action);
    }
}

/// The pass card's action row: one button (disabled while a pass runs, so a
/// second one cannot start), the polled progress line while running with its
/// cancel control, and the settled outcome otherwise.
fn replaygain_pass_action_row(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SMALL_BTN_H),
        egui::Sense::hover(),
    );
    let button_w = 128.0;
    let button_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - PREF_ROW_PAD - button_w, rect.top()),
        egui::vec2(button_w, SMALL_BTN_H),
    );
    let running = content.pass_running;
    if filled_button(
        ui,
        cache,
        palette,
        content.reduce_motion,
        button_rect,
        egui::Id::new("settings_start_replaygain_pass"),
        if running {
            "Measuring\u{2026}"
        } else {
            "Measure Library"
        },
        if running {
            "Measuring\u{2026}"
        } else {
            "Measure Library"
        },
        None,
        Variant::Primary,
        true,
        !running,
    ) {
        actions.push(SettingsAction::StartReplayGainPass);
    }

    let line = if running {
        let (done, total) = content.pass_progress;
        if total > 0 {
            Some(format!("Measuring ReplayGain ({done}/{total})\u{2026}"))
        } else {
            Some("Measuring ReplayGain\u{2026}".to_string())
        }
    } else {
        content.pass_outcome.clone()
    };
    if let Some(text) = line {
        // The progress/outcome line is a real widget (not a bare paint), so
        // the status is readable by assistive tech and visible to the test
        // harness like every other label on this stage.
        let line_w = rect.width() - button_w - 3.0 * PREF_ROW_PAD;
        let line_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + PREF_ROW_PAD, rect.top()),
            egui::vec2(line_w, SMALL_BTN_H),
        );
        let text_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS).clone();
        let galley = ui.painter().layout_no_wrap(text, text_font, palette.ink_3);
        ui.put(line_rect, egui::Label::new(galley).truncate());
        // The cancel control sits beside the progress line while a pass
        // runs: it asks, never forces — the pass stops at its next
        // measurement boundary and keeps what it committed.
        if running {
            let cancel_w = 60.0;
            let cancel_rect = egui::Rect::from_min_size(
                egui::pos2(
                    button_rect.left() - 8.0 - cancel_w,
                    rect.center().y - SMALL_BTN_H / 2.0,
                ),
                egui::vec2(cancel_w, SMALL_BTN_H),
            );
            if filled_button(
                ui,
                cache,
                palette,
                content.reduce_motion,
                cancel_rect,
                egui::Id::new("settings_cancel_replaygain_pass"),
                "Cancel",
                "Cancel",
                None,
                Variant::Secondary,
                true,
                true,
            ) {
                actions.push(SettingsAction::CancelReplayGainPass);
            }
        }
    }
}

// --- App adapter --------------------------------------------------------------------

impl super::app::RiffApp {
    /// Apply one [`SettingsAction`] through the app's state/service/store
    /// paths.
    fn apply_settings_action(
        &mut self,
        action: SettingsAction,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        match action {
            SettingsAction::Back => library.view_mode = ViewMode::Library,
            SettingsAction::SelectSection(section) => self.settings_section = section,
            SettingsAction::AddLibrary => self.add_library_via_platform_picker(library),
            // Scan intent goes through the Library Scan Service seam (ADR
            // 0006): dedup against in-flight scans and the whole walk/commit
            // flow live behind it.
            SettingsAction::Scan(path) => self.scans.request(path),
            SettingsAction::ScanAll => {
                for path in library.library_paths.paths() {
                    self.scans.request(path.clone());
                }
            }
            SettingsAction::Remove(path) => {
                // One call owns all five facts: the list entry, the Readiness
                // slot, the Watch State, the live watcher, and the store rows.
                let mut watcher = self.watcher_manager.lock_or_recover();
                library.library_paths.retire(
                    &path,
                    &mut watcher,
                    self.settings_store.as_mut(),
                    self.library_mutations.as_mut(),
                );
            }
            SettingsAction::SetWatch(path, watching) => {
                let mut watcher = self.watcher_manager.lock_or_recover();
                library.library_paths.set_watch(
                    &path,
                    watching,
                    &mut watcher,
                    self.settings_store.as_mut(),
                );
            }
            SettingsAction::ClearLibrary => self.clear_library_confirm = true,
            SettingsAction::ClearThumbnailCache => {
                self.clear_thumbnail_cache_confirm = true;
            }
            SettingsAction::SetAdvanced(value) => {
                library.ui_flags.advanced_mode = value;
            }
            SettingsAction::SetHighContrast(value) => {
                library.ui_flags.high_contrast = value;
            }
            SettingsAction::SetReduceMotion(value) => {
                library.ui_flags.reduce_motion = value;
            }
            SettingsAction::SetReplayGain(value) => {
                playback.replaygain_enabled = value;
            }
            SettingsAction::SetReplayGainMode(mode) => {
                playback.replaygain_mode = mode;
            }
            // The two value-kind checkboxes are persisted with the rest of the
            // Library preferences by the frame's Settings round-trip; Force is
            // session-local and never joins that snapshot.
            SettingsAction::SetReplayGainPassTrack(value) => {
                library.pass_prefs.track_values = value;
            }
            SettingsAction::SetReplayGainPassAlbum(value) => {
                library.pass_prefs.album_values = value;
            }
            SettingsAction::SetReplayGainPassForce(value) => {
                self.pass_force_choice = value;
            }
            SettingsAction::StartReplayGainPass => {
                let prefs = &library.pass_prefs;
                self.passes.submit(PassCommand::LibraryWide {
                    track_values: prefs.track_values,
                    album_values: prefs.album_values,
                    force: self.pass_force_choice,
                });
            }
            SettingsAction::CancelReplayGainPass => self.passes.cancel(),
            SettingsAction::SetWatchAll(watching) => {
                // Every root in one batch with one durable write.
                let mut watcher = self.watcher_manager.lock_or_recover();
                library.library_paths.set_watching_for_all(
                    watching,
                    &mut watcher,
                    self.settings_store.as_mut(),
                );
            }
            SettingsAction::SetSkipHidden(value) => {
                library.scan_prefs.skip_hidden_files = value;
            }
            SettingsAction::SetFormat(extension, enabled) => {
                let prefs = &mut library.scan_prefs;
                if enabled && !prefs.scan_formats.iter().any(|f| f == &extension) {
                    prefs.scan_formats.push(extension);
                    // Restore the canonical AUDIO_EXTENSIONS order so the
                    // chips and the persisted list render stably.
                    prefs.scan_formats.sort_by_key(|format| {
                        AUDIO_EXTENSIONS
                            .iter()
                            .position(|candidate| candidate == format)
                            .unwrap_or(usize::MAX)
                    });
                } else if !enabled {
                    prefs.scan_formats.retain(|format| format != &extension);
                }
            }
            SettingsAction::SetReadEmbeddedArtwork(value) => {
                library.scan_prefs.read_embedded_artwork = value;
                // Drop the shared placeholder tile: the tracks behind it
                // were resolved as artless under the old policy, and only a
                // fresh request lets real art surface.
                self.evict_generated_covers();
            }
            SettingsAction::SetCloseQuitsApp(value) => {
                library.ui_flags.close_quits_app = value;
            }
        }
    }

    /// The inline confirmation for the destructive Clear Library action,
    /// rendered beneath the stage until confirmed or cancelled.
    fn render_clear_library_confirm(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        // The composition is the pure widget seam in [`crate::ui::prompts`]
        // (golden-image gap audit P1-7): the same pixels the golden pins.
        let palette = self.theme.active;
        let outcome = crate::ui::prompts::clear_library_confirm(
            ui,
            &mut self.icons,
            &palette,
            library.ui_flags.reduce_motion,
        );
        match outcome {
            Some(crate::ui::prompts::PromptOutcome::Confirm) => {
                self.clear_library_confirm = false;
                match self.library_mutations.clear_library() {
                    Ok(removed) => {
                        // The mutation adapter bumps the session generation;
                        // the mirror no longer tracks collection data. The
                        // fact-set is told too, so no root keeps claiming it
                        // is indexed after the wipe.
                        library.library_paths.clear_collection_data();
                        self.feedback.set_library(
                            format!(
                                "Library cleared ({removed} tracks removed). Rescan to rebuild."
                            ),
                            riff_backend::app::events::NoticeSeverity::Info,
                        );
                        library.scan_status = self.feedback.display_message();
                    }
                    Err(e) => {
                        tracing::error!("Failed to clear the library: {e}");
                        self.feedback.set_library(
                            "Failed to clear the library \u{2014} nothing was changed.".to_string(),
                            riff_backend::app::events::NoticeSeverity::Error,
                        );
                        library.scan_status = self.feedback.display_message();
                    }
                }
            }
            Some(crate::ui::prompts::PromptOutcome::Cancel) => {
                self.clear_library_confirm = false;
            }
            None => {}
        }
    }

    /// The inline confirmation for the destructive Clear Thumbnail cache action,
    /// shaped like [`Self::render_clear_library_confirm`]. What differs is what
    /// Confirm does: the deletion is handed to the cover worker rather than run on
    /// this thread, because `remove_dir_all` over a few hundred thousand entries
    /// takes seconds and a frame may not wait for it.
    fn render_clear_thumbnail_cache_confirm(&mut self, ui: &mut egui::Ui, reduce_motion: bool) {
        let palette = self.theme.active;
        let outcome = crate::ui::prompts::clear_thumbnail_cache_confirm(
            ui,
            &mut self.icons,
            &palette,
            reduce_motion,
        );
        match outcome {
            Some(crate::ui::prompts::PromptOutcome::Confirm) => {
                self.clear_thumbnail_cache_confirm = false;
                self.request_thumbnail_cache_clear();
            }
            Some(crate::ui::prompts::PromptOutcome::Cancel) => {
                self.clear_thumbnail_cache_confirm = false;
            }
            None => {}
        }
    }

    /// Render the sectioned Settings modal inside the shell's central panel
    /// and apply everything the user did this frame. The modal itself is a
    /// pure renderer ([`show_settings_modal`]); this adapter owns the
    /// effects: watcher start/stop, store mutations, scan requests through
    /// the Library Scan Service, and the platform folder-picker split.
    pub fn show_settings_view(
        &mut self,
        ui: &mut egui::Ui,
        library: &mut LibrarySession,
        playback: &mut PlaybackSession,
    ) {
        // Per-root indexed-track counts come from the store through the
        // Session Views seam (one bounded count read per row, invalidated by
        // generation bumps) — never the former in-memory mirror.
        let content = SettingsContent {
            libraries: library
                .library_paths
                .paths()
                .iter()
                .map(|path| LibraryRow {
                    path: path.clone(),
                    status: library.library_paths.readiness(path),
                    watch: library.library_paths.watch_state(path),
                    indexed_tracks: self.views.folder_track_count(path),
                })
                .collect(),
            advanced_mode: library.ui_flags.advanced_mode,
            high_contrast: library.ui_flags.high_contrast,
            reduce_motion: library.ui_flags.reduce_motion,
            replaygain_enabled: playback.replaygain_enabled,
            replaygain_mode: playback.replaygain_mode,
            watch_any: library.library_paths.watches_any(),
            skip_hidden_files: library.scan_prefs.skip_hidden_files,
            scan_formats: library.scan_prefs.scan_formats.clone(),
            read_embedded_artwork: library.scan_prefs.read_embedded_artwork,
            close_quits_app: library.ui_flags.close_quits_app,
            pass_track: library.pass_prefs.track_values,
            pass_album: library.pass_prefs.album_values,
            pass_force: self.pass_force_choice,
            pass_running: self.passes.is_running(),
            pass_progress: self.passes.poll_progress(),
            pass_outcome: self
                .last_pass_report
                .as_ref()
                .map(crate::ui::frame::pass_outcome_line),
            last_scan: self.views.last_full_scan_summary(),
        };

        let palette = self.theme.active;
        for action in show_settings_modal(
            ui,
            &mut self.icons,
            &palette,
            &content,
            self.settings_section,
        ) {
            self.apply_settings_action(action, library, playback);
        }

        // Transient rows beneath the stage column.
        #[cfg(target_os = "linux")]
        self.render_library_path_input(ui, library);
        if self.clear_library_confirm {
            self.render_clear_library_confirm(ui, library);
        }
        if self.clear_thumbnail_cache_confirm {
            self.render_clear_thumbnail_cache_confirm(ui, library.ui_flags.reduce_motion);
        }
    }
}
