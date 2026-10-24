//! Application-layer ports for the `SQLite` `Application Store`.
//!
//! The `Application Store` is the single authoritative persistent state of the
//! application (`Library`, `Playlists`, `Settings`). These ports keep the app layer
//! free of infrastructure types: infrastructure implements them over a shared
//! `rusqlite` connection, and the UI never imports `rusqlite` directly.

use crate::errors::StoreError;
use crate::playlist::{Playlist, PlaylistId};
use crate::rule::{RootBox, RuleBox, RuleChild, RuleValue};
use crate::track::{Album, Artist, GenreCount, SmartPlaylistKind, Track, TrackId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Per-root filesystem watching state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WatchState {
    #[default]
    Disabled,
    Enabled,
    Warning(String),
}

/// The audio file extensions the Library Scan indexes, lowercase, no dots —
/// the single source shared by the scanner and the Settings Library pane's
/// format chips (design-handoff issue 12).
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg", "flac", "wav"];

/// The Library Scan's behavior knobs, projected from the persisted
/// [`ScalarSettings`] and handed to the filesystem walk as an explicit
/// value (design-handoff issue 12). The default is the persisted row's own
/// default, so the historical behavior (hidden entries skipped, every
/// [`AUDIO_EXTENSIONS`] format indexed) is spelled once, in the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    /// Leave dot-prefixed files and directories out of the walk.
    pub skip_hidden_files: bool,
    /// The enabled audio extensions, lowercase without dots.
    pub formats: Vec<String>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self::from(&ScalarSettings::default())
    }
}

impl From<&ScalarSettings> for ScanOptions {
    fn from(scalars: &ScalarSettings) -> Self {
        Self {
            skip_hidden_files: scalars.skip_hidden_files,
            formats: scalars.scan_formats.clone(),
        }
    }
}

/// One completed full Library Scan's recorded summary (design-handoff
/// issue 12): when it finished and what it saw, served back by
/// [`LibraryQueryStore::last_full_scan`] for the Settings Library pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullScanSummary {
    /// When the scan completed.
    pub at: SystemTime,
    /// Audio files the scan discovered under the scanned root(s).
    pub files: usize,
    /// Discovered files that could not be indexed (unreadable metadata);
    /// they are skipped, never committed.
    pub errors: usize,
}

/// The single-row scalar preferences persisted in the Application Store's
/// typed settings table. Volume is `None` while the user has not yet moved
/// the slider, so the caller applies its own default.
///
/// `repeat_mode` encodes the playback repeat cycle as an integer so this
/// crate stays dependency-free: 0 = off, 1 = repeat all, 2 = repeat one.
///
/// The Library scan preferences (design-handoff issue 12): `skip_hidden_files`
/// leaves dot-entries out of the scan walk, `scan_formats` lists the enabled
/// audio extensions (subsets of [`AUDIO_EXTENSIONS`]), and
/// `read_embedded_artwork` drives cover resolution.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the struct mirrors the scalar settings row's typed columns"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct ScalarSettings {
    pub volume: Option<f32>,
    pub advanced_mode: bool,
    pub high_contrast: bool,
    /// `true` = the Smart Lists sidebar section is folded away (persisted
    /// display preference, restored on launch).
    pub smart_lists_collapsed: bool,
    pub replaygain_enabled: bool,
    /// Which `ReplayGain` pair playback levels at, encoded as an integer so
    /// this crate stays dependency-free exactly like `repeat_mode` above:
    /// `0` = the Track pair (the default), `1` = the Album pair.
    pub replaygain_mode: i64,
    pub shuffle: bool,
    pub repeat_mode: i64,
    /// Skip hidden (dot-prefixed) files and directories during scans.
    pub skip_hidden_files: bool,
    /// The enabled audio extensions, lowercase without dots. Defaults to
    /// every entry of [`AUDIO_EXTENSIONS`].
    pub scan_formats: Vec<String>,
    /// Read artwork embedded in track tags before filesystem fallbacks.
    pub read_embedded_artwork: bool,
    /// Whether the custom title-bar close button quits the app instead of
    /// minimizing it to the system tray. `false` (the default) preserves the
    /// minimize-to-tray behavior; inert on Linux, which has no tray.
    pub close_quits_app: bool,
    /// The `ReplayGain` Pass's Settings gating: which value kinds a
    /// library-wide pass writes, both on demand and automatically after
    /// every Library Scan. Both default off, so a scan's behavior is
    /// unchanged until the listener opts in. Force is deliberately NOT
    /// persisted — an automatic pass never redoes finished work.
    pub replaygain_pass_track: bool,
    pub replaygain_pass_album: bool,
    /// Accessibility flag: when `true` the UI suppresses non-essential motion
    /// (view transitions, animated reveals) in favor of instant changes,
    /// honouring the listener's system reduce-motion preference. `false`
    /// (the default) leaves motion as it is.
    pub reduce_motion: bool,
}

impl Default for ScalarSettings {
    /// The scanner's historical behavior: hidden files skipped, every
    /// supported format indexed, embedded artwork read. The title-bar close
    /// button minimizes to the tray unless the user opts into quitting.
    fn default() -> Self {
        Self {
            volume: None,
            advanced_mode: false,
            high_contrast: false,
            smart_lists_collapsed: false,
            replaygain_enabled: false,
            replaygain_mode: 0,
            shuffle: false,
            repeat_mode: 0,
            skip_hidden_files: true,
            scan_formats: AUDIO_EXTENSIONS
                .iter()
                .map(|extension| (*extension).to_string())
                .collect(),
            read_embedded_artwork: true,
            close_quits_app: false,
            replaygain_pass_track: false,
            replaygain_pass_album: false,
            reduce_motion: false,
        }
    }
}

/// Age threshold for the Lost Gems smart playlist: tracks whose last play is
/// older than this are considered forgotten gems worth resurfacing. Tracks
/// that were never played qualify unconditionally ("unheard" includes
/// never-heard). Lives beside the [`LibraryQueryStore::smart_playlist`] port
/// because it parameterizes that query's semantics; the SQL implementation
/// in infrastructure reads it from here.
pub const LOST_GEMS_THRESHOLD: Duration = Duration::from_hours(2160);

/// The user preferences the settings surface owns: single-row scalar values,
/// library paths, and per-path watch states.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub scalars: ScalarSettings,
    /// Library root paths in registration order.
    pub library_paths: Vec<PathBuf>,
    /// Per-path watch state; paths absent from this map default to
    /// [`WatchState::Disabled`].
    pub watch_states: HashMap<PathBuf, WatchState>,
}

/// Port for bringing the `Application Store` schema up to date.
///
/// Implemented by infrastructure over a real `SQLite` connection. Opening the
/// store and running migrations are fatal-startup operations: failures carry a
/// clear message and must never be silently swallowed.
pub trait StoreMigrations {
    /// Open (creating it if needed) the store at `path`, configure the
    /// connection for durability, and apply every pending migration.
    fn open_and_migrate(&self, path: &std::path::Path) -> Result<(), StoreError>;
}

/// Port for reading and writing the Settings section of the `Application
/// Store`.
///
/// Every setter commits as one small durable transaction so a crash right
/// after a change cannot lose it. Implemented by infrastructure over a real
/// `SQLite` connection; app-layer orchestration tests drive this port through
/// mocks.
pub trait SettingsStore {
    /// Load every persisted setting. Missing values yield their defaults:
    /// volume `None` (the caller applies its own fallback), toggles off,
    /// empty path list, no watch states.
    fn load_settings(&self) -> Result<Settings, StoreError>;

    /// Persist the scalar block as one small durable transaction.
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), StoreError>;

    /// Replace the library-path list as one small durable transaction.
    fn save_library_paths(&mut self, paths: &[PathBuf]) -> Result<(), StoreError>;

    /// Replace the whole watch-state map as one small durable transaction.
    fn save_watch_states(
        &mut self,
        states: &HashMap<PathBuf, WatchState>,
    ) -> Result<(), StoreError>;
}

