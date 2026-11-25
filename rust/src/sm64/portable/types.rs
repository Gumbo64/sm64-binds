//! Lookups over the generated tables: struct layouts by name, what a pointer
//! points to, and the offsets of `struct Object` the rules need.
//!
//! Everything here is a pure function of `tables.rs`; nothing reads memory.
//!
//! Read next: `regions.rs`.

use std::sync::OnceLock;

use super::layout::{Item, K};
use super::tables::*;

/// Calls `f(address, kind, layout, offset in that layout)` for every scalar
/// leaf of something with `layout` at `base`.
pub(super) fn leaves(layout: u16, base: u32, f: &mut impl FnMut(u32, K, u16, u32)) {
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

/// (size, layout) of the struct called `name`.
pub(super) fn named_by(name: &str) -> Option<(u32, u16)> {
    NAMED
        .binary_search_by(|(m, _, _)| m.cmp(&name))
        .ok()
        .map(|i| (NAMED[i].1, NAMED[i].2))
}

/// The layout of `struct Object`.
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

/// The layout of `struct AllocOnlyPool`.
pub(super) fn pool_layout() -> Option<u16> {
    named_by("AllocOnlyPool").map(|x| x.1)
}
