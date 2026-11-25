//! Plausibility checks: is the struct a pointer says it points to really there?
//!
//! A pointer can outlive what it points to (a freed object, a level's pool
//! after the level was unloaded), and malloc reuses the memory for something
//! else. Applying the pointer's struct layout to that memory would read its
//! numbers as pointers: the export would then "relocate" them, and the loaded
//! game would have different numbers there. So before the walk applies a
//! layout behind a pointer into the heap, it checks that the struct is
//! plausibly there. A failed check only means "do not type this memory": the
//! pointer itself is still translated, and the block is still part of the state.
//!
//! Each rule, and the bug it prevents:
//!
//! | struct | rule | bug it prevents |
//! |---|---|---|
//! | `GraphNode` | a known `type`, and `prev`/`next` link back to it | typing a stale node pointer into reused memory |
//! | `SpawnInfo` | `behaviorScript` is one of the game's behaviors; the next two pointers are null or point somewhere | a terrain copy's s16 pair (y, 0x17) read as 0x0017yyyy, an address in the rendering build's constant data; the typed region relocated two collision vertices: +146 surface nodes, then a crash (out of bounds in `geo_process_node_and_siblings`) |
//! | `AllocOnlyPool` | its last block is a malloc block (or, headless, a global) at least `size` long, and it used no more than `size` | the stale `sMemPoolForGoddard` (freed after the intro, reused for Mario's graph nodes) was typed as a pool, and the pool fix-up zeroed a switch node's `parent`: Mario drawn twice (standing and running bodies) after a load |
//! | `MemoryPool` | its first word is a plausible `AllocOnlyPool` in the heap | a stale `gEffectsMemoryPool` typed Mario's animated-part node: a padding word became a pointer (translation z -28720), Mario invisible after a load |
//! | `Object` | never in the heap (`gObjectPool` is a global array) | stale platform/parent pointers into reused memory typed graph nodes and collision data as objects |
//! | `Surface` | its `object` is null or a global (a pool object) | a stale floor into a later level's collision data |
//!
//! Read next: `walk.rs`, which applies these.

use super::image::{Image, PointsTo};
use super::tables::*;
use super::types::named_by;

/// The struct (size, layout) a pointer of type `name` really points to at
/// `addr`. For `GraphNode` that is the node's own struct, by its `type` field,
/// and only if the node sits in its sibling list; `None` if it does not look
/// like a node. Other names are looked up as they are.
pub(super) fn graph_node_struct(img: &Image, name: &str, addr: u32) -> Option<(u32, u16)> {
    if name == "GraphNode" {
        let t = u16::from_le_bytes(
            img.mem
                .get(addr as usize..addr as usize + 2)?
                .try_into()
                .ok()?,
        );
        // Not a node at all (a pointer into the middle of one, or a stale
        // one into memory that has been reused): no struct to apply.
        let &(_, n) = NODE_TYPES.iter().find(|x| x.0 == t)?;
        // Nodes sit in circular doubly linked lists.
        let (prev, next) = (img.read(addr + 4)?, img.read(addr + 8)?);
        if prev == 0 || next == 0 || img.read(next + 4)? != addr || img.read(prev + 8)? != addr {
            return None;
        }
        return named_by(n);
    }
    named_by(name)
}

/// Whether the struct `name` is plausibly at `addr` (a heap address): see the
/// table at the top of this file. Structs without a rule are always plausible.
pub(super) fn plausible(img: &Image, name: &str, addr: u32) -> bool {
    match name {
        // `behaviorScript` (+0x14) is always set, and is one of the game's behaviors (an
        // address in constant data is not enough: a terrain copy's s16 pairs, y and 0x17, read
        // as 0x0017yyyy, an address in the rendering build's constant data, and the exporter
        // then translated them as the pointer they are not, changing a vertex).
        // The pointers next to it are null or point to something.
        "SpawnInfo" => {
            let ok = |v: Option<u32>| {
                v.is_some_and(|v| {
                    matches!(
                        img.classify(v),
                        PointsTo::Null | PointsTo::Heap(..) | PointsTo::Global(_)
                    )
                })
            };
            img.read(addr + 0x14)
                .is_some_and(|b| match img.classify(b) {
                    PointsTo::Global(g) => BEHAVIOR_POINTER_SLOTS
                        .binary_search_by_key(&g, |x| x.0)
                        .is_ok(),
                    _ => false,
                })
                && ok(img.read(addr + 0x18))
                && ok(img.read(addr + 0x1c))
        }
        // A pool's last block comes from malloc (or, headless, the draw arena: a global),
        // and it has used no more of it than it has. A stale pointer to a freed pool
        // (sMemPoolForGoddard after the intro) points into memory reused for something
        // else, such as graph nodes, which the pool's layout would then mangle.
        "AllocOnlyPool" => {
            let (Some(last), Some(size), Some(next)) =
                (img.read(addr), img.read(addr + 4), img.read(addr + 8))
            else {
                return false;
            };
            next <= size
                && match img.classify(last) {
                    PointsTo::Null | PointsTo::Global(_) => true,
                    PointsTo::Heap(ci, off) => off == 0 && img.chunks[ci].len >= size,
                    PointsTo::Unresolved => false,
                }
        }
        // `gEffectsMemoryPool` (system malloc): an allocation pool first. A stale pointer
        // to a freed pool (the level's pools are released, their memory reused) points
        // at something else, graph nodes say, whose words the pool's layout would turn
        // into pointers (a padding word of a graph node became a pointer).
        "MemoryPool" => img.read(addr).is_some_and(|p| match img.classify(p) {
            PointsTo::Heap(..) => plausible(img, "AllocOnlyPool", p),
            _ => false,
        }),
        // Objects live in `gObjectPool`, a global array; one in a heap block is a stale
        // pointer (a platform, a parent) into memory reused for something else, and
        // an Object layout over graph nodes or collision data makes pointers of
        // numbers (and walks into more of the same).
        "Object" => false,
        // A surface's `object` (+0x2c) is null or a pool object. A stale floor pointer into
        // reused memory (a level's collision data was freed, a later level's data sits there)
        // reads other numbers there.
        "Surface" => img
            .read(addr + 0x2c)
            .is_some_and(|o| matches!(img.classify(o), PointsTo::Null | PointsTo::Global(_))),
        _ => true,
    }
}
