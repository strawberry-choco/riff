# Commit Messages Are a Machine-Read Interface

**Status**: Accepted
**Date**: 2026-10-07

Before this decision, riff's commit history was freeform — `add animation`,
`improve ui`, `refactor` — and the Changelog's promise that Conventional
Commits are adopted going forward was a promise, not a mechanism: a
well-formed message was a matter of memory, and nothing anywhere could
reject a malformed one. From the next commit forward, every commit message
in riff follows Conventional Commits 1.0.0 under a riff-specific profile,
and the reason is not style: the format is load-bearing.

## Decision

**Commit messages are a machine-read interface.** The changelog generator
(git-cliff, configured by `cliff.toml`) parses commit messages to decide
both what text lands in the Changelog and which version bump a release
takes. That makes the message format part of the change rather than
decoration on top of it: a commit message that does not parse is a defect
in the change, the same class of problem as a build failure, and is
treated as one — a malformed message is rejected, not commented on. A
contributor is not writing prose for future humans alone; they are writing
structured input for the release tooling, and the tooling's parse is the
reason the format exists.

The format is enforced by **one validator with three entry points** —
`tools/validate-commit-msg.py` reading a message file (what the local
`commit-msg` hook receives), reading a message from standard input (what
both CI `commit-lint` jobs use), and a `--self-test` mode (the built-in
table of known cases both CI jobs run before validating anything). There
is one rule implementation and no second copy of any rule anywhere, which
is what makes it structurally impossible for the local verdict and the CI
verdict to disagree: a message that passes locally cannot fail in CI,
because they ran the same code.

**The Changelog is a generated artifact from 0.2.0 onward.** Each version
section is produced by git-cliff from the commit history between tags — a
pure function of the git state, so running the regeneration twice is
harmless and yields the same file. Hand-editing a generated section does
not improve the Changelog; it contradicts the record the commits carry,
and the next regeneration silently discards the edit. The consistency
guard in the release workflow's notes job is what upgrades this from a
stated intention to a checked invariant: it regenerates the tagged
version's section from the tag range and fails the release on a mismatch,
so a commit landing between release preparation and tagging fails loudly
instead of diverging quietly. The hand-written 0.1.0 section is preserved
verbatim as a one-off; it predates the format and is never generated.

**Pre-conventional history is deliberately not retro-fitted.** Everything
before the first conventional commit stays freeform on purpose — no
prefixes added, no messages rewritten, no per-release changelog
reconstructed from a log that does not contain that structure. Every
enforcement point operates on a commit range (merge-base to branch head,
previous tip to current tip, tag range), never on the whole log, so no
gate ever reaches back into the era the rules do not govern.

**Rebase-and-merge is the only merge strategy.** This is a judgement call,
and it is what makes the rest coherent: GitHub replays a pull request's
commits onto the merge target's tip preserving every message, author, and
author date, so the messages a contributor writes on a branch **are** the
messages that land on the default branch. There is no rewrite step between
the branch and the history, therefore no pull-request title check anywhere,
and no automated pull request needs an exemption from the message rules —
the dependency bot's existing `build`/`ci` prefixes reach the default
branch intact and become load-bearing for the first time. The accepted
cost is real: every commit receives a new SHA in the replay, so commit-SHA
references in review comments and issues point at commits that will not
exist after the merge. The pull request number is the durable reference.

**Pre-1.0 version semantics are pinned explicitly in `cliff.toml` rather
than inherited from git-cliff's defaults.** A feature bumps the minor
version (`features_always_bump_minor = true`, where the default at 0.x is
a patch bump); a fix or performance change bumps the patch version; a
breaking change bumps the **minor** version while the major version is
zero (`breaking_always_bump_major = false`, where the default is a major
bump). This is a judgement call recorded as such, not a computation: at
major zero, SemVer's own contract is that nothing is stable yet, so
choosing "features are visible progress worth a minor bump" and "breaking
changes do not fake a 1.0 per bump" is a policy the project states —
pinned in one place so the rule, not release-time taste, answers "is this
a minor or a major".

## Considered Options

- **Squash merging (rejected)**: the squash step discards the per-commit
  messages and replaces them with a pull-request title — exactly the
  messages the changelog generator parses. Under squash, the entire
  message profile is decoration and the dependency bot's prefixes are dead
  configuration, discarded before reaching the default branch.
- **Node-based tooling such as commitlint (rejected)**: it would introduce
  a Node toolchain, lockfile, and install step for a repository that has
  deliberately none, to enforce rules a single versioned Python script
  enforces completely. Ruled out by decision, not deferred.
- **Trusting memory with no gate (rejected)**: the Changelog had already
  promised the format with no mechanism behind it — a promise the first
  malformed commit would silently break. This was the status quo and is
  what this decision replaces.
- **One validator, three entry points, rebase-and-merge, generated
  Changelog (chosen)**: the smallest shape in which local and CI cannot
  disagree, the messages written are the messages parsed, and the
  Changelog cannot drift from the commits it describes.

## Consequences

- A malformed commit message is a defect with a defined failure surface:
  rejected at commit time locally (fast feedback, the direct-push path),
  at pull-request time in CI (the required branch-protection check), and
  on push to the default branch (defense in depth whose range is exactly
  what landed). A missing `python3` on a contributor's machine steps the
  hook aside with a notice; CI is the real gate.
- The Changelog is no longer written by hand at release time and cannot
  drift from the history: regeneration is a pure function of the git
  state, the release notes' change section comes from the same source, and
  the consistency guard converts "nothing merges between preparation and
  tagging" from a discipline into a checked invariant whose failure mode
  is "re-run preparation", never "edit by hand".
- Commit SHAs are unreliable across a merge; the pull request number is
  the durable reference. Contributors and reviewers reference pull
  requests, not SHAs, in anything meant to survive the merge.
- Extending the changelog catalog (a new type, a new section mapping) or
  the pre-1.0 bump rules means editing `cliff.toml` — the single place
  that catalog lives — not a document and a config together. The prose in
  `docs/engineering/commit-conventions.md` describes the configuration; it
  does not restate it as a second source of truth.
- Changing the message profile is now a release-tooling change with
  regeneration and guard implications, argued against this record — not a
  per-commit stylistic preference.
- The pre-conventional era remains permanently freeform, and the gates
  never reach it: nobody "fixes" old messages, and a range that resolves
  to nothing fails loudly rather than passing vacuously.
