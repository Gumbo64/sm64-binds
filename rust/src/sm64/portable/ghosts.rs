//! Ghosts: what stale pointers into freed memory keep reading.
//!
//! A pointer can outlive what it points to (an object's floor after the
//! surface pool was freed). The original game keeps reading the old bytes,
//! since freed memory is untouched until malloc reuses it, and it may
//! dereference the pointer at any time. Freed memory is not state, so a loaded
//! game would read other bytes there. For every typed pointer (to a struct we
//! know, 4-aligned) and every object pointer slot whose value is inside a
//! *free* dlmalloc chunk, the export copies a window of the bytes there (the
//! struct's size, at least 0x40, 8-aligned, clipped to the free chunk;
//! overlapping windows merged) and appends it to the block list as a **ghost**.
//! The pointers are then translated into the ghost like into any block.
//!
//! Guarantees: a ghost's bytes are never interpreted (no layout, no
//! relocations), a load allocates every ghost and never frees it (a leak of a
//! few hundred bytes, `Portable::ghost_bytes`), and `gDisplayListHeap` and
//! `gMatStackFixed` are never ghosted. Limit: a ghost is a snapshot; if the
//! original game later reuses the freed memory, the stale pointer there sees
//! the new data, and the ghost does not.
//!
//! Format: version 2 of the portable format adds the ghost count (`format.rs`).
//!
//! Read next: `relocate.rs`.

use std::collections::HashMap;

use super::addresses::HEAP_BASE;
use super::image::PointsTo;
use super::layout::K;
use super::regions::{Owner, Region};
use super::rules::{is_display_list_heap, is_number_slot, is_render_list, region_leaves};
use super::state::{Buf, Walked};
use super::tables::VARS;
use super::types::{object_layout, pointee};
use super::walk::Walk;

/// Finds the stale pointers of `regions`, appends their ghosts to `out`'s
/// blocks (the next one at portable offset `*base`), and returns the portable
/// value of each stale pointer, by (owner, offset in the owner).
pub(super) fn add_ghosts(
    w: &Walk,
    regions: &[Region],
    out: &mut Walked,
    base: &mut u32,
) -> HashMap<(Owner, u32), u32> {
    let mem = w.img.mem;
    let obj = object_layout();
    // (owner, offset in it, value, window)
    let mut stale: Vec<(Owner, u32, u32, u32)> = Vec::new();
    for r in regions {
        if w.mat_stack_fixed.is_some_and(|m| r.owner == Owner::Var(m)) {
            continue;
        }
        let mut items: Vec<(u32, K, u16, u32)> = Vec::new();
        region_leaves(r.layout, r.addr, r.count, &mut |a, k, l, o| {
            items.push((a, k, l, o))
        });
        for (a, k, l, o) in items {
            if w.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                continue;
            }
            let Some(v) = w.img.read(a) else { continue };
            if is_display_list_heap(r.owner) {
                continue;
            }
            let window = match k {
                // A struct we know, at an aligned address (a typed region over reused
                // memory holds numbers that look like pointers).
                K::Ptr(pid) if !is_render_list(pid) && v % 4 == 0 => {
                    pointee(pid).map(|p| p.0.max(0x40))
                }
                // Only an object's pointer slots: other words may be numbers.
                K::Word
                    if Some(l) == obj && !is_number_slot(&w.img, obj, l, a, o) && v % 4 == 0 =>
                {
                    Some(0x40)
                }
                _ => None,
            };
            if let Some(win) = window
                && matches!(w.img.classify(v), PointsTo::Unresolved)
                && w.img.free_chunk_of(v).is_some()
            {
                stale.push((r.owner, a - r.addr + r.off, v, win));
            }
        }
    }
    let mut windows: Vec<(u32, u32)> = Vec::new();
    for &(_, _, v, win) in &stale {
        let (c, size) = w.img.free_chunk_of(v).unwrap();
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
    let mut ghost_bases = Vec::new();
    for &(s0, e0) in &ghosts {
        ghost_bases.push(*base);
        out.chunk_addrs.push(s0);
        out.chunk_lens.push(e0 - s0);
        out.chunk_bases.push(*base);
        out.chunks.push(Buf {
            data: mem[s0 as usize..e0 as usize].to_vec(),
            relocs: Vec::new(),
        });
        *base += (e0 - s0 + 15) & !15;
    }
    out.ghosts = ghosts.len();
    let mut ghost_val: HashMap<(Owner, u32), u32> = HashMap::new();
    for &(owner, off, v, _) in &stale {
        let gi = ghosts.partition_point(|g| g.0 <= v) - 1;
        ghost_val.insert(
            (owner, off),
            HEAP_BASE + ghost_bases[gi] + (v - ghosts[gi].0),
        );
        let what = match owner {
            Owner::Var(i) => VARS[i].0.to_string(),
            Owner::Chunk(c) => format!("block@{:#x}", w.img.chunks[c].user),
        };
        out.ghost_refs.push((what, off, v));
    }
    ghost_val
}
