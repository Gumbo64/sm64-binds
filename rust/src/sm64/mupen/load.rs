//! Loading a Mupen64 savestate into a port game.
//!
//! The port is not an N64: it is the decomp compiled to WASM, with its own
//! memory layout, its own heap (system malloc instead of the N64's pools) and
//! no ROM segments (assets are linked in). So a state cannot be copied in
//! byte for byte. Instead the port game is first brought to the same level
//! and area (by playing to it, as the N64 did), and then the game state
//! variables are carried over one by one (`tables.rs` says which, where on
//! each side, and how they are laid out), byte-swapped, with every pointer
//! translated (`translate.rs`). What the port builds when it loads a level
//! (geometry, the static collision, spawn lists, the model table) stays the
//! port's own, and serves to translate pointers into the N64's.

use anyhow::{Result, bail};

use super::MupenState;
use super::layout::{Counts, convert, named, var};
use super::tables::{BUILD_ID, N64_ONLY, PORT_ONLY, TARGETS, VARS};
use super::translate::Translator;
use crate::sm64::SM64Game;

/// What [`SM64Game::load_mupen_state`] did.
#[derive(Debug, Clone, Default)]
pub struct MupenLoadReport {
    pub level: i16,
    pub area: i16,
    /// Variables copied (out of those both builds have).
    pub vars_copied: usize,
    /// Pointer values met, and how many could not be translated (the port's
    /// value was kept).
    pub pointers: u32,
    pub pointers_kept: u32,
    /// Pointers learned from memory both sides hold (scripts, pools, tables).
    pub pairs_learned: usize,
    /// Static surfaces: (N64 count, port count, how many differ after
    /// conversion). They come from the level's collision, so they should be
    /// identical; a mismatch means the two games did not load the same area.
    pub static_surfaces: (u32, u32, u32),
    /// Dynamic surfaces (rebuilt every frame from objects): (N64 count,
    /// copied into the port's slots).
    pub dynamic_surfaces: (u32, u32),
    /// Object slots whose behavior differs between the state and the port
    /// before loading (objects the port had not spawned, or had in another
    /// slot): their pointers into level data may not translate.
    pub objects_differing: u32,
    /// Untranslated pointers, as (where, N64 address holding it, value), at
    /// most a few hundred.
    pub kept: Vec<(String, u32, u32)>,
    pub notes: Vec<String>,
    /// Environment effect particles copied: (N64 buffer, port buffer, count).
    pub envfx: Option<(u32, u32, u32)>,
}

impl std::fmt::Display for MupenLoadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "level {} area {}: {} variables, {} pointers ({} kept as the port's), {} learned pairs",
            self.level,
            self.area,
            self.vars_copied,
            self.pointers,
            self.pointers_kept,
            self.pairs_learned
        )?;
        writeln!(
            f,
            "static surfaces n64 {} port {} differing {}; dynamic surfaces n64 {} copied {}; objects differing {}",
            self.static_surfaces.0,
            self.static_surfaces.1,
            self.static_surfaces.2,
            self.dynamic_surfaces.0,
            self.dynamic_surfaces.1,
            self.objects_differing
        )?;
        for n in &self.notes {
            writeln!(f, "note: {n}")?;
        }
        Ok(())
    }
}

fn n64_only(name: &str) -> Option<u32> {
    N64_ONLY.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

fn port_only(name: &str) -> Option<u32> {
    PORT_ONLY.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

/// Little-endian reads of a port memory snapshot.
fn p32(m: &[u8], a: u32) -> u32 {
    m.get(a as usize..a as usize + 4)
        .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()))
}
fn p16(m: &[u8], a: u32) -> u16 {
    m.get(a as usize..a as usize + 2)
        .map_or(0, |b| u16::from_le_bytes(b.try_into().unwrap()))
}

/// A variable's value, as an integer, on each side (2 or 4 bytes).
fn n64_int(st: &MupenState, name: &str) -> Option<i64> {
    let v = var(name)?;
    Some(match v.size {
        1 => st.u8(v.n64)? as i8 as i64,
        2 => st.s16(v.n64)? as i64,
        _ => st.u32(v.n64)? as i32 as i64,
    })
}
fn port_int(m: &[u8], name: &str) -> Option<i64> {
    let v = var(name)?;
    Some(match v.size {
        1 => *m.get(v.port as usize)? as i8 as i64,
        2 => p16(m, v.port) as i16 as i64,
        _ => p32(m, v.port) as i32 as i64,
    })
}

/// The items of an alloc-only pool (the port's surface pools), in
/// allocation order. Blocks are 100, 200, 400... bytes, each item in one
/// block, and the newest block is the pool's `lastBlock`.
fn pool_items(m: &[u8], pool: u32, item: u32, count: u32) -> Vec<u32> {
    let mut blocks = Vec::new();
    let mut b = p32(m, pool);
    while b != 0 && blocks.len() < 64 {
        blocks.push(b);
        b = p32(m, b);
    }
    blocks.reverse();
    let mut out = Vec::with_capacity(count as usize);
    let mut size = 100u32;
    for (i, &blk) in blocks.iter().enumerate() {
        let n = if i + 1 == blocks.len() {
            p32(m, pool + 8) / item
        } else {
            size / item
        };
        for j in 0..n {
            out.push(blk + 8 + j * item);
        }
        size = size.saturating_mul(2).max(item);
    }
    out.truncate(count as usize);
    out
}

/// Where an N64 address is, for reports: a variable and offset, or heap.
fn describe(a: u32) -> String {
    let i = VARS.partition_point(|v| v.n64 <= a);
    if let Some(v) = i.checked_sub(1).map(|i| &VARS[i])
        && a - v.n64 < v.size
    {
        return format!("{}+{:#x}", v.name, a - v.n64);
    }
    format!("heap {a:#010x}")
}

