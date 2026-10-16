// riff-infra — the sidebar-wide name rule: one name, one sidebar row.
//
// A Playlist name and a Custom Smart Playlist name identify the same thing —
// the row the listener clicks — so the store's mutation path reads BOTH
// `playlists` and `smart_playlists` and refuses a name one of them already
// answers to, compared trimmed and lowercased with Rust's full Unicode
// folding. Asserted at the `PlaylistStore` seam against a real SQLite file in
// a fresh temp directory per test: never a table's internals, never which
// helper ran. The Custom Smart Playlist save path does not exist yet (that is
// the write-path ticket), so the Smart-Playlist half of the rule is pinned by
// seeding a `smart_playlists` row with SQL — which proves the check reads the
// second table without inventing a write path for it.
//
// The six Built-in Smart Playlist display names are deliberately absent here:
// they exist only as code, so refusing them is an application-layer rule.
//
// Run: cargo test -p riff-infra --test integration playlist_name_uniqueness

use super::*;

/// A fresh, fully migrated store — so the three rule tables exist and the
/// connection carries the store's own pragmas (`foreign_keys`, WAL). Every
/// test starts from its own temp directory.
fn fresh_store() -> (
    tempfile::TempDir,
    SqliteStore,
    crossbeam_channel::Receiver<StoreChanged>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let store = SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("opening the Application Store must succeed");
    (dir, store, changes_rx)
}

/// Seed one `smart_playlists` row directly.
///
/// This is deliberate: the Custom Smart Playlist save path is not built yet,
/// and a test fixture must not invent one. Raw SQL is the only legal way to
/// put a Smart Playlist name in the store today — and the rule under test
/// lives in the read of that table, not in whoever wrote the row, which is
/// exactly what the ticket asks the check to guarantee ("it holds however the
/// data arrives").
fn seed_smart_playlist(store: &SqliteStore, id: &str, name: &str) {
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO smart_playlists (id, name, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, name, 1_700_000_000_000_000_000_i64],
            )
        })
        .expect("seeding a Custom Smart Playlist row must work");
}

/// The refusal's reason, or a panic naming what came back instead.
///
/// A store refusal has exactly one spelling — [`StoreError::InvalidOperation`]
/// carrying a human-readable reason — so this also pins "not a new variant,
/// and not a panic".
fn refusal_reason<T: std::fmt::Debug>(outcome: Result<T, StoreError>, what: &str) -> String {
    match outcome {
        Err(StoreError::InvalidOperation(reason)) => reason,
        other => panic!(
            "{what} must be refused as a `StoreError::InvalidOperation`, got {other:?}"
        ),
    }
}

/// The names the sidebar actually holds, as one comparable set.
fn held_names(store: &SqliteStore) -> Vec<String> {
    store
        .load_playlists()
        .expect("loading playlists must work")
        .into_iter()
        .map(|playlist| playlist.name)
        .collect()
}

#[test]
fn creating_onto_a_name_a_playlist_already_holds_is_refused() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    store
        .create_playlist("Night Drive", &[])
        .expect("the first row's name is free");

    let reason = refusal_reason(
        store.create_playlist("Night Drive", &[]).map(|_| ()),
        "creating a second row under a held name",
    );
    assert!(
        reason.contains("Night Drive"),
        "the refusal must name the offending value, got {reason:?}"
    );
    assert_eq!(
        held_names(&store),
        vec!["Night Drive".to_string()],
        "a refused create commits no row"
    );
}

#[test]
fn renaming_onto_a_name_another_playlist_holds_is_refused() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    store
        .create_playlist("Night Drive", &[])
        .expect("the first name is free");
    let other = store
        .create_playlist("Wash", &[])
        .expect("a second name is free");

    let reason = refusal_reason(
        store.rename_playlist(&other, "Night Drive").map(|_| ()),
        "renaming onto a held name",
    );
    assert!(
        reason.contains("Night Drive"),
        "the refusal must name the offending value, got {reason:?}"
    );
    assert_eq!(
        held_names(&store),
        vec!["Night Drive".to_string(), "Wash".to_string()],
        "a refused rename rewrites no name"
    );
    let untouched = store
        .load_playlists()
        .expect("loading playlists must work")
        .into_iter()
        .find(|playlist| playlist.id == other)
        .expect("the row being renamed is still there");
    assert_eq!(
        untouched.name, "Wash",
        "and it still answers to its own name"
    );
}

