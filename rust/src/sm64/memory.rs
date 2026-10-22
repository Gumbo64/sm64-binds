//! A game's linear memory: its layout, reading it, and copying it between
//! games and save states.

use std::ops::Range;

use anyhow::{Error, Result};

use super::SM64Game;
use super::wasm_module::DataSegments;

/// Where a game's linear memory is constant, read from its WASM.
///
/// Linear memory is not one mutable region but two, with the read-only data
/// between them. For the build this crate ships:
///
/// ```text
///   0x000000 ┌────────────────────────┐
///            │ stack      (mutable)   │  <- emscripten stack, rewritten every frame
///   0x010000 ├────────────────────────┤
///            │ .rodata    (constant)  │  <- excluded from save states and copies
///   0x7b4620 ├────────────────────────┤
///            │ .data, .bss and heap   │  <- mutable, most of the save state
///            └────────────────────────┘
/// ```
///
/// wasm-ld writes `.rodata` as the first data segment and `.data` as the
/// next, so the read-only range is the first segment's. With fewer than two
/// segments it cannot tell them apart and treats all of memory as mutable.
/// `tests/save_state_ranges.rs` and `tests/lockstep.rs` check the range never
/// changes while playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MemoryLayout {
    read_only_start: usize,
    read_only_end: usize,
}

impl MemoryLayout {
    pub(super) fn from_data(data: &DataSegments) -> MemoryLayout {
        match data.as_slice() {
            [(address, bytes), _, ..] => MemoryLayout {
                read_only_start: *address,
                read_only_end: address + bytes.len(),
            },
            _ => MemoryLayout {
                read_only_start: 0,
                read_only_end: 0,
            },
        }
    }

    /// The bytes before the read-only data: the stack.
    pub(super) fn stack_len(&self) -> usize {
        self.read_only_start
    }

    /// The mutable regions, as half-open `[start, end)` offsets. The second
    /// ends at `usize::MAX`, to be clamped to the size of linear memory.
    pub(super) fn mutable_ranges(&self) -> [(usize, usize); 2] {
        [(0, self.read_only_start), (self.read_only_end, usize::MAX)]
    }
}

impl SM64Game {
    /// The part of linear memory that holds the game's constant data
    /// (`.rodata`), which never changes while playing. Save states and copies
    /// leave it out: they are the bytes before it (the stack) and from its
    /// end onwards. Read from the WASM, so it follows rebuilds.
    pub fn read_only_memory(&self) -> Range<usize> {
        self.layout.read_only_start..self.layout.read_only_end
    }

    /// Linear memory's address and size, valid until it grows or the game is
    /// next used mutably.
    pub(super) fn mem(&self) -> (*mut u8, usize) {
        let view = self.exports.memory.view(&self.store);
        (view.data_ptr(), view.data_size() as usize)
    }

    /// Like [`SM64Game::mem`], for writing through: counts as a change to the
    /// game's memory.
    pub(super) fn mem_mut(&mut self) -> (*mut u8, usize) {
        self.sync.touch();
        self.mem()
    }

    /// Calls `f` with all of linear memory, without copying it.
    pub fn with_memory<R>(&self, f: impl FnOnce(&[u8]) -> R) -> R {
        let (ptr, len) = self.mem();
        // SAFETY: valid for `len` bytes while `self` is borrowed, which
        // outlasts the call; nothing can change it meanwhile.
        f(unsafe { std::slice::from_raw_parts(ptr, len) })
    }

    /// Copies `buf.len()` bytes of linear memory starting at `addr` into `buf`.
    pub fn read_memory(&self, addr: u32, buf: &mut [u8]) -> Result<()> {
        self.with_memory(|memory| {
            let source = (addr as usize)
                .checked_add(buf.len())
                .and_then(|end| memory.get(addr as usize..end))
                .ok_or_else(|| {
                    Error::msg(format!(
                        "read of {} bytes at {addr:#x} out of bounds",
                        buf.len()
                    ))
                })?;
            buf.copy_from_slice(source);
            Ok(())
        })
    }

