// MP4 export. Plays a frame range on the viewer's emulator one frame at a
// time, draws each picture with the annotations over it and encodes it with
// WebCodecs, so the video has every frame (none dropped, as a screen capture
// would) and is made faster than real time on most machines.
//
// Codecs: H.264 + AAC where the browser has encoders for them (Chrome and
// Edge on Windows and macOS do), else VP9 + Opus, in an MP4 either way. YouTube
// takes both.
//
// Pictures go to the encoder as TV-range BT.709 YUV (tvrange.js), not as the
// canvas' RGB, which Chrome encodes as full range that many players show too
// dark.

import { ArrayBufferTarget, FileSystemWritableFileStreamTarget, Muxer } from 'mp4-muxer';
import { FPS, formatTime } from './m64.js';
import { regionAt } from './regions.js';
import { BUTTONS, courseName } from './sm64.js';
import { createTvRangeConverter } from './tvrange.js';

export const EXPORT_SIZES = [
  { id: '1920x1080', w: 1920, h: 1080, label: '1920 × 1080 (16:9)' },
  { id: '3840x2160', w: 3840, h: 2160, label: '3840 × 2160 (4K, 16:9)' },
  { id: '1280x720', w: 1280, h: 720, label: '1280 × 720 (16:9)' },
  { id: '1440x1080', w: 1440, h: 1080, label: '1440 × 1080 (4:3)' },
  { id: '2880x2160', w: 2880, h: 2160, label: '2880 × 2160 (4K, 4:3)' },
  { id: '960x720', w: 960, h: 720, label: '960 × 720 (4:3)' },
];

// Codec strings carry a level, the largest picture size and rate the stream
// may use; above 1080p (VP9 and AV1 level 4.1's limit) they need level 5.1.
//
// Encoders run at constant quality where they can (bitrateMode 'quantizer',
// `quantizer` below; lower is better: 0-51 for H.264, 0-63 for VP9 and AV1):
// with no bitrate to hit there is no rate control, which on some hardware
// H.264 encoders drops frames when they're fed faster than real time.
const BIG = 2228224; // pixels: VP9 / AV1 level 4.1's largest picture
const VIDEO_CODECS = [
  {
    muxer: 'avc',
    label: 'H.264',
    quantizer: 20,
    codec: (w, h) => `avc1.6400${w * h > BIG ? '33' : '28'}`,
  },
  { muxer: 'vp9', label: 'VP9', quantizer: 24, codec: (w, h) => `vp09.00.${w * h > BIG ? '51' : '41'}.08` },
  { muxer: 'av1', label: 'AV1', quantizer: 28, codec: (w, h) => `av01.0.${w * h > BIG ? '13' : '09'}M.08` },
];
const AUDIO_CODECS = [
  { codec: 'mp4a.40.2', muxer: 'aac', label: 'AAC' },
  { codec: 'opus', muxer: 'opus', label: 'Opus' },
];

// 12 Mbps at 1080p (YouTube suggests 8 for 1080p30 uploads; the game's
// sharp edges and HUD text like a little more), scaled by pixel count, up to
// YouTube's 45 Mbps for 4K.
const videoBitrate = (w, h) => Math.min(45e6, Math.round((12e6 * w * h) / (1920 * 1080)));
const AUDIO_BITRATE = 192000;
const KEYFRAME_SECONDS = 2;

function videoConfig(c, w, h, constantQuality) {
  const config = { codec: c.codec(w, h), width: w, height: h, framerate: FPS, latencyMode: 'quality' };
  if (constantQuality) config.bitrateMode = 'quantizer';
  else config.bitrate = videoBitrate(w, h);
  if (c.muxer === 'avc') config.avc = { format: 'avc' };
  return config;
}

const audioConfig = (c, sampleRate) => ({ codec: c.codec, sampleRate, numberOfChannels: 2, bitrate: AUDIO_BITRATE });

async function firstSupported(Encoder, list, configOf) {
  if (typeof Encoder === 'undefined') return null;
  for (const c of list) {
    const config = configOf(c);
    try {
      if ((await Encoder.isConfigSupported(config)).supported) return { ...c, config };
    } catch {
      // Not this one.
    }
  }
  return null;
}

