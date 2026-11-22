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
const browser = await chromium.launch({ args: ['--no-sandbox', '--disable-dev-shm-usage', '--use-angle=swiftshader-webgl', '--enable-unsafe-swiftshader'] });
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
  } else {
    console.log('usage: see the header of this file');
  }
} finally {
  await browser.close();
  await server.close();
}
