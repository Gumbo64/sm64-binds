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
import crypto from 'crypto';
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
      fs.writeFileSync(path.join(dir, `boot.bin`), Buffer.from(await a.evaluate(() => btoa(Array.from(window.engine._bootState, (c) => String.fromCharCode(c)).join(''))), 'base64'));
      fs.writeFileSync(path.join(dir, `raw_${f}.bin`), Buffer.from(await a.evaluate(() => btoa(Array.from(window.__snap, (c) => String.fromCharCode(c)).join(''))), 'base64'));
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
      await play(a, f, f + Number(process.env.STATICS_AT || 0));
      const m1 = await statics();
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64'));
      await play(a, f, f + Number(process.env.STATICS_AT || 0));
      const m2 = await statics();
      const base = await a.evaluate(() => window.engine.translator.constants(1).dataStart);
      const ranges = [];
      for (let i = 0; i < m1.length; i++) if (m1[i] !== m2[i]) { const s0 = i; while (i < m1.length && m1[i] !== m2[i]) i++; ranges.push([base + s0, i - s0]); }
      fs.writeFileSync(path.join(dir, `statics_${f}.json`), JSON.stringify(ranges));
      // Mario as the renderer sees him: his object, then his model's graph nodes (depth first),
      // each word with, if it points into memory, the 32 bytes it points to.
      const dumpMario = () => a.evaluate(() => {
        const m0 = new DataView(window.engine.memory.buffer); const m = { byteLength: m0.byteLength, getUint32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getUint32(a, l) : 0), getInt16: (a, l) => (a >= 0 && a + 2 <= m0.byteLength ? m0.getInt16(a, l) : 0), getInt32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getInt32(a, l) : 0), getFloat32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getFloat32(a, l) : 0) };
        const lo = window.engine.translator.constants(1).dataStart, hi = window.engine.memory.buffer.byteLength;
        const words = (at, n) => { const o = []; for (let k = 0; k < n; k += 4) { const w = m.getUint32(at + k, true); const t = w >= lo && w + 32 < hi ? Array.from({ length: 8 }, (_, j) => m.getUint32(w + 4 * j, true)) : null; o.push([w, t]); } return o; };
        const obj = m.getUint32(0xb20448, true);
        const out = { obj: words(obj, 0x260), nodes: [] };
        const seen = new Set();
        const walk = (n, d) => { if (!n || seen.has(n) || d > 80 || out.nodes.length > 5000) return; seen.add(n); out.nodes.push([d, m.getInt16(n, true), words(n, 0x40)]); const c = m.getUint32(n + 0x10, true); if (c) { let x = c; do { walk(x, d + 1); x = m.getUint32(x + 8, true); } while (x && x !== c); } };
        walk(m.getUint32(obj + 0x14, true), 0);
        return out;
      });
      const MAT = Number(process.env.MARIO_AT || 0);
      await a.evaluate(() => window.vRestore());
      await play(a, f, f + MAT);
      const d1 = await dumpMario();
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64'));
      await play(a, f, f + MAT);
      const d2 = await dumpMario();
      fs.writeFileSync(path.join(dir, `mario_${f}.json`), JSON.stringify([d1, d2]));
      // Mario's animation DMA table (gMarioAnimsBuf.dmaTable): count, srcAddr, the first entries.
      const dma = () => a.evaluate(() => {
        const m0 = new DataView(window.engine.memory.buffer); const m = { byteLength: m0.byteLength, getUint32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getUint32(a, l) : 0), getInt16: (a, l) => (a >= 0 && a + 2 <= m0.byteLength ? m0.getInt16(a, l) : 0), getInt32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getInt32(a, l) : 0), getFloat32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getFloat32(a, l) : 0) };
        const list = 0xb1e070, t = m.getUint32(list, true);
        const e = []; for (let i = 0; i < 12; i++) e.push(m.getUint32(t + 8 + 4 * i, true));
        return { table: t, count: m.getUint32(t, true), src: m.getUint32(t + 4, true), current: m.getUint32(list + 4, true), buf: m.getUint32(list + 8, true), entries: e };
      });
      await a.evaluate(() => window.vRestore());
      const t1 = await dma();
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64'));
      const t2 = await dma();
      console.log('dma raw', JSON.stringify(t1));
      // The loaded models (gLoadedGraphNodes = area:D_8033A160, 256 pointers): each one's first
      // 30 nodes' types (depth first), raw restore vs portable load.
      const models = () => a.evaluate(() => {
        const m0 = new DataView(window.engine.memory.buffer); const m = { byteLength: m0.byteLength, getUint32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getUint32(a, l) : 0), getInt16: (a, l) => (a >= 0 && a + 2 <= m0.byteLength ? m0.getInt16(a, l) : 0), getInt32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getInt32(a, l) : 0), getFloat32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getFloat32(a, l) : 0) };
        const out = [];
        for (let i = 0; i < 256; i++) {
          const r = m.getUint32(0xb1d210 + 4 * i, true);
          const types = []; const seen = new Set();
          const walk = (n) => { if (!n || n + 0x20 > m.byteLength || seen.has(n) || types.length >= 30) return; seen.add(n); types.push(m.getInt16(n, true)); const c = m.getUint32(n + 0x10, true); if (c) { let x = c; do { walk(x); x = m.getUint32(x + 8, true); } while (x && x !== c && types.length < 30); } };
          walk(r);
          out.push(types.join(','));
        }
        return out;
      });
      await a.evaluate(() => window.vRestore());
      const g1 = await models();
      await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64'));
      const g2 = await models();
      for (let i = 0; i < 256; i++) if (g1[i] !== g2[i]) console.log(`model ${i}: raw [${g1[i]}] vs portable [${g2[i]}]`);
      // Active objects after K frames: behavior, model index, position, gfx flags.
      const objs = () => a.evaluate(() => {
        const m0 = new DataView(window.engine.memory.buffer); const m = { byteLength: m0.byteLength, getUint32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getUint32(a, l) : 0), getInt16: (a, l) => (a >= 0 && a + 2 <= m0.byteLength ? m0.getInt16(a, l) : 0), getInt32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getInt32(a, l) : 0), getFloat32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getFloat32(a, l) : 0) };
        const models = new Map(); for (let i = 0; i < 256; i++) models.set(m.getUint32(0xb1d210 + 4 * i, true), i);
        const out = [];
        for (let s = 0; s < 240; s++) {
          const o = 0xb20b60 + s * 0x260;
          if (!m.getInt16(o + 0x74, true)) continue; // activeFlags
          const sc = m.getUint32(o + 0x14, true);
          out.push([s, m.getUint32(o + 0x20c, true).toString(16), models.has(sc) ? models.get(sc) : 'x' + sc.toString(16), m.getInt16(o + 0x2, true).toString(16), [0x20, 0x24, 0x28].map((k) => Math.round(m.getFloat32(o + k, true)))].join(' '));
        }
        return out;
      });
      for (const [name, load] of [['raw', () => a.evaluate(() => window.vRestore())], ['portable', () => a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64'))]]) {
        await load();
        await play(a, f, f + Number(process.env.OBJ_AT || K));
        fs.writeFileSync(path.join(dir, `objs_${name}.txt`), (await objs()).join('\n'));
      }
      // Frame by frame: raw restore vs portable load, the picture and Mario's animation state.
      const trace = async (load) => {
        await load();
        const out = [];
        for (let i = 0; i < K; i++) {
          await play(a, f + i, f + i + 1);
          const buf = await a.locator('#canvas').screenshot();
          if (process.env.TRACE && i + 1 === Number(process.env.TRACE)) fs.writeFileSync(path.join(dir, `trace_${out.length ? 'x' : 'x'}_${load.name || 'l'}_${Date.now()}.png`), buf);
          const st = await a.evaluate(() => {
            const m0 = new DataView(window.engine.memory.buffer); const m = { byteLength: m0.byteLength, getUint32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getUint32(a, l) : 0), getInt16: (a, l) => (a >= 0 && a + 2 <= m0.byteLength ? m0.getInt16(a, l) : 0), getInt32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getInt32(a, l) : 0), getFloat32: (a, l) => (a >= 0 && a + 4 <= m0.byteLength ? m0.getFloat32(a, l) : 0) };
            const o = m.getUint32(0xb20448, true);
            return [m.getInt16(o + 0x38, true), m.getInt16(o + 0x40, true), m.getInt32(o + 0x44, true), m.getInt32(o + 0x48, true), m.getUint32(0xb1e0e0 + 0xc, true)];
          });
          out.push([crypto.createHash('md5').update(buf).digest('hex').slice(0, 8), st]);
        }
        return out;
      };
      // Draw calls of frame +DRAWS: per draw, the bound texture's last upload hash, the vertex
      // count and a hash of the vertex data. Raw restore vs portable load.
      const draws = async (load) => {
        await load();
        await play(a, f, f + Number(process.env.DRAWS) - 1);
        await a.evaluate(() => {
          const gl = document.getElementById('canvas').getContext('webgl2') || document.getElementById('canvas').getContext('webgl');
          const P = Object.getPrototypeOf(gl);
          window.__log = [];
          const h = (arr) => { let x = 2166136261; const u = new Uint8Array(arr.buffer, arr.byteOffset, arr.byteLength); for (let i = 0; i < u.length; i++) x = Math.imul(x ^ u[i], 16777619); return (x >>> 0).toString(16); };
          window.__texHash ??= new Map();
          if (!window.__hooked) {
            window.__hooked = true;
            const bt = P.bindTexture, ti = P.texImage2D, bd = P.bufferData, da = P.drawArrays;
            P.bindTexture = function (t, tex) { window.__bound = tex; return bt.apply(this, arguments); };
            P.texImage2D = function () { const d = arguments[arguments.length - 1]; if (d && d.buffer) window.__texHash.set(window.__bound, h(d)); return ti.apply(this, arguments); };
            P.bufferData = function (t, d) { if (d && d.buffer) window.__vbo = [d.length, h(d), Array.from(d.slice(0, 4)).map((v) => Math.round(v * 100) / 100)]; return bd.apply(this, arguments); };
            P.drawArrays = function () { if (window.__log) window.__log.push([window.__texHash.get(window.__bound) || '-', ...window.__vbo]); return da.apply(this, arguments); };
          }
        });
        await play(a, f + Number(process.env.DRAWS) - 1, f + Number(process.env.DRAWS));
        return a.evaluate(() => { const l = window.__log; window.__log = null; return l; });
      };
      // The objects the render walk draws: gObjParentGraphNode's children ring, at frame +RING.
      const ring = () => a.evaluate(() => {
        const m = new DataView(window.engine.memory.buffer);
        const out = []; const c = m.getUint32(0xb1a0dc + 0x10, true); let x = c;
        do { const slot = (x - 0xb20b60) / 0x260; out.push(Number.isInteger(slot) && slot >= 0 && slot < 240 ? `s${slot}${m.getInt16(x + 2, true) & 1 ? '' : '(off)'}` : `@${x.toString(16)} t${m.getInt16(x, true).toString(16)} f${m.getInt16(x + 2, true).toString(16)}`); x = m.getUint32(x + 8, true); } while (x && x !== c && out.length < 400);
        return out;
      });
      // Mario's model as the render walk traverses it: every node's children ring followed by
      // `next` until it returns (no de-duplication), as types; depth-first like geo_process.
      const marioTree = () => a.evaluate(() => {
        const m = new DataView(window.engine.memory.buffer);
        const obj = m.getUint32(0xb20448, true); const out = [];
        const rec = (first, depth) => {
          if (!first || depth > 30 || out.length > 3000) return;
          let x = first, n = 0;
          do { const par = m.getUint32(x + 0xc, true); const ty = m.getInt16(x, true) & 0xffff; const dl = [0x19, 0x17, 0x1b, 0x15, 0x16, 0x1a, 0x1c, 0x12a, 0x10c, 0x12e].includes(ty) ? m.getUint32(x + 0x14, true).toString(16) : ''; out.push(`${depth}:${ty.toString(16)} parent ${par ? m.getInt16(par, true).toString(16) : 'null'} dl ${dl} f${m.getInt16(x + 2, true).toString(16)}${ty === 0x10c ? ' sel ' + m.getInt16(x + 0x1a, true) + '/' + m.getInt16(x + 0x18, true) : ''}`); rec(m.getUint32(x + 0x10, true), depth + 1); x = m.getUint32(x + 8, true); n++; } while (x && x !== first && n < 60);
          if (x !== first) out.push(`${depth}:RING-NOT-CLOSED`);
        };
        rec(m.getUint32(obj + 0x14, true), 0);
        return out;
      });
      // The frame's display list (F3DEX2): matrices (hash of the 64 bytes), sub-list calls and
      // vertex loads, in execution order. Raw restore vs loaded state, after frame +DL.
      const dlist = () => a.evaluate(() => {
        const m = new DataView(window.engine.memory.buffer); const len = m.byteLength;
        const u = (x) => (x >= 0 && x + 4 <= len ? m.getUint32(x, true) : 0);
        const h = (x, n) => { let v = 2166136261; for (let i = 0; i < n; i++) v = Math.imul(v ^ (x + i < len ? m.getUint8(x + i) : 0), 16777619); return (v >>> 0).toString(16); };
        const out = []; const stack = [];
        let pc = u(0xb1dfe4); let steps = 0;
        while (pc && steps++ < 200000) {
          const w0 = u(pc), w1 = u(pc + 4), op = w0 >>> 24; pc += 8;
          if (op === 0xde) { out.push(`DL ${w1.toString(16)}`); if (((w0 >>> 16) & 0xff) === 0) stack.push(pc); pc = w1; }
          else if (op === 0xdf) { pc = stack.length ? stack.pop() : 0; }
          else if (op === 0xda) out.push(`MTX ${h(w1, 64)} ${((w0 >>> 0) & 0xff).toString(16)} ${Array.from({ length: 16 }, (_, i) => (w1 + 4 * i + 4 <= len ? m.getFloat32(w1 + 4 * i, true) : 0).toFixed(3)).join(',')}`);
          else if (op === 0x01) out.push(`VTX ${w1.toString(16)} n${(w0 >>> 12) & 0xff}`);
        }
        return out;
      });
      // Only Mario drawn on frame +ANIMG; then the animation globals he left, raw vs loaded.
      if (process.env.ANIMG) {
        const at = Number(process.env.ANIMG);
        const run = async (load) => {
          await load(); await play(a, f, f + at - 1);
          await a.evaluate(() => { const m = new DataView(window.engine.memory.buffer); const mario = m.getUint32(0xb20448, true); for (let s = 0; s < 240; s++) { const o = 0xb20b60 + s * 0x260; if (o !== mario) m.setInt16(o + 2, m.getInt16(o + 2, true) & ~1, true); } });
          await play(a, f + at - 1, f + at);
          return a.evaluate(() => {
            const m = new DataView(window.engine.memory.buffer); const mario = m.getUint32(0xb20448, true); const cur = m.getUint32(mario + 0x3c, true);
            const data = m.getUint32(0xb44ca4, true), attr = m.getUint32(0xb44ca0, true);
            return { type: m.getInt8(0xb44c9a), frame: m.getInt16(0xb44c9c, true), enabled: m.getInt8(0xb44c9e), mult: m.getFloat32(0xb44ca8, true),
              dataOff: data - cur, attrOff: attr - cur, animYTrans: m.getInt16(mario + 0x3a, true), divisor: m.getInt16(cur + 2, true), flags: m.getInt16(cur, true),
              curAnimHead: Array.from({ length: 6 }, (_, i) => m.getUint32(cur + 4 * i, true).toString(16)) };
          });
        };
        console.log('animg raw', JSON.stringify(await run(() => a.evaluate(() => window.vRestore()))));
        console.log('animg loaded', JSON.stringify(await run(() => a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64')))));
      }
      if (process.env.DLAT) {
        const at = Number(process.env.DLAT);
        await a.evaluate(() => window.vRestore()); await play(a, f, f + at); const l1 = await dlist();
        await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64')); await play(a, f, f + at); const l2 = await dlist();
        fs.writeFileSync(path.join(dir, 'dl.json'), JSON.stringify([l1, l2]));
        console.log(`dl: raw ${l1.length} entries, loaded ${l2.length}`);
      }
      if (process.env.SEGS) {
        const segs = () => a.evaluate((at) => Array.from({ length: 32 }, (_, i) => new DataView(window.engine.memory.buffer).getUint32(at + 4 * i, true).toString(16)), Number(process.env.SEGS));
        await a.evaluate(() => window.vRestore()); const s1 = await segs();
        await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64')); const s2 = await segs();
        for (let i = 0; i < 32; i++) console.log(`seg ${i}: raw ${s1[i]} loaded ${s2[i]}${s1[i] === s2[i] ? '' : '   <<<'}`);
      }
      if (process.env.TREE) {
        await a.evaluate(() => window.vRestore()); await play(a, f, f + Number(process.env.TREE)); const t1 = await marioTree();
        await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64')); await play(a, f, f + Number(process.env.TREE)); const t2 = await marioTree();
        fs.writeFileSync(path.join(dir, 'tree.json'), JSON.stringify([t1, t2]));
        console.log(`tree: raw ${t1.length} nodes, portable ${t2.length}`);
      }
      if (process.env.RING) {
        await a.evaluate(() => window.vRestore()); await play(a, f, f + Number(process.env.RING)); const q1 = await ring();
        await a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, `own_${f}.p`)).toString('base64')); await play(a, f, f + Number(process.env.RING)); const q2 = await ring();
        console.log('ring raw', q1.length, q1.join(' '));
        console.log('ring portable', q2.length, q2.join(' '));
      }
      // Per object: the vertex floats drawn on frame +BLAME with that object hidden, both paths.
      if (process.env.BLAME) {
        const B = Number(process.env.BLAME);
        await a.evaluate(() => {
          const gl = document.getElementById('canvas').getContext('webgl2') || document.getElementById('canvas').getContext('webgl');
          const P = Object.getPrototypeOf(gl); const bd = P.bufferData;
          window.__floats = 0; P.bufferData = function (t, d) { if (d && d.length) window.__floats += d.length; return bd.apply(this, arguments); };
        });
        const measure = async (load, hide) => {
          await load(); await play(a, f, f + B - 1);
          if (hide >= 0) await a.evaluate((s) => { const m = new DataView(window.engine.memory.buffer); const o = 0xb20b60 + s * 0x260; m.setInt16(o + 2, m.getInt16(o + 2, true) & ~1, true); }, hide);
          await a.evaluate(() => { window.__floats = 0; });
          await play(a, f + B - 1, f + B);
          return a.evaluate(() => window.__floats);
        };
        const raw = () => a.evaluate(() => window.vRestore());
        const port = () => a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64'));
        const slots = await a.evaluate(() => { const m = new DataView(window.engine.memory.buffer); const o = []; for (let s = 0; s < 240; s++) if (m.getInt16(0xb20b60 + s * 0x260 + 0x74, true) && (m.getInt16(0xb20b60 + s * 0x260 + 2, true) & 1)) o.push(s); return o; });
        const r0 = await measure(raw, -1), p0 = await measure(port, -1);
        console.log(`blame: all drawn raw ${r0} portable ${p0}`);
        for (const s of slots) {
          const r = await measure(raw, s), p = await measure(port, s);
          if (r0 - r !== p0 - p) console.log(`blame slot ${s}: raw share ${r0 - r}, portable share ${p0 - p}`);
        }
      }
      if (process.env.DRAWS) {
        const r1 = await draws(() => a.evaluate(() => window.vRestore()));
        const r2 = await draws(() => a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64')));
        console.log(`draws: raw ${r1.length}, portable ${r2.length}`);
        for (let i = 0; i < Math.max(r1.length, r2.length); i++) if (JSON.stringify(r1[i]) !== JSON.stringify(r2[i])) console.log(`draw ${i}: raw ${JSON.stringify(r1[i])} portable ${JSON.stringify(r2[i])}`);
      }
      const tr1 = await trace(() => a.evaluate(() => window.vRestore()));
      const tr2 = await trace(() => a.evaluate((x) => window.vLoad(x), fs.readFileSync(path.join(dir, process.env.TREEFILE || `own_${f}.p`)).toString('base64')));
      if (process.env.TRACE) for (let i = 0; i < K; i++) console.log(`+${i + 1}`, tr1[i][0] === tr2[i][0] ? 'same' : 'DIFF', JSON.stringify(tr1[i][1]), JSON.stringify(tr2[i][1]));
      console.log('dma portable', JSON.stringify(t2));
      for (const [tag, file] of [['own', `own_${f}.p`], ['h', `h_${f}.p`]]) {
        if (!fs.existsSync(path.join(dir, file))) continue;
        const b = await open();
        if (process.env.FULLTEX) await b.evaluate(() => { globalThis.__fullTexReset = true; });
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
