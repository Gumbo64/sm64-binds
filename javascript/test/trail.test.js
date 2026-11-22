import test from 'node:test';
import assert from 'node:assert/strict';
import { Trail } from '../src/replay/trail.js';

test('nearest returns the latest state at or before the target', () => {
  const t = new Trail(10, 100);
  [0, 10, 20, 30].forEach((f) => t.add(f, `s${f}`));
  assert.deepEqual(t.nearest(25), { frame: 20, state: 's20' });
  assert.deepEqual(t.nearest(30), { frame: 30, state: 's30' });
  assert.equal(new Trail().nearest(5), null);
});

test('due only at multiples without a state', () => {
  const t = new Trail(10, 100);
  assert.ok(t.due(0));
  t.add(0, 'a');
  assert.ok(!t.due(0));
  assert.ok(!t.due(5));
  assert.ok(t.due(10));
});

test('thins out when full, keeping the first', () => {
  const t = new Trail(1, 8);
  for (let f = 0; f < 9; f++) t.add(f, f);
  assert.ok(t.size <= 8);
  assert.ok(t.has(0));
});
