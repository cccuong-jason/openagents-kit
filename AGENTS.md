# AGENTS.md

## Release Playbook

When shipping a new public version of `openagents-kit`, use this exact flow.

### Scope

- Do not stage or commit local testing artifacts such as:
  - `workspace.yaml`
  - `.openagents/`
  - `generated/`
- Only stage the intended version-bump or code-change files.

### Version bump

Update all user-facing version numbers together:

- `package.json`
- `crates/openagents-tui/Cargo.toml`
- `Cargo.lock` if the crate version changed there

Prefer the helper so they stay aligned:

```powershell
node scripts/bump-version.mjs X.Y.Z
```

Before publishing, always run the guard:

```powershell
npm run release:check
```

`npm publish` also runs this guard automatically through `prepublishOnly`, so stale or already-published versions fail fast with a clear message before npm tries to publish them.

### Local verification

On this Windows machine, prefer the real Rust toolchain binaries instead of the `cargo.exe` shim when verification is needed:

```powershell
$cargoBin = & "$HOME\.cargo\bin\rustup.exe" which cargo
$rustcBin = & "$HOME\.cargo\bin\rustup.exe" which rustc
$env:RUSTC = $rustcBin
$env:PATH = "$(Split-Path $rustcBin);$env:PATH"
```

Run:

```powershell
& $cargoBin fmt --all
& $cargoBin test --workspace --all-features
& $cargoBin clippy --workspace --all-targets --all-features -- -D warnings
& $cargoBin run -p openagents-tui --bin openagents-kit -- --help
& $cargoBin run -p openagents-tui --bin openagents-kit -- setup --dry-run
& $cargoBin run -p openagents-tui --bin openagents-kit -- catalog
```

### Git + release flow

Commit the version bump:

```powershell
git add package.json crates/openagents-tui/Cargo.toml Cargo.lock AGENTS.md
git commit -m "chore: release vX.Y.Z"
git push origin main
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

### Release verification

Watch the release workflow:

```powershell
gh run list --workflow release.yml --limit 3
gh run watch <run-id> --interval 10
```

Check the GitHub release:

```powershell
gh release view vX.Y.Z
```

### Installer verification

Preferred public-path verification:

```powershell
npx --yes github:cccuong-jason/openagents-kit#vX.Y.Z
openagents-kit --help
```

Expected install locations:

- Windows: `%LOCALAPPDATA%\OpenAgents\bin\openagents-kit.exe`
- macOS/Linux: `~/.local/bin/openagents-kit`

The installer also repairs the user `PATH` and refreshes an older user-owned `openagents-kit` that already wins on `PATH`, so `openagents-kit --help` should resolve to the new version without a manual copy step.

If verification still shows an older binary, inspect resolution before patching anything:

```powershell
where.exe openagents-kit
Get-Command openagents-kit -All | Format-Table -AutoSize CommandType,Name,Source
```

### npm publishing notes

- npm does not allow overwriting a published version. Always publish a newer version such as `0.3.1`, `0.3.2`, etc.
- If `npx openagents-kit` fails but `npx --yes github:cccuong-jason/openagents-kit#vX.Y.Z` works, the GitHub release is fine and the blocker is npm publishing.
- If `npm publish --access public` fails with `EOTP`, finish the browser auth prompt once and rerun the publish:

```powershell
npm publish --access public
```

- After the package exists on npm, configure trusted publishing so future GitHub releases do not depend on a token:

```powershell
npm trust github openagents-kit --repo cccuong-jason/openagents-kit --file release.yml --yes
```

- `npm trust github` also requires the one-time browser/2FA approval on this account.
- The release workflow now tries trusted publishing first and falls back to `NPM_TOKEN` if it is configured.
- If npm publish fails with `403` mentioning 2FA, use npm account 2FA or a granular access token with bypass 2FA enabled.
- If `npm whoami` returns `401` or `npm publish` returns a misleading `404` for an existing package, the local npm token is stale or invalid. Re-authenticate first:

```powershell
npm logout
npm login
npm whoami
```

- If GitHub Actions shows the same misleading `404`, rotate the `NPM_TOKEN` repository secret because the CI token is stale or does not have publish permission.
