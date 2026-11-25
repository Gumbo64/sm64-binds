//! Finding the game's state in a build's memory and putting it in the
//! portable form: its variables, and the malloc blocks reachable from them,
//! with every pointer replaced by a build-neutral one.
//!
//! The portable address space is the headless build's for globals and
//! constant data, `HEAP_BASE` and up for malloc blocks (each at the offset
//! its position in the state's block list gives it), and the headless
//! function table's indices for function pointers.

use std::collections::{HashMap, HashSet};

use super::addresses::*;
use super::builds::{Build, var_addr};
use super::excluded::*;
use super::heap::*;
use super::layout::K;
use super::tables::*;
use super::types::*;

// --- the walk ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Slot {
    Var(usize),
    Chunk(usize),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Region {
    pub(super) addr: u32,
    pub(super) layout: u16,
    /// For [`PTRS`] regions: how many pointers.
    pub(super) count: u32,
    pub(super) slot: Slot,
    /// Offset of `addr` in the slot's bytes.
    pub(super) off: u32,
}

impl Region {
    pub(super) fn slot_desc(&self) -> String {
        match self.slot {
            Slot::Var(i) => VARS[i].0.to_string(),
            Slot::Chunk(c) => format!("chunk {c}"),
        }
    }
}

/// A pointer or function pointer in a buffer, by offset: the value there is
/// portable. Kind 0 is a pointer, 1 a function table index.
pub type Reloc = (u32, u8);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buf {
    pub data: Vec<u8>,
    pub relocs: Vec<Reloc>,
}

#[derive(Debug, Clone, Default)]
pub struct Walked {
    /// How many of the last `chunks` are ghosts (raw copies of freed memory
    /// that stale pointers read; see `docs/portable-state.md`).
    pub ghosts: usize,
    /// The stale pointers made ghost-relative: (variable or block, offset, value).
    pub ghost_refs: Vec<(String, u32, u32)>,
    /// One per entry of `VARS` (even ones that could not be read: empty).
    pub vars: Vec<Buf>,
    /// The malloc blocks reached, in the order they were found. Block `i` is
    /// at portable address `HEAP_BASE + block_bases[i]`.
    pub chunks: Vec<Buf>,
    pub chunk_addrs: Vec<u32>,
    pub chunk_lens: Vec<u32>,
    pub chunk_bases: Vec<u32>,
    /// Where each block was first found: (variable or block description, offset).
    pub found_by: Vec<(String, u32)>,
    /// GraphNode pointers whose target has an unknown type: (address, type).
    pub bad_nodes: Vec<(u32, u16)>,
    /// Pointers that could not be translated (and were set to null), as
    /// (variable or block description, offset, value).
    pub unresolved: Vec<(String, u32, u32)>,
    pub unresolved_count: u32,
}

pub(super) struct Walk<'a> {
    pub(super) build: Build,
    pub(super) mem: &'a [u8],
    pub(super) chunks: Vec<Chunk>,
    /// Every chunk of the heap (address, size, in use), by address.
    pub(super) all: Vec<(u32, u32, bool)>,
    /// `gMatStackFixed`'s variable index: render walk pointers, not state.
    pub(super) mat_fixed: Option<usize>,
    pub(super) order: Vec<usize>,
    pub(super) reached: HashMap<usize, usize>,
    pub(super) regions: Vec<Region>,
    pub(super) seen: HashSet<(u32, u16)>,
    pub(super) queue: Vec<usize>,
    /// Pointers in this address range are not followed (the display list pool).
    pub(super) skip: Vec<(u32, u32)>,
    pub(super) found_by: Vec<(Slot, u32)>,
    pub(super) bad_nodes: Vec<(u32, u16)>,
}

pub(super) enum Tgt {
    Null,
    Heap(usize, u32),
    Global(u32),
    Unresolved,
}

impl<'a> Walk<'a> {
    pub(super) fn chunk_of(&self, v: u32) -> Option<usize> {
        let i = self.chunks.partition_point(|c| c.user <= v);
        let c = self.chunks.get(i.checked_sub(1)?)?;
        (v - c.user < c.len).then_some(i - 1)
    }

    pub(super) fn classify(&self, v: u32) -> Tgt {
        if v == 0 {
            return Tgt::Null;
        }
        if let Some(ci) = self.chunk_of(v) {
            return Tgt::Heap(ci, v - self.chunks[ci].user);
        }
        match to_canonical_global(self.build, v) {
            Some(g) => Tgt::Global(g),
            None => Tgt::Unresolved,
        }
    }

