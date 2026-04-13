import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import readline from 'node:readline/promises';
import { execFileSync } from 'node:child_process';

import {
  compareVersions,
  createPublishGuardReport,
  readCargoVersion,
  readPackageVersion,
  replaceVersionInCargoLock,
  replaceVersionInCargoToml,
  replaceVersionInPackageJson,
  resolveNpmInvocation,
  resolvePublishedVersion,
} from './release-guard.mjs';

const ROOT_DIR = path.resolve(import.meta.dirname, '..');
const PACKAGE_JSON_PATH = path.join(ROOT_DIR, 'package.json');
const CARGO_TOML_PATH = path.join(ROOT_DIR, 'crates', 'openagents-tui', 'Cargo.toml');
const CARGO_LOCK_PATH = path.join(ROOT_DIR, 'Cargo.lock');
const VERSIONED_FILES = [
  'package.json',
  'crates/openagents-tui/Cargo.toml',
  'Cargo.lock',
];
const ALLOWED_DIRTY_PATHS = [
  'workspace.yaml',
  '.openagents/',
  'generated/',
];

export function computeNextPatchVersion(version) {
  const match = version.match(/^(\d+)\.(\d+)\.(\d+)$/);
  if (!match) {
    throw new Error(`Expected a semantic version like 0.3.4, received "${version}".`);
  }

  const [, major, minor, patch] = match;
  return `${major}.${minor}.${Number.parseInt(patch, 10) + 1}`;
}

export function resolveLatestTaggedVersion(tagNames) {
  return tagNames
    .map((tagName) => {
      const match = tagName.trim().match(/^v(\d+\.\d+\.\d+)$/);
      return match ? match[1] : null;
    })
    .filter(Boolean)
    .sort(compareVersions)
    .at(-1) ?? null;
}

export function selectPublishedBaselineVersion(npmVersion, taggedVersion) {
  if (!taggedVersion) {
    return npmVersion;
  }

  return compareVersions(taggedVersion, npmVersion) > 0 ? taggedVersion : npmVersion;
}

export function resolveReleaseVersionState(packageVersion, cargoVersion, publishedVersion) {
  if (packageVersion !== cargoVersion) {
    throw new Error(
      `Version mismatch before shipping: package.json is ${packageVersion} but crates/openagents-tui/Cargo.toml is ${cargoVersion}.`,
    );
  }

  if (compareVersions(packageVersion, publishedVersion) === 0) {
    return {
      packageVersion,
      cargoVersion,
      publishedVersion,
      targetVersion: computeNextPatchVersion(publishedVersion),
      mode: 'bump',
    };
  }

  if (compareVersions(packageVersion, computeNextPatchVersion(publishedVersion)) === 0) {
    return {
      packageVersion,
      cargoVersion,
      publishedVersion,
      targetVersion: packageVersion,
      mode: 'resume',
    };
  }

  throw new Error(
    `Expected local version ${packageVersion} to match the published npm version ${publishedVersion} or be exactly one patch ahead so ship can resume a pending release.`,
  );
}

function isTruthyConfigValue(value) {
  return value === 'true' || value === '1' || value === true;
}

export function parseShipArgs(argv, env = {}) {
  const flags = {
    yes: isTruthyConfigValue(env.npm_config_yes),
    dryRun: isTruthyConfigValue(env.npm_config_dry_run),
  };

  for (const arg of argv) {
    if (arg === '--yes') {
      flags.yes = true;
      continue;
    }
    if (arg === '--dry-run') {
      flags.dryRun = true;
      continue;
    }

    throw new Error(`Unknown argument "${arg}". Supported flags: --yes, --dry-run.`);
  }

  return flags;
}

function normalizeStatusPath(line) {
  return line.replace(/^[ MARCUD?!]{1,2}\s+/, '').trim();
}

function isAllowedDirtyPath(pathText) {
  const normalized = pathText.replaceAll('\\', '/');
  return ALLOWED_DIRTY_PATHS.some((allowed) => (
    allowed.endsWith('/') ? normalized.startsWith(allowed) : normalized === allowed
  ));
}

export function filterUnexpectedDirtyPaths(statusLines) {
  return statusLines.filter(Boolean).filter((line) => {
    const dirtyPath = normalizeStatusPath(line);
    return !isAllowedDirtyPath(dirtyPath);
  });
}

