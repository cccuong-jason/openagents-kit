#!/usr/bin/env node

import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import https from 'node:https';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const REPO = process.env.OPENAGENTS_REPO ?? 'cccuong-jason/openagents-kit';
const execFileAsync = promisify(execFile);

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

function normalizePathForPlatform(value, platform) {
  if (!value) {
    return '';
  }

  return platform === 'win32'
    ? path.win32.normalize(value).toLowerCase()
    : path.posix.normalize(value);
}

function splitPathEntries(envPath, platform) {
  if (!envPath) {
    return [];
  }

  return envPath
    .split(platform === 'win32' ? ';' : ':')
    .map(entry => entry.trim())
    .filter(Boolean);
}

export function resolveCanonicalInstallDir({
  platform,
  homeDir,
  localAppData = process.env.LOCALAPPDATA,
}) {
  if (platform === 'win32') {
    const baseDir = localAppData ?? path.win32.join(homeDir, 'AppData', 'Local');
    return path.win32.join(baseDir, 'OpenAgents', 'bin');
  }

  return path.posix.join(homeDir, '.local', 'bin');
}

export function resolveInstallDir(options) {
  return resolveCanonicalInstallDir(options);
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

export function isPathOnEnvPath({ candidate, envPath, platform }) {
  const normalizedCandidate = normalizePathForPlatform(candidate, platform);
  return splitPathEntries(envPath, platform)
    .map(entry => normalizePathForPlatform(entry, platform))
    .includes(normalizedCandidate);
}

function isWithinUserHome(candidatePath, homeDir, platform) {
  const normalizedCandidate = normalizePathForPlatform(candidatePath, platform);
  const normalizedHome = normalizePathForPlatform(homeDir, platform);
  const separator = platform === 'win32' ? '\\' : '/';
  return normalizedCandidate === normalizedHome || normalizedCandidate.startsWith(`${normalizedHome}${separator}`);
}

export function shouldRefreshExistingBinary({
  resolvedBinaryPath,
  canonicalBinaryPath,
  platform,
  homeDir,
}) {
  if (!resolvedBinaryPath) {
    return false;
  }

  const normalizedResolved = normalizePathForPlatform(resolvedBinaryPath, platform);
  const normalizedCanonical = normalizePathForPlatform(canonicalBinaryPath, platform);
  if (normalizedResolved === normalizedCanonical) {
    return false;
  }

  const binaryName = platform === 'win32' ? 'openagents-kit.exe' : 'openagents-kit';
  const normalizedBinaryName = platform === 'win32' ? binaryName.toLowerCase() : binaryName;
  if (!normalizedResolved.endsWith(normalizedBinaryName)) {
    return false;
  }

  return isWithinUserHome(resolvedBinaryPath, homeDir, platform);
}

export function findRefreshCandidate({
  resolvedBinaryPaths,
  canonicalBinaryPath,
  platform,
  homeDir,
}) {
  return resolvedBinaryPaths.find(candidate => shouldRefreshExistingBinary({
    resolvedBinaryPath: candidate,
    canonicalBinaryPath,
    platform,
    homeDir,
  }));
}

export function resolvePosixProfilePath({
  homeDir,
  shellPath,
  existingProfiles = [],
}) {
  const profileCandidates = [];
  const shellName = path.posix.basename(shellPath ?? '');
  if (shellName === 'zsh') {
    profileCandidates.push('.zshrc', '.zprofile', '.profile');
  } else if (shellName === 'bash') {
    profileCandidates.push('.bashrc', '.bash_profile', '.profile');
  } else if (shellName === 'fish') {
    profileCandidates.push('.config/fish/config.fish', '.profile');
  } else {
    profileCandidates.push('.profile', '.bashrc', '.zshrc');
  }

  const absoluteCandidates = profileCandidates.map(candidate => path.posix.join(homeDir, candidate));
  const existingSet = new Set(existingProfiles);
  return absoluteCandidates.find(candidate => existingSet.has(candidate)) ?? absoluteCandidates[0];
}

export function buildPosixPathSnippet(installDir, profilePath = '') {
  if (profilePath.endsWith('config.fish')) {
    return `\n# Added by OpenAgents installer\nfish_add_path ${installDir}\n`;
  }

  return `\n# Added by OpenAgents installer\nexport PATH="${installDir}:$PATH"\n`;
}

export function summarizeInstall({
  canonicalInstallDir,
  pathUpdated,
  refreshedBinary,
}) {
  const lines = [`Installed to ${canonicalInstallDir}`];
  if (pathUpdated) {
    lines.push('Updated your user PATH to include the OpenAgents bin directory.');
  }
  if (refreshedBinary) {
    lines.push(`Refreshed the existing PATH-winning binary at ${refreshedBinary}.`);
  }
  if (pathUpdated) {
    lines.push('Open a new shell if your current terminal does not pick up the PATH change immediately.');
  }
  return lines;
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

async function listResolvedBinaryPaths(platform) {
  try {
    if (platform === 'win32') {
      const { stdout } = await execFileAsync('where.exe', ['openagents-kit']);
      return stdout.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
    }

    const { stdout } = await execFileAsync('which', ['-a', 'openagents-kit']);
    return stdout.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
  } catch {
    return [];
  }
}

async function ensureWindowsUserPath(installDir) {
  const currentPath = await execFileAsync('powershell.exe', [
    '-NoProfile',
    '-Command',
    "[Environment]::GetEnvironmentVariable('Path','User')",
  ]);
  const userPath = currentPath.stdout.trim();
  if (isPathOnEnvPath({ candidate: installDir, envPath: userPath, platform: 'win32' })) {
    return false;
  }

  const escapedInstallDir = installDir.replace(/'/g, "''");
  const script = [
    "$current = [Environment]::GetEnvironmentVariable('Path','User')",
    "if (-not $current) { $current = '' }",
    `$install = '${escapedInstallDir}'`,
    "$parts = @()",
    "if ($current) { $parts = $current -split ';' | Where-Object { $_ -ne '' } }",
    "if ($parts -notcontains $install) {",
    "  $newPath = if ($current) { \"$current;$install\" } else { $install }",
    "  [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')",
    "  Write-Output 'UPDATED'",
    "} else {",
    "  Write-Output 'UNCHANGED'",
    "}",
  ].join('; ');
  const { stdout } = await execFileAsync('powershell.exe', ['-NoProfile', '-Command', script]);
  return stdout.includes('UPDATED');
}

async function ensurePosixPath(installDir) {
  if (isPathOnEnvPath({ candidate: installDir, envPath: process.env.PATH ?? '', platform: process.platform })) {
    return { pathUpdated: false, profilePath: null };
  }

  const homeDir = os.homedir();
  const candidates = [
    path.posix.join(homeDir, '.zshrc'),
    path.posix.join(homeDir, '.bashrc'),
    path.posix.join(homeDir, '.profile'),
    path.posix.join(homeDir, '.config', 'fish', 'config.fish'),
  ];
  const existingProfiles = [];
  await Promise.all(candidates.map(async candidate => {
    try {
      await fs.access(candidate);
      existingProfiles.push(candidate);
    } catch {
      // ignore missing profile
    }
  }));

  const profilePath = resolvePosixProfilePath({
    homeDir,
    shellPath: process.env.SHELL ?? '',
    existingProfiles,
  });
  const snippet = buildPosixPathSnippet(installDir, profilePath);
  const current = await fs.readFile(profilePath, 'utf8').catch(() => '');
  if (current.includes(installDir)) {
    return { pathUpdated: false, profilePath };
  }

  await fs.mkdir(path.dirname(profilePath), { recursive: true });
  await fs.appendFile(profilePath, snippet);
  return { pathUpdated: true, profilePath };
}

async function ensurePathContainsInstallDir(installDir, platform) {
  if (platform === 'win32') {
    return { pathUpdated: await ensureWindowsUserPath(installDir), profilePath: null };
  }

  return ensurePosixPath(installDir);
}

export function pathInstruction({ platform, installDir, pathUpdated }) {
  if (pathUpdated) {
    return 'Open a new shell if your current terminal does not pick up the PATH change immediately.';
  }

  if (platform === 'win32') {
    return `Your PATH already includes ${installDir}.`;
  }

  return `Your PATH already includes ${installDir}.`;
}

async function install() {
  const packageVersion = process.env.npm_package_version;
  const version = process.argv[2] ?? resolveReleaseTag(packageVersion);
  const platform = process.platform;
  const arch = process.arch;
  const homeDir = os.homedir();

  const assetName = resolveAssetName({ platform, arch });
  const installDir = resolveCanonicalInstallDir({
    platform,
    homeDir,
    localAppData: process.env.LOCALAPPDATA,
  });
  const binaryName = resolveBinaryName(platform);
  const downloadUrl = resolveDownloadUrl({ version, assetName });
  const destination = path.join(installDir, binaryName);
  const tempFile = path.join(os.tmpdir(), `${binaryName}.${Date.now()}`);
  const resolvedBinaryPaths = await listResolvedBinaryPaths(platform);

  await fs.mkdir(installDir, { recursive: true });
  console.log(`Installing OpenAgents Kit ${version} for ${platform}/${arch}...`);
  console.log(`Downloading ${assetName} from GitHub Releases...`);

  await downloadFile(downloadUrl, tempFile);
  if (platform !== 'win32') {
    await fs.chmod(tempFile, 0o755);
  }
  await fs.rm(destination, { force: true });
  await fs.rename(tempFile, destination);

  const { pathUpdated } = await ensurePathContainsInstallDir(installDir, platform);
  const refreshCandidate = findRefreshCandidate({
    resolvedBinaryPaths,
    canonicalBinaryPath: destination,
    platform,
    homeDir,
  });
  if (refreshCandidate) {
    await fs.copyFile(destination, refreshCandidate);
  }

  console.log(`Installed ${binaryName} to ${destination}`);
  for (const line of summarizeInstall({
    canonicalInstallDir: installDir,
    pathUpdated,
    refreshedBinary: refreshCandidate,
  })) {
    console.log(line);
  }
  console.log('Next step: run openagents-kit');
  console.log(pathInstruction({ platform, installDir, pathUpdated }));
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  install().catch(error => {
    console.error(`OpenAgents install failed: ${error.message}`);
    process.exitCode = 1;
  });
}
