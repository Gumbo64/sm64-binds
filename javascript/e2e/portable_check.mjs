// Cross-build validation of portable states (the rendering build's half).
//
//   env -u DISPLAY node portable_check.mjs record <rom> <movie.m64> <dir> <K> <frame>...
//       plays the movie from the start; at each checkpoint saves v_<frame>.p (a portable
//       state) and records the K frames after it (v_<frame>.json)
//   env -u DISPLAY node portable_check.mjs load <rom> <movie.m64> <dir> <K>
//       for each h_<frame>.p (made by `portable_check record` in rust/examples), a
//       fresh game loads it and records the K frames after it (vh_<frame>.json)
//   env -u DISPLAY node portable_check.mjs snapshot <rom> <movie.m64> <K> <frame>...
//       the game's own save_state/load_state: play to each frame, save, play K frames,
//       load, play K frames again, and compare
//   env -u DISPLAY node portable_check.mjs rewind <rom> <movie.m64> <K> <frame>...
//       what the viewer's seeking does: play to each frame, export a portable state, play K
//       frames, load the state back into the same engine, play K frames again and compare
//
// `portable_check compare <dir> <K>` (Rust) then prints the table. The rom is copied
// to public/rom.z64 for the page.
import { chromium } from 'playwright';
import { createServer } from 'vite';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const here = path.dirname(fileURLToPath(import.meta.url));
const [, , mode, rom, m64, ...rest] = process.argv;
fs.mkdirSync(path.join(here, 'public'), { recursive: true });
fs.copyFileSync(rom, path.join(here, 'public', 'rom.z64'));

const buf = fs.readFileSync(m64);
const n = buf.readUInt32LE(0x18);
const pads = [];
for (let i = 0; i < n; i++) {
  const o = 0x400 + 4 * i;
  pads.push(buf.readUInt16BE(o), buf.readInt8(o + 2), buf.readInt8(o + 3));
}
const slice = (from, k) => pads.slice(3 * from, 3 * (from + k));

const server = await createServer({
  configFile: false,
  root: here,
  resolve: { alias: { '@fe': path.join(here, '..') } },
  server: { port: 5231, fs: { strict: false } },
  logLevel: 'error',
});
await server.listen();
const browser = await chromium.launch({ args: ['--no-sandbox', '--disable-dev-shm-usage', '--use-angle=swiftshader-webgl', '--enable-unsafe-swiftshader', '--enable-features=WebAssemblyUnlimitedSyncCompilation'] });
const open = async () => {
  const page = await browser.newPage();
  page.on('pageerror', (e) => console.log('pageerror', e.message));
  page.on('console', (m) => { if (m.type() === 'error') console.log('console', m.text().slice(0, 200)); });
  await page.goto('http://localhost:5231/');
  const ok = await page.evaluate(() => window.vInit());
  if (!ok) throw new Error('the engine reports portable states as unsupported');
  return page;
};
const play = async (page, from, to) => {
  for (let at = from; at < to; at += 500) await page.evaluate((f) => window.vStep(f), slice(at, Math.min(500, to - at)));
};

