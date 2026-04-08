# Sovereign-SDLC v0.2.0 — Technical Snapshot

**Date:** 2026-04-07  
**Auditor:** Claude Opus 4.6 (System Audit Mode)  
**Binary:** 2.7 MB (release, LTO fat, stripped)  
**Codebase:** 5,178 LOC | 20 source files | 68 tests (all passing)

---

## 1. Workspace Structure

```
sovereign-sdlc/
├── Cargo.toml               Workspace root (resolver = "3", edition 2024)
├── crates/
│   ├── core/   (2,388 LOC)  Foundation — hardware, vector store, grimoire, sessions
│   ├── api/       (66 LOC)  Ollama client wrapper (generate, embed, list_models)
│   ├── tools/    (860 LOC)  Security pipeline (SAST/SCA) + compliance reports
│   ├── query/  (1,280 LOC)  Agent orchestrator (router, ReAct, coordinator, consensus, compact)
│   ├── tui/      (965 LOC)  ratatui 3-panel interface + buddy system
│   └── cli/      (103 LOC)  Binary entry point (TUI default, --repl fallback)
```

### Dependency Graph

```
cli ──→ tui ──→ query ──→ api ──→ core
                  │         │
                  ├──→ tools ──→ core
                  └──→ core
```

### Crate Responsibilities

| Crate | Primary Responsibility | Key Modules |
|---|---|---|
| `sovereign-core` | Hardware detection, vector store (RAG), SQLite DBs (grimoire, history) | `hardware_env.rs`, `memdir.rs`, `grimoire.rs`, `history.rs` |
| `sovereign-api` | Ollama communication (text generation + embeddings) | `client.rs` |
| `sovereign-tools` | Security scanning orchestration + Markdown report generation | `security.rs`, `report.rs` |
| `sovereign-query` | Agent brain: routing, ReAct loop, model hopping, dual-inference, context compression | `router.rs`, `agent.rs`, `coordinator.rs`, `consensus.rs`, `compact.rs` |
| `sovereign-tui` | Terminal interface (3-panel) + RPG buddy system | `app.rs`, `buddy.rs` |
| `sovereign-cli` | Binary entry point, REPL mode | `main.rs` |

---

## 2. Dependency Analysis (Workspace Root Cargo.toml)

### External Dependencies

| Category | Crate | Version | Features | Critical For |
|---|---|---|---|---|
| Async | `tokio` | 1 | `full` | Runtime for all async operations |
| Async | `tokio-stream` | 0.1 | — | Streaming for Ollama responses |
| Async | `async-trait` | 0.1 | — | SecurityTool trait |
| TUI | `ratatui` | 0.29 | — | Terminal UI rendering |
| TUI | `crossterm` | 0.28 | — | Terminal event handling |
| LLM | `ollama-rs` | 0.2 | `stream` | Ollama API (generation + embeddings) |
| Hardware | `sysinfo` | 0.33 | — | CPU/RAM/process monitoring |
| **SQLite** | **`rusqlite`** | **0.32** | **`bundled`** | **Grimoire + Chronicle (bundled = no system libsqlite)** |
| **Embeddings** | **`bincode`** | **1** | — | **Vector store serialization** |
| **Crypto** | **`sha2`** | **0.10** | — | **Session integrity hashing** |
| Crypto | `hex` | 0.4 | — | Hash encoding |
| Serialization | `serde` | 1 | `derive` | All data structures |
| Serialization | `serde_json` | 1 | — | JSON for Semgrep, buddy, sessions |
| Serialization | `toml` | 0.8 | — | Config (declared, not yet used) |
| Filesystem | `walkdir` | 2 | — | Project scanning for RAG indexing |
| Logging | `tracing` | 0.1 | — | Structured logging |
| Logging | `tracing-subscriber` | 0.3 | `env-filter` | Log output |
| Error | `anyhow` | 1 | — | Error handling |
| Error | `thiserror` | 2 | — | Typed errors (declared, not yet used) |
| Time | `chrono` | 0.4 | — | Timestamps for sessions, grimoire |

### Platform-Specific Dependencies