/// One user-playlist entry together with its Library validity: `valid` is
/// true exactly when the referenced Track is still present in the
/// Application Store's Library collection, computed by a SQL LEFT JOIN
/// against tracks. A dangling reference — an entry whose file left the
/// library — stays listed with `valid == false` and its `track` unset;
/// that is product behavior per ADR 0001, not corruption, and it resolves
/// again once the file returns via a rescan. Whether an otherwise known
/// track's file still exists on disk is a filesystem concern no store query
/// can answer: callers apply that check on top of `valid` where playback
/// semantics require it (see `playlist_manager`).
#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    /// The referenced [`TrackId`] (its full file path).
    pub id: TrackId,
    /// The resolved Track when the Library knows it, `None` when dangling.
    pub track: Option<Track>,
    /// Whether the Library collection still contains this entry's track.
    pub valid: bool,
}

/// Port for reading and writing the Playlists section of the `Application
/// Store`.
///
/// Every mutation (create, rename, delete, add entry, remove entries)
/// commits as one immediate durable transaction, so a crash right after the
/// action cannot lose it. Entries are ordered Track references that carry no
/// enforced link to tracks: dangling references stay listed and resolve again
/// once the referenced files return. Implemented by infrastructure over a
/// real `SQLite` connection.
///
/// A name is how the listener finds its row, so it is unique across the whole
/// sidebar — `playlists` and `smart_playlists` together — and both writes that
/// carry a name ([`PlaylistStore::create_playlist`] and
/// [`PlaylistStore::rename_playlist`]) enforce that one rule. It cannot be a
/// table constraint, because no single table holds both kinds of row.
pub trait PlaylistStore {
    /// Load every Playlist in creation order, each with its entries in
    /// playlist order. A fresh store yields an empty `Vec`.
    fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError>;

