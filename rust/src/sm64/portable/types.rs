//! Type lookups over the generated tables.

use std::sync::OnceLock;

use super::layout::{Item, K};
use super::tables::*;

/// The extent of a layout: the end of its last item.
pub fn layout_size(layout: u16) -> usize {
    super::tables::LAYOUTS[layout as usize]
        .iter()
        .map(|&Item(offset, kind, count, stride)| {
            let last = match kind {
                K::S1 => 1,
                K::S2 => 2,
                K::S4 | K::Ptr(_) | K::Fn | K::Word => 4,
                K::S8 => 8,
                K::Opaque(n) => n as usize,
                K::Layout(l) => layout_size(l),
            };
            offset as usize + (count.max(1) as usize - 1) * stride as usize + last
        })
        .max()
        .unwrap_or(0)
}

/// Calls `f(address, kind, layout, offset in that layout)` for every scalar
/// leaf of something with `layout` at `base`.
pub fn leaves(layout: u16, base: u32, f: &mut impl FnMut(u32, K, u16, u32)) {
    for &Item(offset, kind, count, stride) in super::tables::LAYOUTS[layout as usize] {
        for i in 0..count {
            let o = offset + i * stride;
            match kind {
                K::Layout(l) => leaves(l, base + o, f),
                _ => f(base + o, kind, layout, o),
            }
        }
    }
}

/// (size, layout) of what pointee `id` points to, if it is a struct we know.
pub(super) fn pointee(id: u16) -> Option<(u32, u16)> {
    static P: OnceLock<Vec<Option<(u32, u16)>>> = OnceLock::new();
    P.get_or_init(|| {
        POINTEES
            .iter()
            .map(|n| {
                NAMED
                    .binary_search_by(|(m, _, _)| m.cmp(n))
                    .ok()
                    .map(|i| (NAMED[i].1, NAMED[i].2))
            })
            .collect()
    })[id as usize]
}

/// Pointers to the display lists the render walk builds (a master list's
/// heads and tails): the rendering build leaves them set at the end of a frame,
/// the headless one clears them, and they point into per-frame memory. They
/// are not state: a loaded game starts with none.
pub(super) fn is_render_list(pointee: u16) -> bool {
    POINTEES[pointee as usize] == "DisplayListNode"
}

/// Whether a layout is nothing but `uintptr_t` words.
pub(super) fn only_words(layout: u16) -> bool {
    static W: OnceLock<Vec<bool>> = OnceLock::new();
    W.get_or_init(|| {
        LAYOUTS
            .iter()
            .map(|items| !items.is_empty() && items.iter().all(|i| matches!(i.1, K::Word)))
            .collect()
    })
    .get(layout as usize)
    .copied()
    .unwrap_or(false)
}

pub(super) fn named_by(name: &str) -> Option<(u32, u16)> {
    NAMED
        .binary_search_by(|(m, _, _)| m.cmp(&name))
        .ok()
        .map(|i| (NAMED[i].1, NAMED[i].2))
}

pub(super) fn object_layout() -> Option<u16> {
    static L: OnceLock<Option<u16>> = OnceLock::new();
    *L.get_or_init(|| named_by("Object").map(|x| x.1))
}

/// `struct Object`'s first rawData slot, from include/object_fields.h.
pub(super) const OBJ_RAW: u32 = 0x88;
/// `behavior`: the address of the object's behavior script.
pub(super) const OBJ_BEHAVIOR: u32 = 0x20C;
/// `rawData`'s length in words (`asU32[0x50]`): after it come real pointers (`curBhvCommand`, ...).
pub(super) const OBJ_RAW_WORDS: u32 = 0x50;
/// `struct Object::bhvStack` (uintptr_t[8]).
pub(super) const OBJ_BHV_STACK: u32 = 0x1D4;

pub(super) fn pool_layout() -> Option<u16> {
    named_by("AllocOnlyPool").map(|x| x.1)
}

pub(super) fn layout_names(layout: u16) -> String {
    NAMED
        .iter()
        .filter(|n| n.2 == layout)
        .map(|n| n.0)
        .collect::<Vec<_>>()
        .join("|")
}

/// The layout of a run of pointers behind a pointer to pointers (`GraphNode **views`):
/// as many words as hold pointers, up to 64.
pub(super) const PTRS: u16 = 0xFFFE;

pub(super) fn rleaves(layout: u16, base: u32, count: u32, f: &mut impl FnMut(u32, K, u16, u32)) {
    if layout == PTRS {
        for i in 0..count {
            f(base + 4 * i, K::Word, PTRS, 4 * i);
        }
    } else {
        // `AllocOnlyPoolBlock::pad` (a `void *`, "require 8 bytes alignment") is never written:
        // whatever malloc left there is not a pointer, though it can look like one.
        let pool_block = named_by("AllocOnlyPoolBlock").map(|x| x.1);
        leaves(layout, base, &mut |a, k, l, o| {
            if !(Some(l) == pool_block && o == 4) {
                f(a, k, l, o)
            }
        });
    }
}
