//! The `Application Store`'s Custom Smart Playlist rule-tree writes and read.
//!
//! Four durable writes and one read, all of them over the three tables
//! migration 020 created: `smart_playlists`, `smart_rule_boxes`, `smart_rules`.
//! They live here rather than in `sqlite.rs` because they are one capability's
//! whole storage question — what a Rule tree may look like, how one is spelled
//! in a `TEXT` column, and which of those spellings this build can read at all —
//! and because the numbering, the re-insertion and the refusals belong in one
//! file a reader can hold at once.
//!
//! The rules this module exists to enforce, each stated where it is applied:
//!
//! * **Creation is not a separate path.** The first save inserts the playlist
//!   row. Every later save updates that row **in place** — its id and its
//!   `created_at` survive, and its `name` is written from the same draft, because
//!   the editor's name and its Rule tree are one listener action in one
//!   transaction (the rationale is stated at `save_in_tx`) — and replaces the
//!   tree **wholesale**: boxes and Rules are deleted and re-inserted from the
//!   draft, positions and all, inside that same transaction, so no row of the
//!   previous tree can survive and no crash can leave half a rule set.
//! * **Rule and Rule Box ids are generated here**, are opaque, and never reach
//!   the frontend. A draft therefore carries no identity that can go stale.
//! * **Nothing empty is ever stored**: a tree with no Rule, and any box with no
//!   children — the Root Box included — is refused.
//! * **One meaning, one spelling.** `Comparison::IsMissing` with
//!   [`Rule::is_negated`] set is how "is not missing" is stored; there is no
//!   `is_not_missing` id that can even be written, because the closed enum has
//!   none.
//! * **A value is `TEXT` whose meaning comes from the catalog**, and `NULL` only
//!   where the field takes no value at all.
//! * **An Unreadable Rule is preserved, never repaired and never dropped** — and
//!   it blocks a save, naming the row, because re-inserting it would violate the
//!   schema's `CHECK` list and fail the transaction with no explanation.

use riff_persistence::errors::StoreError;
use riff_persistence::field_catalog::{self, ValueType};
use riff_persistence::playlist::PlaylistId;
use riff_persistence::rule::{
    CivilDateTime, Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild,
    RuleValue,
};
use riff_persistence::store::{SmartPlaylistTree, UnreadableRule};
use rusqlite::{Connection, OptionalExtension, params};

use super::sqlite::{SessionGeneration, SqliteStore, now_nanos, store_refusal};

/// The prefix that keeps a Custom Smart Playlist row id out of the space a
/// Playlist row id can occupy.
///
/// Not cosmetic. [`SqliteStore::reject_taken_sidebar_name`] exempts the row being
/// rewritten **by id**, across both sidebar tables, so an id shared between a
/// `playlists` row and a `smart_playlists` row would exempt the wrong row and let
/// a real clash through. Playlist ids are `{slug}-{millis}`; prefixing makes the
/// two namespaces disjoint by construction.
const SMART_ID_PREFIX: &str = "spl-";

/// How many candidate copy names [`free_copy_name`] will try before giving up.
/// A listener with 64 lists named `Copy of X 2` through `Copy of X 64` is not a
/// real case; the bound exists so a genuine query failure cannot spin.
const COPY_NAME_ATTEMPTS: usize = 64;

/// The `smart_playlists` insert, shared by the save that creates and the
/// Duplicate that copies.
const INSERT_SMART_PLAYLIST: &str =
    "INSERT INTO smart_playlists (id, name, created_at) VALUES (?1, ?2, ?3)";

impl SqliteStore {
    /// Save one Custom Smart Playlist: insert the row when the list is new,
    /// rewrite its name and replace its Rule tree wholesale when it is not.
    ///
    /// One immediate durable transaction, `BEGIN IMMEDIATE` to `COMMIT`, so the
    /// name, the delete and every re-inserted row land together. The sidebar-wide
    /// name rule runs inside that transaction and before any write, so a refused
    /// name writes nothing — including a re-save under a name some other sidebar
    /// row holds, which leaves the tree exactly where it was.
    ///
    /// # Errors
    /// [`StoreError::InvalidOperation`] when the tree is not savable (no Rule, an
    /// empty box, a half-filled Rule, an Unreadable Rule in the draft), when the
    /// name is held by another sidebar row, or when `id` names no row.
    pub(crate) fn save_smart_playlist_write(
        &mut self,
        id: Option<&PlaylistId>,
        name: &str,
        tree: &SmartPlaylistTree,
    ) -> Result<PlaylistId, StoreError> {
        self.mutate(
            SessionGeneration::Playlists,
            Result::is_ok,
            "failed to save Custom Smart Playlist",
            |conn| save_in_tx(conn, id, name, tree),
        )
    }

