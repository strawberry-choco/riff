# Access-violation flake (closed)

Status: **closed** — the gap in the concurrency cap was found and closed; a
full parallel run is green with it.

The integration test binary (`cargo test --all-targets`, target `riff-tests`,
which hosts every suite including the goldens) intermittently died mid-run with
`STATUS_ACCESS_VIOLATION` (`0xc0000005`) before it could print its summary. There
was no failing test and no image diff; the next run was usually green.
Contributors saw a crash that pointed at nothing. The App Runtime lifecycle
work was diagnostic scaffolding for this investigation, never the fix.

## Hypotheses

1. **Runtime teardown.** `AppRuntime::spawn` used to drop every worker thread's
   join handle, so every test that spawned a runtime leaked six detached
   workers for the process exit to reap. Plausible mechanism: the test harness
   finishes and tears down its own process state while detached workers are
   still touching it. The leak is now closed: `AppRuntime::spawn` returns
   `(AppRuntime, RuntimeLifecycle)` and `shutdown()` joins all six workers.
2. **wgpu harness concurrency.** Every harness brings up its own wgpu
   device. With all of them arriving together the driver does not survive it.

## Teardown hypothesis refuted; wgpu hypothesis confirmed, with a refinement

Golden-only runs spawn no runtime workers and still crash, so the wgpu harness —
not application teardown — is implicated. The crash point moves from run to run
and lands in the render-heavy region rather than on a particular test, which is
why nothing in the app code is implicated.

The refinement: the cap was real but its coverage was not.
`MAX_CONCURRENT_HARNESSES` lived in `tests/golden_tests.rs` and gated only the
goldens, while `ui_tests` built its harnesses (90+ direct `Harness::builder`
sites, plus the helpers that return one) with no gate at all — dozens of live
wgpu devices whenever the thread pool ran wide. That is why "the cap was not
enough": it was never counting the suites that crashed.

The gate now lives at the crate root (`harness_gate` in `tests/mod.rs`) and
every harness-building scope in the crate reserves a slot: each golden, every
`ui_tests` site (a slot is installed before each harness binding, including
bindings of helper-returned harnesses), and the composed shells (`compose()`
carries its slot inside the returned `ComposedHarness`). The slot is reentrant
per thread — a nested acquisition under a caller's own slot is a no-op — so
installing it at every creation point cannot deadlock against the cap.

Evidence, from the machine that reproduced it (Windows, default parallelism):

- `cargo test --all-targets` crashed 2/2 with `STATUS_ACCESS_VIOLATION`; the
  second crash landed after every golden had passed, in the uncapped
  `ui_tests` region.
- `--test-threads=1` on the same binary was green (862 passed, 0 failed) —
  the crash needs concurrency.
- After the hoist, the same parallel `cargo test --all-targets` is green
  (862 passed, 0 failed, ~25 s) — and faster than the single-threaded run
  (152 s), because the cap throttles only harness-holding tests while the
  rest of the suite keeps the thread pool.

If it ever recurs, the standing policy holds: lower
`MAX_CONCURRENT_HARNESSES` rather than chase pixels.
