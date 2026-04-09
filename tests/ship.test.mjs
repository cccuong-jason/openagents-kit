import test from 'node:test';
import assert from 'node:assert/strict';

import {
  computeNextPatchVersion,
  createShipPlan,
  filterUnexpectedDirtyPaths,
  parseShipArgs,
  resolveChildCommand,
} from '../scripts/ship-release.mjs';

test('computes the next patch version from the published npm version', () => {
  assert.equal(computeNextPatchVersion('0.3.4'), '0.3.5');
  assert.equal(computeNextPatchVersion('1.9.9'), '1.9.10');
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
