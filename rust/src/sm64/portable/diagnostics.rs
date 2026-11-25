//! Diagnostics the export records next to the state (not part of it): the
//! pointers it could not translate, described for a person.
//!
//! Nothing here changes a state's bytes. Tools that print these:
//! `rust/examples/portable_unresolved.rs` and the rest of `portable_*`.

use super::image::Image;
use super::regions::{Owner, Region};
use super::state::Walked;
use super::tables::{NAMED, VARS};

/// The struct names that have `layout` (several structs can share one).
fn layout_names(layout: u16) -> String {
    NAMED
        .iter()
        .filter(|n| n.2 == layout)
        .map(|n| n.0)
        .collect::<Vec<_>>()
        .join("|")
}

/// Counts an untranslatable pointer (value `v` at `a`, in region `r` of `img`) and
/// describes the first 200.
pub(super) fn note_unresolved(out: &mut Walked, img: &Image, r: &Region, a: u32, v: u32) {
    out.unresolved_count += 1;
    if out.unresolved.len() < 200 {
        let what = match r.owner {
            Owner::Var(i) => VARS[i].0.to_string(),
            Owner::Chunk(c) => format!("block@{:#x}", img.chunks[c].user),
        };
        out.unresolved.push((
            format!(
                "{what} [region {} layout {} ({}) at +{:#x}]",
                r.owner_desc(),
                r.layout,
                layout_names(r.layout),
                r.off
            ),
            a - r.addr,
            v,
        ));
    }
}
