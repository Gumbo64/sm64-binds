import { Suspense, lazy, useCallback, useEffect, useRef, useState } from 'react';
import { Player, SPEEDS } from './player.js';
import { formatTime } from './m64.js';
import { courseName } from './sm64.js';
import { regionAt } from './regions.js';
import Hud from './Hud.jsx';
import InputDisplay from './InputDisplay.jsx';
import Timeline from './Timeline.jsx';
import { RESOLUTIONS, loadResolution, saveResolution } from './resolution.js';

// Loaded when first opened (it brings the MP4 muxer).
const ExportDialog = lazy(() => import('./ExportDialog.jsx'));

const speedLabel = (s) => (s === Infinity ? 'max' : `${s}×`);

/**
 * Plays one replay. `initial` = { frame, speed, playing } to restore after a
 * rewind; `onRewind(frame, speed, playing)` is called to go backwards.
 *
 * Also used by other apps (the SM64Coin block page) through sm64-binds-frontend/replay:
 * `playerOptions` (Player's third argument: `headless`, `headlessThreshold`, `makeHeadlessWorker`),
 * `makePlayer(replay, onChange)` makes the Player (a subclass: another way to
 * play the frames), `topbarExtra(view)` / `panelExtra(view)` add to the top
 * bar and replace the regions list in the side panel, `regionLink(region)`
 * adds to the region chip, `milestones` (frames, sorted) enable the previous /
 * next star buttons (n / b, PageDown / PageUp).
 */