    /// Load one Playlist's entries in playlist order, each with its Library
    /// validity flag computed by a SQL LEFT JOIN against tracks (see
    /// [`PlaylistEntry`]). Dangling references stay listed with
    /// `valid == false` — ADR 0001 product behavior, never silently
    /// dropped. Unknown playlist ids yield an empty `Vec`.
    fn load_playlist_entries(&self, id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError>;

    /// Create a Playlist named `name` (trimmed) with optional initial Track
    /// references (exact duplicates dropped, order preserved). The generated
    /// id is made unique against existing playlists by suffixing its slug, so
    /// two distinct names that slug alike in one millisecond cannot collide.
    ///
    /// A name is how the listener finds its sidebar row, so one name belongs
    /// to one row across the WHOLE sidebar: creation is refused when any
    /// Playlist or Custom Smart Playlist already answers to `name`, compared
    /// trimmed and lowercased with Rust's full Unicode folding — see
    /// [`Self::rename_playlist`], which enforces the same rule on the other
    /// half of the sidebar. Duplicate names are therefore not allowed, in
    /// either table, under either spelling; `smart_playlists`' own `UNIQUE`
    /// constraint is only a same-case backstop that cannot see across the two
    /// tables. The six Built-in Smart Playlist display names are not part of
    /// this rule — they exist only as code, so refusing them is the
    /// application layer's job.
    fn create_playlist(
        &mut self,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, StoreError>;

    /// Rename the Playlist with `id` to `new_name` (trimmed). Returns
    /// whether the playlist was found. The same sidebar-wide name rule as
    /// [`Self::create_playlist`] applies, with this Playlist's own row exempt
    /// so a rename that changes nothing never refuses the row it started from.
    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, StoreError>;

    /// Delete the Playlist with `id` together with its entries. Returns
    /// whether anything was removed.
    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError>;

    /// Append `track` to the Playlist with `id`. Exact duplicates are
    /// ignored (returns `false`), as are unknown playlist ids.
    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, StoreError>;

    /// Remove all occurrences of `track` from the Playlist with `id`.
    /// Returns whether anything was removed.
    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, StoreError>;

    /// Persist a new entry order for the Playlist with `id` as ONE immediate
    /// durable transaction: the entries are rewritten to exactly `ordered`
    /// (position 0..n). `ordered` must be a permutation of the playlist's
    /// current entries — callers derive it from loaded state (e.g. the UI's
    /// drag-reorder math), never invent ids. Returns whether the playlist
    /// was found; unknown ids change nothing.
    fn reorder_playlist_entries(
        &mut self,
        id: &PlaylistId,
        ordered: &[TrackId],
    ) -> Result<bool, StoreError>;

    /// The session playlist generation this handle bumps after each
    /// committed playlist mutation (ADR 0002), independent of the Library
    /// generation so entry edits never invalidate Library projections. A
    /// handle shares the implementor's own counter: clones answer the same
    /// counter, so a mutation committed through any clone of the handle is
    /// seen by every holder of the answer.
    ///
    /// This is the read the UI's single read seam wires with: it derives the
    /// counter its Session Projections observe from the ports it keeps, so
    /// the same-store-same-instant rule is enforced by the constructor's
    /// shape rather than re-derived by every wiring site. Widening the port
    /// by this one read is recorded as an ADR 0012 consideration, not a
    /// conflict: it formalizes a read the Composition Root and every test
    /// suite already make through the concrete store type, rather than
    /// growing the contract's vocabulary. It takes no fallible shape — a
    /// wiring read has no failure mode to degrade to.
    #[must_use]
    fn playlist_generation(&self) -> StoreGeneration;
}

/// Session-local monotonically increasing counter bumped after each committed
/// Store mutation (ADR 0002). Session Projections compare the generation they
/// were loaded at against [`Self::current`] and refetch when it moved. The
/// counter lives in memory only and resets on launch; it is never persisted.
#[derive(Clone, Debug, Default)]
pub struct StoreGeneration(Arc<AtomicU64>);

impl StoreGeneration {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    /// Record a committed mutation and return the new generation value.
    pub fn bump(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The current generation; projections reload when this differs from the
    /// generation their cached rows were loaded at.
    pub fn current(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Generic generation-keyed cache slot — the ONE implementation of "drop
/// when the epoch moved" behind every Session Projection (ADR 0002).
///
/// Holds at most one `(epoch, key, value)` entry stamped with the session
/// [`StoreGeneration`] epoch it was loaded at. Per operation a caller
/// observes the counter exactly once ([`Self::observe`]), serves the cached
/// value while [`Self::holds`] holds, and commits a fresh load through
/// [`Self::store`] or [`Self::slot`] only AFTER its loader succeeded — a
/// failed load leaves the previous entry untouched, so stale-but-present
/// data survives to the next retry.
///
/// Seam machinery: projections own private instances and no code outside a
/// projection's implementation may observe an epoch value. The canonical
/// type lives here beside the counter it keys on, so the library, playback,
/// and app-layer read seams share one audited staleness contract.
pub struct GenerationCache<K, V> {
    /// The session counter this cache is keyed on.
    counter: StoreGeneration,
    /// The single cached entry, if any.
    loaded: Option<LoadedEntry<K, V>>,
}

/// One epoch-stamped cache entry.
struct LoadedEntry<K, V> {
    epoch: u64,
    key: K,
    value: V,
}

impl<K, V> GenerationCache<K, V> {
    /// Key a cache on `counter`; starts empty.
    pub fn new(counter: StoreGeneration) -> Self {
        Self {
            counter,
            loaded: None,
        }
    }

    /// The epoch currently being observed. Read ONCE per operation and
    /// reuse it for both the freshness check and the commit stamp, so a
    /// store commit racing mid-frame cannot split one logical read across
    /// two generations.
    pub fn observe(&self) -> u64 {
        self.counter.current()
    }

    /// Whether an entry is present and stamped with `epoch`.
    pub fn loaded_at(&self, epoch: u64) -> bool {
        self.loaded
            .as_ref()
            .is_some_and(|entry| entry.epoch == epoch)
    }

    /// Whether an entry is present, stamped with `epoch`, and keyed `key`.
    pub fn holds(&self, epoch: u64, key: &K) -> bool
    where
        K: PartialEq,
    {
        self.loaded
            .as_ref()
            .is_some_and(|entry| entry.epoch == epoch && entry.key == *key)
    }

    /// The cached value regardless of epoch — the stale-but-present error
    /// fallback reads through here. Callers that need freshness guard the
    /// read themselves: `if cache.loaded_at(cache.observe()) { cache.peek() }`.
    pub fn peek(&self) -> Option<&V> {
        self.loaded.as_ref().map(|entry| &entry.value)
    }

    /// Drop the cached entry whatever it is stamped with.
    pub fn invalidate(&mut self) {
        self.loaded = None;
    }

    /// Commit `value` as loaded at `epoch` for `key`. The stamp happens
    /// here and nowhere else, so a failed load never advances it.
    pub fn store(&mut self, epoch: u64, key: K, value: V) {
        self.loaded = Some(LoadedEntry { epoch, key, value });
    }

    /// The mutable entry slot stamped `epoch` for `key`: drops any entry
    /// from a different epoch or key and hands out a freshly initialized
    /// default one. The commit step for lazily-filled multi-level bundles —
    /// call only after a successful load.
    pub fn slot(&mut self, epoch: u64, key: &K) -> &mut V
    where
        V: Default,
        K: Clone + PartialEq,
    {
        if !self.holds(epoch, key) {
            self.loaded = Some(LoadedEntry {
                epoch,
                key: key.clone(),
                value: V::default(),
            });
        }
        &mut self.loaded.as_mut().expect("entry just ensured").value
    }
}

// --- Custom Smart Playlist Rule trees ---------------------------------------
//
// A Custom Smart Playlist is its Rule tree: one Root Box of Rules and nested
// Rule Boxes, stored one row per Rule and one per Rule Box by the Application
// Store, and written by [`LibraryMutationStore::save_smart_playlist`] wholesale
// from the editor's draft. This is the type that crosses that boundary in both
// directions — the read hands the editor back exactly what a save takes — and
// the reason it is not simply a [`crate::rule::RootBox`] is that a stored Rule
// can name a field or an operator this build does not know.
//
// Rule and Rule Box ids are generated by the store, are opaque, and never
// appear here: a draft therefore carries no identity that can go stale, and
// anything that has to notice a tree changed keys on the tree's shape.

/// One stored Rule this build cannot interpret, kept as the row's own raw ids.
///
/// An Unreadable Rule has no [`crate::rule::Rule`] representation — the field
/// and operator enums are closed, and `from_stored_id` answering `None` is
/// precisely the detection — so the raw ids ride *beside* the tree rather than
/// inside it. Nothing here repairs, coerces or drops such a row: a save refuses
/// while one is in the draft (re-inserting it would violate the schema's `CHECK`
/// list and fail the transaction with no explanation), and the row's own delete
/// control in the editor is the only way out.
///
/// `box_path` and `position` say where it sits: the path of nested Rule Boxes
/// below the Root Box, counted in *box* children only (Rules are not counted,
/// because this tree holds none of them), plus the row's own stored sibling
/// position. That position can leave a gap in the typed tree's numbering, which
/// is exactly what an unreadable row is — a sibling the typed tree cannot show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableRule {
    box_path: Vec<usize>,
    position: usize,
    field_id: String,
    operator_id: String,
    negated: bool,
    value: Option<String>,
}

impl UnreadableRule {
    /// The row as stored: where it hangs, and the five columns that say what it
    /// asks. `value` is the raw `TEXT` exactly as it is held, never re-typed.
    #[must_use]
    pub fn new(
        box_path: Vec<usize>,
        position: usize,
        field_id: String,
        operator_id: String,
        negated: bool,
        value: Option<String>,
    ) -> Self {
        Self {
            box_path,
            position,
            field_id,
            operator_id,
            negated,
            value,
        }
    }

    /// The nested boxes this row hangs inside, from the Root Box downward, as
    /// indices into each box's *box* children. Empty for a row in the Root Box.
    #[must_use]
    pub fn box_path(&self) -> &[usize] {
        &self.box_path
    }

    /// The row's stored sibling position among the Rules and Boxes under its
    /// parent — the number that puts it back in the editor's one interleaved
    /// list.
    #[must_use]
    pub fn position(&self) -> usize {
        self.position
    }

    /// The stored `field_id`, verbatim: an id no [`crate::rule::MatchableField`]
    /// is, or a display name some other writer spelled instead of an id.
    #[must_use]
    pub fn field_id(&self) -> &str {
        &self.field_id
    }

    /// The stored `operator_id`, verbatim.
    #[must_use]
    pub fn operator_id(&self) -> &str {
        &self.operator_id
    }

    /// The stored `negated` flag.
    #[must_use]
    pub const fn is_negated(&self) -> bool {
        self.negated
    }

    /// The stored `value` column, raw, or `None` when it is `NULL`.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

/// A Custom Smart Playlist's whole Rule tree, plus any Rule the store holds
/// that this build cannot read.
///
/// This is what a save writes and what a read returns, so a round-trip is
/// provable: an editor that changes nothing saves exactly what it was given and
/// gets the same value back. The [`crate::rule::RootBox`] is the interpretable
/// tree; [`Self::unreadable`] is the store's honest report that the tree it read
/// is not wholly interpretable — never a repair, never a silent drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartPlaylistTree {
    root: crate::rule::RootBox,
    unreadable: Vec<UnreadableRule>,
}

impl SmartPlaylistTree {
    /// The tree a listener authored, with nothing unreadable in it.
    #[must_use]
    pub fn new(root: crate::rule::RootBox) -> Self {
        Self {
            root,
            unreadable: Vec::new(),
        }
    }

    /// The same tree with the rows this build cannot interpret set beside it —
    /// the store read's own constructor.
    #[must_use]
    pub fn with_unreadable(mut self, unreadable: Vec<UnreadableRule>) -> Self {
        self.unreadable = unreadable;
        self
    }

    /// The Root Box, and through it every Rule and nested Rule Box in the
    /// listener's one shared sibling order.
    #[must_use]
    pub fn root(&self) -> &crate::rule::RootBox {
        &self.root
    }

    /// The stored Rules this build cannot interpret, in tree order.
    #[must_use]
    pub fn unreadable(&self) -> &[UnreadableRule] {
        &self.unreadable
    }

    /// How many Rules this build can see. The unreadable rows are deliberately
    /// not counted: a tree of four rows of which one cannot be read holds three
    /// Rules as far as riff is concerned, and says so separately.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.root.rule_count()
    }
}

/// One Custom Smart Playlist as the surfaces that *enumerate* lists identify
/// it: the store's id, its saved name, and when it was first saved.
///
/// This is the row half of a Custom Smart Playlist — what
/// [`LibraryQueryStore::custom_smart_playlists`] answers. It deliberately
/// carries no Tracks and no Rule tree: which Tracks match is a separate
/// bounded read, and the tree is [`LibraryQueryStore::smart_playlist_tree`].
/// A consumer that needs more than id, name and creation order is holding the
/// wrong read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomSmartPlaylist {
    /// The store-generated id — what Save minted, never reused.
    pub id: PlaylistId,
    /// The list's saved name, as stored (already trimmed by the write path).
    pub name: String,
    /// When the row was first created. A rename or a re-save never moves it,
    /// which is what makes creation order stable.
    pub created_at: SystemTime,
}

impl CustomSmartPlaylist {
    /// One enumerated list, as the store's row holds it — the read's own
    /// constructor.
    #[must_use]
    pub fn new(id: PlaylistId, name: String, created_at: SystemTime) -> Self {
        Self {
            id,
            name,
            created_at,
        }
    }
}

/// Port for writing the Library collection section of the Application Store.
///
/// Implemented by infrastructure over a real `SQLite` connection. Every call
/// commits as one small durable transaction so an interrupted scan keeps all
/// previously committed batches (spec user story 3).
pub trait LibraryMutationStore {
    /// Upsert scanned Tracks as ONE immediate durable transaction — callers
    /// pass about ten tracks per scan batch. Creates missing Artist/Album
    /// parents (album identity is `(album artist, title)`; year/genre derive
    /// from the first-added track), writes the derived Rust-lowercased
    /// search text, and preserves existing play history (`play_count`,
    /// `last_played`, `date_added`) when a track already exists. Rescanning
    /// known paths therefore changes nothing observable. Returns the number
    /// of tracks written.
    ///
    /// Also serves single-track metadata persistence (e.g. a tag edit): the
    /// same upsert semantics apply.
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError>;

    /// Record one completed play for `id` as ONE immediate durable
    /// transaction: `play_count` increments and `last_played` moves to
    /// `played_at` together or not at all, so a crash right after a finished
    /// track cannot lose the play. Returns whether the track was known;
    /// unknown ids change nothing.
    fn record_track_played(
        &mut self,
        id: &TrackId,
        played_at: SystemTime,
    ) -> Result<bool, StoreError>;

    /// Set or clear `id`'s favorite flag as ONE immediate durable
    /// transaction, so a crash right after a toggle cannot lose it. Returns
    /// whether a change was committed: the track was known AND its flag
    /// state actually moved — unknown ids and redundant sets change nothing
    /// (and invalidate no projections).
    fn set_track_favorite(&mut self, id: &TrackId, favorite: bool) -> Result<bool, StoreError>;

    /// The targeted tag-edit refresh: upsert `track`'s metadata as ONE
    /// immediate durable transaction without ever touching its play history
    /// (`play_count`, `last_played`, `date_added`). Unlike a plain rescan,
    /// this flow re-derives every affected album's year/genre from its
    /// first-added remaining track and cleans up albums left empty (and
    /// artists left with no albums) when the edit moves the track to another
    /// album.
    fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError>;

    /// Remove everything belonging to the library root `root` as ONE
    /// immediate durable transaction: exactly that root's tracks (byte-prefix
    /// match on the track path), albums left without tracks, artists left
    /// without albums, and the root's own `library_paths` record. Playlist
    /// entries referencing removed tracks survive as dangling references and
    /// stay listed until the files return. Returns the number of tracks
    /// removed; an unknown root removes nothing.
    fn remove_library_path(&mut self, root: &std::path::Path) -> Result<usize, StoreError>;

    /// The maintenance wipe: delete the Library collection section's
    /// contents — every track (with its play history), album, and artist —
    /// as ONE immediate durable transaction, so a failure mid-clear cannot
    /// leave partial deletion. Playlists and Settings tables are untouched:
    /// playlist entries referencing wiped tracks survive dangling and stay
    /// listed until the files return via a rescan. Returns the number of
    /// tracks removed.
    fn clear_library(&mut self) -> Result<usize, StoreError>;

    /// Record the completion of one full library scan as ONE immediate
    /// durable transaction: the summary overwrites any previous one in the
    /// store's metadata table, and the committed write bumps + notifies the
    /// session Library generation like every Library mutation, so the read
    /// models refetch (design-handoff issues 05 and 12).
    fn record_full_scan_completed(&mut self, summary: FullScanSummary) -> Result<(), StoreError>;

    /// Record that this store's metadata now matches `version`, as ONE
    /// immediate durable transaction. The Library Scan calls it only after a
    /// scan has run to completion: a cancelled or failed scan leaves the
    /// store behind, so the next scan finishes the re-read rather than
    /// believing a partial one.
    fn stamp_metadata_version(&mut self, version: u32) -> Result<(), StoreError>;

    // --- Custom Smart Playlist writes -------------------------------------
    //
    // Four writes, one transaction each, in the same shape as the playlist
    // mutations above. They live on this port because a Custom Smart Playlist
    // is Library collection data, and every one of them moves the session
    // **playlist** generation while it is here — the pairing is odd and is
    // intended: what these writes change is a list's identity and its Rule tree,
    // which is what the playlist counter keys on, while *which Tracks match*
    // moves with the Library generation because membership is a query over
    // Tracks. Nothing on either counter is a third counter.

    /// Write a Custom Smart Playlist's name and whole Rule tree as ONE
    /// immediate durable transaction.
    ///
    /// **Creation is not a separate path**: `id` of `None` inserts the
    /// `smart_playlists` row — which is why the sidebar's `+` writes nothing and
    /// the list first appears in the sidebar on this call — and returns its new
    /// store-generated id. With `id` of `Some` the row is updated **in place**:
    /// its id and `created_at` survive, and its `name` is written, trimmed, from
    /// this call's `name`. That is deliberate and load-bearing: the editor's name
    /// field and its Rule tree are one listener action taken in one form, so both
    /// land in this transaction or neither does. A save that left the name to a
    /// second call could half-apply — the name lands and the tree fails, or the
    /// tree saves and the rename is refused because the name is taken, discarding
    /// what the listener typed while reporting success.
    /// [`Self::rename_smart_playlist`] is therefore not a save's second half but
    /// the sidebar's Rename action: the one write that names a list without
    /// touching its tree.
    ///
    /// The list's boxes and Rules are deleted and re-inserted wholesale from
    /// `tree`, positions and all, renumbered in one dense pass with no gap
    /// management, so a crash partway cannot leave half a rule set and no row of
    /// the previous tree survives.
    ///
    /// Rule and Rule Box ids are generated here, are opaque, and never come
    /// back out — a draft therefore cannot hold a stale identity.
    ///
    /// Refused, writing nothing — neither the name nor the tree — with the reason
    /// carried by [`StoreError::InvalidOperation`]:
    ///
    /// * `tree` holds no Rule, or **any** box inside it — the Root Box included
    ///   — has no children. Nothing riff would have to interpret as "empty" is
    ///   ever stored;
    /// * a Rule in `tree` is not fully formed (the field does not permit the
    ///   comparison, or the value the catalog's widget calls for is missing or
    ///   of the wrong type) — the offender is named by its field;
    /// * `tree` reports an [`UnreadableRule`] ([`SmartPlaylistTree::unreadable`])
    ///   — that row is named by its stored ids and **the save is refused**,
    ///   because re-inserting it would violate the schema's `CHECK` list and
    ///   fail the transaction with no explanation. It is never repaired and
    ///   never silently dropped: removing that row from the draft is what makes
    ///   the same save succeed;
    /// * `name` is held by another sidebar row, in either table, in any case —
    ///   the same rule [`Self::rename_smart_playlist`] enforces, with this
    ///   list's own row exempt when it has one, which is what makes saving a list
    ///   under its own unchanged name legal rather than a conflict with itself.
    ///
    /// A `Some(id)` that no row carries is refused rather than quietly treated as
    /// a create: the draft was written against a list that is gone, and inserting
    /// a second row under it would be exactly the stale identity this shape is
    /// built to make impossible.
    fn save_smart_playlist(
        &mut self,
        id: Option<&PlaylistId>,
        name: &str,
        tree: &SmartPlaylistTree,
    ) -> Result<PlaylistId, StoreError>;

    /// Rename the Custom Smart Playlist with `id` (trimmed) as ONE immediate
    /// durable transaction, replacing only its name. Returns whether the row was
    /// found. The tree is untouched. The sidebar-wide name rule
    /// ([`PlaylistStore::create_playlist`] and
    /// [`Self::save_smart_playlist`] apply the same one) applies, with this
    /// row's own id exempt so a rename that changes nothing never refuses the row
    /// it started from.
    ///
    /// The six Built-in Smart Playlist display names are NOT refused here: they
    /// exist only as code, so shadowing one is the application layer's check.
    fn rename_smart_playlist(
        &mut self,
        id: &PlaylistId,
        new_name: &str,
    ) -> Result<bool, StoreError>;

    /// Delete the Custom Smart Playlist with `id` as ONE immediate durable
    /// transaction, taking its Root Box, every nested Rule Box and every Rule
    /// with it. Returns whether anything was removed.
    ///
    /// Deletion is cascade-only: no other list's tree is rewritten, no Playlist
    /// row or entry is touched, and a Clear Library never reaches any of the
    /// three tables.
    fn delete_smart_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError>;

    /// Copy the Custom Smart Playlist with `id` — its name's copy, its whole
    /// Rule tree, freshly generated ids for every box and every Rule, and no row
    /// shared with the original — as ONE immediate durable transaction. Returns
    /// the copy's id, or `None` when no list carries `id`.
    ///
    /// Duplicate is a store operation rather than "read the tree, then save it
    /// again from the frontend", because two transactions cannot promise the
    /// copy's name is still free: it is chosen against the names that exist at
    /// that instant, `Copy of X` first and `Copy of X 2`, `Copy of X 3` … after
    /// it, tested against **both** sidebar tables by the same rule
    /// [`Self::rename_smart_playlist`] enforces.
    ///
    /// The copy is a row-for-row copy of what is stored, so a Rule whose **value**
    /// this build cannot re-type duplicates faithfully — the copy keeps that row
    /// and keeps refusing a save until it is deleted. Nothing is repaired. A Rule
    /// whose `field_id` or `operator_id` this build does not know cannot be
    /// re-inserted by any write (the schema's `CHECK` list holds exactly the ids
    /// this build enumerates), so that Duplicate is refused naming the Rule rather
    /// than dying on the constraint.
    fn duplicate_smart_playlist(
        &mut self,
        id: &PlaylistId,
    ) -> Result<Option<PlaylistId>, StoreError>;
}

/// The sidebar-count totals for the Library collection, answered by ONE
/// store query (design-handoff issue 05): total tracks, distinct artists,
/// distinct `(album artist, title)` albums, and distinct non-empty per-track
/// genre entries. A fresh store answers all zeros.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibraryCounts {
    /// Total number of stored Tracks.
    pub tracks: usize,
    /// Distinct album artists (the sidebar's Artists rows).
    pub artists: usize,
    /// Distinct `(album artist, title)` albums (the sidebar's Albums rows).
    pub albums: usize,
    /// Distinct non-empty per-track genre entries (the sidebar's Genres
    /// rows); a semicolon-separated tag counts once per entry.
    pub genres: usize,
}

/// The explicit A–Z / Z–A direction of a store listing read. Paged browse
/// reads take this so the direction lands in the query's `ORDER BY`: page
/// offsets stay aligned when the sort reverses, and `Descending` is exact
/// descending SQL order rather than an in-memory reversal of an ascending
/// copy (paginate-browse-columns spec, user story 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// A–Z: the list's canonical ascending order.
    Ascending,
    /// Z–A: the exact reversed order, applied in SQL.
    Descending,
}

