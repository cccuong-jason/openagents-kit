# Auto Ship Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a single `npm run ship` command that auto-bumps the next patch version from npm, runs local release checks, asks for one confirmation, then commits, tags, pushes, publishes, and verifies the new release.

**Architecture:** Build a dedicated Node ship script on top of the existing release guard and bump helpers. The ship command will compute the next version, validate the worktree, present a release plan, optionally pause for confirmation, then orchestrate git and npm commands in a controlled order. Tests will focus on pure planning helpers first so the network-changing orchestration remains thin and predictable.

**Tech Stack:** Node.js scripts, npm scripts, Git CLI, existing Rust verification commands

---

### Task 1: Add failing tests for ship planning helpers

**Files:**
- Create: `tests/ship.test.mjs`
- Modify: `scripts/release-guard.mjs`

- [ ] **Step 1: Write the failing tests**

Cover:
- next patch calculation from a published version
- dirty-file filtering that ignores `workspace.yaml`, `.openagents/`, and `generated/`
- release plan generation for default, `--dry-run`, and `--yes`

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test tests/release-guard.test.mjs tests/ship.test.mjs`
Expected: FAIL because ship helpers do not exist yet.

- [ ] **Step 3: Write minimal helper exports**

Add pure helper functions for:
- parsing semantic versions
- computing the next patch
- filtering allowed dirty paths
- formatting the ship summary

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test tests/release-guard.test.mjs tests/ship.test.mjs`
Expected: PASS

### Task 2: Add the ship CLI orchestration

**Files:**
- Create: `scripts/ship-release.mjs`
- Modify: `package.json`

- [ ] **Step 1: Write the failing tests**

Add test coverage for argument parsing:
- default interactive mode
- `--yes`
- `--dry-run`
- rejecting unsupported extra arguments

- [ ] **Step 2: Run test to verify it fails**

Run: `node --test tests/release-guard.test.mjs tests/ship.test.mjs`
Expected: FAIL because the ship CLI parser is not implemented.

- [ ] **Step 3: Write minimal implementation**

Implement `scripts/ship-release.mjs` to:
- resolve the published version from npm
- compute the next patch version
- bump package/Rust versions
- run `release:check`
- run Rust verification commands
- print planned commit/tag/publish actions
- ask for confirmation unless `--yes`
- support `--dry-run`

- [ ] **Step 4: Run test to verify it passes**

Run: `node --test tests/release-guard.test.mjs tests/ship.test.mjs`
Expected: PASS

### Task 3: Wire docs and final verification

**Files:**
- Modify: `package.json`
- Modify: `AGENTS.md`

- [ ] **Step 1: Update npm scripts and docs**

Add:
- `npm run ship`
- optional explicit aliases if helpful
- AGENTS release playbook updates for the new primary command

- [ ] **Step 2: Run local verification**

Run:
- `node --test tests/release-guard.test.mjs tests/ship.test.mjs tests/workflow-release.test.mjs`
- `npm run ship -- --dry-run`
- `cargo test --workspace --all-features`

- [ ] **Step 3: Commit**

```bash
git add package.json AGENTS.md scripts/ship-release.mjs scripts/release-guard.mjs tests/ship.test.mjs docs/superpowers/plans/2026-04-09-auto-ship-release.md
git commit -m "feat: add one-command ship workflow"
```