const OBJECT_SIZE: u32 = 0x260;
const POOL_CAPACITY: u32 = 240;
const OBJ_BEHAVIOR: u32 = 0x20C;
const OBJ_ANIMATIONS: u32 = 0x88 + 0x26 * 4;
const OBJ_COLLISION: u32 = 0x218;
const OBJ_CUR_ANIM: u32 = 0x3C;
const OBJ_ANIM_ID: u32 = 0x38;
const OBJ_SHARED_CHILD: u32 = 0x14;
const OBJ_SPAWN_INFO: u32 = 0x4C;
const OBJ_RESPAWN_INFO: u32 = 0x25C;

impl MupenState {
    /// The current level number (`gCurrLevelNum`) in a US state.
    pub fn level(&self) -> Option<i16> {
        self.s16(var("gCurrLevelNum")?.n64)
    }
    /// The current area index (`gCurrAreaIndex`) in a US state.
    pub fn area(&self) -> Option<i16> {
        self.s16(var("gCurrAreaIndex")?.n64)
    }
    /// `gGlobalTimer`: frames since the game started.
    pub fn global_timer(&self) -> Option<u32> {
        self.u32(var("gGlobalTimer")?.n64)
    }
    /// Mario's `MarioState` (`gMarioStates[0]`) as the port lays it out,
    /// byte-swapped but with its pointers left as N64 addresses: offsets as
    /// in `include/types.h`.
    pub fn mario_state_bytes(&self) -> Option<Vec<u8>> {
        let v = var("gMarioStates")?;
        let src = self.bytes(v.n64, v.size as usize)?;
        let mut out = vec![0u8; v.size as usize];
        let mut tr = Translator::new(self, None, None);
        convert(
            v.layout,
            src,
            &mut out,
            v.n64,
            &mut tr,
            &mut Counts::default(),
        );
        // Untranslated pointers kept "the port's" value, zero here: put the
        // raw N64 values back.
        for (&at, &val) in &tr.failures {
            let o = (at - v.n64) as usize;
            out[o..o + 4].copy_from_slice(&val.to_le_bytes());
        }
        Some(out)
    }
}

impl SM64Game {
    /// Loads the game state of a Mupen64 savestate (US ROM) into this game.
    ///
    /// This game must already be in the state's level and area (play or
    /// replay to it first): the level's geometry, collision and spawn lists
    /// stay this game's own, and everything that changes while playing is
    /// copied over (Mario, objects, camera, save file, HUD, timers, RNG, ...).
    /// Fails, without changing anything, if the state is from another level
    /// or area, or this game is not the build the tables were made for.
    /// [`MupenLoadReport`] says what could not be translated.
    pub fn load_mupen_state(&mut self, st: &MupenState) -> Result<MupenLoadReport> {
        let id: String = self.build_id.iter().map(|b| format!("{b:02x}")).collect();
        if id != BUILD_ID {
            bail!("Mupen state tables are for build {BUILD_ID}, this game is build {id}");
        }
        let mut report = MupenLoadReport::default();
        if !st.is_us_rom() {
            report.notes.push(format!(
                "state is for ROM MD5 {}, not the US ROM",
                st.rom_md5
            ));
        }
        let snap: Vec<u8> = self.with_memory(|m| m.to_vec());
        let (Some(level), Some(area)) = (st.level(), st.area()) else {
            bail!("state has no level/area");
        };
        let port_level = port_int(&snap, "gCurrLevelNum").unwrap_or(-1) as i16;
        let port_area = port_int(&snap, "gCurrAreaIndex").unwrap_or(-1) as i16;
        if (level, area) != (port_level, port_area) {
            bail!(
                "the state is in level {level} area {area}, this game is in level {port_level} area {port_area}: \
                 bring the game to the same area first"
            );
        }
        report.level = level;
        report.area = area;

        // Room for the state's dynamic surfaces (rebuilt every frame from
        // objects, but Mario's floor can be one of last frame's): a fresh
        // block in each of the port's dynamic pools, from the game's malloc.
        let dynamic = |all: &str, stat: &str| {
            (n64_int(st, all).unwrap_or(0) - n64_int(st, stat).unwrap_or(0)).max(0) as u32
        };
        let (d_surfaces, d_nodes) = (
            dynamic("gSurfacesAllocated", "gNumStaticSurfaces"),
            dynamic("gSurfaceNodesAllocated", "gNumStaticSurfaceNodes"),
        );
        let mut fresh = None;
        if d_surfaces > 0 && d_nodes > 0 && self.has_export("malloc") {
            let sb = self.malloc(8 + 48 * d_surfaces)?;
            let nb = self.malloc(8 + 8 * d_nodes)?;
            if sb != 0 && nb != 0 {
                fresh = Some((sb, d_surfaces, nb, d_nodes));
            }
        }
        let snap: Vec<u8> = self.with_memory(|m| m.to_vec());

        let mut tr = Translator::new(st, n64_only("sSegmentTable"), Some(&snap));
        learn_behaviors(&mut tr, &snap);
        let level_copies = learn_tables(&mut tr, st, &snap, &mut report);
        let surfaces = learn_surfaces(&mut tr, st, &snap, &mut report, fresh);
        let object_pool = learn_object_pool(&mut tr, st, &snap, &mut report);
        let object_heap = learn_objects(
            &mut tr,
            st,
            &snap,
            &mut report,
            object_pool.as_ref().map(|p| p.space),
        );
        let paintings = learn_paintings(&mut tr, st, &snap, level);
        report.pairs_learned = tr.pair_count();

        // The variables.
        let mut writes: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut counts = Counts::default();
        for v in VARS.iter().filter(|v| v.copy) {
            let (Some(src), Some(dst)) = (
                st.bytes(v.n64, v.size as usize),
                snap.get(v.port as usize..(v.port + v.size) as usize),
            ) else {
                continue;
            };
            let mut out = dst.to_vec();
            convert(v.layout, src, &mut out, v.n64, &mut tr, &mut counts);
            writes.push((v.port, out));
            report.vars_copied += 1;
        }

        fix_s16_fields(st, &mut writes, &tr);

        if let (Some((nb, pb, count)), Some((size, layout))) =
            (report.envfx, named("EnvFxParticle"))
        {
            for k in 0..count {
                heap_struct(
                    &mut writes,
                    &mut tr,
                    &mut counts,
                    st,
                    &snap,
                    nb + k * size,
                    pb + k * size,
                    size,
                    layout,
                );
            }
        }

        // Respawn flags in the spawn lists and macro object lists, of every
        // area (they decide what spawns when an area is entered again).
        let read_only = self.read_only_memory();
        for &(n, p, size, unit) in &level_copies {
            if read_only.contains(&(p as usize)) {
                continue;
            }
            if let Some(src) = st.bytes(n, size as usize) {
                let out: Vec<u8> = src
                    .chunks_exact(unit)
                    .flat_map(|h| h.iter().rev().copied())
                    .collect();
                writes.push((p, out));
            }
        }

        // The level's paintings (their ripple state is level data).
        if let Some((size, layout)) = named("Painting") {
            for &(n, p) in &paintings {
                heap_struct(
                    &mut writes,
                    &mut tr,
                    &mut counts,
                    st,
                    &snap,
                    n,
                    p,
                    size,
                    layout,
                );
            }
        }

        // The object memory pool as the N64 has it (where its blocks are, which
        // are free), then what objects allocated in it (chain segments).
        if let Some(pool) = object_pool {
            writes.extend(pool.writes);
        }
        if let Some((size, layout)) = named("ChainSegment") {
            for &(n, p, len) in &object_heap {
                debug_assert_eq!(len, size);
                heap_struct(
                    &mut writes,
                    &mut tr,
                    &mut counts,
                    st,
                    &snap,
                    n,
                    p,
                    size,
                    layout,
                );
            }
        }

        // The camera, which the area allocates.
        if let (Some(gc), Some((size, layout))) = (var("gCamera"), named("Camera")) {
            let (n, p) = (st.u32(gc.n64).unwrap_or(0), p32(&snap, gc.port));
            if n != 0 && p != 0 {
                heap_struct(
                    &mut writes,
                    &mut tr,
                    &mut counts,
                    st,
                    &snap,
                    n,
                    p,
                    size,
                    layout,
                );
            }
        }

        // Dynamic surfaces and nodes into the port's slots; static ones are
        // compared, not written.
        if let Some(s) = surfaces {
            writes.extend(s.writes.iter().cloned());
            for (kind, n64s, ports, statics, item) in [
                (
                    "Surface",
                    &s.n64_surfaces,
                    &s.port_surfaces,
                    s.static_surfaces,
                    48u32,
                ),
                (
                    "SurfaceNode",
                    &s.n64_nodes,
                    &s.port_nodes,
                    s.static_nodes,
                    8u32,
                ),
            ] {
                let Some((size, layout)) = named(kind) else {
                    continue;
                };
                debug_assert_eq!(size, item);
                for (i, (&n, &p)) in n64s.iter().zip(ports.iter()).enumerate() {
                    if (i as u32) < statics {
                        let mut out = snap[p as usize..(p + size) as usize].to_vec();
                        let mut c = Counts::default();
                        convert(
                            layout,
                            st.bytes(n, size as usize).unwrap_or(&[]),
                            &mut out,
                            n,
                            &mut tr,
                            &mut c,
                        );
                        if kind == "Surface" && out != snap[p as usize..(p + size) as usize] {
                            report.static_surfaces.2 += 1;
                        }
                    } else {
                        heap_struct(
                            &mut writes,
                            &mut tr,
                            &mut counts,
                            st,
                            &snap,
                            n,
                            p,
                            size,
                            layout,
                        );
                        if kind == "Surface" {
                            report.dynamic_surfaces.1 += 1;
                        }
                    }
                }
            }
        }

        report.pointers = counts.pointers;
        report.pointers_kept = counts.kept;
        report.kept = tr
            .failures
            .iter()
            .take(400)
            .map(|(&at, &v)| (describe(at), at, v))
            .collect();

        // Everything was computed from the snapshot; now write.
        self.with_memory_mut(|mem| {
            for (addr, bytes) in &writes {
                let a = *addr as usize;
                if let Some(dst) = mem.get_mut(a..a + bytes.len()) {
                    dst.copy_from_slice(bytes);
                }
            }
            fix_mario_animation(st, mem, &mut report);
        });
        Ok(report)
    }
}

