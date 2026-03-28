#!/usr/bin/env node

import fs from 'node:fs/promises';
import https from 'node:https';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = process.env.OPENAGENTS_REPO ?? 'cccuong-jason/openagents-kit';

export function resolveAssetName({ platform, arch }) {
  if (platform === 'win32' && arch === 'x64') {
    return 'openagents-kit-x86_64-pc-windows-msvc.exe';
  }
  if (platform === 'darwin' && arch === 'x64') {
    return 'openagents-kit-x86_64-apple-darwin';
  }
  if (platform === 'darwin' && arch === 'arm64') {
    return 'openagents-kit-aarch64-apple-darwin';
  }
  if (platform === 'linux' && arch === 'x64') {
    return 'openagents-kit-x86_64-unknown-linux-gnu';
  }

  throw new Error(`Unsupported platform: ${platform} ${arch}`);
}

export function resolveInstallDir({ platform, homeDir }) {
  if (platform === 'win32') {
    return path.join(homeDir, '.local', 'bin');
  }

  return `${homeDir}/.local/bin`;
}

export function resolveBinaryName(platform) {
  return platform === 'win32' ? 'openagents-kit.exe' : 'openagents-kit';
}

export function resolveReleaseTag(packageVersion) {
  if (!packageVersion || packageVersion === '0.0.0-dev') {
    return 'latest';
  }

  return `v${packageVersion}`;
}

export function resolveDownloadUrl({ repo = REPO, version, assetName }) {
  if (version === 'latest') {
    return `https://github.com/${repo}/releases/latest/download/${assetName}`;
  }

  return `https://github.com/${repo}/releases/download/${version}/${assetName}`;
}

export function pathInstruction({ platform, installDir }) {
  if (platform === 'win32') {
    return `Add ${installDir} to your PATH if openagents-kit is not recognized yet.`;
  }

  return `Add ${installDir} to your PATH if needed, then run openagents-kit.`;
}

async function downloadFile(url, destination, redirectCount = 0) {
  if (redirectCount > 5) {
    throw new Error('Too many redirects while downloading the OpenAgents binary.');
  }

  await new Promise((resolve, reject) => {
    const request = https.get(url, response => {
      const statusCode = response.statusCode ?? 0;

      if ([301, 302, 303, 307, 308].includes(statusCode)) {
        const redirect = response.headers.location;
        response.resume();
        if (!redirect) {
          reject(new Error('GitHub release redirected without a location header.'));
          return;
        }

        downloadFile(redirect, destination, redirectCount + 1).then(resolve, reject);
        return;
      }

      if (statusCode !== 200) {
        response.resume();
        reject(new Error(`Download failed with status ${statusCode}.`));
        return;
      }

      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.on('end', async () => {
        try {
          await fs.writeFile(destination, Buffer.concat(chunks));
          resolve();
        } catch (error) {
          reject(error);
        }
      });
    });

    request.on('error', reject);
  });
}

async function install() {
  const packageVersion = process.env.npm_package_version;
  const version = process.argv[2] ?? resolveReleaseTag(packageVersion);
  const platform = process.platform;
  const arch = process.arch;
  const homeDir = os.homedir();

  const assetName = resolveAssetName({ platform, arch });
  const installDir = resolveInstallDir({ platform, homeDir });
  const binaryName = resolveBinaryName(platform);
  const downloadUrl = resolveDownloadUrl({ version, assetName });
  const destination = path.join(installDir, binaryName);
  const tempFile = path.join(os.tmpdir(), `${binaryName}.${Date.now()}`);

  await fs.mkdir(installDir, { recursive: true });
  console.log(`Installing OpenAgents Kit ${version} for ${platform}/${arch}...`);
  console.log(`Downloading ${assetName} from GitHub Releases...`);

  await downloadFile(downloadUrl, tempFile);
  if (platform !== 'win32') {
    await fs.chmod(tempFile, 0o755);
  }
  await fs.rm(destination, { force: true });
  await fs.rename(tempFile, destination);

  console.log(`Installed ${binaryName} to ${destination}`);
  console.log('Next step: run openagents-kit');
  console.log(pathInstruction({ platform, installDir }));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  install().catch(error => {
    console.error(`OpenAgents install failed: ${error.message}`);
    process.exitCode = 1;
  });
}