| OS | Crate | Version | Purpose |
|---|---|---|---|
| macOS | `core-foundation` | 0.10 | Apple platform APIs (declared, not actively used) |
| Linux | `libloading` | 0.8 | Dynamic loading of NVML/Vulkan |
| Windows | `libloading` | 0.8 | Dynamic loading of NVML/Vulkan |

### Release Profile

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
```

---

## 3. Hardware Detection Logic (`core/hardware_env.rs`)

### Platform Enum

```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum Platform {
    AppleSilicon {
        chip: String,              // "M1" through "M5"
        unified_memory_gb: u64,
        gpu_cores: u32,
        perf_cores: u32,
        efficiency_cores: u32,
    },
    CudaGpu {
        device_name: String,       // e.g., "NVIDIA RTX 4090"
        vram_gb: f64,
        compute_capability: String,
    },
    VulkanGpu {
        device_name: String,
        vram_gb: f64,
    },
    CpuOnly {
        cpu_name: String,
        cores: usize,
        threads: usize,
    },
}
```

### Performance Tier Classification

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum PerformanceTier {
    ExtraSmall,  // <8 GB effective — 1B-3B models only
    Small,       // 8-12 GB — up to 7B models
    Medium,      // 12-20 GB — up to 14B models
    HighEnd,     // 20+ GB — 14B+, dual-model orchestration
}
```

**Tier Classification Formula:**
```rust
fn classify_tier(platform: &Platform, available_ram_gb: f64) -> PerformanceTier {
    let usable = available_ram_gb - 4.0; // SAFE_LOAD_BUFFER_GB

    let gpu_bonus = match platform {
        CudaGpu { vram_gb, .. }  => *vram_gb,          // Full VRAM
        VulkanGpu { vram_gb, .. } => *vram_gb * 0.7,    // 70% efficiency
        AppleSilicon { .. }       => 0.0,               // Unified = already counted
        CpuOnly { .. }            => 0.0,
    };

    let effective = usable + gpu_bonus;
    // effective >= 20 → HighEnd, >= 12 → Medium, >= 8 → Small, else ExtraSmall
}
```

### SafeLoad Guard

```rust
pub enum SafeLoadResult {
    Safe { model, required_gb, available_gb },
    Warning { model, required_gb, available_gb, message },
    Blocked { model, required_gb, available_gb, suggestion: ModelWeight },
}
```

**Rule:** `(ModelWeight + 4GB buffer) > RAM disponible → BLOCKED`

### Model Weight Formula

```rust
// Required_RAM = (Params_B * Quant_Bits / 8) + KV_Cache_Buffer(1.5GB)
pub fn calculate(name: &str, params_b: f64, quant: f64) -> ModelWeight
```

Quant detection from model tag: Q2=2, Q3=3, Q4=4 (default), Q5=5, Q6=6, Q8=8, FP16=16, FP32=32.

### Model Recommendations by Tier

| Tier | Dev Model | Audit Model | Router | Max Context |
|---|---|---|---|---|
| HighEnd | `qwen2.5-coder:14b-q8_0` | `deepseek-r1:14b` | `qwen2.5:7b` | 32,768 |
| Medium | `qwen2.5-coder:7b` | `deepseek-r1:7b` | `llama3.2:1b` | 16,384 |
| Small | `qwen2.5-coder:3b` | `phi-4:mini` | `llama3.2:1b` | 8,192 |
| ExtraSmall | `llama3.2:3b` | `phi-4:mini` | `llama3.2:1b` | 4,096 |

### Platform Detection (`cfg` Flags)

| Platform | Detection Method | Key Code |
|---|---|---|
| **macOS** | `#[cfg(target_os = "macos")]` — CPU brand string contains "apple" → parse M1-M5 | `estimate_apple_silicon_layout()` returns (perf_cores, eff_cores, gpu_cores) |
| **Linux/Windows + CUDA** | `#[cfg(any(target_os = "linux", target_os = "windows"))]` — dynamically load `libnvidia-ml.so.1` / `nvml.dll` via `libloading` | Unsafe FFI: `nvmlInit_v2`, `nvmlDeviceGetCount_v2`, `nvmlDeviceGetName`, `nvmlDeviceGetMemoryInfo` |
| **Linux/Windows + Vulkan** | Same cfg — fallback: dynamically load `libvulkan.so.1` / `vulkan-1.dll` | Detection only (no device enumeration) |
| **CPU Only** | All cfg fallback | Assumes hyperthreading: `threads = cores * 2` |

