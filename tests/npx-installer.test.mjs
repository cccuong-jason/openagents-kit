import test from 'node:test';
import assert from 'node:assert/strict';

import {
  resolveAssetName,
  resolveInstallDir,
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

test('resolves user-bin install directories by platform', () => {
  assert.equal(
    resolveInstallDir({ platform: 'win32', homeDir: 'C:\\Users\\jason' }),
    'C:\\Users\\jason\\.local\\bin',
  );
  assert.equal(
    resolveInstallDir({ platform: 'darwin', homeDir: '/Users/jason' }),
    '/Users/jason/.local/bin',
  );
});