export default function Viewer({
  replay, initial, onRewind, onClose, backLabel = '← Replays',
  makePlayer, playerOptions, topbarExtra, panelExtra, regionLink, milestones,
}) {
  const container = useRef(null);
  const stage = useRef(null);
  const player = useRef(null);
  const [view, setView] = useState(null);
  const [error, setError] = useState(null);
  const [showHud, setShowHud] = useState(true);
  const [showPanel, setShowPanel] = useState(true);
  const [jump, setJump] = useState('');
  const [resolution, setResolution] = useState(loadResolution);
  const resumeAfterSeek = useRef(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const exportJob = useRef(initial?.exportJob ?? null);

  useEffect(() => {
    const p = makePlayer ? makePlayer(replay, setView) : new Player(replay.pads, setView, playerOptions);
    player.current = p;
    p.setSpeed(initial?.speed ?? 1);
    p.attach(container.current)
      .then(() => {
        if (initial?.frame) {
          p.seek(initial.frame);
          resumeAfterSeek.current = !!initial.playing;
        } else if (initial?.playing) {
          p.play();
        }
      })
      .catch((e) => setError(String(e?.message ?? e)));
    return () => p.dispose();
    // The emulator is created once per mounted replay.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Resume playback once a restore-seek finishes; reopen the export dialog
  // if the page was reloaded to export from an earlier frame.
  useEffect(() => {
    if (view && !view.seeking && resumeAfterSeek.current) {
      resumeAfterSeek.current = false;
      player.current.play();
    }
    if (view && !view.seeking && exportJob.current && !exportOpen) setExportOpen(true);
  }, [view, exportOpen]);

  const seek = useCallback(
    (target, dragging = false) => {
      const p = player.current;
      if (!p || !view) return;
      target = Math.max(0, Math.min(view.total, Math.round(target)));
      if (!p.seek(target) && !dragging) onRewind(target, p.speed, p.playing);
    },
    [view, onRewind],
  );

  // Previous / next star (any milestone: a block's end in the coin's cycle).
  const jumpStar = useCallback(
    (dir) => {
      if (!view || !milestones?.length) return;
      const f = view.frame;
      if (dir > 0) seek(milestones.find((m) => m > f) ?? view.total);
      else seek([...milestones].reverse().find((m) => m < f - 30) ?? 0);
    },
    [view, milestones, seek],
  );

  const changeSpeed = useCallback((dir) => {
    const p = player.current;
    const i = SPEEDS.indexOf(p.speed);
    const next = SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, i + dir))];
    p.setSpeed(next);
  }, []);

  const fullscreen = useCallback(() => {
    if (document.fullscreenElement) document.exitFullscreen();
    else stage.current?.requestFullscreen?.();
  }, []);

  // Real fullscreen: the stage covers the screen, the picture fills it, and
  // a control bar shows while the mouse moves.
  const [isFull, setIsFull] = useState(false);
  const [barVisible, setBarVisible] = useState(true);
  const idle = useRef(0);
  useEffect(() => {
    const onChange = () => {
      const on = document.fullscreenElement === stage.current;
      setIsFull(on);
      setBarVisible(true);
      player.current?.resolution?.setFullscreen(on);
    };
    document.addEventListener('fullscreenchange', onChange);
    return () => document.removeEventListener('fullscreenchange', onChange);
  }, []);
  const wake = useCallback(() => {
    setBarVisible(true);
    clearTimeout(idle.current);
    idle.current = setTimeout(() => setBarVisible(false), 2500);
  }, []);
  useEffect(() => () => clearTimeout(idle.current), []);

  useEffect(() => {
    const onKey = (e) => {
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
      if (exportOpen) {
        if (e.key === 'Escape' && !exporting) setExportOpen(false);
        return;
      }
      const p = player.current;
      if (!p || !view) return;
      switch (e.key) {
        case ' ':
        case 'k':
          e.preventDefault();
          p.toggle();
          break;
        case '.':
        case 'ArrowRight':
          e.preventDefault();
          if (e.shiftKey) seek(view.frame + 300);
          else p.step();
          break;
        case ',':
        case 'ArrowLeft':
          e.preventDefault();
          seek(view.frame - (e.shiftKey ? 300 : 1));
          break;
        case '+':
        case '=':
        case ']':
          changeSpeed(1);
          break;
        case '-':
        case '[':
          changeSpeed(-1);
          break;
        case 'f':
          fullscreen();
          break;
        case 'h':
          setShowHud((v) => !v);
          break;
        case 'p':
          setShowPanel((v) => !v);
          break;
        case 'n':
        case 'PageDown':
          e.preventDefault();
          jumpStar(1);
          break;
        case 'b':
        case 'PageUp':
          e.preventDefault();
          jumpStar(-1);
          break;
        case 'Home':
          seek(0);
          break;
        case 'End':
          seek(view.total);
          break;
        default:
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [view, seek, jumpStar, changeSpeed, fullscreen, exportOpen, exporting]);

  const p = player.current;
  const region = view ? regionAt(replay.regions, view.frame) : null;

  return (
    <div className="viewer sm64-replay">
      <header className="topbar">
        <button className="ghost" onClick={onClose} title={backLabel === '← Tree' ? 'Back to the search tree' : 'Load another replay'}>
          {backLabel}
        </button>
        <span className="title" title={replay.name}>
          {replay.name}
        </span>
        <span className="muted small">
          {replay.pads.length} frames · {formatTime(replay.pads.length)}
          {replay.header ? ' · Mupen64 header skipped' : ''}
        </span>
        {topbarExtra && topbarExtra(view)}
        <span className="spacer" />
        <label className="res-select small muted" htmlFor="resolution">
          Resolution
          <select
            id="resolution"
            value={resolution.id}
            onChange={(e) => {
              const res = RESOLUTIONS.find((r) => r.id === e.target.value);
              setResolution(res);
              saveResolution(res);
              player.current?.setResolution(res);
            }}
          >
            {RESOLUTIONS.map((r) => (
              <option key={r.id} value={r.id}>
                {r.label}
              </option>
            ))}
          </select>
        </label>
        <button className="ghost" onClick={() => setShowHud((v) => !v)} title="Toggle HUD (h)">
          HUD
        </button>
        <button className="ghost" onClick={() => setShowPanel((v) => !v)} title="Toggle side panel (p)">
          Panel
        </button>
        <button className="ghost" onClick={fullscreen} title="Fullscreen (f)">
          ⛶
        </button>
        <button className="ghost" onClick={() => setExportOpen(true)} disabled={!view} title="Save a frame range as an MP4 video">
          Export MP4
        </button>
      </header>

      <div className="body">
        <div
          className={`stage${isFull && !barVisible ? ' idle' : ''}`}
          ref={stage}
          onMouseMove={isFull ? wake : undefined}
          onClick={isFull ? wake : undefined}
        >
          <div className="canvas-box" ref={container} />
          {isFull && view && (
            <div className={`fs-bar${barVisible ? ' show' : ''}`} onMouseMove={wake}>
              <div className="row">
                <button className="primary" onClick={() => p?.toggle()} disabled={view.seeking}>
                  {view.playing ? '❚❚' : '▶'}
                </button>
                <span className="mono small">
                  {view.frame} / {view.total} · {formatTime(view.frame)}
                </span>
                <span className="spacer" />
                <button className="ghost" onClick={() => setShowHud((v) => !v)}>
                  HUD
                </button>
                <button className="ghost" onClick={fullscreen} title="Exit fullscreen (f / Esc)">
                  ✕
                </button>
              </div>
              <Timeline
                pads={replay.pads}
                frame={view.frame}
                seekTarget={view.seekTarget}
                events={view.events}
                regions={replay.regions}
                milestones={milestones}
                onSeek={seek}
              />
            </div>
          )}
          {showHud && view && <Hud frame={view.frame} total={view.total} state={view.state} />}
          {showHud && region && (
            <div className="region-chip" style={{ '--region': region.color }}>
              <b>{region.label}</b>
              <span className="mono muted">
                {region.from}–{region.to}
              </span>
              {region.note && <span className="muted">{region.note}</span>}
              {regionLink && regionLink(region)}
            </div>
          )}
          {view?.seeking && (
            <div className="banner">
              Fast-forwarding to frame {view.seekTarget}… {Math.floor((100 * view.frame) / Math.max(1, view.seekTarget))}%
            </div>
          )}
          {!view && !error && (
            <div className="banner">
              <div className="spinner" /> Starting the game…
            </div>
          )}
          {error && <div className="banner error">Couldn't start the game: {error}</div>}
        </div>

        {showPanel && (
          <aside className="panel">
            <h3>Input</h3>
            <InputDisplay pad={view?.pad} />
            {panelExtra ? panelExtra(view) : replay.regions.length > 0 && (
              <>
                <h3>Regions</h3>
                <ul className="events regions">
                  {replay.regions.map((r) => (
                    <li key={`${r.from}-${r.label}`} className={r === region ? 'current' : undefined}>
                      <button className="link" onClick={() => seek(r.from)} title={r.note}>
                        <span className="swatch" style={{ background: r.color }} />
                        <span className="mono">{r.from}</span> {r.label}
                      </button>
                    </li>
                  ))}
                </ul>
              </>
            )}
            <h3>Events</h3>
            {!view || view.events.length === 0 ? (
              <p className="muted small">Stars collected and course/area changes appear here as they're played.</p>
            ) : (
              <ul className="events">
                {view.events.map((ev, i) => (
                  <li key={i}>
                    <button className="link" onClick={() => seek(ev.frame)}>
                      <span className="mono">{ev.frame}</span>{' '}
                      {ev.kind === 'star' ? `★ ${ev.label}` : `${courseName(ev.course)} (area ${ev.area})`}
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <h3>Keys</h3>
            <dl className="keys small">
              <dt>Space / k</dt>
              <dd>play / pause</dd>
              <dt>→ / .</dt>
              <dd>next frame</dd>
              <dt>← / ,</dt>
              <dd>previous frame (reloads)</dd>
              <dt>Shift + ←/→</dt>
              <dd>±10 s</dd>
              <dt>+ / −</dt>
              <dd>speed</dd>
              {milestones?.length > 0 && (
                <>
                  <dt>n / b</dt>
                  <dd>next / previous star</dd>
                </>
              )}
              <dt>f · h · p</dt>
              <dd>fullscreen · HUD · panel</dd>
            </dl>
          </aside>
        )}
      </div>

      {exportOpen && view && (
        <Suspense fallback={null}>
          <ExportDialog
            replay={replay}
            player={player.current}
            frame={view.frame}
            job={exportJob.current}
            onRewind={(frame, job) => {
              // A player that can go back in place (save states) does; any
              // other reloads. The dialog reopens with `job` once it arrives.
              if (p.seek(frame)) {
                p.emit();
                exportJob.current = job;
                setExportOpen(false);
              } else onRewind(frame, 1, false, job);
            }}
            onBusy={setExporting}
            onClose={() => {
              exportJob.current = null;
              setExportOpen(false);
            }}
          />
        </Suspense>
      )}

      <footer className="controls">
        {view && (
          <Timeline
            pads={replay.pads}
            frame={view.frame}
            seekTarget={view.seekTarget}
            events={view.events}
            regions={replay.regions}
            milestones={milestones}
            onSeek={seek}
          />
        )}
        {/* The readout has its own line, with fixed-width digits: the controls below never shift. */}
        <div className="row readout">
          <span className="mono clock">
            {view ? `${view.frame} / ${view.total}` : '—'} · {view ? formatTime(view.frame) : ''}
          </span>
          <span className="spacer" />
          <span className="muted small">{view?.speed === 1 && view?.playing ? 'sound on' : 'sound only at 1×'}</span>
        </div>
        <div className="row">
          <button onClick={() => seek(0)} disabled={!view} title="Restart (Home)">
            ⏮
          </button>
          {milestones?.length > 0 && (
            <button onClick={() => jumpStar(-1)} disabled={!view} title="Previous star (b / PageUp)">
              ⏮★
            </button>
          )}
          <button onClick={() => seek((view?.frame ?? 1) - 1)} disabled={!view || view.frame === 0} title="Previous frame (←), reloads">
            ◀︎|
          </button>
          <button className="primary" onClick={() => p?.toggle()} disabled={!view || view.seeking} title="Play / pause (space)">
            {view?.playing ? '❚❚ Pause' : '▶ Play'}
          </button>
          <button onClick={() => p?.step()} disabled={!view || view.seeking} title="Next frame (→)">
            |▶︎
          </button>
          {milestones?.length > 0 && (
            <button onClick={() => jumpStar(1)} disabled={!view} title="Next star (n / PageDown)">
              ★⏭
            </button>
          )}
          <label className="row tight">
            Speed
            <select value={String(view?.speed ?? 1)} onChange={(e) => p?.setSpeed(Number(e.target.value))} disabled={!view}>
              {SPEEDS.map((s) => (
                <option key={s} value={String(s)}>
                  {speedLabel(s)}
                </option>
              ))}
            </select>
          </label>
          <span className="spacer" />
          <form
            className="row tight"
            onSubmit={(e) => {
              e.preventDefault();
              const n = Number(jump);
              if (Number.isFinite(n)) seek(n);
            }}
          >
            <input
              className="frame-input"
              type="number"
              min={0}
              max={replay.pads.length}
              placeholder="frame"
              value={jump}
              onChange={(e) => setJump(e.target.value)}
            />
            <button type="submit" disabled={!view}>
              Go
            </button>
          </form>
        </div>
      </footer>
    </div>
  );
}