impl SM64Game {
    /// `size` bytes from the game's own malloc (its exported `malloc`).
    fn malloc(&mut self, size: u32) -> Result<u32> {
        let (store, exports) = self.parts_mut();
        let f: wasmer::TypedFunction<i32, i32> = exports
            .instance
            .exports
            .get_typed_function(store, "malloc")?;
        Ok(f.call(store, size as i32)? as u32)
    }

    /// Brings this game to the level and area of `st`, so that
    /// [`SM64Game::load_mupen_state`] can load it, by starting the warp the
    /// game itself would use (a level change to one of the area's warp
    /// nodes, as a checkpoint so no act select interrupts it) and playing
    /// with no input until the area has loaded. The game must be playing a
    /// level (past the file select); if it is in the middle of a transition
    /// (a fade, an area change) it plays on with no input until that ends.
    /// Returns the frames played: 0 if the game was already there.
    pub fn warp_to_mupen_area(&mut self, st: &MupenState, max_frames: u32) -> Result<u32> {
        let (Some(level), Some(area)) = (st.level(), st.area()) else {
            bail!("state has no level/area");
        };
        let here = |g: &SM64Game| {
            g.with_memory(|m| {
                (
                    port_int(m, "gCurrLevelNum").unwrap_or(-1) as i16,
                    port_int(m, "gCurrAreaIndex").unwrap_or(-1) as i16,
                    port_int(m, "sCurrPlayMode").unwrap_or(-1),
                    p32(m, var("gMarioStates").map_or(0, |v| v.port) + 0x0C),
                )
            })
        };
        let transition = |g: &SM64Game| {
            g.with_memory(|m| p32(m, var("sTransitionUpdate").map_or(0, |v| v.port)))
        };
        if here(self).0 == level && here(self).1 == area {
            return Ok(0);
        }
        // Mid-transition (an area change, a fade): play on until it ends.
        let mut waited = 0;
        loop {
            let (l, a, mode, action) = here(self);
            if (l, a) == (level, area) && mode == 0 && action != 0 {
                return Ok(waited);
            }
            if l > 1 && mode == 0 && action != 0 && transition(self) == 0 {
                break;
            }
            if waited >= max_frames || l <= 1 {
                bail!(
                    "the game must be playing a level to warp (it is in level {l}, play mode {mode})"
                );
            }
            self.step_game(crate::GamePad::default())?;
            waited += 1;
        }
        // A warp node of the destination area, from the state's own list
        // (node 0x0A, the usual entry, if it has one).
        let areas = var("gAreaData").ok_or_else(|| anyhow::anyhow!("no gAreaData"))?;
        let mut node = st.u32(areas.n64 + 60 * area as u32 + 0x14).unwrap_or(0);
        let mut ids = Vec::new();
        while node != 0 && ids.len() < 256 {
            ids.push(st.u8(node).unwrap_or(0));
            node = st.u32(node + 8).unwrap_or(0);
        }
        let Some(&first) = ids.first() else {
            bail!("the state's area has no warp nodes to arrive at");
        };
        let id = if ids.contains(&0x0A) { 0x0A } else { first };
        let writes: Vec<(&str, Vec<u8>)> = vec![
            (
                "gCurrSaveFileNum",
                (st.s16(var("gCurrSaveFileNum").unwrap().n64).unwrap_or(1))
                    .to_le_bytes()
                    .to_vec(),
            ),
            (
                "gCurrActNum",
                (st.s16(var("gCurrActNum").unwrap().n64).unwrap_or(1))
                    .to_le_bytes()
                    .to_vec(),
            ),
            ("sWarpCheckpointActive", vec![1]),
            ("sTransitionTimer", 0i16.to_le_bytes().to_vec()),
            ("sDelayedWarpOp", 0i16.to_le_bytes().to_vec()),
            (
                "sWarpDest",
                vec![1, level as u8, area as u8, id, 0, 0, 0, 0],
            ),
            // PLAY_MODE_CHANGE_LEVEL: with the timer at 0 the next frame
            // leaves for sWarpDest's level (play_mode_change_level).
            ("sCurrPlayMode", 4i16.to_le_bytes().to_vec()),
        ];
        self.with_memory_mut(|mem| {
            for (name, bytes) in writes {
                let v = var(name).unwrap();
                mem[v.port as usize..v.port as usize + bytes.len()].copy_from_slice(&bytes);
            }
        });
        for frame in 1..=max_frames {
            self.step_game(crate::GamePad::default())?;
            let (l, a, mode, action) = here(self);
            if (l, a) == (level, area) && mode == 0 && action != 0 {
                return Ok(waited + frame);
            }
        }
        bail!("the warp to level {level} area {area} did not finish in {max_frames} frames")
    }
}

