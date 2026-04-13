import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const testDir = path.dirname(fileURLToPath(import.meta.url));
const workflowPath = path.join(
  testDir,
  '..',
  '.github',
  'workflows',
  'ci.yml',
);

test('ci workflow runs the release-tooling suite', () => {
  const contents = fs.readFileSync(workflowPath, 'utf8');

  assert.match(contents, /npm run test:release-tooling/);
});