    /// The free chunk (address, size) holding `v`, if `v` is in one.
    pub(super) fn free_chunk_of(&self, v: u32) -> Option<(u32, u32)> {
        let i = self.all.partition_point(|c| c.0 <= v);
        let &(c, size, in_use) = self.all.get(i.checked_sub(1)?)?;
        (!in_use && v >= c + 8 && v < c + size).then_some((c, size))
    }

    pub(super) fn reach(&mut self, ci: usize, by: (Slot, u32)) -> usize {
        if let Some(&i) = self.reached.get(&ci) {
            return i;
        }
        self.found_by.push(by);
        let i = self.order.len();
        self.order.push(ci);
        self.reached.insert(ci, i);
        i
    }

    pub(super) fn push_region(&mut self, addr: u32, layout: u16, slot: Slot, off: u32) {
        self.push_region_n(addr, layout, 0, slot, off);
    }

    pub(super) fn push_region_n(
        &mut self,
        addr: u32,
        layout: u16,
        count: u32,
        slot: Slot,
        off: u32,
    ) {
        if self.seen.insert((addr, layout)) {
            self.regions.push(Region {
                addr,
                layout,
                count,
                slot,
                off,
            });
            self.queue.push(self.regions.len() - 1);
        }
    }

    pub(super) fn process(&mut self, ri: usize) {
        let Region {
            addr,
            layout,
            count,
            slot,
            off: roff,
        } = self.regions[ri];
        if self.mat_fixed.is_some_and(|m| slot == Slot::Var(m)) {
            return;
        }
        // A main pool block that holds an allocation pool (its release handler
        // says so): the pool sits right after the header, reached by no pointer.
        if let (Slot::Chunk(_), Some(pl)) = (slot, pool_layout())
            && Some(layout) == named_by("MainPoolBlock").map(|x| x.1)
            && let Some(h) = rd(self.mem, addr + 8)
            && FNS.iter().any(|f| {
                f.2 == "alloc_only_pool_release_handler"
                    && h == if self.build == Build::Headless {
                        f.0
                    } else {
                        f.1
                    } as u32
            })
            && roff + 12 + 12
                <= self.chunks[match slot {
                    Slot::Chunk(c) => c,
                    _ => 0,
                }]
                .len
        {
            self.push_region(addr + 12, pl, slot, roff + 12);
        }
        let obj = object_layout();
        let mut found: Vec<(u32, K, u16, u32)> = Vec::new();
        rleaves(layout, addr, count, &mut |a, k, l, o| {
            if matches!(k, K::Ptr(_) | K::Word) {
                found.push((a, k, l, o));
            }
        });
        for (a, k, l, o) in found {
            if self.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                continue;
            }
            let Some(v) = rd(self.mem, a) else { continue };
            if let K::Ptr(pid) = k
                && is_render_list(pid)
            {
                continue;
            }
            let tgt = match k {
                K::Ptr(_) => self.classify(v),
                _ if self.numeric_slot(obj, l, a, o) => Tgt::Null,
                _ => self.word_target(v, self.rodata_slot(obj, l, o)),
            };
            if let Tgt::Heap(ci, off) = tgt {
                self.reach(ci, (slot, a - addr + roff));
                if let K::Ptr(pid) = k {
                    let name = POINTEES[pid as usize];
                    let info = if name == "GraphNode" {
                        let r = self.refine(name, v);
                        if r.is_none() && self.bad_nodes.len() < 40 {
                            let t = u16::from_le_bytes(
                                self.mem[v as usize..v as usize + 2].try_into().unwrap(),
                            );
                            self.bad_nodes.push((v, t));
                        }
                        r
                    } else {
                        pointee(pid)
                    };
                    if name == "ptr" {
                        let c = self.chunks[ci];
                        let mut n = 0;
                        while n < 64 && off + 4 * n + 4 <= c.len {
                            let Some(w) = rd(self.mem, v + 4 * n) else {
                                break;
                            };
                            if !matches!(self.word_target(w, false), Tgt::Heap(..) | Tgt::Global(_))
                            {
                                break;
                            }
                            n += 1;
                        }
                        if n > 0 && v % 4 == 0 {
                            self.push_region_n(v, PTRS, n, Slot::Chunk(ci), off);
                        }
                    }
                    if let Some((size, lay)) = info {
                        let c = self.chunks[ci];
                        if off + size <= c.len && v % 2 == 0 && self.plausible(name, v) {
                            self.push_region(v, lay, Slot::Chunk(ci), off);
                        }
                    }
                }
            }
        }
    }
}

