import { courseName } from './sm64.js';
import { formatTime } from './m64.js';

const f1 = (v) => v.toFixed(1);

/** Overlay with the game state after the current frame. */
export default function Hud({ frame, total, state }) {
  if (!state) return null;
  const [vx, vy, vz] = state.vel;
  return (
    <div className="hud">
      <div className="hud-row">
        <span className="big">★ {state.numStars}</span>
        <span>{courseName(state.courseNum)}</span>
        <span className="muted">area {state.areaIndex}</span>
      </div>
      <div className="mono">
        frame {frame} / {total} · {formatTime(frame)}
      </div>
      <div className="mono">
        pos {f1(state.pos[0])}, {f1(state.pos[1])}, {f1(state.pos[2])}
      </div>
      <div className="mono">
        h-speed {f1(Math.hypot(vx, vz))} · v-speed {f1(vy)}
      </div>
    </div>
  );
}
