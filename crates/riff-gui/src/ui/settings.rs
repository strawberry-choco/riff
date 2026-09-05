use crate::ui::icons::{Icon, IconCache};
use crate::ui::theme::{self, Palette};
use eframe::egui;
use riff_backend::app::MutexExt;
use riff_backend::app::state::{
    LibrarySession, LibraryStatus, PlaybackSession, ViewMode, WatchState,
};
use riff_backend::app::store::{
    AUDIO_EXTENSIONS, FullScanSummary, MissingArtworkStrategy, SettingsStore,
};
use std::path::{Path, PathBuf};

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

/// Register one library root: update the Library Session, then persist the path list
/// to the Application Store. A store write failure is logged (the in-memory
/// change stands so the session still works).
fn add_library_path(path: PathBuf, library: &mut LibrarySession, store: &mut dyn SettingsStore) {
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    if library.library_paths.contains(&canonical) {
        return;
    }
    library.library_paths.push(canonical.clone());
    library
        .library_statuses
        .entry(canonical.clone())
        .or_default();
    if let Err(e) = store.save_library_paths(&library.library_paths) {
        tracing::warn!("Failed to save library paths: {e}");
    }
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
    General,
    Library,
    Playback,
    Appearance,
    Shortcuts,
    TagEditing,
    Advanced,
    About,
}

impl SettingsSection {
    /// Every section in left-nav (and focus) order.
    pub const ALL: [SettingsSection; 8] = [
        Self::General,
        Self::Library,
        Self::Playback,
        Self::Appearance,
        Self::Shortcuts,
        Self::TagEditing,
        Self::Advanced,
        Self::About,
    ];

    /// The nav label, verbatim from the mockup.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Library => "Library",
            Self::Playback => "Playback",
            Self::Appearance => "Appearance",
            Self::Shortcuts => "Shortcuts",
            Self::TagEditing => "Tag editing",
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
pub const CLEAR_LIBRARY_NOTE: &str =
    "Clear the indexed collection and rebuild it on the next scan.";

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
/// Preference row copy verbatim from the mockup.
pub const PREF_REPLAYGAIN: (&str, &str) = (
    "ReplayGain",
    "Normalize loudness across tracks when available.",
);

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
/// Library pane preference copy. The mockup's only strategy renders as the
/// current choice; a second strategy would turn this row into a selector.
pub const PREF_MISSING_ART: (&str, &str) = (
    "Missing artwork",
    "Generated colour — a deterministic colour stand-in per item.",
);

/// The Library pane footer's note.
pub const FOOTER_NOTE: &str = "Changes apply immediately";
/// The Library pane footer's restore action.
pub const RESET_DEFAULTS_LABEL: &str = "Reset to defaults";
/// The Library pane footer's closing action.
pub const DONE_LABEL: &str = "Done";

/// The destructive ghost button's fill: transparent until hovered, then the
/// error token at the mockup's 10% (`hover:bg-destructive/10`). The idle
/// fill is derived from a token rather than a flat transparent constructor
/// (ADR 0004).
#[must_use]
pub fn destructive_ghost_fill(palette: &Palette, hovered: bool) -> egui::Color32 {
    if hovered {
        palette.error.gamma_multiply(0.1)
    } else {
        palette.error.gamma_multiply(0.0)
    }
}

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
    /// `ReplayGain` preference (drives the third toggle).
    pub replaygain_enabled: bool,
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
    /// What renders for Tracks and Albums with no artwork.
    pub missing_artwork_strategy: MissingArtworkStrategy,
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
    /// Set the Advanced mode preference.
    SetAdvanced(bool),
    /// Set the High contrast preference.
    SetHighContrast(bool),
    /// Set the `ReplayGain` preference.
    SetReplayGain(bool),
    /// Start or stop watching every configured root (the pane's
    /// "Watch for changes" toggle).
    SetWatchAll(bool),
    /// Set the "Skip hidden files" scan preference.
    SetSkipHidden(bool),
    /// Enable or disable one audio format's indexing (the format chips).
    SetFormat(String, bool),
    /// Set the "Read embedded artwork" preference.
    SetReadEmbeddedArtwork(bool),
    /// Restore the Library pane's preferences to their defaults.
    ResetLibraryDefaults,
}

// --- Mockup dimensions ---------------------------------------------------------

/// Gap between a section header and its card (`mb-4`): 16px.
const HEADER_GAP: f32 = 16.0;

/// Gap between sections (`mb-8` on each `<section>`): 32px.
const SECTION_GAP: f32 = 32.0;

/// Height of one library row (`px-4 py-3` over ~24px of content).
const LIBRARY_ROW_H: f32 = 48.0;

/// Height of the Add Library / Scan All actions row (`px-4 py-4`).
const ACTIONS_ROW_H: f32 = 64.0;

/// Height of one preference row (`px-4 py-3` over title + description).
const PREF_ROW_H: f32 = 60.0;

/// Height of the Clear Library note row (`mt-4`, single line).
const CLEAR_ROW_H: f32 = 28.0;

/// Status-dot diameter (`w-2 h-2`): 8px.
const DOT_SIZE: f32 = 8.0;

/// Secondary-button height (`px-3 py-1.5` at `text-xs`).
const SMALL_BTN_H: f32 = 27.0;

/// Primary/secondary action-button height (`px-4 py-2` at `text-sm`).
const ACTION_BTN_H: f32 = 34.0;

/// Trash affordance hit area (`w-7 h-7`): 28px square.
const TRASH_BTN: f32 = 28.0;

/// Watch checkbox square size (a native checkbox at xs text ≈ 14px).
const WATCH_BOX: f32 = 14.0;

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