    /// Rename one Custom Smart Playlist, writing nothing but its name.
    ///
    /// # Errors
    /// [`StoreError::InvalidOperation`] when the name is held by another sidebar
    /// row — this row's own id exempt, so a rename that changes nothing never
    /// refuses the row it started from.
    pub(crate) fn rename_smart_playlist_write(
        &mut self,
        id: &PlaylistId,
        new_name: &str,
    ) -> Result<bool, StoreError> {
        self.mutate(
            SessionGeneration::Playlists,
            |renamed| matches!(renamed, Ok(true)),
            "failed to rename Custom Smart Playlist",
            |conn| {
                SqliteStore::reject_taken_sidebar_name(conn, new_name, Some(&id.0))?;
                conn.execute(
                    "UPDATE smart_playlists SET name = ?2 WHERE id = ?1",
                    params![id.0, new_name.trim()],
                )
                .map(|rows| rows > 0)
            },
        )
    }

    /// Delete one Custom Smart Playlist and, through the schema's cascades, its
    /// Root Box, every nested Rule Box and every Rule.
    ///
    /// # Errors
    /// [`StoreError::InvalidOperation`] on a query failure. An unknown id is not
    /// an error: it answers `false` and writes nothing.
    pub(crate) fn delete_smart_playlist_write(
        &mut self,
        id: &PlaylistId,
    ) -> Result<bool, StoreError> {
        self.mutate(
            SessionGeneration::Playlists,
            |deleted| matches!(deleted, Ok(true)),
            "failed to delete Custom Smart Playlist",
            |conn| {
                conn.execute("DELETE FROM smart_playlists WHERE id = ?1", [&id.0])
                    .map(|rows| rows > 0)
            },
        )
    }

    /// Copy one Custom Smart Playlist's whole tree under a name chosen against
    /// the names that exist at that instant.
    ///
    /// One transaction, because the name and the copy have to be settled
    /// together: read-then-save across two transactions cannot promise the name
    /// is still free by the time the save runs.
    ///
    /// The copy is **row-for-row**, ids aside, so a Rule whose *value* this build
    /// cannot re-type rides along untouched — the copy is as unreadable as the
    /// original instead of losing the row or inventing one. A `field_id` or
    /// `operator_id` this build does not know cannot be re-inserted at all (the
    /// schema's `CHECK` list holds exactly the ids it enumerates), so that copy is
    /// refused naming the Rule, in the save's own wording, before anything is
    /// written.
    ///
    /// # Errors
    /// [`StoreError::InvalidOperation`] when the source's tree is not a whole
    /// tree, when it holds a Rule this build could not write, or when no copy name
    /// could be settled on. `Ok(None)` when no list carries `id`, which commits
    /// nothing and bumps nothing.
    pub(crate) fn duplicate_smart_playlist_write(
        &mut self,
        id: &PlaylistId,
    ) -> Result<Option<PlaylistId>, StoreError> {
        self.mutate(
            SessionGeneration::Playlists,
            |copied| matches!(copied, Ok(Some(_))),
            "failed to duplicate Custom Smart Playlist",
            |conn| duplicate_in_tx(conn, id),
        )
    }

    /// Read one Custom Smart Playlist's whole Rule tree.
    ///
    /// One connection acquisition for the playlist row, its boxes and its rules,
    /// so a committed save can never leave the boxes and the rules describing two
    /// different moments.
    ///
    /// # Errors
    /// [`StoreError::InvalidOperation`] when the row exists but its tree is not a
    /// tree this build can assemble (no Root Box, more than one, or a box that
    /// cannot say how it combines). An unknown id answers `Ok(None)`.
    pub(crate) fn read_smart_playlist_tree(
        &self,
        id: &PlaylistId,
    ) -> Result<Option<SmartPlaylistTree>, StoreError> {
        self.with_connection(|conn| read_tree_on(conn, id))
    }
}

