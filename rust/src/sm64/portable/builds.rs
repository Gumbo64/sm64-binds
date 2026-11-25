//! The two builds.

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
    pub fn build_id(self) -> &'static str {
        match self {
            Build::Headless => BUILD_ID_HEADLESS,
            Build::Visual => BUILD_ID_VISUAL,
        }
    }
}

/// Which variables the state has, in `VARS` order, as (index, address in
/// `build`'s memory, size, layout).
pub(super) fn var_addr(build: Build, i: usize) -> (u32, u32, u16) {
    let v = &VARS[i];
    (if build == Build::Headless { v.1 } else { v.2 }, v.3, v.4)
}