/// A design-scale [`egui::FontId`] at `size`, riding the family the installed
/// token style mapped onto `key` (so weight families resolve even before the
/// vendored fonts are installed).
fn styled_font(ui: &egui::Ui, key: egui::TextStyle, size: f32) -> egui::FontId {
    let family = ui
        .style()
        .text_styles
        .get(&key)
        .map_or(egui::FontFamily::Proportional, |font| font.family.clone());
    egui::FontId::new(size, family)
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

/// A hand-painted filled button (primary brand or secondary surface) with an
/// optional leading glyph. Returns `true` on click when enabled. `label` is
/// the painted text; `a11y` feeds the accessibility tree (per-path controls
/// suffix their root so labels stay unique).
#[allow(clippy::too_many_arguments)]
fn filled_button(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    id: egui::Id,
    label: &str,
    a11y: &str,
    icon: Option<Icon>,
    primary: bool,
    small_text: bool,
    enabled: bool,
) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    let painter = ui.painter_at(rect);

    let (fill, ink, ring) = match (enabled, primary, response.hovered()) {
        (false, _, _) => (palette.surface, palette.ink_3, false),
        (true, true, _) => (palette.brand_primary, palette.on_brand, false),
        (true, false, false) => (palette.surface_2, palette.ink, false),
        (true, false, true) => (palette.surface_3, palette.ink, true),
    };
    painter.rect_filled(rect, theme::RADIUS_MD, fill);
    if ring {
        painter.rect_stroke(
            rect,
            theme::RADIUS_MD,
            egui::Stroke::new(1.0_f32, palette.focus_ring),
            egui::StrokeKind::Inside,
        );
    }

    let size = if small_text {
        theme::TEXT_XS
    } else {
        theme::TEXT_SM
    };
    let font = styled_font(ui, egui::TextStyle::Button, size);
    let galley = painter.layout_no_wrap(label.to_owned(), font, ink);
    let icon_w: f32 = if icon.is_some() { 16.0 + 8.0 } else { 0.0 };
    let mut x = rect.center().x - icon_w.midpoint(galley.size().x);
    if let Some(icon) = icon {
        let tex_id = cache.texture(ui.ctx(), icon, 16.0, ink);
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(x + 8.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        );
        painter.image(tex_id, icon_rect, UV_FULL, ink);
        x += icon_w;
    }
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );

    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, a11y));
    enabled && response.clicked()
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
fn libraries_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(theme::RADIUS_LG)
        .show(ui, |ui| {
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
                library_row(ui, cache, palette, row, actions);
                row_separator(ui, palette, 0.0);
            }
            actions_row(ui, cache, palette, actions);
        });
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
    row: &LibraryRow,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), LIBRARY_ROW_H),
        egui::Sense::hover(),
    );
    let path_str = row.path.to_string_lossy();
    let display = truncate_path(&path_str, 48);
    let remove_label = format!("Remove {path_str} from your libraries");
    let watch_label = format!("Watch {path_str}");
    let scan_label = format!("Scan {path_str}");

    library_row_path(ui, cache, palette, rect, row, &display);
    // Returns where the Scan button starts so the readiness pair can sit
    // gap-4 to its left.
    let scan_left = library_row_controls(
        ui,
        cache,
        palette,
        rect,
        row,
        &path_str,
        &remove_label,
        &watch_label,
        &scan_label,
        actions,
    );
    library_row_readiness(ui, palette, rect, row, scan_left);
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
) {
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;
    let is_unavailable = matches!(row.status, LibraryStatus::Unavailable);

    let folder_tex_id = cache.texture(ui.ctx(), Icon::Folder, 16.0, palette.ink_3);
    let folder_rect =
        egui::Rect::from_center_size(egui::pos2(rect.left() + 24.0, cy), egui::vec2(16.0, 16.0));
    painter.image(folder_tex_id, folder_rect, UV_FULL, palette.ink_3);

    let body_font = styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM);
    let path_color = if is_unavailable {
        palette.warning
    } else {
        palette.ink
    };
    let path_galley = painter.layout_no_wrap(display.to_owned(), body_font, path_color);
    let path_x = folder_rect.right() + 12.0;
    painter.galley(
        egui::pos2(path_x, cy - path_galley.size().y / 2.0),
        path_galley.clone(),
        path_color,
    );

    // The live per-folder track count (design-handoff issues 05 and 12):
    // muted, beside the path, so each row answers "how much lives here".
    let count_galley = painter.layout_no_wrap(
        format!("{} tracks", row.indexed_tracks),
        styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
        palette.ink_3,
    );
    painter.galley(
        egui::pos2(
            path_x + path_galley.size().x + 12.0,
            cy - count_galley.size().y / 2.0,
        ),
        count_galley,
        palette.ink_3,
    );
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