// --- the save ---------------------------------------------------------------

/// Save body: validate, settle the row, replace the tree. Runs inside
/// [`SqliteStore::save_smart_playlist_write`]'s transaction.
fn save_in_tx(
    conn: &Connection,
    id: Option<&PlaylistId>,
    name: &str,
    tree: &SmartPlaylistTree,
) -> rusqlite::Result<PlaylistId> {
    refuse_unsavable(tree)?;

    let playlist_id = if let Some(existing) = id {
        if !smart_playlist_exists(conn, existing)? {
            return Err(store_refusal(
                "this list cannot be saved: the Custom Smart Playlist its draft was written \
                 against no longer exists"
                    .to_string(),
            ));
        }
        // The save names the list too — deliberately, and not an oversight a later
        // reader should "correct" back into leaving the name alone.
        //
        // **Why:** the editor's name field and its Rule tree arrive as ONE listener
        // action taken in ONE form, so both land in this transaction or neither does
        // (spec user story 46: "my whole rule set written in one step, so a crash
        // partway cannot leave me with half a rule set"). Splitting the write across
        // `save_smart_playlist` and `rename_smart_playlist` cannot give that guarantee;
        // it half-applies in two distinct bad ways — the name lands and the tree save
        // fails, or the tree saves and the rename is refused because the name is taken,
        // silently discarding what the listener typed while reporting success.
        //
        // What survives every save is therefore the row's **identity and its age**: the
        // id and `created_at` are never written, the row is never deleted and
        // re-inserted, and the sidebar's Rename action (`rename_smart_playlist`) is
        // still the only write that names a list without touching its tree.
        //
        // Self-exemption is what keeps a re-save under an untouched name legal: ticket
        // 03's rule runs with this row's id exempt, exactly as the rename path does, so
        // the name check never refuses a list against itself.
        SqliteStore::reject_taken_sidebar_name(conn, name, Some(&existing.0))?;
        conn.execute(
            "UPDATE smart_playlists SET name = ?2 WHERE id = ?1",
            params![existing.0, name.trim()],
        )?;
        existing.0.clone()
    } else {
        // Creation is not a separate path: this insert is the only one, which is
        // why the sidebar's `+` writes nothing and the list appears on Save.
        SqliteStore::reject_taken_sidebar_name(conn, name, None)?;
        let trimmed = name.trim();
        let id = new_smart_playlist_id(conn, trimmed)?;
        conn.execute(INSERT_SMART_PLAYLIST, params![id, trimmed, now_nanos()])?;
        id
    };

    // Wholesale replacement, in one delete. The schema's cascades take every
    // nested box and every Rule with the boxes named here — the same cascade-only
    // deletion a delete of the list itself relies on — so no row of the previous
    // tree can survive the re-insertion below.
    conn.execute(
        "DELETE FROM smart_rule_boxes WHERE playlist_id = ?1",
        [&playlist_id],
    )?;
    insert_tree(conn, &playlist_id, tree.root())?;
    Ok(PlaylistId(playlist_id))
}

/// Whether the draft is a tree riff is willing to store, and if not, which part
/// of it says so.
///
/// Four refusals, in the order a listener would want to hear them, each naming
/// its offender: an Unreadable Rule first (it is the one no amount of filling in
/// fixes), then a tree with no Rule, then a box with no children, then a
/// half-filled Rule. The last one is the catalog's answer, asked through
/// [`Rule::is_fully_formed`] — this module states no operator set, no absence
/// rule and no value type of its own.
fn refuse_unsavable(tree: &SmartPlaylistTree) -> rusqlite::Result<()> {
    if let Some(unreadable) = tree.unreadable().first() {
        return Err(store_refusal(format!(
            "this list cannot be saved: it holds a rule testing {:?} with {:?}, which this \
             version of riff cannot understand — delete that rule before saving",
            unreadable.field_id(),
            unreadable.operator_id()
        )));
    }
    if tree.root().rule_count() == 0 {
        return Err(store_refusal(
            "this list cannot be saved: a list with no rules is never stored".to_string(),
        ));
    }
    refuse_empty_box(tree.root().inner())?;
    refuse_malformed_rule(tree.root().inner())
}

