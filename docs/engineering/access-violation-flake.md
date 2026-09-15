# Access-violation flake: investigation log

Status: **open** — the flake persists.

The integration test binary (`cargo test --all-targets`, target `riff-tests`,
which hosts every suite including the goldens) intermittently dies mid-run with
`STATUS_ACCESS_VIOLATION` (`0xc0000005`) before it can print its summary. There
is no failing test and no image diff; the next run is usually green. Contributors
see a crash that points at nothing.

This file is the running record. Add an entry per measurement campaign rather
than editing earlier ones — the value of the log is the history, not a tidy
conclusion.

## Hypotheses

1. **Runtime teardown.** `AppRuntime::spawn` used to drop every worker thread's
   join handle, so every test that spawned a runtime leaked six detached
   workers for the process exit to reap. Plausible mechanism: the test harness
   finishes and tears down its own process state while detached workers are
   still touching it.
2. **wgpu harness concurrency.** Every golden harness brings up its own wgpu
   device. With all of them arriving together the driver does not survive it.
   `tests/golden_tests.rs` already caps the concurrency at
   `MAX_CONCURRENT_HARNESSES` (4) for exactly this reason, and
   [./golden-image-testing.md](./golden-image-testing.md) records that the cap
   appeared to remove the crash.

## 2026-09-16 — teardown hypothesis refuted, wgpu hypothesis leads

Measured after the App Runtime lifecycle landed (`AppRuntime::spawn` now returns
`(AppRuntime, RuntimeLifecycle)` and `shutdown()` joins all six workers), and
therefore with hypothesis 1's leak closed.

The suite has 478 tests in the integration target; four of them are new and
exercise shutdown directly.

| Run | Command | Result | Died after | Crash code |
|---|---|---|---|---|
| 1 | `cargo test --all-targets` | crash | 321 tests | `0xc0000005` |
| 2 | `cargo test --all-targets` | crash | 354 tests | `0xc0000005` |
| 3 | `cargo test --all-targets` | **pass** | — (478 passed, 0 failed) | — |
| 4 | `cargo test -p riff-tests --test integration golden_tests` | crash | 39 tests | `0xc0000005` |
| 5 | `cargo test -p riff-tests --test integration golden_tests` | crash | 26 tests | `0xc0000005` |
| 6 | `cargo test -p riff-tests --test integration golden_tests` | crash | 31 tests | `0xc0000005` |

Two findings, in order of weight:

- **Runs 4–6 contain no `AppRuntime` at all.** The `golden_tests` filter excludes
  every runtime test, so those runs spawn zero worker threads — and they still
  crashed 3 out of 3. The teardown hypothesis cannot explain them, so it is not
  the (only) cause. **wgpu hypothesis leads.**
- **The crash point moves.** Runs 4–6 died on three different goldens
  (`playerbar_repeat_one_dark`, `library_hero_dark`, `now_playing_empty_dark`)
  at three different test counts (39/26/31), and the full-suite runs died on two
  further goldens. Nothing in the app code is implicated: it is the block, not a
  case.

The leak closure is still worth having on its own terms — it is what makes
run 3's full green reproducible rather than hoped for — but it did not fix the
crash, and run 3 is not evidence that it did.

## Not yet measured

- Whether lowering `MAX_CONCURRENT_HARNESSES` below 4 changes the rate. The cap
  is currently not sufficient; the documented next move when this recurs
  (lower the cap rather than chase pixels) has not been tried since.
- Whether the crash needs concurrency at all — a `--test-threads=1` golden run
  has not been recorded.
- Which wgpu backend/adapter the crashing devices come from, and whether the
  software fallback behaves differently.

## Out of scope for this log

This is a record, not a plan. The App Runtime work was diagnostic scaffolding
for it, never the fix.
