# OpenAgents Kit

OpenAgents Kit is a Rust-based TUI scaffold for centralizing AI workspace setup across tools, accounts, and devices.

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
- The TUI helps users inspect, apply, and repair their setup.

## Getting Started

1. Clone the template repository.
2. Edit `workspace.yaml` to define your profiles and memory backend.
3. Run one of the commands below:

```bash
cargo run -p openagents-tui --bin openagents-kit -- doctor --profile personal-client
cargo run -p openagents-tui --bin openagents-kit -- apply --profile personal-client --tool codex --dry-run
cargo run -p openagents-tui --bin openagents-kit
```

## Project Layout

- `crates/openagents-core` - manifest parsing, profile resolution, and shared models
- `crates/openagents-adapters` - tool renderers and output generation
- `crates/openagents-tui` - interactive setup and diagnostics
- `examples/` - starter manifests and sample profiles

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.
