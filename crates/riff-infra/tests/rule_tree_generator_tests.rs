//! Generated Rule trees — the property half of the Custom Smart Playlist
//! oracle (smart-playlist-rules ticket 10).
//!
//! Ticket 09 hand-built trees that pin one decision each. This suite builds
//! them by the hundred and asks, for every one of them, that the store's
//! compiled predicate answers exactly what the independent Rust walk answers —
//! Track identity, in order, with a total that counts the same rows. That is
//! the only arrangement that can find the interaction nobody wrote a case for.
//!
//! **The reference stays independent, and that is the whole value of the
//! run.** The walk lives in `store_tests::rules_reference` and this module
//! reaches it and nothing else: no helper shared with the query compiler, and
//! in particular no import of `riff_infra`'s field-id → column table
//! (`crates/riff-infra/src/store/field_columns.rs`). That table is the
//! compiler's own knowledge of which column a field tests; if the reference
//! read it, an agreement between the two sides would say nothing about the
//! column choice — it would only say the same table was read twice. The same
//! reason keeps the corpus's `Track` list, rather than any query, as the
//! reference's input. A future refactor that "unifies" the two sides does not
//! simplify this suite, it empties it.
//!
//! **Reproducing a failure.** Every tree comes from a `const` seed and a
//! sixteen-line xorshift (no RNG dependency, and `Cargo.lock` does not move).
//! A failure message carries the seed, the tree's index in the run, its depth,
//! and the tree printed as the very builder calls the generator used, so the
//! counterexample pastes straight into a fixed `#[test]` beside the seed it
//! came from.
//!
//! **The box-level fold, stated because it is tempting to "prove" it.** No
//! generated tree here distinguishes the compiler's `coalesce` at the *Box*
//! level on membership: every leaf is already two-valued, so `AND` / `OR` over
//! two-valued arms cannot produce a third answer, and the fold is defence in
//! depth for a leaf shape the compiler does not know yet. Ticket 09 mutated it
//! away and nothing went red on its matrix; ticket 10's own mutation run — the
//! fold removed, the whole `riff-infra` integration target walked: 253 passed,
//! 2 failed, both engine-depth pins, zero membership or paging reds —
//! reconfirms it over these generated trees rather than assuming it. So `test_the_box_level_fold_is_pinned_by_the_engine_depth_it_spends`
//! pins the fold the only way it is observable — the expression depth it costs
//! per Box level, measured at the exact engine boundary — and no test here
//! claims the fold changes an answer.

use super::*;
use crate::store_tests::rules_reference;
use crate::store_tests::{
    all_of, any_of, assert_rule_membership, boundary, civil, days, millis, needle, number, rule,
    rule_clock, rule_name, rule_not, rule_total, rule_window, seeded_rules_store, yes,
};
use riff_persistence::field_catalog::{self, ValueType, ValueWidget};
use riff_persistence::rule::{CivilDateTime, Combine, Comparison, MatchableField, RuleValue};
use std::collections::BTreeSet;

/// The generator's seed. Constant, so a failure replays: change it deliberately
/// and the whole corpus of trees changes with it.
const GENERATOR_SEED: u64 = 0x5DEE_71C0_10A9_3B77;
/// A second, independent stream for the paging half, so a paging counterexample
/// is not also the membership run's tree number 213.
const PAGED_SEED: u64 = 0xC0FF_EE00_D710_0001;
/// …and for the unreadable-tree half, for the same reason.
const CORRUPT_SEED: u64 = 0x10CC_071E_5B10_0002;
/// …and for the box-fold run.
const BOX_FOLD_SEED: u64 = 0xB0F0_1D10_A900_0003;

/// How many random trees the membership run walks. A few hundred is where an
/// interaction bug shows up — the pair-wise field/comparison coverage below is
/// already systematic, so this is the interaction budget, and it is deliberately
/// not in the tens of thousands: the number is recorded in the ticket file with
/// units, and every extra tree is CI time.
const GENERATED_TREES: usize = 400;
/// How many trees the paging run walks, each under four orders and three
/// needles. Smaller because each tree costs a dozen reads rather than two.
const PAGED_TREES: usize = 60;

/// The editor's nesting ceiling, in Box levels. The store counts no levels — a
/// deeper tree from a newer riff is answered, not refused — so the ceiling is
/// only a *coverage* target here, and the engine's own depth limit is pinned by
/// its own test below.
const EDITOR_DEPTH_CEILING: usize = 7;
/// Arms a generated Box draws before it is satisfied, as an upper bound. Boxes
/// may hold fewer.
const MAX_ARMS_PER_BOX: usize = 3;
/// Rules a generated tree may hold, as a budget. It exists so a seven-level
/// branching tree stays a small statement: Rules are uncapped in the product,
/// and the wide-tree ceiling is already pinned by its own case in
/// `store_tests`.
const MAX_RULES_PER_TREE: usize = 24;
/// How often a leaf in a focused tree reuses that tree's field. High enough
/// that same-field arms — the shape where the two-valued leaf rule earns its
/// keep — are routine rather than lucky.
const FOCUS_CHANCE: usize = 60;

/// One field, one comparison, one polarity: the unit the coverage floor is
/// counted in.
type Triple = (MatchableField, Comparison, bool);

/// The rows of the four shared value pools that split this corpus, one per
/// field the catalog gives a number widget. A pool is a *value* choice, never a
/// legality claim: which comparisons a field takes and whether it draws a value
/// at all stay the catalog's answers, asked through
/// [`field_catalog::FieldSpec::comparisons`] and
/// [`field_catalog::FieldSpec::value_widgets`] like every other caller.
const YEAR_POOL: &[i64] = &[
    1899, 1900, 1974, 1975, 1988, 1990, 1999, 2001, 2010, 2019, 2024, 2025, 3000,
];
const PLAY_COUNT_POOL: &[i64] = &[0, 1, 2, 3, 4, 7, 12, 13];
const SAMPLE_RATE_POOL: &[i64] = &[8_000, 22_050, 44_100, 44_101, 48_000, 96_000];
const CHANNEL_POOL: &[i64] = &[0, 1, 2, 3, 5, 16];

