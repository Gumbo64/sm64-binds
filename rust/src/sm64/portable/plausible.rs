//! Plausibility checks.

use super::heap::rd;
use super::tables::*;
use super::types::*;
use super::walk::*;

impl Walk<'_> {
    /// The struct a pointer to a `GraphNode` really points to.
    pub(super) fn refine(&self, name: &str, addr: u32) -> Option<(u32, u16)> {
        if name == "GraphNode" {
            let t = u16::from_le_bytes(
                self.mem
                    .get(addr as usize..addr as usize + 2)?
                    .try_into()
                    .ok()?,
            );
            // Not a node at all (a pointer into the middle of one, or a stale
            // one into memory that has been reused): no struct to apply.
            let &(_, n) = NODE_TYPES.iter().find(|x| x.0 == t)?;
            // Nodes sit in circular doubly linked lists.
            let (prev, next) = (rd(self.mem, addr + 4)?, rd(self.mem, addr + 8)?);
            if prev == 0
                || next == 0
                || rd(self.mem, next + 4)? != addr
                || rd(self.mem, prev + 8)? != addr
            {
                return None;
            }
            return named_by(n);
        }
        named_by(name)
    }

    /// Whether the struct a pointer says it points to is plausibly there: a
    /// stale pointer (an object that was freed, say) can point into memory that
    /// has been reused for something else, and applying a layout to that would
    /// turn numbers into pointers.
    pub(super) fn plausible(&self, name: &str, addr: u32) -> bool {
        match name {
            // The behavior script is always set, and is one of the game's behaviors (an address
            // in constant data is not enough: a terrain copy's s16 pairs, y and 0x17, read as
            // 0x0017yyyy, an address in the rendering build's constant data, and the exporter
            // then translated them as the pointer they are not, changing a vertex).
            // The pointers next to it are null or point to something.
            "SpawnInfo" => {
                let ok = |v: Option<u32>| {
                    v.is_some_and(|v| {
                        matches!(self.classify(v), Tgt::Null | Tgt::Heap(..) | Tgt::Global(_))
                    })
                };
                rd(self.mem, addr + 0x14).is_some_and(|b| match self.classify(b) {
                    Tgt::Global(g) => BEHAVIOR_POINTER_SLOTS
                        .binary_search_by_key(&g, |x| x.0)
                        .is_ok(),
                    _ => false,
                }) && ok(rd(self.mem, addr + 0x18))
                    && ok(rd(self.mem, addr + 0x1c))
            }
            // A pool's last block comes from malloc (or, headless, the draw arena: a global),
            // and it has used no more of it than it has. A stale pointer to a freed pool
            // (sMemPoolForGoddard after the intro) points into memory reused for something
            // else, such as graph nodes, which the pool's layout would then mangle.
            "AllocOnlyPool" => {
                let (Some(last), Some(size), Some(next)) = (
                    rd(self.mem, addr),
                    rd(self.mem, addr + 4),
                    rd(self.mem, addr + 8),
                ) else {
                    return false;
                };
                next <= size
                    && match self.classify(last) {
                        Tgt::Null | Tgt::Global(_) => true,
                        Tgt::Heap(ci, off) => off == 0 && self.chunks[ci].len >= size,
                        Tgt::Unresolved => false,
                    }
            }
            // `gEffectsMemoryPool` (system malloc): an allocation pool first. A stale pointer
            // to a freed pool (the level's pools are released, their memory reused) points
            // at something else, graph nodes say, whose words the pool's layout would turn
            // into pointers (a padding word of a graph node became a pointer).
            "MemoryPool" => rd(self.mem, addr).is_some_and(|p| match self.classify(p) {
                Tgt::Heap(..) => self.plausible("AllocOnlyPool", p),
                _ => false,
            }),
            // Objects live in `gObjectPool`, a global array; one in a heap block is a stale
            // pointer (a platform, a parent) into memory reused for something else, and
            // an Object layout over graph nodes or collision data makes pointers of
            // numbers (and walks into more of the same).
            "Object" => false,
            // A surface's `object` is null or a pool object. A stale floor pointer into reused
            // memory (a level's collision data was freed, a later level's data sits there)
            // reads other numbers there.
            "Surface" => rd(self.mem, addr + 0x2c)
                .is_some_and(|o| matches!(self.classify(o), Tgt::Null | Tgt::Global(_))),
            _ => true,
        }
    }
}
