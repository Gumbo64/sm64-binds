# Mupen64 savestates (`.st`) in sm64-binds

sm64-binds can read Mupen64 savestates and load the game state they hold into
a port game:

```rust
use sm64_binds::mupen::MupenState;

let st = MupenState::from_file("bob.st")?;          // Mupen64-rr(-lua) or Mupen64Plus
game.warp_to_mupen_area(&st, 600)?;                  // only if the game is in another area
let report = game.load_mupen_state(&st)?;            // Mario, objects, camera, save file, RNG, ...
println!("{report}");
```

- Code: `rust/src/sm64/mupen/`.
  - `st.rs`: the parser.
  - `load.rs`: the loader and warp.
  - `translate.rs`: pointer translation.
  - `layout.rs`: layout-driven byte swapping.
  - `tables.rs`: generated.
- Generator: `maketools/gen_mupen_tables.py`.
- Check tool: `rust/examples/mupen_check.rs`.
- Tests: `rust/tests/mupen.rs`, plus unit tests in `st.rs`.

## 1. File formats

Both emulators write RDRAM as **host-order (little-endian) `u32` words**. The
N64 is big-endian, so the N64 word at address `A` is the LE `u32` at
`rdram_offset + (A & 0x7FFFFC)`. An `s16` at `A` is the upper half of that
word when `A & 2 == 0`. `MupenState` stores RDRAM in N64 byte order, so its
`u8/u16/u32/f32(addr)` read N64 values directly.

### 1.1 Mupen64-rr / mupen64-rr-lua (the TAS emulator)

Sources:
- mupen64-rr-lua `src/Core/Memory/Savestates.cpp`: `generate_savestate`,
  `load_memory_from_buffer`, and `g_first_block[0xA02BB4 - 32]`.
- `src/Common/include/Common/MiscHelpers.hpp`: `auto_decompress`.

Checked against real files from rr-lua 1.4.0-7.

**Wrapper:**
- gzip (`1F 8B`). rr-lua builds that have the `st_lz4` option can write an
  LZ4 frame (`04 22 4D 18`) instead.
- Anything else is taken as uncompressed.
- There is no magic and no version field.

**Fixed part, 0xA02BB4 bytes** (offsets in the decompressed data):

| offset | size | field |
|---|---|---|
| 0x000000 | 32 | ROM MD5, ASCII hex (US: `20B854B239203BAF6C961B850A4A51A2`) |
| 0x000020 | 0x190 | register blocks, raw structs: rdram_register, MI, PI, SP, RSP, SI (SI at 0xDC), VI, RI, AI, DPC, DPS. If bit 31 of `rdram_device_manuf` is set, it is a "skip DMA" marker. |
| 0x0001B0 | 0x800000 | **RDRAM**, 8 MB, LE words |
| 0x8001B0 | 0x1000 | SP DMEM |
| 0x8011B0 | 0x1000 | SP IMEM |
| 0x8021B0 | 0x40 | PIF RAM |
| 0x8021F0 | 24 | flashram state |
| 0x802208 | 0x100000 | tlb_LUT_r (bytes) |
| 0x902208 | 0x100000 | tlb_LUT_w |
| 0xA02208 | 4 | llbit |
| 0xA0220C | 256 | GPRs, 32 × i64 |
| 0xA0230C | 256 | CP0, 32 × (u32 value, 4 unused) |
| 0xA0240C | 16 | lo, hi (i64) |
| 0xA0241C | 256 | FPRs, 32 × 64-bit |
| 0xA0251C | 8 | FCR0, FCR31 |
| 0xA02524 | 1664 | TLB entries, 32 × 52 |
| 0xA02BA4 | 4 | PC (interp_addr or the dynarec PC) |
| 0xA02BA8 | 4 | next_interrupt |
| 0xA02BAC | 4 | next_vi |
| 0xA02BB0 | 4 | vi_field |

**Variable part:**
- Event queue: (type u32, count u32) pairs, ended by a lone `0xFFFFFFFF`. The
  loader rejects a queue with no terminator.
