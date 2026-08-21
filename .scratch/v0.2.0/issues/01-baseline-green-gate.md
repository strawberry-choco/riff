# 01 — Baseline green gate

**What to build:** A trustworthy baseline for the v0.2.0 landing effort. The entire v0.2.0 implementation already exists as one uncommitted blob in the working tree; before any feature slice is verified and committed, the full quality gate must pass locally exactly as CI enforces it: format check, clippy, and the complete test suite. Where the gate fails, fix the root cause — never weaken or delete a test to make it pass. Commit the fixes as a single focused baseline commit.

**Blocked by:** None — can start immediately.

**Status:** done

- [ ] `cargo fmt --check` passes with zero diffs
- [ ] `cargo clippy --all-targets` passes with zero warnings (fix real issues, don't silence lints)
- [ ] `cargo test --all-targets` passes — every test in the integration suite green
- [ ] Any failures are fixed at root cause; no test was weakened, ignored, or deleted to achieve green
- [ ] Baseline fixes committed as a single focused commit

