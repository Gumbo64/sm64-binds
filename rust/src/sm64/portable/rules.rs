//! Which words are pointers.

use super::addresses::*;
use super::heap::rd;
use super::tables::*;
use super::types::*;
use super::walk::*;

impl Walk<'_> {
    /// Whether a word in a union is a pointer (and which sort).
    pub(super) fn word_target(&self, v: u32, rodata_ok: bool) -> Tgt {
        match self.classify(v) {
            Tgt::Heap(c, o) => Tgt::Heap(c, o),
            Tgt::Global(g) if g >= DATA_START || rodata_ok => Tgt::Global(g),
            _ => Tgt::Null,
        }
    }

    /// Variables that only hold render walk state (not game state).
    pub(super) fn render_only(&self, slot: Slot) -> bool {
        matches!(slot, Slot::Var(i) if VARS[i].0.ends_with(":gDisplayListHeap"))
    }

    /// The object slots (`rawData` words) the object at `obj` uses as pointers: the common
    /// fields' and its behavior's (`BEHAVIOR_POINTER_SLOTS`); every slot any field declares a
    /// pointer (`POINTER_SLOTS`) if its behavior is not known.
    pub(super) fn object_pointer_slots(&self, obj: u32) -> u128 {
        let bhv = rd(self.mem, obj + OBJ_BEHAVIOR).and_then(|b| to_canonical_global(self.build, b));
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
    /// changes them.
    pub(super) fn numeric_slot(&self, obj: Option<u16>, layout: u16, a: u32, off: u32) -> bool {
        Some(layout) == obj
            && (OBJ_RAW..OBJ_RAW + 4 * OBJ_RAW_WORDS).contains(&off)
            && (self.object_pointer_slots(a - off) >> ((off - OBJ_RAW) / 4)) & 1 == 0
    }

    pub(super) fn rodata_slot(&self, obj: Option<u16>, layout: u16, off: u32) -> bool {
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
}
