# Portable save states: a reader's guide

This is the guide to the code that moves a game between the two builds of the
game: the **headless** WASM (`sm64-port/build/us_web_headless`, what Rust,
Python, the nodes and the bots run) and the **rendering** WASM
(`sm64-port/build/us_web`, what the browser runs). It says why translation is
needed, how one translation flows through the code, in what order to read the
files, every rule the translator applies and the bug that made it necessary,
the save format, and how to change and test it. Design notes and measurements
are in [`portable-state.md`](portable-state.md).

## 1. Why a state has to be translated

Both builds are compiled from the same game sources with the same struct
layouts (wasm32), so a game's state is "the same bytes" in both, except:

1. **Addresses.** The rendering build links more code and data (the renderer,
   SDL, GL), so every global sits somewhere else (`gMarioStates` is 0xb0a010
   headless, 0xb1e0e0 rendering), and function pointers, which in WASM are
   indices into a function table, have other indices.
2. **The heap.** Everything the game allocates (graph nodes, surfaces, the
   main pool, the effects pool, ...) comes from `malloc`, and the rendering
   build also allocates display lists, textures and GL state, so the two heaps
   look nothing alike. A heap pointer cannot be fixed by adding a constant.

So a raw snapshot of one build's memory, loaded into the other, is full of
pointers to the wrong places. The translator writes the state in a form with
no build-specific addresses: the game's variables, the malloc blocks they
reach, and a list of which words are pointers ("relocations"), each written as
a build-neutral value. A load allocates the blocks in the target, translates
each relocation to the target's addresses, and writes the bytes.

The hard part is the middle step: knowing which words are pointers. C does
not say (unions, `uintptr_t`, an object's untyped `rawData` slots, stale
pointers into memory that has since been reused), and guessing wrong in either
direction changes the game after a load. Most of the code, and every bug in
section 4, is about that.

## 2. One translation, end to end

```text
 EXPORT (either build)                                    files
 ─────────────────────                                    ─────
 memory image of build B (linear memory up to heap top)
   │
   ├─ unsupported?  goddard (intro head) running → refuse   excluded.rs
   ├─ walk dlmalloc's chunks: which blocks are in use       heap.rs, image.rs
   │
   ├─ WALK: one typed region per variable (DWARF layout);   walk.rs
   │    for each word a layout says may be a pointer:
   │      is it one?            ───────────────────────►    rules.rs
   │      points into a block in use → block is reached
   │      struct known & plausibly there → new region  ──►  plausible.rs
   │    then: first word of every reached block = pool link?
   │
   ├─ RELOCATE: copy variables and reached blocks;          relocate.rs
   │    stale pointers into freed memory → ghost blocks ─►  ghosts.rs
   │    each pointer word → portable value + relocation     addresses.rs
   │    zero what is not state (render lists, matrices)  ►  excluded.rs
   │
   └─ Portable { vars, chunks, chunk_bases, ghosts, … }     state.rs, export.rs
        │
        └─ to_bytes  → "SM64PORT" v2 bytes                  format.rs

 LOAD (either build)
 ───────────────────
 bytes ─ from_bytes ─► Portable                             format.rs
   ├─ place: reuse the target's own game blocks (best fit)  load.rs
   │         caller mallocs the rest, and every ghost
   ├─ patches: bytes + every relocation translated to T     load.rs
   └─ caller writes the patches, then resets T's caches:
        headless  → render walk cache (sGeoCache)           portable_game.rs
        rendering → boot snapshot first, keep renderer .bss,  Sm64Game.js,
                    free old display-list blocks,            portable.js
                    reset texture cache                      (via portable-wasm)
```

The portable address space (`addresses.rs`):

| what | portable value |
|---|---|
| a global or constant | the headless build's address (rendering addresses translated through `RUNS`) |
| a malloc block | `0x40000000 + chunk_bases[i] + offset`, block `i` of the state, each block rounded up to 16 bytes |
| a function pointer | the headless build's function table index (translated through `FNS`, the functions both builds have, by name) |
| no translation | null, counted as `unresolved` / `untranslated` |

