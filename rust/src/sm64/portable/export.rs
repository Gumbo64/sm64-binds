//! The export pipeline: a memory image in, a [`Portable`] state out.
//!
//! ```text
//! unsupported? (excluded.rs) -> find_state (walk.rs) -> emit (relocate.rs, ghosts.rs) -> Walked -> Portable
//! ```
//!
//! Guarantees: an export only reads `mem`, and the same image always gives the
//! same state, byte for byte. It fails (rather than giving a wrong state) on a
//! heap that does not walk, a variable outside memory, or a state that has no
//! portable form (goddard running).
//!
//! Read next: `format.rs`.

use super::builds::Build;
use super::excluded::unsupported;
use super::relocate;
use super::state::{Portable, Walked};
use super::walk::find_state;

/// Finds the state in `mem`: the variables, and every malloc block reachable
/// from them by a pointer, in portable form.
pub fn walk(mem: &[u8], build: Build) -> Result<Walked, String> {
    if let Some(why) = unsupported(mem, build) {
        return Err(why);
    }
    walk_unchecked(mem, build)
}

/// [`walk`] of a state that may be unsupported: what the game's own blocks
/// are, to reuse them.
pub(super) fn walk_unchecked(mem: &[u8], build: Build) -> Result<Walked, String> {
    let (w, exact) = find_state(mem, build)?;
    Ok(relocate::emit(w, &exact))
}

/// The malloc blocks that are part of `mem`'s game state (address, usable
/// length): what a state loaded over it can reuse.
pub fn game_blocks(mem: &[u8], build: Build) -> Result<Vec<(u32, u32)>, String> {
    let w = walk_unchecked(mem, build)?;
    Ok(w.chunk_addrs.into_iter().zip(w.chunk_lens).collect())
}

/// Translates the state in `mem` (a copy of the linear memory of a `build`
/// game, at least up to its heap's top) to portable form.
pub fn export(mem: &[u8], build: Build) -> Result<Portable, String> {
    let w = walk(mem, build)?;
    Ok(Portable {
        source: build,
        vars: w.vars,
        chunks: w.chunks,
        chunk_bases: w.chunk_bases,
        ghosts: w.ghosts,
        ghost_refs: w.ghost_refs,
        unresolved: w.unresolved,
        unresolved_count: w.unresolved_count,
        found_by: w.found_by,
    })
}