/// Refuse a box — or any box inside it — that holds nothing.
fn refuse_empty_box(boxed: &RuleBox) -> rusqlite::Result<()> {
    if boxed.children().is_empty() {
        return Err(store_refusal(
            "this list cannot be saved: a rule box with nothing in it is never stored".to_string(),
        ));
    }
    for child in boxed.children() {
        if let RuleChild::Box(inner) = child {
            refuse_empty_box(inner)?;
        }
    }
    Ok(())
}

/// Refuse a Rule the catalog says is not a whole test, naming the field it is
/// about so the reason points at the row that caused it.
fn refuse_malformed_rule(boxed: &RuleBox) -> rusqlite::Result<()> {
    for child in boxed.children() {
        match child {
            RuleChild::Rule(rule) => {
                if !rule.is_fully_formed() {
                    let display = field_catalog::spec(rule.field()).display_name();
                    return Err(store_refusal(format!(
                        "this list cannot be saved: the rule on {display} is not fully filled in"
                    )));
                }
            }
            RuleChild::Box(inner) => refuse_malformed_rule(inner)?,
        }
    }
    Ok(())
}

/// Insert a whole tree under `playlist_id`: exactly one parentless Root Box,
/// then its children depth-first, each numbered in the one shared sibling order.
///
/// Positions are renumbered from zero on every save — dense, whole-tree, no gap
/// management — which is what makes the stored order the authored order.
fn insert_tree(conn: &Connection, playlist_id: &str, root: &RootBox) -> rusqlite::Result<()> {
    let mut tree = TreeWriter::new(playlist_id);
    let root_id = tree.next_box_id();
    conn.execute(
        "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
         VALUES (?1, ?2, NULL, ?3, 0)",
        params![root_id, playlist_id, root.combine().stored_id()],
    )?;
    tree.insert_children(conn, &root_id, root.inner().children())
}

/// The store's own id source for one tree: a playlist-scoped counter, so ids are
/// unique because the playlist's previous tree was deleted in the same
/// transaction before any of these are handed out.
struct TreeWriter<'a> {
    playlist_id: &'a str,
    counter: u32,
}

impl<'a> TreeWriter<'a> {
    fn new(playlist_id: &'a str) -> Self {
        Self {
            playlist_id,
            counter: 0,
        }
    }

    /// A store-generated Rule Box id: opaque, and never handed to the frontend.
    fn next_box_id(&mut self) -> String {
        let id = format!("{}-box-{}", self.playlist_id, self.counter);
        self.counter += 1;
        id
    }

    /// A store-generated Rule id, same guarantees as [`Self::next_box_id`].
    fn next_rule_id(&mut self) -> String {
        let id = format!("{}-rule-{}", self.playlist_id, self.counter);
        self.counter += 1;
        id
    }

    /// Insert one box's children under `parent_box_id`, recursing into nested
    /// boxes, and numbering Rules and Boxes as the one interleaved list they are.
    fn insert_children(
        &mut self,
        conn: &Connection,
        parent_box_id: &str,
        children: &[RuleChild],
    ) -> rusqlite::Result<()> {
        for (position, child) in children.iter().enumerate() {
            let position = i64::try_from(position).unwrap_or(i64::MAX);
            match child {
                RuleChild::Rule(rule) => self.insert_rule(conn, parent_box_id, rule, position)?,
                RuleChild::Box(inner) => {
                    let id = self.next_box_id();
                    conn.execute(
                        "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, \
                         position) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            id,
                            self.playlist_id,
                            parent_box_id,
                            inner.combine().stored_id(),
                            position
                        ],
                    )?;
                    self.insert_children(conn, &id, inner.children())?;
                }
            }
        }
        Ok(())
    }

    fn insert_rule(
        &mut self,
        conn: &Connection,
        box_id: &str,
        rule: &Rule,
        position: i64,
    ) -> rusqlite::Result<()> {
        let id = self.next_rule_id();
        conn.execute(
            "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, position) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                box_id,
                rule.field().stored_id(),
                // The stored operator id comes from the closed enum, so there is
                // no `is_not_missing` to write: "is not missing" can only land as
                // `is_missing` with `negated` set. One meaning, one spelling.
                rule.comparison().stored_id(),
                rule.is_negated(),
                encode_value(rule.value()),
                position
            ],
        )?;
        Ok(())
    }
}