/**
 * Every video codec with how this browser can encode it at w x h: `config`
 * at constant quality (`quantizer` set) or else at a bitrate (`quantizer`
 * null), or `config` null if it can't. In order of preference.
 */
export async function videoCodecs(w, h) {
  return Promise.all(
    VIDEO_CODECS.map(async (c) => {
      if (typeof VideoEncoder !== 'undefined') {
        for (const constantQuality of [true, false]) {
          const config = videoConfig(c, w, h, constantQuality);
          try {
            if ((await VideoEncoder.isConfigSupported(config)).supported) {
              return { ...c, config, quantizer: constantQuality ? c.quantizer : null };
            }
          } catch {
            // Not this way.
          }
        }
      }
      return { ...c, config: null, quantizer: null };
    }),
  );
}

/** The best audio codec for the game's sound, or null (no encoder, or no sound to tap). */
export async function pickAudioCodec(engine) {
  const tap = new AudioTap(engine);
  if (!tap.available) return null;
  return firstSupported(globalThis.AudioEncoder, AUDIO_CODECS, (c) => audioConfig(c, tap.sampleRate));
}

/**
 * The game's sound, pulled at an exact rate. The browser build plays it with
 * SDL: its glue pulls one buffer from SDL's queue whenever its
 * ScriptProcessorNode asks, and the game tops the queue up every frame,
 * making a little more or less sound depending on how full it is. During an
 * export the real AudioContext is suspended and the exporter does the
 * pulling, exactly 1/FPS seconds' worth per frame, so the game makes the
 * sound it makes when played at 1x, in step with the frames.
 */
class AudioTap {
  constructor(engine) {
    this.sdl = engine?.game?.SDL2;
    this.node = this.sdl?.audio?.scriptProcessorNode;
  }

  get available() {
    return !!(this.node?.onaudioprocess && this.sdl.audioContext);
  }

  get sampleRate() {
    return this.sdl.audioContext.sampleRate;
  }

  async start() {
    this.ctx = this.sdl.audioContext;
    this.wasRunning = this.ctx.state === 'running';
    await this.ctx.suspend();
    this.size = this.node.bufferSize;
    this.pulled = 0;
  }

  /** Pulls whole buffers until `total` samples have been pulled in all; returns them as [left, right]. */
  pullTo(total) {
    const n = Math.max(0, Math.floor((total - this.pulled) / this.size)) * this.size;
    if (n === 0) return null;
    const out = [new Float32Array(n), new Float32Array(n)];
    for (let at = 0; at < n; at += this.size) {
      const data = [out[0].subarray(at, at + this.size), out[1].subarray(at, at + this.size)];
      const outputBuffer = {
        numberOfChannels: 2,
        length: this.size,
        sampleRate: this.sampleRate,
        getChannelData: (c) => data[c],
      };
      this.node.onaudioprocess({ outputBuffer });
    }
    this.pulled += n;
    return out;
  }

  async stop() {
    if (this.wasRunning) await this.ctx?.resume();
  }
}

// --- Overlays, drawn at 1080p sizes scaled to the video's height ---

const FONT = 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif';
const MONO = 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
const ACCENT = '#f7c948';

function roundRect(g, x, y, w, h, r) {
  g.beginPath();
  g.roundRect(x, y, w, h, r);
}

function wrapText(g, text, maxWidth, maxLines) {
  const lines = [];
  let line = '';
  for (const word of text.split(/\s+/)) {
    const next = line ? `${line} ${word}` : word;
    if (g.measureText(next).width <= maxWidth || !line) line = next;
    else {
      lines.push(line);
      line = word;
    }
  }
  if (line) lines.push(line);
  if (lines.length > maxLines) {
    lines.length = maxLines;
    lines[maxLines - 1] += ' …';
  }
  return lines;
}

function panel(g, x, y, w, h, s, color) {
  roundRect(g, x, y, w, h, 10 * s);
  g.fillStyle = 'rgba(10, 12, 18, 0.72)';
  g.fill();
  g.strokeStyle = 'rgba(255, 255, 255, 0.08)';
  g.lineWidth = Math.max(1, s);
  g.stroke();
  if (color) {
    g.save();
    g.clip();
    g.fillStyle = color;
    g.fillRect(x, y, 5 * s, h);
    g.restore();
  }
}

