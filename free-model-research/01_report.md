# Free-Model Role-Assignment Report — OpenCode / OpenRouter / SenseNova

Generated: 2026-08-23
Purpose: pick the best free (or near-free) model for each OpenCode `ai agent` role.

---

## 1. Methodology

**Scoring dimensions (1–5 per role):**
- `general` — general-purpose reasoning / chat quality
- `coding` — code generation, debugging, long files
- `agentic` — multi-step tool use, follow instructions, subagent use
- `speed` — latency + tokens/sec (matters for smol/tiny/advisor)
- `vision` — native image understanding
- `context` — long-context ceiling

**Caveats used throughout:**
- OpenCode Zen free models are `stealth` — underlying provider can swap under the same alias without notice. Long-term stability is a risk.
- OpenRouter `:free` caps at 20 req/min; 50 req/day (or 1,000 if you’ve ever bought $10 of credits). Not viable for high-volume `tiny`/`advisor` backgrounds.
- SenseNova 6.7 Flash Lite is the model currently wired into this config; the marketing page now refers to the newer **SenseNova 6.8 Flash Lite**. Benchmarks for 6.8 are claimed improvements over 6.7 (agentic +15–20%); specific public numeric benchmarks for 6.7/6.8 are thin, so scores reflect marketing claims + qualitative reports.
- All scores are **relative** within this free-model pool, not absolute frontier comparisons.

---

## 2. Inventory of Free / Candidate Models

### A. OpenCode Zen free whitelist (in `opencode.jsonc`)

| Model ID (opencode/<id>)              | Alias / origin                                | Cost   | Notes                                                                                          |
|---------------------------------------|-----------------------------------------------|--------|------------------------------------------------------------------------------------------------|
| `opencode/big-pickle`                 | Stealth "reasoning" model                     | Free   | Text only. Self-reported SWE-Atlas Q&A **50.8%** (Aug 2026, single trial, unverified).         |
| `opencode/x-preview-f-free`           | Stealth "Ox Alpha" (OpenRouter)               | Free   | 1M ctx, multimodal, 128K output. Described as frontier-class reasoning + coding.               |
| `opencode/mimo-v2.5-free`            | Xiaomi MiMo-V2.5                              | Free   | 310B params, 1.1M ctx, multimodal. Intelligence Index **38.0** (55/173). Coding 56.8.          |
| `opencode/hy3-free`                   | Tencent Hunyuan Hy3                           | Free   | Open-weights reasoning model; agent-oriented training.                                          |
| `opencode/nemotron-3-ultra-free`      | NVIDIA Nemotron 3 Ultra                       | Free   | ~1M ctx, long-document agentic work.                                                            |
| `opencode/nemotron-3.5-lightning-free`| NVIDIA Nemotron 3.5 Lightning (31.6B/3.6B)    | Free   | Intelligence Index **24**, very fast (~670 tok/s). Top small-model choice.                      |
| `opencode/muse-spark-1.2-contributor-free`| Meta Muse Spark 1.2 (Contributor)            | Free   | Frontier-tier agent (Intelligence Index **54**). Top agentic in this pool.                      |

### B. SenseNova (user’s active provider)

| Model ID                                | Display name           | Cost     | Notes                                                                             |
|-----------------------------------------|------------------------|----------|-----------------------------------------------------------------------------------|
| `sensenova/sensenova-6.7-flash-lite`    | SenseNova 6.7 Flash    | Per-token| Lightweight multimodal agent, "60% fewer tokens vs text-only." Configured as default.|
| `sensenova/sensenova-u1-fast`           | SenseNova U1 Fast      | Per-token| Multimodal U1 derived; strong generation/infographic/PPT. Not a reasoning specialist.|

### C. OpenRouter `:free` (not currently whitelisted but available if you add `openrouter` provider)

Ranked by Artificial Analysis Intelligence Index (where public data exists) / qualitative usage ranking per OpenRouter July 2026:

| Model                              | Provider    | II    | Context | Notes                                              |
|------------------------------------|-------------|-------|---------|----------------------------------------------------|
| `openrouter/xai/grok-4-0:free`*    | xAI         | ~38   | 128K    | Strongest general on OpenRouter free (subject to rotation).|
| `openrouter/xiaomi/mimo-v2.5:free` | Xiaomi      | 38.0  | 1.1M    | Same model as opencode/mimo-v2.5-free.             |
| `openrouter/nvidia/nemotron-3-5-lightning:free` | NVIDIA | 24 | 1M | Same model as opencode/nemotron-3.5-lightning-free.|
| `openrouter/google/gemma-4-31b:free`| Google     | ~30   | 262K    | Multilingual general.                              |
| `openrouter/deepseek-v4-flash:free` | DeepSeek    | ~40   | 128K    | Reasoning + coding. *(availability confirmed in July 2026)* |
| `openrouter/meta/muse-spark-1.2:free` | Meta     | 54    | 1M      | Same as contributor tier.                          |
| `openrouter/qwen/qwen3.5-plus:free`  | Alibaba     | ~35   | 128K    | Reasoning + coding.                                |