export function createShipPlan({
  publishedVersion,
  nextVersion,
  branch,
  dirtyPaths,
  dryRun,
  yes,
}) {
  const unexpectedDirtyPaths = filterUnexpectedDirtyPaths(dirtyPaths);

  if (branch !== 'main') {
    return {
      ok: false,
      message: `Refusing to ship from branch "${branch}". Switch to main first.`,
    };
  }

  if (unexpectedDirtyPaths.length > 0) {
    return {
      ok: false,
      message: `Refusing to ship with unexpected dirty files:\n- ${unexpectedDirtyPaths.join('\n- ')}`,
    };
  }

  const mode = dryRun ? 'dry-run' : yes ? 'non-interactive' : 'interactive';

  return {
    ok: true,
    message: [
      `About to ship openagents-kit ${nextVersion}`,
      `Published release baseline: ${publishedVersion}`,
      `Mode: ${mode}`,
      `Will update: ${VERSIONED_FILES.join(', ')}`,
      'Will run: release guard, Rust verification, git commit/tag/push, npm publish, npm version verify',
    ].join('\n'),
  };
}

function readFile(filePath) {
  return fs.readFileSync(filePath, 'utf8');
}

function writeFile(filePath, contents) {
  fs.writeFileSync(filePath, contents, 'utf8');
}

export function resolveChildCommand(command, platform = process.platform) {
  if (platform === 'win32') {
    if (command === 'npm') return 'npm.cmd';
    if (command === 'git') return 'git.exe';
  }

  return command;
}

function run(command, args, options = {}) {
  if (command === 'npm') {
    const npmInvocation = resolveNpmInvocation();
    return execFileSync(npmInvocation.command, [...npmInvocation.prefixArgs, ...args], {
      cwd: ROOT_DIR,
      encoding: 'utf8',
      stdio: options.stdio ?? 'pipe',
      env: options.env ?? process.env,
    });
  }

  return execFileSync(resolveChildCommand(command), args, {
    cwd: ROOT_DIR,
    encoding: 'utf8',
    stdio: options.stdio ?? 'pipe',
    env: options.env ?? process.env,
  });
}

function runLoud(command, args, options = {}) {
  if (command === 'npm') {
    const npmInvocation = resolveNpmInvocation();
    execFileSync(npmInvocation.command, [...npmInvocation.prefixArgs, ...args], {
      cwd: ROOT_DIR,
      stdio: 'inherit',
      env: options.env ?? process.env,
    });
    return;
  }

  execFileSync(resolveChildCommand(command), args, {
    cwd: ROOT_DIR,
    stdio: 'inherit',
    env: options.env ?? process.env,
  });
}

function getCurrentBranch() {
  return run('git', ['branch', '--show-current']).trim();
}

function getStatusLines() {
  return run('git', ['status', '--short'])
    .split(/\r?\n/)
    .map((line) => line.trimEnd())
    .filter(Boolean);
}

