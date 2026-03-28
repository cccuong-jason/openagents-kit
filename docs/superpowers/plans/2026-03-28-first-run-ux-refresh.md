# First-Run UX Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current first-run TUI with a guided concierge flow that has a stronger OpenAgents identity and more explicit user guidance.

**Architecture:** Keep the existing detection and manifest-generation pipeline, but refactor the setup TUI into explicit screens and wizard steps. Rebuild the visual language around a new mascot, cool-toned palette, boot animation, and clearer footer guidance.

**Tech Stack:** Rust, clap, crossterm, ratatui, tempfile

---

### Task 1: Lock screen-state helpers behind tests

**Files:**
- Modify: `crates/openagents-tui/src/main.rs`

- [ ] **Step 1: Write failing tests for setup screen helpers**
- [ ] **Step 2: Run the targeted test command and verify the new tests fail for the expected reason**
- [ ] **Step 3: Add minimal helper types/functions for screen prompts, controls, and wizard defaults**
- [ ] **Step 4: Run the targeted tests again and verify they pass**

### Task 2: Implement the redesigned setup flow

**Files:**
- Modify: `crates/openagents-tui/src/main.rs`

- [ ] **Step 1: Write failing tests for wizard-step behavior and recommendation/guided transitions**
- [ ] **Step 2: Run the targeted test command and verify the tests fail**
- [ ] **Step 3: Refactor `SetupApp` and the setup event loop to support boot, recommendation, guided steps, and completion**
- [ ] **Step 4: Run the targeted tests again and verify they pass**

### Task 3: Refresh the visual language

**Files:**
- Modify: `crates/openagents-tui/src/main.rs`

- [ ] **Step 1: Write failing tests for footer text and any extracted copy helpers affected by the visual refresh**
- [ ] **Step 2: Run the targeted test command and verify the tests fail**
- [ ] **Step 3: Replace the mascot, palette, loading state, and screen copy with the approved direction**
- [ ] **Step 4: Run the targeted tests again and verify they pass**

### Task 4: Verify no behavior regressions

**Files:**
- Modify: `crates/openagents-tui/src/main.rs`

- [ ] **Step 1: Run `cargo test -p openagents-tui`**
- [ ] **Step 2: Run `cargo test --workspace --all-features`**
- [ ] **Step 3: Run `cargo run -p openagents-tui --bin openagents-kit -- setup --dry-run`**
- [ ] **Step 4: Run `cargo run -p openagents-tui --bin openagents-kit -- --help`**