/// Needles that hit, miss, fold oddly or sit on the documented disagreements.
/// The first entry is the empty string, which the two sides turn out to agree
/// on (`instr(X, '')` is 1 in `SQLite`, and Rust's "the empty string is inside
/// everything" is true), so `contains ''` matches every row on both sides —
/// measured, and kept in the pool because it is a case a hand-written table
/// would never think to ask.
const TEXT_POOL: &[&str] = &[
    "",
    " ",
    "alpha",
    "Alpha",
    "known",
    "Known Artist",
    "unknown artist",
    "aa",
    "first",
    "bach",
    "iva",
    "notes",
    "ambient",
    "electronic",
    "électronique",
    "ÉLECTRONIQUE",
    "progressive rock",
    "art rock",
    "rock;progressive",
    // Two whole segments in order: on genre `is` this asks for the sequence
    // (ADR 0020), which the corpus's `one.flac` answers and its tab-padded
    // near-twin does not. Drawn for any text field, it is an ordinary
    // substring there.
    "art rock;progressive rock",
    "trance",
    "100%",
    "%_done",
    "\"yes\"",
    "golf",
    "november",
    "oscar",
    "no such thing",
];

/// Duration thresholds in whole milliseconds, including the corpus's own
/// round-hour pair and its one-millisecond split.
const DURATION_POOL: &[u64] = &[
    0,
    1,
    61_000,
    120_000,
    180_000,
    299_999,
    300_000,
    500_000,
    3_599_999,
    3_600_000,
    3_600_001,
    86_400_000,
    u64::MAX,
];

/// Day counts for `in the last`, each whole days back from the read. The
/// corpus's own history margins are whole days too, which is what keeps a
/// window's verdict away from a boundary by microseconds of clock drift rather
/// than by luck.
const DAY_POOL: &[u32] = &[0, 1, 2, 3, 4, 7, 20, 30, 60, 61, 400, 401, 500, u32::MAX];

/// Zone-less `before` / `after` boundaries: the corpus's own local-midnight
/// pair and the instants it brackets, plus boundaries far outside it and the
/// saturated shapes a hand-edited row can hold. `civil` is the same spelling a
/// fixed case uses, and the out-of-range last one is a deliberate probe of the
/// compiler's claim that "parts out of range simply never order beside a real
/// stamp, which is false — never unknown, never a crash".
const BOUNDARY_POOL: &[CivilDateTime] = &[
    civil(1970, 1, 1, 0, 0, 0),
    civil(2000, 1, 1, 0, 0, 0),
    civil(2024, 1, 5, 12, 0, 0),
    civil(2024, 3, 1, 23, 30, 0),
    civil(2024, 3, 2, 0, 0, 0),
    civil(2024, 3, 2, 0, 30, 0),
    civil(2024, 6, 1, 9, 0, 0),
    civil(2024, 6, 2, 0, 0, 0),
    civil(2030, 12, 31, 23, 59, 59),
    civil(-1, 13, 45, 99, 99, 99),
];

/// xorshift64\*, in four lines: enough randomness for a tree corpus, and the
/// seed constant is then the entire reproducible history of a run. No RNG crate
/// is added — this suite must not move `Cargo.lock`.
struct Rng {
    state: u64,
}

impl Rng {
    const fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A residue in `0..bound`. Every call site passes a pool length or a
    /// percentage, so a zero bound is a bug in this file rather than an input.
    fn below(&mut self, bound: usize) -> usize {
        let bound = u64::try_from(bound).expect("a pool length fits in u64");
        usize::try_from(self.next_u64() % bound).expect("a residue under the bound fits in usize")
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, options: &'a [T]) -> &'a T {
        assert!(!options.is_empty(), "a generator pool is never empty");
        &options[self.below(options.len())]
    }
}

/// The number pool a numeric field draws from. Keyed by field, because a play
/// count and a sample rate are different questions; keyed nowhere for legality,
/// because the catalog already decided the field takes a number.
fn number_pool(field: MatchableField) -> &'static [i64] {
    match field {
        MatchableField::PlayCount => PLAY_COUNT_POOL,
        MatchableField::SampleRate => SAMPLE_RATE_POOL,
        MatchableField::Channels => CHANNEL_POOL,
        // Years, and any field whose number widget this suite has not been
        // given a pool for: the fallback is a *year-shaped* number, which is
        // still a legal value for a numeric field. `Generator::value_for`
        // asserts the drawn value's type against the field's own, so the day the
        // catalog starts drawing a number widget somewhere else is a failing
        // assertion rather than a silently mis-shaped corpus.
        _ => YEAR_POOL,
    }
}

/// Draws one tree. Borrows the stream so a run of trees is one sequence, and
/// carries the per-tree rule budget and the tree's focus field.
struct Generator<'rng> {
    rng: &'rng mut Rng,
    rules_left: usize,
    focus: Option<MatchableField>,
}

impl<'rng> Generator<'rng> {
    fn new(rng: &'rng mut Rng, focus: Option<MatchableField>) -> Self {
        Self {
            rng,
            rules_left: MAX_RULES_PER_TREE,
            focus,
        }
    }

    /// One leaf: a field — the tree's focus most of the time, which is what puts
    /// a value arm and an absence arm on one column — one comparison that field
    /// permits, the value the catalog's widget answer calls for, and a polarity.
    fn leaf(&mut self, forced: Option<Triple>) -> RuleTree {
        let (field, comparison, negated) = if let Some(triple) = forced {
            triple
        } else {
            let field = match self.focus {
                Some(focus) if self.rng.chance(FOCUS_CHANCE) => focus,
                _ => *self.rng.pick(&MatchableField::ALL),
            };
            let comparison = *self.rng.pick(field_catalog::spec(field).comparisons());
            (field, comparison, self.rng.chance(50))
        };
        let value = self.value_for(field, comparison);
        if negated {
            rule_not(field, comparison, value)
        } else {
            rule(field, comparison, value)
        }
    }

