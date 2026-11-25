//! Typed regions: a layout applied at an address, inside a variable or a
//! malloc block.
//!
//! The walk (`walk.rs`) starts with one region per variable (its DWARF
//! layout) and adds one for every pointer to a struct it can type. Everything
//! after that (which words are pointers, which get relocated, which become
//! ghosts) works region by region, so a word is only ever a pointer if some
//! layout says it may be one.
//!
//! Read next: `rules.rs`.

use super::tables::VARS;

/// Where a region's bytes live: variable `i` of `VARS`, or in-use chunk `i`
/// of the image (an index into `Image::chunks`, not into the state's block list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Owner {
    Var(usize),
    Chunk(usize),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Region {
    pub(super) addr: u32,
    pub(super) layout: u16,
    /// For [`PTRS`] regions: how many pointers.
    pub(super) count: u32,
    pub(super) owner: Owner,
    /// Offset of `addr` in the owner's bytes.
    pub(super) off: u32,
}

impl Region {
    pub(super) fn owner_desc(&self) -> String {
        match self.owner {
            Owner::Var(i) => VARS[i].0.to_string(),
            Owner::Chunk(c) => format!("chunk {c}"),
        }
    }
}

/// The layout of a run of pointers behind a pointer to pointers (`GraphNode **views`):
/// as many words as hold pointers, up to 64.
pub(super) const PTRS: u16 = 0xFFFE;