/// The row's right cluster (gap-4): trash | watch | scan. Returns the Scan
/// button's left edge. All per-path labels arrive precomputed by
/// [`library_row`]; the trash hover text is only built while hovered.
#[allow(clippy::too_many_arguments)]
fn library_row_controls(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    rect: egui::Rect,
    row: &LibraryRow,
    path_str: &str,
    remove_label: &str,
    watch_label: &str,
    scan_label: &str,
    actions: &mut Vec<SettingsAction>,
) -> f32 {
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;
    let is_unavailable = matches!(row.status, LibraryStatus::Unavailable);
    let is_scanning = matches!(row.status, LibraryStatus::Scanning { .. });

    // Trash ghost icon button, destructive on hover. The tooltip string is
    // only formatted while the pointer is actually over the control.
    let right = rect.right() - 16.0;
    let trash_rect = egui::Rect::from_min_size(
        egui::pos2(right - TRASH_BTN, cy - TRASH_BTN / 2.0),
        egui::vec2(TRASH_BTN, TRASH_BTN),
    );
    let mut trash_response = ui.interact(
        trash_rect,
        egui::Id::new(("settings_trash", path_str)),
        egui::Sense::click(),
    );
    if trash_response.hovered() {
        trash_response = trash_response.on_hover_text(remove_label.to_owned());
    }
    let trash_tint = if trash_response.hovered() {
        palette.error
    } else {
        palette.ink_3
    };
    let trash_tex_id = cache.texture(ui.ctx(), Icon::Trash, 16.0, trash_tint);
    painter.image(trash_tex_id, trash_rect.shrink(6.0), UV_FULL, trash_tint);
    trash_response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Remove library")
    });
    if trash_response.clicked() {
        actions.push(SettingsAction::Remove(row.path.clone()));
    }

    // Watch checkbox: muted xs label + a small painted checkbox. Disabled
    // while the watcher is warning or the root is gone, exactly as before.
    let watch_label_w = 38.0;
    let watch_w = watch_label_w + 6.0 + WATCH_BOX;
    let watch_rect = egui::Rect::from_min_size(
        egui::pos2(trash_rect.left() - 16.0 - watch_w, cy - WATCH_BOX / 2.0),
        egui::vec2(watch_w, WATCH_BOX),
    );
    let watch_warning = matches!(row.watch, WatchState::Warning(_));
    let can_watch = !watch_warning && !is_unavailable;
    let watching = row.watch == WatchState::Enabled;
    let watch_response = ui.interact(
        watch_rect.expand2(egui::vec2(0.0, (LIBRARY_ROW_H - WATCH_BOX) / 2.0)),
        egui::Id::new(("settings_watch", path_str)),
        egui::Sense::click(),
    );
    let box_rect = egui::Rect::from_min_size(
        egui::pos2(watch_rect.right() - WATCH_BOX, watch_rect.top()),
        egui::vec2(WATCH_BOX, WATCH_BOX),
    );
    painter.text(
        egui::pos2(watch_rect.left(), watch_rect.center().y),
        egui::Align2::LEFT_CENTER,
        "Watch",
        styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
        palette.ink_3,
    );
    paint_watch_box(&painter, palette, box_rect, watching && can_watch);
    watch_response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, can_watch, watching, watch_label)
    });
    if can_watch && watch_response.clicked() {
        actions.push(SettingsAction::SetWatch(row.path.clone(), !watching));
    }
    if let WatchState::Warning(ref reason) = row.watch {
        watch_response.on_hover_text(reason.clone());
    }

    // Scan secondary button.
    let scan_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
    let scan_label_w = painter
        .layout_no_wrap("Scan".to_owned(), scan_font, palette.ink)
        .size()
        .x;
    let scan_rect = egui::Rect::from_min_size(
        egui::pos2(
            watch_rect.left() - 16.0 - (24.0 + scan_label_w),
            cy - SMALL_BTN_H / 2.0,
        ),
        egui::vec2(24.0 + scan_label_w, SMALL_BTN_H),
    );
    let scan_enabled = !is_scanning && !is_unavailable;
    if filled_button(
        ui,
        cache,
        palette,
        scan_rect,
        egui::Id::new(("settings_scan", path_str)),
        "Scan",
        scan_label,
        None,
        false,
        true,
        scan_enabled,
    ) {
        actions.push(SettingsAction::Scan(row.path.clone()));
    }
    scan_rect.left()
}

/// The small painted Watch checkbox: brand fill + two-stroke checkmark when
/// checked-and-enabled, input-well otherwise.
fn paint_watch_box(
    painter: &egui::Painter,
    palette: &Palette,
    box_rect: egui::Rect,
    checked: bool,
) {
    painter.rect_filled(
        box_rect,
        theme::RADIUS_SM,
        if checked {
            palette.brand_primary
        } else {
            palette.surface_2
        },
    );
    painter.rect_stroke(
        box_rect,
        theme::RADIUS_SM,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );
    if checked {
        let a = egui::pos2(
            box_rect.left() + WATCH_BOX * 0.25,
            box_rect.center().y + WATCH_BOX * 0.05,
        );
        let b = egui::pos2(
            box_rect.left() + WATCH_BOX * 0.42,
            box_rect.bottom() - WATCH_BOX * 0.25,
        );
        let c = egui::pos2(
            box_rect.right() - WATCH_BOX * 0.2,
            box_rect.top() + WATCH_BOX * 0.25,
        );
        let check = egui::Stroke::new(1.5_f32, palette.on_brand);
        painter.line_segment([a, b], check);
        painter.line_segment([b, c], check);
    }
}

/// The readiness dot + label at the far left of the control cluster
/// (gap-2 inside the pair, gap-4 before it).
fn library_row_readiness(
    ui: &mut egui::Ui,
    palette: &Palette,
    rect: egui::Rect,
    row: &LibraryRow,
    scan_left: f32,
) {
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;
    let ready = row.readiness();
    let label_font = styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS);
    let label_galley = painter.layout_no_wrap(ready.label().to_owned(), label_font, palette.ink_3);
    let cluster_left = scan_left - 16.0 - (DOT_SIZE + 8.0 + label_galley.size().x);
    painter.circle_filled(
        egui::pos2(cluster_left + DOT_SIZE / 2.0, cy),
        DOT_SIZE / 2.0,
        ready.dot_color(palette),
    );
    painter.galley(
        egui::pos2(
            cluster_left + DOT_SIZE + 8.0,
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
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ACTIONS_ROW_H),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;

    let add_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let add_label_w = painter
        .layout_no_wrap("Add Library".to_owned(), add_font, palette.on_brand)
        .size()
        .x;
    let add_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 16.0, cy - ACTION_BTN_H / 2.0),
        egui::vec2(16.0 + 8.0 + add_label_w + 32.0, ACTION_BTN_H),
    );
    if filled_button(
        ui,
        cache,
        palette,
        add_rect,
        egui::Id::new("settings_add_library"),
        "Add Library",
        "Add Library",
        Some(Icon::Plus),
        true,
        false,
        true,
    ) {
        actions.push(SettingsAction::AddLibrary);
    }

    let scan_all_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let scan_all_label_w = painter
        .layout_no_wrap("Scan All".to_owned(), scan_all_font, palette.ink)
        .size()
        .x;
    let scan_all_rect = egui::Rect::from_min_size(
        egui::pos2(add_rect.right() + 12.0, cy - ACTION_BTN_H / 2.0),
        egui::vec2(16.0 + 8.0 + scan_all_label_w + 32.0, ACTION_BTN_H),
    );
    if filled_button(
        ui,
        cache,
        palette,
        scan_all_rect,
        egui::Id::new("settings_scan_all"),
        "Scan All",
        "Scan All",
        Some(Icon::RefreshCw),
        false,
        false,
        true,
    ) {
        actions.push(SettingsAction::ScanAll);
    }
}