    /// The value a leaf holds, asked of the catalog: no widget, no value. Every
    /// pool entry is checked against the field's own value type, so a catalog
    /// change cannot quietly turn this suite's trees into unreadable ones.
    fn value_for(&mut self, field: MatchableField, comparison: Comparison) -> Option<RuleValue> {
        let row = field_catalog::spec(field);
        let drawn = match row.value_widgets(comparison).first() {
            None => return None,
            Some(ValueWidget::Text | ValueWidget::ChooseFromLibrary) => {
                needle(self.rng.pick(TEXT_POOL))
            }
            Some(ValueWidget::Number) => number(*self.rng.pick(number_pool(field))),
            Some(ValueWidget::Duration) => millis(*self.rng.pick(DURATION_POOL)),
            Some(ValueWidget::DateWindow) => days(*self.rng.pick(DAY_POOL)),
            Some(ValueWidget::DateTime) => boundary(*self.rng.pick(BOUNDARY_POOL)),
            Some(ValueWidget::YesNo) => yes(self.rng.chance(50)),
        };
        if let Some(value) = &drawn {
            assert_eq!(
                value.value_type(),
                row.value_type(),
                "the pool for {:?} / {comparison:?} drew a value of the wrong kind — the \
                 catalog and this suite's pools have started to disagree",
                row.display_name(),
            );
        }
        drawn
    }

    /// One Box, `levels_left` deep including itself. The deepest path is exactly
    /// as long as asked (a nested Box is forced while levels remain), so the
    /// depth coverage claim below is about real trees rather than about the
    /// number this file passed around.
    fn box_at(&mut self, levels_left: usize, forced: Option<Triple>) -> RuleTree {
        let mut children: Vec<RuleTree> = Vec::new();
        if let Some(triple) = forced {
            children.push(self.leaf(Some(triple)));
            self.rules_left = self.rules_left.saturating_sub(1);
        }
        let arms = 1 + self.rng.below(MAX_ARMS_PER_BOX);
        while children.len() < arms && self.rules_left > 0 {
            children.push(self.leaf(None));
            self.rules_left -= 1;
        }
        if children.is_empty() {
            // A Box with no arm is never stored and never generated either: the
            // budget ran out, and one leaf is cheaper than an unreadable tree.
            children.push(self.leaf(None));
        }
        if levels_left > 1 {
            // Exactly one nested Box, so the deepest path is as long as asked
            // and the tree's size stays bounded by the budget plus one forced
            // Rule per level. Branching still happens — a Box beside the Rule
            // arms above it is the other shape the editor draws — and it is the
            // arms that carry the interactions this run is about.
            let child = self.box_at(levels_left - 1, None);
            children.push(child);
        }
        if self.rng.chance(50) {
            all_of(children)
        } else {
            any_of(children)
        }
    }
}

/// Build one tree exactly `levels` Box levels deep, with `forced` as one of its
/// arms when given, over `focus`'s field when given.
fn generate(
    rng: &mut Rng,
    levels: usize,
    forced: Option<Triple>,
    focus: Option<MatchableField>,
) -> RuleTree {
    let mut generator = Generator::new(rng, focus);
    generator.box_at(levels, forced)
}

/// Every (field, comparison, polarity) the catalog permits. This is the
/// coverage floor: the run does not *hope* to reach all of it, it puts one of
/// each in a tree first and then spends its budget on interactions.
fn permitted_triples() -> Vec<Triple> {
    let mut triples = Vec::new();
    for field in MatchableField::ALL {
        for comparison in field_catalog::spec(field).comparisons() {
            for negated in [false, true] {
                triples.push((field, *comparison, negated));
            }
        }
    }
    assert!(
        triples.len() >= 90,
        "sixteen fields across the comparisons each permits, both polarities, is a floor of \
         ninety triples; {} means the catalog itself shrank",
        triples.len()
    );
    triples
}

fn field_index(field: MatchableField) -> usize {
    MatchableField::ALL
        .iter()
        .position(|candidate| *candidate == field)
        .expect("MatchableField::ALL is the enumeration of the sixteen")
}

fn comparison_index(comparison: Comparison) -> usize {
    Comparison::ALL
        .iter()
        .position(|candidate| *candidate == comparison)
        .expect("Comparison::ALL is the enumeration of the nine")
}

/// What one run actually touched. Reported in every coverage assertion, because
/// a property test that silently generated a narrower corpus than it claims is
/// the failure mode this file exists to avoid.
#[derive(Default, Debug)]
struct Coverage {
    trees: usize,
    rules: usize,
    boxes: usize,
    negated: usize,
    positive: usize,
    all_boxes: usize,
    any_boxes: usize,
    triples: BTreeSet<(usize, usize, bool)>,
    depths: BTreeSet<usize>,
    /// Trees holding two arms on one field inside one Box.
    same_field_pairs: usize,
    /// …of which at least one pairs a value arm with an absence arm on that
    /// field — the shape the two-valued leaf rule earns its keep on.
    mixed_absent_and_present: usize,
    most_rules_in_one_tree: usize,
}

/// Walk a generated tree once, counting what it covers.
#[allow(
    clippy::too_many_lines,
    reason = "one walk that counts every coverage claim this suite asserts"
)]
fn survey(tree: &RuleTree, coverage: &mut Coverage) {
    match tree {
        RuleTree::Rule {
            field_id,
            operator_id,
            negated,
            ..
        } => {
            let field = MatchableField::from_stored_id(field_id)
                .expect("a generated tree names a field this build knows");
            let comparison = Comparison::from_stored_id(operator_id)
                .expect("a generated tree names a comparison this build knows");
            coverage
                .triples
                .insert((field_index(field), comparison_index(comparison), *negated));
            coverage.rules += 1;
            if *negated {
                coverage.negated += 1;
            } else {
                coverage.positive += 1;
            }
        }
        RuleTree::Box {
            combine, children, ..
        } => {
            let combine = Combine::from_stored_id(combine)
                .expect("a generated tree combines with a word this build knows");
            coverage.boxes += 1;
            match combine {
                Combine::All => coverage.all_boxes += 1,
                Combine::Any => coverage.any_boxes += 1,
            }
            let mut fields = children
                .iter()
                .filter_map(|child| match child {
                    RuleTree::Rule { field_id, .. } => Some(field_id.clone()),
                    RuleTree::Box { .. } => None,
                })
                .collect::<Vec<_>>();
            fields.sort();
            let paired = fields.len() - fields.iter().collect::<BTreeSet<_>>().len();
            coverage.same_field_pairs += paired;
            if mixes_absent_and_present_with_value(children) {
                coverage.mixed_absent_and_present += 1;
            }
            for child in children {
                survey(child, coverage);
            }
        }
    }
}

