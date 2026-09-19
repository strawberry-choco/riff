# Retire the grid browser layout and the content top bar; search becomes titlebar chrome

**Status**: Accepted
**Date**: 2026-09-19

The global "Search or jump to…" field used to live in a dedicated content strip — the content top bar — squeezed between the window titlebar and the Library stage: a second 48px bar that only existed on the Library View. The same strip held a list/grid toggle whose grid mode duplicated the list in a less useful form and added a persisted Settings branch nobody used. With grid gone, the strip's only remaining content was the search field, and a bar holding a single field had no value of its own: it consumed vertical space on the most-used View, forked every chrome decision, and forced the Library stage to grow at the top for no reason.

## Decision

Three changes, applied end-to-end per the vertical crate split (ADR 0009):

1. **The search field becomes shared titlebar chrome.** The global search field (search glyph, "Search or jump to…" hint, clear affordance, focus ring, Escape dismissal) moves into the titlebar, drawn after the drag region so it wins pointer interaction exactly like the existing titlebar controls. It is always visible on every View (Library, Now Playing, Settings), centered between the wordmark/scan-status cluster and the nav/caption cluster, capped at its former maximum width, and shrinks before either side as the window narrows with a minimum gap at the minimum window size. Its interaction contract is unchanged: typing edits the Library session's query and filters the Library listing in real time, Ctrl+K requests focus, Escape clears the query and surrenders focus, and navigating the sidebar clears the query. The reusable search helpers move from the content top bar's module into the titlebar's module.
2. **Grid is deleted end-to-end.** The browser-layout enum, its store codecs, the library-session field, the Settings scalar, the Preferences hydration/commit branch, the tile renderer, the thumbnail-grid code, and the list/grid toggle are all deleted outright — "grid" stops being a concept anyone has to reason about. The browser column has exactly one render path: the list.
3. **The content top bar is deleted.** The strip's render call site is removed from the app shell, its module is deleted, and its 48px height token is retired. The Library stage (and every View's stage) starts directly below the titlebar, returning the strip's vertical space.

The persisted choice is retired from the Application Store schema: an append-only, checksummed migration (012) rebuilds the scalar Settings row without the retired `browser_layout` column, carrying every other scalar over exactly. This follows the established in-repo precedent required because SQLite refuses to drop a column that a CHECK constraint references (the same rebuild migration 010 used for the missing-artwork strategy). Fresh installs never create the column. Stores that previously had grid engaged open on the now-permanent list.

## Considered Options

- **Keep the content top bar for the search field (rejected)**: a 48px strip holding a single control has no value of its own. It consumed vertical space on the most-used View, forked every chrome decision between "titlebar chrome" and "content strip", and made the Library stage grow at the top. Moving the field into the titlebar removes the strip without touching the interaction contract.
- **Keep grid behind the toggle, default to list (rejected)**: grid duplicated the list in a less useful form, and its persisted Settings branch was used by nobody. Keeping the code path would keep the maintenance cost and the mode-surprise after a restart for zero users. Permanently list-only is the honest state.
- **Dead-code grid rather than delete it (rejected)**: the issue's maintainer story requires "grid" to stop being a concept anyone has to reason about. Dead-coded enums, codecs, and render paths remain in the symbol table and the docs forever; deletion is what makes the absence visible and testable (a golden comparison catches any regression that resurrects a toggle, a tile path, or the extra strip).

## Consequences

- The titlebar is now the single top chrome strip (ADR 0005): the shell's top chrome became more consolidated, not less. The 32px field center-fits the existing 56px titlebar; no chrome height token changes and the fixed-chrome minimum window size is unchanged — removing the strip makes the Library stage meet its guaranteed minimum size again.
- `BrowserLayout` is gone from `riff-backend`; `browser_layout` is gone from `ScalarSettings` (`riff-persistence`), the `app_settings` table and the scalar read/write SQL (`riff-infra`), and the Preferences round-trip. The store-query model (ADRs 0002, 0003) is untouched beyond the migrated scalar.
- Migration 012 is append-only and checksummed like its predecessors; already-migrated stores reopen without re-applying it.
- The content top bar module (`crate::ui::topbar`) and the `TOPBAR_H` token are deleted; the titlebar module (`crate::ui::chrome`) owns the search helpers.
- Golden baselines: the content-top-bar goldens (idle, light, grid-toggle, search-focus variants) and the browser-grid golden are deleted; new titlebar-with-search goldens (idle, light, focused, focused-HC) replace them; the shell chrome goldens are regenerated with the search field in place and the strip gone. A golden comparison now fails if anyone resurrects a toggle, a tile path, or the extra strip.
- Product documentation no longer lists the content top bar or a grid toggle as shipped; the iTunes comparison stops crediting the list/grid toggle as the view-options story.