#[allow(clippy::too_many_arguments)]
fn heap_struct(
    writes: &mut Vec<(u32, Vec<u8>)>,
    tr: &mut Translator,
    counts: &mut Counts,
    st: &MupenState,
    snap: &[u8],
    n64: u32,
    port: u32,
    size: u32,
    layout: u16,
) {
    let (Some(src), Some(dst)) = (
        st.bytes(n64, size as usize),
        snap.get(port as usize..(port + size) as usize),
    ) else {
        return;
    };
    let mut out = dst.to_vec();
    convert(layout, src, &mut out, n64, tr, counts);
    writes.push((port, out));
}

/// Object fields that hold two s16s were swapped as one 32-bit value by
/// the generic layout (the field union cannot say): swap their halves back,
/// for the slots every object uses that way and those its behavior does.
/// And numbers that look like segmented addresses, in slots that never
/// hold pointers, go back to being numbers.
fn fix_s16_fields(st: &MupenState, writes: &mut [(u32, Vec<u8>)], tr: &Translator) {
    use super::tables::{POINTER_SLOTS, S16_SLOTS, S16_SLOTS_ALL};
    let Some((n_pool, p_pool, _)) = super::translate::target_by_name("gObjectPool") else {
        return;
    };
    let Some(w) = writes.iter_mut().find(|w| w.0 == p_pool) else {
        return;
    };
    for i in 0..POOL_CAPACITY {
        let no = n_pool + i * OBJECT_SIZE;
        let bhv = st.u32(no + OBJ_BEHAVIOR).unwrap_or(0);
        let mut mask = S16_SLOTS_ALL;
        if let Some(name) = bhv_name(tr, bhv)
            && let Some((_, m)) = S16_SLOTS.iter().find(|(n, _)| *n == name)
        {
            mask |= m;
        }
        for slot in 0..0x50u32 {
            let at = no + 0x88 + 4 * slot;
            let Some(v) = st.u32(at) else { continue };
            let o = (i * OBJECT_SIZE + 0x88 + 4 * slot) as usize;
            if mask >> slot & 1 == 0 {
                // A segmented-looking number in a slot no behavior keeps a
                // pointer in is a number (behavior params, for one).
                if POINTER_SLOTS >> slot & 1 == 0 && super::layout::looks_like_segmented(v) {
                    w.1[o..o + 4].copy_from_slice(&v.to_le_bytes());
                }
                continue;
            }
            // Two big-endian halves: each swapped on its own.
            let (hi, lo) = ((v >> 16) as u16, v as u16);
            w.1[o..o + 2].copy_from_slice(&hi.to_le_bytes());
            w.1[o + 2..o + 4].copy_from_slice(&lo.to_le_bytes());
        }
    }
}

/// Pairs the pointers in the two builds' behavior scripts: the commands are
/// the same words, so where they differ both hold the same pointer
/// (collision data, animation tables, native functions). Animation tables
/// are then paired entry by entry.
fn learn_behaviors(tr: &mut Translator, snap: &[u8]) {
    let mut anim_tables = Vec::new();
    for t in TARGETS.iter().filter(|t| t.0 >> 24 == 0x13) {
        let Some(base) = tr.segmented_to_virtual(t.0) else {
            continue;
        };
        let words = t.2 / 4;
        let mut prev_cmd = 0u32;
        for i in 0..words {
            let (Some(n), p) = (tr.st.u32(base + 4 * i), p32(snap, t.1 + 4 * i)) else {
                break;
            };
            if n != p {
                if (0x0100_0000..0x2000_0000).contains(&n) {
                    if let Some(v) = tr.segmented_to_virtual(n) {
                        tr.pair(v, p);
                        if prev_cmd >> 24 == 0x27 {
                            anim_tables.push((v, p));
                        }
                    }
                } else if super::layout::looks_like_pointer(n) {
                    tr.pair(n, p);
                }
            }
            prev_cmd = n;
        }
    }
    for (n, p) in anim_tables {
        pair_anim_table(tr, snap, n, p);
    }
}

