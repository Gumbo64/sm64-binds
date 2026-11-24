# Portable save states (headless and rendering builds)

A game state saved by one of the two WASM builds can be loaded by the other, and
the game goes on identically:

| build | where | what runs it |
|---|---|---|
| headless, `sm64-port/build/us_web_headless` | Rust, Python, the nodes and bots | `SM64Game` |
| rendering, `sm64-port/build/us_web` | the browser | `Sm64VisualEngine` (JavaScript) |

```rust
let state = game.export_portable()?;          // headless game -> portable state
std::fs::write("state.p", state.to_bytes())?;
// ... in another game, of this build:
game.load_portable(&Portable::from_bytes(&bytes)?)?;
```

```js
const bytes = engine.export_portable_state();   // rendering build -> portable state
engine.load_portable_state(bytes);              // from either build
const own = engine.save_state();                // the build's own (raw) snapshot
engine.load_state(own);
```

Code: `rust/src/sm64/portable/` (the translation; `std` only), `rust/src/sm64/portable_game.rs`
(`SM64Game::export_portable` / `load_portable`), `rust/portable-wasm/` (the same translation
as a WASM module for the browser), `javascript/src/scripts/sm64/portable.js` and
`Sm64Game.js`. Generators: `maketools/gen_portable_tables.py` (tables from the two builds'
linker maps, WASMs and DWARF) and `maketools/portable_wasm.py`. Checks:
`rust/tests/portable.rs`, `rust/examples/portable_check.rs`, `javascript/e2e/`.

## 1. Why states do not just load

Both builds are linked from the same objects with the same struct layouts (wasm32), so a
state is "the same bytes" up to two things:

1. **Addresses.** The rendering build adds code and data, so every global moved
   (`gMarioStates` is 0xb0a010 headless and 0xb1e0e0 rendering) and function table indices
   differ. The map of each build (`-Wl,-Map`) lists the same input sections with the same
   sizes (checked: 19,000 sections, none differ) at different addresses.
2. **The heap.** Everything the game allocates (the level's graph nodes, surfaces, the main
   pool, the effects pool, ...) comes from `malloc`, and the two builds' heaps look
   nothing alike: the rendering build also allocates display lists, textures, SDL and GL
   state, and its dlmalloc hands out different blocks. Heap pointers cannot be shifted by a
   constant.

There are also real differences in what the builds hold: the headless build keeps its
display lists in an arena (`sDrawArena`), caches parts of its render walk (`sGeoCache`), clears
the render walk's lists after each frame and has one more field in `struct AllocOnlyPool`;
the rendering build keeps audio synthesis state and renderer caches.

## 2. The portable form

A `Portable` state is the game's **variables** and the **malloc blocks reachable from them**,
with every pointer made build-neutral:

- pointers to globals and constant data are headless addresses (translated between builds
  through the section map, `RUNS` in `tables.rs`);
- pointers to heap blocks are `0x40000000 +` the block's position in the state's list of
  blocks, plus the offset in the block;
- function pointers (WASM table indices) are headless table indices, translated through the
  functions both builds have by name (`FNS`: 1,190 functions).

Each variable and block is stored with its bytes and a list of the words that are pointers or
function pointers ("relocations"), so loading needs no type information: it allocates (or
reuses) blocks in the target, translates every relocation and writes the bytes.

### Format ("SM64PORT", version 2)

```text
  0   8  magic "SM64PORT"
  8   4  format version (little endian) = 2 (version 1, without the ghost count, is still read)
 12   4  source build: 0 headless, 1 rendering
 16  20  source build id: SHA-1 of the source WASM's code (the module without data sections)
 36   4  tables id: a hash of the variables, in order (a state is only loaded with the
        tables that wrote it)
 40   4  number of variables
 44   4  number of blocks (the ghosts included)
 48   4  how many of the last blocks are ghosts (version 2)
 52   -  the variables, then the blocks; each is
         u32 length, u32 number of relocations, the bytes,
         then the relocations, u32 each: (offset << 1) | 1 for a function pointer, 0 for a pointer
```

About 0.5 MB for a level (50 KB deflated). A headless `SaveState` is 16 MB; the rendering
build's own snapshot is about 13 MB.

### Which variables, which blocks

