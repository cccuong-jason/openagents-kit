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
& "$HOME\.local\bin\openagents-kit.exe" --help
```

If the shell still resolves an old binary from `C:\Users\jason\.cargo\bin\openagents-kit.exe`, replace it with the fresh release binary:

```powershell
Copy-Item "$HOME\.local\bin\openagents-kit.exe" "$HOME\.cargo\bin\openagents-kit.exe" -Force
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
