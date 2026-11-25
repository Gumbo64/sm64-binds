//! Relocation: turning what the walk found into portable bytes.
//!
//! [`emit`] copies every variable and every reached block, then rewrites each
//! pointer word of each region to its portable value and records it as a
//! relocation:
//!
//! 1. block `i` of the state gets portable offset `chunk_bases[i]` (the blocks
//!    before it, each rounded up to 16 bytes), ghosts after the reached blocks;
//! 2. a word that is a stale pointer becomes a pointer into its ghost (`ghosts.rs`);
//! 3. a `K::Ptr` word: null stays null, a heap pointer becomes `HEAP_BASE +
//!    chunk_bases[block] + offset`, a global its headless address; one with no
//!    translation is nulled and counted (`unresolved`);
//! 4. a `K::Word` word: translated the same way only if `rules.rs` says it is a
//!    pointer, else left as it is;
//! 5. a `K::Fn` word: the headless function table index (nulled and counted if
//!    the function has no counterpart);
//! 6. not state, and so zeroed: render-list pointers, `gMatStackFixed`, the
//!    headless build's extra `AllocOnlyPool` field, the display list pool's
//!    header (`excluded.rs` says why).
//!
//! Guarantees: the result only depends on `mem` (the same image gives the
//! same bytes); every relocation is at a non-zero word; relocations are sorted
//! and unique.
//!
//! Read next: `format.rs` (how this is written down), then `load.rs`.

use super::addresses::{HEAP_BASE, fn_to_canonical};
use super::builds::var_addr;
use super::diagnostics::note_unresolved;
use super::excluded::draw_pools;
use super::ghosts::add_ghosts;
use super::image::PointsTo;
use super::layout::K;
use super::regions::Owner;
use super::rules::{
    is_number_slot, is_render_list, may_point_to_constants, region_leaves, union_word_target,
};
use super::state::{Buf, Walked};
use super::tables::VARS;
use super::types::{object_layout, pool_layout};
use super::walk::{ExactLink, Walk};

/// The portable form of what `w` found (`exact`: the pool links it followed).
pub(super) fn emit(mut w: Walk, exact: &[ExactLink]) -> Walked {
    let mem = w.img.mem;
    let build = w.img.build;

    // Output buffers: the bytes as they are.
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
        let c = w.img.chunks[ci];
        out.chunk_addrs.push(c.user);
        out.chunk_lens.push(c.len);
        out.chunk_bases.push(base);
        out.chunks.push(Buf {
            data: mem[c.user as usize..(c.user + c.len) as usize].to_vec(),
            relocs: Vec::new(),
        });
        base += (c.len + 15) & !15;
    }
    for &(ci, o, ti, toff) in exact {
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
            Owner::Var(i) => (VARS[i].0.to_string(), o),
            Owner::Chunk(c) => (format!("block@{:#x}", w.img.chunks[c].user), o),
        })
        .collect();
    // Diagnostics: GraphNode pointers the walk could not type.
    out.bad_nodes = std::mem::take(&mut w.bad_nodes);
    let obj = object_layout();
    let pool = pool_layout();
    if let Some(m) = w.mat_stack_fixed {
        // Render walk matrices: pointers into per-frame memory, not state.
        out.vars[m].data.fill(0);
    }

    let ghost_val = add_ghosts(&w, &regions, &mut out, &mut base);

    for r in &regions {
        if w.mat_stack_fixed.is_some_and(|m| r.owner == Owner::Var(m)) {
            continue;
        }
        let mut items: Vec<(u32, K, u16, u32)> = Vec::new();
        region_leaves(r.layout, r.addr, r.count, &mut |a, k, l, o| {
            items.push((a, k, l, o))
        });
        for (a, k, l, o) in items {
            if w.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                // the display list pool (zeroed below)
                continue;
            }
            let Some(v) = w.img.read(a) else { continue };
            if let K::Ptr(pid) = k
                && is_render_list(pid)
            {
                let off = (a - r.addr + r.off) as usize;
                let buf = match r.owner {
                    Owner::Var(i) => &mut out.vars[i],
                    Owner::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
                };
                buf.data[off..off + 4].copy_from_slice(&[0; 4]);
                continue;
            }
            let ghost = ghost_val
                .get(&(r.owner, a - r.addr + r.off))
                .map(|&g| (g, 0u8));
            let canon: Option<(u32, u8)> = match k {
                _ if ghost.is_some() => ghost,
                K::Ptr(_) => match w.img.classify(v) {
                    PointsTo::Null => None,
                    PointsTo::Heap(ci, off) => w
                        .reached
                        .get(&ci)
                        .map(|&bi| (HEAP_BASE + out.chunk_bases[bi] + off, 0)),
                    PointsTo::Global(g) => Some((g, 0)),
                    PointsTo::Unresolved => {
                        note_unresolved(&mut out, &w.img, r, a, v);
                        None
                    }
                },
                K::Word if is_number_slot(&w.img, obj, l, a, o) => None,
                K::Word => match union_word_target(&w.img, v, may_point_to_constants(obj, l, o)) {
                    PointsTo::Heap(ci, off) => w
                        .reached
                        .get(&ci)
                        .map(|&bi| (HEAP_BASE + out.chunk_bases[bi] + off, 0)),
                    PointsTo::Global(g) => Some((g, 0)),
                    _ => None,
                },
                K::Fn => {
                    if v == 0 {
                        None
                    } else {
                        match fn_to_canonical(build, v) {
                            Some(c) => Some((c, 1)),
                            None => {
                                note_unresolved(&mut out, &w.img, r, a, v);
                                None
                            }
                        }
                    }
                }
                _ => None,
            };
            let Some((val, kind)) = canon else { continue };
            let off = a - r.addr + r.off;
            let buf = match r.owner {
                Owner::Var(i) => &mut out.vars[i],
                Owner::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
            };
            buf.data[off as usize..off as usize + 4].copy_from_slice(&val.to_le_bytes());
            buf.relocs.push((off, kind));
        }
        // The headless build's `struct AllocOnlyPool` has one more field (+12, its
        // draw arena flag): not state, zeroed so both builds export the same bytes.
        if Some(r.layout) == pool {
            let off = r.off as usize;
            let buf = match r.owner {
                Owner::Var(i) => &mut out.vars[i],
                Owner::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
            };
            if buf.data.len() >= off + 16 {
                buf.data[off + 12..off + 16].copy_from_slice(&[0; 4]);
            }
        }
    }
    // The display list pool is rebuilt every frame; its blocks are not state
    // (and live in different memory in the two builds).
    for p in draw_pools(mem, build) {
        let Some(ci) = w.img.chunk_of(p) else {
            continue;
        };
        let Some(&bi) = w.reached.get(&ci) else {
            continue;
        };
        let off = (p - w.img.chunks[ci].user) as usize;
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
    out
}
