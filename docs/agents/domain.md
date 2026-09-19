# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root — the glossary of the music-collection domain.
- **`docs/adr/`** — read ADRs that touch the area you're about to work in.
- **`docs/product/decisions/`** — product-level decisions (offline-first, track identity, platform splits) that constrain technical ones.

If any of these files don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms or decisions actually get resolved.

## File structure

Single-context repo — one glossary and one ADR record at the root, shared by all six crates:

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-sqlite-is-the-authoritative-application-store.md
│   └── 0009-vertical-crate-split-of-the-backend.md
└── crates/<crate>/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal — either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/domain-modeling`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (no write-side SessionStore facade) — but worth reopening because…_