/// The destructive ghost action under the libraries card: muted note on the
/// left, error-tinted ghost button on the right.
fn clear_row(ui: &mut egui::Ui, palette: &Palette, actions: &mut Vec<SettingsAction>) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), CLEAR_ROW_H),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;

    painter.text(
        egui::pos2(rect.left() + 4.0, cy),
        egui::Align2::LEFT_CENTER,
        CLEAR_LIBRARY_NOTE,
        styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
        palette.ink_3,
    );

    let font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
    let galley = painter.layout_no_wrap(CLEAR_LIBRARY_LABEL.to_owned(), font, palette.error);
    let btn_w = galley.size().x + 24.0;
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - btn_w - 4.0, cy - SMALL_BTN_H / 2.0),
        egui::vec2(btn_w, SMALL_BTN_H),
    );
    let response = ui.interact(
        btn_rect,
        egui::Id::new("settings_clear_library"),
        egui::Sense::click(),
    );
    painter.rect_filled(
        btn_rect,
        theme::RADIUS_MD,
        destructive_ghost_fill(palette, response.hovered()),
    );
    painter.galley(
        egui::pos2(
            btn_rect.center().x - galley.size().x / 2.0,
            cy - galley.size().y / 2.0,
        ),
        galley,
        palette.error,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, CLEAR_LIBRARY_LABEL)
    });
    if response.clicked() {
        actions.push(SettingsAction::ClearLibrary);
    }
}

/// Height of one format chip (the small secondary-button geometry).
const CHIP_H: f32 = 27.0;

/// Horizontal padding inside a format chip around its label.
const CHIP_LABEL_PAD: f32 = 12.0;

/// Gap between adjacent format chips.
const CHIP_GAP: f32 = 8.0;

/// Height of the last-full-scan card.
const SCAN_CARD_H: f32 = 76.0;

/// Height of the pane footer's action row.
const FOOTER_H: f32 = 48.0;

/// The format chips card: one toggle chip per [`AUDIO_EXTENSIONS`] entry;
/// enabled formats are indexed on the next scan (design-handoff issue 12).
fn formats_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(theme::RADIUS_LG)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), CHIP_H),
                egui::Sense::hover(),
            );
            let painter = ui.painter_at(rect);
            let mut x = rect.left();
            for extension in AUDIO_EXTENSIONS {
                let enabled = content.scan_formats.iter().any(|f| f == extension);
                let label = extension.to_uppercase();
                let font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
                let label_w = painter
                    .layout_no_wrap(label.clone(), font, palette.ink)
                    .size()
                    .x;
                let chip_w = CHIP_LABEL_PAD * 2.0 + label_w;
                let chip_rect = egui::Rect::from_min_size(
                    egui::pos2(x, rect.top()),
                    egui::vec2(chip_w, CHIP_H),
                );
                let a11y = format!("Index {extension} files");
                if filled_button(
                    ui,
                    cache,
                    palette,
                    chip_rect,
                    egui::Id::new(("settings_format_chip", *extension)),
                    &label,
                    &a11y,
                    None,
                    enabled,
                    true,
                    true,
                ) {
                    actions.push(SettingsAction::SetFormat(
                        (*extension).to_string(),
                        !enabled,
                    ));
                }
                x += chip_w + CHIP_GAP;
            }
        });
}

/// The last-full-scan card: when the scan finished and what it saw, with a
/// Rescan now action (design-handoff issue 12).
fn scan_status_card(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    actions: &mut Vec<SettingsAction>,
) {
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(theme::RADIUS_LG)
        .show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), SCAN_CARD_H),
                egui::Sense::hover(),
            );
            let painter = ui.painter_at(rect);
            let cy = rect.center().y;

            let (stamp_line, counts_line) = match content.last_scan {
                Some(summary) => {
                    let elapsed = summary.at.elapsed().unwrap_or_default();
                    (
                        format!(
                            "Last full scan {}",
                            crate::ui::sidebar::format_last_scan_ago(elapsed)
                        ),
                        format!(
                            "{} files indexed \u{b7} {} errors",
                            summary.files, summary.errors
                        ),
                    )
                }
                None => (
                    String::from("No full scan recorded yet"),
                    String::from("Run a scan to index your music folders."),
                ),
            };

            painter.text(
                egui::pos2(rect.left() + 16.0, cy - 10.0),
                egui::Align2::LEFT_CENTER,
                stamp_line,
                styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
                palette.ink,
            );
            painter.text(
                egui::pos2(rect.left() + 16.0, cy + 10.0),
                egui::Align2::LEFT_CENTER,
                counts_line,
                styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
                palette.ink_3,
            );

            let btn_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
            let btn_label_w = painter
                .layout_no_wrap("Rescan now".to_owned(), btn_font, palette.ink)
                .size()
                .x;
            let btn_rect = egui::Rect::from_min_size(
                egui::pos2(
                    rect.right() - 16.0 - (24.0 + btn_label_w),
                    cy - SMALL_BTN_H / 2.0,
                ),
                egui::vec2(24.0 + btn_label_w, SMALL_BTN_H),
            );
            if filled_button(
                ui,
                cache,
                palette,
                btn_rect,
                egui::Id::new("settings_rescan_now"),
                "Rescan now",
                "Rescan now",
                Some(Icon::RefreshCw),
                false,
                true,
                true,
            ) {
                actions.push(SettingsAction::ScanAll);
            }
        });
}

