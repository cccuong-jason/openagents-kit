# OpenAgents Kit

OpenAgents Kit is a Rust-based terminal control plane for keeping AI tools, skills, MCP servers, and memory aligned across devices and projects.

It gives you one source of truth for:

- Global profiles for personal and team contexts
- Memory providers and shared context backends
- Tool adapters for `Codex`, `Claude`, and `Gemini`
- Curated skill and MCP inventories
- Managed sync outputs and project attachments

## What This Repo Is For

This repository is meant to be forked as a public template. Other people can copy the scaffold, rename it, and point it at their own control-plane setup without rebuilding the structure from scratch.

## Core Concepts

- `config.yaml` in the OpenAgents app-config directory is the canonical source of truth.
- `device.yaml` stores machine-local bindings such as managed-output and memory roots.
- `attachments.yaml` maps the current folder or repo to a global profile without storing config inside the repo.
- Profiles describe which tools, skills, MCP servers, and memory behavior OpenAgents should keep in sync.
- The terminal UI drives setup as a conversation first and only shows the dashboard after setup is complete.

## Installation

Primary install:

```bash
npx openagents-kit
```

That installs the native `openagents-kit` binary into an OpenAgents-owned global bin directory, repairs your user `PATH` automatically when needed, and refreshes an older user-owned `openagents-kit` binary if one already wins on `PATH`. After install you can run:

```bash
openagents-kit
```

Canonical install locations:

- Windows: `%LOCALAPPDATA%\OpenAgents\bin`
- macOS/Linux: `~/.local/bin`

Release-script fallbacks:

```powershell
powershell -ExecutionPolicy Bypass -Command "iwr https://raw.githubusercontent.com/cccuong-jason/openagents-kit/main/scripts/install.ps1 -UseBasicParsing | iex"
```

```bash
curl -fsSL https://raw.githubusercontent.com/cccuong-jason/openagents-kit/main/scripts/install.sh | bash
```

Technical fallback:

```bash
cargo install --git https://github.com/cccuong-jason/openagents-kit openagents-tui --bin openagents-kit
```

## Getting Started

1. Install with `npx openagents-kit`.
2. Run `openagents-kit` or `openagents-kit setup`.
3. Let the first-run flow scan local Codex, Claude, and Gemini footprints.
4. Answer the assistant-led questions as OpenAgents proposes a profile, memory backend, tool set, starter skills, and starter MCP servers.
5. Let OpenAgents write the global control plane and sync managed outputs.

Useful commands after setup:

```bash
openagents-kit sync
openagents-kit doctor
openagents-kit memory --ensure
openagents-kit catalog
openagents-kit attach --profile personal-client
openagents-kit setup --dry-run
```

## First-Run UX

- Auto-detects supported Codex, Claude, and Gemini config/state files on first run
- Detects missing memory, missing skills, and missing MCP server inventory
- Builds a recommended global control plane from what it finds
- Drives setup as a conversational interview, one decision at a time
- Attaches the current project to a global profile instead of generating repo-owned config by default
- Syncs managed tool outputs, skill assets, MCP assets, and local filesystem memory from the same source of truth

## Config Layout

OpenAgents stores its control plane in the platform app-config directory:

- Windows: `%APPDATA%/OpenAgents/`
- macOS/Linux: `~/.config/openagents/`

Important files:

- `config.yaml`
- `device.yaml`
- `attachments.yaml`
- `managed/`
- `memory/`

Legacy `workspace.yaml` files are still readable as an import source during setup, but they are no longer the primary model.

## Project Layout

- `crates/openagents-core` - control-plane parsing, legacy manifest import, profile resolution, and shared models
- `crates/openagents-adapters` - tool renderers and managed output generation
- `crates/openagents-tui` - first-run chat setup, detection, control-plane sync, and diagnostics
- `examples/` - starter manifests and sample profiles
- `scripts/` - install helpers for GitHub Releases

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.
