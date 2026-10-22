// The export map only exposes index.js, so apps cannot import from src/.
// Checks index.js re-exports what sm64-viewer needs; importing index.js here
// is not possible because it pulls in JSX.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';

const index = fs.readFileSync(new URL('../index.js', import.meta.url), 'utf8');
const exported = new Set(
  [...index.matchAll(/^export\s*\{([^}]*)\}/gm)].flatMap((m) => m[1].split(',').map((s) => s.trim())),
);

test('index.js exports what apps driving the engine need', () => {
  for (const name of ['Sm64VisualEngine', 'calculateFileHash', 'isRomCached', 'storeROM', 'updateWASMs', 'getROM', 'GamePad']) {
    assert.ok(exported.has(name), `${name} is not exported from index.js`);
  }
});

test('the export map exposes only index.js', () => {
  const pkg = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.deepEqual(pkg.exports, { '.': './index.js' });
});
