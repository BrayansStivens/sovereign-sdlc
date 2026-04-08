
<p align="center">
  <br>
  <code>sovereign</code>
  <br>
  <strong>Your code stays on your machine. So does your AI.</strong>
  <br>
  <br>
</p>

<p align="center">
  <a href="https://github.com/BrayansStivens/sovereign-sdlc/releases/latest">Download</a>
  &nbsp;&middot;&nbsp;
  <a href="#quick-start">Quick Start</a>
  &nbsp;&middot;&nbsp;
  <a href="#tools">Tools</a>
  &nbsp;&middot;&nbsp;
  <a href="#hardware-tiers">Hardware Tiers</a>
</p>

---

Sovereign is a local AI coding agent. It connects to [Ollama](https://ollama.com), streams responses in real-time, and can read, search, edit, and create files on your behalf. It asks before it writes. It runs entirely offline. There is no cloud, no API key, no telemetry.

One binary. No Python, no Node, no Docker. Works on macOS, Linux, and Windows.

```
you > what version of Rust is this project using?

  ~ [Agent] via qwen2.5:7b

  sov | Let me check.
      |
  ~ [tool] read: Cargo.toml
  [+] read (0ms)
      |
  sov | This project uses Rust edition 2024, workspace version 0.5.0.
      | The Cargo.toml shows 6 crates under the workspace.
      |
  [+] 4.1s | 892 tokens | 13.2 tok/s
```

---

## Quick Start

**1. Install Ollama** from [ollama.com](https://ollama.com), then:

```bash
ollama serve
ollama pull qwen2.5:7b
```

**2. Download Sovereign** from the [latest release](https://github.com/BrayansStivens/sovereign-sdlc/releases/latest):

| Platform | Binary |
|----------|--------|
| macOS (Apple Silicon) | `sovereign-aarch64-apple-darwin` |
| macOS (Intel) | `sovereign-x86_64-apple-darwin` |
| Linux (x86_64) | `sovereign-x86_64-linux` |
| Linux (ARM64) | `sovereign-aarch64-linux` |
| Windows | `sovereign-x86_64-windows.exe` |

**3. Run it**

```bash
# macOS / Linux
chmod +x sovereign-*
./sovereign-aarch64-apple-darwin
```

```powershell
# Windows
.\sovereign-x86_64-windows.exe
```

That's it. No Rust, no compiler, no dependencies beyond Ollama.

---

## Two Modes

**TUI** -- full terminal interface with streaming, hardware monitor, and buddy companion:

```bash
sovereign
```

**REPL** -- lightweight, works over SSH, inside containers, or piped:

```bash
sovereign --repl
```

Both use the same agent loop, same tools, same streaming engine.

---

## Tools

The agent has 6 tools. It decides when to use them based on your request. Read-only tools run instantly. Writes and executes ask for your approval.

| Tool | Permission | Description |
|------|-----------|-------------|
| `bash` | Execute | Run shell commands. Working directory persists between calls |
| `read` | ReadOnly | Read files with line numbers. Offset and limit for large files |
| `glob` | ReadOnly | Find files by pattern. Recursive search for plain names |
| `grep` | ReadOnly | Regex search across files. Filter by type, glob, or context lines |
| `edit` | Write | Replace text in files. Validates uniqueness before writing |
| `write` | Write | Create or overwrite files. Creates parent directories |

Tools chain naturally. The agent reads a file, greps for a pattern, edits the match, and verifies -- all in one conversation.

---

## Commands

| Command | Description |
|---------|-------------|
| `/model <name>` | Switch LLM. SafeLoad checks it fits in RAM |
| `/index [path]` | Index project for RAG. Embeds with nomic-embed-text |
| `/scan [path]` | Security scan with semgrep, cargo-audit, clippy |
| `/status` | Hardware, model, RAM, RAG and Grimoire stats |
| `/buddy` | Companion species, rarity, level, XP |
| `/help` | All commands |
| `/quit` | Save and exit |

**Keyboard (TUI):** Enter to submit, Esc to cancel, Up/Down to scroll, y/n to approve tools, Ctrl+C to quit.

---

## Hardware Tiers

Sovereign detects your hardware at startup and picks models automatically.

| Tier | RAM | Dev Model | Audit Model |
|------|-----|-----------|-------------|
| HighEnd | 20+ GB | qwen2.5-coder:14b | deepseek-r1:14b |
| Medium | 12-20 GB | qwen2.5-coder:7b | deepseek-r1:7b |
| Small | 8-12 GB | qwen2.5-coder:3b | phi-4:mini |
| ExtraSmall | <8 GB | llama3.2:3b | phi-4:mini |

Supported platforms: Apple Silicon (unified memory), NVIDIA (CUDA), AMD/Intel (Vulkan), CPU-only.

**SafeLoad** prevents OOM: if `model_weight + 4 GB > available_RAM`, Sovereign blocks the load and suggests a lighter alternative.

### Pull models for your machine

```bash
# High-end (20+ GB free RAM)
ollama pull qwen2.5-coder:14b && ollama pull deepseek-r1:14b

# Mid-range (8-16 GB)
ollama pull qwen2.5:7b

# Lightweight (any machine)
ollama pull llama3.2:3b

# Always needed for RAG
ollama pull nomic-embed-text
```

---

## RAG

Index your project and Sovereign injects relevant code as context into every prompt.

```
> /index .
  Indexed 42 files -> 186 chunks (tier: HighEnd, batch: 32)

> how does the authentication middleware work?
  ~ [Agent +RAG] via qwen2.5-coder:14b
```

The indexer skips `.env`, secrets, API keys, binaries, `node_modules/`, `target/`, `.git/`, and `.gitignore` entries.

---

## Security

| Tool | Type | What It Checks |
|------|------|----------------|
| Semgrep | SAST | OWASP Top 10, injection, XSS, hardcoded secrets |
| cargo-audit | SCA | Known CVEs in Rust dependencies |
| Clippy | Lint | Unsafe patterns, logic bugs |

Install them if you want (Sovereign works without them):

```bash
pip install semgrep
cargo install cargo-audit
```

When a critical vulnerability is found, the agent generates a fix and stores the pattern in the **Grimoire** for future context.

---

## Buddy System

Every project gets a companion. 11 species, 5 rarity tiers, moods that react to your system load. They gain XP when you audit code. `/buddy` to check yours.

---

## Project Data

Per-project state in `.sovereign/`:

```
.sovereign/
  index.bin      # RAG embeddings
  buddy.json     # Companion state
  grimoire.db    # Learned security patterns
  history.db     # Session logs (SHA-256 signed)
```

Add `.sovereign/` to `.gitignore`.

---

## Architecture

```
crates/
  core/     Hardware detection, RAG, grimoire, system prompts
  api/      Ollama client -- streaming chat, embeddings
  tools/    6 agent tools + security scanner
  query/    Agent loop, router, coordinator, compression
  tui/      Terminal interface + buddy + approval overlay
  cli/      Entry point (TUI and REPL modes)
```

134 tests. ~10,000 lines of Rust. Single binary.

---

## Build from Source

Only needed if you want to hack on Sovereign itself or your platform isn't in the releases.

```bash
# Install Rust: https://rustup.rs
git clone https://github.com/BrayansStivens/sovereign-sdlc.git
cd sovereign-sdlc
cargo build --release
./target/release/sovereign
```

---

## License

MIT