#[test]
fn creating_under_a_name_no_other_row_holds_is_allowed() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    store
        .create_playlist("Night Drive", &[])
        .expect("the first name is free");
    seed_smart_playlist(&store, "sp-1", "Chill");

    let id = store
        .create_playlist("Morning Coffee", &[])
        .expect("a name no row holds must be creatable");
    assert_eq!(
        held_names(&store),
        vec!["Night Drive".to_string(), "Morning Coffee".to_string()],
        "the new row carries the name it was asked for"
    );
    // Distinct names stay distinct rows even when they slug alike — the
    // id-dedup loop is about ids, and the name rule never reaches into it.
    let other = store
        .create_playlist("Morning.Coffee", &[])
        .expect("a punctuated sibling of a held name is a different name");
    assert_ne!(id, other, "two rows cannot share an id");
    assert_eq!(
        held_names(&store),
        vec![
            "Night Drive".to_string(),
            "Morning Coffee".to_string(),
            "Morning.Coffee".to_string()
        ],
        "and each keeps its own name"
    );
}

#[test]
fn creating_onto_a_custom_smart_playlist_name_is_refused() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    // No Custom Smart Playlist save path exists yet, so the Smart Playlist
    // half of the sidebar arrives by seeding the table directly.
    seed_smart_playlist(&store, "sp-1", "Night Drive");

    let reason = refusal_reason(
        store.create_playlist("Night Drive", &[]).map(|_| ()),
        "creating a Playlist under a Custom Smart Playlist's name",
    );
    assert!(
        reason.contains("Night Drive"),
        "the refusal must name the offending value, got {reason:?}"
    );
    assert!(
        held_names(&store).is_empty(),
        "a refused create commits no row, so the sidebar still holds one row under that name"
    );
}

#[test]
fn renaming_onto_a_custom_smart_playlist_name_is_refused() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    seed_smart_playlist(&store, "sp-1", "Night Drive");
    let playlist = store
        .create_playlist("Wash", &[])
        .expect("a free name is creatable");

    let reason = refusal_reason(
        store
            .rename_playlist(&playlist, "Night Drive")
            .map(|_| ()),
        "renaming a Playlist onto a Custom Smart Playlist's name",
    );
    assert!(
        reason.contains("Night Drive"),
        "the refusal must name the offending value, got {reason:?}"
    );
    assert_eq!(
        held_names(&store),
        vec!["Wash".to_string()],
        "and the refused rename rewrote nothing"
    );
}

#[test]
fn the_case_of_a_held_name_is_invisible_to_the_rule() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    store
        .create_playlist("Night Drive", &[])
        .expect("a free name is creatable");
    assert!(
        store.create_playlist("NIGHT DRIVE", &[]).is_err(),
        "an uppercase spelling of a held Playlist name is the same name"
    );
    assert!(
        store.create_playlist("night drive", &[]).is_err(),
        "and so is a lowercase one"
    );

    // A Custom Smart Playlist holding the mixed-case spelling refuses the
    // Playlist write that folds DOWN onto it...
    seed_smart_playlist(&store, "sp-1", "Chill Winds");
    let reason = refusal_reason(
        store.create_playlist("chill winds", &[]).map(|_| ()),
        "creating under the lowercase of a held Custom Smart Playlist name",
    );
    assert!(
        reason.contains("chill winds"),
        "the refusal must name the offending value, got {reason:?}"
    );

    // ...and one holding the folded spelling refuses the write that folds UP
    // to it. Both directions, across both tables.
    let (_dir, mut store, _changes_rx) = fresh_store();
    seed_smart_playlist(&store, "sp-1", "morning coffee");
    assert!(
        store.create_playlist("Morning Coffee", &[]).is_err(),
        "the uppercase spelling of a held Custom Smart Playlist name is \
         the same name, and the rule reads that table too"
    );
    assert_eq!(
        held_names(&store),
        Vec::<String>::new(),
        "and the refusal created no row"
    );
}

/// Names are identity and fold with Rust's full Unicode lowercase, while a
/// Rule's tag needle is content and folds through SQLite's ASCII-only
/// `lower()` (ADR 0020). That asymmetry is deliberate — and this test is what
/// keeps it from being "unified": SQL `lower('РОК')` is still `РОК`, so a
/// check that folded in the database would happily let both rows exist.
#[test]
fn a_non_latin_name_folds_both_ways_where_sql_lower_cannot() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    seed_smart_playlist(&store, "sp-upper", "РОК");
    let reason = refusal_reason(
        store.create_playlist("рок", &[]).map(|_| ()),
        "creating under the lowercase of a held Cyrillic name",
    );
    assert!(
        reason.contains("рок"),
        "the refusal must name the offending value, got {reason:?}"
    );

    // The other direction, with the Playlist holding the lowercase spelling.
    let (_dir, mut store, _changes_rx) = fresh_store();
    store
        .create_playlist("рок", &[])
        .expect("a free name is creatable");
    assert!(
        store.create_playlist("РОК", &[]).is_err(),
        "the uppercase spelling of a held non-Latin name is the same name, \
         which only Rust's folding can see"
    );
    assert_eq!(
        held_names(&store),
        vec!["рок".to_string()],
        "and the refusal committed nothing"
    );
}

