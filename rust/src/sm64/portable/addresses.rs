//! Address translation.

use std::collections::HashMap;
use std::sync::OnceLock;

use super::builds::Build;
use super::tables::*;

/// Where portable pointers to malloc blocks start.
pub const HEAP_BASE: u32 = 0x4000_0000;

// --- address translation tables ---------------------------------------------

pub(super) fn runs_by_visual() -> &'static Vec<(u32, u32, u32)> {
    static R: OnceLock<Vec<(u32, u32, u32)>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v: Vec<_> = RUNS.iter().map(|&(h, v, s)| (v, h, s)).collect();
        v.sort_unstable();
        v
    })
}

pub(super) fn lookup_run(runs: &[(u32, u32, u32)], a: u32) -> Option<u32> {
    let i = runs.partition_point(|r| r.0 <= a);
    let r = runs.get(i.checked_sub(1)?)?;
    (a - r.0 < r.2).then(|| r.1 + (a - r.0))
}

/// The headless address of a global or constant at `a` in `build`'s memory.
pub fn to_canonical_global(build: Build, a: u32) -> Option<u32> {
    match build {
        Build::Headless => lookup_run(runs_vec(), a).map(|_| a),
        Build::Visual => lookup_run(runs_by_visual(), a),
    }
}

pub(super) fn runs_vec() -> &'static Vec<(u32, u32, u32)> {
    static R: OnceLock<Vec<(u32, u32, u32)>> = OnceLock::new();
    R.get_or_init(|| RUNS.to_vec())
}

/// Where a portable (headless) global address is in `build`'s memory.
pub fn from_canonical_global(build: Build, a: u32) -> Option<u32> {
    match build {
        Build::Headless => lookup_run(runs_vec(), a).map(|_| a),
        Build::Visual => lookup_run(runs_vec(), a),
    }
}

pub(super) fn fn_maps() -> &'static (HashMap<u32, u32>, HashMap<u32, u32>) {
    static M: OnceLock<(HashMap<u32, u32>, HashMap<u32, u32>)> = OnceLock::new();
    M.get_or_init(|| {
        let h2v = FNS.iter().map(|&(h, v, _)| (h as u32, v as u32)).collect();
        let v2h = FNS.iter().map(|&(h, v, _)| (v as u32, h as u32)).collect();
        (h2v, v2h)
    })
}

pub fn fn_to_canonical(build: Build, idx: u32) -> Option<u32> {
    match build {
        Build::Headless => Some(idx),
        Build::Visual => fn_maps().1.get(&idx).copied(),
    }
}

pub fn fn_from_canonical(build: Build, idx: u32) -> Option<u32> {
    match build {
        Build::Headless => Some(idx),
        Build::Visual => fn_maps().0.get(&idx).copied(),
    }
}
