//! The two builds a state can come from or go to.
//!
//! Both WASMs are linked from the same objects with the same struct layouts
//! (wasm32), so the only things that differ for one variable are its address
//! (headless and visual columns of `tables::VARS`) and, for heap data, where
//! malloc put it.
//!
//! Read next: `addresses.rs`.

use super::tables::*;

/// Which of the two builds a memory image or state belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// `build/us_web_headless`: what nodes, bots and the Rust/Python bindings run.
    Headless,
    /// `build/us_web`: the rendering build the browser runs.
    Visual,
}

impl Build {
    /// 0 for headless, 1 for visual: the column of the per-build tables, and the byte in the format.
    pub fn index(self) -> usize {
        match self {
            Build::Headless => 0,
            Build::Visual => 1,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Build::Headless => "headless",
            Build::Visual => "visual",
        }
    }
    /// The SHA-1 of the WASM's code (without data sections) the tables were made from.
    pub fn build_id(self) -> &'static str {
        match self {
            Build::Headless => BUILD_ID_HEADLESS,
            Build::Visual => BUILD_ID_VISUAL,
        }
    }
}

/// Variable `i` of `VARS` in `build`'s memory: (address, size, layout, 0xFFFF for raw bytes).
pub(super) fn var_addr(build: Build, i: usize) -> (u32, u32, u16) {
    let v = &VARS[i];
    (if build == Build::Headless { v.1 } else { v.2 }, v.3, v.4)
}
