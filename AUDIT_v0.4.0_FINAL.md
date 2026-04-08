# Sovereign-SDLC v0.4.0 — Final Technical Audit

**Date:** 2026-04-08  
**Auditor:** Claude Opus 4.6  
**Verdict:** Functional agent with tool system. 2 critical bugs remaining.

---

## Metrics

| Metric | Value |
|---|---|
| LOC | 8,809 |
| Source files | 30 |
| Crates | 6 |
| Tests | 124 (all passing) |
| Release binary | 4.5 MB |

---

## Architecture Summary

```
cli (main.rs)
 └─> tui (app.rs, buddy.rs, loading.rs, splash.rs, approval.rs)
      └─> query (coordinator.rs, router.rs, agent.rs, consensus.rs, compact.rs)
           ├─> api (client.rs) ──> Ollama
           ├─> tools (tool_trait.rs, tools_impl.rs, security.rs, report.rs)
           └─> core (hardware_env.rs, memdir.rs, grimoire.rs, history.rs,
                     system_prompt.rs, docs.rs, diff.rs, permissions.rs, model_db.rs)
```

---

## Module Status

### WORKING CORRECTLY

| Module | Status | Details |
|---|---|---|
| `api/client.rs` | OK | GenMetrics captures real eval_count, prompt_eval_count, total_duration_ns from Ollama |
| `core/hardware_env.rs` | OK | Apple Silicon uses unified_memory (not free RAM) for tier. CUDA/Vulkan detection. SafeLoad guard. |
| `core/memdir.rs` | OK | Vector store with cosine similarity. Zero-trust file filtering. UTF-8 safe chunking. |
| `core/grimoire.rs` | OK | SQLite security patterns. record_fix(), find_by_rule(), format_for_context(). |
| `core/history.rs` | OK | SQLite sessions with SHA-256 integrity verification. |
| `core/model_db.rs` | OK | 14 models with accurate RAM requirements. Onboarding messages. |
| `core/system_prompt.rs` | OK | Principal DevSecOps Architect identity. Tier-adaptive (compact for ExtraSmall). |
| `core/docs.rs` | OK | Project scanner, public API extraction, Mermaid architecture prompts. |
| `tools/tool_trait.rs` | OK | Tool trait, ToolRegistry, ToolCall parser (```tool, ```json, inline JSON). |
| `tools/tools_impl.rs` | OK | 5 tools: Bash, Read, Glob, Edit, Write. Permission levels assigned. |
| `tools/security.rs` | OK | Semgrep + cargo-audit + Clippy integration. |
| `tools/report.rs` | OK | Markdown compliance reports with SAST/SCA/Familiar/binary hash. |
| `tui/buddy.rs` | OK | 11 species, 5 rarities, 8 moods, XP system, sparkle eyes, persistence. |
| `tui/splash.rs` | OK | Block-letter banner. Sentinel bot with 7 mood-specific expressions. |
| `tui/approval.rs` | OK | Diff overlay with green +lines, red -lines. y/n/e/Esc controls. |
| `tui/loading.rs` | OK | Spinner animation. Loading states. Telemetry display. |
| `query/coordinator.rs` | OK | Model hopping. RAG + Grimoire context. Auto-detect installed models. |
| `query/router.rs` | OK | CODE/LOGIC/CHAT classification with heuristic fast-path. |
| `query/consensus.rs` | OK | Dual-inference Council (parallel on Medium+). |
| `query/compact.rs` | OK | Token estimation. Context compression at 85% threshold. |

### BUGS TO FIX

| # | Bug | Severity | Location |
|---|---|---|---|
| 1 | **Terminal not restored on unclean exit** — TUI chars leak into shell | CRITICAL | `tui/app.rs` — LeaveAlternateScreen not always called |
| 2 | **apply_diff_lines() ignores `-` deletions** — incorrect file edits | HIGH | `tui/app.rs:941` |
| 3 | Paste detection not reliable in crossterm | MEDIUM | `tui/app.rs` — timing-based detection misses some pastes |
| 4 | Explain button (e) not implemented | LOW | `tui/app.rs:326` |
| 5 | PermissionManager not wired to agent flow | LOW | `core/permissions.rs` — TUI has its own approval channel |
| 6 | GenTelemetry estimates tokens instead of using GenMetrics | LOW | `tui/loading.rs:126` |

### NOT YET IMPLEMENTED (v0.5.0 candidates)

| Feature | Priority | Description |
|---|---|---|
| Streaming generation | HIGH | Show tokens as they arrive instead of waiting for full response |
| Terminal restoration | HIGH | Ensure clean exit on panic, Ctrl+C during generation, agent errors |
| Multi-file edits | MEDIUM | Agent currently handles one file per action |
| Web documentation access | MEDIUM | Fetch crate docs, MDN, etc. for context |
| Auto-tool discovery | MEDIUM | Detect Dockerfile → suggest hadolint, package.json → eslint |
| Multi-step planning | MEDIUM | Show plan before executing steps |
| Git integration | MEDIUM | /commit, /diff, branch awareness |
| Persistent permissions | LOW | Save AllowAlways decisions to .sovereign/permissions.json |
| Plugin system | LOW | Custom tool loading |

---

## Tool System (claurst-style)

### Tools Available

| Tool | Permission | Auto-approved |
|---|---|---|
| `bash` | Execute | No — needs y/n |
| `read` | ReadOnly | Yes — auto-approved |
| `glob` | ReadOnly | Yes — auto-approved |
| `edit` | Write | No — needs y/n |
| `write` | Write | No — needs y/n |

### Agent Flow

```
User prompt → needs_agent() detects keywords
  → Spawn async agent loop with tool registry
  → LLM gets tool descriptions in system prompt
  → LLM outputs ```tool JSON block
  → parse_tool_call() extracts ToolCall
  → ReadOnly? auto-execute : show approval overlay
  → User y/n → execute tool → feed result to LLM
  → LLM responds (maybe with another tool call)
  → Max 8 turns → final response
```

### Permission Flow

```
                    ┌─────────────┐
                    │ Tool Called  │
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │ ReadOnly?   │──yes──> Auto-execute
                    └──────┬──────┘
                           │ no
                    ┌──────▼──────┐
                    │ TUI Overlay │
                    │ (y) (n) (e) │
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
          Approved      Denied      Explain
              │            │         (TODO)
         Execute       "Denied"
              │
         Feed result
         back to LLM
```

---

## Performance on Apple Silicon M5

| Aspect | Behavior |
|---|---|
| Tier detection | HighEnd (24GB - 4GB buffer = 20GB effective) |
| TUI refresh | 80ms poll (12.5 FPS) |
| Animation | Buddy + Sentinel tick every 80ms independently |
| Generation | tokio::spawn — UI never freezes |
| Agent loop | Background task with mpsc channels |
| /index batch | 16 concurrent embeddings |

---

## Persistence Map

```
.sovereign/
├── index.bin        Vector store (bincode, embeddings + chunks)
├── buddy.json       Companion (species, rarity, XP, stats)
├── grimoire.db      Security patterns (SQLite)
└── history.db       Session chronicle (SQLite, SHA-256 signed)
```

---

## Recommendation for v0.5.0

**Priority 1:** Fix terminal restoration (prevents the bash leak bug)  
**Priority 2:** Streaming generation (biggest UX improvement)  
**Priority 3:** Web documentation access (biggest capability improvement)

*Audit complete. Architecture is solid. Tool system operational. 2 critical bugs to fix.*