#[test]
fn the_comparison_trims_before_it_folds() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    store
        .create_playlist("  Night Drive  ", &[])
        .expect("a free name is creatable");
    let other = store
        .create_playlist("Wash", &[])
        .expect("a second free name is creatable");
    assert_eq!(
        held_names(&store),
        vec!["Night Drive".to_string(), "Wash".to_string()],
        "the stored name is the trimmed one, exactly as this path always wrote it"
    );

    // The same name with different padding AND a different case is one name.
    let reason = refusal_reason(
        store.create_playlist("  night DRIVE ", &[]).map(|_| ()),
        "creating under a padded, folded twin of a held name",
    );
    assert!(
        reason.contains("night DRIVE"),
        "the refusal names the value as it was written, trimmed, got {reason:?}"
    );

    let reason = refusal_reason(
        store.rename_playlist(&other, "Night Drive   ").map(|_| ()),
        "renaming onto a padded twin of a held name",
    );
    assert!(
        reason.contains("Night Drive"),
        "trimmed, then folded — got {reason:?}"
    );
    assert_eq!(
        held_names(&store),
        vec!["Night Drive".to_string(), "Wash".to_string()],
        "and neither refusal wrote anything"
    );
}

/// A rename that keeps a row's own name must never refuse the row it started
/// from — the same exemption a Custom Smart Playlist re-save needs, expressed
/// through the id of the row being written.
#[test]
fn a_row_is_exempt_from_its_own_name() {
    let (_dir, mut store, _changes_rx) = fresh_store();
    let kept = store
        .create_playlist("Night Drive", &[])
        .expect("a free name is creatable");
    store
        .create_playlist("Wash", &[])
        .expect("a second free name is creatable");

    assert!(
        store
            .rename_playlist(&kept, "Night Drive")
            .expect("the row's own name is not a clash"),
        "a rename that changes nothing still finds its row"
    );
    assert!(
        store
            .rename_playlist(&kept, "  night drive ")
            .expect("the same name folded and trimmed is still its own name"),
        "a rename onto a spelling of its own name is not a clash either"
    );
    assert_eq!(
        held_names(&store),
        vec!["night drive".to_string(), "Wash".to_string()],
        "and the row now carries the trimmed spelling it was given"
    );
    // Only the writing row is exempt: the OTHER row's name is still held.
    assert!(
        store
            .rename_playlist(&kept, "Wash")
            .expect_err("another row's name is a clash")
            .to_string()
            .contains("Wash"),
        "and the refusal names that value"
    );
}

/// The refusal is one store fault like any other: the single
/// `StoreError::InvalidOperation` variant, and — because nothing committed —
/// no generation moved and no notice emitted.
#[test]
fn a_refusal_is_one_invalid_operation_and_moves_nothing() {
    let (_dir, mut store, changes_rx) = fresh_store();
    let kept = store
        .create_playlist("Night Drive", &[])
        .expect("a free name is creatable");
    let other = store
        .create_playlist("Wash", &[])
        .expect("a second free name is creatable");
    // Drain the two committed creates, so what follows sees only the refusals.
    while changes_rx.try_recv().is_ok() {}
    let settled = store.playlist_generation().current();

    let refused_create = store.create_playlist("Night Drive", &[]);
    let refused_rename = store.rename_playlist(&other, "NIGHT DRIVE");
    let refused_self = store.rename_playlist(&kept, "Wash");

    for (label, outcome) in [
        ("create", refused_create.map(|_| ())),
        ("rename", refused_rename.map(|_| ())),
        ("rename", refused_self.map(|_| ())),
    ] {
        refusal_reason(outcome, &format!("a {label} onto a held name"));
    }
    assert_eq!(
        store.playlist_generation().current(),
        settled,
        "a refusal bumps no session generation — the transaction committed nothing"
    );
    assert!(
        changes_rx.try_recv().is_err(),
        "and the emit-beside-bump invariant means no StoreChanged notice either"
    );
    assert_eq!(
        held_names(&store),
        vec!["Night Drive".to_string(), "Wash".to_string()],
        "with every row still under the name it had"
    );
}
