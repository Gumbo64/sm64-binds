//! What an export produces: byte buffers with relocations.
//!
//! A [`Buf`] is the bytes of one variable or one malloc block, already in
//! portable form, plus the offsets of the words in it that are pointers or
//! function pointers ([`Reloc`]). A load needs nothing else: it translates
//! each relocated word to the target build and writes the bytes (`load.rs`).
//! [`Walked`] is everything the export found, with diagnostics; `Portable`
//! keeps the parts that are the state.
//!
//! Read next: `walk.rs`.

use super::builds::Build;

/// A pointer or function pointer in a buffer, by offset: the value there is
/// portable. Kind 0 is a pointer, 1 a function table index.
pub type Reloc = (u32, u8);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buf {
    pub data: Vec<u8>,
    /// Sorted by offset, no duplicates, none at a word that is zero.
    pub relocs: Vec<Reloc>,
}

#[derive(Debug, Clone, Default)]
pub struct Walked {
    /// How many of the last `chunks` are ghosts (raw copies of freed memory
    /// that stale pointers read; see `ghosts.rs`).
    pub ghosts: usize,
    /// The stale pointers made ghost-relative: (variable or block, offset, value).
    pub ghost_refs: Vec<(String, u32, u32)>,
    /// One per entry of `VARS` (even ones that could not be read: empty).
    pub vars: Vec<Buf>,
    /// The malloc blocks reached, in the order they were found, then the
    /// ghosts. Block `i` is at portable address `HEAP_BASE + chunk_bases[i]`.
    pub chunks: Vec<Buf>,
    /// Where each block is in the source memory, and its length.
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

/// The game's state, translated to build-neutral form by `export` (`export.rs`).
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
