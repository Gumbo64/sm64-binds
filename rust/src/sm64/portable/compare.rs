//! Comparing states: what the checks use to say two games are the same.
//!
//! [`Portable::digest`] is one hash per variable with pointers left out, so the
//! two builds playing the same game agree on it frame by frame (the browser
//! side gets it through `p_digest`); [`Portable::diff`] says which variables and
//! blocks of two states differ. Neither is used by export or load.
//!
//! Read next: the checks, `rust/examples/portable_fuzz.rs` and
//! `rust/examples/portable_check.rs`.

use super::state::{Buf, Portable};
use super::tables::{self, VARS};

impl Portable {
    /// One 64-bit hash per variable of the state's values: what two builds
    /// playing the same game should agree on frame by frame. Pointers (and
    /// words that look like stale ones: values in the address range of the
    /// game's data and heap) are left out, as the builds put things in
    /// different places. Compare with [`Portable::var_name`] to say which
    /// variable differs.
    pub fn digest(&self) -> Vec<u64> {
        self.vars
            .iter()
            .map(|b| {
                let rel: std::collections::HashSet<u32> =
                    b.relocs.iter().map(|r| r.0 & !3).collect();
                let mut h: u64 = 0xcbf29ce484222325;
                for (i, w) in b.data.chunks(4).enumerate() {
                    let mut x = [0u8; 4];
                    x[..w.len()].copy_from_slice(w);
                    let v = u32::from_le_bytes(x);
                    if rel.contains(&((i * 4) as u32))
                        || (tables::DATA_STARTS[0]..0x0100_0000).contains(&v)
                    {
                        continue;
                    }
                    h = (h ^ v as u64).wrapping_mul(0x100000001b3);
                }
                h
            })
            .collect()
    }

    /// A report of how two portable states differ (the same game state
    /// exported from the two builds should be identical): the variables and
    /// blocks that differ, with the first differing words.
    pub fn diff(a: &Portable, b: &Portable, limit: usize) -> Vec<String> {
        let mut out = Vec::new();
        if a.chunks.len() != b.chunks.len() {
            out.push(format!("{} blocks vs {}", a.chunks.len(), b.chunks.len()));
        }
        let describe = |name: &str, x: &Buf, y: &Buf, out: &mut Vec<String>| {
            // A block loaded into a larger one of the target's differs in
            // length only: its tail is not part of the state.
            let n = x.data.len().min(y.data.len());
            if x.data.len() != y.data.len() && !name.starts_with("block") {
                out.push(format!(
                    "{name}: {} bytes vs {}",
                    x.data.len(),
                    y.data.len()
                ));
                return;
            }
            let mut words = Vec::new();
            for o in (0..n).step_by(4) {
                let e = (o + 4).min(n);
                if x.data[o..e] != y.data[o..e] {
                    let g = |d: &[u8]| {
                        let mut w = [0u8; 4];
                        w[..e - o].copy_from_slice(&d[o..e]);
                        u32::from_le_bytes(w)
                    };
                    words.push(format!("+{o:#x}: {:#x} vs {:#x}", g(&x.data), g(&y.data)));
                }
            }
            if !words.is_empty() {
                let n = words.len();
                words.truncate(limit);
                out.push(format!("{name}: {n} words differ: {}", words.join(", ")));
            }
        };
        for (i, (x, y)) in a.vars.iter().zip(&b.vars).enumerate() {
            describe(VARS[i].0, x, y, &mut out);
        }
        for i in 0..a.chunks.len().min(b.chunks.len()) {
            describe(&format!("block {i}"), &a.chunks[i], &b.chunks[i], &mut out);
        }
        out
    }
}
