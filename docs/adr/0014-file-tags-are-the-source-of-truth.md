# File Tags Are the Source of Truth; the Store Mirrors Them

**Status**: Accepted
**Date**: 2026-10-04

For every fact that also lives in a Track's file tags — Metadata and `ReplayGain` values alike — **the file is authoritative and the Application Store mirrors it**. Every write goes file tags first, then the Store facts as one durable change; a file-write failure is reported and the Store is left untouched, so the app never claims a value the file does not carry. Drift between the two — an external tool re-tagging a file while riff is not looking — self-heals through the Library Scan's freshness filter: the next scan re-reads the changed file (a `METADATA_VERSION` bump forces one full re-read) and refreshes the mirror in place. There is no push invalidation, no watcher on tag writes, no bidirectional merge.

This extends, and makes the general rule of, the semantics the Library Scan and Tag Edit already implemented: the scan has always treated the file as the thing to read, and Tag Edit has always written file-first. The ReplayGain work (measurement passes writing measured values, hand edits via the Inline Tag Editor) is what made the pattern a rule instead of two one-offs.

The mirror exists for the UI and playback, not for correctness: Session Projections read the Store so rendering never opens audio files per frame, and playback resolves a Track's gains from Store facts without re-parsing tags at track load. The Store's `tracks` table carries the values as nullable columns with no defaults — an unmeasured value is `NULL`, never a stand-in zero.

## Considered Options

- **The Store as source of truth (rejected)**: it is riff's own authoritative persistent state and it is tempting to treat measured values as Store facts that happen to get exported to files. But files leave the app: an external tool writes them, a user re-runs rsgain on them, and any policy where riff's Store could out-argue the file ends in either stale app values or clobbered user files. The file survives the Store (Clear Library wipes the mirror; the tags remain), so the file must be the source.
- **Push invalidation via file watching (rejected)**: a watcher that re-reads re-tagged files would heal drift instantly, but it duplicates the watcher's job for a case the scan already owns, and external re-tags usually arrive with the file still open by the other tool. The freshness filter's "refresh on next scan" is the reconciler, deliberately.
- **Per-fact ownership split (rejected)**: e.g. play history owned by the Store (it does not live in tags) while Metadata is file-owned. That is already true and stays true — but the *shared* facts must have exactly one authority, and splitting ownership per-fact would mean some Metadata fields read from files and others from the Store, which is unobservable complexity for no user.

## Consequences

- Every new writer of shared facts composes the same two steps: file tags first, Store commit second, and the Store commit is skipped entirely when the file write fails.
- A hand edit (Inline Tag Editor) and a measurement (ReplayGain Pass) are equally "the file being re-tagged": measurement always wins over a manual edit, because the next write re-opens the same source of truth.
- Clear Library wipes the mirrored values with the rest of the collection data; re-analysis starts from a clean slate while the files keep whatever they carry.
