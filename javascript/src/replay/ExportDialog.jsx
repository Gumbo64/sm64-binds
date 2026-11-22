import { useEffect, useMemo, useRef, useState } from 'react';
import { FPS, formatTime } from './m64.js';
import {
  EXPORT_SIZES,
  ExportCancelled,
  exportMp4,
  mergeSegments,
  pickAudioCodec,
  videoCodecs,
} from './exporter.js';

const AUDIO_KBPS = 192;

function download(blob, name) {
  const url = URL.createObjectURL(blob);
  const a = Object.assign(document.createElement('a'), { href: url, download: name });
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
}

const mb = (bytes) => `${Math.round(bytes / 1e6).toLocaleString('en-US')} MB`;
const n = (v) => v.toLocaleString('en-US');

/**
 * Export parts of the replay as one MP4 with the annotations drawn in: any of
 * its regions plus custom frame ranges, back to back in frame order. `job`
 * (from a reload, see App's rewind) restores the settings;
 * `onRewind(frame, job)` reloads at an earlier frame; `onBusy(bool)` while
 * exporting.
 */
export default function ExportDialog({ replay, player, frame, job, onRewind, onBusy, onClose }) {
  const total = replay.pads.length;
  const regions = replay.regions;
  const [picked, setPicked] = useState(() => job?.picked ?? regions.map(() => true));
  const [custom, setCustom] = useState(() => job?.custom ?? (regions.length ? [] : [{ from: 0, to: total }]));
  const [sizeId, setSizeId] = useState(job?.sizeId ?? EXPORT_SIZES[0].id);
  const [codecChoice, setCodecChoice] = useState(job?.codecChoice ?? 'auto');
  const [overlays, setOverlays] = useState(job?.overlays ?? { chip: true, bar: true, hud: false, input: false });
  const [withAudio, setWithAudio] = useState(job?.withAudio ?? true);
  const [codecs, setCodecs] = useState(null);
  const [status, setStatus] = useState(null); // { phase, done, total, started }
  const [message, setMessage] = useState(job ? 'Reloaded at the first segment: ready to export.' : null);
  const [error, setError] = useState(null);
  const abort = useRef(null);
  const profile = useRef(null);
  const size = EXPORT_SIZES.find((s) => s.id === sizeId);
  const running = status !== null;

  useEffect(() => {
    let live = true;
    setCodecs(null);
    Promise.all([videoCodecs(size.w, size.h), pickAudioCodec(player.engine)]).then(([videos, audio]) => {
      if (live) setCodecs({ videos, audio });
    });
    return () => {
      live = false;
    };
  }, [size, player]);

  const segments = useMemo(
    () =>
      mergeSegments(
        [
          ...regions.filter((_, i) => picked[i]),
          ...custom.map((c) => ({ from: Number(c.from), to: Number(c.to) })),
        ],
        total,
      ),
    [regions, picked, custom, total],
  );
  const frames = segments.reduce((sum, seg) => sum + seg.to - seg.from, 0);
  const first = segments[0]?.from ?? 0;
  const audio = withAudio ? codecs?.audio : null;
  // Auto: the first this browser can encode (H.264, VP9, AV1).
  const video = codecs
    ? codecChoice === 'auto'
      ? (codecs.videos.find((v) => v.config) ?? null)
      : (codecs.videos.find((v) => v.muxer === codecChoice && v.config) ?? null)
    : null;
  // Only a bitrate gives a size estimate; constant quality depends on the picture.
  const estimate =
    video && video.quantizer === null
      ? ((frames / FPS) * (video.config.bitrate + (audio ? AUDIO_KBPS * 1000 : 0))) / 8
      : 0;
  const base = replay.name.replace(/\.[^.]*$/, '');
  const fileName =
    segments.length === 1
      ? `${base}-${segments[0].from}-${segments[0].to}.mp4`
      : `${base}-${segments.length}-segments.mp4`;

  const setCustomAt = (i, patch) => setCustom(custom.map((c, j) => (j === i ? { ...c, ...patch } : c)));

  const start = async () => {
    setError(null);
    setMessage(null);
    const settings = { picked, custom, sizeId, codecChoice, overlays, withAudio };
    if (first < frame) {
      // Earlier than the emulator is: reload there, then come back here.
      onRewind(first, settings);
      return;
    }
    let handle = null;
    if (window.showSaveFilePicker) {
      try {
        handle = await window.showSaveFilePicker({
          suggestedName: fileName,
          types: [{ description: 'MP4 video', accept: { 'video/mp4': ['.mp4'] } }],
        });
      } catch (e) {
        if (e.name === 'AbortError') return;
        setError(String(e.message ?? e));
        return;
      }
    }
    abort.current = new AbortController();
    const started = performance.now();
    setStatus({ phase: 'seek', done: 0, total: 1, started });
    onBusy(true);
    try {
      const blob = await exportMp4({
        player,
        regions,
        segments,
        size,
        overlays,
        video,
        audio,
        handle,
        signal: abort.current.signal,
        onProgress: (p) => {
          if (p.profile) profile.current = p.profile;
          setStatus({ ...p, started });
        },
      });
      if (blob) download(blob, fileName);
      const took = (performance.now() - started) / 1000;
      setMessage(
        `Saved ${handle?.name ?? fileName} (${formatTime(frames)} of video in ${Math.round(took)} s, ` +
          `${(frames / took).toFixed(1)} frames/s; time went to ${profile.current}).`,
      );
    } catch (e) {
      if (e instanceof ExportCancelled) setMessage('Export cancelled.');
      else setError(`Export failed: ${e.message ?? e}`);
    } finally {
      setStatus(null);
      onBusy(false);
    }
  };

  const pct = status ? Math.floor((100 * status.done) / Math.max(1, status.total)) : 0;
  const speed = status?.phase === 'encode' ? status.done / ((performance.now() - status.started) / 1000) : 0;
  const eta = speed > 0 ? (status.total - status.done) / speed : null;

  return (
    <div className="modal-backdrop" onClick={running ? undefined : onClose}>
      <div className="card export-dialog" role="dialog" aria-label="Export MP4" onClick={(e) => e.stopPropagation()}>
        <h2>Export MP4</h2>
        <p className="muted small">
          Plays the chosen parts frame by frame and encodes every frame, with the annotations drawn in, for uploading
          (for example to YouTube). Parts are joined in frame order; the gaps are skipped.
        </p>

        <fieldset disabled={running}>
          <div className="field top">
            <span>Segments</span>
            <div className="segments">
              {regions.length > 0 && (
                <>
                  <div className="row tight">
                    <button type="button" className="ghost small" onClick={() => setPicked(regions.map(() => true))}>
                      All regions
                    </button>
                    <button type="button" className="ghost small" onClick={() => setPicked(regions.map(() => false))}>
                      None
                    </button>
                  </div>
                  {regions.map((r, i) => (
                    <label key={`${r.from}-${r.label}`} className="segment">
                      <input
                        type="checkbox"
                        checked={!!picked[i]}
                        onChange={(e) => setPicked(picked.map((v, j) => (j === i ? e.target.checked : v)))}
                      />
                      <span className="swatch" style={{ background: r.color }} />
                      <span className="grow">{r.label}</span>
                      <span className="muted mono small">
                        {n(r.from)}–{n(r.to)} · {formatTime(r.to - r.from)}
                      </span>
                    </label>
                  ))}
                </>
              )}
              {custom.map((c, i) => (
                <div key={i} className="row tight segment">
                  <input
                    className="frame-input"
                    type="number"
                    min={0}
                    max={total}
                    value={c.from}
                    onChange={(e) => setCustomAt(i, { from: e.target.value })}
                    aria-label={`Range ${i + 1} from frame`}
                  />
                  <span className="muted">to</span>
                  <input
                    className="frame-input"
                    type="number"
                    min={0}
                    max={total}
                    value={c.to}
                    onChange={(e) => setCustomAt(i, { to: e.target.value })}
                    aria-label={`Range ${i + 1} to frame`}
                  />
                  <button
                    type="button"
                    className="ghost small"
                    onClick={() => setCustom(custom.filter((_, j) => j !== i))}
                    aria-label={`Remove range ${i + 1}`}
                  >
                    ✕
                  </button>
                </div>
              ))}
              <div className="row tight">
                <button
                  type="button"
                  className="ghost small"
                  onClick={() => setCustom([...custom, { from: 0, to: total }])}
                >
                  + Frame range
                </button>
                <button
                  type="button"
                  className="ghost small"
                  onClick={() => setCustom([...custom, { from: frame, to: Math.min(total, frame + 30 * FPS) }])}
                >
                  + Range from here
                </button>
              </div>
              <span className="small">
                {segments.length === 0
                  ? 'Nothing selected.'
                  : `${segments.length} ${segments.length === 1 ? 'segment' : 'segments'}: ${segments
                      .slice(0, 4)
                      .map((seg) => `${n(seg.from)}–${n(seg.to)}`)
                      .join(', ')}${segments.length > 4 ? ', …' : ''} · ${formatTime(frames)} of video`}
              </span>
            </div>
          </div>
          <label className="field">
            <span>Size</span>
            <select value={sizeId} onChange={(e) => setSizeId(e.target.value)}>
              {EXPORT_SIZES.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            <span>Codec</span>
            <select value={codecChoice} onChange={(e) => setCodecChoice(e.target.value)}>
              <option value="auto">Auto (H.264 if this browser has it, else VP9)</option>
              {(codecs?.videos ?? []).map((v) => (
                <option key={v.muxer} value={v.muxer} disabled={!v.config}>
                  {v.label}
                  {v.config ? '' : ' (not in this browser)'}
                </option>
              ))}
            </select>
          </label>
          <div className="field top">
            <span>Overlays</span>
            <div className="checks">
              <label>
                <input
                  type="checkbox"
                  checked={overlays.chip}
                  onChange={(e) => setOverlays({ ...overlays, chip: e.target.checked })}
                />{' '}
                Region name and note
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={overlays.bar}
                  onChange={(e) => setOverlays({ ...overlays, bar: e.target.checked })}
                />{' '}
                Region bar along the bottom
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={overlays.hud}
                  onChange={(e) => setOverlays({ ...overlays, hud: e.target.checked })}
                />{' '}
                HUD (stars, course, frame, time)
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={!!overlays.input}
                  onChange={(e) => setOverlays({ ...overlays, input: e.target.checked })}
                />{' '}
                Controller input (stick and buttons)
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={withAudio}
                  disabled={codecs && !codecs.audio}
                  onChange={(e) => setWithAudio(e.target.checked)}
                />{' '}
                Game sound
              </label>
            </div>
          </div>
        </fieldset>
        {regions.length === 0 && (overlays.chip || overlays.bar) && (
          <p className="muted small">This replay has no regions in index.json, so there are no annotations to draw.</p>
        )}

        <p className="muted small codec-line">
          {!codecs
            ? 'Checking which encoders this browser has…'
            : !video
              ? codecs.videos.some((v) => v.config)
                ? "This browser can't encode that codec at this size; pick another."
                : "This browser can't encode video (it has no WebCodecs encoders). Try a current Chrome or Edge."
              : `${video.label} at ${video.quantizer !== null ? 'constant quality' : `${Math.round(video.config.bitrate / 1e6)} Mbps`}${
                  audio ? ` + ${audio.label}` : ', no sound'
                }${estimate ? ` · about ${mb(estimate)}` : ''}${
                  window.showSaveFilePicker ? '' : ' · built in memory, then downloaded'
                }`}
        </p>

        {running && (
          <div className="export-progress">
            <div className="bar">
              <div style={{ width: `${pct}%` }} />
            </div>
            <span className="small mono">
              {status.phase === 'seek' && `Fast-forwarding to the next segment… ${pct}% (frame ${n(frame)})`}
              {status.phase === 'encode' &&
                `${n(status.done)} of ${n(status.total)} frames · ${pct}% · ${speed.toFixed(0)} frames/s${
                  eta !== null ? ` · ${formatTime(eta * FPS)} left` : ''
                }`}
              {status.phase === 'finish' && 'Finishing the file…'}
            </span>
          </div>
        )}
        {message && <p className="small">{message}</p>}
        {error && <p className="error">{error}</p>}

        <div className="row">
          <span className="spacer" />
          {running ? (
            <button onClick={() => abort.current?.abort()}>Cancel export</button>
          ) : (
            <>
              <button onClick={onClose}>Close</button>
              <button className="primary" disabled={segments.length === 0 || !video} onClick={start}>
                {segments.length > 0 && first < frame ? `Reload at frame ${n(first)}` : 'Export'}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
