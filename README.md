# Sovereign-SDLC

Local AI agent for secure software development. Runs 100% offline with Ollama. Hardware-adaptive — works on a MacBook M5 (full power) or an office laptop with no GPU (lightweight mode).

Built in Rust. Single binary. No Python, no Node, no Docker required.

## What It Does

- **Smart Routing** — Classifies your prompt (CODE / LOGIC / CHAT) and routes to the optimal model
- **RAG Memory** — Indexes your project with `nomic-embed-text` embeddings for context-aware answers
- **Security Pipeline** — Semgrep (SAST), cargo-audit (SCA), Clippy integrated. Vulnerabilities detected automatically
- **Auto-Fix** — Generates diff patches for critical vulnerabilities and learns from fixes (Grimoire)
- **Dual Inference** — On powerful hardware, runs dev + audit models in parallel for consensus
- **Session History** — SQLite-backed, SHA-256 signed chat logs per project
- **Buddy System** — RPG pet companion that levels up as you audit code

## Requirements

- [Ollama](https://ollama.com) installed and running (`ollama serve`)
- At least one model pulled (see Hardware Tiers below)

## Installation

### Pre-built Binary (No Rust Required)

Download from [Releases](../../releases/latest):

| Platform | File |
|---|---|
| macOS (Apple Silicon) | `sovereign-aarch64-apple-darwin` |
| macOS (Intel) | `sovereign-x86_64-apple-darwin` |
| Linux (x86_64) | `sovereign-x86_64-unknown-linux-gnu` |
| Windows | `sovereign-x86_64-pc-windows-msvc.exe` |

```bash
# macOS / Linux
chmod +x sovereign-*
./sovereign-aarch64-apple-darwin

# Or move to PATH
sudo mv sovereign-* /usr/local/bin/sovereign
sovereign
```

### Build from Source

```bash
git clone https://github.com/YOUR_USER/sovereign-sdlc.git
cd sovereign-sdlc
cargo build --release
./target/release/sovereign
```

## Usage

### TUI Mode (Default)

```bash
sovereign
```

Opens a 3-panel terminal interface:
- **Left** — Chat with the AI agent
- **Right Top** — Live hardware monitor (CPU/RAM bars)
- **Right Middle** — Security dashboard (vulnerability counters)
- **Right Bottom** — Buddy companion (animated, reactive to system state)

### REPL Mode

```bash
sovereign --repl
# or
sovereign -r
```

Simple terminal interface without the TUI panels.

## Commands

| Command | Description |
|---|---|
| `/model <name>` | Switch LLM model. SafeLoad validates it fits in RAM before loading |
| `/index [path]` | Index a project directory for RAG. Scans files, generates embeddings, enables context-aware answers. Skips `.env`, secrets, binaries automatically (Zero-Trust) |
| `/scan [path] [-t tool]` | Run security scan. Tools: `semgrep`, `cargo-audit`, `clippy`. Omit `-t` to run all available |
| `/doc [path]` | Generate ARCHITECTURE.md with Mermaid diagrams from project analysis |
| `/status` | Show hardware tier, active model, RAM/CPU usage, RAG index size, Grimoire pattern count |
| `/buddy` | Show companion stats: species, rarity, level, XP, lifetime auditing stats |
| `/sessions` | List previous sessions (stored in SQLite with integrity hashes) |
| `/load <id>` | Restore a previous session's context |
| `/audit` | Toggle OWASP audit mode on generated code |
| `/help` | Show all available commands |
| `/quit` | Save buddy state and exit |

### Keyboard Shortcuts (TUI)

| Key | Action |
|---|---|
| `Enter` | Submit message or command |
| `Ctrl+C` | Quit (saves buddy) |
| `Esc` | Quit (saves buddy) |
| `Up/Down` | Scroll chat history |
| `Left/Right` | Move cursor in input |

## Hardware Tiers

Sovereign detects your hardware at startup and adapts automatically:

| Tier | RAM Available | Dev Model | Audit Model | Context | Animation |
|---|---|---|---|---|---|
| **HighEnd** | 20+ GB | `qwen2.5-coder:14b` | `deepseek-r1:14b` | 64k tokens | 10 FPS |
| **Medium** | 12-20 GB | `qwen2.5-coder:7b` | `deepseek-r1:7b` | 32k tokens | 4 FPS |
| **Small** | 8-12 GB | `qwen2.5-coder:3b` | `phi-4:mini` | 16k tokens | 2 FPS |
| **ExtraSmall** | <8 GB | `llama3.2:3b` | `phi-4:mini` | 8k tokens | 1 FPS |

### Pull Models for Your Tier

```bash
# HighEnd (MacBook M5, RTX GPU)
ollama pull qwen2.5-coder:14b
ollama pull deepseek-r1:14b
ollama pull nomic-embed-text

# Small / ExtraSmall (Office laptop, no GPU)
ollama pull llama3.2:3b
ollama pull phi-4:mini
ollama pull nomic-embed-text
```

### Platform Detection

| Platform | Detection Method |
|---|---|
| Apple Silicon (M1-M5) | CPU brand string, unified memory sizing |
| NVIDIA GPU | NVML dynamic library probing (CUDA) |
| AMD/Intel GPU | Vulkan library detection |
| CPU Only | Fallback — aggressive threading, lightweight models |

### SafeLoad Guard

Before loading any model, Sovereign checks:

```
(Model Weight + 4 GB buffer) > Available RAM  -->  BLOCKED
```

If blocked, it suggests a quantized alternative that fits. You will never crash from OOM.

## Security Pipeline

### Integrated Tools

| Tool | Type | What It Checks |
|---|---|---|
| **Semgrep** | SAST | OWASP Top 10, injection, XSS, secrets in code |
| **cargo-audit** | SCA | Known vulnerabilities in Rust dependencies |
| **Clippy** | Lint | Unsafe patterns, logic bugs, Rust best practices |

### Install Security Tools (Optional)

```bash
# Semgrep
pip install semgrep

# cargo-audit
cargo install cargo-audit
```

Sovereign works without these — it just won't run those specific scans.

### Auto-Fix Protocol

When a critical vulnerability is detected:
1. Sovereign generates a `diff` patch that fixes the issue
2. The fix preserves existing functionality
3. The error-fix pair is stored in the **Grimoire** (SQLite) for future learning
4. The buddy earns XP

## RAG (Retrieval Augmented Generation)

```bash
sovereign --repl
> /index .
# Scans project, chunks files, generates embeddings via nomic-embed-text
# Persists to .sovereign/index.bin

> explain the authentication flow
# [CODE +RAG] -> qwen2.5-coder:14b
# Retrieves relevant code chunks and injects them as context
```

### Zero-Trust Filtering

The indexer automatically skips:
- `.env`, credentials, API keys, certificates
- Binary files, images, archives
- `node_modules/`, `target/`, `.git/`
- Files in `.gitignore`

## Buddy System

Every project gets a companion that levels up as you work:

### Species (7)

```
Gato:    =^.^=  /  =^-^=  /  =>.<=
Buho:    (O,O)  /  (-,O)  /  (X,X)
Dragon:  (@\___ /  (O\___ /  (X\***
Fractal: {*_*}  /  {~_~}  /  {!_!}
Raven:   (o v o)/  (- v -)/  (O V O)
Spirit:  -{_}-  /  ~{_}~  /  !{_}!
Golem:   [O_O]  /  [o_o]  /  [X_X]
```

### Rarity (Gacha Roll on Project Init)

| Rarity | Chance | Color |
|---|---|---|
| Common | 75% | White |
| Uncommon | 15% | Green |
| Rare | 6% | Blue |
| Epic | 3% | Magenta |
| **SOVEREIGN** | **1%** | **Gold** |

### Reactive Moods

| Condition | Mood | Behavior |
|---|---|---|
| Critical security finding | **ANGRY** | Red, vibrates |
| Models disagree (Council) | **Confused** | Magenta |
| CPU/RAM > 90% | Exhausted | Gray |
| Loading old session | Remembering | Blue |
| Clean code scan | Happy | Green |

### XP System

| Action | XP |
|---|---|
| Lines of code audited | +1 per 10 lines |
| Vulnerability caught | +25 |
| Auto-fix applied | +30 |
| Clean scan (0 findings) | +50 |

Level up formula: `(level + 1) * 100` XP needed.

## Project Data

Sovereign stores per-project data in `.sovereign/`:

```
.sovereign/
  index.bin     # RAG vector store (embeddings)
  buddy.json    # Companion state (species, XP, stats)
  grimoire.db   # Security patterns learned from fixes (SQLite)
  history.db    # Session history with SHA-256 integrity (SQLite)
```

Add `.sovereign/` to your `.gitignore`. It contains project-specific data, not source code.

## Architecture

```
sovereign-sdlc/
  crates/
    core/     Hardware detection, vector store, grimoire, sessions, docs
    api/      Ollama client (generate, embed, list_models)
    tools/    Security pipeline (Semgrep, cargo-audit, Clippy) + reports
    query/    Agent brain (router, ReAct loop, coordinator, consensus, compression)
    tui/      Terminal UI (3-panel) + buddy system
    cli/      Binary entry point
```

Binary name: `sovereign`

## License

MIT