The browser runs the same Rust code: `rust/portable-wasm` compiles
`rust/src/sm64/portable/` to a small WASM module, shipped base64-encoded as
`javascript/src/scripts/sm64/pkg/portable_wasm.js` and driven by
`portable.js`. Its exports equal the native ones byte for byte.

## 3. Reading order

Bottom up; every Rust file starts with what it is, what it guarantees, and
what to read next. Sizes are lines.

**Generated (do not edit):**

| file | lines | what |
|---|---|---|
| `rust/src/sm64/portable/tables.rs` | 11,364 | both builds' layouts: `VARS` (the state's variables, both addresses, layout), `LAYOUTS`, `NAMED` structs, `RUNS`, `FNS`, `NODE_TYPES`, object pointer slots, constants (`GM`, `DATA_STARTS`, `RENDERER`, ...) |
| `javascript/src/scripts/sm64/pkg/portable_wasm.js` | 4 (428 KB) | the translator WASM, base64 |

**The translator** (`rust/src/sm64/portable/`, `std` only):

| layer | file | lines | what |
|---|---|---|---|
| 1 vocabulary | `layout.rs` | 32 | `K` (kind of value) and `Item`, the types `tables.rs` is written in |
| 2 lookups | `types.rs` | 83 | layouts by struct name, pointee structs, `struct Object` offsets |
| | `builds.rs` | 48 | `Build` (headless / visual), a variable's address per build |
| | `addresses.rs` | 92 | portable addresses: globals via `RUNS`, functions via `FNS`, `HEAP_BASE` |
| | `heap.rs` | 68 | dlmalloc's chunks in an image |
| | `image.rs` | 82 | `Image::classify`: null / in a block / global / unresolved |
| | `regions.rs` | 44 | a typed region (layout at an address, in a variable or a block) |
| 3 rules | `rules.rs` | 121 | which words of a region are pointers |
| | `excluded.rs` | 89 | what is not state; states that cannot be translated |
| | `plausible.rs` | 120 | is the struct a pointer names really there (table of rules and bugs) |
| 4 walk | `walk.rs` | 280 | which blocks are reached |
| 5 relocation | `relocate.rs` | 207 | copy, rewrite pointers, record relocations |
| | `ghosts.rs` | 130 | copies of freed memory that stale pointers read |
| | `state.rs` | 73 | `Buf`, `Walked`, `Portable`, `Patch` |
| | `diagnostics.rs` | 43 | descriptions of unresolved pointers |
| | `export.rs` | 58 | `walk`, `export`, `game_blocks` |
| 6 format, load | `format.rs` | 163 | `to_bytes` / `from_bytes` |
| | `load.rs` | 187 | `place`, `patches`, `untranslatable` |
| | `compare.rs` | 90 | `digest`, `diff` (for the checks) |
| API | `mod.rs` | 119 | module map, re-exports, per-build constants |

There are no cycles between these files (each `use super::x` points down the
table, except `mod.rs`, which only declares and re-exports).

**Callers:**

| file | lines | what |
|---|---|---|
| `rust/src/sm64/portable_game.rs` | 107 | `SM64Game::export_portable` / `load_portable` (headless, Rust) |
| `rust/portable-wasm/src/lib.rs` | 250 | the C ABI the browser calls (`p_export`, `p_load_begin`, `p_load_finish`, ...) |
| `javascript/src/scripts/sm64/portable.js` | 206 | `PortableTranslator`: copies memory in, runs the WASM, reads results |
| `javascript/src/scripts/sm64/Sm64Game.js` | 269 | `Sm64VisualEngine.export_portable_state` / `load_portable_state`, raw `save_state` / `load_state` |

**Generators:** `maketools/gen_portable_tables.py` (658, writes `tables.rs`),
`maketools/portable_wasm.py` (44, writes `portable_wasm.js`),
`maketools/portable_data_changed.txt` (449, input: which `.data` variables
change while playing).

**Checks and debug tools** (none is on the export or load path):

