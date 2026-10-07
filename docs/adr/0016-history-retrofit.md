# The Commit History Was Retrofitted to Conform

**Status**: Accepted
**Date**: 2026-10-07

ADR 0015 made commit messages a machine-read interface going forward, and
for a while the log itself carried the exception: everything before the
first conventional commit stayed freeform on purpose, the changelog said
so, and the conventions document said "do not fix old messages". This
record reverses that policy. The reversal is dated and stands on its own;
nothing in it is smuggled backwards into the history it describes.

## Decision

**The entire commit history conforms to the commit-message profile.** On
2026-10-07 the repository's log was rewritten as a one-time maintenance
action: every message — including the July-era freeform commits — now
satisfies `tools/validate-commit-msg.py`, the merge structure is linear,
junk-only commits are gone, multi-feature commits are split, and the
changelog is generated from the rewritten log.

The constraint that made the rewrite worth paying for: the message format
is load-bearing (ADR 0015). A freeform era in the log is not a cosmetic
blemish — it is a range the changelog generator cannot parse, a range the
gates cannot judge, and a standing instruction to every future reader that
the format is optional. Keeping the exemption meant keeping three classes
of tooling with a permanently special-cased hole.

## What the rewrite destroyed

The costs are real and are recorded rather than minimized:

- **Every pre-rewrite commit SHA is gone.** Any external reference — an
  issue comment, a gist, a forum post — that cites an old SHA now points
  at an object unreachable from the default branch. The mapping from old
  SHAs to new commits travels with the rewrite as `mapping.tsv`, not in
  the history.
- **The merge structure is gone.** The four pull-request merge commits
  (`bd379ee`, `0240078`, `53edb99`, `e4376f9`) and one sync merge were
  flattened; the commits they carried survive, in replay order, as
  first-parent commits. GitHub retains `refs/pull/N/head`, so the PR
  pages remain reachable.
- **Commit dates are preserved, authorship is preserved; committer
  identity and commit order reflect the rewrite.** The log's `--oneline`
  view is now the unit of review, not the diff-and-merge shape.

## Consequences

- The commit conventions document states the retrofit instead of denying
  it, and its "do not fix old messages" instruction is retired with the
  era it guarded.
- The changelog generator runs with `filter_unconventional = false`:
  with nothing unconventional left, the filter's only remaining power
  would be to silently hide a commit that slipped past the gates. A
  regression must be visible, never absorbed.
- The enforcement points (hook, pull-request job, push job) are
  unchanged. They guard a history that now meets the rules everywhere,
  not just from some commit onward.
- The rewrite is a one-time action, not an accepted practice: history is
  not rewritten again for convenience. Any future rewrite needs its own
  ADR with its own costs.