/// Finds the state in `mem`: the variables, and every malloc block reachable
/// from them by a pointer.
pub fn walk(mem: &[u8], build: Build) -> Result<Walked, String> {
    if let Some(why) = unsupported(mem, build) {
        return Err(why);
    }
    walk_unchecked(mem, build)
}

/// [`walk`] of a state that may be unsupported: what the game's own blocks
/// are, to reuse them.
#[allow(clippy::needless_range_loop)]
pub fn walk_unchecked(mem: &[u8], build: Build) -> Result<Walked, String> {
    let all = heap_all(mem, build)?;
    let chunks = heap_chunks(mem, build)?;
    let mut w = Walk {
        build,
        mem,
        chunks,
        all,
        mat_fixed: VARS.iter().position(|v| v.0.ends_with(":gMatStackFixed")),
        order: Vec::new(),
        reached: HashMap::new(),
        regions: Vec::new(),
        seen: HashSet::new(),
        queue: Vec::new(),
        skip: draw_pools(mem, build)
            .iter()
            .map(|&p| (p, p + 16))
            .collect(),
        found_by: Vec::new(),
        bad_nodes: Vec::new(),
    };
    for i in 0..VARS.len() {
        let (addr, size, layout) = var_addr(build, i);
        if addr as usize + size as usize > mem.len() {
            return Err(format!("variable {} is outside memory", VARS[i].0));
        }
        if layout != 0xFFFF {
            w.push_region(addr, layout, Slot::Var(i), 0);
        }
    }
    while let Some(ri) = w.queue.pop() {
        w.process(ri);
    }
    // Words no layout describes that hold exactly the address of a block (the
    // links of allocation pools, whose owners sit in structs reached only by
    // value): followed, and translated like pointers.
    let mut exact: Vec<(usize, u32, usize, u32)> = Vec::new(); // (chunk, offset, target chunk, offset in it)
    {
        let mut cover: HashMap<usize, Vec<bool>> = HashMap::new();
        for r in &w.regions {
            if let Slot::Chunk(ci) = r.slot {
                let c = cover
                    .entry(ci)
                    .or_insert_with(|| vec![false; w.chunks[ci].len as usize]);
                rleaves(r.layout, r.addr, r.count, &mut |a, k, _, _| {
                    let sz = match k {
                        K::S1 => 1,
                        K::S2 => 2,
                        K::S8 => 8,
                        K::Opaque(n) => n,
                        _ => 4,
                    } as usize;
                    let o = (a - r.addr + r.off) as usize;
                    for b in o..(o + sz).min(c.len()) {
                        c[b] = true;
                    }
                });
            }
        }
        let mut done = 0;
        while done < w.order.len() {
            let ci = w.order[done];
            done += 1;
            let c = w.chunks[ci];
            let cov = cover.get(&ci);
            // The link of an allocation pool's blocks: the first word of a block
            // is the exact address of the pool's previous block. (Other words are
            // not looked at: in a block of s16 data, one in a few thousand looks
            // like an address.)
            let Some(v) = rd(mem, c.user) else { continue };
            if v == 0 || v % 4 != 0 {
                continue;
            }
            if cov.is_some_and(|x| x[0]) {
                continue;
            }
            if let Some(i) = w.chunk_of(v).filter(|&i| w.chunks[i].user == v && i != ci) {
                w.reach(i, (Slot::Chunk(ci), 0));
                exact.push((ci, 0, i, 0));
            }
        }
    }

    // Output buffers.
    let mut out = Walked::default();
    for i in 0..VARS.len() {
        let (addr, size, _) = var_addr(build, i);
        out.vars.push(Buf {
            data: mem[addr as usize..(addr + size) as usize].to_vec(),
            relocs: Vec::new(),
        });
    }
    let mut base = 0u32;
    for &ci in &w.order {
        let c = w.chunks[ci];
        out.chunk_addrs.push(c.user);
        out.chunk_lens.push(c.len);
        out.chunk_bases.push(base);
        out.chunks.push(Buf {
            data: mem[c.user as usize..(c.user + c.len) as usize].to_vec(),
            relocs: Vec::new(),
        });
        base += (c.len + 15) & !15;
    }
    for &(ci, o, ti, toff) in &exact {
        let buf = &mut out.chunks[w.reached[&ci]];
        let val = HEAP_BASE + out.chunk_bases[w.reached[&ti]] + toff;
        buf.data[o as usize..o as usize + 4].copy_from_slice(&val.to_le_bytes());
        buf.relocs.push((o, 0));
    }
    let regions = std::mem::take(&mut w.regions);
    out.found_by = w
        .found_by
        .iter()
        .map(|&(s, o)| match s {
            Slot::Var(i) => (VARS[i].0.to_string(), o),
            Slot::Chunk(c) => (format!("block@{:#x}", w.chunks[c].user), o),
        })
        .collect();
    let obj = object_layout();
    let pool = pool_layout();
    if let Some(m) = w.mat_fixed {
        // Render walk matrices: pointers into per-frame memory, not state.
        out.vars[m].data.fill(0);
    }

    // Stale pointers: into freed heap memory. The original game reads what is
    // left there; the memory is not state, so a ghost (a raw copy of the bytes
    // around the target, never interpreted) keeps them reading the same.
    let mut stale: Vec<(Slot, u32, u32, u32)> = Vec::new(); // (slot, offset in it, value, window)
    for r in &regions {
        if w.mat_fixed.is_some_and(|m| r.slot == Slot::Var(m)) {
            continue;
        }
        let mut items: Vec<(u32, K, u16, u32)> = Vec::new();
        rleaves(r.layout, r.addr, r.count, &mut |a, k, l, o| {
            items.push((a, k, l, o))
        });
        for (a, k, l, o) in items {
            if w.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                continue;
            }
            let Some(v) = rd(mem, a) else { continue };
            if w.render_only(r.slot) {
                continue;
            }
            let window = match k {
                // A struct we know, at an aligned address (a typed region over reused
                // memory holds numbers that look like pointers).
                K::Ptr(pid) if !is_render_list(pid) && v % 4 == 0 => {
                    pointee(pid).map(|p| p.0.max(0x40))
                }
                // Only an object's pointer slots: other words may be numbers.
                K::Word if Some(l) == obj && !w.numeric_slot(obj, l, a, o) && v % 4 == 0 => {
                    Some(0x40)
                }
                _ => None,
            };
            if let Some(win) = window
                && matches!(w.classify(v), Tgt::Unresolved)
                && w.free_chunk_of(v).is_some()
            {
                stale.push((r.slot, a - r.addr + r.off, v, win));
            }
        }
    }
    let mut windows: Vec<(u32, u32)> = Vec::new();
    for &(_, _, v, win) in &stale {
        let (c, size) = w.free_chunk_of(v).unwrap();
        let start = (v & !7).max(c + 8);
        let end = ((start + win + 7) & !7).min(c + size).max(start);
        windows.push((start, end));
    }
    windows.sort_unstable();
    let mut ghosts: Vec<(u32, u32)> = Vec::new();
    for (s0, e0) in windows {
        match ghosts.last_mut() {
            Some(g) if s0 <= g.1 => g.1 = g.1.max(e0),
            _ => ghosts.push((s0, e0)),
        }
    }
    let ghost_first = out.chunks.len();
    let mut ghost_bases = Vec::new();
    for &(s0, e0) in &ghosts {
        ghost_bases.push(base);
        out.chunk_addrs.push(s0);
        out.chunk_lens.push(e0 - s0);
        out.chunk_bases.push(base);
        out.chunks.push(Buf {
            data: mem[s0 as usize..e0 as usize].to_vec(),
            relocs: Vec::new(),
        });
        base += (e0 - s0 + 15) & !15;
    }
    out.ghosts = ghosts.len();
    let mut ghost_val: HashMap<(Slot, u32), u32> = HashMap::new();
    for &(slot, off, v, _) in &stale {
        let gi = ghosts.partition_point(|g| g.0 <= v) - 1;
        ghost_val.insert(
            (slot, off),
            HEAP_BASE + ghost_bases[gi] + (v - ghosts[gi].0),
        );
        let what = match slot {
            Slot::Var(i) => VARS[i].0.to_string(),
            Slot::Chunk(c) => format!("block@{:#x}", w.chunks[c].user),
        };
        out.ghost_refs.push((what, off, v));
    }
    let _ = ghost_first;

    for r in &regions {
        if w.mat_fixed.is_some_and(|m| r.slot == Slot::Var(m)) {
            continue;
        }
        let mut items: Vec<(u32, K, u16, u32)> = Vec::new();
        rleaves(r.layout, r.addr, r.count, &mut |a, k, l, o| {
            items.push((a, k, l, o))
        });
        for (a, k, l, o) in items {
            if w.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                // the display list pool (zeroed below)
                continue;
            }
            let Some(v) = rd(mem, a) else { continue };
            if let K::Ptr(pid) = k
                && is_render_list(pid)
            {
                let off = (a - r.addr + r.off) as usize;
                let buf = match r.slot {
                    Slot::Var(i) => &mut out.vars[i],
                    Slot::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
                };
                buf.data[off..off + 4].copy_from_slice(&[0; 4]);
                continue;
            }
            let ghost = ghost_val
                .get(&(r.slot, a - r.addr + r.off))
                .map(|&g| (g, 0u8));
            let canon: Option<(u32, u8)> = match k {
                _ if ghost.is_some() => ghost,
                K::Ptr(_) => match w.classify(v) {
                    Tgt::Null => None,
                    Tgt::Heap(ci, off) => w
                        .reached
                        .get(&ci)
                        .map(|&bi| (HEAP_BASE + out.chunk_bases[bi] + off, 0)),
                    Tgt::Global(g) => Some((g, 0)),
                    Tgt::Unresolved => {
                        note_unresolved(&mut out, r, a, v);
                        None
                    }
                },
                K::Word if w.numeric_slot(obj, l, a, o) => None,
                K::Word => match w.word_target(v, w.rodata_slot(obj, l, o)) {
                    Tgt::Heap(ci, off) => w
                        .reached
                        .get(&ci)
                        .map(|&bi| (HEAP_BASE + out.chunk_bases[bi] + off, 0)),
                    Tgt::Global(g) => Some((g, 0)),
                    _ => None,
                },
                K::Fn => {
                    if v == 0 {
                        None
                    } else {
                        match fn_to_canonical(build, v) {
                            Some(c) => Some((c, 1)),
                            None => {
                                note_unresolved(&mut out, r, a, v);
                                None
                            }
                        }
                    }
                }
                _ => None,
            };
            let Some((val, kind)) = canon else { continue };
            let off = a - r.addr + r.off;
            let buf = match r.slot {
                Slot::Var(i) => &mut out.vars[i],
                Slot::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
            };
            buf.data[off as usize..off as usize + 4].copy_from_slice(&val.to_le_bytes());
            if kind != 9 {
                buf.relocs.push((off, kind));
            }
        }
        // The pool's extra headless field and (below) the display list pool.
        if Some(r.layout) == pool {
            let off = r.off as usize;
            let buf = match r.slot {
                Slot::Var(i) => &mut out.vars[i],
                Slot::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
            };
            if buf.data.len() >= off + 16 {
                buf.data[off + 12..off + 16].copy_from_slice(&[0; 4]);
            }
        }
    }
    // The display list pool is rebuilt every frame; its blocks are not state
    // (and live in different memory in the two builds).
    for p in draw_pools(mem, build) {
        let Some(ci) = w.chunk_of(p) else { continue };
        let Some(&bi) = w.reached.get(&ci) else {
            continue;
        };
        let off = (p - w.chunks[ci].user) as usize;
        let buf = &mut out.chunks[bi];
        if buf.data.len() >= off + 16 {
            buf.data[off..off + 16].copy_from_slice(&[0; 16]);
            buf.relocs
                .retain(|r| (r.0 as usize) < off || (r.0 as usize) >= off + 12);
        }
    }
    for b in out.vars.iter_mut().chain(out.chunks.iter_mut()) {
        // Pointers that ended up null (a render list, a cleared pool) need no translation.
        let data = &b.data;
        b.relocs
            .retain(|&(o, _)| data[o as usize..o as usize + 4] != [0; 4]);
        b.relocs.sort_unstable();
        b.relocs.dedup();
    }
    let _ = layout_size;
    Ok(out)
}

pub(super) fn note_unresolved(out: &mut Walked, r: &Region, a: u32, v: u32) {
    out.unresolved_count += 1;
    if out.unresolved.len() < 200 {
        let what = match r.slot {
            Slot::Var(i) => VARS[i].0.to_string(),
            Slot::Chunk(c) => format!("chunk@{:#x}", c),
        };
        out.unresolved.push((
            format!(
                "{what} [region {} layout {} ({}) at +{:#x}]",
                r.slot_desc(),
                r.layout,
                layout_names(r.layout),
                r.off
            ),
            a - r.addr,
            v,
        ));
    }
}

/// The malloc blocks that are part of `mem`'s game state (address, usable
/// length): what a state loaded over it can reuse.
pub fn game_blocks(mem: &[u8], build: Build) -> Result<Vec<(u32, u32)>, String> {
    let w = walk_unchecked(mem, build)?;
    Ok(w.chunk_addrs.into_iter().zip(w.chunk_lens).collect())
}
