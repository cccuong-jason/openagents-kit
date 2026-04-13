import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildVerificationCommands,
  computeNextPatchVersion,
  createShipPlan,
  filterUnexpectedDirtyPaths,
  parseShipArgs,
  resolveLatestTaggedVersion,
  shouldCreateReleaseCommit,
  selectPublishedBaselineVersion,
  resolveReleaseVersionState,
  resolveChildCommand,
} from '../scripts/ship-release.mjs';

test('computes the next patch version from the published npm version', () => {
  assert.equal(computeNextPatchVersion('0.3.4'), '0.3.5');
  assert.equal(computeNextPatchVersion('1.9.9'), '1.9.10');
});

test('finds the highest semantic version from release tags', () => {
  assert.equal(resolveLatestTaggedVersion(['v0.3.4', 'v0.3.5', 'notes']), '0.3.5');
  assert.equal(resolveLatestTaggedVersion(['release', 'foo']), null);
});

test('uses the newer of npm and git tag release baselines', () => {
  assert.equal(selectPublishedBaselineVersion('0.3.4', '0.3.5'), '0.3.5');
  assert.equal(selectPublishedBaselineVersion('0.3.6', '0.3.5'), '0.3.6');
  assert.equal(selectPublishedBaselineVersion('0.3.4', null), '0.3.4');
});

test('resumes a pending release when local version is exactly one patch ahead of npm', () => {
  assert.deepEqual(resolveReleaseVersionState('0.3.5', '0.3.5', '0.3.4'), {
    packageVersion: '0.3.5',
    cargoVersion: '0.3.5',
    publishedVersion: '0.3.4',
    targetVersion: '0.3.5',
    mode: 'resume',
  });
});

test('auto-bumps when local version still matches npm', () => {
  assert.deepEqual(resolveReleaseVersionState('0.3.4', '0.3.4', '0.3.4'), {
    packageVersion: '0.3.4',
    cargoVersion: '0.3.4',
    publishedVersion: '0.3.4',
    targetVersion: '0.3.5',
    mode: 'bump',
  });
});

test('creates a release commit only for fresh bumps', () => {
  assert.equal(shouldCreateReleaseCommit('bump'), true);
  assert.equal(shouldCreateReleaseCommit('resume'), false);
});

test('rejects ambiguous multi-version drift ahead of npm', () => {
  assert.throws(
    () => resolveReleaseVersionState('0.3.7', '0.3.7', '0.3.4'),
    /Expected local version 0\.3\.7 to match the published npm version 0\.3\.4 or be exactly one patch ahead/,
  );
});

test('filters allowed local artifacts out of the dirty path list', () => {
  const dirtyPaths = [
    ' M workspace.yaml',
    ' M package.json',
    '?? .openagents/history.json',
    '?? generated/codex/config.toml',
    ' M scripts/npx-installer.mjs',
  ];

  assert.deepEqual(filterUnexpectedDirtyPaths(dirtyPaths), [
    ' M package.json',
    ' M scripts/npx-installer.mjs',
  ]);
});

test('parses ship flags', () => {
  assert.deepEqual(parseShipArgs([]), { yes: false, dryRun: false });
  assert.deepEqual(parseShipArgs(['--yes']), { yes: true, dryRun: false });
  assert.deepEqual(parseShipArgs(['--dry-run']), { yes: false, dryRun: true });
  assert.deepEqual(parseShipArgs(['--yes', '--dry-run']), { yes: true, dryRun: true });
});

test('parses npm config fallback flags', () => {
  assert.deepEqual(parseShipArgs([], { npm_config_dry_run: 'true' }), { yes: false, dryRun: true });
  assert.deepEqual(parseShipArgs([], { npm_config_yes: 'true' }), { yes: true, dryRun: false });
  assert.deepEqual(parseShipArgs([], { npm_config_yes: 'true', npm_config_dry_run: 'true' }), { yes: true, dryRun: true });
});

test('ignores ambient npm config flags unless they are passed explicitly', () => {
  process.env.npm_config_yes = 'true';
  process.env.npm_config_dry_run = 'true';

  try {
    assert.deepEqual(parseShipArgs([]), { yes: false, dryRun: false });
  } finally {
    delete process.env.npm_config_yes;
    delete process.env.npm_config_dry_run;
  }
});

test('rejects unknown ship flags', () => {
  assert.throws(() => parseShipArgs(['--wat']), /Unknown argument "--wat"/);
});

test('uses npm.cmd on windows child process calls', () => {
  assert.equal(resolveChildCommand('npm', 'win32'), 'npm.cmd');
  assert.equal(resolveChildCommand('git', 'win32'), 'git.exe');
  assert.equal(resolveChildCommand('npm', 'linux'), 'npm');
});

test('creates a dry-run ship plan summary', () => {
  const plan = createShipPlan({
    publishedVersion: '0.3.4',
    nextVersion: '0.3.5',
    branch: 'main',
    dirtyPaths: [],
    dryRun: true,
    yes: false,
  });

  assert.equal(plan.ok, true);
  assert.match(plan.message, /About to ship openagents-kit 0\.3\.5/);
  assert.match(plan.message, /Mode: dry-run/);
  assert.match(plan.message, /Will update: package\.json, crates\/openagents-tui\/Cargo\.toml, Cargo\.lock/);
});

test('blocks ship when unexpected dirty files are present', () => {
  const plan = createShipPlan({
    publishedVersion: '0.3.4',
    nextVersion: '0.3.5',
    branch: 'main',
    dirtyPaths: [' M scripts/npx-installer.mjs'],
    dryRun: false,
    yes: false,
  });

  assert.equal(plan.ok, false);
  assert.match(plan.message, /Refusing to ship with unexpected dirty files/);
  assert.match(plan.message, /scripts\/npx-installer\.mjs/);
});

test('blocks ship outside main', () => {
  const plan = createShipPlan({
    publishedVersion: '0.3.4',
    nextVersion: '0.3.5',
    branch: 'feat/test',
    dirtyPaths: [],
    dryRun: false,
    yes: false,
  });

  assert.equal(plan.ok, false);
  assert.match(plan.message, /Refusing to ship from branch "feat\/test"/);
});

test('verification suite uses a hermetic catalog smoke check', () => {
  const catalogCommand = buildVerificationCommands().find((entry) => (
    entry.command === 'cargo' && entry.args.includes('catalog')
  ));

  assert.ok(catalogCommand);
  assert.deepEqual(catalogCommand.args.slice(-2), ['catalog', '--help']);
});
