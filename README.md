# OpenAgents Kit

OpenAgents Kit is a Rust-based terminal setup kit for centralizing AI workspace setup across tools, accounts, and devices.

It gives you one source of truth for:

- Profiles for personal and team contexts
- Memory providers and shared context backends
- Tool adapters for `Codex`, `Claude`, and `Gemini`
- Generated outputs and bootstrap artifacts

## What This Repo Is For

This repository is meant to be forked as a public template. Other people can copy the scaffold, rename it, and point it at their own memory and tooling setup without rebuilding the structure from scratch.

## Core Concepts

- `workspace.yaml` is the canonical manifest.
- Profiles describe how a workspace should behave for a given context.
- Adapters render tool-specific config from the same source of truth.
- The terminal UI helps users detect existing tools, generate a starter workspace, and repair their setup.

## Installation

Medium-term release installs:

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

1. Install `openagents-kit`.
2. Run `openagents-kit` or `openagents-kit setup`.
3. Let the first-run flow scan local Codex, Claude, and Gemini footprints.
4. Review the proposed starter workspace and generate `workspace.yaml` plus adapter outputs.

If you prefer direct manifest editing, the old workflow still works:

```bash
openagents-kit doctor --profile personal-client
openagents-kit apply --profile personal-client --tool codex --dry-run
openagents-kit setup --dry-run
```

## First-Run UX

- Auto-detects supported Codex, Claude, and Gemini config/state files on first run
- Builds a recommended starter manifest from what it finds
- Falls back into guided setup when no trustworthy local tool state is available
- Keeps `workspace.yaml` as the canonical runtime file for technical users and automation

## Project Layout

- `crates/openagents-core` - manifest parsing, profile resolution, and shared models
- `crates/openagents-adapters` - tool renderers and output generation
- `crates/openagents-tui` - first-run setup, detection, and diagnostics
- `examples/` - starter manifests and sample profiles
- `scripts/` - install helpers for GitHub Releases

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.
