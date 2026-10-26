//! Translating N64 pointers into port pointers.
//!
//! In order of preference, an N64 pointer is
//!
//! 1. null,
//! 2. one learned exactly (`pairs`): N64 and port memory that hold the same
//!    thing side by side, such as the two builds' behavior scripts (their
//!    collision, animation and function pointers), the loaded model tables,
//!    the surface pools walked in allocation order,
//! 3. inside a region known to correspond (`regions`): heap structs reached
//!    from both sides (the camera, spawn infos, warp nodes, Mario's animation
//!    buffer),
//! 4. inside a variable both builds name (`TARGETS`), at the same offset,
//! 5. inside a loaded ROM segment whose symbol both builds name (behavior
//!    scripts, segment 0x13), through the N64's segment table.
//!
//! Anything else is not translated, and the loader keeps the port's value.

use std::collections::{BTreeMap, HashMap};

use super::MupenState;
use super::tables::{SEGMENT_NAMED, TARGETS};

/// The segment behavior scripts are loaded into.
pub const BEHAVIOR_SEGMENT: u32 = 0x13;

pub struct Translator<'a> {
    pub st: &'a MupenState,
    /// N64 pointer -> port pointer. `None` where two sources disagreed.
    pairs: HashMap<u32, Option<u32>>,
    /// (N64 start, length, port start), sorted by N64 start when used.
    regions: Vec<(u32, u32, u32)>,
    sorted: bool,
    /// The N64 segment table: physical base of each segment.
    pub segments: [u32; 32],
    /// Untranslated pointers by the N64 address that held them: the value.
    pub failures: BTreeMap<u32, u32>,
    pub translated: u32,
    /// The port's memory, for checking candidates by content.
    port: Option<&'a [u8]>,
    /// Port data already checked against the N64's: whether it agreed.
    checked: HashMap<u32, bool>,
}

impl<'a> Translator<'a> {
    pub fn new(st: &'a MupenState, segment_table: Option<u32>, port: Option<&'a [u8]>) -> Self {
        let mut segments = [0u32; 32];
        if let Some(t) = segment_table {
            for (i, s) in segments.iter_mut().enumerate() {
                *s = st.u32(t + 4 * i as u32).unwrap_or(0);
            }
        }
        Translator {
            st,
            pairs: HashMap::new(),
            regions: Vec::new(),
            sorted: true,
            segments,
            failures: BTreeMap::new(),
            translated: 0,
            port,
            checked: HashMap::new(),
        }
    }

    /// Learns that N64 pointer `n64` is port pointer `port`. A second,
    /// different answer for the same pointer makes it untranslatable.
    pub fn pair(&mut self, n64: u32, port: u32) {
        if n64 == 0 || port == 0 {
            return;
        }
        match self.pairs.get(&n64) {
            None => {
                self.pairs.insert(n64, Some(port));
            }
            Some(Some(p)) if *p != port => {
                self.pairs.insert(n64, None);
            }
            _ => {}
        }
    }

    /// Learns that `len` bytes from N64 address `n64` are at port `port`.
    pub fn region(&mut self, n64: u32, len: u32, port: u32) {
        if n64 != 0 && port != 0 && len != 0 {
            self.regions.push((n64, len, port));
            self.sorted = false;
        }
    }

    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// The N64 virtual address of segmented address `seg`, if its segment
    /// is loaded.
    pub fn segmented_to_virtual(&self, seg: u32) -> Option<u32> {
        let s = (seg >> 24) as usize;
        if s == 0 || s >= 32 || self.segments[s] == 0 {
            return None;
        }
        Some(0x8000_0000 | (self.segments[s] + (seg & 0x00FF_FFFF)))
    }

    /// The segmented address of N64 virtual address `v`: the loaded segment
    /// with the greatest base at or below it.
    pub fn virtual_to_segmented(&self, v: u32) -> Option<u32> {
        let phys = v & 0x1FFF_FFFF;
        let mut best: Option<(usize, u32)> = None;
        for (s, &base) in self.segments.iter().enumerate().skip(1) {
            if base != 0 && base <= phys && best.is_none_or(|(_, b)| base > b) {
                best = Some((s, base));
            }
        }
        let (s, base) = best?;
        let off = phys - base;
        (off < 0x0100_0000).then_some(((s as u32) << 24) | off)
    }

    /// The port pointer for N64 pointer `v`, found at N64 address `at`
    /// (recorded when it fails).
    pub fn translate(&mut self, v: u32, at: u32) -> Option<u32> {
        let r = self.lookup(v);

        match r {
            Some(_) => self.translated += 1,
            None => {
                self.failures.insert(at, v);
            }
        }
        r
    }

