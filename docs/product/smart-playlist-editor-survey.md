# Smart Playlist Editors: A Survey of How Other Software Builds Them

A survey of rule-builder editors in music players and beyond — the editor UI, the data model behind
it, and the evaluation semantics — written to inform riff's Custom Smart Playlist editor
([ADR 0021](../adr/0021-smart-playlist-rules-are-collection-data.md),
[ADR 0020](../adr/0020-rule-matching-lowercases-at-query-time.md)).

**Status**: Survey (research document, not a decision record). "Recommendations" sections are
clearly labeled suggestions; everything else describes the surveyed software as verified against the
cited sources.

## 1. Method and scope

What was surveyed, in three groups:

1. **The canonical reference** — Apple's Music app (iTunes) smart playlist editor, including the
   best available documentation of its binary "Smart Criteria" serialization.
2. **Open-source players with a real editor** — Clementine and its fork Strawberry, Quod Libet,
   Rhythmbox, Banshee, Amarok, Cantata, plus the players documented from official vendor material
   (MusicBee, MediaMonkey) and expression-language players (foobar2000, beets). Kodi, Navidrome and
   a Jellyfin plugin round out the serialization-format spread. Source files were read directly
   (GitHub raw, GNOME GitLab raw) wherever the project publishes source; vendor features were read
   from official documentation.
3. **Rule builders outside music** — Notion database filters, Linear's filter builder, GitHub issue
   filters, and Gmail's filter form, each cited from official docs.

