//! Portable addresses: how a pointer or function pointer of one build is
//! written so the other build can read it.
//!
//! The portable address space is:
//!
//! - globals and constant data: the **headless** build's addresses. The
//!   rendering build's are translated through `tables::RUNS`, runs of input
//!   sections that sit in the same order in both linker maps (headless start,
//!   visual start, length);
//! - malloc blocks: [`HEAP_BASE`] + the block's offset in the state's block list
//!   (each block rounded up to 16 bytes) + the offset inside it (see `relocate.rs`);
//! - function pointers (WASM table indices): the headless build's index,
//!   translated through `tables::FNS`, the functions both builds have by name.
//!
//! An address outside every run, or a function only one build has, has no
//! translation: the export nulls it and counts it as unresolved.
//!
//! Read next: `heap.rs`.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::builds::Build;
use super::tables::*;

/// Where portable pointers to malloc blocks start.
pub const HEAP_BASE: u32 = 0x4000_0000;

/// `RUNS` as (visual start, headless start, length), sorted by visual start.
fn runs_by_visual() -> &'static Vec<(u32, u32, u32)> {
    static R: OnceLock<Vec<(u32, u32, u32)>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v: Vec<_> = RUNS.iter().map(|&(h, v, s)| (v, h, s)).collect();
        v.sort_unstable();
        v
    })
}

/// `RUNS` as (headless start, visual start, length), sorted by headless start.
fn runs_by_headless() -> &'static Vec<(u32, u32, u32)> {
    static R: OnceLock<Vec<(u32, u32, u32)>> = OnceLock::new();
    R.get_or_init(|| RUNS.to_vec())
}

/// `a` translated by the run (from, to, length) that holds it, if one does.
fn lookup_run(runs: &[(u32, u32, u32)], a: u32) -> Option<u32> {
    let i = runs.partition_point(|r| r.0 <= a);
    let r = runs.get(i.checked_sub(1)?)?;
    (a - r.0 < r.2).then(|| r.1 + (a - r.0))
}

/// The portable (headless) address of a global or constant at `a` in `build`'s memory.
pub fn to_canonical_global(build: Build, a: u32) -> Option<u32> {
    match build {
        Build::Headless => lookup_run(runs_by_headless(), a).map(|_| a),
        Build::Visual => lookup_run(runs_by_visual(), a),
    }
}

/// Where a portable (headless) global address is in `build`'s memory.
pub fn from_canonical_global(build: Build, a: u32) -> Option<u32> {
    match build {
        Build::Headless => lookup_run(runs_by_headless(), a).map(|_| a),
        Build::Visual => lookup_run(runs_by_headless(), a),
    }
}

/// (headless index -> visual index, visual index -> headless index).
fn fn_maps() -> &'static (HashMap<u32, u32>, HashMap<u32, u32>) {
    static M: OnceLock<(HashMap<u32, u32>, HashMap<u32, u32>)> = OnceLock::new();
    M.get_or_init(|| {
        let h2v = FNS.iter().map(|&(h, v, _)| (h as u32, v as u32)).collect();
        let v2h = FNS.iter().map(|&(h, v, _)| (v as u32, h as u32)).collect();
        (h2v, v2h)
    })
}

/// The portable (headless) function table index of `build`'s index `idx`.
pub fn fn_to_canonical(build: Build, idx: u32) -> Option<u32> {
    match build {
        Build::Headless => Some(idx),
        Build::Visual => fn_maps().1.get(&idx).copied(),
    }
}

/// `build`'s function table index for portable index `idx`.
pub fn fn_from_canonical(build: Build, idx: u32) -> Option<u32> {
    match build {
        Build::Headless => Some(idx),
        Build::Visual => fn_maps().0.get(&idx).copied(),
    }
}
