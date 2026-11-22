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
