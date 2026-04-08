# Sovereign SDLC

A local AI agent that writes, edits, and audits code on your machine. No cloud. No API keys. No telemetry. Just you, your hardware, and an LLM that can actually touch your filesystem.

Built in Rust. Single binary. Runs on macOS, Linux, and Windows through [Ollama](https://ollama.com).

---

## How It Works

You type a request. The agent streams a response token-by-token. If it needs to read a file, search your codebase, or run a command, it calls one of its 6 built-in tools automatically. You approve writes and executes; reads are instant. The result feeds back into the conversation and the agent continues until it has your answer.

```
you > what version of Rust is this project using?

  ~ [Agent] via qwen2.5:7b

  sov  Let me check.

  ~ [tool] read: {"path":"Cargo.toml"}
  [+] read (0ms)

  sov  This project uses Rust edition 2024, with workspace version 0.5.0.
       The Cargo.toml shows 6 workspace members under crates/.

  [+] 4.1s | 892 tokens | 13.2 tok/s
```

No prompt engineering required. The agent decides what tools to use based on your question.

---

## Quick Start

### 1. Install Ollama

Download from [ollama.com](https://ollama.com) and start the server:

```bash
ollama serve
```

### 2. Pull a model

```bash
# Good default for most machines (4.7 GB)
ollama pull qwen2.5:7b

# Embeddings for RAG indexing (274 MB)
ollama pull nomic-embed-text
```

### 3. Run Sovereign

**macOS / Linux**
```bash
# Download from https://github.com/BrayansStivens/sovereign-sdlc/releases/latest
chmod +x sovereign
./sovereign
```

**Windows (PowerShell)**
```powershell
# Download sovereign.exe from Releases
.\sovereign.exe
```

**Build from source (any platform)**
```bash
git clone https://github.com/BrayansStivens/sovereign-sdlc.git
cd sovereign-sdlc
cargo build --release
# Binary at: target/release/sovereign (or sovereign.exe on Windows)
```

---

## Two Modes

### TUI (default)

```bash
sovereign
```

Full terminal interface. Three panels: chat on the left, hardware monitor + buddy companion on the right. Streaming text, tool execution indicators, approval overlays.

### REPL

```bash
sovereign --repl
```

Lightweight mode. Same agent loop, same tools, same streaming — just plain text in your terminal. Works everywhere, including inside other tools and over SSH.

---

## Tools

The agent has 6 tools it can call during a conversation. Read-only tools execute instantly. Write and execute tools ask for your approval first.

| Tool | Permission | What It Does |
|------|-----------|--------------|
| `bash` | Execute | Run shell commands. Persistent working directory across calls. Timeout: 120s default, 600s max. Output capped at 100K chars |
| `read` | ReadOnly | Read files with line numbers. Supports offset/limit for large files (default: 2000 lines) |
| `glob` | ReadOnly | Find files by pattern. `**/*.rs`, `src/**/*.ts`, or plain filenames for recursive search |
| `grep` | ReadOnly | Regex search across files. Three output modes: file paths, matching lines, or match counts. Filter by file type or glob |
| `edit` | Write | Replace text in files. Rejects ambiguous matches unless `replace_all` is set. You must approve before it writes |
| `write` | Write | Create or overwrite files. Auto-creates parent directories |

The agent can chain multiple tool calls in a single conversation. It reads a file, greps for a pattern, edits the match, and verifies the change — all in one flow.

---

## Commands

Type these in the chat prompt:

| Command | What It Does |
|---------|-------------|
| `/model <name>` | Switch LLM model. SafeLoad checks it fits in RAM first |
| `/index [path]` | Index a project for RAG. Scans, chunks, embeds with nomic-embed-text. Skips secrets, binaries, node_modules |
| `/scan [path]` | Run security scan (semgrep, cargo-audit, clippy) |
| `/status` | Hardware tier, active model, RAM/CPU, RAG stats, Grimoire patterns |
| `/buddy` | Companion stats: species, rarity, level, XP |
| `/help` | Show all commands |
| `/quit` | Save and exit |

### Keyboard (TUI)

| Key | Action |
|-----|--------|
| Enter | Submit prompt |
| Esc | Cancel active generation |
| Up/Down | Scroll chat |
| Ctrl+C | Quit |
| Ctrl+Z | Suspend to background (Unix) |
| y/n | Approve or deny tool execution |

---

## Hardware Tiers

Sovereign detects your hardware at startup. No configuration needed.

| Tier | RAM Available | Dev Model | Audit Model |
|------|-------------|-----------|-------------|
| **HighEnd** | 20+ GB | qwen2.5-coder:14b | deepseek-r1:14b |
| **Medium** | 12-20 GB | qwen2.5-coder:7b | deepseek-r1:7b |
| **Small** | 8-12 GB | qwen2.5-coder:3b | phi-4:mini |
| **ExtraSmall** | <8 GB | llama3.2:3b | phi-4:mini |

Pull models for your tier:

```bash
# High-end (20+ GB)
ollama pull qwen2.5-coder:14b && ollama pull deepseek-r1:14b && ollama pull nomic-embed-text

# Mid-range (8-16 GB)
ollama pull qwen2.5:7b && ollama pull nomic-embed-text

# Lightweight (any machine)
ollama pull llama3.2:3b && ollama pull nomic-embed-text
```

### Platform Detection

| Platform | How It Detects | What Happens |
|----------|---------------|--------------|
| Apple Silicon | CPU brand, unified memory | Full RAM available for models |
| NVIDIA GPU | CUDA / NVML | Layers offloaded to VRAM |
| AMD/Intel GPU | Vulkan detection | Partial VRAM offload |
| CPU only | Fallback | Smaller models, lower refresh rate |

### SafeLoad

Before loading any model, Sovereign checks `model_weight + 4 GB < available_RAM`. If it fails, it suggests a quantized alternative. No OOM crashes.

---

## RAG

```bash
sovereign --repl
> /index .
# Scans project -> chunks files -> embeds with nomic-embed-text
# Persists to .sovereign/index.bin

> how does the authentication middleware work?
# Agent retrieves relevant code chunks as context before answering
```

The indexer skips `.env`, credentials, API keys, certificates, binaries, `node_modules/`, `target/`, `.git/`, and anything in `.gitignore`.

---

## Security Pipeline

| Tool | Type | Checks |
|------|------|--------|
| Semgrep | SAST | OWASP Top 10, injection, XSS, hardcoded secrets |
| cargo-audit | SCA | Known CVEs in Rust dependencies |
| Clippy | Lint | Unsafe patterns, logic bugs, Rust idioms |

Install them (optional — Sovereign works without them):

```bash
pip install semgrep
cargo install cargo-audit
```

When a critical vulnerability is found, the agent can generate a fix patch and store the pattern in the Grimoire (SQLite) for future reference.

---

## Buddy System

Every project gets a companion. 11 species, 5 rarity tiers (Common to Sovereign), reactive moods based on system load. They earn XP when you audit code and level up over time. Run `/buddy` to check yours.

---

## Project Data

Stored per-project in `.sovereign/`:

```
.sovereign/
  index.bin      RAG vector store
  buddy.json     Companion state
  grimoire.db    Security patterns (SQLite)
  history.db     Session history (SQLite, SHA-256 signed)
```

Add `.sovereign/` to your `.gitignore`.

---

## Architecture

```
sovereign-sdlc/
  crates/
    core/     Hardware detection, RAG, grimoire, sessions, system prompts
    api/      Ollama client — streaming chat, embeddings, model listing
    tools/    6 agent tools + security scanner (semgrep, cargo-audit, clippy)
    query/    Agent loop, smart router, coordinator, context compression
    tui/      Terminal UI + buddy system + approval overlay
    cli/      Binary entry point (TUI + REPL modes)
```

134 tests. ~10,000 lines of Rust. Single binary, no runtime dependencies beyond Ollama.

---

## Building for Windows

```powershell
# Install Rust: https://rustup.rs
git clone https://github.com/BrayansStivens/sovereign-sdlc.git
cd sovereign-sdlc
cargo build --release
# Binary at: target\release\sovereign.exe
```

The TUI uses crossterm which supports Windows Terminal, PowerShell, and cmd.exe natively. ConPTY is used for proper ANSI rendering on Windows 10+.

---

## License

MIT