/// The id for a new Custom Smart Playlist row: the [`PlaylistId`] slug,
/// namespaced by [`SMART_ID_PREFIX`], deduped against the rows this table already
/// holds so two lists settled on in the same millisecond cannot collide.
fn new_smart_playlist_id(conn: &Connection, name: &str) -> rusqlite::Result<String> {
    let slug = PlaylistId::new(name).0;
    let mut id = format!("{SMART_ID_PREFIX}{slug}");
    let mut suffix = 2;
    loop {
        let taken: i64 = conn.query_row(
            "SELECT COUNT(*) FROM smart_playlists WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )?;
        if taken == 0 {
            return Ok(id);
        }
        id = format!("{SMART_ID_PREFIX}{slug}-{suffix}");
        suffix += 1;
    }
}

/// Whether a `smart_playlists` row carries `id`.
fn smart_playlist_exists(conn: &Connection, id: &PlaylistId) -> rusqlite::Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM smart_playlists WHERE id = ?1",
            [&id.0],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

// --- the duplicate ----------------------------------------------------------

/// Duplicate body. Runs inside
/// [`SqliteStore::duplicate_smart_playlist_write`]'s transaction, so the name it
/// settles on is still free when the copy lands.
fn duplicate_in_tx(conn: &Connection, id: &PlaylistId) -> rusqlite::Result<Option<PlaylistId>> {
    let source: Option<String> = conn
        .query_row(
            "SELECT name FROM smart_playlists WHERE id = ?1",
            [&id.0],
            |row| row.get(0),
        )
        .optional()?;
    let Some(source_name) = source else {
        return Ok(None);
    };

    let boxes = load_box_rows(conn, id)?;
    let rules = load_rule_rows(conn, id)?;
    let root_index = root_of(&boxes)?;
    refuse_uncopyable_rule(&rules)?;

    let name = free_copy_name(conn, &source_name)?;
    let copy_id = new_smart_playlist_id(conn, &name)?;
    conn.execute(INSERT_SMART_PLAYLIST, params![copy_id, name, now_nanos()])?;

    let mut copier = TreeCopier {
        boxes: &boxes,
        rules: &rules,
        writer: TreeWriter::new(&copy_id),
    };
    copier.copy_box(conn, &boxes[root_index], None)?;
    Ok(Some(PlaylistId(copy_id)))
}

/// Refuse a Duplicate whose source holds a Rule row this build could not write.
///
/// The copy is raw, so a `value` this build cannot re-type for its field rides
/// along untouched — the copy is as unreadable as the original, and as honest. A
/// `field_id` or `operator_id` outside this build's enums is a different matter:
/// re-inserting it violates the schema's `CHECK` list, and a transaction that
/// dies on a `CHECK` reports a column constraint to a listener who wanted a
/// copy. So the copy is refused **naming that Rule**, in the save's own wording,
/// before anything is written. The row's delete control is the way out there too.
fn refuse_uncopyable_rule(rules: &[RuleRow]) -> rusqlite::Result<()> {
    if let Some(row) = rules
        .iter()
        .find(|row| MatchableField::from_stored_id(&row.field_id).is_none())
    {
        return Err(store_refusal(format!(
            "this list cannot be duplicated: it holds a rule testing {:?}, which this version of \
             riff cannot understand — delete that rule before duplicating",
            row.field_id
        )));
    }
    if let Some(row) = rules
        .iter()
        .find(|row| Comparison::from_stored_id(&row.operator_id).is_none())
    {
        return Err(store_refusal(format!(
            "this list cannot be duplicated: it holds a rule testing {:?} with {:?}, which this \
             version of riff cannot understand — delete that rule before duplicating",
            row.field_id, row.operator_id
        )));
    }
    Ok(())
}

/// The first copy name no sidebar row holds: `Copy of X`, then `Copy of X 2`, `Copy of X 3` …
///
/// Free-ness is decided by [`SqliteStore::reject_taken_sidebar_name`] itself, so
/// this states no part of the name rule — it only asks the one helper whether a
/// candidate is usable. A candidate is unusable either because a row holds the
/// name or because the check could not run; both send the loop to the next
/// candidate, and the last error is what surfaces if none is free, so a genuine
/// query failure is reported rather than swallowed.
fn free_copy_name(conn: &Connection, source_name: &str) -> rusqlite::Result<String> {
    let base = format!("Copy of {}", source_name.trim());
    let mut last = String::new();
    for attempt in 1..=COPY_NAME_ATTEMPTS {
        let candidate = if attempt == 1 {
            base.clone()
        } else {
            format!("{base} {attempt}")
        };
        if let Err(e) = SqliteStore::reject_taken_sidebar_name(conn, &candidate, None) {
            last = format!(" ({e})");
            continue;
        }
        return Ok(candidate);
    }
    Err(store_refusal(format!(
        "this list cannot be duplicated: every copy name from {base:?} onward is already taken{last}"
    )))
}

/// One stored box row, read raw so a copy carries what is stored.
struct BoxRow {
    id: String,
    parent_id: Option<String>,
    combine: String,
    position: i64,
}

/// One stored rule row, read raw for the same reason: an Unreadable Rule is
/// copied as it stands rather than repaired on the way through.
struct RuleRow {
    box_id: String,
    field_id: String,
    operator_id: String,
    negated: i64,
    value: Option<String>,
    position: i64,
}

fn load_box_rows(conn: &Connection, playlist_id: &PlaylistId) -> rusqlite::Result<Vec<BoxRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, parent_id, combine, position FROM smart_rule_boxes \
         WHERE playlist_id = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map([&playlist_id.0], |row| {
        Ok(BoxRow {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            combine: row.get(2)?,
            position: row.get(3)?,
        })
    })?;
    rows.collect()
}