    /// Grows linear memory so `heap_len` bytes fit after the read-only data.
    fn expand_memory(&mut self, heap_len: usize) -> Result<(), Error> {
        self.expand_memory_to((self.layout.read_only_end + heap_len) as u64)
    }

    fn expand_memory_to(&mut self, total_bytes: u64) -> Result<(), Error> {
        if self.mem().1 as u64 >= total_bytes {
            return Ok(());
        }
        let (store, exports) = self.parts_mut();
        exports.memory.grow_at_least(store, total_bytes)?;

        // grow_at_least rounds up to a page, but a runtime may refuse growth
        // entirely; re-check so no caller writes out of bounds.
        let actual = self.mem().1 as u64;
        if actual < total_bytes {
            return Err(Error::msg(format!(
                "could not grow linear memory to {total_bytes} bytes (got {actual})"
            )));
        }
        Ok(())
    }

    /// The mutable regions of `memory` (this game's): the stack before the
    /// read-only data, and everything after it.
    pub(super) fn mutable_parts<'m>(&self, memory: &'m [u8]) -> (&'m [u8], &'m [u8]) {
        let len = memory.len();
        (
            &memory[..self.layout.read_only_start.min(len)],
            &memory[self.layout.read_only_end.min(len)..],
        )
    }

    /// Writes mutable regions as [`SM64Game::mutable_parts`] lays them out:
    /// the stack, then the heap. A short slice leaves the rest untouched.
    pub(super) fn write_mutable_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // Size the destination for the largest state it might hold, then place
        // the ranges. A short slice leaves the trailing range untouched.
        let stack_len = self.layout.stack_len();
        let stack = bytes.len().min(stack_len);
        let heap = bytes.len().saturating_sub(stack_len);
        self.expand_memory(heap)?;

        let (ptr, _) = self.mem_mut();

        // SAFETY: `expand_memory` verified linear memory reaches
        // `read_only_end + heap`, which is past `stack <= read_only_start`.
        // `bytes` is a separate allocation, so neither copy can overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, stack);
            if heap > 0 {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr().add(stack_len),
                    ptr.add(self.layout.read_only_end),
                    heap,
                );
            }
        }
        Ok(())
    }

    /// Makes this game a copy of `source`: copies its mutable regions, without
    /// materialising a save state in between.
    ///
    /// If you repeatedly load from a previous state (like a tree search), [`SM64Game::sync_from`]
    /// is faster: it copies back only the pages that have changed.
    ///
    /// Fails if `source` is a different build of the game.
    pub fn copy_from(&mut self, source: &SM64Game) -> Result<(), Error> {
        if source.build_id != self.build_id || source.layout != self.layout {
            return Err(Error::msg(
                "copy_from: the source game is a different build of the game",
            ));
        }
        let (src_ptr, src_size) = source.mem();

        // This game has to be at least as large as the source's mutable
        // ranges. The stack needs no growth, only the heap does. Growing can
        // move this game's base pointer, so its view is taken after.
        let needed = src_size.saturating_sub(self.layout.read_only_end);
        self.expand_memory(needed)?;

        let (dst_ptr, dst_size) = self.mem_mut();

        for (start, end) in self.layout.mutable_ranges() {
            let src_end = end.min(src_size);
            let dst_end = end.min(dst_size);
            let len = src_end.min(dst_end).saturating_sub(start);
            if len == 0 {
                continue;
            }
            // SAFETY: `len` bytes are readable from `start` in the source and
            // `expand_memory` guaranteed the same range is writable here. The
            // two memories are distinct allocations, so the ranges cannot
            // overlap.
            unsafe {
                std::ptr::copy_nonoverlapping(src_ptr.add(start), dst_ptr.add(start), len);
            }
        }
        Ok(())
    }
}