| file | lines | what |
|---|---|---|
| `rust/tests/portable.rs` | 223 | same-build round trips (`cargo test --test portable`) |
| `rust/examples/portable_golden.rs` | 321 | fingerprint of every translator output on a corpus: before/after a change |
| `rust/examples/portable_fuzz.rs` | 764 | differential fuzz: played game vs. game loaded from its export |
| `rust/examples/portable_check.rs` | 550 | cross-build record / load / compare with a TAS movie |
| `javascript/e2e/portable_check.mjs` | 519 | the rendering build's half, in headless Chromium (`quick`: render check) |
| `rust/examples/portable_diff.rs` | 247 | pointer-aware diff of two `.p` files |
| `rust/examples/portable_unreached.rs` | 107 | blocks in use that a state does not reach |
| `rust/examples/portable_unresolved.rs` | 40 | unresolved pointers of an export |
| `rust/examples/portable_obj.rs` | 39 | one object slot of a state, pointers marked |
| `rust/examples/portable_probe.rs` | 35 | what an export makes of one word |
| `rust/examples/portable_slots.rs` | 28 | pointers in object slots no field declares |
| `rust/examples/portable_raw2p.rs` | 26 | raw rendering snapshot → `.p` with the native translator |

## 4. The rules, and the bug behind each

### Which words are pointers (`rules.rs`, `relocate.rs`)

| rule | why (the bug it prevents) |
|---|---|
| A word is only a pointer if a layout (DWARF) says it may be one (`K::Ptr`, `K::Word`), never because its value looks like an address | the general one: s16 data looks like an address once in a few thousand words |
| An object's `rawData` slot is a pointer only if its behavior uses that slot as one (`BEHAVIOR_POINTER_SLOTS`, plus the common fields); unknown behaviors use every slot any field declares a pointer | numbers in the address range were relocated: `oInteractType = INTERACT_TEXT` (0x800000, signs), a Boo's oscillation timer, spinning objects' yaw, `oBehParams`; objects diverged a frame after a load |
| A union word (`K::Word`) is a pointer only into a block in use or at writable data; into constant data only in `uintptr_t` arrays (segment table, script stacks), declared object pointer slots and `bhvStack` | numbers translated as constant-data addresses |
| `AllocOnlyPoolBlock::pad` is never a pointer | malloc leaves it unwritten; leftovers looked like pointers |
| The first word of a reached block that is exactly another block's address, in bytes no region covers, is a pointer (an allocation pool's link) | pool blocks reached only by value were missing from the state |
| An allocation pool inside a main pool block (release handler `alloc_only_pool_release_handler`) is typed right after the block header | its blocks were not reached |
| `ptr` (pointer to pointers, `GraphNode **views`) is typed as up to 64 words that are pointers | a root's views were not walked |

### What is not state (`excluded.rs`)

