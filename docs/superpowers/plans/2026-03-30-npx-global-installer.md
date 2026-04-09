# npx Global Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `npx openagents-kit` install the latest binary in a canonical global location and ensure `openagents-kit` resolves to that new version on Windows, macOS, and Linux without manual copy or PATH steps.

**Architecture:** Extend the existing npm installer with explicit install-path, PATH-repair, and user-owned shadow-binary refresh helpers. Keep the release asset download flow intact while adding platform-specific user PATH handling and tests around candidate selection and safe overwrite rules.

**Tech Stack:** Node.js, built-in `fs`/`path`/`os`, Windows registry commands via PowerShell, POSIX shell profile editing, node:test

---

### Task 1: Expand installer behavior with path and overwrite helpers

**Files:**
- Modify: `scripts/npx-installer.mjs`
- Test: `tests/npx-installer.test.mjs`

- [ ] **Step 1: Write the failing tests for canonical install paths and overwrite decisions**

Add tests for:
- Windows canonical install path resolving to `%LOCALAPPDATA%\OpenAgents\bin`
- POSIX canonical install path resolving to `~/.local/bin`
- choosing a writable existing user-owned `openagents-kit` on PATH for refresh
- rejecting system-managed or unrelated paths

- [ ] **Step 2: Run the installer tests to verify they fail**

Run: `node --test tests/npx-installer.test.mjs`
Expected: FAIL because the current installer always installs to a fixed directory and has no PATH/overwrite helpers.

- [ ] **Step 3: Implement canonical install dir, PATH parsing, and safe refresh helpers**

Add small focused helpers in `scripts/npx-installer.mjs` for:
- canonical install dir resolution
- PATH splitting and normalization
- refresh-candidate detection
- safe user-owned path checks

- [ ] **Step 4: Run the installer tests to verify they pass**

Run: `node --test tests/npx-installer.test.mjs`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add scripts/npx-installer.mjs tests/npx-installer.test.mjs
git commit -m "feat: improve installer path resolution"
```

### Task 2: Add automatic PATH repair and success reporting

**Files:**
- Modify: `scripts/npx-installer.mjs`
- Test: `tests/npx-installer.test.mjs`

- [ ] **Step 1: Write the failing tests for PATH repair helpers**

Add tests for:
- Windows PATH missing detection and registry update command generation
- POSIX shell profile selection and idempotent PATH snippet generation
- success summary flags for `pathUpdated` and `refreshedBinary`

- [ ] **Step 2: Run the installer tests to verify they fail**

Run: `node --test tests/npx-installer.test.mjs`
Expected: FAIL because PATH repair helpers do not exist yet.

- [ ] **Step 3: Implement PATH repair and summary reporting**

Implement:
- Windows user PATH update through `powershell.exe`
- POSIX shell profile update with managed markers
- final output lines describing the install path, PATH change, and refreshed binary behavior

- [ ] **Step 4: Run the installer tests to verify they pass**

Run: `node --test tests/npx-installer.test.mjs`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add scripts/npx-installer.mjs tests/npx-installer.test.mjs
git commit -m "feat: add installer path repair"
```

### Task 3: Update docs and verify the package contract

**Files:**
- Modify: `README.md`
- Modify: `AGENTS.md`
- Test: `package.json`

- [ ] **Step 1: Write the documentation updates**

Document that:
- `npx openagents-kit` installs to an OpenAgents-owned global location
- PATH is repaired automatically
- older user-owned `openagents-kit` binaries are refreshed when safe

- [ ] **Step 2: Verify the npm package contents still look right**

Run: `npm pack --dry-run`
Expected: package contains only the installer wrapper and package metadata.

- [ ] **Step 3: Run the full installer verification commands**

Run:
- `node --test tests/npx-installer.test.mjs`
- `npm pack --dry-run`

Expected: all pass

- [ ] **Step 4: Commit**

```bash
git add README.md AGENTS.md
git commit -m "docs: explain global installer behavior"
```
