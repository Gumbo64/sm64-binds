# Changelog

All three packages (Rust `sm64-binds`, Python `sm64-binds`, JavaScript
`sm64-binds-frontend`) are released together at the same version.

## 2.1.3

**The game build changes** (build id 73f3e66d... -> 5e3b86e1...; rendering build 76691905... -> 6dafbeca...), so a state
or replay from 2.1.2 is not interchangeable with this one where the differences below show. No API change.

- Game fix, port vs N64: the surface pools. With `USE_SYSTEM_MALLOC` the port kept surfaces and surface nodes in
  four unbounded alloc-only pools (static and dynamic). They are now the N64's two pools, arrays of the N64's sizes
  (`sSurfaceNodePool`, 7000 nodes; `sSurfacePool`, 2300 surfaces), allocated in order from index 0 when an area
  loads, dynamic entries after the static ones, so counts, indices and stale entries are the N64's. The arrays come
  from malloc once, at the first level, and are never freed, so the game's variables keep their addresses in both
  builds.
- Game fix, port vs N64: `load_area_terrain` no longer calls `clear_dynamic_surfaces()` (a port addition, against
  dangling pointers into freed pools). That reset the counters it had just zeroed to the previous area's
  `gNumStaticSurfaces` / `gNumStaticSurfaceNodes`, so they grew with every area loaded (32,736 "static" nodes in BoB
  late in the v0.2 cycle 1, where the N64 has 5,340), and emptied the dynamic partition.
  As on the N64, the counters now start at 0 for every area, and after an area change the dynamic partition keeps
  listing the old area's dynamic surfaces (their entries are still in the pools) until the next frame clears it, so
  Mario's spawn `find_floor` sees what it sees on the N64. Found by comparing the counters with Mupen64Plus at
  every poll: they differed at 35,370 of 35,405 polls of the v0.2 cycle 1 and now match at all of them.
- Overflow: the N64 has no bounds check (`alloc_surface` / `alloc_surface_node`: "A bounds check! ... we, um...").
  Nodes past 7000 are written over the surface pool's main pool block header and then over the surfaces (static
  ones first); surfaces past 2300 into the free main pool, which the display list heap overwrites while the frame
  is drawn, and past that into the memory after the main pool. The values written over surfaces are N64 pointers
  and display lists, so this cannot be reproduced; the port gives the pools spare entries instead (4096 nodes, 1024
  surfaces): what overflows works for the game logic as written, as on the N64 until something reads the
  overwritten memory (the camera during the frame's drawing, or a corrupted static surface). Past the spare
  entries the surface or node is left out (the counters still count, as on the N64). In vanilla play the pools
  never fill: the fullest areas seen (TASes, SM64Coin cycles, in Mupen64Plus) peak at 2196 of 2300 surfaces (BBH)
  and 5655 of 7000 nodes. Forced overflows (`gNumStaticSurfaces` or `gNumStaticSurfaceNodes` raised in BoB, in
  Mupen64Plus and the port alike: up to 48 surfaces or 84 nodes past the pools) stay identical, Mario and the RNG,
  for the rest of the cycle.
- Mupen64 states: the surface and node pools are copied entry for entry (pointers into them translate by offset),
  as are the four surface counters; the dynamic-block allocation the loader did is gone.
- Portable states: every entry of the two pools is typed, stale ones too.
- The WASM payloads, the precompiled module, the Mupen and portable-state tables (build ids, the surface_load
  variables) and the browser translator were regenerated for the new build.

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
