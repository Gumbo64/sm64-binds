//! A memory image of one build, and what an address in it points to.
//!
//! [`Image`] is a build's linear memory (at least up to its heap's top) plus
//! its dlmalloc chunks. [`Image::classify`] is the one question every later
//! step asks of a 32-bit value: is it null, inside a malloc block that is in
//! use, a global or constant of the game, or none of those? It answers from
//! the address alone; whether the value *is* a pointer is decided by the
//! layouts (`walk.rs`) and the rules (`rules.rs`, `plausible.rs`).
//!
//! Read next: `types.rs` and `regions.rs` (how layouts are applied), then
//! `rules.rs`.

use super::addresses::to_canonical_global;
use super::builds::Build;
use super::heap::{Chunk, heap_all, heap_chunks, read_u32};

/// What a value would point to, read as an address.
pub(super) enum PointsTo {
    Null,
    /// Into in-use chunk `.0` (an index into [`Image::chunks`]), at offset `.1`.
    Heap(usize, u32),
    /// A global or constant, at this portable (headless) address.
    Global(u32),
    /// Not null, in no block in use, and not a global: freed memory, the
    /// stack, a build-specific global, or not an address at all.
    Unresolved,
}

pub(super) struct Image<'a> {
    pub(super) build: Build,
    pub(super) mem: &'a [u8],
    /// The malloc blocks in use, by address.
    pub(super) chunks: Vec<Chunk>,
    /// Every chunk of the heap (address, size, in use), by address.
    pub(super) all: Vec<(u32, u32, bool)>,
}

impl<'a> Image<'a> {
    pub(super) fn new(mem: &'a [u8], build: Build) -> Result<Self, String> {
        let all = heap_all(mem, build)?;
        let chunks = heap_chunks(mem, build)?;
        Ok(Image {
            build,
            mem,
            chunks,
            all,
        })
    }

    /// The word at `a`.
    pub(super) fn read(&self, a: u32) -> Option<u32> {
        read_u32(self.mem, a)
    }

    /// The in-use chunk holding `v`, if one does.
    pub(super) fn chunk_of(&self, v: u32) -> Option<usize> {
        let i = self.chunks.partition_point(|c| c.user <= v);
        let c = self.chunks.get(i.checked_sub(1)?)?;
        (v - c.user < c.len).then_some(i - 1)
    }

    pub(super) fn classify(&self, v: u32) -> PointsTo {
        if v == 0 {
            return PointsTo::Null;
        }
        if let Some(ci) = self.chunk_of(v) {
            return PointsTo::Heap(ci, v - self.chunks[ci].user);
        }
        match to_canonical_global(self.build, v) {
            Some(g) => PointsTo::Global(g),
            None => PointsTo::Unresolved,
        }
    }

    /// The free chunk (address, size) holding `v`, if `v` is in one: where a
    /// stale pointer points (see `ghosts.rs`).
    pub(super) fn free_chunk_of(&self, v: u32) -> Option<(u32, u32)> {
        let i = self.all.partition_point(|c| c.0 <= v);
        let &(c, size, in_use) = self.all.get(i.checked_sub(1)?)?;
        (!in_use && v >= c + 8 && v < c + size).then_some((c, size))
    }
}