/// An animation table: N64 entries are segmented, the port's are pointers.
fn pair_anim_table(tr: &mut Translator, snap: &[u8], n: u32, p: u32) {
    for i in 0..64 {
        let (Some(ne), pe) = (tr.st.u32(n + 4 * i), p32(snap, p + 4 * i)) else {
            break;
        };
        if ne == 0 || pe == 0 {
            break;
        }
        let Some(nv) = tr
            .segmented_to_virtual(ne)
            .or_else(|| super::layout::looks_like_pointer(ne).then_some(ne))
        else {
            break;
        };
        tr.pair(nv, pe);
    }
}

/// The model table, the areas' heap structs and Mario's animation buffer.
/// What the level holds that changes while playing, to copy after the
/// variables: (N64 address, port address, size, swapped as 2 or 4-byte
/// values).
type LevelCopies = Vec<(u32, u32, u32, usize)>;

fn learn_tables(
    tr: &mut Translator,
    st: &MupenState,
    snap: &[u8],
    report: &mut MupenLoadReport,
) -> LevelCopies {
    let mut copies = Vec::new();
    learn_envfx(tr, st, snap, report);
    if let Some(v) = var("gLoadedGraphNodes") {
        let (n, p) = (st.u32(v.n64).unwrap_or(0), p32(snap, v.port));
        if n != 0 && p != 0 {
            for i in 0..256 {
                tr.pair(st.u32(n + 4 * i).unwrap_or(0), p32(snap, p + 4 * i));
            }
        }
    }
    for (name, len) in [
        ("gMarioAnimsMemAlloc", 0x4000),
        ("gDemoInputsMemAlloc", 0x800),
    ] {
        if let Some(v) = var(name) {
            tr.region(st.u32(v.n64).unwrap_or(0), len, p32(snap, v.port));
        }
    }
    let camera = named("Camera").map_or(0, |c| c.0);
    let spawn = named("SpawnInfo").map_or(32, |c| c.0);
    let warp = named("ObjectWarpNode").map_or(12, |c| c.0);
    if let Some(v) = var("gCamera") {
        tr.region(st.u32(v.n64).unwrap_or(0), camera, p32(snap, v.port));
    }
    let Some(areas) = var("gAreaData") else {
        return copies;
    };
    if named("Area").map(|a| a.0) != Some(60) {
        report
            .notes
            .push("struct Area is not 60 bytes; areas not paired".into());
        return copies;
    }
    for i in 0..areas.size / 60 {
        let (na, pa) = (areas.n64 + 60 * i, areas.port + 60 * i);
        let n = |o: u32| st.u32(na + o).unwrap_or(0);
        let p = |o: u32| p32(snap, pa + o);
        tr.pair(n(0x04), p(0x04));
        tr.region(n(0x24), camera, p(0x24));
        tr.region(n(0x1C), 32, p(0x1C));
        for o in [0x2C, 0x30] {
            tr.region(n(o), 8, p(o));
        }
        // Macro objects: 5 s16 per object, ended by 0x1E.
        let (nm, pm) = (n(0x10), p(0x10));
        if nm != 0 && pm != 0 {
            let mut len = 0;
            while len < 0x8000 && p16(snap, pm + len) != 0x1E {
                len += 10;
            }
            tr.region(nm, len + 2, pm);
            // Their respawn flags (set when one is collected) are in the list.
            copies.push((nm, pm, len + 2, 2));
        }
        // Spawn infos and warp nodes: lists, walked side by side.
        for (head, size, next) in [(0x20, spawn, 0x1C), (0x14, warp, 0x08)] {
            let (mut ns, mut ps) = (n(head), p(head));
            let mut k = 0;
            while ns != 0 && ps != 0 && k < 1024 {
                tr.region(ns, size, ps);
                // Spawn infos hold the respawn flags of what they spawned,
                // in behaviorArg.
                if head == 0x20 {
                    copies.push((ns + 0x10, ps + 0x10, 4, 4));
                }
                ns = st.u32(ns + next).unwrap_or(0);
                ps = p32(snap, ps + next);
                k += 1;
            }
            if (ns == 0) != (ps == 0) {
                report.notes.push(format!(
                    "area {i}: lists at {head:#x} have different lengths"
                ));
            }
        }
    }
    copies
}

