//! How the generated tables describe memory: a layout is a list of [`Item`]s,
//! `count` values of a kind at an offset, `stride` bytes apart.

/// What a value is.
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

#[derive(Debug, Clone, Copy)]
pub struct Item(pub u32, pub K, pub u32, pub u32);
