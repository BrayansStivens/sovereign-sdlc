# Sovereign-SDLC

Local AI agent for secure software development. Runs 100% offline with Ollama. Hardware-adaptive — automatically detects your system capabilities and selects the right models, whether you have a high-end workstation with a GPU or a basic office laptop with just a CPU.

Cross-platform: macOS, Linux, Windows. Built in Rust. Single binary. No Python, no Node, no Docker required.

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

Download from [Releases](../../releases/latest) the binary for your platform.

```bash
# macOS / Linux — make it executable and run
chmod +x sovereign-*
./sovereign-*

# Or move to your PATH for global access
sudo mv sovereign-* /usr/local/bin/sovereign
sovereign
```

```powershell
# Windows — just run it
.\sovereign-x86_64-pc-windows-msvc.exe
```

> **Note:** Pre-built binaries are added as they become available. If your platform isn't listed yet, build from source (see below).

### Build from Source

```bash
git clone https://github.com/BrayansStivens/sovereign-sdlc.git
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

Sovereign detects your hardware at startup and adapts automatically. No configuration needed.

| Tier | When It Activates | Dev Model | Audit Model | Context | TUI FPS |
|---|---|---|---|---|---|
| **HighEnd** | 20+ GB free RAM (or GPU with 16+ GB VRAM) | `qwen2.5-coder:14b` | `deepseek-r1:14b` | 64k tokens | 10 |
| **Medium** | 12-20 GB free RAM | `qwen2.5-coder:7b` | `deepseek-r1:7b` | 32k tokens | 4 |
| **Small** | 8-12 GB free RAM | `qwen2.5-coder:3b` | `phi-4:mini` | 16k tokens | 2 |
| **ExtraSmall** | <8 GB free RAM (CPU only) | `llama3.2:3b` | `phi-4:mini` | 8k tokens | 1 |

### Pull Models for Your System

```bash
# If you have 20+ GB RAM or a dedicated GPU
ollama pull qwen2.5-coder:14b
ollama pull deepseek-r1:14b
ollama pull nomic-embed-text

# If you have 8-12 GB RAM or no GPU
ollama pull qwen2.5-coder:3b
ollama pull phi-4:mini
ollama pull nomic-embed-text

# Minimum viable (any machine)
ollama pull llama3.2:3b
ollama pull nomic-embed-text
```

### Platform Detection

| Platform | Detection Method | Optimization |
|---|---|---|
| Apple Silicon | CPU brand string, unified memory | Full memory available for models |
| NVIDIA GPU (Linux/Windows) | NVML dynamic library (CUDA) | Model layers offloaded to VRAM |
| AMD/Intel GPU (Linux/Windows) | Vulkan library detection | Partial VRAM offload |
| CPU Only (any OS) | Automatic fallback | Aggressive threading, lightweight models, lower TUI refresh |

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

Every project gets a companion that levels up as you audit code. 11 species, 5 rarity tiers, reactive moods. Run `/buddy` in-app to see yours.

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