/// The paintings of the current level: structs in level data (segment 7 on
/// the N64) that hold the ripple state. The painting lists in the game's
/// own data point at them on both sides.
fn learn_paintings(
    tr: &mut Translator,
    st: &MupenState,
    snap: &[u8],
    level: i16,
) -> Vec<(u32, u32)> {
    let list = match level {
        6 => "sInsideCastlePaintings",
        7 => "sHmcPaintings",
        36 => "sTtmPaintings",
        _ => return Vec::new(),
    };
    let (Some(v), Some((size, _))) = (var(list), named("Painting")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for i in 0..v.size / 4 {
        let (n, p) = (
            st.u32(v.n64 + 4 * i).unwrap_or(0),
            p32(snap, v.port + 4 * i),
        );
        let n = if super::layout::looks_like_pointer(n) {
            Some(n)
        } else {
            tr.segmented_to_virtual(n)
        };
        if let Some(n) = n
            && p != 0
        {
            tr.region(n, size, p);
            out.push((n, p));
        }
    }
    out
}

/// The environment effect particles (snow, bubbles, flowers, lava): an
/// allocation made when the area loads, paired whole and copied.
fn learn_envfx(tr: &mut Translator, st: &MupenState, snap: &[u8], report: &mut MupenLoadReport) {
    let (Some(buf), Some(mode), Some((size, _))) = (
        var("gEnvFxBuffer"),
        n64_int(st, "gEnvFxMode"),
        named("EnvFxParticle"),
    ) else {
        return;
    };
    let count = if mode == 0 {
        return;
    } else if mode < 10 {
        "gSnowParticleMaxCount"
    } else {
        "sBubbleParticleCount"
    };
    let n = n64_int(st, count)
        .unwrap_or(0)
        .min(port_int(snap, count).unwrap_or(0));
    let (nb, pb) = (st.u32(buf.n64).unwrap_or(0), p32(snap, buf.port));
    if n > 0 && nb != 0 && pb != 0 && port_int(snap, "gEnvFxMode") == Some(mode) {
        tr.region(nb, n as u32 * size, pb);
        report.envfx = Some((nb, pb, n as u32));
    }
}

struct Surfaces {
    n64_surfaces: Vec<u32>,
    port_surfaces: Vec<u32>,
    static_surfaces: u32,
    n64_nodes: Vec<u32>,
    port_nodes: Vec<u32>,
    static_nodes: u32,
    /// The port's pool and counter updates for fresh dynamic blocks.
    writes: Vec<(u32, Vec<u8>)>,
}

/// Pairs the surfaces and surface nodes by allocation order: the N64 keeps
/// them in two arrays, the port in alloc-only pools (static and dynamic).
fn learn_surfaces(
    tr: &mut Translator,
    st: &MupenState,
    snap: &[u8],
    report: &mut MupenLoadReport,
    fresh: Option<(u32, u32, u32, u32)>,
) -> Option<Surfaces> {
    let n_pool = st.u32(n64_only("sSurfacePool")?)?;
    let n_nodes = st.u32(n64_only("sSurfaceNodePool")?)?;
    let get = |name: &str| {
        (
            n64_int(st, name).unwrap_or(0) as u32,
            port_int(snap, name).unwrap_or(0) as u32,
        )
    };
    let (n_static, p_static_count) = get("gNumStaticSurfaces");
    let (n_all, p_all) = get("gSurfacesAllocated");
    let (n_static_nodes, p_static_nodes_count) = get("gNumStaticSurfaceNodes");
    let (n_all_nodes, p_all_nodes) = get("gSurfaceNodesAllocated");
    report.dynamic_surfaces = (n_all.saturating_sub(n_static), 0);
    // The port's static pools hold what loading the area made. Its
    // gNumStaticSurfaces can count a few more (allocations made while the
    // area loaded), so the pools, not the counters, say how many there are.
    let pool = |name| p32(snap, port_only(name).unwrap_or(0));
    let mut port_surfaces = pool_items(snap, pool("sStaticSurfacePool"), 48, u32::MAX);
    let mut port_nodes = pool_items(snap, pool("sStaticSurfaceNodePool"), 8, u32::MAX);
    let (p_static, p_static_nodes) = (port_surfaces.len() as u32, port_nodes.len() as u32);
    report.static_surfaces = (n_static, p_static, 0);
    if n_static != p_static || n_static_nodes != p_static_nodes {
        report.notes.push(format!(
            "static collision differs (surfaces {n_static} vs {p_static}, nodes {n_static_nodes} vs {p_static_nodes}): \
             surfaces not paired"
        ));
        return None;
    }
    let mut writes = Vec::new();
    if let Some((sb, ns, nb, nn)) = fresh {
        // The fresh blocks become the pools' newest blocks, holding all of
        // the state's dynamic surfaces and nodes; the older blocks are freed
        // with them when the next frame clears the pools.
        for (pool_var, block, count, item, all, stat) in [
            (
                "sDynamicSurfacePool",
                sb,
                ns,
                48,
                "gSurfacesAllocated",
                p_static_count,
            ),
            (
                "sDynamicSurfaceNodePool",
                nb,
                nn,
                8,
                "gSurfaceNodesAllocated",
                p_static_nodes_count,
            ),
        ] {
            let pool_at = pool(pool_var);
            if pool_at == 0 {
                return None;
            }
            let mut header = p32(snap, pool_at).to_le_bytes().to_vec();
            header.extend_from_slice(&[0; 4]);
            writes.push((block, header));
            let mut fields = block.to_le_bytes().to_vec();
            fields.extend_from_slice(&(count * item).to_le_bytes());
            fields.extend_from_slice(&(count * item).to_le_bytes());
            writes.push((pool_at, fields));
            if let Some(v) = var(all) {
                writes.push((
                    v.port,
                    (stat + count).to_le_bytes()[..v.size as usize].to_vec(),
                ));
            }
        }
        port_surfaces.extend((0..ns).map(|j| sb + 8 + 48 * j));
        port_nodes.extend((0..nn).map(|j| nb + 8 + 8 * j));
    } else {
        port_surfaces.extend(pool_items(
            snap,
            pool("sDynamicSurfacePool"),
            48,
            p_all.saturating_sub(p_static_count),
        ));
        port_nodes.extend(pool_items(
            snap,
            pool("sDynamicSurfaceNodePool"),
            8,
            p_all_nodes.saturating_sub(p_static_nodes_count),
        ));
    }
    let n64_surfaces: Vec<u32> = (0..n_all).map(|i| n_pool + 48 * i).collect();
    let n64_nodes: Vec<u32> = (0..n_all_nodes).map(|i| n_nodes + 8 * i).collect();
    for (n, p) in n64_surfaces.iter().zip(&port_surfaces) {
        tr.pair(*n, *p);
    }
    for (n, p) in n64_nodes.iter().zip(&port_nodes) {
        tr.pair(*n, *p);
    }
    let n = n64_surfaces.len().min(port_surfaces.len());
    let m = n64_nodes.len().min(port_nodes.len());
    Some(Surfaces {
        n64_surfaces: n64_surfaces[..n].to_vec(),
        port_surfaces: port_surfaces[..n].to_vec(),
        static_surfaces: n_static,
        n64_nodes: n64_nodes[..m].to_vec(),
        port_nodes: port_nodes[..m].to_vec(),
        static_nodes: n_static_nodes,
        writes,
    })
}

/// Object slots running the same behavior on both sides pair their pointers
/// into level data (animations, collision, model, spawn info), which other
/// slots with the same objects can then use.
/// The object memory pool (`gObjectMemoryPool`), copied block for block.
struct ObjectPool {
    /// Its space: (N64 address, length, port address).
    space: (u32, u32, u32),
    /// The port's pool header and space, as the N64 has them.
    writes: Vec<(u32, Vec<u8>)>,
}

/// The port's object memory pool runs the N64's allocator on the same 0x800
/// bytes (memory.c), so the N64's pool is copied as it is: the same blocks at
/// the same offsets, the same free list (the N64's links are addresses, the
/// port's offsets from the pool), so that later allocations succeed or fail
/// as on the N64. Allocated blocks' contents are swapped as words here; the
/// chain segments in them are then copied by their layout. Pointers into the
/// space translate by offset.
fn learn_object_pool(
    tr: &mut Translator,
    st: &MupenState,
    snap: &[u8],
    report: &mut MupenLoadReport,
) -> Option<ObjectPool> {
    let v = var("gObjectMemoryPool")?;
    let (n_pool, p_pool) = (st.u32(v.n64)?, p32(snap, v.port));
    let (size, _) = named("MemoryPool")?;
    if n_pool == 0 || p_pool == 0 {
        return None;
    }
    // N64: totalSpace, firstBlock, freeList {next, size}, then the space.
    // Port: the same four words at the end of its struct, offsets for pointers.
    let fixed = p_pool + size - 16;
    let n_total = st.u32(n_pool)?;
    let n_space = st.u32(n_pool + 4)?;
    let (p_total, p_first) = (p32(snap, fixed), p32(snap, fixed + 4));
    if n_total != p_total || n_total == 0 || n_space != n_pool + 16 || p_first != size {
        report.notes.push(format!(
            "object memory pool not copied: N64 {n_total:#x} bytes at {n_space:#x}, port {p_total:#x} at offset {p_first:#x}"
        ));
        return None;
    }
    let p_space = p_pool + p_first;
    let offset = |a: u32| {
        if a >= n_space && a - n_space < n_total {
            a - n_space + p_first
        } else {
            0
        }
    };
    let mut free = std::collections::HashSet::new();
    let mut at = st.u32(n_pool + 8)?;
    while at != 0 && free.len() < 1024 {
        if offset(at) == 0 || !free.insert(at - n_space) {
            break;
        }
        at = st.u32(at)?;
    }
    let mut space = vec![0u8; n_total as usize];
    let mut off = 0u32;
    while off < n_total {
        let (next, len) = (st.u32(n_space + off)?, st.u32(n_space + off + 4)?);
        if len < 8 || len % 4 != 0 || off + len > n_total {
            report.notes.push(format!(
                "object memory pool not copied: a block of {len:#x} bytes at offset {off:#x}"
            ));
            return None;
        }
        let o = off as usize;
        space[o..o + 4].copy_from_slice(&offset(next).to_le_bytes());
        space[o + 4..o + 8].copy_from_slice(&len.to_le_bytes());
        if !free.contains(&off) {
            for w in (8..len).step_by(4) {
                space[o + w as usize..o + w as usize + 4]
                    .copy_from_slice(&st.u32(n_space + off + w)?.to_le_bytes());
            }
        }
        off += len;
    }
    // (freeList.size, never read, stays the port's.)
    let mut head = Vec::with_capacity(12);
    for w in [n_total, p_first, offset(st.u32(n_pool + 8)?)] {
        head.extend_from_slice(&w.to_le_bytes());
    }
    tr.region(n_space, n_total, p_space);
    Some(ObjectPool {
        space: (n_space, n_total, p_space),
        writes: vec![(fixed, head), (p_space, space)],
    })
}

fn learn_objects(
    tr: &mut Translator,
    st: &MupenState,
    snap: &[u8],
    report: &mut MupenLoadReport,
    pool: Option<(u32, u32, u32)>,
) -> Vec<(u32, u32, u32)> {
    let mut heap = Vec::new();
    let Some((n_pool, p_pool, _)) = super::translate::target_by_name("gObjectPool") else {
        return heap;
    };
    for i in 0..POOL_CAPACITY {
        let (no, po) = (n_pool + i * OBJECT_SIZE, p_pool + i * OBJECT_SIZE);
        let nb = st.u32(no + OBJ_BEHAVIOR).unwrap_or(0);
        let pb = p32(snap, po + OBJ_BEHAVIOR);
        learn_path(tr, st, snap, no, nb);
        if tr.lookup(nb) != Some(pb) {
            report.objects_differing += 1;
            continue;
        }
        if nb == 0 {
            continue;
        }
        let n = |o: u32| st.u32(no + o).unwrap_or(0);
        let p = |o: u32| p32(snap, po + o);
        for o in [
            OBJ_ANIMATIONS,
            OBJ_COLLISION,
            OBJ_SHARED_CHILD,
            OBJ_RESPAWN_INFO,
        ] {
            if super::layout::looks_like_pointer(n(o)) && p(o) != 0 && tr.lookup(n(o)).is_none() {
                tr.pair(n(o), p(o));
            }
        }
        if n(OBJ_ANIMATIONS) != 0 && p(OBJ_ANIMATIONS) != 0 {
            pair_anim_table(tr, snap, n(OBJ_ANIMATIONS), p(OBJ_ANIMATIONS));
        }
        if st.u16(no + OBJ_ANIM_ID) == Some(p16(snap, po + OBJ_ANIM_ID))
            && tr.lookup(n(OBJ_CUR_ANIM)).is_none()
        {
            tr.pair(n(OBJ_CUR_ANIM), p(OBJ_CUR_ANIM));
        }
        if let Some((size, _)) = named("SpawnInfo")
            && tr.lookup(n(OBJ_SPAWN_INFO)).is_none()
        {
            tr.region(n(OBJ_SPAWN_INFO), size, p(OBJ_SPAWN_INFO));
        }
        let name = bhv_name(tr, nb);
        // Chain segments, which these objects allocate from the object
        // memory pool: where the pool was copied, at the same offset in the
        // port's; otherwise the port object's own allocation. Their contents
        // are copied too.
        for &(bhv, slot, count) in CHAIN_SEGMENTS {
            let o = 0x88 + 4 * slot;
            if name != Some(bhv) || !super::layout::looks_like_pointer(n(o)) {
                continue;
            }
            let len = count * CHAIN_SEGMENT_SIZE;
            let at = match pool {
                Some((ns, nl, ps)) if n(o) >= ns && n(o) - ns + len <= nl => ps + (n(o) - ns),
                _ if p(o) != 0 => {
                    tr.region(n(o), len, p(o));
                    p(o)
                }
                _ => continue,
            };
            for k in 0..count {
                heap.push((
                    n(o) + k * CHAIN_SEGMENT_SIZE,
                    at + k * CHAIN_SEGMENT_SIZE,
                    CHAIN_SEGMENT_SIZE,
                ));
            }
        }
    }
    heap
}

/// Paths (lists of 8-byte waypoints ended by a lone -1, in level data):
/// found in the port by their contents, so that the start field and the
/// current waypoint field both translate, whatever the port's slot holds.
fn learn_path(tr: &mut Translator, st: &MupenState, snap: &[u8], no: u32, nb: u32) {
    let start = st.u32(no + 0x88 + 4 * WAYPOINT_START_SLOT).unwrap_or(0);
    if !super::layout::looks_like_pointer(start)
        || !bhv_name(tr, nb).is_some_and(|b| super::tables::PATH_BEHAVIORS.contains(&b))
    {
        return;
    }
    let mut want = Vec::new();
    loop {
        let Some(w) = st.bytes(start + want.len() as u32, 8) else {
            return;
        };
        // The end is a lone -1 (TRAJECTORY_END), not a whole waypoint.
        if w[0] == 0xFF && w[1] == 0xFF {
            want.extend_from_slice(&[0xFF, 0xFF]);
            break;
        }
        for k in 0..4 {
            want.extend_from_slice(&[w[2 * k + 1], w[2 * k]]);
        }
        if want.len() > 0x1000 {
            return;
        }
    }
    // Where the start already translates (by name), the whole list follows
    // it; otherwise look for the list itself.
    let at = tr
        .lookup(start)
        .filter(|&p| snap.get(p as usize..p as usize + want.len()) == Some(&want[..]));
    if let Some(p) = at.or_else(|| find_first(snap, &want)) {
        tr.region(start, want.len() as u32, p);
    }
}

/// Where `want` first occurs in `mem` (2-byte aligned). Read-only data
/// with the same contents is interchangeable, so a copy elsewhere is as good.
fn find_first(mem: &[u8], want: &[u8]) -> Option<u32> {
    let mut at = 0;
    while at + want.len() <= mem.len() {
        if mem[at] == want[0] && mem[at + 1] == want[1] && mem[at..at + want.len()] == *want {
            return Some(at as u32);
        }
        at += 2;
    }
    None
}

/// oPathedStartWaypoint / oPlatformOnTrackStartWaypoint.
const WAYPOINT_START_SLOT: u32 = 0x1D;

/// Objects that allocate chain segments: (behavior, field slot, count), as
/// in chain_chomp.inc.c and wiggler.inc.c.
const CHAIN_SEGMENTS: &[(&str, u32, u32)] =
    &[("bhvChainChomp", 0x1B, 5), ("bhvWigglerHead", 0x1C, 4)];
const CHAIN_SEGMENT_SIZE: u32 = 0x14;

/// The name of the behavior script at N64 address `bhv`.
fn bhv_name(tr: &Translator, bhv: u32) -> Option<&'static str> {
    let seg = tr.virtual_to_segmented(bhv)?;
    TARGETS.iter().find(|t| t.0 == seg).map(|t| t.3)
}

