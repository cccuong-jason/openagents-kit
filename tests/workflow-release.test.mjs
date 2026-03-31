import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const workflowPath = path.join(
  process.cwd(),
  '.github',
  'workflows',
  'release.yml',
);

test('release workflow does not reference secrets directly in if conditions', () => {
  const contents = fs.readFileSync(workflowPath, 'utf8');
  const invalidIfLines = contents
    .split(/\r?\n/)
    .filter((line) => line.trimStart().startsWith('if:'))
    .filter((line) => line.includes('secrets.'));

  assert.deepEqual(invalidIfLines, []);
});