/// Whether one Box's arms hold the same field twice, once with a value and once
/// with none — `genre is "ambient"` beside `genre is missing`, say. Across a
/// `any` Box that pair is where a three-valued SQL `NOT` and a two-valued Rust
/// walk would part company, and it is the shape the ticket names as the point.
fn mixes_absent_and_present_with_value(children: &[RuleTree]) -> bool {
    let mut valued = BTreeSet::new();
    let mut valueless = BTreeSet::new();
    for child in children {
        if let RuleTree::Rule {
            field_id, value, ..
        } = child
        {
            if value.is_some() {
                valued.insert(field_id.clone());
            } else {
                valueless.insert(field_id.clone());
            }
        }
    }
    valued.iter().any(|field| valueless.contains(field))
}

/// Box levels on the tree's deepest path.
fn box_depth(tree: &RuleTree) -> usize {
    match tree {
        RuleTree::Rule { .. } => 0,
        RuleTree::Box { children, .. } => 1 + children.iter().map(box_depth).max().unwrap_or(0),
    }
}

/// How many Rules a tree holds — reported so the budget's effect is visible.
fn rule_count(tree: &RuleTree) -> usize {
    match tree {
        RuleTree::Rule { .. } => 1,
        RuleTree::Box { children, .. } => children.iter().map(rule_count).sum(),
    }
}

/// Print a tree as the builder calls that built it, so a counterexample pastes
/// into a fixed case in this module next to its seed. An id this build cannot
/// read — which only a deliberately corrupted tree carries — is printed as the
/// `RuleTree` literal instead, which is equally pasteable.
fn render(tree: &RuleTree) -> String {
    match tree {
        RuleTree::Rule {
            field_id,
            operator_id,
            negated,
            value,
        } => {
            let value = render_value(value.as_ref());
            match (
                MatchableField::from_stored_id(field_id),
                Comparison::from_stored_id(operator_id),
            ) {
                (Some(field), Some(comparison)) => {
                    let builder = if *negated { "rule_not" } else { "rule" };
                    format!(
                        "{builder}(MatchableField::{field:?}, Comparison::{comparison:?}, {value})"
                    )
                }
                _ => format!(
                    "RuleTree::Rule {{ field_id: {field_id:?}.to_string(), operator_id: \
                     {operator_id:?}.to_string(), negated: {negated}, value: {value} }}"
                ),
            }
        }
        RuleTree::Box { combine, children } => {
            let inner = children.iter().map(render).collect::<Vec<_>>().join(", ");
            match Combine::from_stored_id(combine) {
                Some(Combine::All) => format!("all_of(vec![{inner}])"),
                Some(Combine::Any) => format!("any_of(vec![{inner}])"),
                None => format!(
                    "RuleTree::Box {{ combine: {combine:?}.to_string(), children: vec![{inner}] }}"
                ),
            }
        }
    }
}

/// A value in the same pastable spelling the generator drew it with.
fn render_value(value: Option<&RuleValue>) -> String {
    match value {
        None => "None".to_string(),
        Some(RuleValue::Text(text)) => format!("needle({text:?})"),
        Some(RuleValue::Number(number)) => format!("number({number})"),
        Some(RuleValue::DurationMillis(millis)) => format!("millis({millis})"),
        Some(RuleValue::Days(days)) => format!("days({days})"),
        Some(RuleValue::DateTime(instant)) => format!(
            "boundary(civil({}, {}, {}, {}, {}, {}))",
            instant.year, instant.month, instant.day, instant.hour, instant.minute, instant.second
        ),
        Some(RuleValue::YesNo(answer)) => format!("yes({answer})"),
    }
}

/// The label every generated assertion carries: which run, which tree, what
/// seed, what shape — and the tree itself in pastable form.
fn label(run: &str, seed: u64, index: usize, tree: &RuleTree) -> String {
    format!(
        "{run} tree #{index} of seed {seed:#018x} at depth {} with {} Rules: {tree}",
        box_depth(tree),
        rule_count(tree),
        tree = render(tree)
    )
}