    pub fn lookup(&mut self, v: u32) -> Option<u32> {
        if v == 0 {
            return Some(0);
        }
        if let Some(p) = self.pairs.get(&v) {
            return *p;
        }
        // Segmented addresses stored as such (the N64 keeps some, such as
        // animation tables, as the scripts gave them).
        if (0x0100_0000..0x2000_0000).contains(&v) {
            if v >> 24 == BEHAVIOR_SEGMENT
                && let Some(p) = target(v)
            {
                return Some(p);
            }
            let virt = self.segmented_to_virtual(v)?;
            if let Some(p) = self.pairs.get(&virt) {
                return *p;
            }
            return self.segment_named(v, virt);
        }
        if !self.sorted {
            self.regions.sort_unstable();
            self.sorted = true;
        }
        let i = self.regions.partition_point(|r| r.0 <= v);
        // Regions may nest (a struct inside a buffer): take the innermost
        // one that contains v, searching back a little.
        for &(start, len, port) in self.regions[..i].iter().rev().take(8) {
            if v - start < len {
                return Some(port + (v - start));
            }
        }
        if let Some(p) = target(v) {
            return Some(p);
        }
        // Inside a loaded segment: segments can sit below one another, so
        // every segment that starts at or below v is a candidate.
        let phys = v & 0x1FFF_FFFF;
        let mut candidates: Vec<(u32, u32)> = (1..32u32)
            .filter_map(|s| {
                let base = self.segments[s as usize];
                (base != 0 && base <= phys && phys - base < 0x0100_0000)
                    .then_some((base, (s << 24) | (phys - base)))
            })
            .collect();
        candidates.sort_unstable_by_key(|c| std::cmp::Reverse(c.0));
        for (_, seg) in candidates {
            let found = if seg >> 24 == BEHAVIOR_SEGMENT {
                target(seg)
            } else {
                self.segment_named(seg, v)
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// Segmented address `seg` (N64 virtual `virt`) inside port data named
    /// after its segmented address, if the contents on both sides agree (a
    /// segment number is reused by many levels and actor groups, so the
    /// name alone does not say which one the N64 has loaded).
    fn segment_named(&mut self, seg: u32, virt: u32) -> Option<u32> {
        let port = self.port?;
        let i = SEGMENT_NAMED.partition_point(|e| e.0 <= seg);
        let start = SEGMENT_NAMED.get(i.checked_sub(1)?)?.0;
        let mut found = None;
        for &(s, p, size) in SEGMENT_NAMED[..i].iter().rev().take_while(|e| e.0 == start) {
            if seg - s >= size {
                continue;
            }
            let ok = *self.checked.entry(p).or_insert_with(|| {
                let base = virt - (seg - s);
                same_contents(self.st, &self.segments, port, base, p, size)
            });
            if ok {
                if found.is_some() {
                    return None; // two candidates agree: ambiguous
                }
                found = Some(p + (seg - s));
            }
        }
        if let Some(p) = found {
            self.pairs.insert(virt, Some(p));
        }
        found
    }
}

/// `v` inside a symbol both builds have: the same offset in the port's.
pub fn target(v: u32) -> Option<u32> {
    let i = TARGETS.partition_point(|t| t.0 <= v);
    let t = TARGETS.get(i.checked_sub(1)?)?;
    (v - t.0 < t.2).then(|| t.1 + (v - t.0))
}

/// A symbol both builds have, by name: (N64 address, port address, size).
pub fn target_by_name(name: &str) -> Option<(u32, u32, u32)> {
    TARGETS
        .iter()
        .find(|t| t.3 == name)
        .map(|t| (t.0, t.1, t.2))
}

/// Whether N64 data at `n64` and port data at `port` (`size` bytes) are the
/// same thing: every 4 bytes equal as bytes, as a 32-bit value or as two
/// 16-bit values, or a pointer on both sides; at least one equal non-zero
/// scalar. An array of only pointers (an animation table) is checked one
/// level down: each pointee's first 12 bytes as six 16-bit values.
fn same_contents(
    st: &MupenState,
    segments: &[u32; 32],
    mem: &[u8],
    n64: u32,
    port: u32,
    size: u32,
) -> bool {
    let (Some(n), Some(p)) = (
        st.bytes(n64, size as usize),
        mem.get(port as usize..(port + size) as usize),
    ) else {
        return false;
    };
    let virt = |w: u32| -> Option<u32> {
        if super::layout::looks_like_pointer(w) {
            return Some(w);
        }
        let s = (w >> 24) as usize;
        ((1..32).contains(&s) && segments[s] != 0)
            .then(|| 0x8000_0000 | (segments[s] + (w & 0xFF_FFFF)))
    };
    let mut scalars = 0;
    let mut pointers = Vec::new();
    for (wn, wp) in n.as_chunks::<4>().0.iter().zip(p.as_chunks::<4>().0) {
        let bn = u32::from_be_bytes(*wn);
        let bp = u32::from_le_bytes(*wp);
        let halves = (bn >> 16) as u16 == u16::from_le_bytes([wp[0], wp[1]])
            && bn as u16 == u16::from_le_bytes([wp[2], wp[3]]);
        if bn == bp || wn == wp || halves {
            scalars += u32::from(bn != 0);
        } else if let Some(v) = virt(bn)
            && (bp as usize) < mem.len()
            && bp != 0
        {
            pointers.push((v, bp));
        } else {
            return false;
        }
    }
    if scalars > 0 {
        return true;
    }
    !pointers.is_empty()
        && pointers.iter().all(|&(v, bp)| {
            let (Some(a), Some(b)) = (st.bytes(v, 12), mem.get(bp as usize..bp as usize + 12))
            else {
                return false;
            };
            (0..6).all(|k| a[2 * k] == b[2 * k + 1] && a[2 * k + 1] == b[2 * k])
        })
}
