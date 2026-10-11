# sm64-binds-frontend

Mario 64 binds with graphics for the browser. [sm64-binds](https://github.com/Gumbo64/sm64-binds)

To avoid shipping copyrighted ROM data, the WASM module is distributed XOR'd
against a US `.z64` ROM and recombined in the browser at runtime. The ROM is
required: a US `z64` copy, exactly 8.00 MB.

## Install

```
npm install sm64-binds-frontend
```

## Usage

Drop in the `<FileUpload>` component, let the user pick their ROM, then drive
the game:

```jsx
import { useRef } from "react";
import { FileUpload, sm64_record, GameConfig, GamepadButtons } from "sm64-binds-frontend";

function Game({ hasRom }) {
    const canvas = useRef(null);

    if (!hasRom) {
        return <FileUpload setHasRom={...} />;
    }

    return <canvas ref={canvas} className="emscripten" />;
}
```

The WASM module draws to a `<canvas>`; it needs the `emscripten` class for
correct sizing.

### Recording and replaying

`sm64_record` plays the game and records every frame's controller input, so a
run can be replayed or solved later:

```js
import { sm64_record, sm64_evaluate, GameConfig, RngConfig } from "sm64-binds-frontend";

const game_config = GameConfig.defaultConfig();
const solution = await sm64_record(canvas, 22, game_config);
const success = await sm64_evaluate(canvas, solution, 22);
```

### Randomised input

Pass an `RngConfig` to perturb the controller, so a replay has to match the
seed exactly:

```js
import { RngConfig } from "sm64-binds-frontend";

const rng = new RngConfig(100, 5, 2, 0.5, 0.5, 0.2);
const game_config = new GameConfig(NaN, rng);
```

Note that the WASM RNG is active from boot with the defaults above, even
before you construct a config.

### Controller input

`rng_pad` returns what the game actually received after RNG. `GamepadButtons`
has the N64 masks (`A_BUTTON`, `START_BUTTON`, `U_JPAD`, and so on).

```js
const pad = engine.rng_pad(controllerPad);
if (pad.is_pressed(GamepadButtons.U_JPAD)) {
    // ...
}
```

## API

| Export | Purpose |
| --- | --- |
| `FileUpload` | React component that validates and stores a ROM in IndexedDB. |
| `sm64_record` | Plays and records inputs. |
| `sm64_evaluate` | Replays a recorded solution. |
| `sm64_playback` | Replays with manual control. |
| `GameConfig`, `RngConfig` | Playback and RNG tuning. |
| `GamePad`, `GameState`, `GamepadButtons` | Controller input and game state. |

## Example

See `js_example/` in the [repository](https://github.com/Gumbo64/sm64-binds)
for a complete Vite + React app.

## Licence

ISC. The game itself is © Nintendo; this package contains no ROM or game data.