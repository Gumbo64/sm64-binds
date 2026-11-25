//! The walk: which malloc blocks are part of the game's state.
//!
//! The state is the game's variables (`tables::VARS`) and every malloc block
//! reachable from them. The walk starts with one typed region per variable and
//! follows pointers, newest region first (a stack): a pointer into a block in use
//! *reaches* the block (it becomes part of the state, numbered in the order
//! found), and, if the pointer's struct is known and plausible there
//! (`plausible.rs`), adds a typed region for it, whose pointers are followed in
//! turn. Then the first word of every reached block is checked for the link of
//! an allocation pool (the exact address of another block), which no type
//! describes.
//!
//! Guarantees: the walk only reads `mem`; it reaches the same blocks, in the
//! same order, for the same memory; and a word is only ever followed (or later
//! relocated) if a layout says it may be a pointer and `rules.rs` agrees.
//!
//! `export.rs` then hands the result to `relocate.rs`, which turns it into
//! portable bytes. Read next: `relocate.rs` (and `ghosts.rs`).

use std::collections::{HashMap, HashSet};

use super::builds::{Build, var_addr};
use super::excluded::draw_pools;
use super::image::{Image, PointsTo};
use super::layout::K;
use super::plausible::{graph_node_struct, plausible};
use super::regions::{Owner, PTRS, Region};
use super::rules::*;
use super::tables::*;
use super::types::*;

pub(super) struct Walk<'a> {
    pub(super) img: Image<'a>,
    /// `gMatStackFixed`'s variable index: render walk pointers, not state.
    pub(super) mat_stack_fixed: Option<usize>,
    /// The chunks reached (indices into `img.chunks`), in the order found: the
    /// state's block list.
    pub(super) order: Vec<usize>,
    /// Chunk index -> its position in `order`.
    pub(super) reached: HashMap<usize, usize>,
    pub(super) regions: Vec<Region>,
    /// (address, layout) of every region, so none is walked twice.
    seen: HashSet<(u32, u16)>,
    /// Regions not walked yet (indices into `regions`), taken from the end.
    queue: Vec<usize>,
    /// Pointers in this address range are not followed (the display list pool).
    pub(super) skip: Vec<(u32, u32)>,
    pub(super) found_by: Vec<(Owner, u32)>,
    pub(super) bad_nodes: Vec<(u32, u16)>,
}

/// A word no layout describes that holds exactly the address of a block (an
/// allocation pool's link to its previous block): (chunk, offset in it,
/// target chunk, offset in the target). Translated like a pointer.
pub(super) type ExactLink = (usize, u32, usize, u32);

impl<'a> Walk<'a> {
    /// Adds chunk `ci` to the state (if it is not yet), first found at `by`.
    /// Returns its position in the state's block list.
    pub(super) fn reach(&mut self, ci: usize, by: (Owner, u32)) -> usize {
        if let Some(&i) = self.reached.get(&ci) {
            return i;
        }
        self.found_by.push(by);
        let i = self.order.len();
        self.order.push(ci);
        self.reached.insert(ci, i);
        i
    }

    fn push_region(&mut self, addr: u32, layout: u16, owner: Owner, off: u32) {
        self.push_region_n(addr, layout, 0, owner, off);
    }

    fn push_region_n(&mut self, addr: u32, layout: u16, count: u32, owner: Owner, off: u32) {
        if self.seen.insert((addr, layout)) {
            self.regions.push(Region {
                addr,
                layout,
                count,
                owner,
                off,
            });
            self.queue.push(self.regions.len() - 1);
        }
    }

