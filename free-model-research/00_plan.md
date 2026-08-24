# Free Model Comparison & Role-Assignment Report

Generated: 2026-08-23
Scope: Free models across opencode "zen"/free tier + openrouter free + sensenova.
Target roles: default, smol, slow, vision, plan, designer, commit, tiny, task, advisor.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Setup & inventory | ✅ done |
| 2 | Research & source-gather | in progress |
| 3 | Scoring & role matrix | pending |
| 4 | Final recommendation | pending |

## Inventory

### opencode free whitelist (from opencode.jsonc)
- `opencode/big-pickle`
- `opencode/x-preview-f-free`
- `opencode/mimo-v2.5-free`
- `opencode/hy3-free`
- `opencode/nemotron-3-ultra-free`
- `opencode/nemotron-3.5-lightning-free`
- `opencode/muse-spark-1.2-contributor-free`

### sensenova (opencode config)
- `sensenova/sensenova-6.7-flash-lite`
- `sensenova/sensenova-u1-fast`

### openrouter free models
_(to be enumerated from API / online research)_

## Roles & requirements (from user)
| Role | Used for |
|------|----------|
| default | session, fallback |
| smol | fast/cheap prewalk, plan-yolo, tiny fallback |
| slow | thorough reasoning |
| vision | image inspection |
| plan | planning mode |
| designer | UI/design |
| commit | commit messages |
| tiny | session titles, memory, classification (locally downloaded preferred) |
| task | default worker subagent |
| advisor | runtime watchdog |