/// The explicit order of a flat track-listing read
/// ([`LibraryQueryStore::tracks_page`] / [`LibraryQueryStore::search_page`]):
/// the canonical path order and its reversal, or the title A–Z / Z–A order
/// the listing's sort control offers. The order lands in the query's
/// `ORDER BY` — page offsets stay aligned when it changes, and no in-memory
/// copy of a window is ever reversed (the same contract [`SortDirection`]
/// gives the browse reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TrackListOrder {
    /// Path-ascending: the flat list's canonical order (ADR 0003).
    #[default]
    PathAsc,
    /// Path-descending: the exact reversal, applied in SQL.
    PathDesc,
    /// Title A–Z, case-insensitive (`COLLATE NOCASE`), path as the tiebreak.
    TitleAsc,
    /// Title Z–A, case-insensitive, path as the tiebreak.
    TitleDesc,
}

impl TrackListOrder {
    /// The `ORDER BY` clause this order compiles to. One house for the four
    /// spellings, so the listing window and any future count read cannot
    /// disagree.
    #[must_use]
    pub fn order_by(self) -> &'static str {
        match self {
            Self::PathAsc => "path ASC",
            Self::PathDesc => "path DESC",
            Self::TitleAsc => "title COLLATE NOCASE ASC, path ASC",
            Self::TitleDesc => "title COLLATE NOCASE DESC, path DESC",
        }
    }
}