/// One generated tree, asserted against the reference and counted into the
/// run's coverage. Every assertion names the tree, so a red run is a
/// reproducible case rather than a mystery.
fn check_generated(
    store: &SqliteStore,
    tracks: &[Track],
    clock: &rules_reference::Clock,
    tree: &RuleTree,
    text: &str,
    coverage: &mut Coverage,
) {
    coverage.trees += 1;
    coverage.depths.insert(box_depth(tree));
    coverage.most_rules_in_one_tree = coverage.most_rules_in_one_tree.max(rule_count(tree));
    survey(tree, coverage);
    assert!(
        rules_reference::can_read(tree, clock),
        "{text}: a generated tree is built from the catalog's own comparisons, so the \
         independent walk refusing it means the generator and the catalog disagree"
    );
    assert_rule_membership(store, tracks, tree, None, clock, text);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the run and its coverage assertions are one story"
)]
fn test_every_generated_rule_tree_answers_the_reference_track_for_track() {
    let dir = tempfile::tempdir().expect("temp dir for the generated-tree run");
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);
    let mut rng = Rng::seeded(GENERATOR_SEED);
    let mut coverage = Coverage::default();

    // The coverage floor, first: one tree carrying each permitted
    // (field, comparison, polarity) as one of its arms, its depth walking the
    // editor's whole ceiling. Then the interaction budget.
    let triples = permitted_triples();
    for (index, triple) in triples.iter().enumerate() {
        let levels = index % EDITOR_DEPTH_CEILING + 1;
        let tree = generate(&mut rng, levels, Some(*triple), Some(triple.0));
        assert_eq!(
            box_depth(&tree),
            levels,
            "the generator was asked for {levels} Box levels and produced {tree:?}; the depth \
             coverage below would be a claim about nothing",
            tree = render(&tree)
        );
        let text = label("coverage-floor", GENERATOR_SEED, index, &tree);
        check_generated(&store, &tracks, &clock, &tree, &text, &mut coverage);
    }

    for index in 0..GENERATED_TREES {
        let levels = index % EDITOR_DEPTH_CEILING + 1;
        let focus = Some(*rng.pick(&MatchableField::ALL));
        let tree = generate(&mut rng, levels, None, focus);
        let text = label("generated", GENERATOR_SEED, index, &tree);
        check_generated(&store, &tracks, &clock, &tree, &text, &mut coverage);
    }
    // What the run actually touched. Each of these is the ticket's coverage
    // promise, asserted rather than assumed.
    for triple in &triples {
        let seen = coverage.triples.contains(&(
            field_index(triple.0),
            comparison_index(triple.1),
            triple.2,
        ));
        assert!(
            seen,
            "the catalog permits {:?} / {:?} negated={}, and no generated tree held it — the \
             coverage floor silently skipped a row",
            triple.0, triple.1, triple.2,
        );
    }
    for depth in 1..=EDITOR_DEPTH_CEILING {
        assert!(
            coverage.depths.contains(&depth),
            "depths from one Box to the editor's {EDITOR_DEPTH_CEILING}-level ceiling are the \
             ticket: no tree was generated {depth} deep (saw {:?})",
            coverage.depths,
        );
    }
    assert!(
        coverage.all_boxes > 20 && coverage.any_boxes > 20,
        "both combine modes must be routine, not incidental: {} all Boxes, {} any Boxes",
        coverage.all_boxes,
        coverage.any_boxes,
    );
    assert!(
        coverage.negated > 20 && coverage.positive > 20,
        "negation on and off: {} negated leaves, {} plain ones",
        coverage.negated,
        coverage.positive,
    );
    assert!(
        coverage.same_field_pairs > 20,
        "arms sharing one field inside one Box are what makes the interactions interesting; \
         the run saw {}",
        coverage.same_field_pairs,
    );
    assert!(
        coverage.mixed_absent_and_present > 20,
        "trees whose arms mix an absent and a present value on the same field are where the \
         two-valued-leaf rule earns its keep; the run saw {}/{} trees of that shape",
        coverage.mixed_absent_and_present,
        coverage.trees,
    );
    assert_eq!(
        coverage.trees,
        triples.len() + GENERATED_TREES,
        "every tree the run claimed to walk"
    );
    // The budget's real ceiling: `MAX_RULES_PER_TREE` arm Rules drawn while the
    // budget lasts, plus one forced Rule per Box level for the Boxes the budget
    // ran out on — a Box is never allowed to hold nothing.
    assert!(
        coverage.most_rules_in_one_tree <= MAX_RULES_PER_TREE + EDITOR_DEPTH_CEILING,
        "the rule budget keeps a generated statement small: {} Rules in the widest tree, over \
         a budget of {MAX_RULES_PER_TREE} plus one forced Rule per Box level",
        coverage.most_rules_in_one_tree,
    );

    // The empty needle, pinned by name rather than left to the pool. `contains
    // ''` asks whether the empty string is inside the folded text, and both
    // sides answer *every* row — `instr(X, '')` is 1 in this engine, and Rust's
    // `str::contains("")` is true — which is the agreement worth having on the
    // record, because it is the one text case where the two spellings could
    // have silently meant different things.
    let empty = rule(MatchableField::Title, Comparison::Contains, needle(""));
    assert_rule_membership(&store, &tracks, &empty, None, &clock, "title contains ''");
    assert_eq!(
        rule_total(&store, &empty, None),
        tracks.len(),
        "an empty containment needle matches every row of the corpus on both sides, not none \
         of them — the reading a `WHERE instr(…) > 0` written in haste would give"
    );
}

