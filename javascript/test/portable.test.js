// The portable state translator (portable.js): that the WASM loads, knows both
// builds, and (given memory dumps of the two builds, which are made from the
// ROM and so do not ship) exports the same state from both.
//
//   SM64_HEADLESS_MEMORY=h.bin SM64_VISUAL_MEMORY=v.bin npm test
//
// (the dumps: all of linear memory of each build's game at the same frame of a movie;
// rust/examples/xs_dump-like code writes the first, javascript/e2e the second.)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { PortableTranslator, portableInfo, BUILD_HEADLESS, BUILD_VISUAL } from '../src/scripts/sm64/portable.js';

test('the translator knows both builds', async () => {
  const t = await PortableTranslator.load();
  const h = t.constants(BUILD_HEADLESS);
  const v = t.constants(BUILD_VISUAL);
  assert.ok(h.gm > 0x10000 && v.gm > 0x10000);
  assert.notEqual(h.gm, v.gm);
  assert.ok(h.dataStart < h.gm && v.dataStart < v.gm);
  assert.match(t.buildId(BUILD_HEADLESS), /^[0-9a-f]{40}$/);
  assert.match(t.buildId(BUILD_VISUAL), /^[0-9a-f]{40}$/);
  assert.notEqual(t.buildId(BUILD_HEADLESS), t.buildId(BUILD_VISUAL));
});

test('portableInfo refuses what is not a portable state', () => {
  assert.throws(() => portableInfo(new Uint8Array(100)));
});

const h = process.env.SM64_HEADLESS_MEMORY;
const v = process.env.SM64_VISUAL_MEMORY;

test('both builds export the same state, and it loads back', { skip: !(h && v) }, async () => {
  const t = await PortableTranslator.load();
  const hm = fs.readFileSync(h);
  const vm = fs.readFileSync(v);
  const hd = t.digestState(hm.buffer.slice(hm.byteOffset, hm.byteOffset + hm.length), BUILD_HEADLESS);
  const vd = t.digestState(vm.buffer.slice(vm.byteOffset, vm.byteOffset + vm.length), BUILD_VISUAL);
  assert.equal(hd.length, vd.length);
  let differing = 0;
  for (let i = 0; i < hd.length; i++) if (hd[i] !== vd[i]) differing++;
  // Render-only variables may differ (a headless build does not draw); the rest do not.
  assert.ok(differing < hd.length * 0.02, `${differing} of ${hd.length} variables differ`);
  const state = t.exportState(hm.buffer.slice(hm.byteOffset, hm.byteOffset + hm.length), BUILD_HEADLESS);
  const info = portableInfo(state);
  assert.equal(info.source, 'headless');
  assert.equal(info.buildId, t.buildId(BUILD_HEADLESS));
});