fn load_rule_rows(conn: &Connection, playlist_id: &PlaylistId) -> rusqlite::Result<Vec<RuleRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.box_id, r.field_id, r.operator_id, r.negated, r.value, r.position \
         FROM smart_rules r JOIN smart_rule_boxes b ON b.id = r.box_id \
         WHERE b.playlist_id = ?1 ORDER BY r.position",
    )?;
    let rows = stmt.query_map([&playlist_id.0], |row| {
        Ok(RuleRow {
            box_id: row.get(0)?,
            field_id: row.get(1)?,
            operator_id: row.get(2)?,
            negated: row.get(3)?,
            value: row.get(4)?,
            position: row.get(5)?,
        })
    })?;
    rows.collect()
}

/// The parentless box of a tree — which the save path guarantees is exactly one,
/// and which a read and a copy therefore refuse to invent.
fn root_of(boxes: &[BoxRow]) -> rusqlite::Result<usize> {
    let mut roots = boxes
        .iter()
        .enumerate()
        .filter(|(_, row)| row.parent_id.is_none());
    let (index, _) = roots.next().ok_or_else(|| {
        store_refusal(
            "this list's rule tree has no root rule box, so there is nothing to read or copy"
                .to_string(),
        )
    })?;
    if roots.next().is_some() {
        return Err(store_refusal(
            "this list's rule tree holds more than one root rule box".to_string(),
        ));
    }
    Ok(index)
}

/// The box-and-rule copy, driven from the source's own rows.
struct TreeCopier<'a> {
    boxes: &'a [BoxRow],
    rules: &'a [RuleRow],
    writer: TreeWriter<'a>,
}

impl TreeCopier<'_> {
    /// Copy one box, then its Rules, then every box below it — parent before
    /// child, because `parent_id` is a foreign key, so the insertion order is the
    /// traversal order. Positions are copied as they are; ids are not.
    fn copy_box(
        &mut self,
        conn: &Connection,
        source: &BoxRow,
        parent_new_id: Option<&str>,
    ) -> rusqlite::Result<()> {
        let copy_playlist_id = self.writer.playlist_id;
        let new_id = self.writer.next_box_id();
        conn.execute(
            "INSERT INTO smart_rule_boxes (id, playlist_id, parent_id, combine, position) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                new_id,
                copy_playlist_id,
                parent_new_id,
                source.combine,
                source.position
            ],
        )?;

        for rule in self.rules.iter().filter(|row| row.box_id == source.id) {
            let rule_id = self.writer.next_rule_id();
            conn.execute(
                "INSERT INTO smart_rules (id, box_id, field_id, operator_id, negated, value, \
                 position) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    rule_id,
                    new_id,
                    rule.field_id,
                    rule.operator_id,
                    rule.negated,
                    rule.value,
                    rule.position
                ],
            )?;
        }

        let children: Vec<usize> = self
            .boxes
            .iter()
            .enumerate()
            .filter(|(_, row)| row.parent_id.as_deref() == Some(source.id.as_str()))
            .map(|(index, _)| index)
            .collect();
        for index in children {
            self.copy_box(conn, &self.boxes[index], Some(&new_id))?;
        }
        Ok(())
    }
}

