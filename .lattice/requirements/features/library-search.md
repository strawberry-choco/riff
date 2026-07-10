---
feature: Library Search
epic: Music Library
status: implemented
priority: P1
depends_on: [Metadata Extraction]
personas: []
source_docs: []
implementation_notes: |
  Implemented in app/library_manager.rs search() method. Linear scan across
  title, artist, album, album_artist fields. Case-insensitive substring matching.
---

# Library Search

## Problem Statement

Users with large music libraries (thousands of tracks) cannot browse file-by-file to find what they want to hear. They need a fast search interface that filters the library by artist, album artist, album title, or track title. The search should be instantaneous (sub-100ms) for libraries up to 50,000 tracks.

## User / Personas

**Large Library Owner**: A user with 10,000+ tracks who remembers artists and albums but not exact filenames. They need to type "radiohead" and see all Radiohead tracks and albums immediately.

**Genre Explorer**: A user who wants to find all jazz albums or all tracks by a specific composer. They need multi-field search capability.

## Scope

**In scope:**
- Real-time text search across: Artist, Album Artist, Album, Title
- Case-insensitive search
- Substring matching (searching "head" matches "Radiohead")
- Search results grouped by category (Artists, Albums, Tracks)
- Clear search button and empty-state message
- Keyboard shortcut to focus search box (Ctrl+F)

**Out of scope:**
- Full-text search across lyrics or comments
- Fuzzy search (typo tolerance, Levenshtein distance)
- Advanced query syntax (AND, OR, NOT, field:prefix)
- Search history or saved searches
- Search within a specific folder only

## Boundary Conditions

- Empty search query shows all library items (no filtering)
- Search with no matches shows an empty state message
- Search is responsive even while the library is being scanned in the background
- Special characters in search query are treated literally (no regex injection)
- Search operates on the in-memory metadata index, not by re-reading files

## Assumptions

- The entire library metadata fits in RAM (50,000 tracks * ~200 bytes metadata = ~10MB, easily fits)
- Users search by typing partial names, not by exact full strings
- Search performance is acceptable with a simple linear scan for the target library sizes
- No need for persistent search index on disk (in-memory is sufficient)

## Scenarios

### Scenario 1: Search by artist name
A user types an artist name to find their music.

**Acceptance Criteria:**
- Given the library contains tracks with Artist="Radiohead" and Album Artist="Radiohead", when the user types "radiohead" in the search box, then all matching tracks and albums are displayed
- Given the user types "RADIOHEAD" (all caps), when the search executes, then results are identical to the lowercase search (case-insensitive)
- Given the user types "head" (partial match), when the search executes, then "Radiohead" tracks are included in results
- Given the user types a non-matching string "zzzzzz", when the search executes, then an empty state message is shown

### Scenario 2: Search by album title
A user types an album name to find a specific album.

**Acceptance Criteria:**
- Given the library contains an album "OK Computer", when the user types "ok computer", then the album and its tracks are displayed
- Given the user types "computer", when the search executes, then "OK Computer" is included in results (substring match)

### Scenario 3: Clear search
A user wants to return to the full library view after searching.

**Acceptance Criteria:**
- Given search results are displayed, when the user clicks the clear button, then the full library view is restored
- Given search results are displayed, when the user presses Escape, then the search box is cleared and the full library view is restored

## Implementation Notes

1. **In-memory index**: Maintain a `Vec<TrackMetadata>` in the library manager. Search performs a linear scan filtering tracks where any of the searchable fields contains the query string (case-insensitive).
2. **Normalization**: Convert both the query and the field values to lowercase for comparison. Use `to_lowercase()` which handles Unicode correctly.
3. **Debouncing**: Debounce the search input by 100ms so that typing "radiohead" does not trigger 10 intermediate searches.
4. **Result grouping**: Group search results into three sections: Matching Artists (unique artist names), Matching Albums (unique album + album artist combinations), Matching Tracks (individual tracks). This gives users context about what matched.
5. **Performance**: For libraries >10,000 tracks, if linear scan becomes slow, consider building a simple inverted index (HashMap<String, Vec<TrackId>>) for each field. But start with linear scan for simplicity.

## Open Questions

- [ ] Should search include the "Comment" and "Composer" fields? (Non-blocking: start with basic fields, expand if requested)
- [ ] Should search results update live as the library scanner finds new files? (Non-blocking: yes, the index is updated in real-time)

## Links

- Design: *(updated when design-blueprint creates a context anchor doc for this feature)*
- Epic index: [index.md](../index.md)