- **Variables** (`VARS`, about 750, 250 KB): every non-const variable of the game's own objects
  (`src/game`, `src/engine`, `src/menu`, `src/buffers/gSaveBuffer`), with its layout from the DWARF
  of the sources (which bytes are 1, 2, 4, 8-byte values, pointers, function pointers or
  union words), plus the moving textures and paintings the game edits in `levels/`. Excluded:
  build specific state (libc, dlmalloc, SDL, GL, `src/pc`), the display list pool and what
  points into it, matrices of the render walk that the next frame rebuilds (`gMatStackFixed`, the
  pointers into the display lists the walk made, is zeroed: the rendering build keeps them set at
  the end of a frame, the headless one does not, and they would add display list blocks to the
  rendering build's export; the `gMatStack` values themselves are state), audio (`src/audio`)
  and goddard (`src/goddard`). Audio and goddard are not transferred: the target keeps its own.
  `.data` that holds pointers is only included if it ever changes while playing a
  120 star movie (`maketools/portable_data_changed.txt`): the rest are tables the linker
  filled in.
- **Blocks:** found by walking the variables' typed pointers (`walk.rs`): a pointer to a known
  struct gives a typed region in the block it points into, whose pointers are followed in
  turn. Graph nodes are typed by their `type` field (and checked: nodes sit in circular lists,
  which stale pointers into reused memory do not satisfy), `ptr` arrays (a root's views) by
  looking for pointers. Words in a reached block that no type describes, but hold the
  address of a heap block, are pointers too (the links between an allocation pool's blocks).
  A block that is not reachable is not part of the state (and none is left: all 67 blocks in
  use are reached).

### Loading

`place` reuses the target's own game blocks, best fit first: a block that the game frees
later is freed whole, so a loaded state needs no allocator call for blocks that fit; the
rest come from the target's `malloc`. The target's variables are overwritten, every
relocation is translated, then:

- headless target: the render walk cache is invalidated (`sGeoCache`), the display list pool's
  arena flag is set;
- rendering target: the blocks of its display list pool, whose list the load drops, are freed,
  and the renderer's texture cache (GL textures by data address) is reset.

## 3. What is not in a state, and the limits

- **The intro, while the Mario head runs** (the title screen's goddard): goddard keeps an
  object graph in malloc blocks that no type describes, so `export_portable` fails
  ("goddard ... is running"). The file select menu, the castle and every level are fine.
  Loading into an engine that is in the intro works (that is what a fresh engine is in).
- **Audio.** Sequence players and the audio heap are not transferred, so the music after a
  load restarts according to the target's state; the game does not read it back.
- **Display lists, textures, matrices of the render walk:** rebuilt by the next frame.
- **Typed pointers into reused memory** are checked before a struct layout is applied (`plausible`
  in `walk.rs`): a `SpawnInfo` must have one of the game's behaviors (not just any address in
  constant data: a terrain copy's s16 pairs read as such an address and the exporter translated
  them, changing two vertices of a level's collision data, which showed 146 more surface nodes
  after the level was loaded in a game that loaded the rendering build's export), an `Object` is
  never in the heap (`gObjectPool` holds them all), a `Surface`'s `object` is null or a global,
  and `AllocOnlyPoolBlock::pad`, which malloc leaves unwritten, is not a pointer.
- **Stale pointers** (an object slot's `respawnInfo` after the object died) into memory that
  was reused for a block we reach are translated like any other. Typed pointers into freed heap
  memory are ghosts (next section).
- A state is only valid for the builds the tables were made for (`tables.rs` records both
  builds' ids; regenerate with `maketools/gen_portable_tables.py` and
  `maketools/portable_wasm.py` after a rebuild). `Sm64VisualEngine` checks the build id
  of the WASM it runs.
- The tables read the rendering build's map: it is written by `make sm64-build`
  (`-Wl,-Map`). The headless WASM is not changed by any of this.

### Ghosts: stale pointers into freed memory

A pointer can outlive what it points to: an object's floor (`sObjFloor`, a `Surface *`) after
the surface pool was freed. A raw replay keeps reading the old bytes (freed memory is untouched
until `malloc` reuses it), and the game may dereference such a pointer at any time. The freed
memory is not state, so a loaded game would have other bytes there.

When the export finds a typed pointer (to a struct we know, 4-aligned) or an object's pointer
slot whose value is inside a *free* dlmalloc chunk (not in any block in use, so not reached),
it records a **ghost**: a raw copy of the bytes at the target (the pointee's size, at least 0x40
bytes, 8-aligned, clipped to the free chunk; overlapping windows are merged), with no type
walking, since stale data must not be interpreted. Ghosts are appended to the block list (the
header says how many of the last blocks are ghosts) and the pointers are translated into them
like pointers into any block. A load allocates every ghost with the target's `malloc`, never
reuses a game block for one and **never frees it**: a deliberate leak of a few hundred bytes
(`PortableLoadReport::ghost_bytes`, `ghostBytes` in JavaScript). The rendering target restores
its boot snapshot before each load, so the leak does not add up there. `gDisplayListHeap` is
not ghosted (display lists are rebuilt).

Limits: a ghost is a snapshot. The original game may later reuse the freed memory for a new
allocation, which the stale pointer then sees, and a ghost does not mimic that. A stale pointer
into a block that is in use at the export (memory reused by something else) is not a ghost: it
reads the loaded copy of that block, and the game's later frees and allocations place memory
differently in the two heaps. Measured on a 107,931 frame chain (`portable_check ghost`): an
export has 0 to 6 stale pointers (`sObjFloor`, floors in `gObjectPool`), 0 to 384 ghost bytes,
and the pointees read identically for 600 frames after the load at every checkpoint.

## 4. The rendering build's own snapshots

`save_state()` / `load_state()` copy the stack and everything after the read-only data (13 MB)
like the headless build's `SaveState`. They also contain the renderer's and SDL's memory, so
they are meant for going back within one session (seeking, undo), not for storing. What the
page keeps outside the WASM's memory is not in them: GL objects (textures are cached by
data address; loading resets the cache's lookup table, nodes keep their GL texture ids and
are reused), the audio queue's timing. Use portable states to store or send.

## 5. Validation

Two checks, both with the TASVideos 120 star movie (`tas/tasvideos-7310M-120star.m64`, 128,808
frames), which plays identically in both builds from power on.

**Same build** (`rust/tests/portable.rs`, `cargo test --test portable`): a state exported at
frames 600, 1500, 12000 and 30000 and loaded into a game that is somewhere else (frame 50)
goes on with the same `GameState` for 300 frames, and the same value of every variable of
the state, checked every 10 frames.

**Cross build** (`rust/examples/portable_check.rs`, `javascript/e2e/portable_check.mjs`): the
headless build plays the movie to checkpoints, exports a portable state at each and records
the next K = 600 frames (the `GameState` and a hash of every variable's values per frame). The
rendering build, in headless Chromium (swiftshader), does the same from its own run. Then each
build loads the other's states into a fresh game and records the K frames that follow.
`portable_check compare` reports the first frame where a loaded run differs from the run of
the build it is loaded into. Variables that differ between the builds' own runs (state of the
render walk, which the headless build does not draw; they are listed with the table) are left
out. Commands:

```sh
cd rust && cargo build --release --example portable_check
DIR=/tmp/pc; mkdir -p $DIR
CP="100 250 1000 1900 2600 3700 5100 6300 7000 9500 12000 14000 17000 22000 30000 35000 39000 41000 50000 52000 54000 70000 90000 110000 124000 128000"
./target/release/examples/portable_check record ../baserom.us.z64 ../tas/tasvideos-7310M-120star.m64 $DIR 600 $CP
cd ../javascript/e2e && npm install && env -u DISPLAY node portable_check.mjs record ../../baserom.us.z64 ../../tas/tasvideos-7310M-120star.m64 $DIR 600 $CP
cd ../../rust && ./target/release/examples/portable_check load ../baserom.us.z64 ../tas/tasvideos-7310M-120star.m64 $DIR 600
cd ../javascript/e2e && env -u DISPLAY node portable_check.mjs load ../../baserom.us.z64 ../../tas/tasvideos-7310M-120star.m64 $DIR 600
cd ../../rust && ./target/release/examples/portable_check compare $DIR 600
```

### Results

K = 600 frames after each checkpoint. "identical" means every compared variable and the
`GameState` match on every one of those frames. Checkpoints 39000 (the intro's Mario head is
on screen; refused by design) and 50000 and later were not run: the run was stopped early.

| checkpoint | headless -> rendering | rendering -> headless |
|---|---|---|
| 250 | identical | identical |
| 1000 | not run | identical |
| 1900 | not run | identical |
| 2600 | identical | identical |
| 3700, 5100, 6300 | not run | identical |
| 7000 | not run | differs at +210: `mario_misc:gBodyStates` only |
| 9500 | not run | identical |
| 12000 | identical | identical |
| 14000, 17000 | not run | identical |
| 22000 | not run | differs at +8: `mario_misc:gBodyStates` only |
| 30000 | identical | identical |
| 35000, 41000 | not run | identical |

Both builds also agree from boot at every checkpoint above. Known remaining difference:
`gBodyStates` (Mario's body-state record, written while the render walk runs) diverges at 7000
and 22000 when the state comes from the rendering build; `GameState` stays identical in
those runs. The rendering build's `save_state`/`load_state`/delta were not exercised in
Chromium by this run (modes `snapshot` and `rewind` of `portable_check.mjs` exist).