// --- the read ---------------------------------------------------------------

/// Read body: one playlist row's whole tree, or `None` when no row carries `id`.
fn read_tree_on(conn: &Connection, id: &PlaylistId) -> rusqlite::Result<Option<SmartPlaylistTree>> {
    if !smart_playlist_exists(conn, id)? {
        return Ok(None);
    }
    let boxes = load_box_rows(conn, id)?;
    let rules = load_rule_rows(conn, id)?;
    let root_index = root_of(&boxes)?;

    let mut unreadable: Vec<UnreadableRule> = Vec::new();
    let root_box = assemble_box(
        &boxes[root_index],
        &boxes,
        &rules,
        Vec::new(),
        &mut unreadable,
    )?;
    Ok(Some(
        SmartPlaylistTree::new(RootBox::new(root_box)).with_unreadable(unreadable),
    ))
}

/// Assemble one box: its nested boxes and Rules, interleaved by the single
/// position numbering the schema stores, emitted in tree order.
///
/// A Rule this build cannot read is not coerced, guessed at or dropped: it goes
/// into `unreadable` carrying its raw ids and where it hangs, and the typed tree
/// simply lacks that sibling — which is why a read's own numbering can show a
/// hole. `path` addresses the box from the Root Box, counted in *box* children,
/// and is what puts an Unreadable Rule back where it sits.
fn assemble_box(
    box_row: &BoxRow,
    boxes: &[BoxRow],
    rules: &[RuleRow],
    path: Vec<usize>,
    unreadable: &mut Vec<UnreadableRule>,
) -> rusqlite::Result<RuleBox> {
    let combine = Combine::from_stored_id(&box_row.combine).ok_or_else(|| {
        store_refusal(format!(
            "this list's rule tree cannot be read: a box combines with {:?}, which this version \
             of riff does not know",
            box_row.combine
        ))
    })?;

    // Both kinds of child in one sequence, ordered by the numbering they share.
    let mut box_children: Vec<&BoxRow> = boxes
        .iter()
        .filter(|row| row.parent_id.as_deref() == Some(box_row.id.as_str()))
        .collect();
    box_children.sort_by_key(|row| row.position);
    let mut rule_children: Vec<&RuleRow> = rules
        .iter()
        .filter(|row| row.box_id == box_row.id)
        .collect();
    rule_children.sort_by_key(|row| row.position);

    let mut children: Vec<RuleChild> = Vec::with_capacity(box_children.len() + rule_children.len());
    let mut next_box = 0_usize;
    let mut next_rule = 0_usize;
    while next_box < box_children.len() || next_rule < rule_children.len() {
        let take_box = match (
            box_children.get(next_box).map(|row| row.position),
            rule_children.get(next_rule).map(|row| row.position),
        ) {
            (Some(box_position), Some(rule_position)) => box_position <= rule_position,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };
        if take_box {
            let row = box_children[next_box];
            let mut child_path = path.clone();
            child_path.push(next_box);
            let nested = assemble_box(row, boxes, rules, child_path, unreadable)?;
            children.push(RuleChild::Box(nested));
            next_box += 1;
        } else {
            let row = rule_children[next_rule];
            let position = usize::try_from(row.position).unwrap_or(usize::MAX);
            match read_rule(row) {
                Some(rule) => children.push(RuleChild::Rule(rule)),
                None => unreadable.push(UnreadableRule::new(
                    path.clone(),
                    position,
                    row.field_id.clone(),
                    row.operator_id.clone(),
                    row.negated != 0,
                    row.value.clone(),
                )),
            }
            next_rule += 1;
        }
    }
    Ok(RuleBox::new(combine).with_children(children))
}