*DeepSeek, Gemini, and Mistral free IDs rotate frequently — verify on openrouter.ai/models before hardcoding.

---

## 3. Capability Matrix (relative scores, 1–5)

Scoring key: `gen | cod | age | spd | vis | ctx`. Totals shown; tiebreaker is coding + agentic for coding-agent roles.

| Model                                              | gen | cod | age | spd | vis | ctx | Σ    |
|----------------------------------------------------|-----|-----|-----|-----|-----|-----|------|
| Muse Spark 1.2 (Contributor free)                  | 5   | 5   | 5   | 3   | 3   | 5   | **26** |
| Big Pickle (stealth)                               | 4   | 4   | 4   | 3   | 1   | 3   | **19** |
| MiMo-V2.5 free                                     | 4   | 4   | 4   | 3   | 5   | 5   | **25** |
| Ox Alpha (`x-preview-f-free`, stealth)             | 4   | 5   | 4   | 3   | 5   | 5   | **26** |
| Nemotron 3.5 Lightning free                        | 3   | 3   | 3   | 5   | 1   | 5   | **20** |
| Nemotron 3 Ultra free                              | 3   | 3   | 3   | 3   | 1   | 5   | **18** |
| Hy3 free                                           | 3   | 3   | 3   | 3   | 3   | 4   | **19** |
| SenseNova 6.7 Flash Lite (per-token)               | 4   | 3   | 4   | 3   | 5   | 4   | **23** |
| SenseNova U1 Fast                                  | 3   | 2   | 3   | 3   | 5   | 4   | **20** |
| gpt-oss-120b (if available free on OR)             | 4   | 4   | 3   | 3   | 1   | 3   | **18** |