/// The four shared sort directions the paged contract is pinned under — the
/// enum's own variant list, not a restatement of any catalog fact.
const PAGED_ORDERS: [TrackListOrder; 4] = [
    TrackListOrder::PathAsc,
    TrackListOrder::PathDesc,
    TrackListOrder::TitleAsc,
    TrackListOrder::TitleDesc,
];
/// Needles for the half of the contract that `AND`s a search into the tree: one
/// that hits titles, one that hits artists, and one that is a `LIKE` wildcard
/// character and must be read as a literal. No empty search needle: an empty box
/// means *no needle*, which is the `None` arm every read above already walks.
const PAGED_NEEDLES: [&str; 3] = ["alpha", "known", "%"];
/// The window size the paging run cuts at, and the offset step matching it.
const PAGED_WINDOW: usize = 3;

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the window, the total, the four orders and the needle are one contract"
)]
fn test_generated_trees_page_one_ordered_answer_under_every_sort_and_needle() {
    let dir = tempfile::tempdir().expect("temp dir for the generated paging run");
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);
    let mut rng = Rng::seeded(PAGED_SEED);

    for index in 0..PAGED_TREES {
        let levels = index % EDITOR_DEPTH_CEILING + 1;
        let focus = Some(*rng.pick(&MatchableField::ALL));
        let tree = generate(&mut rng, levels, None, focus);
        let text = label("paged", PAGED_SEED, index, &tree);
        assert!(
            rules_reference::can_read(&tree, &clock),
            "{text}: the generator built this tree from the catalog's own comparisons"
        );
        let reference = rules_reference::matches(&tracks, &tree, None, &clock);
        // The four directions' whole answers, kept so the direction itself can
        // be asserted below: the tiling further down is only self-consistent,
        // and a read that ignored `order` would tile just as consistently.
        let mut wholes: Vec<(TrackListOrder, Vec<TrackId>)> =
            Vec::with_capacity(PAGED_ORDERS.len());

        for order in PAGED_ORDERS {
            let (whole, whole_total) = page_of(&store, &tree, order, None, 0, usize::MAX, &text);
            assert_eq!(
                whole_total,
                reference.len(),
                "{text}: the total under {order:?} must be the reference's count"
            );
            wholes.push((order, whole.clone()));
            let mut canon = whole.clone();
            canon.sort_by(|left, right| left.0.cmp(&right.0));
            assert_eq!(
                canon,
                reference,
                "{text}: under {order:?} the same rows must be the answer the reference gives, \
                 and the names of the rows that disagreed are: {}",
                difference_names(&canon, &reference),
            );

            // Windows tile the one ordering: each is the contiguous slice of it,
            // each reports the same total, and together they are the whole
            // answer with no row twice and no row missing.
            let mut tiled = Vec::new();
            let mut offset = 0usize;
            while offset < whole.len() {
                let (window, window_total) =
                    page_of(&store, &tree, order, None, offset, PAGED_WINDOW, &text);
                assert_eq!(
                    window_total, whole_total,
                    "{text}: the total does not move with the offset ({order:?}, {offset})"
                );
                assert_eq!(
                    window,
                    slice_of(&whole, offset, PAGED_WINDOW),
                    "{text}: the window at offset {offset} is not the contiguous slice of the \
                     same tree's ordering under {order:?}"
                );
                assert!(
                    window.len() <= PAGED_WINDOW,
                    "{text}: a window of {PAGED_WINDOW} came back longer ({order:?}, {offset})"
                );
                tiled.extend(window);
                offset += PAGED_WINDOW;
            }
            assert_eq!(
                tiled, whole,
                "{text}: the windows of one answer must be disjoint and complete ({order:?})"
            );
        }

        // The window respects the shared sort directions, asked of the read's
        // own four answers: `path` is unique and the title order flips its
        // `path` tiebreak together with its key, so each descending answer is
        // the exact reversal of its ascending twin. This is what an
        // order-blind read fails — the tiling above and the set comparison
        // before it would both stay green if `order` never reached the
        // `ORDER BY`.
        assert_eq!(
            ids_in(&wholes, TrackListOrder::PathDesc),
            reversed(ids_in(&wholes, TrackListOrder::PathAsc)),
            "{text}: path-descending must be the exact reversal of path-ascending"
        );
        assert_eq!(
            ids_in(&wholes, TrackListOrder::TitleDesc),
            reversed(ids_in(&wholes, TrackListOrder::TitleAsc)),
            "{text}: title Z-A must be the exact reversal of title A-Z"
        );

        // The needle is ANDed with the tree inside the store read — never chosen
        // against it, and never applied after the window is cut.
        for needle_text in PAGED_NEEDLES {
            let expected = rules_reference::matches(&tracks, &tree, Some(needle_text), &clock);
            let (rows, total) = page_of(
                &store,
                &tree,
                TrackListOrder::PathAsc,
                Some(needle_text),
                0,
                usize::MAX,
                &text,
            );
            assert_eq!(
                rows, expected,
                "{text}: with the needle {needle_text:?} the answer must be the reference's \
                 tree-AND-needle set, and the store's total must agree with it"
            );
            assert_eq!(
                total,
                expected.len(),
                "{text}: needle {needle_text:?} total"
            );
            for offset in [0usize, 2] {
                let (window, window_total) = page_of(
                    &store,
                    &tree,
                    TrackListOrder::PathAsc,
                    Some(needle_text),
                    offset,
                    2,
                    &text,
                );
                assert_eq!(
                    window,
                    slice_of(&expected, offset, 2),
                    "{text}: a needle's window at offset {offset} is the filtered answer's own \
                     slice, not a page of the unfiltered one"
                );
                assert_eq!(
                    window_total,
                    expected.len(),
                    "{text}: needle {needle_text:?} total \
                     must hold at every offset"
                );
            }
        }
    }
}

/// The seam, unwrapped: one page of a generated tree, as `(rows, total)`.
fn page_of(
    store: &SqliteStore,
    tree: &RuleTree,
    order: TrackListOrder,
    needle_text: Option<&str>,
    offset: usize,
    limit: usize,
    text: &str,
) -> (Vec<TrackId>, usize) {
    match store.custom_smart_playlist_page(tree, order, needle_text, offset, limit) {
        Ok(page) => (
            page.rows().iter().map(|track| track.id.clone()).collect(),
            page.total(),
        ),
        Err(error) => panic!("{text}: a legal tree must be answerable, got {error}"),
    }
}

/// One order's whole answer out of a generated tree's four reads.
fn ids_in(wholes: &[(TrackListOrder, Vec<TrackId>)], order: TrackListOrder) -> &[TrackId] {
    &wholes
        .iter()
        .find(|(asked, _)| *asked == order)
        .expect("every order in PAGED_ORDERS is read once")
        .1
}

fn reversed(ids: &[TrackId]) -> Vec<TrackId> {
    ids.iter().rev().cloned().collect()
}

fn slice_of(ids: &[TrackId], offset: usize, limit: usize) -> Vec<TrackId> {
    let start = offset.min(ids.len());
    let end = start.saturating_add(limit).min(ids.len());
    ids[start..end].to_vec()
}

/// The names of the rows two answers disagree on, for a failure a person can
/// read without decoding fifteen `r:\rules\…` paths.
fn difference_names(left: &[TrackId], right: &[TrackId]) -> String {
    let only_left: Vec<&str> = left
        .iter()
        .filter(|id| !right.contains(id))
        .map(rule_name)
        .collect();
    let only_right: Vec<&str> = right
        .iter()
        .filter(|id| !left.contains(id))
        .map(rule_name)
        .collect();
    format!("store-only {only_left:?} / reference-only {only_right:?}")
}

/// A deliberate defect in a generated tree, in the shape a row from a newer
/// riff or a hand-edited store arrives as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Corruption {
    /// A `field_id` this build does not know.
    UnknownField,
    /// An `operator_id` this build does not know.
    UnknownOperator,
    /// A comparison the schema's union of ids allows and the field forbids.
    ComparisonTheFieldForbids,
    /// A value of the wrong kind for the field.
    ValueOfTheWrongKind,
    /// No value where the catalog draws one.
    NoValueWhereOneIsRequired,
    /// A combine word this build does not know.
    UnknownCombine,
    /// A Box holding no Rule at all.
    EmptyBox,
    /// A value sitting on a test that takes none — the one shape that must
    /// still be answered, because absence is the column's answer either way.
    SpareValueOnAValuelessTest,
}

impl Corruption {
    const ALL: [Self; 8] = [
        Self::UnknownField,
        Self::UnknownOperator,
        Self::ComparisonTheFieldForbids,
        Self::ValueOfTheWrongKind,
        Self::NoValueWhereOneIsRequired,
        Self::UnknownCombine,
        Self::EmptyBox,
        Self::SpareValueOnAValuelessTest,
    ];

    /// Whether the read must refuse. Only the spare value may be tolerated.
    const fn refuses(self) -> bool {
        !matches!(self, Self::SpareValueOnAValuelessTest)
    }

