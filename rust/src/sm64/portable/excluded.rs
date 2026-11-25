//! What is not game state, and states that cannot be translated.
//!
//! Some memory a variable reaches is rebuilt every frame (display lists,
//! render walk matrices) or belongs to one build (its renderer); it is left out
//! of a state, or zeroed in it, so both builds export the same bytes for the
//! same game and a load does not overwrite what the target needs:
//!
//! - the display list pool (`gGfxAllocOnlyPool`'s blocks, [`draw_pools`]):
//!   not followed, its header zeroed; the rendering target frees its old blocks
//!   after a load ([`display_list_blocks`]);
//! - render-list pointers (`DisplayListNode *`) and `gMatStackFixed` (pointers
//!   into the frame's display lists, which the rendering build leaves set and
//!   the headless one clears): zeroed (`rules.rs`, `relocate.rs`);
//! - `gDisplayListHeap`: never ghosted;
//! - the headless build's extra `AllocOnlyPool` field (+12, a draw-arena
//!   flag): zeroed in every pool; a headless load sets it again in the display
//!   list pool, which it also empties (`load.rs`);
//! - goddard (the intro's Mario head): its object graph has no types, so a
//!   state taken while it runs is refused ([`unsupported`]);
//! - by the tables (`maketools/gen_portable_tables.py`): audio, goddard, libc,
//!   dlmalloc, SDL, GL and `src/pc` variables are not in `VARS` at all; the
//!   rendering build's renderer `.bss` (`tables::RENDERER`) is kept by a load.
//!
//! Read next: `plausible.rs`.

use super::builds::{Build, var_addr};
use super::heap::{heap_chunks, read_u32};
use super::tables::*;

/// The pools the render walk draws into (`gGfxAllocOnlyPool`, `gDisplayListHeap`):
/// not state, rebuilt every frame. Their addresses in `mem`.
pub fn draw_pools(mem: &[u8], build: Build) -> Vec<u32> {
    [":gGfxAllocOnlyPool"]
        .iter()
        .filter_map(|n| {
            let i = VARS.iter().position(|v| v.0.ends_with(n))?;
            read_u32(mem, var_addr(build, i).0).filter(|&p| p != 0)
        })
        .collect()
}

/// Why a state cannot be made portable, if it cannot: goddard (the intro's
/// Mario head and the like) keeps its own object graph in malloc blocks,
/// whose pointers are not described by types, so a state taken while it is
/// running does not translate. Its pool is only allocated while it runs.
pub fn unsupported(mem: &[u8], build: Build) -> Option<String> {
    let pool_at = |name: &str| {
        let i = VARS.iter().position(|v| v.0.ends_with(name))?;
        read_u32(mem, var_addr(build, i).0)
    };
    // The variable keeps pointing at the pool after it is freed, so look for
    // the pool in the main pool's list of blocks.
    let pool = pool_at(":sMemPoolForGoddard")?;
    if pool == 0 {
        return None;
    }
    let mut b = pool_at(":sPoolListHeadL")?;
    for _ in 0..100_000 {
        if b == 0 {
            return None;
        }
        if b + 12 == pool {
            return Some(
                "goddard (the intro's Mario head) is running; its state has no portable form"
                    .to_string(),
            );
        }
        b = read_u32(mem, b)?;
    }
    None
}

/// The blocks of the display list pool (the rendering build's, which draw
/// from malloc): they are not part of a state, but loading one drops the
/// pool's list of them, so whoever loads it has to free them. Empty in the
/// headless build, whose pool is in an arena.
pub fn display_list_blocks(mem: &[u8], build: Build) -> Result<Vec<u32>, String> {
    let chunks = heap_chunks(mem, build)?;
    let mut out = Vec::new();
    let is_block = |b: u32| chunks.binary_search_by(|c| c.user.cmp(&b)).is_ok();
    for pool in draw_pools(mem, build) {
        let mut b = read_u32(mem, pool).unwrap_or(0);
        while b != 0 && is_block(b) && out.len() < 100_000 {
            out.push(b);
            b = read_u32(mem, b).unwrap_or(0);
        }
    }
    Ok(out)
}
