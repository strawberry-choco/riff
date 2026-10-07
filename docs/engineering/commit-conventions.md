# Commit conventions

Every commit lands with a Conventional Commit message. The format is not a house style that review nudges people toward — it is checked by tooling at three points, listed at the end of this document. The one rule implementation is `tools/validate-commit-msg.py`: the local hook and both CI jobs run it, so a message that passes locally can never fail in CI.

## The message format

A message is a single header, optionally followed by a body:

```
type(scope)!: description

body
```

Both the scope and the `!` are optional; the minimal shape is `type: description`.

**Types.** The type is one of a closed set of twelve:

> `feat`, `fix`, `perf`, `refactor`, `deps`, `revert`, `build`, `ci`, `chore`, `docs`, `test`, `style`

The type is matched case-sensitively, and anything outside the set — `feature:`, `Feat:`, a message with no prefix at all — is rejected. The rejection message lists the allowed types, so a fix is one edit. `build` and `ci` are in the set because the dependency bot already emits them: its updates satisfy the same rules as everyone else's, and there is no bot exemption anywhere in the tooling.

**Scope.** The scope is optional and free-form — there is deliberately no closed list, so naming a scope after a crate or a subsystem is never a chore. The only rule is the character set: lowercase alphanumerics, with `-`, `_`, `/`, and `.` as separators between runs. `ui`, `cover_cache`, and `ui/cover_cache` are all valid; `UI` is not, and neither is an empty scope (`feat(): ...`).

**Length limits.** The header is at most 72 characters, and every body line is at most 80. These are the bounds that keep subjects and body lines readable in a terminal and in a log, instead of wrapped or truncated by whatever happens to display them. The validator counts characters exactly — 72 passes, 73 does not — and names the over-long line when it rejects one.

**Body.** A body must be separated from the header by a blank line. Before checking anything, the validator strips editor decoration: comment lines and git's scissors line are ignored, leading and trailing blank lines are ignored, and CRLF line endings are tolerated.

**Description.** The description must not be empty, and it must not end with a period.

## Breaking changes

A breaking change is expressed by an exclamation mark after the type and optional scope, or by a breaking-change footer, or by both at once. Any one of the three is enough:

- `feat!: change the AudioOutput port`
- `feat(api)!: change the AudioOutput port`
- a footer line `BREAKING CHANGE: <description>`
- a footer line `BREAKING-CHANGE: <description>` — both spellings are accepted

The description after a breaking-change footer must not be empty.

Near-miss spellings — `BREAKING:`, `Breaking Change:`, `BREAKING_CHANGE:`, and their other case and separator variants — are rejected rather than guessed at, and the rejection message names the canonical forms. The exactness is deliberate: the version bump for a breaking change is decided by the generator finding one of these markers, so a near-miss that looked acceptable would silently mis-version a release.

## Messages the tooling never blocks

Git generates some message texts itself, and the validator accepts those without checking them: a message starting with `Merge `, `Revert "`, `fixup! `, `squash! `, `amend! `, `WIP:`, or `WIP ` passes through untouched. Ordinary git operations — merging a branch, reverting a commit, autosquashing during a rebase, saving a change in progress — are never blocked by the message rules.

## What each commit does to the Changelog and the version

The changelog generator (git-cliff, configured by `cliff.toml`) groups each commit by its type into a [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) section and derives the version bump from what landed. This table paraphrases `cliff.toml`, which is the single source of the mapping — extending the catalog means editing that file, not a document and a config together.

| Type | Changelog section | Version bump (pre-1.0) |
|---|---|---|
| `feat` | Added | minor |
| `fix` | Fixed | patch |
| `perf` | Changed | patch |
| `refactor` | Changed | none |
| `deps` | Changed | none |
| `build`, `ci`, `chore`, `docs`, `test`, `style`, `revert` | — (hidden) | none |

The pre-1.0 semantics are pinned in `cliff.toml`'s `[bump]` section, set explicitly rather than inherited from git-cliff's defaults, so "is this a minor or a major" is answered by a rule rather than by taste at release time:

- A feature bumps the **minor** version, even while the major version is zero.
- A breaking change bumps the **minor** version while the major version is zero. After 1.0 the standard SemVer rule — a breaking change bumps major — resumes.
- A fix or a performance change bumps the **patch** version.
- A type that never reaches a changelog section never affects a version bump either.

## Referencing changes: the pull request, not the SHA

Pull requests are merged with rebase-and-merge, and the replay gives every commit a new SHA when it lands. A commit SHA quoted in a review comment or an issue therefore points at an object that no longer exists on `master` after the merge. The durable reference is the **pull request number**. See [Merge strategy](../../CONTRIBUTING.md#merge-strategy) for what the replay preserves and what it changes.

## The format is load-bearing

The commit message is a machine-read interface, not decoration: the changelog generator parses these messages to write `CHANGELOG.md` and to compute version bumps. A malformed message would not merely look untidy — left in, it would put a change in the wrong section or miss a version bump, silently. That is why a malformed message is rejected rather than tidied, why the type set is closed, and why near-miss breaking markers are checked for exact spellings.

## Where the rules are enforced

Three enforcement points run the same validator, so there is exactly one rule implementation and the local verdict can never disagree with the CI verdict:

1. **The commit-time hook.** An optional local convenience, not a requirement. One command wires it in:
   ```bash
   git config core.hooksPath .githooks
   ```
   The hook is a thin shim over the validator and needs `python3` on the `PATH`; when the interpreter is missing, it prints a one-line notice naming the CI job that is the real gate and exits successfully, so a contributor without Python is never blocked by an optional local convenience.

2. **The `commit messages (pull request)` job.** The pre-merge gate. It runs on every pull request, runs the validator's self-test before validating anything — so a broken validator fails loudly instead of passing by crashing into a pass — and then validates every commit between the merge base and the branch head. An empty range is an explicit error, never a vacuous pass. This job is the required check in branch protection.

3. **The `commit messages (push)` job.** Defense in depth on the default branch. It runs on pushes to `master` and validates every commit between the previous tip and the new one — exactly the commits that landed — so a push that arrives by any route meets the same rules a pull request does.

Both CI jobs run on Linux only, so the matrix does not grow.

## History before conventional commits

Everything before the first conventional commit stays freeform on purpose. Nothing has been retro-fitted: no old message has been rewritten and no changelog has been reconstructed for that era. No gate ever reads whole history, either — the hook sees one draft message, and each CI job validates a commit range. The changelog generator filters non-conventional commits out rather than failing on them, and it hides anything it cannot parse. Do not "fix" old messages.
