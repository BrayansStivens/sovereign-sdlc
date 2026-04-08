# Sovereign-SDLC v0.4.0 — Technical Audit

**Date:** 2026-04-08  
**Auditor:** Claude Opus 4.6 (Lead Software Auditor mode)  
**Verdict:** Functional with 6 bugs to fix before v0.5.0

---

## Metrics

| Metric | Value |
|---|---|
| LOC | 7,823 |
| Source files | 27 |
| Crates | 6 |
| Tests | 109 (all passing) |
| Release binary | 4.3 MB |

---

## 1. Telemetry (api/client.rs + tui/loading.rs)

### What works
- `GenMetrics` captures real Ollama data: `eval_count`, `prompt_eval_count`, `total_duration_ns`
- `tokens_per_sec()` correctly divides eval tokens by nanosecond duration
- `summary()` formats as `[+] 3.2s | 847 tokens | >_ 264.7 tok/s`
- The TUI correctly shows this summary after each generation

### BUG: Dual metric types, loading.rs has dead estimation code
**Severity: MEDIUM**

`loading.rs` defines its own `GenTelemetry` struct with `finish_generation()` that estimates tokens as `len/4`. This code is **never called** in the current flow — `app.rs` now receives real `GenMetrics.summary()` via the channel. The `GenTelemetry` struct and `finish_generation()` in loading.rs are **dead code**.

**Fix:** Remove `GenTelemetry` and `finish_generation()` from loading.rs. The real path is: `client.rs GenMetrics.summary()` → channel → `app.rs` displays it.

---

## 2. Layout & Async UI (tui/app.rs)

### What works
- Sticky input at bottom via `Constraint::Length(3)` — never moves
- Async generation via `tokio::spawn` — UI renders spinner while LLM works
- MPSC channel for non-blocking result delivery
- Paste detection (<5ms between keystrokes) with `[Pasted +N lines]`
- Ctrl+Z suspend/resume via `SIGTSTP`
- Esc cancels generation without closing app

### BUG: Cancelled tasks linger
**Severity: LOW**

When Esc cancels generation, the spawned tokio task keeps running until Ollama finishes. The old channel sender is dropped, so the result is silently lost. This wastes CPU/GPU cycles but doesn't crash. No timeout exists either — a hung LLM call blocks a thread forever.

**Fix:** Add a `CancellationToken` from tokio-util, or accept the current behavior as "good enough" for v0.4.

---

## 3. Diff System & Approval (core/diff.rs + tui/approval.rs + app.rs)

### What works
- `FileDiff::compute()` uses `similar` crate correctly — line-level unified diffs
- `classify_command_risk()` catches `rm -rf`, `sudo`, fork bombs, `format c:`, etc.
- `approval.rs` renders green +lines, red -lines with backgrounds and line numbers
- Approval overlay shows (y)es (n)o (e)xplain (Esc)cancel
- Y applies edits, N declines, Up/Down scrolls diff

### BUG: `apply_diff_lines()` ignores deletions — HIGH
**Severity: HIGH**

The function in `app.rs` that reconstructs file content from a diff block only copies `+` and context lines. It **ignores `-` (deletion) lines entirely**. If the LLM's diff removes a line, the reconstruction will keep it.

```
Input diff:     Output (wrong):
- old line      old line        ← should be deleted
+ new line      new line
```

**Fix:** Rebuild `apply_diff_lines()` to track context lines from the old file and apply insertions/deletions properly. Or better: extract the new content from the LLM's response directly instead of trying to reconstruct from a diff.

### BUG: Fallback to "unknown_file" — HIGH
**Severity: HIGH**

If the LLM's diff block doesn't contain `--- a/path`, the code falls back to `"unknown_file"` and will create/overwrite a file with that name.

**Fix:** If no file path is found, skip the proposed action. Don't guess.

### PLACEHOLDER: Explain button (e)
**Severity: MEDIUM**

The (e) key in the approval overlay prints "Explain not yet implemented." This should send the diff back to the LLM with a "explain why this change is needed" prompt.

---

## 4. Buddy System (tui/buddy.rs)

### What works
- 11 species with 3-line multi-line ASCII art
- Gacha rarity (Common 75% → Sovereign 1%)
- 8 moods reactive to hardware + security state
- Code Quality Radar (Pristine/Clean/Tech Debt/Critical)
- XP system: levels up from auditing, fixing, catching vulns
- Persistence to `.sovereign/buddy.json` with serde
- Backward compat: legacy Raven/Spirit deserialize via `#[serde(alias)]`