---

## 4. Orchestrator State (`query/coordinator.rs`)

### Coordinator Struct

```rust
pub struct Coordinator {
    pub client: OllamaClient,
    pub router: SmartRouter,
    pub hw: HardwareEnv,
    pub recommendation: ModelRecommendation,
    pub force_model: Option<String>,       // User override via /model
    pub security_mode: bool,               // OWASP prompt injection
    pub memory: VectorStore,               // RAG vector store
    pub project_root: PathBuf,
    pub rag_enabled: bool,                 // Auto-enabled when index > 0 chunks
}
```

### Generation Flow (RAG + Security)

```
User prompt
    │
    ▼
┌─────────────────────────┐
│ 1. SECURITY_SYSTEM_PROMPT│ ← OWASP ASVS injection (if security_mode=true)
│    (always prepended)    │
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│ 2. RAG Context Retrieval │ ← embed(query) → cosine search top-5
│    (if rag_enabled)      │    from VectorStore
│                          │    Format: "[Contexto Local Relevante (Auditado)]"
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│ 3. "User request: ..."  │ ← Actual user prompt
└────────────┬────────────┘
             │
             ▼
       OllamaClient.generate(model, full_prompt)
```

### Model Hopping — YES, Implemented

The `route_prompt()` method:
1. If `force_model` is set → use it for everything
2. Else → `SmartRouter.route(prompt)` classifies as CODE/LOGIC/CHAT
3. Category maps to tier-recommended model:
   - CODE → `recommendation.dev_model`
   - LOGIC → `recommendation.audit_model`
   - CHAT → `recommendation.dev_model`

### RAG — YES, Implemented

- `/index [path]` scans project via `scan_project()` (Zero-Trust filtered)
- Chunks text (2048 chars, 256 overlap)
- Embeds via `nomic-embed-text` (768 dims, f32)
- Stores in `VectorStore` (bincode persistence → `.sovereign/index.bin`)
- Before each `generate()`: `retrieve_context()` does cosine similarity search, top-5
- Hardware-adaptive batch sizes: HighEnd=16, Medium=8, Small=4, ExtraSmall=1

### Validated Generation Pipeline

```
generate(prompt)
    │
    ▼
response contains "```" ?
    │ yes                    │ no
    ▼                        ▼
Audit Model review       Return as-is
(deepseek-r1)
    │
    ▼
ValidatedResponse {
    original,
    audit_review: Some(...),
    passed_validation: true
}
```

### ReAct Agent Loop (`agent.rs`)

```
for iteration in 0..8 {
    LLM(context) → parse Thought + Action
    
    match Action {
        ReadFile(path) → fs::read_to_string → Observation
        Execute(cmd)   → needs user approval [y/N] → sh -c → Observation
        Respond(text)  → return final answer
    }
    
    context += Observation
}
```

### Dual-Inference Council (`consensus.rs`)

```rust
pub enum ConsensusLevel { Aligned, MinorDivergence, Conflicted }
```

- **Medium+ tier:** parallel inference via `tokio::join!(dev_model, audit_model)`
- **Lower tiers:** sequential (dev first, then audit reviews dev's output)
- Conflict detection: counts "vulnerability", "insecure", "dangerous" signals in audit response
- 3+ conflict signals → `Conflicted` → buddy enters `Confused` mood

### Context Compression (`compact.rs`)

| Tier | Token Limit | Compression Trigger (85%) |
|---|---|---|
| HighEnd | 65,536 | 55,706 |
| Medium | 32,768 | 27,853 |
| Small | 16,384 | 13,926 |
| ExtraSmall | 8,192 | 6,963 |

Token estimation: `text.len() / 4 + 1` (rough heuristic, ~4 chars/token).  
Compression: summarize oldest messages via LLM, keep last 3 minimum, target 50% of limit.

---

## 5. Security System (`tools/security.rs`)

### SecurityTool Trait

```rust
pub trait SecurityTool {
    fn name(&self) -> &str;
    fn is_available(&self) -> bool;
    fn scan(&self, target: &Path) -> Result<ScanReport>;
}
```

### Implementations

| Tool | Type | External Binary | Detection | Output Parsing |
|---|---|---|---|---|
| `Semgrep` | SAST | `semgrep` | `semgrep --version` | JSON (`--json --quiet`) → `SemgrepOutput` struct |
| `CargoAudit` | SCA | `cargo audit` | `cargo audit --version` | JSON (`--json`) → `CargoAuditOutput` struct |
| `ClippyLint` | SAST (Rust) | `cargo clippy` | `cargo clippy --version` | JSON (`--message-format=json`) → compiler messages |

### Severity Enum

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity { Info, Warning, Error, Critical }
```

