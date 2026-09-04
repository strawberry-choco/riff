# Plan: Add `crates/` Folder Convention

**Goal**: Move all capability crates under a `crates/` directory, following the Rust workspace convention (The Rust Book ch14-03, Cargo Book guide). The `tests/` crate stays at the workspace root.

## Before / After

```
BEFORE (root):                    AFTER (conventional):
├── Cargo.toml                    ├── Cargo.toml
├── Cargo.lock                    ├── Cargo.lock
├── riff-persistence/             ├── crates/
├── riff-playback/                │   ├── riff-persistence/
├── riff-library/                 │   ├── riff-playback/
├── riff-infra/                   │   ├── riff-library/
├── riff-backend/                 │   ├── riff-infra/
├── riff-gui/                     │   ├── riff-backend/
└── tests/                        │   └── riff-gui/
                                  └── tests/          ← unchanged
```

## Path dependency impact

All crates under `crates/` reference siblings with the same relative path (`../riff-X`). **No changes needed** for `riff-backend`, `riff-gui`, `riff-infra`, `riff-library`, `riff-playback` path deps.

The **only** Cargo.toml path that changes is `tests/Cargo.toml` — currently `../riff-X` becomes `../crates/riff-X`.

---

## Phases

### Phase 1: Staging commit (safe, non-destructive)

| # | Action | Detail |
|---|--------|--------|
| 1 | Create `crates/` dir | `mkdir crates` |
| 2 | Move 6 crate directories | `git mv riff-persistence riff-playback riff-library riff-infra riff-backend riff-gui crates/` |
| 3 | Update root `Cargo.toml` members | Change every `"riff-X"` to `"crates/riff-X"` in `[workspace].members` |
| 4 | Update `tests/Cargo.toml` path deps | 5 paths: `../riff-backend` → `../crates/riff-backend`, etc. |
| 5 | Verify build | `cargo check --workspace` |
| 6 | Verify tests | `cargo test --all-targets` |
| 7 | Commit | `git commit -m "refactor: move crates into crates/ directory (conventional layout)"` |

**Validation**: `cargo check` + `cargo test` pass. No path breakage.

### Phase 2: Documentation updates

| # | File | Change |
|---|------|--------|
| 1 | `AGENTS.md` | Update all `riff-X/src/` path references to `crates/riff-X/src/` |
| 2 | `docs/technical/architecture.md` | Update crate path descriptions, the Overview table paths |
| 3 | `docs/engineering/coding-standards.md` | Update path references (`riff-backend/src/composition.rs` etc.) |
| 4 | `docs/engineering/contributing.md` | Update crate listing in the "source tree" bullet |
| 5 | `docs/adr/0009-vertical-crate-split-of-the-backend.md` | Update path references in the proposal and "As built" sections |
| 6 | `docs/engineering/testing-strategy.md` | Update `riff-infra/tests/` → `crates/riff-infra/tests/` reference |
| 7 | `docs/engineering/golden-image-testing.md` | Update `riff-gui/` path if present |
| 8 | `docs/README.md` | Update any `riff-X/` path references |
| 9 | Root `Cargo.toml` header comment | Update path list (lines 4-15) |

Commit: `git commit -m "docs: update paths for crates/ directory convention"`

### Phase 3: Graph rebuild + verification

| # | Action | Detail |
|---|--------|--------|
| 1 | Rebuild code-review-graph | Graph paths are stale after the move |
| 2 | Final verification | `cargo check --workspace && cargo test --all-targets` |

---

## Files touched

| File | Phase | Change type |
|------|-------|-------------|
| `Cargo.toml` | 1 | Edit `[workspace].members` + header comment |
| `tests/Cargo.toml` | 1 | Edit 5 `path = ` lines |
| `AGENTS.md` | 2 | Path references |
| `docs/technical/architecture.md` | 2 | Path references |
| `docs/engineering/coding-standards.md` | 2 | Path references |
| `docs/engineering/contributing.md` | 2 | Path references |
| `docs/adr/0009-vertical-crate-split-of-the-backend.md` | 2 | Path references |
| `docs/engineering/testing-strategy.md` | 2 | Path references |
| `docs/engineering/golden-image-testing.md` | 2 | Path references (if present) |
| `docs/README.md` | 2 | Path references |

**No source code changes.** Only directory moves, Cargo.toml path updates, and documentation edits.

## Risk

- **Low.** Pure structural move. Cargo resolves crate names from `[workspace].members`, not directory layout. All `use riff_backend::...` imports are crate-name-based, not path-based.
- `tests/Cargo.toml` is the only file with relative paths that break.
- `git mv` preserves history.