/// The Missing artwork strategy row: title + description on the left, the
/// current strategy as a static brand chip on the right. One strategy ships
/// today; a second would turn this into a selector.
fn strategy_row(ui: &mut egui::Ui, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), PREF_ROW_H),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.text(
        egui::pos2(rect.left() + 16.0, rect.top() + 11.0),
        egui::Align2::LEFT_TOP,
        PREF_MISSING_ART.0,
        styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
        palette.ink,
    );
    painter.text(
        egui::pos2(rect.left() + 16.0, rect.bottom() - 11.0),
        egui::Align2::LEFT_BOTTOM,
        PREF_MISSING_ART.1,
        styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
        palette.ink_3,
    );

    let label = "Generated colour";
    let font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS);
    let label_w = painter
        .layout_no_wrap(label.to_owned(), font, palette.on_brand)
        .size()
        .x;
    let chip_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 16.0 - label_w / 2.0 - 12.0, rect.center().y),
        egui::vec2(label_w + 24.0, SMALL_BTN_H),
    );
    painter.rect_filled(chip_rect, theme::RADIUS_MD, palette.brand_primary);
    painter.text(
        chip_rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        styled_font(ui, egui::TextStyle::Button, theme::TEXT_XS),
        palette.on_brand,
    );
}

/// The Library pane footer: the immediate-apply note on the left, the
/// Reset-to-defaults (secondary) and Done (primary) actions on the right.
fn library_footer(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), FOOTER_H),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let cy = rect.center().y;

    painter.text(
        egui::pos2(rect.left() + 4.0, cy),
        egui::Align2::LEFT_CENTER,
        FOOTER_NOTE,
        styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
        palette.ink_3,
    );

    // Done (primary) hugs the right edge; Reset sits to its left.
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
    if filled_button(
        ui,
        cache,
        palette,
        done_rect,
        egui::Id::new("settings_done"),
        DONE_LABEL,
        DONE_LABEL,
        None,
        true,
        false,
        true,
    ) {
        actions.push(SettingsAction::Back);
    }

    let reset_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let reset_w = painter
        .layout_no_wrap(RESET_DEFAULTS_LABEL.to_owned(), reset_font, palette.ink)
        .size()
        .x
        + 32.0;
    let reset_rect = egui::Rect::from_min_size(
        egui::pos2(done_rect.left() - 12.0 - reset_w, cy - ACTION_BTN_H / 2.0),
        egui::vec2(reset_w, ACTION_BTN_H),
    );
    if filled_button(
        ui,
        cache,
        palette,
        reset_rect,
        egui::Id::new("settings_reset_defaults"),
        RESET_DEFAULTS_LABEL,
        RESET_DEFAULTS_LABEL,
        None,
        false,
        false,
        true,
    ) {
        actions.push(SettingsAction::ResetLibraryDefaults);
    }
}

/// The boolean preferences the stage drives through the reusable toggle
/// switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preference {
    Advanced,
    HighContrast,
    ReplayGain,
    WatchChanges,
    SkipHidden,
    ReadEmbedded,
}

impl Preference {
    /// `(title, description)` copy verbatim from the mockup.
    fn copy(self) -> (&'static str, &'static str) {
        match self {
            Self::Advanced => PREF_ADVANCED,
            Self::HighContrast => PREF_HIGH_CONTRAST,
            Self::ReplayGain => PREF_REPLAYGAIN,
            Self::WatchChanges => PREF_WATCH_CHANGES,
            Self::SkipHidden => PREF_SKIP_HIDDEN,
            Self::ReadEmbedded => PREF_READ_EMBEDDED,
        }
    }

    /// The action reporting `value` for this preference.
    fn action(self, value: bool) -> SettingsAction {
        match self {
            Self::Advanced => SettingsAction::SetAdvanced(value),
            Self::HighContrast => SettingsAction::SetHighContrast(value),
            Self::ReplayGain => SettingsAction::SetReplayGain(value),
            Self::WatchChanges => SettingsAction::SetWatchAll(value),
            Self::SkipHidden => SettingsAction::SetSkipHidden(value),
            Self::ReadEmbedded => SettingsAction::SetReadEmbeddedArtwork(value),
        }
    }

    /// Stable widget-id stem.
    fn id(self) -> &'static str {
        match self {
            Self::Advanced => "pref_advanced",
            Self::HighContrast => "pref_high_contrast",
            Self::ReplayGain => "pref_replaygain",
            Self::WatchChanges => "pref_watch_changes",
            Self::SkipHidden => "pref_skip_hidden",
            Self::ReadEmbedded => "pref_read_embedded",
        }
    }

    /// The preference's current persisted value.
    fn checked(self, content: &SettingsContent) -> bool {
        match self {
            Self::Advanced => content.advanced_mode,
            Self::HighContrast => content.high_contrast,
            Self::ReplayGain => content.replaygain_enabled,
            Self::WatchChanges => content.watch_any,
            Self::SkipHidden => content.skip_hidden_files,
            Self::ReadEmbedded => content.read_embedded_artwork,
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
    egui::Frame::new()
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(theme::RADIUS_LG)
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            for (i, pref) in prefs.iter().enumerate() {
                if i > 0 {
                    row_separator(ui, palette, 16.0);
                }
                preference_row(ui, palette, *pref, pref.checked(content), actions);
            }
        });
}

