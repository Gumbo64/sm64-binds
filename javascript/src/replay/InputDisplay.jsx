import { BUTTONS } from './sm64.js';

/** The controller input of the frame just played: stick and buttons. */
export default function InputDisplay({ pad }) {
  const x = pad ? pad.stick_x : 0;
  const y = pad ? pad.stick_y : 0;
  const button = pad ? pad.button : 0;
  // Stick range is about ±80 after the deadzone; clamp the dot to the ring.
  const r = 30;
  const dx = Math.max(-1, Math.min(1, x / 80)) * r;
  const dy = -Math.max(-1, Math.min(1, y / 80)) * r;
  return (
    <div className="input-display">
      <svg width="76" height="76" viewBox="-38 -38 76 76" aria-label={`stick ${x}, ${y}`}>
        <circle r={r + 4} className="ring" />
        <line x1="0" y1="0" x2={dx} y2={dy} className="stick-line" />
        <circle cx={dx} cy={dy} r="6" className="stick-dot" />
      </svg>
      <div className="buttons">
        {BUTTONS.map((b) => (
          <span key={b.name} className={`btn${button & b.mask ? ' on' : ''}`} title={b.title ?? b.name}>
            {b.name}
          </span>
        ))}
      </div>
      <div className="mono small muted">
        x {String(x).padStart(4)} y {String(y).padStart(4)}
      </div>
    </div>
  );
}