Ordered: `Critical > Error > Warning > Info` (used for sorting in reports).

### Finding Struct

```rust
pub struct Finding {
    pub tool: String,
    pub severity: Severity,
    pub rule_id: String,
    pub message: String,
    pub file: PathBuf,
    pub line: Option<usize>,
    pub owasp_category: Option<String>,
}
```

### Security Scanner Orchestrator

```rust
pub struct SecurityScanner {
    tools: Vec<Box<dyn SecurityTool>>,  // [Semgrep, CargoAudit, ClippyLint]
}
```

Methods:
- `available_tools()` → lists installed tools
- `scan_all(target)` → runs all available tools, collects reports
- `scan_with(tool_name, target)` → runs specific tool
- `total_findings(reports)` → sum
- `severity_counts(reports)` → `(critical, error, warning, info)`

### Semgrep Config

Default: `p/default`. Configurable via `Semgrep::with_config()`.  
OWASP metadata extraction from Semgrep results (`extra.metadata.owasp`).

### Auto-Fix Prompt Generation

`ScanReport::auto_fix_prompt()` → generates LLM prompt from findings with severity >= Warning.  
Format references OWASP ASVS standards and requests file/line/corrected code.

### OWASP System Prompt (injected into all LLM calls)

```rust
pub const SECURITY_SYSTEM_PROMPT: &str = "\
You are a secure code generation assistant. Follow these rules strictly:
- Generate code following OWASP ASVS (Application Security Verification Standard).
- Never generate code with SQL injection, XSS, command injection, or path traversal.
- Always use parameterized queries for database operations.
- Sanitize and validate all user inputs at system boundaries.
- Use safe memory patterns — no unchecked indexing, no raw pointer arithmetic.
- Prefer standard library cryptographic primitives over custom implementations.
- Log security-relevant events but never log secrets, tokens, or PII.
If the user asks for something insecure, warn them and provide the secure alternative.\n\n";
```

### Compliance Report Generator (`report.rs`)

`generate_report()` produces Markdown with sections:
1. Executive Summary (finding counts, risk level: HIGH/MEDIUM/LOW/CLEAN)
2. SAST Results (Semgrep + Clippy findings table)
3. SCA Results (cargo-audit advisories table)
4. Familiar Status (buddy name, level, vulns caught)
5. Binary Integrity (SHA-256 of binary)

---

## 6. TUI State (`crates/tui/`)

### Layout (3-panel + buddy)

```
┌──────────────────────────────────────────┬──────────────────┐
│                                          │  Hardware (25%)  │
│          CHAT PANEL (68%)                │  CPU/RAM bars    │
│  role-colored messages                   │  Recommended     │
│  system=Yellow, you=Cyan,                │  models          │
│  sovereign=Green, security=Red           ├──────────────────┤
│                                          │  Security (35%)  │
│                                          │  CRIT/ERR/WARN   │
│                                          │  tool list       │
├──────────────────────────────────────────┤  total findings  │
│  INPUT (3 lines, green border)           ├──────────────────┤
│  cursor-tracked, Ctrl+C/Esc to quit     │  BUDDY (40%)     │
│                                          │  ASCII sprite    │
│                                          │  name [rarity]   │
│                                          │  LVL + XP bar    │
│                                          │  HP/MP bars      │
│                                          │  lifetime stats  │
└──────────────────────────────────────────┴──────────────────┘
```