- `is_movie` (u32).
- If `is_movie` is non-zero, the movie freeze block follows:
  - `size`, `uid` (same as the m64 header's), `current_sample`, `current_vi`,
    `length_samples`
  - then `(length_samples + 1) × 4` bytes of inputs in the m64 BUTTONS layout.
- Optional "screen section": a marker, width, height, then RGBA pixels. With
  the no-video plugin it is absent.

The real 1.4.0-7 files from this TAS decompress to 11,012,252 bytes:
- 0xA02BB4 for the fixed part
- 44 bytes of queue (5 events)
- 4 for `is_movie`
- 20 for the freeze header
- 515,236 for 128,809 inputs

**Versions:**
- Old Mupen64-rr (≤ 1.0) wrote the same fixed part and the event queue, and
  no movie block. Loading then stops after the queue.
- rr-lua added `st_lz4`, the screen section and the "skip DMA" bit trick.
- None of this changes the offsets of the fixed part, so one parser covers
  all of them. The PJ64 (`.pj`) format is not supported.

### 1.2 Mupen64Plus

Source: mupen64plus-core `src/main/savestates.c`
(`savestates_load_m64p`, `savestates_save_m64p`). Checked against real files
from libmupen64plus 2.6.0, which writes version 0x00010900.

- gzip (`osal_gzopen`). gzread also reads uncompressed files.
- The total size is 16,788,288 bytes of fixed part, plus 1024 bytes of event
  queue (v1.1+), 4 bytes of `using_tlb` (v1.1+) and 4096 bytes of extra state
  (v1.2+). The 2.6 core writes 16,793,412.

| offset | size | field |
|---|---|---|
| 0x000 | 8 | magic `M64+SAVE` |
| 0x008 | 4 | version, **big-endian** (major 1 accepted: 1.0, 1.1, 1.2, …, 1.9) |
| 0x00C | 32 | ROM MD5, ASCII |
| 0x02C | 400 | RCP registers: RDRAM 10 words, MI with padding, PI, SP plus SP status bytes, SI, VI, RI, AI with fifo, DPC plus status bytes, DPS |
| 0x1BC | 0x800000 | **RDRAM**, LE words |
| 0x8001BC | 0x2000 | SP DMEM + IMEM |
| 0x8021BC | 0x40 | PIF RAM |
| 0x8021FC | 4 + 20 | use_flashram, old flashram state |
| 0x802214 | 0x400000 | LUT_r (0x100000 u32) |
| 0xC02214 | 0x400000 | LUT_w |
| 0x1002214 | 4 | llbit |
| 0x1002218 | 256 | GPRs |
| 0x1002318 | 128 | CP0, 32 × u32 |
| 0x1002398 | 16 | lo, hi |
| 0x10023A8 | 256 | FPRs |
| 0x10024A8 | 8 | FCR0, FCR31 |
| 0x10024B0 | 1664 | TLB entries |
| 0x1002B30 | 4 | PC |
| 0x1002B34 | 12 | next_interrupt, next_vi (0), vi_field |
| 0x1002B40 | 1024 | event queue (v1.1+) |
| +1024 | 4 | using_tlb |
| +4 | 4096 | 1.2+ extra state (AI, cart, RTC, controllers, …) |

Mupen64Plus can also save PJ64 states (`.pj`, zip). The parser recognises
them and refuses them.

### 1.3 Timing

**rr-lua:**
- `savestate.do_file` requested in `emu.atinput` at sample N completes at the
  next input poll.
- The file's freeze block then says `current_sample = N+1`, and RDRAM
  gGlobalTimer is N+2.
- That state equals the port after `current_sample` `step_game` calls.

**Mupen64Plus:**
- A save requested from the input plugin's GetKeys is taken at the next VI
  interrupt (PC 0x80000180). It is a clean between-frames state.
- gGlobalTimer = T in the state equals the port after T−1 steps.
- Mupen64Plus makes one more GetKeys call at boot than rr, so an m64 needs one
  leading zero poll to sync.

**In both cases:** the port frame is `gGlobalTimer − 1`. The check tool and the
tests use that.

## 2. Feasibility

The port is the decomp compiled to wasm32. It is not an N64 emulator:
- It has its own memory layout.
- It uses system malloc instead of the N64 main pool.
- It has no ROM segments: the assets are linked in as C data.

So copying RDRAM into the port wholesale is impossible. What makes a typed
transplant possible:

- **Same struct layouts.** wasm32 and MIPS o32 both have 4-byte pointers and
  natural alignment, so the game structs have the same offsets (Object 0x260,
  MarioState 0xC8, Surface 0x30, …). Only the byte order and the pointer values
  differ. The generator warns when a variable is bigger than its N64 slot; the
  only cases are the envfx `Vtx` buffers (Vtx is bigger in the port), which are
  not copied.
- **Types from DWARF.** The generator compiles each game, engine, menu and
  buffers file again with `emcc -g`, using the headless build's defines, and
  reads the DWARF of every variable. That says where the 1, 2, 4 and 8-byte
  scalars and the pointers are. 822 variables have an address on both sides,
  and 645 of them are copied.
- **Addresses from two maps.**
  - N64: a US decomp linker map. STROOP ships one, `Mappings/MappingUS.map`.
    A few old names are aliased, e.g. `D_80339D10` = `gMarioAnimsBuf`.
  - Port: `build/us_web_headless/sm64.map`. The output is stamped with the
    WASM build id (538e01e4…), and the loader refuses other builds.
- **Pointer translation** (`translate.rs`), in this order:
  1. Exact pairs learned from memory that both sides hold:
     - The two builds' behavior scripts, word by word. They give collision
       data, animation tables and native-function pointers (the port's are WASM
       table indices). Animation tables are then paired entry by entry.
     - The model table (`gLoadedGraphNodes`).
     - Surfaces and surface nodes in allocation order. The N64 keeps them in
       arrays; the port keeps them in alloc-only pools, which the loader walks.
     - Same-behavior object slots: their model, animation, collision and
       respawn pointers.
  2. Regions:
     - the camera
     - spawn info and warp node lists, walked side by side
     - macro object lists
     - instant warps and whirlpools
     - Mario's animation buffer
     - chain segments (chain chomp, wiggler)
     - the envfx particle buffer
     - the level's paintings
     - **paths**: waypoint lists, found in the port by their contents
  3. Same-named variables, at the same offset. Each one is bounded by the next
     N64 symbol, so heap pointers are not mistaken for a variable's.
  4. Pointers into a loaded ROM segment:
     - Behavior scripts (segment 0x13), by symbol.
     - Otherwise, port data named after its N64 segmented address (the decomp's
       `…_0600E264` names, about 14,000 of them). This is accepted only if the
       contents on both sides agree, compared word by word as bytes, 32-bit or
       2×16-bit values, or (for pointer tables) one level down. Every segment
       whose base is at or below the pointer is tried.

  Anything else keeps the port's own value, and the report lists it.

**Hard parts, and what the loader does about each:**
- **The object field union** (`rawData`): 4-byte slots that are a pointer,
  float or int depending on the behavior.
  - Words that look like KSEG0 pointers (0x80000001–0x807FFFFF) are
    translated.
  - Segmented-looking words (the N64 keeps e.g. `oAnimations` as the script
    gave it) are translated only in slots that some field type declares as a
    pointer (`POINTER_SLOTS`). Elsewhere they stay numbers; `oBehParams`
    0x06000000 is one.
- **Two-s16 slots.**
  - Slots 0x02 and 0x4C (`oDialogResponse/State`, `oFloorType/Room`) are two
    s16s in every object.
  - Other slots are two s16s only for some behaviors (Bowser, Boo, Koopa,
    Ukiki, …). The generator works these out field → file → native function
    → behavior, following GOTO/CALL between scripts (bhvUkiki jumps into
    bhvMacroUkiki).
  - These slots get their halves swapped separately.
- **uintptr_t** (`bhvStack`, …) is treated as an ambiguous word.
- **Mario's animation**: the loader reloads the animation the N64 has in its
  DMA buffer into the port's buffer, as `load_patchable_table` does.
- **Dynamic surfaces**: the state's surfaces go into a fresh block in each of
  the port's dynamic pools, allocated with the game's exported `malloc`, so
  that Mario's floor or wall can point at last frame's platform surface.
- **Static collision** is not copied. It is compared instead, as a check that
  both sides loaded the same area: every test matched byte for byte after
  conversion. The port's `gNumStaticSurfaces` counts a few extra area-load
  allocations, so the pool walk gives the real count.
- **Level data that changes while playing** is copied into the port's copies:
  - respawn flags in every area's spawn infos and macro lists
  - the paintings' ripple state
  - the envfx particles
- **Variables the port has but no unit claims** are copied too:
  `gAreaUpdateCounter`, which drives object animation timers, and
  `gObjParentGraphNode`. So are `gMarioObject` and `gMarioCurrentRoom`
  (rooms decide which objects are active, e.g. in HMC).

**What is not loaded**, and stays the port's:
- The level geometry and graph nodes, the static collision and the spawn
  lists. The port builds these when it loads the area, so the game must be in
  the same level and area first.
  - `warp_to_mupen_area` gets it there. It sets `sWarpDest` to one of the
    state's warp nodes, as a checkpoint so there is no act select, sets
    PLAY_MODE_CHANGE_LEVEL, and steps with no input. The level loads in 3
    frames.
  - The game must be in a level for this. If it is mid-transition, the warp
    first plays on with no input until the transition ends.
  - An area with no warp nodes cannot be warped to.
- Audio, rendering state (display lists, matrices, `cameraToObject`, text and
  moving textures), thread and message queues, the level script interpreter,
  allocator state.
- **Menus.** A title, file select or act select state does not continue,
  because their state lives in the level script and menu code. Load in-level
  states.
- **States taken while a level is being torn down.** rr_104000 is in a star
  dance exit, with the areas already cleared. It loads, then diverges after 31
  frames.
- Function pointers not found in a behavior script (camera mode tables and the
  like): the port's values are kept. These are constant tables, so nothing is
  lost.
- Heap data that objects allocate, other than chain segments and the envfx
  buffer. No other case came up in the tests.
- CPU state and RCP registers are parsed but not used: the port runs from
  frame boundaries.

## 3. Validation

The states were made from the 2026 120-star TAS
(`ref/tas/full-120star-2026-U.m64`):
- **Mupen64-rr-lua 1.4.0-7** under Wine 10 + Xvfb, with the no-video, no-audio,
  TASInput and TASRSP plugins and a Lua script. 34 states: samples 200, 1646
  and 23172, then every 4000 from 4000 to 124000.
- **Mupen64Plus 2.6.0** (Debian libmupen64plus2 + rsp-hle) through a small C
  front end, with an m64 input plugin and a null video plugin. 41 states:
  9 hand-picked, then every 4000 frames from 2000 to 126000.

Both emulators play the whole movie in sync with the port.

The tool:

```
cargo run --release --example mupen_check -- <rom> <m64> <st> [earlier] [after]
```

1. It replays the movie in the port to the state's frame, as the reference.
2. It replays a second game to `earlier` frames before that. If needed, it
   warps that game to the state's area.
3. It loads the state into the second game, diffs every copied variable
   against the reference, and plays both on.

Options:
- `MUPEN_FROM=<frame>`: start the second game from another frame or level.
- `MUPEN_LATER=<st>`: also compare with a later state of the same movie,
  i.e. against the emulator itself.
- `MUPEN_DETAIL`, `MUPEN_WATCH`, `MUPEN_ALL`: diagnostics.

**Results** (all 75 states):

| check | result |
|---|---|
| in-level states loaded 100 frames early (300 or 10 when the game was mid-transition there; 10 needed a warp from another area), then 2000 frames against the port's replay | 69 of 70 in sync. The exception is rr_104000 (level teardown, above). |
| each wide state loaded, then played to the next state (4000 frames later) and compared with what the emulator had there | 60 of 61 identical (rr 29/30, Mupen64Plus 31/31). The exception is rr_104000 again. |
| cross-level loads, e.g. a BoB state into a game at castle grounds frame 1700 (warp) | in sync for 3000–6000 frames, through the star, the course exit and later levels |
| the same state loaded into the same frame | copied variables differ only by heap addresses (relocated surfaces, nodes, spawn infos), the save buffer's checksums (the port never writes EEPROM), star-select leftovers and rendering-only fields |
| title and file-select states (5) | not in a level; refused or not continued |

"In sync" and "identical" mean Mario's action, position, velocity and forward
speed were equal, bit for bit. Hidden differences in object state show up as
Mario differences within a few thousand frames. Finding and fixing those is
how the respawn flags, paths, `gAreaUpdateCounter`, envfx, `gMarioObject` and
the two-s16 field cases above were found.

The integration test `tests/mupen.rs` repeats this for every in-level `.st` in
`SM64_MUPEN_STATES`, against the movie in `SM64_MUPEN_M64`: it loads from 100
frames before (warping if needed), then checks Mario right away and 300 frames
later. The states hold RDRAM derived from the ROM, so none are in the
repository.

## 4. Regenerating the tables

```
python3 maketools/gen_mupen_tables.py /path/to/MappingUS.map [sm64-port]
```

- Needs emcc and llvm-dwarfdump (emsdk) and the headless build's `sm64.map`
  and `sm64.us.wasm`. Set `JOBS` to the number of parallel compiles
  (default 2).
- Rerun it after rebuilding the WASM: the loader checks the build id.
- Besides the variables, it writes:
  - the per-behavior two-s16 field table
  - the pointer slots
  - the path behaviors
  - the segment-named port symbols

  The first three come from `include/object_fields.h`,
  `src/game/behaviors/*.inc.c` and `data/behavior_data.c`.