/// One preference row: title + muted description on the left, the toggle
/// switch on the right. The whole row is clickable, like the mockup's
/// wrapping `<label>`.
fn preference_row(
    ui: &mut egui::Ui,
    palette: &Palette,
    pref: Preference,
    checked: bool,
    actions: &mut Vec<SettingsAction>,
) {
    let copy = pref.copy();
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), PREF_ROW_H),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);
    if rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
        painter.rect_filled(rect, theme::RADIUS_MD, palette.surface_2);
    }

    painter.text(
        egui::pos2(rect.left() + 16.0, rect.top() + 11.0),
        egui::Align2::LEFT_TOP,
        copy.0,
        styled_font(ui, egui::TextStyle::Body, theme::TEXT_SM),
        palette.ink,
    );
    painter.text(
        egui::pos2(rect.left() + 16.0, rect.bottom() - 11.0),
        egui::Align2::LEFT_BOTTOM,
        copy.1,
        styled_font(ui, egui::TextStyle::Small, theme::TEXT_XS),
        palette.ink_3,
    );

    let pill_rect = egui::Rect::from_center_size(
        egui::pos2(
            rect.right() - 16.0 - super::toggle_switch::TOGGLE_W / 2.0,
            rect.center().y,
        ),
        egui::vec2(
            super::toggle_switch::TOGGLE_W,
            super::toggle_switch::TOGGLE_H,
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

/// The Advanced & platform info section: muted factual lines. The tray line
/// exists only where a tray exists (decision 002); the picker line covers
/// both platform splits in one sentence, as the mockup writes it.
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

// --- Sectioned modal (issue 11) --------------------------------------------------

/// Modal card width cap (`max-w-3xl`-ish).
const MODAL_MAX_W: f32 = 760.0;

/// Modal card height cap.
const MODAL_MAX_H: f32 = 600.0;

/// Backdrop margin around the card (`p-8`).
const MODAL_PAD: f32 = 32.0;

/// Header height (title row + close control).
const MODAL_HEADER_H: f32 = 56.0;

/// Left-nav column width.
const NAV_W: f32 = 180.0;

/// One left-nav row's height (`py-2` at text-sm).
const NAV_ITEM_H: f32 = 32.0;

/// Draw the sectioned Settings modal (Issue 11): a centered card with a
/// header, a left nav listing [`SettingsSection::ALL`], and the current
/// section's pane. Must run inside the shell's central stage panel; reports
/// every interaction as [`SettingsAction`]s so the app adapter applies them
/// through its state/command/store paths.
pub fn show_settings_modal(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    content: &SettingsContent,
    current: SettingsSection,
) -> Vec<SettingsAction> {
    let mut actions = Vec::new();

    // Center the card in the stage (a vertical layout never scrolls
    // horizontally, so the cursor's left is the stage's left edge).
    let avail = ui.available_size();
    let card_w = (avail.x - 2.0 * MODAL_PAD).clamp(320.0, MODAL_MAX_W);
    let card_h = (avail.y - 2.0 * MODAL_PAD).clamp(240.0, MODAL_MAX_H);
    let x0 = ui.cursor().left() + ((avail.x - card_w) * 0.5).max(0.0);
    let y0 = ui.cursor().top() + ((avail.y - card_h) * 0.5).max(0.0);
    let card = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(card_w, card_h));

    let painter = ui.painter_at(card);
    painter.rect_filled(card, theme::RADIUS_LG, palette.surface);
    painter.rect_stroke(
        card,
        theme::RADIUS_LG,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );

    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        modal_header(ui, cache, palette, &mut actions);
        let body_top = ui.cursor().top();
        let body_h = card.bottom() - body_top;

        // Left nav strip (below the header — anchored to the card's top it
        // would paint its first item over the title).
        let nav =
            egui::Rect::from_min_size(egui::pos2(card.left(), body_top), egui::vec2(NAV_W, body_h));
        let nav_inner = egui::Rect::from_min_max(
            egui::pos2(nav.left(), nav.top() + 8.0),
            egui::pos2(nav.right(), nav.bottom()),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav_inner), |ui| {
            for section in SettingsSection::ALL {
                nav_item(ui, palette, section, section == current, &mut actions);
            }
        });
        // Hairline between nav and pane.
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(nav.right(), body_top),
                egui::pos2(nav.right() + 1.0, card.bottom()),
            ),
            0.0,
            palette.border,
        );

        // Current section's pane (dispatch added with the pane slices).
        let pane = egui::Rect::from_min_max(egui::pos2(nav.right() + 1.0, body_top), card.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(pane), |ui| {
            section_pane(ui, cache, palette, content, current, &mut actions);
        });
    });

    actions
}

/// The modal's header: the "Settings" xl heading with a bordered close
/// (arrow-left) control whose activation reports [`SettingsAction::Back`].
fn modal_header(
    ui: &mut egui::Ui,
    cache: &mut IconCache,
    palette: &Palette,
    actions: &mut Vec<SettingsAction>,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), MODAL_HEADER_H),
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

    // Close control at the header's right edge, hugging its content.
    let body_font = styled_font(ui, egui::TextStyle::Button, theme::TEXT_SM);
    let label_galley = painter.layout_no_wrap("Back".to_owned(), body_font, palette.ink_2);
    let btn_w = 16.0 + 8.0 + label_galley.size().x + 12.0;
    let btn_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - btn_w - 12.0, rect.center().y - 16.0),
        egui::vec2(btn_w, 32.0),
    );
    let response = ui.interact(
        btn_rect,
        egui::Id::new("settings_back"),
        egui::Sense::click(),
    );
    painter.rect_filled(
        btn_rect,
        theme::RADIUS_MD,
        if response.hovered() {
            palette.surface_2
        } else {
            palette.surface
        },
    );
    painter.rect_stroke(
        btn_rect,
        theme::RADIUS_MD,
        egui::Stroke::new(1.0_f32, palette.border),
        egui::StrokeKind::Inside,
    );
    let tint = if response.hovered() {
        palette.ink
    } else {
        palette.ink_2
    };
    let tex_id = cache.texture(ui.ctx(), Icon::ArrowLeft, 16.0, tint);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(btn_rect.left() + 12.0 + 8.0, btn_rect.center().y),
        egui::vec2(16.0, 16.0),
    );
    painter.image(tex_id, icon_rect, UV_FULL, tint);
    painter.galley(
        egui::pos2(
            icon_rect.right() + 8.0,
            btn_rect.center().y - label_galley.size().y / 2.0,
        ),
        label_galley,
        tint,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Back to Library")
    });
    if response.clicked() {
        actions.push(SettingsAction::Back);
    }
}

/// One left-nav row. The active section is visually indicated: a surface fill
/// plus a brand accent bar and full ink, against muted ink for the rest.
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

/// Pane inset around its content.
const PANE_PAD: i8 = 24;