/** The current region's name, frame range and note, bottom left (as in the viewer). */
function drawRegionChip(g, region, W, H, s, bottom) {
  const margin = 24 * s;
  const pad = 16 * s;
  const maxW = Math.min(820 * s, W - 2 * margin);
  g.font = `700 ${28 * s}px ${FONT}`;
  const titleW = g.measureText(region.label).width;
  g.font = `${20 * s}px ${MONO}`;
  const range = `${region.from}–${region.to}`;
  const rangeW = g.measureText(range).width;
  g.font = `${22 * s}px ${FONT}`;
  const note = region.note ? wrapText(g, region.note, maxW - 2 * pad - 5 * s, 3) : [];
  const noteW = Math.max(0, ...note.map((l) => g.measureText(l).width));
  const w = Math.min(maxW, Math.max(titleW + 14 * s + rangeW, noteW) + 2 * pad + 5 * s);
  const h = pad * 2 + 32 * s + note.length * 29 * s;
  const x = margin;
  const y = H - bottom - margin - h;
  panel(g, x, y, w, h, s, region.color);
  g.textBaseline = 'top';
  g.fillStyle = '#e6e9ef';
  g.font = `700 ${28 * s}px ${FONT}`;
  g.fillText(region.label, x + pad + 5 * s, y + pad);
  g.fillStyle = '#8b93a7';
  g.font = `${20 * s}px ${MONO}`;
  g.fillText(range, x + pad + 5 * s + titleW + 14 * s, y + pad + 6 * s);
  g.font = `${22 * s}px ${FONT}`;
  note.forEach((l, i) => g.fillText(l, x + pad + 5 * s, y + pad + 36 * s + i * 29 * s));
}

/**
 * The regions along the bottom edge, along the exported timeline (the
 * segments back to back, with a dark mark at each cut), with a playhead at
 * exported frame `k`.
 */
function drawRegionBar(g, regions, segments, total, k, W, H, s) {
  const h = 12 * s;
  const y = H - h;
  const xOf = (i) => (i / Math.max(1, total)) * W;
  g.fillStyle = 'rgba(10, 12, 18, 0.75)';
  g.fillRect(0, y, W, h);
  const head = xOf(k);
  let offset = 0;
  for (const seg of segments) {
    for (const r of regions) {
      const lo = Math.max(r.from, seg.from);
      const hi = Math.min(r.to, seg.to);
      if (hi <= lo) continue;
      const a = xOf(offset + lo - seg.from);
      const b = xOf(offset + hi - seg.from);
      g.fillStyle = r.color;
      g.globalAlpha = 0.9;
      g.fillRect(a, y, Math.max(0, Math.min(b, head) - a), h);
      g.globalAlpha = 0.35;
      g.fillRect(Math.max(a, head), y, Math.max(0, b - Math.max(a, head)), h);
      g.globalAlpha = 1;
      g.fillStyle = 'rgba(10, 12, 18, 0.9)';
      g.fillRect(a, y, Math.max(1, s), h);
    }
    if (offset > 0) {
      g.fillStyle = '#05060a';
      g.fillRect(xOf(offset) - 2 * s, y - 2 * s, 4 * s, h + 2 * s);
    }
    offset += seg.to - seg.from;
  }
  g.fillStyle = ACCENT;
  g.fillRect(head - 1.5 * s, y - 4 * s, 3 * s, h + 4 * s);
}

/** Stars, course, frame and time, top left under the game's own HUD row (lives, coins, stars). */
function drawHud(g, state, frame, W, H, s) {
  if (!state) return;
  const pad = 14 * s;
  const lines = [
    `★ ${state.numStars}   ${courseName(state.courseNum)}   area ${state.areaIndex}`,
    `frame ${frame} · ${formatTime(frame)}`,
  ];
  g.font = `600 ${24 * s}px ${FONT}`;
  const w = Math.max(...lines.map((l) => g.measureText(l).width)) + 2 * pad;
  const h = 2 * pad + 2 * 32 * s;
  const x = 24 * s;
  const y = 150 * s;
  panel(g, x, y, w, h, s);
  g.textBaseline = 'top';
  g.fillStyle = '#e6e9ef';
  g.fillText(lines[0], x + pad, y + pad);
  g.font = `${22 * s}px ${MONO}`;
  g.fillStyle = '#c4c9d4';
  g.fillText(lines[1], x + pad, y + pad + 34 * s);
}

