import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';

import {
  replaceVersionInCargoLock,
  replaceVersionInCargoToml,
  replaceVersionInPackageJson,
} from './release-guard.mjs';

const ROOT_DIR = path.resolve(import.meta.dirname, '..');
const PACKAGE_JSON_PATH = path.join(ROOT_DIR, 'package.json');
const CARGO_TOML_PATH = path.join(ROOT_DIR, 'crates', 'openagents-tui', 'Cargo.toml');
const CARGO_LOCK_PATH = path.join(ROOT_DIR, 'Cargo.lock');

function assertVersionShape(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`Expected a semantic version like 0.3.5, received "${version}".`);
  }
}

function main() {
  const nextVersion = process.argv[2];
  if (!nextVersion) {
    throw new Error('Usage: node scripts/bump-version.mjs <next-version>');
  }

  assertVersionShape(nextVersion);

  fs.writeFileSync(
    PACKAGE_JSON_PATH,
    replaceVersionInPackageJson(fs.readFileSync(PACKAGE_JSON_PATH, 'utf8'), nextVersion),
    'utf8',
  );
  fs.writeFileSync(
    CARGO_TOML_PATH,
    replaceVersionInCargoToml(fs.readFileSync(CARGO_TOML_PATH, 'utf8'), nextVersion),
    'utf8',
  );
  fs.writeFileSync(
    CARGO_LOCK_PATH,
    replaceVersionInCargoLock(fs.readFileSync(CARGO_LOCK_PATH, 'utf8'), nextVersion),
    'utf8',
  );

  console.log(`Bumped package.json, crates/openagents-tui/Cargo.toml, and Cargo.lock to ${nextVersion}.`);
}

try {
  main();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
