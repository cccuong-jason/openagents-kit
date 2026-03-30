# OpenAgents npx Global Installer Design

## Summary

`npx openagents-kit` should install the latest OpenAgents binary in a way that makes the next `openagents-kit` command resolve to that same version without extra manual steps. The installer should own a canonical install location per OS, repair user PATH automatically when needed, and refresh an already-winning user-owned `openagents-kit` binary when one exists earlier on PATH.

## Goals

- Make `npx openagents-kit` the only command users need to remember for installation and upgrades.
- Keep OpenAgents installed in an OpenAgents-owned location rather than a language-tooling directory.
- Ensure fresh installs and upgrades work across Windows, macOS, and Linux.
- Avoid overwriting unrelated or system-managed binaries.

## Canonical Install Locations

- Windows: `%LOCALAPPDATA%\OpenAgents\bin\openagents-kit.exe`
- macOS: `~/.local/bin/openagents-kit`
- Linux: `~/.local/bin/openagents-kit`

These are the source-of-truth destinations for the installer.

## Install Flow

1. Detect platform and architecture.
2. Resolve the release asset from GitHub Releases.
3. Download the release asset to a temporary file.
4. Install the binary into the canonical OpenAgents-owned location.
5. Detect whether the canonical install directory is on the user PATH.
6. If missing, update the user PATH automatically.
7. Detect whether `openagents-kit` already resolves to an older binary in a user-owned writable directory.
8. If so, refresh that winning binary so the next shell invocation uses the latest version immediately.
9. Print a success summary that explains what was updated.

## PATH Repair

### Windows

- Inspect the user PATH from the registry and current process environment.
- If `%LOCALAPPDATA%\OpenAgents\bin` is missing, update the user PATH in `HKCU\Environment`.
- Broadcast or message that a new shell may be needed for the PATH update to be picked up.

### macOS / Linux

- If `~/.local/bin` is missing from PATH, append a managed PATH export snippet to the best shell profile candidate.
- Prefer existing shell profiles associated with the active shell, then fall back to common profiles such as:
  - `~/.zshrc`
  - `~/.bashrc`
  - `~/.profile`
- The snippet should be idempotent so repeated installs do not add duplicate entries.

## Winning Binary Refresh

If `openagents-kit` already exists on PATH:

- Only refresh it when the path is user-owned and writable.
- Only refresh obvious user install locations, such as:
  - Windows user profile locations including `.cargo\bin`, `.local\bin`, `%LOCALAPPDATA%\OpenAgents\bin`
  - macOS/Linux paths under the user home directory, especially `~/.local/bin`
- Do not overwrite system-managed locations like `/usr/local/bin`, Homebrew-managed prefixes, or other non-user paths.

## Success Output

The installer should report:

- canonical install path
- whether PATH was updated
- whether a previously winning binary was refreshed
- whether opening a new shell is recommended

## Testing

- Unit tests for canonical install directory resolution
- Unit tests for PATH parsing and missing-path detection
- Unit tests for safe refresh candidate selection
- Unit tests for shell profile choice and managed PATH snippet generation
- Smoke test for the installer script on Windows path shadowing scenarios

## Constraints

- The npm wrapper stays thin; it only downloads, installs, repairs PATH, and refreshes compatible user-owned binaries.
- System-wide PATH or admin-only install locations are out of scope.
- We prefer safe predictability over aggressive takeover of arbitrary binaries.
