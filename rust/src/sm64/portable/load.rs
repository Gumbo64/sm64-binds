//! Loading a portable state into a build's memory.
//!
//! A load is three calls, so that the caller (Rust `SM64Game::load_portable`,
//! or JavaScript through `rust/portable-wasm`) does the allocating with the
//! target game's own `malloc`:
//!
//! 1. [`Portable::place`]: which of the target's own game blocks (from
//!    `game_blocks`) each state block reuses, best fit first; 0 for the rest,
//!    which the caller allocates ([`Portable::chunk_sizes`]). Ghosts are always
//!    allocated, and never freed.
//! 2. [`Portable::patches`]: the bytes to write, every relocation translated
//!    to the target: a heap pointer to the target address of its block, a
//!    global through `RUNS`, a function through `FNS`. A pointer with no
//!    translation becomes null and is counted ([`Portable::untranslatable`]
//!    lists them). The display list pool is emptied, and marked as the draw
//!    arena's in a headless target.
//! 3. The caller writes the patches, then resets what the target caches about
//!    the old state: the headless render walk cache (`Portable::geo_cache`), or
//!    the rendering build's texture cache, its display list blocks, and its
//!    renderer state (`Portable::renderer`, kept across the load).
//!
//! Guarantees: `patches` only reads the state and the addresses given; a load
//! needs no type information.
//!
//! Read next: `compare.rs`, or `mod.rs` for the public API.

use super::addresses::{HEAP_BASE, fn_from_canonical, from_canonical_global};
use super::builds::Build;
use super::state::{Buf, Patch, Portable};
use super::tables::VARS;

impl Portable {
    /// The bytes of the ghosts: what a load leaks.
    pub fn ghost_bytes(&self) -> usize {
        self.chunks[self.chunks.len() - self.ghosts..]
            .iter()
            .map(|c| c.data.len())
            .sum()
    }

    /// The malloc sizes to allocate in the target, one per block, in order.
    pub fn chunk_sizes(&self) -> Vec<u32> {
        self.chunks.iter().map(|c| c.data.len() as u32).collect()
    }

    /// Where each block of this state goes in the target: the target's own
    /// game blocks (`old`, from [`game_blocks`]) are reused, best fit first
    /// (a block that is freed later is freed whole, so reusing one needs no
    /// allocator call); 0 for the blocks that fit none, which the caller must
    /// allocate (`malloc` of the size from [`Portable::chunk_sizes`]).
    pub fn place(&self, old: &[(u32, u32)]) -> Vec<u32> {
        let mut free: Vec<(u32, u32)> = old.iter().map(|&(a, l)| (l, a)).collect();
        free.sort_unstable();
        // Ghosts are always allocated: the game never frees them.
        let mut order: Vec<usize> = (0..self.chunks.len() - self.ghosts).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.chunks[i].data.len()));
        let mut addrs = vec![0; self.chunks.len()];
        for i in order {
            let need = self.chunks[i].data.len() as u32;
            let at = free.partition_point(|f| f.0 < need);
            if at < free.len() {
                addrs[i] = free.remove(at).1;
            }
        }
        addrs
    }

    fn pointer_to(&self, target: Build, v: u32, addrs: &[u32], bad: &mut u32) -> u32 {
        if v >= HEAP_BASE {
            let rel = v - HEAP_BASE;
            let i = self.chunk_bases.partition_point(|&b| b <= rel);
            if let Some(i) = i.checked_sub(1)
                && let Some(&a) = addrs.get(i)
            {
                return a + (rel - self.chunk_bases[i]);
            }
        } else if let Some(a) = from_canonical_global(target, v) {
            return a;
        }
        *bad += 1;
        0
    }

    fn translated(&self, target: Build, b: &Buf, addrs: &[u32], bad: &mut u32) -> Vec<u8> {
        let mut data = b.data.clone();
        for &(o, kind) in &b.relocs {
            let o = o as usize;
            let v = u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
            let t = if kind == 1 {
                fn_from_canonical(target, v).unwrap_or_else(|| {
                    *bad += 1;
                    0
                })
            } else {
                self.pointer_to(target, v, addrs, bad)
            };
            data[o..o + 4].copy_from_slice(&t.to_le_bytes());
        }
        data
    }

    /// The pointers this state has that `target` has no equivalent of, as (variable or
    /// block, offset, portable value): what [`Portable::patches`] counts and nulls.
    pub fn untranslatable(&self, target: Build, chunk_addrs: &[u32]) -> Vec<(String, u32, u32)> {
        let mut out = Vec::new();
        let mut one = |name: String, b: &Buf| {
            for &(o, kind) in &b.relocs {
                let v = u32::from_le_bytes(b.data[o as usize..o as usize + 4].try_into().unwrap());
                let mut bad = 0;
                if kind == 1 {
                    if fn_from_canonical(target, v).is_none() {
                        bad = 1;
                    }
                } else {
                    self.pointer_to(target, v, chunk_addrs, &mut bad);
                }
                if bad > 0 {
                    out.push((name.clone(), o, v));
                }
            }
        };
        for (i, b) in self.vars.iter().enumerate() {
            one(VARS[i].0.to_string(), b);
        }
        for (i, b) in self.chunks.iter().enumerate() {
            one(format!("block {i}"), b);
        }
        out
    }

    /// What to write into `target`'s memory to load this state, once the
    /// blocks have been allocated there (`chunk_addrs[i]` holds
    /// [`Portable::chunk_sizes`]`()[i]` bytes). Also returns how many
    /// pointers had no translation (set to null).
    pub fn patches(&self, target: Build, chunk_addrs: &[u32]) -> Result<(Vec<Patch>, u32), String> {
        if chunk_addrs.len() != self.chunks.len() {
            return Err("wrong number of block addresses".into());
        }
        let mut bad = 0;
        let mut out: Vec<Patch> = Vec::new();
        for (i, b) in self.vars.iter().enumerate() {
            if b.data.is_empty() {
                continue;
            }
            let a = if target == Build::Headless {
                VARS[i].1
            } else {
                VARS[i].2
            };
            out.push((a, self.translated(target, b, chunk_addrs, &mut bad)));
        }
        for (i, b) in self.chunks.iter().enumerate() {
            out.push((
                chunk_addrs[i],
                self.translated(target, b, chunk_addrs, &mut bad),
            ));
        }
        // The display list pools start empty (their blocks are not state), and the
        // headless build draws into an arena of its own: its pools are marked
        // (`struct AllocOnlyPool`'s extra headless field, +12, set to 1).
        for name in [":gGfxAllocOnlyPool"] {
            let Some(i) = VARS.iter().position(|v| v.0.ends_with(name)) else {
                continue;
            };
            let at = if target == Build::Headless {
                VARS[i].1
            } else {
                VARS[i].2
            };
            let Some((_, d)) = out.iter().find(|p| p.0 == at) else {
                continue;
            };
            let p = u32::from_le_bytes(d[..4].try_into().unwrap());
            let inside = chunk_addrs
                .iter()
                .zip(&self.chunks)
                .any(|(&a, c)| p >= a && p + 16 <= a + c.data.len() as u32);
            if p != 0 && inside {
                out.push((p, vec![0; 12]));
                if target == Build::Headless {
                    out.push((p + 12, 1u32.to_le_bytes().to_vec()));
                }
            }
        }
        Ok((out, bad))
    }
}