function getTagNames() {
  return run('git', ['tag', '--list', 'v*'])
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function resolveCargoExecution() {
  if (process.platform !== 'win32') {
    return { cargoBin: 'cargo', env: process.env };
  }

  const rustupBin = path.join(os.homedir(), '.cargo', 'bin', 'rustup.exe');
  if (!fs.existsSync(rustupBin)) {
    return { cargoBin: 'cargo', env: process.env };
  }

  try {
    const cargoBin = execFileSync(rustupBin, ['which', 'cargo'], {
      cwd: ROOT_DIR,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    const rustcBin = execFileSync(rustupBin, ['which', 'rustc'], {
      cwd: ROOT_DIR,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
    return {
      cargoBin,
      env: {
        ...process.env,
        RUSTC: rustcBin,
        PATH: `${path.dirname(rustcBin)};${process.env.PATH ?? ''}`,
      },
    };
  } catch {
    return { cargoBin: 'cargo', env: process.env };
  }
}

function ensureVersionsAreAlignedWithPublished(packageVersion, cargoVersion, publishedVersion) {
  return resolveReleaseVersionState(packageVersion, cargoVersion, publishedVersion);
}

function applyVersionBump(nextVersion) {
  writeFile(
    PACKAGE_JSON_PATH,
    replaceVersionInPackageJson(readFile(PACKAGE_JSON_PATH), nextVersion),
  );
  writeFile(
    CARGO_TOML_PATH,
    replaceVersionInCargoToml(readFile(CARGO_TOML_PATH), nextVersion),
  );
  writeFile(
    CARGO_LOCK_PATH,
    replaceVersionInCargoLock(readFile(CARGO_LOCK_PATH), nextVersion),
  );
}

async function confirmRelease(nextVersion) {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
  });

  try {
    const answer = await rl.question(`Ship openagents-kit ${nextVersion}? [y/N] `);
    return /^y(es)?$/i.test(answer.trim());
  } finally {
    rl.close();
  }
}

function verifyReleaseGuard(nextVersion, publishedVersion) {
  const report = createPublishGuardReport({
    packageVersion: nextVersion,
    cargoVersion: nextVersion,
    publishedVersion,
  });

  if (!report.ok) {
    throw new Error(report.message);
  }
}

export function buildVerificationCommands() {
  return [
    { command: 'node', args: ['--test', 'tests/release-guard.test.mjs', 'tests/ship.test.mjs', 'tests/workflow-release.test.mjs'] },
    { command: 'npm', args: ['run', 'release:check'] },
    { command: 'cargo', args: ['fmt', '--all'] },
    { command: 'cargo', args: ['test', '--workspace', '--all-features'] },
    { command: 'cargo', args: ['clippy', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings'] },
    { command: 'cargo', args: ['run', '-p', 'openagents-tui', '--bin', 'openagents-kit', '--', '--help'] },
    { command: 'cargo', args: ['run', '-p', 'openagents-tui', '--bin', 'openagents-kit', '--', 'setup', '--dry-run'] },
    { command: 'cargo', args: ['run', '-p', 'openagents-tui', '--bin', 'openagents-kit', '--', 'catalog', '--help'] },
  ];
}

export function shouldCreateReleaseCommit(mode) {
  return mode === 'bump';
}

function runVerificationSuite() {
  const { cargoBin, env } = resolveCargoExecution();
  for (const step of buildVerificationCommands()) {
    const command = step.command === 'cargo' ? cargoBin : step.command;
    const commandEnv = step.command === 'cargo' ? env : process.env;
    runLoud(command, step.args, { env: commandEnv });
  }
}

function runGitAndPublish(nextVersion, mode) {
  if (shouldCreateReleaseCommit(mode)) {
    runLoud('git', ['add', ...VERSIONED_FILES]);
    runLoud('git', ['commit', '-m', `chore: release v${nextVersion}`]);
  }
  runLoud('git', ['push', 'origin', 'main']);
  runLoud('git', ['tag', '-a', `v${nextVersion}`, '-m', `v${nextVersion}`]);
  runLoud('git', ['push', 'origin', `v${nextVersion}`]);
  runLoud('npm', ['publish', '--access', 'public']);
}

function verifyPublishedVersion(nextVersion) {
  const publishedVersion = run('npm', ['view', 'openagents-kit', 'version']).trim();
  if (publishedVersion !== nextVersion) {
    throw new Error(`Expected npm to report ${nextVersion} after publish, but it returned ${publishedVersion}.`);
  }
}

export async function main(argv = process.argv.slice(2)) {
  const { yes, dryRun } = parseShipArgs(argv, process.env);
  const packageVersion = readPackageVersion(readFile(PACKAGE_JSON_PATH));
  const cargoVersion = readCargoVersion(readFile(CARGO_TOML_PATH));
  const publishedVersion = resolvePublishedVersion();
  const taggedVersion = resolveLatestTaggedVersion(getTagNames());
  const baselineVersion = selectPublishedBaselineVersion(publishedVersion, taggedVersion);
  const versionState = ensureVersionsAreAlignedWithPublished(packageVersion, cargoVersion, baselineVersion);
  const nextVersion = versionState.targetVersion;
  const branch = getCurrentBranch();
  const dirtyPaths = getStatusLines();
  const plan = createShipPlan({
    publishedVersion: baselineVersion,
    nextVersion,
    branch,
    dirtyPaths,
    dryRun,
    yes,
  });

  if (!plan.ok) {
    throw new Error(plan.message);
  }

  console.log(plan.message);

  if (taggedVersion && compareVersions(taggedVersion, publishedVersion) !== 0) {
    console.log(`Latest Git tag version: ${taggedVersion}`);
  }

  if (versionState.mode === 'resume') {
    console.log(`Resuming pending release ${nextVersion} because the published baseline is still on ${baselineVersion}.`);
  }

  if (dryRun) {
    return;
  }

  if (!yes) {
    const confirmed = await confirmRelease(nextVersion);
    if (!confirmed) {
      throw new Error('Release cancelled.');
    }
  }

  if (versionState.mode === 'bump') {
    applyVersionBump(nextVersion);
  }
  verifyReleaseGuard(nextVersion, publishedVersion);
  runVerificationSuite();
  runGitAndPublish(nextVersion, versionState.mode);
  verifyPublishedVersion(nextVersion);

  console.log(`Ship complete: openagents-kit ${nextVersion}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.filename)) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