    /// The offender's own name, which the refusal reason has to carry — the one
    /// thing that makes "this list cannot be computed" tellable apart from
    /// "this list matched nothing".
    fn hint(self) -> &'static str {
        match self {
            Self::UnknownField => "bpm",
            Self::UnknownOperator => "starts_with",
            Self::ComparisonTheFieldForbids => "does not permit",
            Self::ValueOfTheWrongKind => "wrong kind",
            Self::NoValueWhereOneIsRequired => "holds no value",
            Self::UnknownCombine => "none",
            Self::EmptyBox => "holds no Rule",
            Self::SpareValueOnAValuelessTest => "unused",
        }
    }
}

/// Apply one corruption at the first node it fits, so the rest of the tree
/// stays the readable tree it was generated as and the refusal is attributable.
fn corrupt(tree: &RuleTree, kind: Corruption) -> Option<RuleTree> {
    let mut applied = false;
    let rebuilt = rebuild(tree, kind, &mut applied);
    applied.then_some(rebuilt)
}

fn rebuild(tree: &RuleTree, kind: Corruption, applied: &mut bool) -> RuleTree {
    if *applied {
        return tree.clone();
    }
    match tree {
        RuleTree::Rule { .. } => {
            let mutated = mutate_leaf(tree, kind);
            if mutated.is_some() {
                *applied = true;
            }
            mutated.unwrap_or_else(|| tree.clone())
        }
        RuleTree::Box { combine, children } => {
            if matches!(kind, Corruption::UnknownCombine | Corruption::EmptyBox) {
                *applied = true;
                return RuleTree::Box {
                    combine: if kind == Corruption::UnknownCombine {
                        "none".to_string()
                    } else {
                        combine.clone()
                    },
                    children: if kind == Corruption::EmptyBox {
                        Vec::new()
                    } else {
                        children.clone()
                    },
                };
            }
            RuleTree::Box {
                combine: combine.clone(),
                children: children
                    .iter()
                    .map(|child| rebuild(child, kind, applied))
                    .collect(),
            }
        }
    }
}

/// The leaf corruptions, each guarded by the catalog question that makes it a
/// refusal rather than an answer.
fn mutate_leaf(tree: &RuleTree, kind: Corruption) -> Option<RuleTree> {
    let RuleTree::Rule {
        field_id,
        operator_id,
        negated,
        value,
    } = tree
    else {
        return None;
    };
    let field = MatchableField::from_stored_id(field_id)?;
    let comparison = Comparison::from_stored_id(operator_id)?;
    let row = field_catalog::spec(field);
    let draws_a_value = !row.value_widgets(comparison).is_empty();
    let leaf = |operator_id: &str, value: Option<RuleValue>| RuleTree::Rule {
        field_id: field_id.clone(),
        operator_id: operator_id.to_string(),
        negated: *negated,
        value,
    };
    match kind {
        Corruption::UnknownField => Some(RuleTree::Rule {
            field_id: "bpm".to_string(),
            operator_id: operator_id.clone(),
            negated: *negated,
            value: value.clone(),
        }),
        Corruption::UnknownOperator => Some(leaf("starts_with", value.clone())),
        Corruption::ComparisonTheFieldForbids => {
            if row.permits(Comparison::GreaterThan) {
                return None;
            }
            Some(leaf("greater_than", value.clone()))
        }
        // Only where a value is drawn (otherwise the value is ignored on both
        // sides, which is the tolerance case) and only where the field is not
        // already a number (otherwise the value would be the right kind).
        Corruption::ValueOfTheWrongKind => {
            if !draws_a_value || row.value_type() == ValueType::Number {
                return None;
            }
            Some(leaf(operator_id, Some(RuleValue::Number(7))))
        }
        Corruption::NoValueWhereOneIsRequired => {
            if !draws_a_value {
                return None;
            }
            Some(leaf(operator_id, None))
        }
        Corruption::SpareValueOnAValuelessTest => {
            if draws_a_value {
                return None;
            }
            Some(leaf(operator_id, needle("ignored")))
        }
        Corruption::UnknownCombine | Corruption::EmptyBox => None,
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one assertion per corruption shape, generated rather than hand-built"
)]
fn test_a_generated_tree_that_cannot_be_computed_reports_a_reason_not_an_empty_window() {
    let dir = tempfile::tempdir().expect("temp dir for the generated refusal run");
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);
    let mut rng = Rng::seeded(CORRUPT_SEED);
    let mut applied: BTreeSet<Corruption> = BTreeSet::new();
    let mut tolerated = 0usize;
    let mut rounds = 0usize;

    // Some shapes need a leaf of a particular kind, so a round that did not fit
    // is skipped rather than forced; the run asserts at the end that all eight
    // landed, which is the difference between coverage and a wish.
    while applied.len() < Corruption::ALL.len() && rounds < 2_000 {
        rounds += 1;
        let kind = Corruption::ALL[rounds % Corruption::ALL.len()];
        let levels = rounds % 3 + 1;
        let focus = Some(*rng.pick(&MatchableField::ALL));
        let tree = generate(&mut rng, levels, None, focus);
        let Some(broken) = corrupt(&tree, kind) else {
            continue;
        };
        applied.insert(kind);
        let text = label("corrupted", CORRUPT_SEED, rounds, &broken);
        let answer =
            store.custom_smart_playlist_page(&broken, TrackListOrder::PathAsc, None, 0, 50);
        if kind.refuses() {
            let error = match answer {
                Ok(page) => panic!(
                    "{text}: a tree this build cannot compute must refuse with a reason, not \
                     answer {} rows of total {}",
                    page.rows().len(),
                    page.total()
                ),
                Err(error) => error,
            };
            let reason = error.to_string();
            assert!(
                reason.contains("cannot match this Custom Smart Playlist"),
                "{text}: the reason must name the surface that failed, so a listing that \
                 cannot be computed is distinguishable from one that matched nothing: {reason}"
            );
            assert!(
                reason.contains(kind.hint()),
                "{text}: the reason must carry the offender — looked for {:?} and got: {reason}",
                kind.hint(),
            );
            assert!(
                !rules_reference::can_read(&broken, &clock),
                "{text}: the independent walk refuses the same tree, so the store is not \
                 refusing alone by accident"
            );
        } else {
            assert!(
                rules_reference::can_read(&broken, &clock),
                "{text}: a value on a test that takes none is the save path's mistake, not a \
                 reason to stop matching"
            );
            assert_rule_membership(&store, &tracks, &broken, None, &clock, &text);
            assert_eq!(
                rule_window(&store, &broken, None),
                rule_window(&store, &tree, None),
                "{text}: the spare value changes nothing beside the tree it was added to"
            );
            assert_eq!(
                rule_total(&store, &broken, None),
                rule_total(&store, &tree, None),
                "{text}: and nothing to its total"
            );
            tolerated += 1;
        }
    }

    assert_eq!(
        applied.len(),
        Corruption::ALL.len(),
        "every unreadable shape must have been generated and asserted at least once; \
         the run reached {rounds} rounds and covered {applied:?}",
    );
    assert!(
        tolerated > 0,
        "the tolerance case is part of this contract too: {tolerated} trees had a spare value \
         on a valueless test"
    );
}