/**
 * The controller input of the frame just played (as the side panel shows
 * it): stick, buttons and stick values, bottom right above the game's camera
 * icon.
 */
function drawInput(g, pad, W, H, s, bottom) {
  const x = pad?.stick_x ?? 0;
  const y = pad?.stick_y ?? 0;
  const button = pad?.button ?? 0;
  const inset = 14 * s;
  const ring = 44 * s;
  const btnW = 38 * s;
  const btnH = 28 * s;
  const gap = 5 * s;
  const cols = 7;
  const gridW = cols * btnW + (cols - 1) * gap;
  const w = 2 * inset + 2 * ring + 18 * s + gridW;
  const h = 2 * inset + 2 * ring;
  const px = W - 24 * s - w;
  const py = H - bottom - 190 * s - h;
  panel(g, px, py, w, h, s);

  // Stick: the range is about ±80 after the deadzone; the dot stays in the ring.
  const cx = px + inset + ring;
  const cy = py + inset + ring;
  const r = ring - 8 * s;
  const dx = Math.max(-1, Math.min(1, x / 80)) * r;
  const dy = -Math.max(-1, Math.min(1, y / 80)) * r;
  g.beginPath();
  g.arc(cx, cy, ring - 2 * s, 0, 2 * Math.PI);
  g.fillStyle = '#0d0f14';
  g.fill();
  g.strokeStyle = '#2a3142';
  g.lineWidth = 2 * s;
  g.stroke();
  g.beginPath();
  g.moveTo(cx, cy);
  g.lineTo(cx + dx, cy + dy);
  g.strokeStyle = '#8b93a7';
  g.stroke();
  g.beginPath();
  g.arc(cx + dx, cy + dy, 8 * s, 0, 2 * Math.PI);
  g.fillStyle = ACCENT;
  g.fill();

  // Buttons, two rows of seven, lit while pressed; the stick values below.
  const bx = cx + ring + 18 * s;
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  BUTTONS.forEach((b, i) => {
    const on = (button & b.mask) !== 0;
    const x0 = bx + (i % cols) * (btnW + gap);
    const y0 = py + inset + Math.floor(i / cols) * (btnH + gap);
    roundRect(g, x0, y0, btnW, btnH, 5 * s);
    g.fillStyle = on ? ACCENT : '#1c2130';
    g.fill();
    g.strokeStyle = on ? ACCENT : '#2a3142';
    g.lineWidth = Math.max(1, s);
    g.stroke();
    g.fillStyle = on ? '#1a1400' : '#8b93a7';
    g.font = `${on ? 700 : 500} ${16 * s}px ${FONT}`;
    g.fillText(b.name, x0 + btnW / 2, y0 + btnH / 2 + s);
  });
  g.textAlign = 'left';
  g.font = `${20 * s}px ${MONO}`;
  g.fillStyle = '#c4c9d4';
  g.fillText(`x ${String(x).padStart(4)}  y ${String(y).padStart(4)}`, bx, py + inset + 2 * (btnH + gap) + 18 * s);
  g.textBaseline = 'top';
}

const nextTask = () => new Promise((r) => setTimeout(r, 0));
const nextAnimationFrame = () => new Promise((r) => requestAnimationFrame(() => r()));

export class ExportCancelled extends Error {}

/** `list` of { from, to } clamped to [0, total], sorted, with overlapping or touching ones merged. */
export function mergeSegments(list, total) {
  const sorted = list
    .map((r) => ({ from: Math.max(0, Math.round(r.from)), to: Math.min(total, Math.round(r.to)) }))
    .filter((r) => Number.isFinite(r.from) && Number.isFinite(r.to) && r.to > r.from)
    .sort((a, b) => a.from - b.from);
  const out = [];
  for (const r of sorted) {
    const last = out[out.length - 1];
    if (last && r.from <= last.to) last.to = Math.max(last.to, r.to);
    else out.push({ ...r });
  }
  return out;
}

