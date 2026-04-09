import test from 'node:test';
import assert from 'node:assert/strict';

import {
  compareVersions,
  createPublishGuardReport,
  resolveNpmInvocation,
  replaceVersionInCargoToml,
  replaceVersionInCargoLock,
  replaceVersionInPackageJson,
} from '../scripts/release-guard.mjs';

test('compares semantic versions correctly', () => {
  assert.equal(compareVersions('0.3.4', '0.3.4'), 0);
  assert.equal(compareVersions('0.3.5', '0.3.4'), 1);
  assert.equal(compareVersions('0.3.4', '0.3.5'), -1);
});

test('fails when npm and cargo versions do not match', () => {
  const report = createPublishGuardReport({
    packageVersion: '0.3.5',
    cargoVersion: '0.3.4',
    publishedVersion: '0.3.4',
  });

  assert.equal(report.ok, false);
  assert.match(
    report.message,
    /Version mismatch: package\.json is 0\.3\.5 but crates\/openagents-tui\/Cargo\.toml is 0\.3\.4\./,
  );
});

test('fails when the version was already published', () => {
  const report = createPublishGuardReport({
    packageVersion: '0.3.4',
    cargoVersion: '0.3.4',
    publishedVersion: '0.3.4',
  });

  assert.equal(report.ok, false);
  assert.match(
    report.message,
    /Refusing to publish openagents-kit 0\.3\.4 because npm already has 0\.3\.4/,
  );
  assert.match(report.message, /Bump the version first or update your checkout from origin\/main\./);
});

test('passes when local versions match and are newer than npm', () => {
  const report = createPublishGuardReport({
    packageVersion: '0.3.5',
    cargoVersion: '0.3.5',
    publishedVersion: '0.3.4',
  });

  assert.deepEqual(report, {
    ok: true,
    message: 'Release guard passed for openagents-kit 0.3.5.',
  });
});

test('updates package.json version content', () => {
  const updated = replaceVersionInPackageJson(
    '{\n  "name": "openagents-kit",\n  "version": "0.3.4"\n}\n',
    '0.3.5',
  );

  assert.match(updated, /"version": "0\.3\.5"/);
});

test('updates Cargo.toml version content', () => {
  const updated = replaceVersionInCargoToml(
    '[package]\nname = "openagents-tui"\nversion = "0.3.4"\n',
    '0.3.5',
  );

  assert.match(updated, /version = "0\.3\.5"/);
});

test('updates openagents-tui entry in Cargo.lock', () => {
  const updated = replaceVersionInCargoLock(
    '[[package]]\nname = "openagents-tui"\nversion = "0.3.4"\ndependencies = []\n',
    '0.3.5',
  );

  assert.match(updated, /name = "openagents-tui"\nversion = "0\.3\.5"/);
});

test('prefers node plus npm-cli.js when npm_execpath is available', () => {
  assert.deepEqual(
    resolveNpmInvocation('win32', { npm_execpath: 'C:/Users/jason/AppData/Roaming/npm/node_modules/npm/bin/npm-cli.js' }, 'node.exe'),
    {
      command: 'node.exe',
      prefixArgs: ['C:/Users/jason/AppData/Roaming/npm/node_modules/npm/bin/npm-cli.js'],
    },
  );
});

test('falls back to shell npm command when npm_execpath is unavailable', () => {
  assert.deepEqual(resolveNpmInvocation('linux', {}, 'node'), {
    command: 'npm',
    prefixArgs: [],
  });
  assert.deepEqual(resolveNpmInvocation('win32', {}, 'node.exe'), {
    command: 'cmd.exe',
    prefixArgs: ['/d', '/s', '/c', 'npm'],
  });
});
