//! The vocabulary of the generated tables: what kind of value sits at each
//! offset of a struct or variable.
//!
//! A layout (`tables::LAYOUTS[i]`) is a list of [`Item`]s. This file holds only the
//! two types, because `tables.rs` (generated) imports them; the helpers that read
//! the tables are in `types.rs`, so there is no cycle between the two.
//!
//! Read next: `types.rs`.

/// What a value is (`K` for kind: the generated tables spell it out thousands of times).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum K {
    S1,
    S2,
    S4,
    S8,
    /// A pointer; the number indexes `POINTEES`, the name of what it points to.
    Ptr(u16),
    /// A function pointer: a function table index.
    Fn,
    /// Four bytes that are sometimes a pointer and sometimes not (unions, `uintptr_t`).
    Word,
    /// Bytes that are copied as they are (bitfields).
    Opaque(u32),
    /// A nested layout.
    Layout(u16),
}

/// `Item(offset, kind, count, stride)`: `count` values of `kind`, the first at
/// `offset`, the next ones `stride` bytes apart (an array; `count` 1 for a field).
#[derive(Debug, Clone, Copy)]
pub struct Item(pub u32, pub K, pub u32, pub u32);
