//! How the generated tables (`tables.rs`) describe memory, and turning N64
//! bytes into port bytes with them.
//!
//! A layout is a list of [`Item`]s: at this offset, `count` values of this
//! kind, `stride` bytes apart. N64 and wasm32 structs have the same offsets
//! (both are 32-bit with natural alignment), so converting is per value:
//! swap 2, 4 or 8-byte scalars from big to little endian, and translate
//! pointers from N64 addresses to port addresses ([`Translator`]).

use super::translate::Translator;

/// What a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum K {
    S1,
    S2,
    S4,
    S8,
    /// A pointer; the number is an index into `POINTEES`, the name of what
    /// it points to.
    Ptr(u16),
    /// A function pointer.
    Fn,
    /// Four bytes that are sometimes a pointer and sometimes not (the object
    /// fields' union, `uintptr_t`).
    Word,
    /// Bytes that cannot be converted value by value (bitfields): the port's
    /// are kept.
    Opaque(u32),
    /// A nested layout (arrays of structs).
    Layout(u16),
}

/// `count` values of `kind` at `offset`, `stride` bytes apart.
#[derive(Debug, Clone, Copy)]
pub struct Item(pub u32, pub K, pub u32, pub u32);

/// A variable both builds have.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Var {
    pub name: &'static str,
    /// The source file (object) it is defined in.
    pub unit: &'static str,
    pub n64: u32,
    pub port: u32,
    pub size: u32,
    pub layout: u16,
    /// Whether loading a Mupen state copies it (game state, rather than the
    /// port's rendering, allocation or level loading state).
    pub copy: bool,
    pub constant: bool,
}

/// A symbol both maps have: (N64 address, port address, port size, name).
#[derive(Debug, Clone, Copy)]
pub struct Target(pub u32, pub u32, pub u32, pub &'static str);

/// What converting one variable did.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub pointers: u32,
    /// Pointers that could not be translated (the port's values were kept).
    pub kept: u32,
    /// Words that held pointers (from the object field union and the like).
    pub words_as_pointers: u32,
}

/// Converts the N64 bytes `n64` (big endian) of something with layout
/// `layout` into `out`, which holds the port's current bytes on entry.
/// Untranslatable pointers keep the port's values. `at` is the N64 address of
/// `n64[0]`, for reporting.
pub fn convert(
    layout: u16,
    n64: &[u8],
    out: &mut [u8],
    at: u32,
    tr: &mut Translator,
    counts: &mut Counts,
) {
    for &Item(offset, kind, count, stride) in super::tables::LAYOUTS[layout as usize] {
        for i in 0..count {
            let o = (offset + i * stride) as usize;
            convert_value(kind, n64, out, o, at, tr, counts);
        }
    }
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Whether an N64 word that might be a pointer is one: a KSEG0 address in
/// the 8 MB of RDRAM, other than 0x80000000 itself (which as a float is -0.0).
pub fn looks_like_pointer(v: u32) -> bool {
    (0x8000_0001..0x8080_0000).contains(&v)
}

fn convert_value(
    kind: K,
    n64: &[u8],
    out: &mut [u8],
    o: usize,
    at: u32,
    tr: &mut Translator,
    c: &mut Counts,
) {
    let size = match kind {
        K::S1 => 1,
        K::S2 => 2,
        K::S4 | K::Ptr(_) | K::Fn | K::Word => 4,
        K::S8 => 8,
        K::Opaque(_) => return,
        K::Layout(l) => {
            let len: usize = layout_size(l);
            if let (Some(src), Some(dst)) = (n64.get(o..o + len), out.get_mut(o..o + len)) {
                convert(l, src, dst, at + o as u32, tr, c);
            }
            return;
        }
    };
    let (Some(src), Some(_)) = (n64.get(o..o + size), out.get(o..o + size)) else {
        return;
    };
    match kind {
        K::S1 | K::S2 | K::S4 | K::S8 => {
            let dst = &mut out[o..o + size];
            for (d, s) in dst.iter_mut().zip(src.iter().rev()) {
                *d = *s;
            }
        }
        K::Ptr(_) | K::Fn => {
            let v = be32(n64, o).unwrap();
            c.pointers += 1;
            match tr.translate(v, at + o as u32) {
                Some(p) => out[o..o + 4].copy_from_slice(&p.to_le_bytes()),
                None => c.kept += 1,
            }
        }
        K::Word => {
            let v = be32(n64, o).unwrap();
            // Segmented addresses (the N64 keeps some, such as animation
            // tables, as the script gave them) only when known exactly: as
            // numbers they are tiny floats or huge integers, rare but real.
            if looks_like_segmented(v)
                && let Some(p) = tr.lookup(v)
            {
                c.pointers += 1;
                c.words_as_pointers += 1;
                out[o..o + 4].copy_from_slice(&p.to_le_bytes());
            } else if looks_like_pointer(v) {
                c.pointers += 1;
                c.words_as_pointers += 1;
                match tr.translate(v, at + o as u32) {
                    Some(p) => out[o..o + 4].copy_from_slice(&p.to_le_bytes()),
                    None => c.kept += 1,
                }
            } else {
                out[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        K::Opaque(_) | K::Layout(_) => unreachable!(),
    }
}

/// Whether a word might be a segmented address (segment 1 to 0x1F).
pub fn looks_like_segmented(v: u32) -> bool {
    (0x0100_0000..0x2000_0000).contains(&v)
}

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

/// A named struct's (size, layout), from `NAMED`.
pub fn named(name: &str) -> Option<(u32, u16)> {
    super::tables::NAMED
        .binary_search_by(|(n, _, _)| n.cmp(&name))
        .ok()
        .map(|i| (super::tables::NAMED[i].1, super::tables::NAMED[i].2))
}

/// A variable by name.
pub fn var(name: &str) -> Option<&'static Var> {
    super::tables::VARS.iter().find(|v| v.name == name)
}