/// Mario's animation is loaded into a buffer from a table (the N64 by DMA
/// from ROM, the port by copying): load the one the state has loaded, the way
/// `load_patchable_table` and `set_mario_animation` do.
fn fix_mario_animation(st: &MupenState, mem: &mut [u8], report: &mut MupenLoadReport) {
    let Some(v) = var("gMarioAnimsBuf") else {
        return;
    };
    let rd = |m: &[u8], a: u32| p32(m, a);
    // N64: which animation is in the buffer.
    let (Some(n_table), Some(n_current)) = (st.u32(v.n64), st.u32(v.n64 + 4)) else {
        return;
    };
    let n_count = st.u32(n_table).unwrap_or(0);
    let n_src = st.u32(n_table + 4).unwrap_or(0);
    let index = (0..n_count.min(1024))
        .find(|&i| st.u32(n_table + 8 + 8 * i).map(|o| n_src.wrapping_add(o)) == Some(n_current));
    let Some(index) = index else {
        report
            .notes
            .push("could not tell which Mario animation the state has loaded".into());
        return;
    };
    let (p_table, p_current, p_buf) = (rd(mem, v.port), rd(mem, v.port + 4), rd(mem, v.port + 8));
    if p_table == 0 || p_buf == 0 || index >= rd(mem, p_table) {
        return;
    }
    let src = rd(mem, p_table + 4).wrapping_add(rd(mem, p_table + 8 + 8 * index));
    let size = rd(mem, p_table + 12 + 8 * index);
    if src == p_current {
        return;
    }
    let (s, d, n) = (src as usize, p_buf as usize, size as usize);
    if s + n > mem.len() || d + n > mem.len() || n > 0x4000 {
        report
            .notes
            .push("Mario's animation is out of bounds; not loaded".into());
        return;
    }
    mem.copy_within(s..s + n, d);
    // values and index are offsets from the animation until loaded.
    for o in [0x0C, 0x10] {
        let off = rd(mem, p_buf + o);
        mem[d + o as usize..d + o as usize + 4]
            .copy_from_slice(&p_buf.wrapping_add(off).to_le_bytes());
    }
    mem[v.port as usize + 4..v.port as usize + 8].copy_from_slice(&src.to_le_bytes());
}