/// One stored rule row as a [`Rule`], or `None` when this build cannot read it:
/// an unknown field id, an unknown operator id, or a value that does not fit the
/// field's type — including a value stored where `is missing` stores none.
/// `None` is the detection, and the caller reports the row exactly as stored.
fn read_rule(row: &RuleRow) -> Option<Rule> {
    let field = MatchableField::from_stored_id(&row.field_id)?;
    let comparison = Comparison::from_stored_id(&row.operator_id)?;
    let value = if comparison == Comparison::IsMissing {
        if row.value.is_some() {
            return None;
        }
        None
    } else {
        let raw = row.value.as_deref()?;
        Some(decode_value(field, comparison, Some(raw))?)
    };
    let rule = Rule::new(field, comparison, value);
    Some(if row.negated != 0 {
        rule.negated()
    } else {
        rule
    })
}

// --- the value column -------------------------------------------------------

/// The `TEXT` a Rule's value becomes, per the storage rules: a year and every
/// other number as its digits (`1975`), a duration as **whole milliseconds**,
/// `in_last` as a **whole number of days**, `before`/`after` as
/// `YYYY-MM-DD HH:MM:SS`, a yes/no as `1`/`0`, text as itself — and `NULL`
/// exactly where the field takes no value at all, which is `is missing` and
/// nothing else.
///
/// Nothing here infers a shape from the field id: the value arrives already typed
/// by [`RuleValue`], which is the catalog's answer recorded once, in ticket 01.
fn encode_value(value: Option<&RuleValue>) -> Option<String> {
    value.map(|value| match value {
        RuleValue::Text(text) => text.clone(),
        RuleValue::Number(number) => number.to_string(),
        RuleValue::DurationMillis(millis) => millis.to_string(),
        RuleValue::Days(days) => days.to_string(),
        RuleValue::DateTime(moment) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            moment.year, moment.month, moment.day, moment.hour, moment.minute, moment.second
        ),
        RuleValue::YesNo(yes) => if *yes { "1" } else { "0" }.to_string(),
    })
}

/// The [`RuleValue`] a stored `TEXT` means for this field, or `None` when this
/// build cannot make one out of it — which makes the row Unreadable rather than
/// repaired or dropped.
///
/// The type comes from the catalog's row for the field and is nowhere restated.
/// The one place an operator decides the shape is a history date, which the
/// catalog answers as a Date and the comparison tells apart: a window of whole
/// days for `in the last`, a day plus a time for `before` and `after`.
fn decode_value(
    field: MatchableField,
    comparison: Comparison,
    raw: Option<&str>,
) -> Option<RuleValue> {
    let expected = field_catalog::spec(field).value_type();
    match expected {
        ValueType::Text => raw.map(|text| RuleValue::Text(text.to_string())),
        ValueType::Number => raw
            .and_then(|text| text.parse::<i64>().ok())
            .map(RuleValue::Number),
        ValueType::Duration => raw
            .and_then(|text| text.parse::<u64>().ok())
            .map(RuleValue::DurationMillis),
        ValueType::YesNo => match raw {
            Some("1") => Some(RuleValue::YesNo(true)),
            Some("0") => Some(RuleValue::YesNo(false)),
            _ => None,
        },
        ValueType::Date => {
            if comparison == Comparison::InLast {
                raw.and_then(|text| text.parse::<u32>().ok())
                    .map(RuleValue::Days)
            } else {
                raw.and_then(parse_civil_datetime).map(RuleValue::DateTime)
            }
        }
    }
}

/// `YYYY-MM-DD HH:MM:SS`, zone-less by design: the listener's own clock on the
/// machine reading it is the whole of its meaning, and a stored zone would make
/// one saved Rule answer differently on another machine.
fn parse_civil_datetime(raw: &str) -> Option<CivilDateTime> {
    let (date, time) = raw.split_once(' ')?;
    let mut days = date.split('-');
    let year = days.next()?.parse::<i32>().ok()?;
    let month = days.next()?.parse::<u8>().ok()?;
    let day = days.next()?.parse::<u8>().ok()?;
    if days.next().is_some() {
        return None;
    }
    let mut clock = time.split(':');
    let hour = clock.next()?.parse::<u8>().ok()?;
    let minute = clock.next()?.parse::<u8>().ok()?;
    let second = clock.next()?.parse::<u8>().ok()?;
    if clock.next().is_some() {
        return None;
    }
    Some(CivilDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}
