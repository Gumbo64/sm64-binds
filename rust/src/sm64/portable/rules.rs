//! Relocation rules: which words of a typed region are pointers.
//!
//! A layout says a word *may* be a pointer (`K::Ptr`, or `K::Word` for unions
//! and `uintptr_t`); these rules decide, per word, whether it is one. Getting
//! this wrong in either direction breaks a load: a pointer left alone points
//! at the wrong build's memory, and a number "relocated" as a pointer
//! changes the game (each rule below is here because of such a bug).
//!
//! - `K::Ptr` words are pointers, except render-list pointers
//!   ([`is_render_list`]: zeroed, they point into per-frame memory).
//! - `AllocOnlyPoolBlock::pad` is never a pointer ([`region_leaves`]): malloc
//!   leaves it unwritten.
//! - `K::Word` words are pointers only if they point into a block in use or at
//!   writable data; constant data counts only where a pointer to scripts or
//!   assets is expected ([`union_word_target`], [`may_point_to_constants`]).
//! - An object's `rawData` slot is a pointer only if its behavior uses that
//!   slot as one ([`is_number_slot`], `tables::BEHAVIOR_POINTER_SLOTS`).
//!   Otherwise its value is a number, even one that looks like an address:
//!   `oInteractType`'s `INTERACT_TEXT` is 0x800000, a Boo's oscillation timer
//!   and a spinning object's yaw grow without wrapping. Translating those
//!   corrupted them after a load.
//!
//! Read next: `plausible.rs`, the checks made before a region is applied at all.

use super::addresses::to_canonical_global;
use super::image::{Image, PointsTo};
use super::layout::K;
use super::regions::{Owner, PTRS};
use super::tables::*;
use super::types::*;

/// Pointers to the display lists the render walk builds (a master list's
/// heads and tails): the rendering build leaves them set at the end of a frame,
/// the headless one clears them, and they point into per-frame memory. They
/// are not state: a loaded game starts with none.
pub(super) fn is_render_list(pointee: u16) -> bool {
    POINTEES[pointee as usize] == "DisplayListNode"
}

/// Calls `f(address, kind, layout, offset in that layout)` for every scalar
/// leaf of a region: [`leaves`] of its layout, or its words for a [`PTRS`] region.
pub(super) fn region_leaves(
    layout: u16,
    base: u32,
    count: u32,
    f: &mut impl FnMut(u32, K, u16, u32),
) {
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

/// Whether a word in a union is a pointer (and which sort): one into a block
/// in use, or to a global at or after the start of `.data` (or into constant
/// data too, if `rodata_ok`). Anything else is a number ([`PointsTo::Null`]).
pub(super) fn union_word_target(img: &Image, v: u32, rodata_ok: bool) -> PointsTo {
    match img.classify(v) {
        PointsTo::Heap(c, o) => PointsTo::Heap(c, o),
        PointsTo::Global(g) if g >= DATA_START || rodata_ok => PointsTo::Global(g),
        _ => PointsTo::Null,
    }
}

/// Variables that only hold render walk state (not game state): never ghosted.
pub(super) fn is_display_list_heap(owner: Owner) -> bool {
    matches!(owner, Owner::Var(i) if VARS[i].0.ends_with(":gDisplayListHeap"))
}

/// The object slots (`rawData` words) the object at `obj` uses as pointers: the common
/// fields' and its behavior's (`BEHAVIOR_POINTER_SLOTS`); every slot any field declares a
/// pointer (`POINTER_SLOTS`) if its behavior is not known.
pub(super) fn object_pointer_slots(img: &Image, obj: u32) -> u128 {
    let bhv = img
        .read(obj + OBJ_BEHAVIOR)
        .and_then(|b| to_canonical_global(img.build, b));
    match bhv.and_then(|b| {
        BEHAVIOR_POINTER_SLOTS
            .binary_search_by_key(&b, |x| x.0)
            .ok()
    }) {
        Some(i) => COMMON_POINTER_SLOTS | BEHAVIOR_POINTER_SLOTS[i].1,
        None => POINTER_SLOTS,
    }
}

/// Whether the word at `a`, at offset `off` of a `layout` region, is an object's `rawData`
/// slot that its behavior does not use as a pointer: a number, whatever its value. Values
/// there fall in the address range (`INTERACT_TEXT` is 0x800000; a Boo's oscillation timer
/// or a spinning object's yaw grows without wrapping), and translating them as addresses
/// changes them. `obj` is [`object_layout`].
pub(super) fn is_number_slot(img: &Image, obj: Option<u16>, layout: u16, a: u32, off: u32) -> bool {
    Some(layout) == obj
        && (OBJ_RAW..OBJ_RAW + 4 * OBJ_RAW_WORDS).contains(&off)
        && (object_pointer_slots(img, a - off) >> ((off - OBJ_RAW) / 4)) & 1 == 0
}

/// Whether a union word at offset `off` of a `layout` region may point into
/// constant data (scripts, assets), not only at writable data.
pub(super) fn may_point_to_constants(obj: Option<u16>, layout: u16, off: u32) -> bool {
    // arrays of uintptr_t (the segment table, the level script and geo layout
    // stacks) hold addresses of scripts and assets
    if only_words(layout) {
        return true;
    }
    Some(layout) == obj
        && (((OBJ_RAW..OBJ_RAW + 4 * 128).contains(&off)
            && (POINTER_SLOTS >> ((off - OBJ_RAW) / 4)) & 1 != 0)
            // bhvStack: addresses inside behavior scripts
            || (OBJ_BHV_STACK..OBJ_BHV_STACK + 32).contains(&off))
}
