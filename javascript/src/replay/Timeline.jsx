import { useEffect, useMemo, useRef, useState } from 'react';
import { formatTime } from './m64.js';
import { regionAt } from './regions.js';

const LANES = [
  { mask: 0x8000, color: '#4ade80' }, // A
  { mask: 0x4000, color: '#60a5fa' }, // B
  { mask: 0x2000, color: '#c084fc' }, // Z
];

/**
 * Scrubbable timeline. The strip shows, per pixel column, stick magnitude
 * (grey) and A / B / Z presses (coloured lanes) of the frames it covers.
 * Annotated `regions` (see lib/regions.js) get a labelled track above it;
 * clicking one seeks to its start.
 */
export default function Timeline({ pads, frame, seekTarget, events, regions = [], milestones, onSeek }) {
  const strip = useRef(null);
  const wrap = useRef(null);
  const drag = useRef(false);
  const [width, setWidth] = useState(800);
  const [hover, setHover] = useState(null);
  const total = pads.length;

  useEffect(() => {
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(100, Math.floor(e.contentRect.width))));
    if (wrap.current) ro.observe(wrap.current);
    return () => ro.disconnect();
  }, []);

  // Per-column summaries.
  const columns = useMemo(() => {
    const cols = [];
    for (let x = 0; x < width; x++) {
      const a = Math.floor((x / width) * total);
      const b = Math.max(a + 1, Math.floor(((x + 1) / width) * total));
      let stick = 0;
      const lanes = LANES.map(() => 0);
      for (let i = a; i < b && i < total; i++) {
        const p = pads[i];
        stick = Math.max(stick, Math.hypot(p.stick_x, p.stick_y) / 128);
        LANES.forEach((l, k) => {
          if (p.button & l.mask) lanes[k] += 1;
        });
      }
      cols.push({ stick, lanes: lanes.map((c) => c / Math.max(1, b - a)) });
    }
    return cols;
  }, [pads, width, total]);

  useEffect(() => {
    const c = strip.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const h = 36;
    c.width = width * dpr;
    c.height = h * dpr;
    const g = c.getContext('2d');
    g.scale(dpr, dpr);
    g.clearRect(0, 0, width, h);
    columns.forEach((col, x) => {
      const sh = col.stick * 18;
      g.fillStyle = '#3f4656';
      g.fillRect(x, 18 - sh, 1, sh);
      col.lanes.forEach((v, k) => {
        if (v > 0) {
          g.globalAlpha = 0.35 + 0.65 * Math.min(1, v * 3);
          g.fillStyle = LANES[k].color;
          g.fillRect(x, 21 + k * 5, 1, 4);
          g.globalAlpha = 1;
        }
      });
    });
  }, [columns, width]);

  const frameAt = (e) => {
    const r = wrap.current.getBoundingClientRect();
    return Math.round(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) * total);
  };
  const head = seekTarget ?? frame; // while seeking the playhead is already at the target
  const pct = (f) => `${(100 * f) / Math.max(1, total)}%`;
  const current = regionAt(regions, frame);
  const hovered = hover !== null ? regionAt(regions, hover) : null;

  return (
    <div className="timeline-wrap">
      {regions.length > 0 && (
        <div className="region-track">
          {regions.map((r) => (
            <button
              key={`${r.from}-${r.label}`}
              className={`region${r === current ? ' current' : ''}`}
              style={{ left: pct(r.from), width: pct(r.to - r.from), '--region': r.color }}
              title={`${r.label}: frames ${r.from}–${r.to}${r.note ? `\n${r.note}` : ''}`}
              onClick={() => onSeek(r.from)}
            >
              <span>{r.label}</span>
            </button>
          ))}
        </div>
      )}
      <div
        className="timeline"
        ref={wrap}
        onMouseMove={(e) => setHover(frameAt(e))}
        onMouseLeave={() => setHover(null)}
        onPointerDown={(e) => {
          // Click or drag: the playhead follows the pointer (a drag's seeks are
          // marked, so a player that must reload to go back does so only on release).
          e.currentTarget.setPointerCapture?.(e.pointerId);
          drag.current = true;
          onSeek(frameAt(e), true);
        }}
        onPointerMove={(e) => {
          if (drag.current) onSeek(frameAt(e), true);
        }}
        onPointerUp={(e) => {
          if (!drag.current) return;
          drag.current = false;
          onSeek(frameAt(e), false);
        }}
        onPointerCancel={() => {
          drag.current = false;
        }}
      >
        <canvas ref={strip} className="strip" style={{ width: '100%', height: 36 }} />
        <div className="played" style={{ width: pct(head) }} />
        {seekTarget !== null && <div className="target" style={{ left: pct(seekTarget) }} />}
        {milestones?.map((f) => (
          <div key={`m${f}`} className="marker star" style={{ left: pct(f) }} title={`Star: frame ${f}`} />
        ))}
        {(milestones ? events.filter((ev) => ev.kind !== 'star') : events).map((ev, i) => (
          <div
            key={i}
            className={`marker ${ev.kind}`}
            style={{ left: pct(ev.frame) }}
            title={`${ev.kind === 'star' ? ev.label : 'Area change'} — frame ${ev.frame}`}
          />
        ))}
        <div className="playhead" style={{ left: pct(head) }} />
        {hover !== null && (
          <div className="hover" style={{ left: pct(hover) }}>
            {hover} · {formatTime(hover)}
            {hovered && ` · ${hovered.label}`}
          </div>
        )}
      </div>
    </div>
  );
}