/// A Section's or Drill Column's total and its visible window, read as one
/// fact.
///
/// The guarantee is the **acquisition**: the store takes its connection once
/// for the whole read, so no committed write can interleave the two halves and
/// leave a total disagreeing with the rows beside it. Carrying the generation
/// that was current at the read turned out to be inert data across the seam —
/// the cache observes its own epoch before loading and commits at that — so
/// which generation a level was filled at is the Session Projection's concern,
/// not the page's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    total: usize,
    rows: Vec<T>,
}

impl<T> Page<T> {
    /// A page whose total and rows were taken under one connection
    /// acquisition.
    #[must_use]
    pub fn new(total: usize, rows: Vec<T>) -> Self {
        Self { total, rows }
    }

    /// The listing's total, taken in the same acquisition as [`Self::rows`].
    #[must_use]
    pub fn total(&self) -> usize {
        self.total
    }

    /// The visible window, taken in the same acquisition as [`Self::total`].
    #[must_use]
    pub fn rows(&self) -> &[T] {
        &self.rows
    }

    /// Take the window out of the page, consuming it. A Session Projection
    /// caches the rows it will keep showing; the total stays with the page
    /// until then.
    #[must_use]
    pub fn into_rows(self) -> Vec<T> {
        self.rows
    }
}

/// A Custom Smart Playlist Rule tree in the Application Store's own spelling:
/// every leaf carries the **stored** field and operator ids and every box
/// carries the stored combine word, so a row this build cannot read stays the
/// data it is.
///
/// That is the whole reason this type exists rather than the port taking
/// [`crate::rule::RootBox`] directly. [`crate::rule`]'s enums are closed, so an
/// Unreadable Rule has no representation in them, and a tree that had to be
/// coerced into [`crate::rule::MatchableField`] to reach a read would have
/// already thrown the row away — or guessed at it. Keying on the id string is
/// also how the store names a field everywhere else: the `smart_rules` row's
/// own `field_id`, and the value pickers' field-keyed reads.
///
/// The value stays typed ([`RuleValue`]) because reading a stored `value`
/// column back into one is the save path's question, not the compiler's: by
/// the time a tree reaches a read, its values are already what the catalog says
/// its field holds. A leaf whose value the compiler cannot use is refused with
/// a reason, never silently ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleTree {
    /// One Rule row: the stored `field_id`, `operator_id`, `negated` flag, and
    /// the value it tests — `None` exactly where the catalog draws no value
    /// input (`is missing`).
    Rule {
        /// The stored field id, `smart_rules.field_id`.
        field_id: String,
        /// The stored operator id, `smart_rules.operator_id`.
        operator_id: String,
        /// Whether the test's answer is inverted.
        negated: bool,
        /// The value tested against, already typed by the field.
        value: Option<RuleValue>,
    },
    /// One Rule Box row: the stored `combine` word and its children in their
    /// single shared sibling order.
    Box {
        /// The stored combine word, `smart_rule_boxes.combine`.
        combine: String,
        /// Rules and Boxes together, in the one sibling numbering.
        children: Vec<RuleTree>,
    },
}

impl RuleTree {
    /// The store's spelling of a tree the frontend can hold: each closed enum
    /// becomes its own stored id and every Rule keeps its typed value, so a
    /// readable draft reaches a read unchanged.
    ///
    /// This is the only place the domain vocabulary is written into stored ids
    /// for a read — a draft, a projection and a listing all go through here, so
    /// no caller can restate the id spelling.
    #[must_use]
    pub fn from_root_box(root: &RootBox) -> Self {
        Self::from_rule_box(root.inner())
    }