Sourcing rules followed while writing: **primary sources for load-bearing claims** — official
documentation, source code, or first-party help; when the claim rests on source code the file is
named (e.g. `clementine-player/Clementine/src/smartplaylists/search.h`). **Community
reverse-engineering is marked as such** — Apple has never documented the "Smart Criteria" binary;
the format description below comes from the `itunessmart` parser, a Python implementation "based on
[the] banshee-itunes-import-plugin by Scott Peterson"
(<https://github.com/cvzi/itunes_smartplaylist>), whose `itunessmart/data_structure.py` is the best
available field-by-field map of the format: authoritative-but-community-sourced. **Community wikis
are marked** — the MusicBee and foobar2000 descriptions rest on their community wikis (via Wayback
Machine snapshots) plus vendor forum material. Every claim carries an inline citation; the
[sources index](#6-sources-index) lists everything consulted.

## 2. Where riff stands today

Every later section compares against this. The facts, with file:line citations:

**The model** ([ADR 0021](../adr/0021-smart-playlist-rules-are-collection-data.md)): a Custom Smart
Playlist is collection data in its own typed tables, authored on a Settings page. The rule vocabulary
lives in `crates/riff-persistence/src/rule.rs`:

- One Rule is one `MatchableField` tested by one `Comparison` against one `RuleValue`, with a
  `negated` bit that inverts any test — `is not missing` is deliberately not a tenth stored operator
  (`rule.rs:131-152`, `rule.rs:359-376`). Sixteen fields (`rule.rs:23-105`), nine comparisons whose
  `is` and `equal_to` spellings collapse into one `Semantic` (`rule.rs:209-221`), values typed as
  Text / Number / DurationMillis / Days / DateTime / YesNo (`rule.rs:307-339`).
- Rules live in Rule Boxes; a box is one `Combine` (all/any) plus children in a single interleaved
  sibling order; a list has exactly one `RootBox` (`rule.rs:245-278`, `rule.rs:441-544`). Nesting is
  unbounded in the type; the seven-level cap is an editor-display rule only
  (`crates/riff-gui/src/ui/rule_box.rs:45`, `:100-103`).
- What a field permits is answered once by the Matchable Field catalog
  (`crates/riff-persistence/src/field_catalog.rs`): value type (`:31`), how absence is decided —
  empty text vs `IS NULL` (`:52`) — and which value widget *idea* the editor draws (`:72`).

**The store** (`crates/riff-infra/src/store/sqlite.rs:493-518`): three typed tables —
`smart_playlists` (id, name, created_at), `smart_rule_boxes` (parent link, combine CHECK,
position), `smart_rules` (box link, field/operator CHECK lists, negated, single `value` TEXT
column, position). No blobs; unknown future ids read back as Unreadable Rules rather than failing
([ADR 0021](../adr/0021-smart-playlist-rules-are-collection-data.md) consequences).

**The evaluation** ([ADR 0020](../adr/0020-rule-matching-lowercases-at-query-time.md)): rules
compile to SQL predicates at query time, folding case inside the predicate (`lower(column)` against
a Rust-lowered needle), with the documented ASCII-only limitation. No materialization: the UI reads
counts through Session Projections invalidated by the store's generation counter.

**The editor** (`crates/riff-gui/src/ui/smart_playlists_page.rs`): a two-column Settings page —
saved lists left, the draft right. The draft is frontend-local (`SmartPlaylistsEditor`, `:213`);
dirtiness is derived by comparing against the opened-from snapshot (`:305-307`); Save is one store
call writing name and whole tree (`prepare_save`, `:605-646`), refusing in a fixed order —
Unreadable Rule, half-filled row (named by what it is missing, `:699-730`), name problems, then the
store's own refusal. The live match count is worn inside the name field's well (`:1042-1068`) and
counts the **committed** answers, not keystroke state (`committed_tree`, `:529-543`). Rule rows
(`crates/riff-gui/src/ui/rule_row.rs`) are field → comparison → value driven entirely by the
catalog; the value picker reads Library values once per open, capped at 50 (`rule_row.rs:76`).
Nested boxes draw as cards ("Add rule" / "Add group", `smart_playlists_page.rs:1312-1362`); rows
drag within their sibling list only (`:997-1020`).

**What riff does not have today**: no limit ("N items/hours/MB"), no selected-by ordering, no sort
order, no live-updating toggle or static snapshot, and no predefined starter templates. Sections
[4](#4-cross-cutting-analysis) and [5](#5-recommendations-for-riff) return to each.

## 3. Per-implementation sections

### 3.1 iTunes / Music.app (Apple) — the canonical reference

**Editor UI.** Apple's guide describes the flow: "Choose File > New > Smart Playlist, then specify
the criteria" — "Click the first pop-up menu, choose an option (such as Genre, Favorite, or Year),
then complete the rule using the other available menus or fields"; "To set additional rules, click
the Add button. To remove a rule, click the Remove button." With two or more rules, a menu "lets you
choose whether an item must meet any or all of the criteria to be included" — the "Match [Music] for
[all/any] of the following rules" header. Nesting is an affordance on the same button: "To specify
another set of criteria within a rule, press the Option key, then click the Nest button" — the Add
button becomes Nest while Option is held, producing a parenthesized sub-group with its own all/any
selector. Limits are a checkbox ("Limit to") whose example is "2 hours, with songs that were most
recently added"; further checkboxes are "Match only checked items" and "Live updating", which "has
Music update the playlist as items in your library are added, removed, or changed". Editing goes
through the playlist's More button → "Edit Rules" ([Apple, "Create, edit, and delete Smart
Playlists in Music on Mac"](https://support.apple.com/guide/music/create-edit-and-delete-smart-playlists-mus1712973f4/mac)).

**Data model and serialization.** Rules serialize into two opaque binary blobs carried on the
playlist dict in the (legacy) `iTunes Music Library.xml` — `Smart Info` and `Smart Criteria`, each
base64-encoded binary data ([cvzi/itunes_smartplaylist
README](https://github.com/cvzi/itunes_smartplaylist); the criteria blob decodes to an `SLSt`
signature, visible in the README's sample). Apple documents neither. The community
reverse-engineering (community-sourced; `itunessmart/data_structure.py`, itself based on Scott
Peterson's banshee-itunes-import-plugin) gives the layout:

- **Smart Info** carries the playlist-level switches by absolute byte offset: `LIVEUPDATE` = 0,
  `MATCHBOOL` = 1, `LIMITBOOL` = 2, `LIMITMETHOD` = 3, `SELECTIONMETHOD` = 7, `LIMITINT` = 8,
  `LIMITCHECKED` = 12, and a `SELECTIONMETHODSIGN` byte = 13 that flips "most/least" for selection
  methods (`Offset` enum). Limit methods enumerate as Minutes/MB/Items/Hours/GB; selection methods
  as Random/Name/Album/Artist/Genre/HighestRating/LowestRating/RecentlyPlayed/OftenPlayed/
  RecentlyAdded (`LimitMethods`, `SelectionMethods` enums).
- **Smart Criteria** is a sequence of fixed-size records: top-level `LOGICTYPE` (all/any) at byte
  15; each criteria record starts at `FIELD` = 139, with `LOGICSIGN` (+1: positive or negative
  form), `LOGICRULE` (+4: is/contains/starts/ends/greater/less as **combinable bit flags**
  0x01/0x02/0x04/0x08/0x10/0x40), value payloads at `STRING` (+54), `INTA` (+57, `INTB` +24), and
  time windows at `TIMEVALUE`/`TIMEMULTIPLE`; a **sub-expression** is a record of length
  `SUBEXPRESSIONLENGTH` = 192 carrying its own `SUBLOGICTYPE` (+68) — how nesting works in the
  format. Fields are partitioned by value type: `StringFields` (Name, Artist, Album, AlbumArtist,
  Genre, Composer, Comments, Grouping, …), `IntFields` (Year, BPM, BitRate, SampleRate, Plays,
  Skips, Rating, Duration, TrackNumber, DiskNumber, Size, Compilation, …), `DateFields`
  (DateAdded, DateModified, LastPlayed, LastSkipped), `BooleanFields` (HasArtwork, Purchased,
  Checked), plus MediaKind / PlaylistPersistentID / iCloud / Love fields.

The parser's output shows the shape the bytes encode: a tree whose leaves are
`{field, type, operator, value}` records and whose inner nodes are literal `and`/`or` objects —
e.g. `(Plays > 15) AND ( (Plays > 16) OR (Plays > 17) OR (Plays > 18) ) AND (Rating > 4)`
([itunessmart README](https://github.com/cvzi/itunes_smartplaylist)).

**Evaluation semantics.** The library stores the resolved track list on the playlist dict
(`Playlist Items`), and the `liveupdate` flag the parser recovers from Smart Info is exposed as
first-class output alongside the tree (`"liveupdate": true, "onlychecked": false` — [itunessmart
README](https://github.com/cvzi/itunes_smartplaylist)): with live updating on, Music re-evaluates as
the library changes ([Apple's
doc](https://support.apple.com/guide/music/create-edit-and-delete-smart-playlists-mus1712973f4/mac));
with it off, the playlist keeps the items it matched when created.

**Sources**: [Apple support doc](https://support.apple.com/guide/music/create-edit-and-delete-smart-playlists-mus1712973f4/mac);
[cvzi/itunes_smartplaylist](https://github.com/cvzi/itunes_smartplaylist) (README +
`itunessmart/data_structure.py`) — community-sourced reverse engineering.

### 3.2 Clementine / Strawberry — the closest open-source analog

Strawberry forked Clementine's implementation and carries it forward under new names
(`strawberrymusicplayer/strawberry` `src/smartplaylists/`: `smartplaylistsearch.cpp/.h`,
`smartplaylistsearchterm.cpp/.h`, `smartplaylistquerywizardplugin.cpp`, live preview widget
`smartplaylistsearchpreview.cpp` — same model, per the directory listing fetched from the repo).
Clementine is treated as the reference here.

**Data model.** The whole model is two value types
(`clementine-player/Clementine/src/smartplaylists/search.h`, `searchterm.h`):

```cpp
class Search {                       // search.h:26-64
  enum SearchType { Type_And = 0, Type_Or, Type_All };   // one combinator for ALL terms
  enum SortType   { Sort_Random = 0, Sort_FieldAsc, Sort_FieldDesc };
  SearchType search_type_;
  TermList terms_;                   // QList<SearchTerm> — a FLAT list, no nesting
  SortType sort_type_; SearchTerm::Field sort_field_; int limit_;
  QString ToSql(const QString& songs_table) const;
};

class SearchTerm {                   // searchterm.h:26-130
  enum Field    { Field_Title, Field_Artist, Field_Album, ... FieldCount }; // 25 fields
  enum Operator { Op_Contains, Op_NotContains, Op_StartsWith, Op_EndsWith,
                  Op_GreaterThan, Op_LessThan, Op_Equals, Op_NotEquals,
                  Op_NumericDate, Op_RelativeDate, Op_NumericDateNot,
                  Op_Empty, Op_NotEmpty };                                  // 13 operators
  enum Type     { Type_Text, Type_Date, Type_Time, Type_Number, Type_Rating };
  enum DateType { Date_Hour, Date_Day, Date_Week, Date_Month, Date_Year };
  Field field_; Operator operator_; QVariant value_; DateType date_;
  QVariant second_value_;            // relative dates need a second parameter
};
```

The operator catalog is per value type — `OperatorsForType` gives text six word-operators plus
empty/not-empty, dates `after/before/on/not on/in the last/not in the last/between`, numbers
equals/not-equals/greater/less (`searchterm.cpp:219-231`); `TypeOf(field)` maps the 25 fields to the
five types (`searchterm.cpp:196-217`); value validation lives in `SearchTerm::is_valid`
(`searchterm.cpp:150-178`), including "the second value must be greater than the first" for
between-dates (`searchtermwidget.cpp:374-375`).

**Serialization.** `Search` and `SearchTerm` stream through `QDataStream` operators (the `<<`/`>>`
declarations in both headers); smart playlists persist in `Clementine.conf` under
`[SerialisedSmartPlaylists]` (per Clementine issue #5941, referenced by the project's own tracker).
The persisted enums carry an explicit "add to the end of the enum only" contract
(`search.h:30-31`, `searchterm.h:28-29`) — the same discipline as riff's CHECK-listed stable ids.

**Editor UI.** Clementine's editor is a **wizard** (`src/smartplaylists/wizard.cpp`,
`querywizardplugin.cpp`): a type page, then the search-terms page whose `.ui` file defines three
radio buttons — "Match every search term (AND)", "Match one or more search terms (OR)", "Include all
songs" (`querysearchpage.ui:41-51`) — a "new term" button that appends rows, and a live
`SearchPreview` widget "added at the bottom of the search terms page" showing matching songs
(`querywizardplugin.cpp:102-123`), refreshed on the sort page too (`:140-154`). Each term row
(`searchtermwidget.cpp`) is a field picker, an operator picker filtered by the field's type, and a
typed value editor. There is **no nesting**: the single `SearchType` applies to the whole flat list,
and terms always AND/OR together globally. The generated SQL confirms it: one boolean operator
joined over all term clauses (`search.cpp:47-77`).

**Evaluation semantics.** Everything is query-time SQL: `Search::ToSql` builds
`SELECT ... WHERE (<terms joined by AND/OR>) ... ORDER BY random()|column ... LIMIT n`
(`search.cpp:47-77`); dynamic generators reuse the same search with `id_not_in_` to append
not-yet-played tracks (`search.cpp:60-66`). Sort + limit are part of the model, not afterthoughts.

**Sources** (all fetched from the repository): `src/smartplaylists/search.h`, `searchterm.h`,
`search.cpp`, `searchterm.cpp`, `wizard.cpp`, `querywizardplugin.cpp`, `querysearchpage.ui`,
`searchtermwidget.cpp` — <https://github.com/clementine-player/Clementine>;
Strawberry listing — <https://github.com/strawberrymusicplayer/strawberry>.

### 3.3 Quod Libet — expression language, no editor

Quod Libet has no row-based smart-playlist editor at all: the whole product is driven by one query
language available in every search box. The [Searching Your Library
guide](https://quodlibet.readthedocs.io/en/latest/guide/searching.html) documents it:

- Combination and negation: `&(electro, !ambient)`, `| ( mangum , neutral milk )`, `!|(classical,
  smiths)`; arbitrary nesting via parentheses, e.g. `genre = &(|(Disco, Jazz), !Acid Jazz)`.
- Tag-scoped terms: `artist = delerium`, `album = "bargainville"` (quotes for exact match),
  `/regex/` forms, multiple tags at once `artist, performer = "Boa"c`.
- **Case handling is part of the syntax**: matching is case-insensitive by default, and "You can put
  a `c` after the last quote to make the search case-sensitive" — `artist = "BoA" c` vs
  `artist = "Boa" c`.
- Numeric terms in `#(...)`: comparisons `=`, `<`, `>`, `<=`, `>=`, `!=`, **chained comparisons**
  `#(10 <= track < 100)`, and semi-English units — `#(added < 1 day)`, `#(2:00 < length < 3:00)`,
  with months = 30 days and years = 365 days.
- Cross-object terms: the internal `~playlists` tag searches membership in ordinary playlists
  (`~playlists=chilled`, `&(#(rating>=0.75), ~playlists="")` for high-rated tracks in no playlist).

The "Search Library" view — renamed "Track List" in Quod Libet 4.4.0 (per the project
[changelog](https://quodlibet.readthedocs.io/en/quodlibet-4.6/changelog.html): "Renamed 'Search
Library' to 'Track List' browser") and implemented as
`quodlibet/browsers/tracks.py` (<https://github.com/quodlibet/quodlibet>) — is a saved search
presented as a track list, not a stored playlist object: searches "can enter search terms and save
them" ([guide](https://quodlibet.readthedocs.io/en/latest/guide/searching.html)). Evaluation is
always query-time. Quod Libet is the survey's purest expression-language datapoint, and its
opt-in case-sensitivity suffix is the inverse of riff's always-folded predicate
([ADR 0020](../adr/0020-rule-matching-lowercases-at-query-time.md)).

**Sources**: [Searching Your Library](https://quodlibet.readthedocs.io/en/latest/guide/searching.html);
[4.6 changelog](https://quodlibet.readthedocs.io/en/quodlibet-4.6/changelog.html);
[quodlibet/browsers/tracks.py](https://github.com/quodlibet/quodlibet).

### 3.4 Rhythmbox — flat rows, live query model

Rhythmbox's "automatic playlist" editor is `rb-query-creator.c`
(`GNOME/rhythmbox/widgets/rb-query-creator.c`), whose own comment states its scope: a flat row list
of criteria that "cannot [represent] nested criteria or represent full [queries]"
(`rb-query-creator.c:111-115`). The dialog:

- Criteria rows, each a property menu + criteria menu (per-type operator catalog:
  `create_criteria_option_menu`, `RBQueryCreatorCriteriaOption`) + a typed value widget built by
  the property type's `criteria_create_widget` (`rb-query-creator.c:55-62`, `:555-558`).
- A `disjunctionCheck` toggle ("match any") that switches the whole row set between AND and OR
  (`rb-query-creator.c:221`, `:560-605`).
- Limit controls: `limitCheck` + spin entry + `limitOption` combo mapping to
  `RHYTHMDB_QUERY_MODEL_LIMIT_COUNT / SIZE / GB / TIME` (`:222-224`, `:408-445`, `:620-650`).
- A sort menu and descending toggle (`:226-228`), plus a free-text search entry ANDed into the
  query.

`rb_query_creator_get_query` compiles the rows into a `GPtrArray` RhythmDB query, inserting
`RHYTHMDB_QUERY_DISJUNCTION` between rows when the disjunction toggle is set
(`rb-query-creator.c:560-605`). Auto playlists persist as **XML attributes** on the playlist node
in the playlists file — `RB_PLAYLIST_LIMIT_COUNT / LIMIT / LIMIT_SIZE / LIMIT_TIME`, sort key and
direction — read back by `rb_auto_playlist_source_new_from_xml`
(`GNOME/rhythmbox/sources/rb-auto-playlist-source.c:374-437`). Evaluation is live: the auto
playlist *is* a query model that re-runs over the library (searching "is implemented by appending
the query criteria generated from" the query model — `rb-auto-playlist-source.c:56-58`).

**Sources**: `widgets/rb-query-creator.c`, `sources/rb-auto-playlist-source.c` —
<https://gitlab.gnome.org/GNOME/rhythmbox>.

### 3.5 Banshee — a real tree, serialized as XML in SQLite

Banshee's smart playlist editor (`Archive/banshee`,
`src/Core/Banshee.ThickClient/Banshee.SmartPlaylist.Gui/Editor.cs`) is a dialog with a name entry,
a `BansheeQueryBox` builder (Hyena's term-row query box), and a notable extra: a "Predefined Smart
Playlists" expander listing templates that can be "Open[ed] in editor" or "Create[d] and save[d]"
directly (`Editor.cs:110-135`). Loading a playlist restores name, condition tree, limit
enabled/value, limit kind and query order (`Editor.cs:60-71`).

**The data model is a genuine combinator tree.** `SmartPlaylistSource` holds a Hyena `QueryNode`
`ConditionTree`; setting it derives both the SQL and the stored form at once: `condition.ToSql(...)`
for query-time SQL, `condition.ToXml(...)` for the stored form, plus a walk collecting the
`QueryTermNode` fields the tree names (`SmartPlaylistSource.cs:110-134`); the XML round-trips
through `XmlQueryParser.Parse` (`:147-153`). Inner AND/OR structure lives inside the `QueryNode`
tree; limits are three first-class properties — `QueryOrder`, `QueryLimit`,
`IntegerQueryValue limit_value` — compiled to `OrderSql`/`LimitSql` (`:68-70`, `:179-189`).

**Serialization is a single XML TEXT column in SQLite** — `CoreSmartPlaylists (Name, Condition,
OrderBy, LimitNumber, LimitCriterion, ...)` in the store schema
(`src/Core/Banshee.Services/Banshee.Database/BansheeDbFormatMigrator.cs:1161-1172`). This is the
shape ADR 0021 rejected for riff (an opaque tree cannot be migrated field-by-field); Banshee pays
for it with a parser round-trip on every load and no column-level knowledge of which fields a tree
names.

**Sources**: `Editor.cs`, `SmartPlaylistSource.cs`, `BansheeDbFormatMigrator.cs` —
<https://gitlab.gnome.org/Archive/banshee>.

### 3.6 Amarok — token canvas and a constraint tree

Amarok 2.x contributes two distinct models:

**Edit Filter Dialog** (`src/dialogs/EditFilterDialog.cpp/.h`) — a token canvas: search terms are
dragged in as icons, reordered by drag, with AND/OR tokens added, and per-term "Invert condition"
to exclude matches (`EditFilterDialog.h` slots `slotTokenSelected`, `slotInvert`,
`slotSeparatorChange`). The official handbook describes exactly this: "drag down the icons for the
search terms you want, and drag to change search-term order... You can also add OR and AND here...
Select a term and check Invert condition to exclude items matching this term... Choose AND or OR for
all terms in this search" ([Amarok handbook, `doc/index.docbook`,
"Edit Filter Dialog"](https://github.com/KDE/amarok/blob/master/doc/index.docbook)). Field coverage
is wide (Title through Playcount, Label, Last Modified).

**The Automatic Playlist Generator (APG)** is a full nested constraint tree with a solver — the
survey's most unusual evaluation model. The handbook: presets "based on constraints you set... based
on song data (artist, genre, year) or access time (last played)", importable/exportable as .xml
([handbook, "Automatic Playlist
Generator"](https://github.com/KDE/amarok/blob/master/doc/index.docbook)). The source is a
`ConstraintNode` / `ConstraintGroup` recursion (`src/playlistgenerator/ConstraintGroup.h`) whose
groups carry `enum MatchType { MatchAny, MatchAll }` (`:38`) — per group, like riff's `Combine` —
and serialize recursively as `<constrainttree>` / `<group matchtype="any|all">` / `<constraint>`
XML elements (`ConstraintGroup.cpp:31-68`). `ConstraintSolver.cpp` satisfies the tree by picking
candidates rather than filtering a query — constraints are goals to solve, not predicates to
evaluate.

**Sources**: `src/dialogs/EditFilterDialog.h`, `src/playlistgenerator/ConstraintGroup.h/.cpp`,
`doc/index.docbook` — <https://github.com/KDE/amarok>.

### 3.7 MusicBee — nested sub-rules, limit + selected-by

MusicBee's Auto Playlist editor is documented from its vendor forum and community wiki
(community-sourced, via Wayback snapshots). Rule rows pair a field — custom and virtual tags
included — with operators like "is not" or "contains" and a value, "[playing track]", or a regular
expression; multiple rules combine under a "match ANY or ALL" choice; **nesting is per-rule**:
sub-rules are added "via the [.] button" and "operate exactly like other rules, except that they are
applied as AND rules only for the main rule they are attached to". The limit section offers
minutes/hours, MB/GB, items (default 25), or albums; "selected by" offers random (default),
highest/lowest rating, most/least recently played, most/least often played, most/least recently
added. Contents refresh "each time you access it" by default ("automatically refresh the matching
tracks" ticked; otherwise a "Refresh Now" button), and exporting produces a static snapshot
([MusicBee Wiki "Playlists"](https://musicbee.fandom.com/wiki/Playlists), via
[Wayback](http://web.archive.org/web/20260714142759/https://musicbee.fandom.com/wiki/Playlists)).
A vendor-forum thread shows the editor's own wording — "Match any of the following rules", with
rules like "Grouping is Album" and "Playlist is not 'Out of Rotation'"
([getmusicbee.com forum, topic 34969](http://web.archive.org/web/20221007220912/https://getmusicbee.com/forum/index.php?topic=34969.0)).

MusicBee's per-rule attachment of sub-rules is a different nesting grammar from riff's free tree
(and from iTunes'): the tree is shaped rule-then-its-sub-rules rather than box-holds-children.

### 3.8 MediaMonkey — nested criteria, auto-refresh

MediaMonkey's AutoPlaylist editor, from the official wiki: criteria added with "Match +"; "you can
use most tags from your files as Criteria" and "you can nest Criteria using [Nested condition]".
The Match setting decides whether "All or Any of the Criteria should be matched" (illustrated with
Genre = Rock + Date = 2008: All yields only Rock from 2008; Any yields "Rock songs from 2007 and Pop
songs from 2008" too). "Limit to" caps by "files, megabytes or minutes", with "Selected by"
determining how the cap picks tracks — an "(auto-refresh) option" makes the playlist pick fresh
tracks (e.g. random) each time it is viewed. AutoPlaylists are "Criteria based" and "automatically
reflect those changes" when files are added, tagged, or removed; they live in the MediaMonkey
database ("Losing the database means the AutoPlaylists are lost too")
([MediaMonkey Wiki, "Using AutoPlaylists"](https://www.mediamonkey.com/wiki/WebHelp:Using_AutoPlaylists)).

### 3.9 foobar2000 — keyword query language

foobar2000 has no rule-row editor; queries are typed expressions used by Library Search and
autoplaylists. The reference (community-maintained Hydrogenaudio wiki, the de-facto documentation,
via Wayback) documents: keyword operators in caps — `IS`, `HAS` (text, case-insensitive), `EQUAL`,
`GREATER`, `LESS` (integers), `MISSING`/`PRESENT`, and time operators `BEFORE`, `AFTER`, `SINCE`,
`DURING`, `DURING LAST`; boolean `AND`/`OR`/`NOT` with parentheses — `( (artist IS blah) AND (title
HAS blah) ) OR (rating GREATER 3)`; fields referenced as names or title-formatting expressions
(`$info(codec)`); case-sensitive matching only via title formatting (`"$strcmp(%field%,string)"
EQUAL 1`); and trailing `SORT BY`/`SORT DESCENDING BY` clauses ([Foobar2000:Query syntax,
Hydrogenaudio wiki](https://web.archive.org/web/2024/https://wiki.hydrogenaud.io/index.php?title=Foobar2000:Query_syntax)).

### 3.10 beets — materialized playlists from query strings

beets' `smartplaylist` plugin is the survey's cleanest **materialization** model: it "generates m3u
playlists from beets queries whenever your library changes", configured in YAML:

```yaml
smartplaylist:
    relative_to: ~/Music
    playlist_dir: ~/.mpd/playlists
    forward_slash: no
    playlists:
        - name: all.m3u
          query: ''
        - name: beatles.m3u
          query: 'artist:Beatles'
```

With `auto: yes` (default) each playlist rebuilds "at the end of the session if an item or album it
matches changed in the library database"; `beet splupdate` forces a rebuild ([beets docs,
smartplaylist plugin](https://beets.readthedocs.io/en/stable/plugins/smartplaylist.html)). The
queries are beets' general query language
([query reference](https://beets.readthedocs.io/en/stable/reference/query.html)): bare keywords AND
implicitly, comma-space for OR, `field:value` scoping, `=` for case-sensitive exact vs `=~`
case-insensitive (substring by default), `field::regex` for regex, `..` numeric ranges
(`year:1990..1999`), relative date windows (`added:-6w..-4w`), and `^`/`-` negation. Sorting is
part of the playlist spec (`year+` in the query).

### 3.11 Cantata (MPD client) — rule rows written to a text file

Cantata's "smart rules" playlists (`cdrummond/cantata`, `playlists/rulesplaylists.h/.cpp`,
`playlists/smartplaylists.cpp`) are a flat rule list plus special-cased numeric fields. The model
(`rulesplaylists.h:58-78`): `typedef QMap<QString, QString> Rule` — one rule row, field to value —
held in `QList<Rule> rules` (**flat**, no nesting), alongside promoted first-class fields:
`ratingFrom`/`ratingTo` and `includeUnrated`, `minDuration`/`maxDuration`, `maxAge`, `numTracks`
(default 10), and `Order` (AlbumArtist through Genre, Rating, Title, Age, Random) with
`orderAscending`.

Serialization is a plain text file of `key:value` lines — `Rating:2-5`, `Duration:60-300`,
`MaxAge:30`, `Order:Random`, `OrderAscending:true`, then repeated `Rule` blocks each followed by
`field:value` lines (`rulesplaylists.cpp:205-241`; `constKeyValSep = ':'`, `constRangeSep = '-'` at
`:64`). Note the pattern: whatever does not fit the row model (ranges, age, order) is promoted to a
named top-level field rather than expressed as rules.

**Source**: <https://github.com/cdrummond/cantata> (`playlists/rulesplaylists.h/.cpp`,
`playlists/smartplaylists.cpp/.h`).

### 3.12 Kodi — the .xsp XML format

Kodi smart playlists are files first: an XML format saved under `userdata/playlists`
([Kodi Wiki, Smart playlists](http://web.archive.org/web/20260826203253/https://kodi.wiki/view/Smart_playlists);
[XSP Method](http://web.archive.org/web/2026/https://kodi.wiki/view/Smart_playlists/XSP_Method)):

```xml
<?xml version="1.0" encoding="UTF-8" standalone="yes" ?>
<smartplaylist type="movies">
<name>SciFi by Year</name>
<match>all</match>
<rule field="genre" operator="is">
<value>Science Fiction</value>
<value>Sci-Fi</value>
<value>SciFi</value>
</rule>
<group>years</group>
<limit>100</limit>
<order direction="descending">lastplayed</order>
</smartplaylist>
```

`<match>` is `all` or `one` (single-level combinator per group); rules carry `field`/`operator`
attributes with one or more `<value>` children — "matching is case-insensitive"; `<order
direction="descending">lastplayed</order>` and `<limit>` cap the result; a single `<rule>` may
contain nested rule elements for groupings (nested rules are documented on the Rules & Groupings
page the XSP page links to). The GUI editor generates exactly this format. Notably, iTunes-style
**arbitrary** nesting does not fit Kodi's format — the itunessmart converter simulates nested
iTunes rules "with sub-playlists" named with a hash prefix
([itunessmart README](https://github.com/cvzi/itunes_smartplaylist)).

### 3.13 Navidrome — JSON files, lazy refresh

Navidrome (music server) defines smart playlists as **JSON `.nsp` files** imported from library
folders, or created by client apps via API (stored in the database). The [official
docs](https://www.navidrome.org/docs/usage/features/smart-playlists) show the format:

```json
{
  "name": "Recently Played",
  "comment": "Recently played tracks",
  "all": [{ "inTheLast": { "lastPlayed": 30 } }],
  "sort": "lastPlayed",
  "order": "desc",
  "limit": 100
}
```

Top-level `all`/`any` are the group combinators; a condition is one operator object — `{"gt": {
"rating": 3 }}`, `{"is": { "loved": true }}`, `{"inPlaylist": { "id": ... }}` — with an operator
catalog including `is`, `isNot`, `gt`, `lt`, `contains`, `startsWith`, `inTheRange`, `before`,
`after`, `inTheLast`, `notInTheLast`, `inPlaylist`/`notInPlaylist`, `isMissing`, `isPresent`.
Grouping guidance: keep rules "under a single top-level group (all or any)"; nest a group to mix the
two. Evaluation is **lazy with a floor**: playlists "refresh when accessed", throttled by
`SmartPlaylistRefreshDelay` (default 5s), with per-playlist `refreshDelay` overrides ("a rolling
window measured from the last evaluation, not aligned to calendar days"). Sorting takes
comma-separated fields with `+`/`-` prefixes and `"sort": "random"`; `limit` caps counts and
`limitPercent` takes a percentage of matches.

### 3.14 Jellyfin (smart playlist plugin) — JSON DNF

The `ankenyr/jellyfin-smartplaylist-plugin` stores each playlist as JSON in Jellyfin's data
directory: `Name`, `FileName`, `User`, `ExpressionSets` ("lists of `Expressions`, each set OR'ed
together" — so a two-level disjunction-of-conjunctions), and `Order`. Each expression is
`MemberName` + `Operator` + `TargetValue`, with string operators "Equals, StartsWith, EndsWith,
Contains" and regex operators "MatchRegex, NotMatchRegex" (per the
[plugin README](https://github.com/ankenyr/jellyfin-smartplaylist-plugin)). Regenerated playlists
are real (materialized) playlists. A deliberately simpler grammar than riff's tree — DNF covers most
real rules at the cost of not expressing arbitrary OR-of-AND-of-OR shapes.

### 3.15 Rule builders outside music

Four builders whose patterns transfer to a rule-tree editor:

**Notion (database filters).** Filters are per-property conditions; combining logic is explicit:
"You can create more specific database views and combine AND and OR logic by using filter groups."
The docs quantify the nesting: "These can be nested up to three layers deep!" — and offer a
migration path from flat to grouped: "turn a simple filter into an advanced one" via the filter's
••• menu → "Add to advanced filter" ([Notion Help, "Database views, filters, sorts &
groups"](https://www.notion.com/help/views-filters-and-sorts)). riff's editor already exceeds the
depth here; the transferable ideas are the depth ceiling as a product decision, and the
flat-to-grouped conversion affordance.

**Linear (filter builder).** A view "can have one or more filters at the same time"; the advanced
builder supports "grouping conditions and combining them with AND/OR logic (including nested filter
groups)". Operators are typed per property — "is or is not", "is either of or is not" (multi-value),
"includes any, all, neither, either, or none" (labels), "before or after" (dates). Filtering is
keyboard-first (`F` opens the menu; type a value to match), and applied filters encode into the URL
so a view is shareable ([Linear Docs, "Filters"](https://linear.app/docs/filters)). The typed
operator set per property is the same answer as riff's catalog; the URL-encoding of view state is a
pattern riff could borrow for saved filter sharing if it ever wants one.

**GitHub (issues).** A qualifier grammar — `state:open is:issue label:"bug"`, negation with `-`
(`state:open is:issue -author:octocat`), explicit `AND`/`OR`, comma-as-OR for labels
(`label:"bug","wip"`), parenthesized groups "nested up to five levels", and a Filters dropdown for
the common cases; "Filtered views update the page URL automatically, so you can share the link"
([GitHub Docs, "Filtering and searching issues and pull
requests"](https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/filtering-and-searching-issues-and-pull-requests)).
The lesson for a row-based editor: the textual qualifier bar and the structured editor are
complementary entry points to the same model.

**Gmail (filter form).** The flow starts from the search box, not a settings page: "Show search
options" → "Enter your search criteria" (with Search to verify before committing) → "Create filter"
→ "Choose what you'd like the filter to do" → "Create filter"; filters can be exported/imported as
.xml from Settings ([Gmail Help, "Create rules to manage your email" —
support.google.com/mail/answer/6579](https://support.google.com/mail/answer/6579)). The
try-before-you-save preview is the same instinct as riff's live match count, applied at filter
creation time.

## 4. Cross-cutting analysis

### 4.1 Data model patterns

**Three grammar families**, across every implementation surveyed:

1. **Flat row list + one global combinator.** Clementine/Strawberry (`Search.search_type_` over a
   flat `TermList`, `search.h:26-56`), Rhythmbox (rows + `disjunctionCheck`,
   `rb-query-creator.c:111`), Cantata (`QList<Rule>`), MediaMonkey's top level. Cheap to build and
   store; the cost is no way to say "genre is rock AND (artist is X OR artist is Y)", and the
   persisted enum's comment — "These values are persisted, so add to the end of the enum only"
   (`search.h:30`) — is the tell that the shape froze early.
2. **Combinator tree (boxes with all/any).** iTunes (192-byte sub-expression records each carrying
   their own all/any byte), Banshee (Hyena `QueryNode`), Amarok APG (`ConstraintGroup` with
   `MatchAny/MatchAll`), Kodi (nested rule elements), Navidrome (nestable `all`/`any` groups),
   Notion (groups "nested up to three layers deep"), Linear (nested filter groups), Jellyfin
   (deliberately two-level DNF), and riff itself (`RuleBox`/`RootBox`, `rule.rs:441-544`). The
   consensus shape for AND-of-OR; every row-list implementation that survived user demand grew
   toward it or grew a workaround.
3. **Expression language.** Quod Libet, foobar2000, beets, GitHub: maximal power, minimal UI — and
   every one still ships *some* structured affordance (GitHub's Filters dropdown) because raw
   syntax does not serve discovery.

**Field/operator typing.** Every implementation partitions fields by value type and gives each type
an operator catalog: Clementine's `TypeOf` + `OperatorsForType` (`searchterm.cpp:196-231`), iTunes'
String/Int/Date/Boolean field classes with `LogicRule` bit flags
(`itunessmart/data_structure.py:190-207`), beets' per-modifier semantics (`=` vs `=~` vs `::`),
Linear's per-property operator table, Navidrome's operator catalog. riff's `field_catalog`
(ValueType + permitted comparisons + widget idea per field, `field_catalog.rs`) is exactly this
pattern; the survey found nothing it lacks at its current scope.

**Two encoding strategies for operators.** Some projects enumerate flat operator ids including the
negations (`Op_NotContains`, `Op_NumericDateNot` — `searchterm.h:60-87`; Navidrome's `isNot`,
`notInTheLast`); others keep an orthogonal negation bit or invert affordance (iTunes' `LogicSign`
positive/negative byte; Amarok's per-term "Invert condition"; riff's `negated` bit —
`rule.rs:359-376`). The orthogonal bit is the smaller surface: riff's doc comment already argues
"one meaning spelled two ways would make 'has this tree changed?' comparisons lie", and the survey
bears that out — every flat-negation implementation duplicates its whole operator list.

**Where special cases go.** Cantata promotes what rows cannot express (rating range, duration
range, max age) to named top-level entry fields rather than inventing range operators
(`rulesplaylists.h:62-77`); Clementine gives relative dates a second value parameter
(`searchterm.h:113-116`); beets expresses ranges in-band (`year:1990..1999`); iTunes carries time
windows in dedicated payload bytes (`TIMEMULTIPLE`, `TIMEVALUE`). riff's `InLast(Days)` plus
`Before/After(DateTime)` sits in the Clementine lineage: a dedicated operator per date shape, which
matches how every typed-catalog implementation does it.

**Serialization spread.**

| Format | Who | Example |
|---|---|---|
| Typed relational tables | riff (`smart_playlists`/`smart_rule_boxes`/`smart_rules` — `sqlite.rs:493-518`); Banshee's table for the *metadata* (`CoreSmartPlaylists.Name/OrderBy/LimitNumber/LimitCriterion`) | columns + CHECK lists |
| XML TEXT column inside a table | Banshee's `Condition` column (`BansheeDbFormatMigrator.cs:1161`) | `QueryNode.ToXml()` |
| XML files | Rhythmbox playlists.xml (`rb-auto-playlist-source.c:374-437`), Kodi `.xsp`, Amarok APG presets | `<rule field= operator=>` |
| Plain text | Cantata `.rules` files (`rulesplaylists.cpp:205-241`) | `Rating:2-5` |
| YAML/JSON | beets config, Navidrome `.nsp`, Jellyfin plugin JSON | `{"all": [...]}` |
| Opaque binary | iTunes Smart Info/Smart Criteria plists | base64 blobs |
| QDataStream config blobs | Clementine/Strawberry in `Clementine.conf` | binary stream of the structs |

The two extremes make the trade-off visible: iTunes' binary is opaque — nothing outside the app can
migrate or audit it — while Banshee's XML-in-a-column is inspectable but forces a parse round-trip
and hides field usage from SQL. riff's typed tables are the only surveyed scheme where a migration
can *query* which rules name a field ([ADR 0021](../adr/0021-smart-playlist-rules-are-collection-data.md)
rejected the serialized tree on exactly this ground), and the survey supports that decision.

### 4.2 Editor interaction patterns

**Row-based builder** (iTunes, Clementine/Strawberry, Rhythmbox, Banshee, MusicBee, MediaMonkey,
riff): the dominant paradigm, differing mainly in how much of the row the catalog drives and where
nesting attaches:

- *Nesting as a modal affordance on the add button*: iTunes — "press the Option key, then click the
  Nest button" (Apple doc). riff: an explicit "Add group" button (`smart_playlists_page.rs:1312`).
- *Nesting attached to a rule*: MusicBee's sub-rules "applied as AND rules only for the main rule
  they are attached to" (MusicBee wiki).
- *Nesting as free tree*: Banshee (tree model end to end), Amarok APG, riff.
- *No nesting at all*: Clementine/Strawberry, Rhythmbox, Cantata — all flat.

**Depth caps are product decisions, not technical ones**: Notion documents "up to three layers
deep"; riff displays a 7-level cap by hiding the control (`rule_box.rs:45`, `:100-103`); iTunes,
Banshee and Amarok APG appear practically unbounded. Hiding the control rather than refusing the
save (riff's choice) has no surveyed analog — every other capped implementation documents the
number — but it is strictly friendlier than an error.

**Token canvas** (Amarok EditFilterDialog): drag field tokens into a canvas, drag to reorder, AND/OR
tokens between them, per-term invert checkbox. The most spatial paradigm, the most expensive to
build, and the handbook still has to explain the combinator in prose. **Expression entry** (Quod
Libet, foobar2000, beets, GitHub) is its inverse: one text field, documentation as the editor —
GitHub pairs it with a Filters dropdown, the recurring compromise of a structured surface for the
common cases and the language for the rest.

**Preview / live count.** Clementine embeds a `SearchPreview` widget at the bottom of both the
search-terms and sort wizard pages, refreshed as inputs change (`querywizardplugin.cpp:115-154`) —
a full sample of matching tracks, not just a count. Gmail makes "verify it works by clicking Search
first" part of the flow. riff shows the count inside the name field's well
(`smart_playlists_page.rs:1042-1068`), computed from committed values only so it cannot "move per
keystroke" (`committed_tree`, `:506-543`). iTunes and MusicBee have no editor-time count at all —
their feedback is the running playlist itself. Every music implementation except Quod Libet stores
smart playlists as named first-class objects; the non-music builders split (GitHub: URL state;
Notion: view settings; Linear: saved views). riff's sidebar-row placement (ADR 0021) matches the
music-player consensus.

### 4.3 Evaluation semantics

**Query-time vs materialized** — the deepest split in the survey:

- *Query-time, always live*: riff (projections + generation counter; [ADR
  0020](../adr/0020-rule-matching-lowercases-at-query-time.md)), Rhythmbox (the auto playlist is a
  query model), MediaMonkey ("automatically reflect those changes"), Banshee (SQL derived from the
  tree at set-time, re-run per view), Clementine (`ToSql` on demand), Navidrome (refresh on access
  with a throttle floor). The materializers — beets (writes .m3u files when the library changes,
  `auto: yes` or `splupdate`) and the Jellyfin plugin (regenerates real playlists from JSON) —
  exist because a *consumer* needs a file (MPD, Jellyfin clients); players that evaluate in-process
  never needed it. riff is in the in-process camp; the interesting gap is freezability, not
  materialization.
- *Live by default, freezable*: iTunes ("Live updating" checkbox — off means the playlist keeps
  what it matched when created; Apple doc), MusicBee (refresh-on-access ticked by default; unticked
  → "Refresh Now"; export produces a static snapshot), Kodi (dynamic .xsp, save-as-static).

**Limit + selected-by** is the single most-consistent feature across implementations that riff
lacks: iTunes (limit methods Items/Minutes/MB/Hours/GB × selection methods random/name/album/
artist/genre/highest rating/least+most recently played/often played/recently added, plus a
most/least sign byte and an exclude-unchecked bit — `data_structure.py` `LimitMethods`,
`SelectionMethods`, `Offset` enums); Rhythmbox (COUNT/SIZE/GB/TIME + sort field + direction,
`rb-query-creator.c:408-445`); Clementine (sort random/field ± , `limit_`, `search.h:38-46`);
Banshee (`QueryOrder` + `QueryLimit` + `LimitValue`, `SmartPlaylistSource.cs:68-70`); MusicBee and
MediaMonkey (minutes/MB/items + selected-by with most/least axes); Kodi (`<limit>`, `<order
direction=>`); Navidrome (multi-key `sort`, `limit`, and a `limitPercent` variant); Cantata
(`numTracks` + `Order` + `orderAscending`). The iTunes most/least selection axis is the one place
where "selected by X" is a *signed* choice, which the sign byte (`SELECTIONMETHODSIGN`) encodes.
Where riff's model stores none of this, every surveyed player that grew past toy rules grew it.

**Case folding.** riff folds at query time, ASCII-only, and documents the non-Latin gap
([ADR 0020](../adr/0020-rule-matching-lowercases-at-query-time.md)). The survey's datapoints: Quod
Libet — case-insensitive by default with an opt-in `c` suffix for case-sensitive; foobar2000 —
`IS`/`HAS` case-insensitive, case-sensitive only via `$strcmp`; Kodi — "matching is
case-insensitive"; beets — substring matching case-insensitive by default with `=` for the
case-sensitive exact form; Clementine — folds in SQL (its schema stores lowercase helper columns
per its `searchterm.cpp` ToSql). **Every surveyed player defaults to case-insensitive matching**, and
two offer opt-in case-sensitivity. ADR 0020's accepted direction (always fold) is the consensus;
the documented ASCII limitation is riff-specific and is the one thing here a future ADR might
revisit.

### 4.4 Comparison table

| Implementation | Rule structure | Editor paradigm | Nested groups? | Limit + selected-by | Live update | Serialization | Language |
|---|---|---|---|---|---|---|---|
| iTunes / Music.app | tree (sub-expressions) | row builder, Option+Nest | yes | items/hours/MB/GB × random/name/… ± most/least | checkbox ("Live updating") | binary plists (base64) | closed |
| Clementine / Strawberry | flat list, one AND/OR | wizard + rows + live preview | no | sort random/field ± + limit | query-time SQL | QDataStream in config | C++ (Qt) |
| Quod Libet | expression tree | text query language | yes (parens) | none | always query-time | saved search text | Python |
| Rhythmbox | flat rows + disjunction toggle | dialog + rows | no | count/size/GB/time + sort ± | live query model | XML attributes | C (GTK) |
| Banshee | combinator tree | dialog + term rows + templates | yes | QueryOrder + QueryLimit + value | query-time | XML in SQLite column | C# |
| Amarok 2.x | constraint tree (APG); token canvas (Edit Filter) | token drag-drop; tree editor | yes | solver + candidate pick | generator/solver | XML presets | C++ (Qt) |
| MusicBee | rows + per-rule sub-rules | dialog + rows | yes (attached) | minutes/MB/items/albums × random/rating/… ± | refresh on access (toggle) | app database | closed |
| MediaMonkey | rows + nested condition | dialog + rows | yes | files/MB/minutes × selected-by (auto-refresh option) | always dynamic | app database | closed |
| foobar2000 | expression | keyword query language | yes (parens) | SORT BY clause | query-time | — (query strings) | closed |
| beets | expression terms | config YAML | yes (query syntax) | sort suffix in query | materialized on change | .m3u files | Python |
| Cantata | flat rows + named special fields | page + rows | no | numTracks + order ± | evaluated when served | plain text .rules | C++ (Qt) |
| Kodi | rule tree (nested rule elements) | GUI editor or XML | yes | `<limit>` + `<order direction>` | dynamic on display | .xsp XML | C++ |
| Navidrome | all/any group tree | files/JSON (client editors) | yes (1 mixing level) | multi-key sort ± , limit, limitPercent | lazy refresh, ≥5s | JSON .nsp / database | Go |
| Jellyfin plugin | DNF (OR of AND sets) | JSON files | two-level | none documented | regenerated playlists | JSON files | C# |
| **riff** | **box tree (all/any), flat value types** | **Settings page, rows + cards** | **yes (7 displayed levels)** | **none today** | **always live (projections)** | **typed SQLite tables** | **Rust** |

Keep the cells short on purpose: the prose in 4.1–4.3 is the reading of this table.

## 5. Recommendations for riff

Clearly labeled suggestions; none is implemented. Each names the evidence.

1. **Keep the box-tree model; do not simplify it.** The tree is what iTunes (sub-expressions with
   their own all/any byte), Banshee (`QueryNode`), Amarok APG (`ConstraintGroup`) and Kodi (nested
   rule elements) converge on, while every flat implementation either grew workarounds or froze
   (Clementine's persisted-enum comment shows the shape locking). riff's `RuleBox`/`RootBox` with
   one interleaved sibling order (`rule.rs:441-544`) is already the consensus shape with a better
   store story than any of them.
2. **Add limit + ordering to the model and store** — the one real feature gap. Every mature
   implementation has it; the natural design is riff-native first-class columns on
   `smart_playlists` (a `limit_kind` CHECK list of items/seconds/bytes, `limit_value`, a
   `sort_kind` list including random — see Banshee's `OrderBy`/`LimitNumber`/`LimitCriterion`
   triplet and iTunes' `LIMITMETHOD`/`SELECTIONMETHOD`/`LIMITINT`/`SELECTIONMETHODSIGN` bytes for
   the two mature encodings of the same three facts). This is a store migration widening the kind
   of ADR 0021 already contemplates; a "selected by" axis needs a sign-like bit only if riff adopts
   most/least pairs rather than separate ids — Navidrome's separate ids and iTunes' sign byte are
   the two precedents.
3. **Consider "starts with" / "ends with" as the next text comparisons.** Clementine's operator
   catalog is the survey's most complete row-editor set, and the two it has that riff lacks are
   exactly these (`Op_StartsWith`/`Op_EndsWith`, `searchterm.h:60-87`); foobar2000's `HAS`/`IS` pair
   and Navidrome's `startsWith` confirm the demand. They slot into riff's existing `Comparison`
   CHECK-list migration path; nothing else in the catalog changes.
4. **Keep case-insensitive-by-default, and keep folding at query time** ([ADR
   0020](../adr/0020-rule-matching-lowercases-at-query-time.md)). Every surveyed player defaults to
   case-insensitive; Quod Libet and beets offer opt-in case-sensitivity, which riff could add later
   as a per-rule toggle without touching storage. No change recommended now; the survey is the
   evidence that the accepted ADR direction matches the industry baseline.
5. **Keep the orthogonal `negated` bit; do not grow negated operator ids.** The survey's
   flat-negation implementations (Clementine's `Op_Not*`, Navidrome's `isNot`/`notInTheLast`)
   duplicate their operator surface, while iTunes (`LogicSign`), Amarok ("Invert condition") and
   riff keep inversion orthogonal. riff's "is not missing" normalisation to `IsMissing` + negation
   (`rule_row.rs:86-91`) is the right side of this line.
6. **Offer a "save as snapshot" path before offering a live-updating toggle.** The live/frozen
   distinction appears in three implementations as a per-playlist flag (iTunes "Live updating",
   MusicBee's refresh tick, Kodi save-as-static), but MusicBee's variant — always-live, with export
   to a static playlist when the listener wants to freeze — is the best fit for riff's always-live
   projections: it adds no evaluation-mode state to the engine, just a write of current matches
   into the ordinary playlist store.
7. **Steal Banshee's predefined templates for the empty state.** Banshee's editor ships a
   "Predefined Smart Playlists" expander whose entries "Open in editor" or "Create and save"
   (`Editor.cs:110-135`). riff's empty state (`smart_playlists_page.rs:912-931`) could offer two or
   three starter drafts (e.g. "Never played", "Added in the last 30 days" — both expressible with
   today's catalog) as pre-filled drafts, at zero model cost.
8. **The live count's committed-value cadence is validated — keep it, and consider Clementine's
   sample as the eventual upgrade.** riff counts committed answers so the number cannot "move per
   keystroke" (`smart_playlists_page.rs:506-543`); iTunes and MusicBee offer no editor-time count at
   all. If the count ever feels too coarse, Clementine's `SearchPreview` (a live sample of matching
   tracks under the rows, refreshed per change — `querywizardplugin.cpp:115-123`) is the surveyed
   midpoint between a bare count and running the playlist; it would read through the existing
   `SessionViews` seam like the count does.

## 6. Sources index

**Official documentation and support pages.** Apple — Create, edit, and delete Smart Playlists in
Music on Mac: <https://support.apple.com/guide/music/create-edit-and-delete-smart-playlists-mus1712973f4/mac>.
Quod Libet — Searching Your Library: <https://quodlibet.readthedocs.io/en/latest/guide/searching.html>
and the 4.6 changelog: <https://quodlibet.readthedocs.io/en/quodlibet-4.6/changelog.html>.
beets — smartplaylist plugin: <https://beets.readthedocs.io/en/stable/plugins/smartplaylist.html>
and query reference: <https://beets.readthedocs.io/en/stable/reference/query.html>.
MediaMonkey — Using AutoPlaylists: <https://www.mediamonkey.com/wiki/WebHelp:Using_AutoPlaylists>.
Navidrome — Smart Playlists: <https://www.navidrome.org/docs/usage/features/smart-playlists>.
Notion Help — Database views, filters, sorts & groups: <https://www.notion.com/help/views-filters-and-sorts>.
Linear Docs — Filters: <https://linear.app/docs/filters>. GitHub Docs — Filtering and searching
issues and pull requests:
<https://docs.github.com/en/issues/tracking-your-work-with-issues/using-issues/filtering-and-searching-issues-and-pull-requests>.
Gmail Help — Create rules (filters): <https://support.google.com/mail/answer/6579>.

**Source code read for this survey.**

- Clementine — `src/smartplaylists/search.h`, `searchterm.h`, `search.cpp`, `searchterm.cpp`,
  `wizard.cpp`, `querywizardplugin.cpp`, `querysearchpage.ui`, `searchtermwidget.cpp`:
  <https://github.com/clementine-player/Clementine>; Strawberry — `src/smartplaylists/` listing:
  <https://github.com/strawberrymusicplayer/strawberry>
- Quod Libet — `quodlibet/browsers/tracks.py` (listing): <https://github.com/quodlibet/quodlibet>
- Rhythmbox — `widgets/rb-query-creator.c`, `sources/rb-auto-playlist-source.c`:
  <https://gitlab.gnome.org/GNOME/rhythmbox>
- Banshee — `src/Core/Banshee.ThickClient/Banshee.SmartPlaylist.Gui/Editor.cs`,
  `src/Core/Banshee.Services/Banshee.SmartPlaylist/SmartPlaylistSource.cs`,
  `src/Core/Banshee.Services/Banshee.Database/BansheeDbFormatMigrator.cs`:
  <https://gitlab.gnome.org/Archive/banshee>
- Amarok — `src/dialogs/EditFilterDialog.h`, `src/playlistgenerator/ConstraintGroup.h/.cpp`,
  `doc/index.docbook`: <https://github.com/KDE/amarok>
- Cantata — `playlists/rulesplaylists.h/.cpp`, `playlists/smartplaylists.h`:
  <https://github.com/cdrummond/cantata>
- Jellyfin smart playlist plugin — README:
  <https://github.com/ankenyr/jellyfin-smartplaylist-plugin>
- itunessmart (iTunes Smart Criteria parser) — README and `itunessmart/data_structure.py`:
  <https://github.com/cvzi/itunes_smartplaylist>

**Community-maintained references** (marked in text, consulted via Wayback Machine snapshots).
Kodi Wiki — Smart playlists <https://kodi.wiki/view/Smart_playlists> (snapshot 2026-08-26) and XSP
Method <https://kodi.wiki/view/Smart_playlists/XSP_Method>. MusicBee Wiki — Playlists
<https://musicbee.fandom.com/wiki/Playlists> (snapshot 2026-07-14); MusicBee forum — Auto-Playlist
Question, topic 34969 (snapshot 2022-10-07). Foobar2000:Query syntax — Hydrogenaudio wiki
(snapshot 2024): <https://wiki.hydrogenaud.io/index.php?title=Foobar2000:Query_syntax>.

**riff internals cited** (relative to repo root): `crates/riff-persistence/src/rule.rs`,
`crates/riff-persistence/src/field_catalog.rs`, `crates/riff-infra/src/store/sqlite.rs`
(smart playlist DDL), `crates/riff-gui/src/ui/smart_playlists_page.rs`,
`crates/riff-gui/src/ui/rule_row.rs`, `crates/riff-gui/src/ui/rule_box.rs`,
`docs/adr/0020-rule-matching-lowercases-at-query-time.md`,
`docs/adr/0021-smart-playlist-rules-are-collection-data.md`.