Notes:
- MiMo-V2.5 has the best **verified public benchmark** (II 38, Coding Index 56.8, Tau² agentic 91%, GPQA Diamond 85%). Highest-evidence entry in the free pool.
- Muse Spark 1.2 is the strongest on **agentic/knowledge work** (II 54, GDPval #5, Terminal-Bench 80%) but it's not always stable on OpenRouter free; the `Contributor` tier on OpenCode Zen is your safest bet.
- Big Pickle's SWE-Atlas 50.8% is a strong **coding** signal, but it's a self-reported single-trial run with an unconfirmed underlying model.
- Nemotron 3.5 Lightning is the clear **speed** leader (~670 tok/s), making it ideal for high-volume roles.

---

## 4. Recommended Assignments per OpenCode Role

### Final picks (primary → fallback → tertiary)

| Role      | Primary                                | Fallback                           | Tertiary                           | Rationale                                                                                                     |
|-----------|----------------------------------------|------------------------------------|------------------------------------|---------------------------------------------------------------------------------------------------------------|
| `default` | Muse Spark 1.2 (Contributor free)      | MiMo-V2.5 free                     | Big Pickle                         | Default needs top general quality. Muse Spark is the highest-verified free agent; MiMo is the best benchmarked alternative. |
| `task`    | Muse Spark 1.2 (Contributor free)      | Big Pickle                         | MiMo-V2.5 free                     | Task subagents run real code edits → needs strong coding + agentic. Big Pickle is the empirical coding leader among free stealth models. |
| `slow`    | Muse Spark 1.2 (Contributor free)      | Big Pickle                         | Ox Alpha (`x-preview-f-free`)      | Thorough reasoning favors large context + strong reasoning. Muse Spark II 54 > all others.                    |
| `smol`    | Nemotron 3.5 Lightning free            | Nemotron 3 Ultra free              | Hy3 free                           | Speed is the point. Nemotron 3.5 Lightning = ~670 tok/s at Intelligence 24 — fastest verified in the pool.     |
| `plan`    | Big Pickle                             | MiMo-V2.5 free                     | Muse Spark 1.2                     | Planning needs coherent, long-form reasoning; Big Pickle's 200K ctx + verified SWE reasoning makes it strong. |
| `designer`| MiMo-V2.5 free                         | SenseNova U1 Fast                  | SenseNova 6.7 Flash Lite           | Vision + design generation. MiMo-V2.5 has native multimodal + Design Arena Elo 1288 (competitive); SenseNova U1 is tuned for infographic/PPT layouts. |
| `vision`  | MiMo-V2.5 free                         | SenseNova U1 Fast                  | Ox Alpha (`x-preview-f-free`)      | Only these free models accept native image input with strong vision.                                            |
| `commit`  | Nemotron 3.5 Lightning free            | Nemotron 3 Ultra free              | Hy3 free                           | Commit messages are short, high-volume, cheap. Speed matters; quality floor is low. Nemotron 3.5 Lightning is fastest. |
| `tiny`    | **Local tiny models (`omp tiny-models`)** | Nemotron 3.5 Lightning free    | Nemotron 3 Ultra free              | `tiny` is background-only and **designed to run locally**. Only use remote free models as fallback.            |
| `advisor` | Nemotron 3.5 Lightning free            | Nemotron 3 Ultra free              | Hy3 free                           | Advisor is a watchdog — low complexity, high frequency. Speed + stability beat raw intelligence.                |

### Provider-availability reminder

`tiny` in the roles table explicitly refers to the locally downloaded `omp tiny-models` (GPT-2-class, ~1–2B params) — that is the correct deployment for that role by design, and doesn't depend on any of these free tiers.

---

## 5. Sensible Config Skeleton

Below is a copy-ready fragment for `opencode.jsonc` → `"provider"` / top-level model config. Add only the roles you want to override; unset roles fall back to `default`.

```jsonc
{
  // Top-level (default = session + fallback)
  "model": "opencode/muse-spark-1.2-contributor-free",

  "role": {
    "smol":    "opencode/nemotron-3.5-lightning-free",
    "slow":    "opencode/muse-spark-1.2-contributor-free",
    "plan":    "opencode/big-pickle",
    "task":    "opencode/muse-spark-1.2-contributor-free",
    "designer":"opencode/mimo-v2.5-free",
    "vision":  "opencode/mimo-v2.5-free",
    "commit":  "opencode/nemotron-3.5-lightning-free",
    "tiny":    "sensenova/sensenova-6.7-flash-lite",   // OR local `omp tiny-models`
    "advisor": "opencode/nemotron-3.5-lightning-free"
  }
}
```

If you want to keep SenseNova as `default` (it's what you already have) and only override the cheaper/slower roles, swap `default` back to `sensenova/sensenova-6.7-flash-lite` — it's a perfectly competent default for interactive sessions and is faster to reason about because it's a paid-tier model you control.

---

## 6. Risks & Warnings

1. **Stealth alias drift.** `big-pickle` and `x-preview-f-free` are unnamed underlying models. The provider can swap the alias at any time. Any long-running automation that hardcodes them should add retry/fallback logic.
2. **OpenRouter rate limits.** 20 req/min and 50–1000 req/day. Do NOT route `tiny` or `advisor` through OpenRouter — you will hit the daily cap in an afternoon.
3. **Opencode Zen `free` window.** Every model in your whitelist is labelled "available for a limited time" except Ox Alpha (zero-retention policy). They can be pulled without notice.
4. **Benchmark uncertainty.** Big Pickle's 50.8% SWE-Atlas is a single self-reported trial. Treat it as a strong signal, not a certified score.
5. **Nemotron 3.5 Lightning vs Nemotron 3 Ultra.** Both are NVIDIA; Lightning is the newer, faster, smaller (31.6B/3.6B active) model. Prefer Lightning for `smol`/`commit`/`advisor`; use Ultra only if you specifically need the larger context window of Nemotron 3 Ultra's variant.

---

## 7. Sources

- OpenCode Zen docs: <https://opencode.ai/docs/zen/>
- Big Pickle spec (models.dev): <https://github.com/sst/models.dev/blob/dev/providers/opencode/models/big-pickle.toml>
- Big Pickle SWE-Atlas 50.8% (Aug 2026, single trial): <https://news.lavx.hu/article/big-pickle-posts-50-8-on-swe-atlas-codebase-q-a-benchmark>
- MiMo-V2.5 benchmarks (modelgrep, Intelligence 38.0, Coding 56.8, Tau² 91%): <https://modelgrep.com/models/xiaomi/mimo-v2.5>
- Muse Spark 1.2 (Artificial Analysis, II 54, GDPval #5, Terminal-Bench 80%): <https://artificialanalysis.ai/articles/muse-spark-1-2>
- Nemotron 3.5 Lightning launch (Artificial Analysis, II 24, ~670 tok/s): <https://artificialanalysis.ai/articles/nemotron-3-5-lightning-launch>
- Ox Alpha spec (1M ctx, multimodal, free on OpenRouter): <https://oxalpha.io/>
- SenseNova native multimodal agent models: <https://www.sensenova.cn/en/models>
- OpenRouter free-model ranking (July 2026, buldrr): <https://buldrr.com/openrouter-free-models-list-2026-all-27-models-ranked-tested/>
- OpenRouter free-models collection: <https://openrouter.ai/collections/free-models>
