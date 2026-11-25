//! Portable save states: the game's state in a form either build can load.
//!
//! The headless WASM (`build/us_web_headless`, what Rust, Python and the nodes
//! run) and the rendering WASM (`build/us_web`, what the browser runs) are
//! linked from the same objects, but at different addresses, and their
//! mallocs hand out different blocks. A [`Portable`] state holds the game's
//! variables and the malloc blocks they reach, with every pointer and
//! function pointer replaced by a build-neutral one, so it loads in either
//! build and the game goes on identically. See `docs/portable-state.md`.
//!
//! This module only needs `std`: the same source is compiled to WASM for the
//! browser (`rust/portable-wasm`).

mod addresses;
mod builds;
mod compare;
mod excluded;
mod format;
mod heap;
mod layout;
mod load;
mod plausible;
mod rules;
#[rustfmt::skip]
mod tables;
mod types;
mod walk;

pub use builds::Build;
pub use excluded::{display_list_blocks, unsupported};
pub use heap::{heap_all, heap_chunks};
use tables::VARS;
pub use walk::{Buf, Walked, game_blocks, walk};

/// The game's state, translated to build-neutral form by [`export`].
#[derive(Debug, Clone)]
pub struct Portable {
    /// The build the state was exported from.
    pub source: Build,
    pub vars: Vec<Buf>,
    pub chunks: Vec<Buf>,
    pub chunk_bases: Vec<u32>,
    /// How many of the last `chunks` are ghosts: raw copies of freed memory that
    /// stale pointers point into. They are allocated and written on a load and
    /// never freed; no type walk touches them.
    pub ghosts: usize,
    /// The stale pointers translated into ghosts (variable or block, offset, original value).
    pub ghost_refs: Vec<(String, u32, u32)>,
    /// Pointers that had no translation and were set to null.
    pub unresolved: Vec<(String, u32, u32)>,
    pub unresolved_count: u32,
    /// Where each block was first found (for diagnostics).
    pub found_by: Vec<(String, u32)>,
}

/// A group of bytes to write into the target's memory.
pub type Patch = (u32, Vec<u8>);

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

impl Portable {
    /// Where the headless build's render walk cache is, for invalidating it
    /// after a load: (generation address, count address).
    pub fn geo_cache() -> [u32; 2] {
        tables::GEO_CACHE
    }

    /// Where constant data ends in `build` (its `.data` starts): memory below
    /// it is the stack and read-only data, which no state includes.
    pub fn data_start(build: Build) -> u32 {
        tables::DATA_STARTS[build.index()]
    }

    /// The rendering build's texture cache (address, size), to reset after a
    /// state is loaded: it maps texture addresses to GL textures.
    pub fn texture_cache() -> (u32, u32) {
        tables::TEXTURE_CACHE
    }

    /// The rendering build's renderer state (address, size): what it knows of
    /// the live GL context (the GL state it mirrors, shader programs, the
    /// texture cache, the window's size). It is not part of the game, so a
    /// load keeps it, whatever the loaded memory held there.
    pub fn renderer() -> (u32, u32) {
        tables::RENDERER
    }

    /// The addresses of the allocator's state, per build.
    pub fn gm(build: Build) -> u32 {
        tables::GM[build.index()]
    }

    /// The address and size of variable `i` in `build`'s memory.
    pub fn var_at(i: usize, build: Build) -> (u32, u32) {
        let v = &VARS[i];
        (if build == Build::Headless { v.1 } else { v.2 }, v.3)
    }

    /// How many variables a state has.
    pub const VAR_COUNT: usize = VARS.len();

    /// Names of the variables, for diagnostics.
    pub fn var_name(i: usize) -> &'static str {
        VARS[i].0
    }
}
