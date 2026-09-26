# Access-violation flake (open)

Status: **open** — the flake persists.

The integration test binary (`cargo test --all-targets`, target `riff-tests`,
which hosts every suite including the goldens) intermittently dies mid-run with
`STATUS_ACCESS_VIOLATION` (`0xc0000005`) before it can print its summary. There
is no failing test and no image diff; the next run is usually green. Contributors
see a crash that points at nothing. The App Runtime lifecycle work was diagnostic
scaffolding for this investigation, never the fix.

## Hypotheses

1. **Runtime teardown.** `AppRuntime::spawn` used to drop every worker thread's
   join handle, so every test that spawned a runtime leaked six detached
   workers for the process exit to reap. Plausible mechanism: the test harness
   finishes and tears down its own process state while detached workers are
   still touching it. The leak is now closed: `AppRuntime::spawn` returns
   `(AppRuntime, RuntimeLifecycle)` and `shutdown()` joins all six workers.
2. **wgpu harness concurrency.** Every golden harness brings up its own wgpu
   device. With all of them arriving together the driver does not survive it.
   `tests/golden_tests.rs` already caps the concurrency at
   `MAX_CONCURRENT_HARNESSES` for exactly this reason, and
   [./golden-image-testing.md](./golden-image-testing.md) records the policy: if
   the crash recurs, lower the cap rather than chase pixels.

## Teardown hypothesis refuted, wgpu hypothesis leads

Golden-only runs spawn no runtime workers and still crash, so the wgpu harness —
not application teardown — is implicated. The crash point moves from run to run
and lands in the block rather than on a particular golden, which is why nothing
in the app code is implicated.

## Open questions

- Whether lowering `MAX_CONCURRENT_HARNESSES` changes the rate. The cap is
  currently not sufficient, and the documented next move (lower the cap rather
  than chase pixels) has not been tried since.
- Whether the crash needs concurrency at all — a `--test-threads=1` golden run
  has not been recorded.
- Which wgpu backend/adapter the crashing devices come from, and whether the
  software fallback behaves differently.