    /// One Rule Box and everything under it, spelled as stored ids.
    #[must_use]
    pub fn from_rule_box(boxed: &RuleBox) -> Self {
        Self::Box {
            combine: boxed.combine().stored_id().to_string(),
            children: boxed
                .children()
                .iter()
                .map(|child| match child {
                    RuleChild::Rule(rule) => Self::Rule {
                        field_id: rule.field().stored_id().to_string(),
                        operator_id: rule.comparison().stored_id().to_string(),
                        negated: rule.is_negated(),
                        value: rule.value().cloned(),
                    },
                    RuleChild::Box(nested) => Self::from_rule_box(nested),
                })
                .collect(),
        }
    }

    /// The ONE bridge between the two spellings of a Rule tree.
    ///
    /// [`SmartPlaylistTree`] is the read type: closed enums the editor can
    /// hold, plus the rows this build cannot interpret, kept beside the tree
    /// rather than forced into it. `RuleTree` is a bounded read's input: the
    /// same tree in stored-id spelling, so the query compiler can reject an
    /// unknown field id before any SQL is produced. This is the only place
    /// one becomes the other, and the rule it enforces is the reason it
    /// exists: a tree reporting even one [`UnreadableRule`] converts to
    /// **nothing** — `None`, meaning no query at all, a list that matches
    /// nothing. Guessing at the unreadable row or dropping it and querying
    /// the rest would both answer a question the listener never saved.
    /// Whoever reports the refusal builds its reason from
    /// [`SmartPlaylistTree::unreadable`]; the ids the rows carry are the
    /// store's own and never a frontend-facing identity.
    #[must_use]
    pub fn from_smart_playlist_tree(tree: &SmartPlaylistTree) -> Option<Self> {
        if tree.unreadable().is_empty() {
            Some(Self::from_root_box(tree.root()))
        } else {
            None
        }
    }
}

/// Port for reading the Library collection section of the Application Store.
///
/// Flat-list and search reads are bounded windows (ADR 0003): callers fetch
/// only the visible row range plus a total count, ordered deterministically
/// by the `TrackListOrder` the read names (path-ascending by default).
/// Search parity with the former in-memory
/// implementation is guaranteed by storing a derived Rust-lowercased
/// search-text column at write time and lowercasing the query in Rust here;
/// matching is literal substring (no wildcard semantics).
///
/// ## Listing Pages
///
/// A browse surface that shows a total above a window of rows reads both as
/// one [`Page`] (`*_page` methods): the store takes its connection once for
/// the whole read, so a committed mutation can never leave the total and the
/// rows describing different moments. Which generation a listing was cached
/// at is the Session Projection's question, not the page's.
///
/// Two obligations are part of this interface. A page read must compose
/// private connection-level helpers and never call back through a public
/// method on this trait — the connection lock is not reentrant, so a nested
/// call deadlocks. And a page read takes an offset, never a window length it
/// invents: how many rows a surface displays belongs to that surface's Scroll
/// Memory, so the store must not guess at pagination.
///
/// [`Self::folder_track_count`] is deliberately not a page read. It counts an
/// unbounded list that the listing already materialises, so it has no second
/// moment to disagree with.
pub trait LibraryQueryStore {
    /// Resolve one `Track` by its `TrackId` (its full file path). Playback uses
    /// this instead of any in-memory copy. `None` when unknown.
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError>;

    /// The [`METADATA_VERSION`](crate::track::METADATA_VERSION) this store's
    /// metadata was last written under. A value below the running binary's
    /// means the scan's freshness filter must treat every known path as stale
    /// for one scan; the filter reads this ONCE per scan, never per path.
    fn metadata_version(&self) -> Result<u32, StoreError>;

