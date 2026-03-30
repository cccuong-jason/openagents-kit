import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildPosixPathSnippet,
  findRefreshCandidate,
  isPathOnEnvPath,
  resolveCanonicalInstallDir,
  resolveAssetName,
  resolveInstallDir,
  resolvePosixProfilePath,
  shouldRefreshExistingBinary,
  summarizeInstall,
} from '../scripts/npx-installer.mjs';

test('resolves release asset names for supported platforms', () => {
  assert.equal(
    resolveAssetName({ platform: 'win32', arch: 'x64' }),
    'openagents-kit-x86_64-pc-windows-msvc.exe',
  );
  assert.equal(
    resolveAssetName({ platform: 'darwin', arch: 'arm64' }),
    'openagents-kit-aarch64-apple-darwin',
  );
  assert.equal(
    resolveAssetName({ platform: 'linux', arch: 'x64' }),
    'openagents-kit-x86_64-unknown-linux-gnu',
  );
});

test('resolves install directories by platform', () => {
  assert.equal(
    resolveInstallDir({
      platform: 'win32',
      homeDir: 'C:\\Users\\jason',
      localAppData: 'C:\\Users\\jason\\AppData\\Local',
    }),
    'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin',
  );
  assert.equal(
    resolveInstallDir({ platform: 'darwin', homeDir: '/Users/jason' }),
    '/Users/jason/.local/bin',
  );
});

test('resolves canonical install directories by platform', () => {
  assert.equal(
    resolveCanonicalInstallDir({
      platform: 'win32',
      homeDir: 'C:\\Users\\jason',
      localAppData: 'C:\\Users\\jason\\AppData\\Local',
    }),
    'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin',
  );
  assert.equal(
    resolveCanonicalInstallDir({
      platform: 'darwin',
      homeDir: '/Users/jason',
    }),
    '/Users/jason/.local/bin',
  );
});

test('detects whether a path is already on PATH', () => {
  assert.equal(
    isPathOnEnvPath({
      candidate: 'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin',
      envPath: [
        'C:\\Windows\\System32',
        'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin',
      ].join(';'),
      platform: 'win32',
    }),
    true,
  );
  assert.equal(
    isPathOnEnvPath({
      candidate: '/Users/jason/.local/bin',
      envPath: '/usr/bin:/Users/jason/bin',
      platform: 'linux',
    }),
    false,
  );
});

test('refreshes only safe user-owned winning binaries', () => {
  assert.equal(
    shouldRefreshExistingBinary({
      resolvedBinaryPath: 'C:\\Users\\jason\\.cargo\\bin\\openagents-kit.exe',
      canonicalBinaryPath: 'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin\\openagents-kit.exe',
      platform: 'win32',
      homeDir: 'C:\\Users\\jason',
    }),
    true,
  );
  assert.equal(
    shouldRefreshExistingBinary({
      resolvedBinaryPath: 'C:\\Program Files\\OpenAgents\\openagents-kit.exe',
      canonicalBinaryPath: 'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin\\openagents-kit.exe',
      platform: 'win32',
      homeDir: 'C:\\Users\\jason',
    }),
    false,
  );
});

test('finds the first safe refresh candidate on PATH', () => {
  assert.equal(
    findRefreshCandidate({
      resolvedBinaryPaths: [
        'C:\\Users\\jason\\.cargo\\bin\\openagents-kit.exe',
        'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin\\openagents-kit.exe',
      ],
      canonicalBinaryPath: 'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin\\openagents-kit.exe',
      platform: 'win32',
      homeDir: 'C:\\Users\\jason',
    }),
    'C:\\Users\\jason\\.cargo\\bin\\openagents-kit.exe',
  );
});

test('chooses a likely POSIX shell profile and builds an idempotent PATH snippet', () => {
  assert.equal(
    resolvePosixProfilePath({
      homeDir: '/Users/jason',
      shellPath: '/bin/zsh',
      existingProfiles: ['/Users/jason/.zshrc'],
    }),
    '/Users/jason/.zshrc',
  );
  assert.equal(
    buildPosixPathSnippet('/Users/jason/.local/bin'),
    '\n# Added by OpenAgents installer\nexport PATH="/Users/jason/.local/bin:$PATH"\n',
  );
});

test('summarizes install side effects for user messaging', () => {
  assert.deepEqual(
    summarizeInstall({
      canonicalInstallDir: 'C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin',
      pathUpdated: true,
      refreshedBinary: 'C:\\Users\\jason\\.cargo\\bin\\openagents-kit.exe',
    }),
    [
      'Installed to C:\\Users\\jason\\AppData\\Local\\OpenAgents\\bin',
      'Updated your user PATH to include the OpenAgents bin directory.',
      'Refreshed the existing PATH-winning binary at C:\\Users\\jason\\.cargo\\bin\\openagents-kit.exe.',
      'Open a new shell if your current terminal does not pick up the PATH change immediately.',
    ],
  );
});
