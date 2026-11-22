// The replay player UI: play, step, seek (both ways, through a Trail of engine
// save states), speeds, timeline with annotated regions, HUD, input display,
// resolution options and MP4 export, for any movie of controller pads.
//
//   import { Viewer, Player } from 'sm64-binds-frontend/replay';
//   import 'sm64-binds-frontend/replay.css';
//
// React is a peer dependency; `mp4-muxer` (a dependency) is only loaded when the
// export dialog is first opened. Loading the replay, its annotations and the
// ROM is the app's job; see the README.
export { default as Viewer } from './Viewer.jsx';
export { Player, SPEEDS } from './player.js';
export { Trail } from './trail.js';
export { parseReplay, hasMupenHeader, formatTime, FPS } from './m64.js';
export { normalizeRegions, regionAt } from './regions.js';
export { RESOLUTIONS, loadResolution, saveResolution } from './resolution.js';
export { BUTTONS, courseName } from './sm64.js';