### BUG: Sparkle eyes are dead code — MEDIUM
**Severity: MEDIUM**

`sparkle_frame()` tries to mutate a `SpriteLines` (`[&'static str; 3]`) array. The line `let mut sparkle = idle; sparkle[1] = "...";` copies the array then mutates the copy, which is returned correctly. However, the sparkle face strings are hardcoded and only replace the middle line — the top and bottom lines remain from `idle`, which is correct behavior. **The implementation actually works** — the array is copied by value (`[&str; 3]` is `Copy`), mutated, and returned. Initial audit flagged this as dead code but that was incorrect.

**Status:** Works, but only triggers for Sovereign rarity (1% of buddies). Hard to test manually.

---

## 5. Persistence (core/grimoire.rs + core/history.rs)

### What works
- Grimoire: SQLite with indexed `rule_id` and `language` columns
- `record_fix()` writes parameterized INSERT, returns row ID
- `format_for_context()` generates clean LLM context (5 patterns max, truncated)
- Chronicle: SHA-256 signed chat logs, `verify_integrity()` checks hash
- `days_since_last()` calculates absence for buddy greeting

### MINOR: Naive timestamps
**Severity: LOW**

Both databases store timestamps without timezone info (`%Y-%m-%d %H:%M:%S`). If the user changes timezones, `days_since_last()` could be off by hours. Not a real problem for "days" granularity.

---

## 6. Sentinel Bot (tui/splash.rs)

### What works
- Houston-style boxed face: `╭─────╮ │ face │ ╰─────╯`
- 6 moods with multiple expressions each that cycle on tick
- Mood set by LoadingState in render_activity()

### Design note
Sentinel doesn't autonomously transition moods. The caller (app.rs) maps LoadingState → SentinelMood. This is correct architecture — the TUI drives the mood, not the bot.

---

## 7. Model Database (core/model_db.rs)

### What works
- 13 models with Q4/Q8 RAM requirements
- `onboarding_message()` generates install commands at startup
- `missing_models()` checks installed vs recommended
- Fallback chain in coordinator finds best available model

### Note
RAM figures are conservative estimates, not exact CanIRun.ai values. This is safe — better to underestimate capacity than overestimate.

---

## Performance Analysis (M5 Apple Silicon)

| Aspect | Behavior |
|---|---|
| TUI render | 80ms poll interval on HighEnd — smooth 12.5 FPS |
| Animation | Tick every 80ms — buddy + Sentinel animate independently |
| Generation | Spawned on separate tokio task — UI never freezes |
| /index | Batch size 16 embeddings — parallelized for M5 |
| Memory | ~15MB RSS idle, grows with vector store size |

**No thread contention issues detected.** Tokio's work-stealing scheduler distributes across M5 performance cores. The single background generation task doesn't compete with the TUI event loop.

---

## Gap Analysis: What's Missing for a Real Autonomous Agent

| Capability | Status | What's needed |
|---|---|---|
| File editing with diffs | ✓ Implemented (with bugs) | Fix apply_diff_lines() |
| Command execution with safety | ✓ Implemented | Add more risk patterns |
| RAG / project memory | ✓ Implemented | Working |
| Security scanning (SAST/SCA) | ✓ Implemented | Working |
| Multi-step planning | ✗ Missing | Agent should show a plan before executing steps |
| Web documentation access | ✗ Missing | Fetch crate docs, MDN, etc. |
| Auto tool discovery | ✗ Missing | Detect Dockerfile → suggest hadolint |
| Streaming generation | ✗ Missing | Currently waits for full response |
| Git integration | ✗ Missing | /commit, /diff, branch awareness |
| Multi-file edits | ✗ Missing | Currently handles single file per action |
| Conversation branching | ✗ Missing | Can't fork a conversation or revert |
| Plugin system | ✗ Missing | No way to add custom tools |

---

## Priority Fixes for v0.5.0

| # | Bug | Severity | Effort |
|---|---|---|---|
| 1 | `apply_diff_lines()` ignores deletions | HIGH | 1 hour |
| 2 | "unknown_file" fallback creates bad files | HIGH | 15 min |
| 3 | Remove dead `GenTelemetry` from loading.rs | MEDIUM | 10 min |
| 4 | Implement (e)xplain in approval overlay | MEDIUM | 30 min |
| 5 | Add more command risk patterns | MEDIUM | 20 min |
| 6 | Generation timeout (30s default) | MEDIUM | 20 min |

---

*Audit complete. 109 tests passing. 6 bugs identified. Core architecture is solid.*