/// The right pane's content for the current section. Sections with existing
/// content show it; the rest show a clear placeholder (full implementations
/// are later tickets).
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
                .inner_margin(egui::Margin::same(PANE_PAD))
                .show(ui, |ui| match current {
                    SettingsSection::Library => {
                        section_header(ui, palette, SECTION_LIBRARIES);
                        ui.add_space(HEADER_GAP);
                        libraries_card(ui, cache, palette, content, actions);
                        ui.add_space(HEADER_GAP);
                        clear_row(ui, palette, actions);

                        ui.add_space(SECTION_GAP);
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
                        egui::Frame::new()
                            .fill(palette.surface)
                            .stroke(egui::Stroke::new(1.0_f32, palette.border))
                            .corner_radius(theme::RADIUS_LG)
                            .inner_margin(egui::Margin::same(4))
                            .show(ui, |ui| {
                                preference_row(
                                    ui,
                                    palette,
                                    Preference::ReadEmbedded,
                                    content.read_embedded_artwork,
                                    actions,
                                );
                                row_separator(ui, palette, 16.0);
                                strategy_row(ui, palette);
                            });

                        ui.add_space(SECTION_GAP);
                        library_footer(ui, cache, palette, actions);
                    }
                    SettingsSection::Advanced => {
                        preferences_card(ui, palette, content, actions, &[Preference::Advanced]);
                        ui.add_space(SECTION_GAP);
                        section_header(ui, palette, SECTION_ADVANCED_INFO);
                        ui.add_space(HEADER_GAP);
                        info_lines(ui, palette);
                    }
                    SettingsSection::Playback => {
                        preferences_card(ui, palette, content, actions, &[Preference::ReplayGain]);
                    }
                    SettingsSection::Appearance => {
                        preferences_card(
                            ui,
                            palette,
                            content,
                            actions,
                            &[Preference::HighContrast],
                        );
                    }
                    _ => {
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

// --- Linux-only folder picker -------------------------------------------------------

/// Text-based folder picker with directory autocomplete (no native dialog on
/// Linux). Rendered under the stage when the Add Library flow is open.
#[cfg(target_os = "linux")]
fn pick_folder_ui(
    ui: &mut egui::Ui,
    library: &mut LibrarySession,
    store: &mut dyn SettingsStore,
    text_input: &mut String,
    show_input: &mut bool,
    path_error: &mut Option<String>,
) {
    if *show_input {
        if let Some(ref err) = *path_error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        }
        ui.horizontal(|ui| {
            ui.label("Path:");
            ui.text_edit_singleline(text_input);
            if ui.button("Confirm").clicked() {
                let path = expand_tilde(text_input);
                if !path.exists() {
                    *path_error = Some(format!("Path does not exist: {}", path.display()));
                } else if !path.is_dir() {
                    *path_error = Some(format!("Not a directory: {}", path.display()));
                } else {
                    add_library_path(path, library, store);
                    *text_input = String::new();
                    *show_input = false;
                    *path_error = None;
                }
            }
            if ui.button("Cancel").clicked() {
                *text_input = String::new();
                *show_input = false;
                *path_error = None;
            }
        });

        // Directory autocomplete: clickable suggestions that fill the input so
        // the user can drill down without a native dialog. Clicking appends a
        // separator, which lists that directory's children on the next frame.
        for suggestion in suggest_directories(text_input.as_str(), 8) {
            let label = suggestion.to_string_lossy().to_string();
            if ui
                .selectable_label(false, format!("\u{1F4C1} {label}"))
                .clicked()
            {
                *text_input = format!("{label}/");
                *path_error = None;
            }
        }
    }
}

// --- App adapter --------------------------------------------------------------------

impl super::app::RiffApp {
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
        // Session Views facade (component-wise subtree ids, invalidated by
        // generation bumps) — never the former in-memory mirror.
        let content = SettingsContent {
            libraries: library
                .library_paths
                .iter()
                .map(|path| {
                    let indexed_tracks = self.views.folder_subtree_ids(path).len();
                    LibraryRow {
                        path: path.clone(),
                        status: library
                            .library_statuses
                            .get(path)
                            .cloned()
                            .unwrap_or_default(),
                        watch: library.watch_states.get(path).cloned().unwrap_or_default(),
                        indexed_tracks,
                    }
                })
                .collect(),
            advanced_mode: library.ui_flags.advanced_mode,
            high_contrast: library.ui_flags.high_contrast,
            replaygain_enabled: playback.replaygain_enabled,
            watch_any: library
                .library_paths
                .iter()
                .any(|path| library.watch_states.get(path) == Some(&WatchState::Enabled)),
            skip_hidden_files: library.scan_prefs.skip_hidden_files,
            scan_formats: library.scan_prefs.scan_formats.clone(),
            read_embedded_artwork: library.scan_prefs.read_embedded_artwork,
            missing_artwork_strategy: library.scan_prefs.missing_artwork_strategy,
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
        if self.settings_show_input {
            pick_folder_ui(
                ui,
                library,
                self.settings_store.as_mut(),
                &mut self.settings_text_input,
                &mut self.settings_show_input,
                &mut self.settings_path_error,
            );
        }
        if self.clear_library_confirm {
            self.render_clear_library_confirm(ui, library);
        }
    }

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
                for path in &library.library_paths {
                    self.scans.request(path.clone());
                }
            }
            SettingsAction::Remove(path) => self.remove_library_path(&path, library),
            SettingsAction::SetWatch(path, watching) => {
                self.set_watch_state(&path, watching, library);
            }
            SettingsAction::ClearLibrary => self.clear_library_confirm = true,
            SettingsAction::SetAdvanced(value) => {
                library.ui_flags.advanced_mode = value;
                self.persist_scalars(playback, library);
            }
            SettingsAction::SetHighContrast(value) => {
                library.ui_flags.high_contrast = value;
                self.persist_scalars(playback, library);
            }
            SettingsAction::SetReplayGain(value) => {
                playback.replaygain_enabled = value;
                self.persist_scalars(playback, library);
            }
            SettingsAction::SetWatchAll(watching) => {
                let paths = library.library_paths.clone();
                for path in paths {
                    self.set_watch_state(&path, watching, library);
                }
            }
            SettingsAction::SetSkipHidden(value) => {
                library.scan_prefs.skip_hidden_files = value;
                self.persist_scalars(playback, library);
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
                self.persist_scalars(playback, library);
            }
            SettingsAction::SetReadEmbeddedArtwork(value) => {
                library.scan_prefs.read_embedded_artwork = value;
                self.persist_scalars(playback, library);
                // Drop the generated cover blocks (issue 14): the tracks
                // behind them were resolved as artless under the old
                // policy, and only a fresh request lets real art surface.
                self.evict_generated_covers();
            }
            SettingsAction::ResetLibraryDefaults => {
                library.scan_prefs = riff_backend::app::state::ScanPrefs::default();
                let paths = library.library_paths.clone();
                // Re-enable watching for every root so the restored pane
                // matches its default of watching configured folders.
                for path in paths {
                    self.set_watch_state(&path, true, library);
                }
                self.persist_scalars(playback, library);
            }
        }
    }

    /// Register a new library root through the platform picker: the native
    /// folder dialog everywhere except Linux, which opens the text-input row
    /// rendered beneath the stage. Also called from the sidebar footer
    /// (design-handoff issue 07).
    pub(crate) fn add_library_via_platform_picker(&mut self, library: &mut LibrarySession) {
        #[cfg(not(target_os = "linux"))]
        {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Add Music Library")
                .pick_folder()
            {
                add_library_path(path, library, self.settings_store.as_mut());
            }
        }
        #[cfg(target_os = "linux")]
        {
            self.settings_show_input = true;
            self.settings_path_error = None;
            let _ = library;
        }
    }

    /// Remove one library root: one durable store transaction drops the
    /// root's tracks, orphaned parents, and the path record (playlist entries
    /// survive dangling so they recover when files return); the mutation
    /// adapter bumps the session generation so projections refetch, then the
    /// session state catches up.
    fn remove_library_path(&mut self, path: &PathBuf, library: &mut LibrarySession) {
        if let Err(e) = self.library_mutations.remove_library_path(path) {
            tracing::error!("Failed to remove {path:?} from store: {e}");
        }
        library.library_paths.retain(|p| p != path);
        library.library_statuses.remove(path);
        if let Err(e) = self
            .settings_store
            .save_library_paths(&library.library_paths)
        {
            tracing::warn!("Failed to save library paths: {e}");
        }
    }

    /// Start or stop the filesystem watcher for one root and persist the new
    /// [`WatchState`]. A failed start degrades to a Warning carrying the
    /// diagnostic, exactly as before the restyle.
    fn set_watch_state(&mut self, path: &Path, watching: bool, library: &mut LibrarySession) {
        if watching {
            let result = {
                let mut guard = self.watcher_manager.lock_or_recover();
                guard.as_mut().map_or_else(
                    || Err("Watcher not initialized".to_string()),
                    |mgr| mgr.start_watching(path),
                )
            };
            match result {
                Ok(()) => {
                    library
                        .watch_states
                        .insert(path.to_path_buf(), WatchState::Enabled);
                }
                Err(reason) => {
                    library
                        .watch_states
                        .insert(path.to_path_buf(), WatchState::Warning(reason));
                }
            }
        } else {
            if let Some(ref mut mgr) = *self.watcher_manager.lock_or_recover() {
                mgr.stop_watching(path);
            }
            library
                .watch_states
                .insert(path.to_path_buf(), WatchState::Disabled);
        }
        if let Err(e) = self.settings_store.save_watch_states(&library.watch_states) {
            tracing::warn!("Failed to save watch states: {e}");
        }
    }

    /// The inline confirmation for the destructive Clear Library action,
    /// rendered beneath the stage until confirmed or cancelled.
    fn render_clear_library_confirm(&mut self, ui: &mut egui::Ui, library: &mut LibrarySession) {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Remove every indexed track? Playlists and settings are kept.")
                .color(ui.visuals().warn_fg_color),
        );
        ui.horizontal(|ui| {
            if ui.button("Confirm").clicked() {
                self.clear_library_confirm = false;
                match self.library_mutations.clear_library() {
                    Ok(removed) => {
                        // The mutation adapter bumps the session generation;
                        // the mirror no longer tracks collection data.
                        library.scan_status = Some(format!(
                            "Library cleared ({removed} tracks removed). Rescan to rebuild."
                        ));
                    }
                    Err(e) => {
                        tracing::error!("Failed to clear the library: {e}");
                        library.scan_status = Some(
                            "Failed to clear the library \u{2014} nothing was changed.".to_string(),
                        );
                    }
                }
            }
            if ui.button("Cancel").clicked() {
                self.clear_library_confirm = false;
            }
        });
    }

    /// Commit the current scalar preferences as one small durable
    /// transaction.
    fn persist_scalars(&mut self, playback: &PlaybackSession, library: &LibrarySession) {
        let repeat_mode = match playback.queue.repeat {
            riff_backend::domain::RepeatMode::None => 0,
            riff_backend::domain::RepeatMode::All => 1,
            riff_backend::domain::RepeatMode::One => 2,
        };
        let scalars = riff_backend::app::state::ScalarSettings {
            volume: Some(playback.current_volume),
            advanced_mode: library.ui_flags.advanced_mode,
            high_contrast: library.ui_flags.high_contrast,
            replaygain_enabled: playback.replaygain_enabled,
            shuffle: playback.queue.shuffle,
            repeat_mode,
            browser_layout: library.browser_layout.as_store_code(),
            skip_hidden_files: library.scan_prefs.skip_hidden_files,
            scan_formats: library.scan_prefs.scan_formats.clone(),
            read_embedded_artwork: library.scan_prefs.read_embedded_artwork,
            missing_artwork_strategy: library.scan_prefs.missing_artwork_strategy,
        };
        if let Err(e) = self.settings_store.save_scalars(&scalars) {
            tracing::warn!("Failed to save settings: {e}");
        }
    }
}
