# sm64-binds

Interfaces for Mario 64 in Rust, Python and JavaScript, all running the same
WebAssembly build of [sm64-port](https://github.com/sm64-port/sm64-port) through
a WASM runtime.

To avoid shipping copyrighted ROM data, the WASM module is distributed XOR'd
against a US `.z64` ROM. Each binding recombines it with the ROM you supply at
runtime. The ROM is required: a US `z64` copy, exactly 8.00 MB.

| Binding | Package | Notes |
| --- | --- | --- |
| Rust | [crates.io/crates/sm64-binds](https://crates.io/crates/sm64-binds) | Most actively maintained. |
| Python | [pypi.org/project/sm64-binds](https://pypi.org/project/sm64-binds) | |
| JavaScript | [npmjs.com/package/sm64-binds-frontend](https://www.npmjs.com/package/sm64-binds-frontend) | Browser frontend, see `js_example/`. |

## Layout

| Path | Contents |
| --- | --- |
| `rust/` | The Rust crate. `src/lib.rs` is the API, `tests/` runs the game. |
| `python/` | The Python package. |
| `javascript/` | The npm package, `sm64-binds-frontend`. |
| `js_example/` | A Vite + React app consuming the npm package. |
| `maketools/` | Scripts that build and package the distributable WASM. |
| `scripts/` | Release helpers: version reporting and artifact checks. |
| `rust/benchmark/` | Timing programs; see `rust/benchmark/README.md`. |
| `WASM_ZIP_XOR/` | The distributable XOR'd payloads. |
| `sm64-port/` | Vendored sm64-port checkout, only needed to rebuild the WASM. |

## Usage

Rust:

```rust
use sm64_binds::{GamePad, SM64GameGenerator, buttons};

let generator = SM64GameGenerator::new(std::fs::read("baserom.us.z64")?)?;
let mut game = generator.create_game()?;

let mut pad = GamePad::default();
pad.button = buttons::A_BUTTON;
game.step_game(pad)?;

let state = game.get_game_state()?;
println!("{state}");           // numStars, position, levelNum, lives, health, ...
println!("stars: {}", state.num_stars);
```

Python:

```python
from sm64_binds import GamePad, SM64GameGenerator, buttons

generator = SM64GameGenerator.from_file("baserom.us.z64")
game = generator.create_game()

pad = GamePad(buttons.A_BUTTON, 0, 0)
game.step_game(pad)

state = game.get_game_state()
print(state)
```

Clone a game to branch off from the same state:

```python
clone = generator.copy(game)
```

## Building the WASM

With a ROM at the repo root:

```
make sm64-build
```

If emscripten (`emcc`) is on `PATH` this rebuilds `sm64-port` and regenerates
`WASM_ZIP_XOR/`. Otherwise it recombines the existing payloads with your ROM,
which is enough to run every binding.

The devcontainer (`.devcontainer/`) provisions emscripten, wasm-pack,
wasm-bindgen and binaryen.

## Tests

```
make test
```

Runs the Rust, Python, JavaScript and maketools suites. Each one is also
runnable on its own:

```
make test-rust          # cargo test, in rust/
make test-python        # pytest, in python/
make test-javascript    # node --test, in javascript/
make test-maketools     # pytest, in maketools/
make lint               # clippy, rustfmt and eslint
```

Tests that boot the game need a ROM. Point `SM64_BINDS_ROM` at it if it is not
at the repo root; otherwise those tests skip instead of failing, so the suite
passes on a checkout without the copyrighted file.

## Releasing

Rust, Python and JavaScript all ship from this repo, one tag at a time:

```
make versions                 # see where each package stands
python scripts/bump_versions.py 1.0.25
```

Then commit, tag `v1.0.25`, and push the tag; GitHub Actions builds and
publishes all three. See `RELEASING.md` for the one-time token setup and the
release checklist.

## Benchmarks

```
make bench           # Rust: stepping, save states, instantiation
make bench-python    # Python: the same paths

# Or individually, from rust/:
cargo bench --bench hot_paths -- ../baserom.us.z64
cargo bench --bench diagnostics -- ../baserom.us.z64
```

Both language harnesses share one measurement contract (`rust/benchmark/harness.rs`,
mirrored by `python/benchs/harness.py`): load and instantiation costs, stepping
through a scripted input, per-call overhead, and the save-state copy paths,
each with per-unit times, throughput and derived rows for per-frame figures.
Save-state copying is a single `memcpy` instead of the runtime's chunked
accessors, and `step_game` is unchanged because it is bounded by the emulated
game logic rather than by the binding — an empty WASM call through the same API
costs ~30 ns against a ~200 µs frame.