### Buddy System — FULLY IMPLEMENTED

**Species** (4): Raven `(o v o)`, Golem `[O_O]`, Spirit `-{_}-`, Dragon `(@\___`  
**Rarity** (5): Common(75%), Uncommon(15%), Rare(6%), Epic(3%), Sovereign(1%)  
**Moods** (8): Happy, Idle, Working, Stressed, Angry, Exhausted, Confused, Remembering

**Persistence:** `.sovereign/buddy.json` per project (serde JSON).  
**XP:** `+lines/10` for audited code, `+25` per vulnerability caught.  
**Level formula:** next level at `(level + 1) * 100` XP.

**Animation FPS (hardware-adaptive):**

| Tier | FPS | Interval |
|---|---|---|
| HighEnd | 10 | 100ms |
| Medium | 4 | 250ms |
| Small | 2 | 500ms |
| ExtraSmall | 1 | 1000ms (only on user input) |

**Reactive moods:** Critical findings → Angry (red, vibrating). Council conflict → Confused (magenta). Hardware >90% → Exhausted. Session load → Remembering (blue).

### TUI Commands Handled

| Command | Action |
|---|---|
| `/model <name>` | SafeLoad + switch |
| `/index [path]` | RAG indexing + buddy XP |
| `/status` | Hardware + model + RAG status |
| `/buddy` | Full buddy stats |
| `/help` | Command list |
| `/quit` | Save buddy + exit |

---

## 7. Test Status

**68 tests — ALL PASSING (0 failures)**

| Crate | Module | Tests | Coverage Area |
|---|---|---|---|
| `sovereign-core` | `hardware_env` | 10 | Quant detection, param extraction, weight calc, tier classification, GPU bonus, SafeLoad |
| `sovereign-core` | `memdir` | 12 | Cosine similarity, chunking, sensitive file detection, vector store CRUD, persistence, tier params |
| `sovereign-core` | `grimoire` | 3 | Record/find by rule, keyword search, count |
| `sovereign-core` | `history` | 4 | Save/load sessions, integrity verification, list, restore messages |
| `sovereign-query` | `router` | 4 | Heuristic classification (code/logic/ambiguous), model mapping |
| `sovereign-query` | `agent` | 7 | Action parsing (ReadFile/Execute/Answer/fallback), thought log, recent thoughts, command execution |
| `sovereign-query` | `consensus` | 4 | Consensus levels (aligned/conflicted/minor), diff summary format |
| `sovereign-query` | `compact` | 6 | Context limits, token estimation, compression detection, split preservation, prompt format, status display |
| `sovereign-tools` | `security` | 6 | Severity ordering, finding display, report summary, auto-fix prompt, empty report, scanner creation |
| `sovereign-tools` | `report` | 3 | Clean report, report with findings, section verification |
| `sovereign-tui` | `buddy` | 9 | Species/rarity rolls, name generation, XP leveling, mood priority, animation frames, dragon fire, persistence, stat bar |

### Untested Areas (0 test coverage)

| Module | Reason |
|---|---|
| `api/client.rs` | Requires live Ollama server — integration test candidate |
| `query/coordinator.rs` | Requires live Ollama + filesystem — integration test candidate |
| `tui/app.rs` | TUI event loop — requires terminal mocking |
| `cli/main.rs` | Entry point — covered by integration/E2E tests |

---

## 8. Persistence Map (`.sovereign/` per project)

| File | Format | Module | Content |
|---|---|---|---|
| `index.bin` | bincode | `memdir.rs` | VectorStore (embeddings + chunks) |
| `buddy.json` | JSON | `buddy.rs` | BuddyData (species, rarity, XP, stats) |
| `grimoire.db` | SQLite | `grimoire.rs` | Security patterns (error→fix pairs) |
| `history.db` | SQLite | `history.rs` | Session records (SHA-256 signed chat logs) |

---

*Snapshot generated for Sovereign-SDLC v0.2.0 upgrade planning to v0.4.0*