/// The deepest single-arm Box chain the bundled parser still prepares, for the
/// one leaf shape below. Measured, not published: `SQLite` keeps its expression
/// depth as a parser property rather than a stable number, and this compiler
/// spends part of that budget at every Box level (`coalesce((…), 0)`), so the
/// boundary that remains is a fact about the fold.
///
/// Measured on 2026-10-16 against the `rusqlite` / `SQLite` this `Cargo.lock`
/// pins: 621 levels prepare and 622 do not. Two ways this constant moves, and
/// each is worth knowing about: a compiler change that adds or removes a fold
/// per Box level moves it — which is the pin, and what ticket 10's mutation run
/// checked — and an engine bump moves it too, in which case re-measure and
/// update the number rather than loosening the assertion.
///
/// The leaf is fixed rather than generated because the boundary is a budget the
/// leaf and the chain spend *together*: a genre Rule owns a deeper expression
/// than a year Rule, so a generated leaf would make this constant a statement
/// about that leaf. The generated-leaf half of the same claim — the fold changes
/// no answer, at any depth — is asserted over generated leaves below.
const PINNED_DEEPEST_ANSWERABLE_BOX_CHAIN: usize = 621;

/// How many Box levels a generated leaf is chained at for the inertness check:
/// well inside the measured ceiling for every leaf shape this generator can
/// draw, including the ones whose own expression is a few levels deep.
const GENERATED_CHAIN_LEVELS: usize = 300;

/// How many generated leaves take part in it.
const GENERATED_CHAINS: usize = 12;

/// Wrap a tree in `levels` single-arm Boxes, alternating the combine word so
/// the chain exercises both fold arms rather than one.
fn chain(leaf: &RuleTree, levels: usize) -> RuleTree {
    let mut tree = leaf.clone();
    for level in 0..levels {
        tree = if level % 2 == 0 {
            all_of(vec![tree])
        } else {
            any_of(vec![tree])
        };
    }
    tree
}

#[test]
fn test_the_box_level_fold_is_pinned_by_the_engine_depth_it_spends() {
    let dir = tempfile::tempdir().expect("temp dir for the box-depth run");
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);
    let mut rng = Rng::seeded(BOX_FOLD_SEED);
    let mut generator = Generator::new(&mut rng, None);

    // Half one, over generated content: wrapping one two-valued Rule in three
    // hundred Boxes answers exactly that Rule's own rows, whatever the Rule is.
    // This is the box fold's inertness *stated as a measurement* — the finding
    // ticket 09 reached by mutation, reconfirmed here over generated leaves
    // rather than assumed. No case in this file claims the fold changes an
    // answer, because on this corpus it does not.
    for index in 0..GENERATED_CHAINS {
        let field = *generator.rng.pick(&MatchableField::ALL);
        let comparison = *generator.rng.pick(field_catalog::spec(field).comparisons());
        let negated = generator.rng.chance(50);
        let leaf = generator.leaf(Some((field, comparison, negated)));
        let deep = chain(&leaf, GENERATED_CHAIN_LEVELS);
        let text = format!(
            "box-fold tree #{index} of seed {BOX_FOLD_SEED:#018x}: {} wrapped in \
             {GENERATED_CHAIN_LEVELS} Box levels",
            render(&leaf)
        );
        assert!(
            rules_reference::can_read(&deep, &clock),
            "{text}: the walk folds a Box of two-valued arms with Rust's own `all` / `any` over \
             `bool`, so there is no third answer for it to lose"
        );
        assert_rule_membership(&store, &tracks, &deep, None, &clock, &text);
        assert_eq!(
            rule_window(&store, &deep, None),
            rule_window(&store, &leaf, None),
            "{text}: a Box around a two-valued test is that same test; the levels spend depth \
             and nothing else"
        );
    }

    // Half two is the pin: the exact depth boundary, for one fixed leaf shape,
    // because the leaf and the chain spend the parser's budget *jointly* (see
    // the constant's doc). The fold wraps every Box level in
    // `coalesce((…), 0)`, and that wrapper is what reaches the limit, so a
    // compiler change to the fold moves this boundary and reddens one of the two
    // assertions below — which is what ticket 10's mutation run checked.
    let simple = rule(MatchableField::Year, Comparison::EqualTo, number(1975));
    let inside = chain(&simple, PINNED_DEEPEST_ANSWERABLE_BOX_CHAIN);
    assert_rule_membership(
        &store,
        &tracks,
        &inside,
        None,
        &clock,
        &format!(
            "{} Box levels around {} is still a tree, and the engine prepares it",
            PINNED_DEEPEST_ANSWERABLE_BOX_CHAIN,
            render(&simple)
        ),
    );

    // One level more and the bundled parser says no in its own words, which the
    // read carries out as a reason through the same path every refusal takes —
    // never as an empty window.
    let past = chain(&simple, PINNED_DEEPEST_ANSWERABLE_BOX_CHAIN + 1);
    let error = store
        .custom_smart_playlist_page(&past, TrackListOrder::PathAsc, None, 0, 50)
        .expect_err("one Box level past the measured parser ceiling is refused");
    let reason = error.to_string();
    assert!(
        reason.contains("failed to read a listing page") && reason.len() > 40,
        "the engine's own reason is carried out of the read, not swallowed: {reason}"
    );
}