/**
 * Exports `segments` (from mergeSegments) of the player's replay back to back
 * into one video, fast-forwarding over the gaps. The player must be at or
 * before the first segment. `handle` is a FileSystemFileHandle to stream
 * into, or null to build the file in memory and return it as a Blob.
 *
 * opts: { player, regions, segments, size: {w, h}, overlays: {chip, bar, hud, input},
 *         video (an entry of videoCodecs with a config), audio (from pickAudioCodec, may be
 *         null), handle, onProgress({ phase, done, total }), signal }
 */
export async function exportMp4(opts) {
  const { player: p, regions, segments, size, overlays, video, audio, handle, onProgress, signal } = opts;
  const { w: W, h: H } = size;
  const s = H / 1080;
  const total = segments.reduce((n, seg) => n + seg.to - seg.from, 0);
  const check = () => {
    if (signal?.aborted) throw new ExportCancelled('Export cancelled');
  };

  p.pause();
  p.exporting = true;
  const resolution = p.resolution.res;
  const tap = audio ? new AudioTap(p.engine) : null;
  let videoEncoder = null;
  let tv = null;
  let audioEncoder = null;
  let writable = null;
  let finished = false;
  let last = performance.now();

  // Up to `frame` without sound (between segments the sound pauses with the video).
  const fastForward = async (frame) => {
    if (p.frame >= frame) return;
    p.setAudio(false);
    const start = p.frame;
    while (p.frame < frame) {
      p.advance();
      if (performance.now() - last > 30) {
        check();
        onProgress?.({ phase: 'seek', done: p.frame - start, total: frame - start });
        p.emit();
        await nextTask();
        last = performance.now();
      }
    }
    if (tap) p.setAudio(true);
  };

  try {
    // 1. Up to the first frame.
    await fastForward(segments[0].from);

    // 2. Render at the export size (the game resizes on the next frames).
    p.setResolution({ id: 'export', kind: 'fixed', w: W, h: H, label: '' });
    await nextAnimationFrame();
    await nextAnimationFrame();
    check();

    // 3. Encoders and the file.
    let target;
    if (handle) {
      writable = await handle.createWritable();
      target = new FileSystemWritableFileStreamTarget(writable);
    } else {
      target = new ArrayBufferTarget();
    }
    const muxer = new Muxer({
      target,
      video: { codec: video.muxer, width: W, height: H, frameRate: FPS },
      audio: audio ? { codec: audio.muxer, numberOfChannels: 2, sampleRate: audio.config.sampleRate } : undefined,
      fastStart: handle ? false : 'in-memory',
      firstTimestampBehavior: 'offset',
    });
    let failure = null;
    const frameUs = 1e6 / FPS;
    // Every frame sent should come back encoded. Some encoders skip frames
    // instead (their rate control giving up), which makes the video stutter:
    // stop then rather than finish a broken file. Outputs may come a few
    // frames late or reordered, hence the slack.
    // Where the time goes, per step of the frame loop (reported at the end).
    const timings = {};
    const time = (step, since) => {
      const now = performance.now();
      timings[step] = (timings[step] ?? 0) + now - since;
      return now;
    };
    let received = 0;
    let highest = -1;
    const skipped = () => highest + 1 - received;
    const skipError = () =>
      new Error(
        `The ${video.label} encoder skipped frames (${skipped()} so far), which makes the video stutter. ` +
          `Choose another codec under Codec${video.muxer === 'vp9' ? '' : ' (VP9 is the safest)'} or a smaller size.`,
      );
    videoEncoder = new VideoEncoder({
      output: (chunk, meta) => {
        muxer.addVideoChunk(chunk, meta);
        received += 1;
        highest = Math.max(highest, Math.round(chunk.timestamp / frameUs));
        if (skipped() > 8 && !failure) failure = skipError();
      },
      error: (e) => (failure = e),
    });
    videoEncoder.configure(video.config);
    if (audio) {
      audioEncoder = new AudioEncoder({
        output: (chunk, meta) => muxer.addAudioChunk(chunk, meta),
        error: (e) => (failure = e),
      });
      audioEncoder.configure(audio.config);
      await tap.start();
      p.setAudio(true);
    }

    const canvas =
      typeof OffscreenCanvas !== 'undefined'
        ? new OffscreenCanvas(W, H)
        : Object.assign(document.createElement('canvas'), { width: W, height: H });
    const g = canvas.getContext('2d');
    g.imageSmoothingQuality = 'high';
    tv = createTvRangeConverter(W, H);
    const keyEvery = KEYFRAME_SECONDS * FPS;
    const bar = overlays.bar && regions.length > 0 ? 12 * s : 0;

    // 4. Every frame: step (which draws it), copy the picture in the same
    // task (a WebGL canvas is cleared once shown), add the overlays, encode.
    // `k` counts exported frames, across segments.
    let k = 0;
    for (const seg of segments) {
      await fastForward(seg.from);
      while (p.frame < seg.to) {
        let t = performance.now();
        p.advance();
        t = time('game', t);
        g.drawImage(p.canvas, 0, 0, W, H);
        if (k === total - 1) p.capture(); // the picture shown paused afterwards
        const frame = p.frame;
        const region = regionAt(regions, frame);
        if (overlays.hud) drawHud(g, p.state, frame, W, H, s);
        if (overlays.input) drawInput(g, p.pads[frame - 1], W, H, s, bar);
        if (overlays.chip && region) drawRegionChip(g, region, W, H, s, bar);
        if (bar) drawRegionBar(g, regions, segments, total, k + 1, W, H, s);
        t = time('overlays', t);
        const timestamp = Math.round(k * frameUs);
        const vf = tv
          ? tv.frame(canvas, timestamp, Math.round(frameUs))
          : new VideoFrame(canvas, { timestamp, duration: Math.round(frameUs) });
        videoEncoder.encode(vf, {
          keyFrame: k % keyEvery === 0 || p.frame === seg.from + 1,
          ...(video.quantizer !== null && { [video.muxer]: { quantizer: video.quantizer } }),
        });
        vf.close();
        t = time('encode', t);

        if (tap) {
          const start = tap.pulled;
          const pcm = tap.pullTo(Math.floor(((k + 1) * tap.sampleRate) / FPS));
          if (pcm) {
            const data = new Float32Array(pcm[0].length * 2);
            data.set(pcm[0]);
            data.set(pcm[1], pcm[0].length);
            const ad = new AudioData({
              format: 'f32-planar',
              sampleRate: tap.sampleRate,
              numberOfFrames: pcm[0].length,
              numberOfChannels: 2,
              timestamp: Math.round((start / tap.sampleRate) * 1e6),
              data,
            });
            audioEncoder.encode(ad);
            ad.close();
          }
        }
        t = time('sound', t);
        k += 1;

        if (failure) throw failure;
        while (videoEncoder.encodeQueueSize > 6) {
          await new Promise((r) => videoEncoder.addEventListener('dequeue', r, { once: true }));
        }
        time('waiting for the encoder', t);
        if (performance.now() - last > 50) {
          check();
          onProgress?.({ phase: 'encode', done: k, total });
          p.emit();
          await nextTask();
          last = performance.now();
        }
      }
    }

    // 5. Finish the file.
    onProgress?.({ phase: 'finish', done: total, total });
    await videoEncoder.flush();
    if (audioEncoder) await audioEncoder.flush();
    if (failure) throw failure;
    if (received < total) throw skipError();
    const spent = Object.values(timings).reduce((a, b) => a + b, 0);
    const profile = Object.entries(timings)
      .sort((a, b) => b[1] - a[1])
      .map(([step, ms]) => `${step} ${Math.round((100 * ms) / spent)}%`)
      .join(', ');
    console.info(`[export] ${video.label} ${W}x${H}, ${total} frames: ${profile}`, timings);
    onProgress?.({ phase: 'finish', done: total, total, profile });
    muxer.finalize();
    finished = true;
    if (writable) {
      await writable.close();
      return null;
    }
    return new Blob([target.buffer], { type: 'video/mp4' });
  } finally {
    for (const e of [videoEncoder, audioEncoder]) {
      if (e && e.state !== 'closed') e.close();
    }
    tv?.close();
    if (writable && !finished) await writable.abort().catch(() => {});
    p.setAudio(false);
    await tap?.stop();
    // The last frame was captured (p.capture above); don't let tick() take
    // another copy of the now-cleared canvas, and show the captured one.
    p.rendered = false;
    p.setResolution(resolution);
    p.exporting = false;
    p.showFrozen();
    p.emit();
  }
}
