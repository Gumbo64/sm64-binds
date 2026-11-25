//! Portable save states: the game's state in a form either build can load.
//!
//! The headless WASM (`build/us_web_headless`, what Rust, Python and the nodes
//! run) and the rendering WASM (`build/us_web`, what the browser runs) are
//! linked from the same objects, but at different addresses, and their
//! mallocs hand out different blocks, so a raw memory snapshot of one does not
//! load in the other. A [`Portable`] state holds the game's variables and the
//! malloc blocks they reach, with every pointer and function pointer replaced
//! by a build-neutral one, so it loads in either build and the game goes on
//! identically. The guide is `docs/PORTABLE.md`; the design notes and
//! measurements are in `docs/portable-state.md`.
//!
//! One translation:
//!
//! ```text
//! export:  memory image --heap.rs--> chunks --walk.rs--> regions + reached blocks
//!            --relocate.rs (+ ghosts.rs)--> Portable --format.rs--> bytes
//! load:    bytes --format.rs--> Portable --load.rs (place, patches)--> writes into the target
//! ```
//!
//! Reading order, bottom up (each file starts with what it is, what it
//! guarantees and what to read next):
//!
//! | layer | files |
//! |---|---|
//! | memory layout, generated | `tables.rs` (by `maketools/gen_portable_tables.py`), `layout.rs` |
//! | lookups and addresses | `types.rs`, `builds.rs`, `addresses.rs`, `heap.rs`, `image.rs`, `regions.rs` |
//! | which words are pointers | `rules.rs`, `excluded.rs`, `plausible.rs` |
//! | the walk | `walk.rs` |
//! | relocation | `relocate.rs`, `ghosts.rs`, `state.rs`, `diagnostics.rs`, `export.rs` |
//! | format | `format.rs` |
//! | loading | `load.rs` |
//! | checks | `compare.rs` (digest, diff) |
//! | public API | this file |
//!
//! This module only needs `std`: the same source is compiled to WASM for the
//! browser (`rust/portable-wasm`).

// Layer 1: the memory layout of both builds (generated) and its vocabulary.
mod layout;
#[rustfmt::skip]
mod tables;
// Layer 2: lookups over the tables and over a memory image.
mod addresses;
mod builds;
mod heap;
mod image;
mod regions;
mod types;
// Layer 3: which words are pointers, and what is not state.
mod excluded;
mod plausible;
mod rules;
// Layer 4: finding the state.
mod walk;
// Layer 5: making it portable.
mod diagnostics;
mod export;
mod ghosts;
mod relocate;
mod state;
// Layer 6: the byte format, loading, and comparing states.
mod compare;
mod format;
mod load;

pub use builds::Build;
pub use excluded::{display_list_blocks, unsupported};
pub use export::{export, game_blocks, walk};
pub use heap::{heap_all, heap_chunks};
pub use state::{Buf, Patch, Portable, Walked};
use tables::VARS;

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