    /// Follows the pointers of region `ri`: reaches the blocks they point
    /// into and queues regions for the structs behind them.
    fn process(&mut self, ri: usize) {
        let Region {
            addr,
            layout,
            count,
            owner,
            off: roff,
        } = self.regions[ri];
        if self.mat_stack_fixed.is_some_and(|m| owner == Owner::Var(m)) {
            return;
        }
        // A main pool block that holds an allocation pool (its release handler
        // says so): the pool sits right after the header, reached by no pointer.
        if let (Owner::Chunk(ci), Some(pl)) = (owner, pool_layout())
            && Some(layout) == named_by("MainPoolBlock").map(|x| x.1)
            && let Some(h) = self.img.read(addr + 8)
            && FNS.iter().any(|f| {
                f.2 == "alloc_only_pool_release_handler"
                    && h == if self.img.build == Build::Headless {
                        f.0
                    } else {
                        f.1
                    } as u32
            })
            && roff + 12 + 12 <= self.img.chunks[ci].len
        {
            self.push_region(addr + 12, pl, owner, roff + 12);
        }
        let obj = object_layout();
        let mut found: Vec<(u32, K, u16, u32)> = Vec::new();
        region_leaves(layout, addr, count, &mut |a, k, l, o| {
            if matches!(k, K::Ptr(_) | K::Word) {
                found.push((a, k, l, o));
            }
        });
        for (a, k, l, o) in found {
            if self.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                continue;
            }
            let Some(v) = self.img.read(a) else { continue };
            if let K::Ptr(pid) = k
                && is_render_list(pid)
            {
                continue;
            }
            let tgt = match k {
                K::Ptr(_) => self.img.classify(v),
                _ if is_number_slot(&self.img, obj, l, a, o) => PointsTo::Null,
                _ => union_word_target(&self.img, v, may_point_to_constants(obj, l, o)),
            };
            let PointsTo::Heap(ci, off) = tgt else {
                continue;
            };
            self.reach(ci, (owner, a - addr + roff));
            let K::Ptr(pid) = k else { continue };
            let name = POINTEES[pid as usize];
            let info = if name == "GraphNode" {
                let r = graph_node_struct(&self.img, name, v);
                if r.is_none() && self.bad_nodes.len() < 40 {
                    let t = u16::from_le_bytes(
                        self.img.mem[v as usize..v as usize + 2].try_into().unwrap(),
                    );
                    self.bad_nodes.push((v, t));
                }
                r
            } else {
                pointee(pid)
            };
            if name == "ptr" {
                // A pointer to pointers (`GraphNode **views`): as many words as hold pointers.
                let c = self.img.chunks[ci];
                let mut n = 0;
                while n < 64 && off + 4 * n + 4 <= c.len {
                    let Some(w) = self.img.read(v + 4 * n) else {
                        break;
                    };
                    if !matches!(
                        union_word_target(&self.img, w, false),
                        PointsTo::Heap(..) | PointsTo::Global(_)
                    ) {
                        break;
                    }
                    n += 1;
                }
                if n > 0 && v % 4 == 0 {
                    self.push_region_n(v, PTRS, n, Owner::Chunk(ci), off);
                }
            }
            if let Some((size, lay)) = info {
                let c = self.img.chunks[ci];
                if off + size <= c.len && v % 2 == 0 && plausible(&self.img, name, v) {
                    self.push_region(v, lay, Owner::Chunk(ci), off);
                }
            }
        }
    }

    /// Words no layout describes that hold exactly the address of a block (the
    /// links of allocation pools, whose owners sit in structs reached only by
    /// value): followed, and returned to be translated like pointers.
    fn follow_pool_links(&mut self) -> Vec<ExactLink> {
        let mut exact = Vec::new();
        // Which bytes of each chunk some region covers.
        let mut cover: HashMap<usize, Vec<bool>> = HashMap::new();
        for r in &self.regions {
            if let Owner::Chunk(ci) = r.owner {
                let c = cover
                    .entry(ci)
                    .or_insert_with(|| vec![false; self.img.chunks[ci].len as usize]);
                region_leaves(r.layout, r.addr, r.count, &mut |a, k, _, _| {
                    let sz = match k {
                        K::S1 => 1,
                        K::S2 => 2,
                        K::S8 => 8,
                        K::Opaque(n) => n,
                        _ => 4,
                    } as usize;
                    let o = (a - r.addr + r.off) as usize;
                    for b in o..(o + sz).min(c.len()) {
                        c[b] = true;
                    }
                });
            }
        }
        // `order` grows while this runs: a block reached by a link is checked too.
        let mut done = 0;
        while done < self.order.len() {
            let ci = self.order[done];
            done += 1;
            let c = self.img.chunks[ci];
            let cov = cover.get(&ci);
            // The link of an allocation pool's blocks: the first word of a block
            // is the exact address of the pool's previous block. (Other words are
            // not looked at: in a block of s16 data, one in a few thousand looks
            // like an address.)
            let Some(v) = self.img.read(c.user) else {
                continue;
            };
            if v == 0 || v % 4 != 0 {
                continue;
            }
            if cov.is_some_and(|x| x[0]) {
                continue;
            }
            if let Some(i) = self
                .img
                .chunk_of(v)
                .filter(|&i| self.img.chunks[i].user == v && i != ci)
            {
                self.reach(i, (Owner::Chunk(ci), 0));
                exact.push((ci, 0, i, 0));
            }
        }
        exact
    }
}

/// Walks `mem` (which may hold an unsupported state): the regions and the
/// reached blocks, and the pool links followed.
#[allow(clippy::needless_range_loop)]
pub(super) fn find_state(mem: &[u8], build: Build) -> Result<(Walk<'_>, Vec<ExactLink>), String> {
    let mut w = Walk {
        img: Image::new(mem, build)?,
        mat_stack_fixed: VARS.iter().position(|v| v.0.ends_with(":gMatStackFixed")),
        order: Vec::new(),
        reached: HashMap::new(),
        regions: Vec::new(),
        seen: HashSet::new(),
        queue: Vec::new(),
        skip: draw_pools(mem, build)
            .iter()
            .map(|&p| (p, p + 16))
            .collect(),
        found_by: Vec::new(),
        bad_nodes: Vec::new(),
    };
    for i in 0..VARS.len() {
        let (addr, size, layout) = var_addr(build, i);
        if addr as usize + size as usize > mem.len() {
            return Err(format!("variable {} is outside memory", VARS[i].0));
        }
        if layout != 0xFFFF {
            w.push_region(addr, layout, Owner::Var(i), 0);
        }
    }
    while let Some(ri) = w.queue.pop() {
        w.process(ri);
    }
    let exact = w.follow_pool_links();
    Ok((w, exact))
}
