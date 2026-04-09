import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { execFileSync } from 'node:child_process';

const PACKAGE_NAME = 'openagents-kit';
const ROOT_DIR = path.resolve(import.meta.dirname, '..');
const PACKAGE_JSON_PATH = path.join(ROOT_DIR, 'package.json');
const CARGO_TOML_PATH = path.join(ROOT_DIR, 'crates', 'openagents-tui', 'Cargo.toml');

export function compareVersions(left, right) {
  const leftParts = left.split('.').map((part) => Number.parseInt(part, 10));
  const rightParts = right.split('.').map((part) => Number.parseInt(part, 10));
  const length = Math.max(leftParts.length, rightParts.length);

  for (let index = 0; index < length; index += 1) {
    const a = leftParts[index] ?? 0;
    const b = rightParts[index] ?? 0;
    if (a > b) return 1;
    if (a < b) return -1;
  }

  return 0;
}

export function readPackageVersion(contents) {
  return JSON.parse(contents).version;
}

export function readCargoVersion(contents) {
  const match = contents.match(/^version = "([^"]+)"$/m);
  if (!match) {
    throw new Error(`Could not find a version in ${path.relative(ROOT_DIR, CARGO_TOML_PATH)}.`);
  }
  return match[1];
}

export function createPublishGuardReport({
  packageVersion,
  cargoVersion,
  publishedVersion,
}) {
  if (packageVersion !== cargoVersion) {
    return {
      ok: false,
      message: `Version mismatch: package.json is ${packageVersion} but crates/openagents-tui/Cargo.toml is ${cargoVersion}. Run the bump helper so npm and Rust stay aligned.`,
    };
  }

  if (publishedVersion && compareVersions(packageVersion, publishedVersion) <= 0) {
    return {
      ok: false,
      message: `Refusing to publish ${PACKAGE_NAME} ${packageVersion} because npm already has ${publishedVersion}. Bump the version first or update your checkout from origin/main.`,
    };
  }

  return {
    ok: true,
    message: `Release guard passed for ${PACKAGE_NAME} ${packageVersion}.`,
  };
}

export function resolvePublishedVersion() {
  const override = process.env.OPENAGENTS_PUBLISHED_VERSION;
  if (override) {
    return override.trim();
  }

  const npmInvocation = resolveNpmInvocation();

  try {
    return execFileSync(
      npmInvocation.command,
      [...npmInvocation.prefixArgs, 'view', PACKAGE_NAME, 'version'],
      {
        cwd: ROOT_DIR,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
      },
    ).trim();
  } catch (error) {
    const detail = error.stderr?.toString().trim() || error.message;
    throw new Error(
      `Failed to query the published npm version for ${PACKAGE_NAME}. ${detail}`,
    );
  }
}

export function resolveNpmInvocation(
  platform = process.platform,
  env = process.env,
  nodeExecPath = process.execPath,
) {
  if (env.npm_execpath) {
    return {
      command: nodeExecPath,
      prefixArgs: [env.npm_execpath],
    };
  }

  if (platform === 'win32') {
    return {
      command: 'cmd.exe',
      prefixArgs: ['/d', '/s', '/c', 'npm'],
    };
  }

  return {
    command: 'npm',
    prefixArgs: [],
  };
}

export function replaceVersionInPackageJson(contents, nextVersion) {
  return contents.replace(
    /"version":\s*"[^"]+"/,
    `"version": "${nextVersion}"`,
  );
}

export function replaceVersionInCargoToml(contents, nextVersion) {
  return contents.replace(
    /^version = "([^"]+)"$/m,
    `version = "${nextVersion}"`,
  );
}

export function replaceVersionInCargoLock(contents, nextVersion) {
  return contents.replace(
    /(\[\[package\]\]\r?\nname = "openagents-tui"\r?\nversion = ")([^"]+)(")/,
    `$1${nextVersion}$3`,
  );
}

function main() {
  const packageContents = fs.readFileSync(PACKAGE_JSON_PATH, 'utf8');
  const cargoContents = fs.readFileSync(CARGO_TOML_PATH, 'utf8');
  const packageVersion = readPackageVersion(packageContents);
  const cargoVersion = readCargoVersion(cargoContents);
  const publishedVersion = resolvePublishedVersion();
  const report = createPublishGuardReport({
    packageVersion,
    cargoVersion,
    publishedVersion,
  });

  if (!report.ok) {
    console.error(report.message);
    process.exitCode = 1;
    return;
  }

  console.log(report.message);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(import.meta.filename)) {
  main();
}