| rule | bug it prevents |
|---|---|
| Render-list pointers (`DisplayListNode *`) are zeroed | they point into the frame's display lists, which the rendering build leaves set and the headless one clears |
| `gMatStackFixed` is zeroed and not walked | the same, for the render walk's matrices: it added display-list blocks to the rendering build's exports only, so the builds' exports differed |
| The display list pool (`gGfxAllocOnlyPool`) is not followed and its header is zeroed; a rendering load frees its old blocks, a headless load marks the pool as the draw arena's | the pool's blocks are per-frame memory, in different places in the two builds |
| The headless build's extra `AllocOnlyPool` field (+12) is zeroed | the builds' exports differed in it |
| A state taken while goddard (the intro's Mario head) runs is refused | goddard's object graph has no types |
| Rendering load: the renderer's `.bss` (`tables::RENDERER`, the GL state it mirrors, shader programs, window size) is kept, not loaded | **black screen** after every headless → rendering load: restored renderer state made it skip GL calls it thought were done |
| Rendering load: start from the boot snapshot, then reset the texture cache | a heap holding a level's data ends up corrupted (abort, or black textures); cached textures by address show stale data |
| Headless load: invalidate `sGeoCache` | the render walk cache holds nodes of the old state |

### Plausibility: is the struct really there (`plausible.rs`)

A pointer can outlive what it points to, and malloc reuses the memory. Typing
reused memory with the pointer's struct turns its numbers into "pointers".
Before a struct layout is applied behind a heap pointer:

| struct | rule | bug it prevents |
|---|---|---|
| `GraphNode` | a known `type`, and `prev`/`next` link back to it | stale node pointers into reused memory |
| `SpawnInfo` | `behaviorScript` is one of the game's behaviors; the next two pointers are null or point somewhere | a terrain copy's s16 pair (y, 0x17) read as 0x0017yyyy, an address in the rendering build's constant data: two collision vertices were rewritten, **+146 surface nodes**, then a **crash** in `geo_process_node_and_siblings` |
| `AllocOnlyPool` | its last block is a malloc block (or, headless, a global) at least `size` long, and it used no more than `size` | the stale `sMemPoolForGoddard` (reused for Mario's graph nodes) was typed as a pool and the +12 fix-up zeroed a switch node's `parent`: **Mario drawn twice** |
| `MemoryPool` | its first word is a plausible `AllocOnlyPool` | a stale `gEffectsMemoryPool` typed Mario's animated-part node, a padding word became a pointer: **invisible Mario** |
| `Object` | never in the heap (`gObjectPool` is a global array) | stale platform/parent pointers typed graph nodes and collision data as objects (part of the crash above) |
| `Surface` | its `object` is null or a global | a stale floor into a later level's collision data |

A failed check only means "do not type this memory"; the pointer is still
translated and its block is still part of the state.

### Stale pointers into freed memory: ghosts (`ghosts.rs`, format v2)

A typed pointer (to a known struct, 4-aligned) or an object pointer slot whose
value is inside a *free* dlmalloc chunk (an object's floor after the surface
pool was freed) keeps reading the old bytes in the original game. The export
copies a window there (the struct's size, at least 0x40 bytes, 8-aligned,
clipped to the free chunk, overlaps merged) as a **ghost** block, appended
after the reached blocks, and translates the pointer into it. A ghost is never
interpreted (no layout, no relocations); a load always allocates ghosts and
never frees them (a leak of 0 to a few hundred bytes, `ghost_bytes`).
`gDisplayListHeap` is never ghosted. Limit: a ghost is a snapshot; if the
original later reuses the memory, the stale pointer sees new data there and
the ghost does not.

## 5. The format ("SM64PORT")

```text
offset size
   0    8  magic "SM64PORT"
   8    4  format version, little endian: 2 (1 is still read)
  12    4  source build: 0 headless, 1 rendering
  16   20  source build id (SHA-1 of the source WASM without its data sections)
  36    4  tables id (FNV-1a of the variables' names and sizes, in order, ^ number of pointee names)
  40    4  number of variables (must equal the tables')
  44    4  number of blocks, ghosts included
  48    4  version 2 only: how many of the last blocks are ghosts
52 (v2) / 48 (v1)
           the variables, then the blocks; each:
             u32 length, u32 number of relocations, the bytes,
             the relocations: u32 each, (offset << 1) | 1 for a function pointer, | 0 for a pointer
```

A block's portable base is not stored: it is the sum of the earlier blocks'
lengths, each rounded up to 16. Version 1 is version 2 without the ghost count
(no ghosts). About 0.5 MB for a level (50 KB deflated). The source build id is
written but not checked on load; the tables id is.

## 6. Changing it

### A new behavior, or a behavior that keeps a pointer in an object slot

Object slots are pointers per behavior. `gen_portable_tables.py`
(`behavior_pointer_slots`) works them out from the sources, so usually there
is nothing to do but regenerate:

1. Declare the field in `include/object_fields.h` with a pointer kind:
   `OBJECT_FIELD_ANIMS`, `_WAYPOINT`, `_OBJ`, `_SURFACE`, `_VPTR`, `_CVPTR`,
   `_S16P`, `_S32P` or `_CHAIN_SEGMENT` (`POINTER_KINDS`). A pointer stored in
   an `OBJECT_FIELD_S32` field is invisible to the generator, and the export
   will treat it as a number.
2. Use the field (`o->oMyPointer`) in a function the behavior's script reaches:
   `CALL_NATIVE` in `data/behavior_data.c`, or anything those functions call,
   tables of functions they name, or scripts they `GOTO`/`CALL`. The generator
   follows all of these transitively.
3. Regenerate the tables (below) and check
   `BEHAVIOR_POINTER_SLOTS` in `tables.rs` for the behavior's script address.

Fields before the `/* Pathed */` line of `object_fields.h` are common to every
object (`COMMON_POINTER_SLOTS`). A behavior the tables do not list gets every
declared pointer slot (`POINTER_SLOTS`), which is safe for pointers but may
relocate numbers.

### A new struct the walk should type

Nothing to do: every named struct in the DWARF of the sources the generator
reads (`SOURCES` in `gen_portable_tables.py`) is in `NAMED`. If
stale pointers to it can point into reused memory, add a rule to `plausible`
with a line in its table.

### Regenerating the tables

After the game is rebuilt (`make sm64-build`, which writes both builds' linker
maps):

```sh
make portable    # = maketools/gen_portable_tables.py, then maketools/portable_wasm.py
```

The first reads both builds' `sm64.map` and WASMs and the DWARF of the game's
sources and writes `rust/src/sm64/portable/tables.rs`; the second compiles the
translator to WASM (`wasm32-unknown-unknown`, then `wasm-opt -Oz`) into
`pkg/portable_wasm.js`. `PORTABLE_DUMPS=<dir>` regenerates
`portable_data_changed.txt` from memory dumps (`mem_0.bin`, `mem_*.bin`). The
headless WASM itself must not change (SM64Coin consensus).

## 7. Testing

| check | command | what it proves |
|---|---|---|
| golden | `portable_golden <rom> <out> <movie>:<every>... [raw snapshots...]`, before and after, then `cmp` the manifests | a refactor changed no output (exports, digests, heap walks, placements, load patches, v1 parsing, `load_portable`) |
| unit | `cd rust && cargo test --release` (`--test portable` for the round trips; needs `baserom.us.z64` and `tas/`) | same-build round trips play on identically |
| fuzz | `THREADS=3 portable_fuzz <rom> <movie> <seeds> <frames> <checkpoint>...` (`B2DIR=` adds rendering exports) | a loaded game stays identical to the played one under random input (GameState every frame, whole state every `EXACT_EVERY`) |
| cross build | `portable_check record/load/compare` + `portable_check.mjs record/load` | each build loads the other's states and plays on identically (see `portable-state.md` §5) |
| render | `env -u DISPLAY node javascript/e2e/portable_check.mjs quick <rom> <movie> <dir> <K> <frame>...` | a rendering build loading its own and the headless build's state draws what the played game draws |
| package | `cd javascript && npm ci && npm run build` | the JS package builds with the translator |

For a behaviour-preserving change, the golden check is the gate: an identical
manifest means every output byte is the same on the corpus. It is sensitive:
disabling the object-slot rule (and the `Surface` check) changed 2,639 of its
lines, in 369 of the 427 memory images.

## 8. Known oddities (found while writing this guide; behaviour unchanged)

- `Walked::bad_nodes` is always empty: the walk collects graph-node pointers
  with an unknown type, but never copies them to its output.
- `diagnostics::note_unresolved` labels a block `chunk@0x..` with its chunk
  *index* in hex, while `found_by` and `ghost_refs` use `block@<address>`.
- `may_point_to_constants` checks 128 object words from `rawData`, while
  `is_number_slot` checks the 80 (`OBJ_RAW_WORDS`) that are `rawData`; the extra
  range has no bits in `POINTER_SLOTS`, so it does nothing.
- `from_bytes` reads the source build id but does not check it; only the
  tables id (variable names and sizes) is checked, so a state from another
  build of the same variables, with other function indices, would load wrongly.
- `gen_portable_tables.py` hard-codes the headless `.data` start (0x7B4620) in
  two places instead of using the value it reads from the map, and has two
  dead expressions (`... and False`, `... if False else None`).
