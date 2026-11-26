# Changelog

All three packages (Rust `sm64-binds`, Python `sm64-binds`, JavaScript
`sm64-binds-frontend`) are released together at the same version.

## 2.1.2

**The game build changes** (build id 897378b4... -> 73f3e66d...), so a state or replay from 2.1.1 is not
interchangeable with this one where the differences below show. No API change.

- Game fix, port vs N64: `bhv_small_water_wave_loop` (water_objs.inc.c) and `bhv_object_bubble_loop`
  (bubble.inc.c) guarded their splash spawns with `#ifdef USE_SYSTEM_MALLOC` (always defined here) instead of
  `MALLOC_OBJECTS`, like `try_to_spawn_object`. With the object pool full the port spawned a splash (after evicting
  one more unimportant object) that the N64 refuses, and the RNG and object list parted from there. Found by
  replaying a 107,931-frame cycle in Mupen64Plus; now identical to the N64. Runs that never fill the pool in water
  are unaffected.
- Game fix, port vs N64: the object and effects memory pools (`gObjectMemoryPool`, 0x800 bytes;
  `gEffectsMemoryPool`, 0x4000) had no limit with `USE_SYSTEM_MALLOC`. They now run the N64's allocator
  (memory.c: first fit over the free list, 8-byte block headers, sizes rounded to 4, the same pool sizes), so every
  allocation succeeds or fails, at the same offset, as on the N64. The Wiggler never frees its 4 chain segments:
  on the N64 its 24th spawn in one level finds no room, and that Wiggler stays hidden and uninitialized (no body,
  no star fight; it retries every frame), where the port kept spawning it. Chain chomps free theirs and are unaffected unless the pool is full. The port-only goddard heap (size 0) keeps
  the unlimited allocator. `struct MemoryPool` grows by the N64 fields (offsets instead of pointers, so states need
  no translation for them).
- Mupen64 states: the object memory pool is copied block for block (free list included), so allocations after a
  load succeed or fail as on the N64; chain segments are placed at the N64's offsets in it.
- The WASM payloads, the precompiled module, the Mupen and portable-state tables (build ids; the `MemoryPool`
  layout) and the browser translator were regenerated for the new build.

## 2.1.1

- Release fix: Cargo.lock pins wasm-bindgen 0.2.108 again (0.2.129 broke wasmer 6.1.0's JS backend, so 2.1.0's release build failed and it was never published). Otherwise identical to 2.1.0.

## 2.1.0

**Breaking:** the in-game controller perturbation is gone from the game and the
bindings. Perturbation lives with whoever needs it (SM64Coin runs its own, outside
the game): `RngConfig`, `rng_pad`, `set_rng_config`, `set_rng_seed`, `using_rng` and
`has_rng` are removed from the Rust, Python and JavaScript APIs, with their tests and
benchmarks.

### Added

- **Portable save states**: a state saved by the headless build (Rust, Python, the nodes)
  loads in the rendering build (the browser) and the other way round, and the game goes
  on identically. Rust: `SM64Game::export_portable` / `load_portable`, `sm64_binds::portable::Portable`
  (`to_bytes` / `from_bytes`, format "SM64PORT"). JavaScript: `Sm64VisualEngine.export_portable_state()` /
  `load_portable_state(bytes)`. See `docs/portable-state.md` for the format, what a state holds and
  its limits (no state while the intro's Mario head runs; audio is not transferred).
- `Sm64VisualEngine.save_state()` / `load_state()`: the rendering build's own raw snapshot (13 MB),
  for seeking and undo within a session. Loading resets the renderer's texture cache.
- The translation as a WASM module for the browser (`rust/portable-wasm`, shipped as
  `javascript/src/scripts/sm64/pkg/portable_wasm.js`), and generators for its tables:
  `maketools/gen_portable_tables.py` (from the linker maps, WASMs and DWARF of the two builds) and
  `maketools/portable_wasm.py` (`make portable`).
- `rust/examples/portable_check.rs` and `javascript/e2e/portable_check.mjs`: cross-build validation of
  states with a TAS movie in headless Chromium.

### Changed

- The rendering build writes a linker map (`build/us_web/sm64.map`).
- The Mupen64 state tables and the precompiled LLVM module (`rust/pkg/*.wasmu`) were regenerated for the
  stock game build.

## 2.0.1

- Export the engine and ROM cache helpers from the JavaScript entry.

## 2.0.0

- Fast save states (`sync_from`), tests, benchmarks, the N64 oracle.