    /// The flat library list's Listing Page at `offset`: the total number of
    /// stored Tracks and one window of `limit` rows, in `order`, read as one
    /// fact under a single connection acquisition. The default order
    /// ([`TrackListOrder::PathAsc`]) is the canonical flat ordering.
    fn tracks_page(
        &self,
        order: TrackListOrder,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, StoreError>;

    /// The Library-count totals — tracks, artists, albums, genres — in ONE
    /// query (design-handoff issue 05), so the sidebar-counts read model
    /// costs one store round trip per generation instead of four queries.
    /// Genre semantics match [`Self::genre_counts`]: distinct non-empty
    /// per-track genre entries.
    fn library_counts(&self) -> Result<LibraryCounts, StoreError>;

    /// Every smart playlist's unbounded total in [`SmartPlaylistKind::ALL`]
    /// order — how many tracks each list would contain with no limit
    /// (design-handoff issue 05). Membership semantics are exactly the
    /// [`Self::smart_playlist`] queries' filters; the `Lost Gems` count is
    /// relative to the current clock like the list itself.
    fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError>;

    /// Every Track id in the collection, ordered by full path ascending —
    /// the canonical flat ordering (ADR 0003). Serves Queue Fill: the whole
    /// Library loads into the Playback Queue in this order when playback
    /// starts from an empty queue.
    fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError>;

    /// The search listing's Listing Page for `query` at `offset`: the match
    /// total and one window of `limit` rows, in `order`, read as one fact
    /// under a single connection acquisition.
    fn search_page(
        &self,
        query: &str,
        order: TrackListOrder,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, StoreError>;

    // --- Custom Smart Playlist reads (rule trees as SQL) ------------------

    /// A Custom Smart Playlist's Listing Page: the total number of Tracks its
    /// Rule tree matches and one window of `limit` of them, in `order`, read as
    /// one fact under a single connection acquisition.
    ///
    /// The tree is compiled to one predicate over the `tracks` table — never a
    /// join to the `albums` or `artists` entity tables, because a Rule tests
    /// the Track's own tag, not the album's. `needle` is the listing's current
    /// search text and is combined into this read with `AND`, the way the folder
    /// and genre-filtered reads combine theirs: a needle applied after paging
    /// would silently drop rows from a window that was cut on the unfiltered
    /// answer. `None` means no needle.
    ///
    /// A tree this store cannot turn into a predicate is an error carrying a
    /// reason, never an empty page: a tree holding a field id this build does
    /// not read, an operator its field does not permit, a Box with nothing in
    /// it, or a tree wide enough to pass the engine's own ceiling on bound
    /// values. Each is refused before any SQL is produced, and a listing that
    /// cannot be computed says so instead of showing nothing — "no matches" and
    /// "cannot tell" are different answers, and only one of them is a page.
    fn custom_smart_playlist_page(
        &self,
        tree: &RuleTree,
        order: TrackListOrder,
        needle: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Track>, StoreError>;

    /// Every artist in the collection, name-ascending (byte-wise, matching
    /// the former UI sort). Each artist's `albums` lists its composite keys
    /// (`"album artist - title"`) in the canonical browsing order — year
    /// descending with missing years last, then title ascending — so callers
    /// never re-sort.
    fn all_artists(&self) -> Result<Vec<Artist>, StoreError>;

    /// One artist's albums in canonical browsing order: year descending with
    /// missing years last (treated as 0), then title ascending byte-wise.
    /// Each album carries its track ids in album-track order (track number
    /// ascending with missing numbers first, path tiebreak). Unknown artists
    /// yield an empty `Vec`.
    fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError>;

    /// One album's tracks in full, in canonical album-track order: track
    /// number ascending with missing numbers first (the legacy
    /// `unwrap_or(0)` slot), then path tiebreak. Unknown albums yield an
    /// empty `Vec`.
    fn album_tracks(&self, album_artist: &str, album_title: &str)
    -> Result<Vec<Track>, StoreError>;

    /// Whether any stored Track lives under `folder` (component-wise path
    /// prefix, exactly like the former in-memory `Path::starts_with` checks:
    /// case-sensitive, and a sibling sharing a byte-prefix — `a` vs `ab` —
    /// never matches). Backed by escaped SQL prefix matching over stored
    /// track paths; there is no folder table. Separators are matched
    /// literally, so callers pass consistently separated paths — what scan
    /// joins and `Path::join` always produce.
    fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, StoreError>;

    /// Whether any stored Track under `folder` matches the search query
    /// (same substring semantics as [`Self::search_page`]: the query is
    /// lowercased in Rust and matched literally against the derived
    /// Rust-lowercased search text).
    fn folder_has_search_match(
        &self,
        folder: &std::path::Path,
        query: &str,
    ) -> Result<bool, StoreError>;

    /// Every Track id under `folder`, ordered by full path exactly like the
    /// former in-memory tree listing (component-wise `Path` order).
    fn track_ids_in_folder_tree(
        &self,
        folder: &std::path::Path,
    ) -> Result<Vec<TrackId>, StoreError>;

    /// The Tracks directly inside `folder` (their parent is exactly
    /// `folder`), ordered by track number ascending with missing numbers
    /// first, then filename — the former in-memory direct-listing order.
    fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, StoreError>;

    /// How many tracks live under `folder` (component-wise path prefix, the
    /// same membership as [`Self::track_ids_in_folder_tree`]) — the
    /// per-music-folder count the Settings Library pane shows
    /// (design-handoff issue 05).
    fn folder_track_count(&self, folder: &std::path::Path) -> Result<usize, StoreError>;

    /// The last completed full library scan's summary — when it finished and
    /// what it saw — as recorded by
    /// [`LibraryMutationStore::record_full_scan_completed`]. `None` when no
    /// scan has ever completed in this store (design-handoff issue 12).
    fn last_full_scan(&self) -> Result<Option<FullScanSummary>, StoreError>;

    /// The direct child directories of `folder` that contain at least one
    /// stored Track anywhere beneath them, name-ascending (`PathBuf` order,
    /// like the former in-memory tree walk). Derived purely from stored
    /// track paths: a child counts as a directory when some stored track
    /// lives deeper inside it.
    fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, StoreError>;

    /// Compute one read-only smart playlist as a store query, reproducing
    /// the former in-memory semantics precisely: the same filters, ordering
    /// tie-breaks (including display-title fallbacks), limits, and the
    /// ninety-day Lost Gems threshold. Tracks arrive ready to render — the
    /// former id list plus per-id resolution composed into one result.
    fn smart_playlist(
        &self,
        kind: SmartPlaylistKind,
        limit: usize,
    ) -> Result<Vec<Track>, StoreError>;

    /// Every genre entry carrying at least one stored Track, name-ascending
    /// (byte-wise), each with its per-track count aggregated from the
    /// tracks' own genre metadata — not the albums' derived genre column.
    /// A stored genre string is split on `;` (each segment trimmed; empty
    /// segments dropped), so `"Rock; Jazz"` yields one row for `Rock` and
    /// one for `Jazz`, each counting the track once. Tracks whose genre is
    /// missing or empty aggregate into nothing: there is no row for them.
    /// The sidebar's total-genres count is this list's length.
    ///
    /// **Unbounded, and recorded as a follow-up rather than fixed here.** Its
    /// two consumers ask a membership question over the *whole* Library —
    /// "is this genre one the listener has?" before drilling in, and "what is
    /// this genre's track count?" in the inspector — so a bound here would not
    /// answer a smaller list, it would report a real genre as absent. It is
    /// also not a per-frame read: both call sites reach it through the genre
    /// Session Projection, which fetches once per store generation and hands
    /// out an `Arc` clone after that, and its length is the listener's
    /// distinct genre-entry count rather than their track count. The
    /// follow-up, if the Genres listing ever needs a capped list read of its
    /// own, is to page it through [`Self::genres_page`] — which already
    /// answers a window plus the total — and to give the two membership
    /// questions a point read, not to bound this one. Deliberately **not**
    /// reused by the picker reads below: they answer a different question, on
    /// a bounded path, per picker open.
    fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError>;

    /// Every artist having at least one Track with genre `genre`,
    /// name-ascending, each with only the album keys of albums holding at
    /// least one matching track, in canonical browsing order. A track
    /// matches when `genre` equals one of its semicolon-separated (trimmed)
    /// genre entries — exact, so `"Indie Rock"` never matches `"Rock"`.
    /// Unknown genres yield an empty `Vec`.
    fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError>;

    /// One artist's albums holding at least one Track with genre `genre`,
    /// in canonical browsing order, each carrying only its matching track
    /// ids in album-track order. Matching follows [`Self::artists_in_genre`]
    /// (semicolon-separated entries). Unknown artists or genres yield an
    /// empty `Vec`.
    fn artist_albums_in_genre(&self, artist: &str, genre: &str) -> Result<Vec<Album>, StoreError>;

    /// One album's Tracks with genre `genre`, in canonical album-track
    /// order: track number ascending with missing numbers first, then path
    /// tiebreak. Matching follows [`Self::artists_in_genre`]
    /// (semicolon-separated entries). Unknown albums or genres yield an
    /// empty `Vec`.
    fn album_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
    ) -> Result<Vec<Track>, StoreError>;

    // --- Entity hit reads (search across Library sections) -----------------

    /// The hit-album Drill Column's Listing Page for `query` at `offset`:
    /// the hit total and one window of `limit` albums in the canonical
    /// browsing order, read as one fact under a single connection acquisition.
    fn hit_albums_page(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, StoreError>;

    /// The hit-artist Drill Column's Listing Page for `query` at `offset`:
    /// the hit total and one window of `limit` artists, name-ascending, read
    /// as one fact under a single connection acquisition.
    fn hit_artists_page(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Artist>, StoreError>;

    /// One album's tracks that match `query`, in canonical album-track
    /// order (track number ascending with missing numbers first, path
    /// tiebreak). Unknown albums yield an empty `Vec`.
    fn album_hit_tracks(
        &self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Result<Vec<Track>, StoreError>;

    /// Whether `album` is itself a name hit for `query` — its album artist
    /// or title matches case-insensitively — as opposed to a track hit.
    /// Unknown albums yield `false`.
    fn album_is_name_hit(
        &self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Result<bool, StoreError>;

    /// Whether `artist` is itself a name hit for `query` — its own name
    /// matches case-insensitively — as opposed to an album or track hit.
    /// Unknown artists yield `false`.
    fn artist_is_name_hit(&self, artist: &str, query: &str) -> Result<bool, StoreError>;

    /// One bounded window of hit albums within `genre` for `query`, in the
    /// canonical browsing order. An album appears when it holds at least one
    /// track with `genre` (semicolon-separated entries, exactly like
    /// [`Self::artists_in_genre`]) and is itself a hit — its album artist or
    /// title matches, or a genre-bearing member track matches. Each returned
    /// album carries only the genre-bearing hit track ids in canonical
    /// album-track order.
    fn hit_albums_in_genre(
        &self,
        genre: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Album>, StoreError>;

    /// One bounded window of hit artists within `genre` for `query`,
    /// name-ascending. An artist appears when at least one of its albums is
    /// a genre-scoped hit (holds a `genre`-bearing track and is itself a
    /// hit); each artist carries only those genre-scoped hit-album keys in
    /// canonical browsing order.
    fn hit_artists_in_genre(
        &self,
        genre: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Artist>, StoreError>;

    /// One album's tracks that match `query` among the tracks carrying
    /// `genre` (semicolon-separated entries), in canonical album-track
    /// order.
    fn album_hit_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        query: &str,
    ) -> Result<Vec<Track>, StoreError>;

    /// Every genre containing at least one hit track, name-ascending, with
    /// its hit-track count. Semicolon-separated genre segments split exactly
    /// like [`Self::genre_counts`] — a hit track tagged `"Rock; Jazz"`
    /// counts once per entry.
    fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError>;

    // --- Paged browse reads (paginate-browse-columns) ----------------------
    //
    // The browse columns (Artists, Albums, Genres) and the genre drill-downs
    // serve bounded windows with authoritative totals exactly like the flat
    // and hit listings (ADR 0003). Each windowed read takes the list's
    // explicit [`SortDirection`] so the A–Z / Z–A toggle lands in the SQL
    // `ORDER BY` — page offsets stay aligned when the sort reverses, and
    // descending is exact descending SQL order, not an in-memory reversal.

    /// The Artists root's Listing Page at `offset` for `direction`: the
    /// artist total and one window of `limit` artists, each carrying its
    /// album keys, read as one fact under a single connection acquisition.
    fn artists_page(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Artist>, StoreError>;

    /// The Albums root's Listing Page at `offset` for `direction`: the album
    /// total and one window of `limit` albums in the flat browsing order or
    /// its exact reversal, read as one fact under a single connection
    /// acquisition.
    fn albums_page(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, StoreError>;

    /// The Genres root's Listing Page at `offset` for `direction`: the genre
    /// total and one window of `limit` entries, read as one fact under a
    /// single connection acquisition.
    fn genres_page(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Page<GenreCount>, StoreError>;

    /// The genre drill-down's Listing Page of artists within `genre` at
    /// `offset`: the total and one window of `limit` artists, read as one
    /// fact under a single connection acquisition.
    fn artists_in_genre_page(
        &self,
        genre: &str,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Artist>, StoreError>;

    /// The artist-and-genre drill-down's Listing Page at `offset`: the album
    /// total and one window of `limit` albums, read as one fact under a
    /// single connection acquisition.
    fn artist_albums_in_genre_page(
        &self,
        artist: &str,
        genre: &str,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Page<Album>, StoreError>;

    // --- Custom Smart Playlist value pickers ------------------------------
    //
    // A Rule's value picker offers what the Library actually holds. Both
    // reads are keyed on the Rule's **field id string** — `"genre"`,
    // `"artist"`, `"album_artist"`, `"album"`, `"sample_rate"`,
    // `"channels"` — and the adapter resolves that id through its own
    // field-id → `tracks`-column table, gated by this crate's field catalog,
    // so a column name is never taken from input and an id that is not a
    // pickable field is rejected before any query is built. Each is a
    // `DISTINCT` read over `tracks`, over the very column a Rule tests, and
    // not over the `albums` / `artists` entity tables the browse listings
    // read: a pickable value has to be one the tested column can contain.
    // The consequence is deliberate — the Artists listing shows `Unknown
    // Artist` for a track whose artist tag is absent, but that fallback
    // lives in the entity key, so the picker can never offer it and "is
    // missing" is how those tracks are asked for.

    /// At most `limit` distinct values of the pickable **text** field
    /// `field_id`, ordered byte-wise ascending, read as one bounded query.
    ///
    /// Absent values are excluded rather than surfaced as an empty entry: a
    /// `NULL` or all-whitespace tag offers nothing a Rule could match. Genre
    /// is the one text field stored as a `;`-separated set, so its distinct
    /// values are the tag's trimmed *entries* — `"Ambient; Techno"` yields
    /// `Ambient` and `Techno` and never the combined string, matching the
    /// whole-segment semantics [`Self::genre_counts`] aggregates with.
    ///
    /// The bound is the store's, not the caller's: the answer never exceeds
    /// `limit` values however large the Library is, so a 20,000-track Library
    /// cannot fill a picker pane, and `limit` of `0` answers empty. An
    /// `field_id` that is not a pickable text field — an id no field is, a
    /// field the catalog gives a typed control for, or one whose value is a
    /// number — is an [`StoreError`], never an empty list that hides the
    /// mistake.
    fn picker_texts(&self, field_id: &str, limit: usize) -> Result<Vec<String>, StoreError>;

    /// At most `limit` distinct values of the pickable **numeric** field
    /// `field_id`, ascending, read as one bounded query.
    ///
    /// Absent here means `NULL` alone: a stored `0` is a present value and is
    /// offered like any other. The same bound and unknown-field rules as
    /// [`Self::picker_texts`] apply, with the same `DISTINCT`-over-`tracks`
    /// shape, and the same refusal of an id that is not a pickable numeric
    /// field.
    fn picker_numbers(&self, field_id: &str, limit: usize) -> Result<Vec<i64>, StoreError>;

    // --- Custom Smart Playlist tree read ----------------------------------

    /// The whole Rule tree of the Custom Smart Playlist with `id` as the store
    /// holds it, or `None` when no list carries that id.
    ///
    /// One read of the three rule tables in one connection acquisition: the Root
    /// Box, every nested Rule Box and every Rule, each in its stored sibling
    /// order, so what comes back preserves nesting and order across Rules and
    /// Boxes drawn as one interleaved list. It is the draft a save is written
    /// from, and the assertion that a saved tree reads back identical is made
    /// against this.
    ///
    /// A Rule whose stored `field_id` or `operator_id` this build cannot read —
    /// and a value it cannot re-type for its field — is not guessed at, dropped
    /// or repaired: it is returned beside the tree as an [`UnreadableRule`]
    /// carrying the row's own raw ids and where it sits. A schema `CHECK`
    /// constrains writes, not reads, so such a row is genuinely readable, and
    /// the closed field and operator enums mean it has no [`crate::rule::Rule`]
    /// representation to force it into.
    ///
    /// A list whose row exists but whose tree is not a whole tree — no Root Box,
    /// or boxes belonging to another list — is a store inconsistency this read
    /// reports as an error rather than inventing a box for.
    fn smart_playlist_tree(&self, id: &PlaylistId)
    -> Result<Option<SmartPlaylistTree>, StoreError>;

    /// Every Custom Smart Playlist the listener has saved — id, name and
    /// `created_at` — in **creation order**: the order the listener made them.
    ///
    /// This is the *which lists exist* question, and nothing else: no Tracks,
    /// no Rule tree, no counts. A re-save or a rename keeps the row's
    /// `created_at` and therefore its position, so the order a listener built
    /// is the order every consumer — the sidebar's custom rows and the
    /// Settings page's list column — shows. The six Built-in Smart Playlists
    /// are code, not rows, and never appear here.
    fn custom_smart_playlists(&self) -> Result<Vec<CustomSmartPlaylist>, StoreError>;

    /// The session Library generation this handle bumps after each committed
    /// Library mutation (ADR 0002). A handle shares the implementor's own
    /// counter: clones answer the same counter, so a mutation committed
    /// through any clone of the handle is seen by every holder of the
    /// answer.
    ///
    /// This is the read the UI's single read seam wires with: it derives the
    /// counter its Session Projections observe from the ports it keeps, so
    /// the same-store-same-instant rule is enforced by the constructor's
    /// shape rather than re-derived by every wiring site. Widening the port
    /// by this one read is recorded as an ADR 0012 consideration, not a
    /// conflict: it formalizes a read the Composition Root and every test
    /// suite already make through the concrete store type, rather than
    /// growing the contract's vocabulary. It takes no fallible shape — a
    /// wiring read has no failure mode to degrade to.
    #[must_use]
    fn library_generation(&self) -> StoreGeneration;
}

/// Notification the `Application Store` emits (best-effort) over a
/// crossbeam channel sender whenever a committed mutation bumps its Library
/// session generation (ADR 0002). `BackendEvents` drains this channel on the
/// frontend frame path, coalesces rapid Library bumps, and surfaces them as
/// `BackendEvents`. The playlist generation has no variant here: its readers
/// are the session projections, which observe the counter directly — the
/// bump itself, not a notice about it, is the mutation's observable.
///
/// Invariant (emit-beside-bump): every committed Library mutation in
/// the `SqliteStore` adapter sends exactly one of these over the
/// sender, immediately after bumping the Library generation. A
/// mutation that fails to commit sends nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum StoreChanged {
    /// The session Library generation moved to `gen` on a committed
    /// Library mutation (scan batch, play recording, tag edit, remove,
    /// clear). It is also the generation **which tracks match** a Custom
    /// Smart Playlist moves with, because membership is a query over these
    /// Tracks — the Rule tree's own writes are below, and move this counter
    /// never. `BackendEvents` coalesces these to roughly four per second.
    Library(u64),
}