try {
  if (mode === 'record') {
    const [dir, k, ...frames] = rest;
    const K = Number(k);
    const page = await open();
    let at = 0;
    for (const f of frames.map(Number).sort((a, b) => a - b)) {
      await play(page, at, f);
      at = f;
      try {
        const b64 = await page.evaluate(() => window.vExport());
        fs.writeFileSync(path.join(dir, `v_${f}.p`), Buffer.from(b64, 'base64'));
      } catch (e) {
        console.log(`v_${f}: no portable state: ${e.message.split('\n')[0]}`);
        continue;
      }
      const rec = await page.evaluate((flat) => window.vRecord(flat), slice(f, K));
      fs.writeFileSync(path.join(dir, `v_${f}.json`), JSON.stringify({ frame: f, frames: rec }));
      at = f + K;
      console.log(`v_${f}: recorded`);
    }
  } else if (mode === 'load') {
    const [dir, k] = rest;
    const K = Number(k);
    for (const name of fs.readdirSync(dir).sort()) {
      const m = name.match(/^h_(\d+)\.p$/);
      if (!m) continue;
      const f = Number(m[1]);
      const page = await open();
      try {
        const info = await page.evaluate((b64) => window.vLoad(b64), fs.readFileSync(path.join(dir, name)).toString('base64'));
        console.log(`vh_${f}: loaded`, JSON.stringify(info));
        const rec = await page.evaluate((flat) => window.vRecord(flat), slice(f, K));
        fs.writeFileSync(path.join(dir, `vh_${f}.json`), JSON.stringify({ frame: f, frames: rec }));
      } catch (e) {
        console.log(`vh_${f}: failed: ${e.message.split('\n')[0]}`);
      }
      await page.close();
    }
  } else if (mode === 'snapshot') {
    const [k, ...frames] = rest;
    const K = Number(k);
    const page = await open();
    let at = 0;
    for (const f of frames.map(Number).sort((a, b) => a - b)) {
      await play(page, at, f);
      at = f;
      const size = await page.evaluate(() => window.vSnapshot());
      const a = await page.evaluate((flat) => window.vRecord(flat), slice(f, K));
      await page.evaluate(() => window.vRestore());
      const b = await page.evaluate((flat) => window.vRecord(flat), slice(f, K));
      const same = JSON.stringify(a) === JSON.stringify(b);
      const first = a.findIndex((x, i) => JSON.stringify(x) !== JSON.stringify(b[i]));
      console.log(`snapshot at ${f} (${size} bytes): ${same ? 'identical' : 'DIFFERS at frame +' + (first + 1)}`);
      at = f + K;
    }
  } else if (mode === 'rewind') {
    const [k, ...frames] = rest;
    const K = Number(k);
    const page = await open();
    let at = 0;
    for (const f of frames.map(Number).sort((a, b) => a - b)) {
      await play(page, at, f);
      at = f;
      let b64;
      try {
        b64 = await page.evaluate(() => window.vExport());
      } catch (e) {
        console.log(`rewind at ${f}: no portable state: ${e.message.split('\n')[0]}`);
        continue;
      }
      const a = await page.evaluate((flat) => window.vRecord(flat), slice(f, K));
      await page.evaluate((b) => window.vLoad(b), b64);
      const b = await page.evaluate((flat) => window.vRecord(flat), slice(f, K));
      const first = a.findIndex((x, i) => JSON.stringify(x) !== JSON.stringify(b[i]));
      console.log(`rewind at ${f} (${(b64.length * 0.75) | 0} bytes): ${first < 0 ? 'identical' : 'DIFFERS at frame +' + (first + 1)}`);
      at = f + K;
    }
  } else if (mode === 'render') {
    // What the viewer's headless seek shows: at each frame, (a) the rendering build played
    // there and K frames on, (b) a fresh rendering build loading the headless build's portable
    // state of that frame, then the same K frames, (c) the rendering build reloading its own
    // export (no cross-build translation). Canvas screenshots in <dir>: a_/b_/c_<frame>.png.
    const [dir, k, ...frames] = rest;
    const K = Number(k);
    fs.mkdirSync(dir, { recursive: true });
    const shot = (page, name) => page.locator('#canvas').screenshot({ path: path.join(dir, name) });
    for (const f of frames.map(Number).sort((a, b) => a - b)) {
      const a = await open();
      await play(a, 0, f);
      const own = await a.evaluate(() => window.vExport());
      const save = (name, b64) => fs.writeFileSync(path.join(dir, name), Buffer.from(b64, 'base64'));
      save(`own_${f}.p`, own);
      await play(a, f, f + K);
      await shot(a, `a_${f}.png`);
      save(`a_${f + K}.p`, await a.evaluate(() => window.vExport()));
      try {
        const ci = await a.evaluate((b) => window.vLoad(b), own);
        await play(a, f, f + K);
        await shot(a, `c_${f}.png`);
        console.log(`render at ${f}: own reload ${JSON.stringify(ci)}`);
      } catch (e) {
        console.log(`render at ${f}: own reload FAILED: ${e.message.split('\n')[0]}`);
      }
      await a.close();
      const b = await open();
      // Like the viewer: the rendering build has played elsewhere first (PRE frames, env).
      const pre = Number(process.env.PRE || 0);
      // RESET=1: back to the engine's own power-on snapshot before loading (the fix under test).
      if (process.env.RESET === '1') await b.evaluate(() => window.vSnapshot());
      if (pre) await play(b, 0, pre);
      if (process.env.RESET === '1') await b.evaluate(() => window.vRestore());
      await b.evaluate(() => window.hInit());
      for (let at = 0; at < f; at += 2000) await b.evaluate((fl) => window.hStep(fl), slice(at, Math.min(2000, f - at)));
      const hp = await b.evaluate(() => window.hExport());
      save(`hp_${f}.p`, hp);
      const info = await b.evaluate((x) => window.vLoad(x), hp);
      save(`bload_${f}.p`, await b.evaluate(() => window.vExport()));
      await play(b, f, f + K);
      await shot(b, `b_${f}.png`);
      save(`b_${f + K}.p`, await b.evaluate(() => window.vExport()));
      await b.close();
      console.log(`render at ${f}: headless load ${JSON.stringify(info)}`);
    }
  } else if (mode === 'quick') {
    // Fast render check: one rendering build plays to each frame with drawing switched off (the
    // reference), exports its own state there, then draws the next K frames: q_a_<frame>.png.
    // Fresh rendering builds then load (1) that own state and (2) <dir>/h_<frame>.p (the native
    // headless build's, `portable_check record`), draw the same K frames: q_own_/q_h_<frame>.png.
    const [dir, k, ...frames] = rest;
    const K = Number(k);
    const mute = (page, on) => page.evaluate((on) => {
      const gl = document.getElementById('canvas').getContext('webgl2') || document.getElementById('canvas').getContext('webgl');
      const P = Object.getPrototypeOf(gl);
      window.__orig ??= { drawElements: P.drawElements, drawArrays: P.drawArrays, clear: P.clear };
      for (const m of Object.keys(window.__orig)) P[m] = on ? () => {} : window.__orig[m];
    }, on);
    const shot = (page, name) => page.locator('#canvas').screenshot({ path: path.join(dir, name) });
    const a = await open();
    let at = 0;
    for (const f of frames.map(Number).sort((x, y) => x - y)) {
      await mute(a, true);
      await play(a, at, f);
      await mute(a, false);
      await a.evaluate(() => window.vSnapshot());
      fs.writeFileSync(path.join(dir, `own_${f}.p`), Buffer.from(await a.evaluate(() => window.vExport()), 'base64'));
      await play(a, f, f + K);
      at = f + K;
      await shot(a, `q_a_${f}.png`);
      fs.writeFileSync(path.join(dir, `after_a_${f}_${K}.p`), Buffer.from(await a.evaluate(() => window.vExport()), 'base64'));
      // The same state reloaded into this engine (which has been drawing), the same K frames.
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64'));
      await play(a, f, f + K);
      await shot(a, `q_same_${f}.png`);
      // The raw snapshot of this engine's memory at <frame> (no portable translation).
      await a.evaluate(() => window.vRestore());
      await play(a, f, f + K);
      await shot(a, `q_raw_${f}.png`);
      // Static memory after the raw restore vs after the portable load (same engine, same build).
      const statics = () => a.evaluate(() => {
        const c = window.engine.translator.constants(1);
        return Array.from(new Uint8Array(window.engine.memory.buffer, c.dataStart, 0xc60000 - c.dataStart));
      });
      await a.evaluate(() => window.vRestore());
      const m1 = await statics();
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64'));
      const m2 = await statics();
      const base = await a.evaluate(() => window.engine.translator.constants(1).dataStart);
      const ranges = [];
      for (let i = 0; i < m1.length; i++) if (m1[i] !== m2[i]) { const s0 = i; while (i < m1.length && m1[i] !== m2[i]) i++; ranges.push([base + s0, i - s0]); }
      fs.writeFileSync(path.join(dir, `statics_${f}.json`), JSON.stringify(ranges));
      // Mario as the renderer sees him: his object, then his model's graph nodes (depth first),
      // each word with, if it points into memory, the 32 bytes it points to.
      const dumpMario = () => a.evaluate(() => {
        const m = new DataView(window.engine.memory.buffer);
        const lo = window.engine.translator.constants(1).dataStart, hi = window.engine.memory.buffer.byteLength;
        const words = (at, n) => { const o = []; for (let k = 0; k < n; k += 4) { const w = m.getUint32(at + k, true); const t = w >= lo && w + 32 < hi ? Array.from({ length: 8 }, (_, j) => m.getUint32(w + 4 * j, true)) : null; o.push([w, t]); } return o; };
        const obj = m.getUint32(0xb20448, true);
        const out = { obj: words(obj, 0x260), nodes: [] };
        const seen = new Set();
        const walk = (n, d) => { if (!n || seen.has(n) || d > 40 || out.nodes.length > 400) return; seen.add(n); out.nodes.push([d, m.getInt16(n, true), words(n, 0x40)]); const c = m.getUint32(n + 0x10, true); if (c) { let x = c; do { walk(x, d + 1); x = m.getUint32(x + 8, true); } while (x && x !== c); } };
        walk(m.getUint32(obj + 0x14, true), 0);
        return out;
      });
      await a.evaluate(() => window.vRestore());
      const d1 = await dumpMario();
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64'));
      const d2 = await dumpMario();
      fs.writeFileSync(path.join(dir, `mario_${f}.json`), JSON.stringify([d1, d2]));
      for (const [tag, file] of [['own', `own_${f}.p`], ['h', `h_${f}.p`]]) {
        if (!fs.existsSync(path.join(dir, file))) continue;
        const b = await open();
        const info = await b.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, file)).toString('base64'));
        await play(b, f, f + K);
        await shot(b, `q_${tag}_${f}.png`);
        fs.writeFileSync(path.join(dir, `after_${tag}_${f}_${K}.p`), Buffer.from(await b.evaluate(() => window.vExport()), 'base64'));
        console.log(`quick ${f} ${tag}: ${JSON.stringify(info)}`);
        await b.close();
      }
    }
  } else if (mode === 'loadshot') {
    // A fresh rendering build loads each portable state file and plays K frames of the movie
    // from <frame>: <dir>/ls_<name>.png.
    const [dir, k, f0, ...files] = rest;
    const K = Number(k), f = Number(f0);
    for (const file of files) {
      const page = await open();
      const pre = Number(process.env.PRE || 0);
      if (pre) await play(page, 0, pre);
      await page.evaluate(() => {
        window.__draws = 0;
        for (const P of [WebGLRenderingContext.prototype, WebGL2RenderingContext.prototype]) for (const m of ['drawArrays', 'drawElements']) {
          const o = P[m]; P[m] = function (...a) { window.__draws++; return o.apply(this, a); };
        }
      });
      const d0 = await page.evaluate(() => { const n = window.__draws; window.vStep([0, 0, 0]); return window.__draws - n; });
      const info = await page.evaluate((x) => window.vLoad(x), fs.readFileSync(file).toString('base64'));
      await page.locator('#canvas').screenshot({ path: path.join(dir, `ls_${path.basename(file)}_0.png`) });
      const n1 = await page.evaluate(() => window.__draws);
      await play(page, f, f + K);
      const d1 = (await page.evaluate(() => window.__draws) - n1) / K;
      console.log(`draw calls per frame: before the load ${d0}, after ${d1}`);
      await page.locator('#canvas').screenshot({ path: path.join(dir, `ls_${path.basename(file)}.png`) });
      console.log(`loadshot ${file}: ${JSON.stringify(info)}`);
      await page.close();
    }
  } else {
    console.log('usage: see the header of this file');
  }
} finally {
  await browser.close();
  await server.close();
}
