# ADR-006: Local-Only Discovery and Metadata Strategy

## Status

Accepted

## Context

The music player pain points research identified that users want both the zero-friction experience of streaming AND the ownership/control of local files, but no product delivers both. Streaming services degrade over time (feature removal, rising prices, algorithmic manipulation), while local players have sustainability issues (outdated UI, plugin breakage, Windows-only).

Under riff's constraints (local-only, multi-platform), we cannot replicate online discovery or auto-tagging from external databases. However, we can address the core pain points by leveraging the inherent advantages of local ownership while providing streaming-like UX polish.

## Decision

We will build for the **"owner" persona** — someone with a local collection who wants zero-config polish, never-destructive operations, and offline discovery capabilities. This involves:

1. **Metadata tag writing** - Users can edit tags directly in riff without external tools, using lofty's write capabilities. File tags are the source of truth; cache is derived.

2. **Offline discovery via smart playlists** - Auto-generated playlists based on metadata (Recently Added, Most Played, Never Played, Lost Gems) that require no internet connection and provide the "discovery" experience users expect from streaming.

3. **Progressive disclosure UI** - Default to minimal controls, reveal advanced features behind explicit toggles. This addresses the core tension between simplicity and power that no player currently balances well.

4. **Linux first-class support** - Treat Linux as equal to Windows/macOS, providing polished experiences even where native dialogs aren't available.

5. **Accessibility from day one** - Address the research's blind spot by supporting keyboard navigation, high contrast, screen readers, and cognitive load reduction.

## Rationale

- **Tag writing** eliminates the "needs external tools" pain point (#24) while staying local-only. Using file tags as source of truth avoids creating another disagreeing metadata source (#30).

- **Smart playlists** provide offline discovery that mimics streaming UX without requiring internet. This directly addresses the Academic's "paradox of choice" finding and the Practitioner's desire for navigation without server dependency.

- **Progressive disclosure** solves the core cross-cutting pain (#29) of no player balancing simplicity AND power. It's the streaming UX principle applied to local-only context.

- **Linux first-class** addresses the High severity pain (#34) of poor Linux support across commercial players. The research specifically calls out cross-platform as eliminating 80% of competitors.

- **Accessibility** addresses the research's blind spot where all 35 pain points ignored users with disabilities. This is a differentiator and the right thing to do.

## Consequences

- **Positive**: riff becomes the anti-lock-in player (open metadata, never-destructive), addresses the worst local player failure modes, and provides discovery without internet dependency.

- **Negative**: We cannot replicate online recommendation quality or auto-tagging from external databases. Smart playlists are limited to metadata-based similarity.

- **Technical**: Tag writing requires careful error handling and background threading. Smart playlists need play count tracking in the cache. Progressive disclosure requires careful UI state management.

## Implementation Notes

- Tag writing must be background-threaded to avoid UI blocking
- File tags are source of truth; cache rebuilds on conflict
- Smart playlists update automatically when play counts change
- Linux folder picker needs thoughtful fallback design
- Accessibility must be implemented from the start, not bolted on later

## Related Decisions

- ADR-004: Library Cache as JSON (provides the persistence layer for play counts)
- ADR-005: Track Identity is Path (ensures tag edits map to correct files)
- ADR-002: No Tray on Linux (consistent with treating Linux as first-class but different)