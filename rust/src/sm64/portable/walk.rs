//! Finding the game's state in a build's memory and putting it in the
//! portable form: its variables, and the malloc blocks reachable from them,
//! with every pointer replaced by a build-neutral one.
//!
//! The portable address space is the headless build's for globals and
//! constant data, `HEAP_BASE` and up for malloc blocks (each at the offset
//! its position in the state's block list gives it), and the headless
//! function table's indices for function pointers.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::layout::{K, layout_size, leaves};
use super::tables::*;

/// Where portable pointers to malloc blocks start.
pub const HEAP_BASE: u32 = 0x4000_0000;

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

/// A malloc block in use: its user address and usable length.
#[derive(Debug, Clone, Copy)]
pub struct Chunk {
    pub user: u32,
    pub len: u32,
}

fn rd(mem: &[u8], a: u32) -> Option<u32> {
    Some(u32::from_le_bytes(
        mem.get(a as usize..a as usize + 4)?.try_into().ok()?,
    ))
}

/// Every malloc block in use, found by walking dlmalloc's chunks from the
/// start of its heap to its top.
pub fn heap_chunks(mem: &[u8], build: Build) -> Result<Vec<Chunk>, String> {
    let gm = GM[build.index()];
    let (Some(least), Some(top)) = (rd(mem, gm + 16), rd(mem, gm + 24)) else {
        return Err("memory too short for the allocator state".into());
    };
    let mut out = Vec::new();
    let mut c = least;
    while c < top {
        let head = rd(mem, c + 4).ok_or("chunk past the end of memory")?;
        let size = head & !7;
        if size < 16 || c + size > top {
            return Err(format!("corrupt chunk at {c:#x} (size {size})"));
        }
        let next = c + size;
        let next_head = rd(mem, next + 4).ok_or("chunk past the end of memory")?;
        if next_head & 1 != 0 {
            out.push(Chunk {
                user: c + 8,
                len: size - 4,
            });
        }
        c = next;
    }
    if c != top {
        return Err(format!(
            "heap walk ended at {c:#x}, not at its top {top:#x}"
        ));
    }
    Ok(out)
}

// --- address translation tables ---------------------------------------------

fn runs_by_visual() -> &'static Vec<(u32, u32, u32)> {
    static R: OnceLock<Vec<(u32, u32, u32)>> = OnceLock::new();
    R.get_or_init(|| {
        let mut v: Vec<_> = RUNS.iter().map(|&(h, v, s)| (v, h, s)).collect();
        v.sort_unstable();
        v
    })
}

fn lookup_run(runs: &[(u32, u32, u32)], a: u32) -> Option<u32> {
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

fn runs_vec() -> &'static Vec<(u32, u32, u32)> {
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

fn fn_maps() -> &'static (HashMap<u32, u32>, HashMap<u32, u32>) {
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

/// (size, layout) of what pointee `id` points to, if it is a struct we know.
fn pointee(id: u16) -> Option<(u32, u16)> {
    static P: OnceLock<Vec<Option<(u32, u16)>>> = OnceLock::new();
    P.get_or_init(|| {
        POINTEES
            .iter()
            .map(|n| {
                NAMED
                    .binary_search_by(|(m, _, _)| m.cmp(n))
                    .ok()
                    .map(|i| (NAMED[i].1, NAMED[i].2))
            })
            .collect()
    })[id as usize]
}

/// Pointers to the display lists the render walk builds (a master list's
/// heads and tails): the rendering build leaves them set at the end of a frame,
/// the headless one clears them, and they point into per-frame memory. They
/// are not state: a loaded game starts with none.
fn is_render_list(pointee: u16) -> bool {
    POINTEES[pointee as usize] == "DisplayListNode"
}

/// Whether a layout is nothing but `uintptr_t` words.
fn only_words(layout: u16) -> bool {
    static W: OnceLock<Vec<bool>> = OnceLock::new();
    W.get_or_init(|| {
        LAYOUTS
            .iter()
            .map(|items| !items.is_empty() && items.iter().all(|i| matches!(i.1, K::Word)))
            .collect()
    })
    .get(layout as usize)
    .copied()
    .unwrap_or(false)
}

fn named_by(name: &str) -> Option<(u32, u16)> {
    NAMED
        .binary_search_by(|(m, _, _)| m.cmp(&name))
        .ok()
        .map(|i| (NAMED[i].1, NAMED[i].2))
}

fn object_layout() -> Option<u16> {
    static L: OnceLock<Option<u16>> = OnceLock::new();
    *L.get_or_init(|| named_by("Object").map(|x| x.1))
}

/// `struct Object`'s first rawData slot, from include/object_fields.h.
const OBJ_RAW: u32 = 0x88;
/// `struct Object::bhvStack` (uintptr_t[8]).
const OBJ_BHV_STACK: u32 = 0x1D4;

fn pool_layout() -> Option<u16> {
    named_by("AllocOnlyPool").map(|x| x.1)
}

// --- the walk ------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    Var(usize),
    Chunk(usize),
}

#[derive(Debug, Clone, Copy)]
struct Region {
    addr: u32,
    layout: u16,
    /// For [`PTRS`] regions: how many pointers.
    count: u32,
    slot: Slot,
    /// Offset of `addr` in the slot's bytes.
    off: u32,
}

impl Region {
    fn slot_desc(&self) -> String {
        match self.slot {
            Slot::Var(i) => VARS[i].0.to_string(),
            Slot::Chunk(c) => format!("chunk {c}"),
        }
    }
}

fn layout_names(layout: u16) -> String {
    NAMED
        .iter()
        .filter(|n| n.2 == layout)
        .map(|n| n.0)
        .collect::<Vec<_>>()
        .join("|")
}

/// The layout of a run of pointers behind a pointer to pointers (`GraphNode **views`):
/// as many words as hold pointers, up to 64.
const PTRS: u16 = 0xFFFE;

fn rleaves(layout: u16, base: u32, count: u32, f: &mut impl FnMut(u32, K, u16, u32)) {
    if layout == PTRS {
        for i in 0..count {
            f(base + 4 * i, K::Word, PTRS, 4 * i);
        }
    } else {
        leaves(layout, base, f);
    }
}

/// A pointer or function pointer in a buffer, by offset: the value there is
/// portable. Kind 0 is a pointer, 1 a function table index.
pub type Reloc = (u32, u8);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buf {
    pub data: Vec<u8>,
    pub relocs: Vec<Reloc>,
}

#[derive(Debug, Clone, Default)]
pub struct Walked {
    /// One per entry of `VARS` (even ones that could not be read: empty).
    pub vars: Vec<Buf>,
    /// The malloc blocks reached, in the order they were found. Block `i` is
    /// at portable address `HEAP_BASE + block_bases[i]`.
    pub chunks: Vec<Buf>,
    pub chunk_addrs: Vec<u32>,
    pub chunk_lens: Vec<u32>,
    pub chunk_bases: Vec<u32>,
    /// Where each block was first found: (variable or block description, offset).
    pub found_by: Vec<(String, u32)>,
    /// GraphNode pointers whose target has an unknown type: (address, type).
    pub bad_nodes: Vec<(u32, u16)>,
    /// Pointers that could not be translated (and were set to null), as
    /// (variable or block description, offset, value).
    pub unresolved: Vec<(String, u32, u32)>,
    pub unresolved_count: u32,
}

struct Walk<'a> {
    build: Build,
    mem: &'a [u8],
    chunks: Vec<Chunk>,
    order: Vec<usize>,
    reached: HashMap<usize, usize>,
    regions: Vec<Region>,
    seen: HashSet<(u32, u16)>,
    queue: Vec<usize>,
    /// Pointers in this address range are not followed (the display list pool).
    skip: Vec<(u32, u32)>,
    found_by: Vec<(Slot, u32)>,
    bad_nodes: Vec<(u32, u16)>,
}

enum Tgt {
    Null,
    Heap(usize, u32),
    Global(u32),
    Unresolved,
}

impl<'a> Walk<'a> {
    fn chunk_of(&self, v: u32) -> Option<usize> {
        let i = self.chunks.partition_point(|c| c.user <= v);
        let c = self.chunks.get(i.checked_sub(1)?)?;
        (v - c.user < c.len).then_some(i - 1)
    }

    fn classify(&self, v: u32) -> Tgt {
        if v == 0 {
            return Tgt::Null;
        }
        if let Some(ci) = self.chunk_of(v) {
            return Tgt::Heap(ci, v - self.chunks[ci].user);
        }
        match to_canonical_global(self.build, v) {
            Some(g) => Tgt::Global(g),
            None => Tgt::Unresolved,
        }
    }

    /// Whether a word in a union is a pointer (and which sort).
    fn word_target(&self, v: u32, rodata_ok: bool) -> Tgt {
        match self.classify(v) {
            Tgt::Heap(c, o) => Tgt::Heap(c, o),
            Tgt::Global(g) if g >= DATA_START || rodata_ok => Tgt::Global(g),
            _ => Tgt::Null,
        }
    }

    fn reach(&mut self, ci: usize, by: (Slot, u32)) -> usize {
        if let Some(&i) = self.reached.get(&ci) {
            return i;
        }
        self.found_by.push(by);
        let i = self.order.len();
        self.order.push(ci);
        self.reached.insert(ci, i);
        i
    }

    fn push_region(&mut self, addr: u32, layout: u16, slot: Slot, off: u32) {
        self.push_region_n(addr, layout, 0, slot, off);
    }

    fn push_region_n(&mut self, addr: u32, layout: u16, count: u32, slot: Slot, off: u32) {
        if self.seen.insert((addr, layout)) {
            self.regions.push(Region {
                addr,
                layout,
                count,
                slot,
                off,
            });
            self.queue.push(self.regions.len() - 1);
        }
    }

    /// The struct a pointer to a `GraphNode` really points to.
    fn refine(&self, name: &str, addr: u32) -> Option<(u32, u16)> {
        if name == "GraphNode" {
            let t = u16::from_le_bytes(
                self.mem
                    .get(addr as usize..addr as usize + 2)?
                    .try_into()
                    .ok()?,
            );
            // Not a node at all (a pointer into the middle of one, or a stale
            // one into memory that has been reused): no struct to apply.
            let &(_, n) = NODE_TYPES.iter().find(|x| x.0 == t)?;
            // Nodes sit in circular doubly linked lists.
            let (prev, next) = (rd(self.mem, addr + 4)?, rd(self.mem, addr + 8)?);
            if prev == 0
                || next == 0
                || rd(self.mem, next + 4)? != addr
                || rd(self.mem, prev + 8)? != addr
            {
                return None;
            }
            return named_by(n);
        }
        named_by(name)
    }

    /// Whether the struct a pointer says it points to is plausibly there: a
    /// stale pointer (an object that was freed, say) can point into memory that
    /// has been reused for something else, and applying a layout to that would
    /// turn numbers into pointers.
    fn plausible(&self, name: &str, addr: u32) -> bool {
        match name {
            // The behavior script is always set, and in constant data.
            "SpawnInfo" => rd(self.mem, addr + 0x14)
                .is_some_and(|b| matches!(self.classify(b), Tgt::Global(_))),
            _ => true,
        }
    }

    fn process(&mut self, ri: usize) {
        let Region {
            addr,
            layout,
            count,
            slot,
            off: roff,
        } = self.regions[ri];
        // A main pool block that holds an allocation pool (its release handler
        // says so): the pool sits right after the header, reached by no pointer.
        if let (Slot::Chunk(_), Some(pl)) = (slot, pool_layout())
            && Some(layout) == named_by("MainPoolBlock").map(|x| x.1)
            && let Some(h) = rd(self.mem, addr + 8)
            && FNS.iter().any(|f| {
                f.2 == "alloc_only_pool_release_handler"
                    && h == if self.build == Build::Headless {
                        f.0
                    } else {
                        f.1
                    } as u32
            })
            && roff + 12 + 12
                <= self.chunks[match slot {
                    Slot::Chunk(c) => c,
                    _ => 0,
                }]
                .len
        {
            self.push_region(addr + 12, pl, slot, roff + 12);
        }
        let obj = object_layout();
        let mut found: Vec<(u32, K, u16, u32)> = Vec::new();
        rleaves(layout, addr, count, &mut |a, k, l, o| {
            if matches!(k, K::Ptr(_) | K::Word) {
                found.push((a, k, l, o));
            }
        });
        for (a, k, l, o) in found {
            if self.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                continue;
            }
            let Some(v) = rd(self.mem, a) else { continue };
            if let K::Ptr(pid) = k
                && is_render_list(pid)
            {
                continue;
            }
            let tgt = match k {
                K::Ptr(_) => self.classify(v),
                _ => self.word_target(v, self.rodata_slot(obj, l, o)),
            };
            if let Tgt::Heap(ci, off) = tgt {
                self.reach(ci, (slot, a - addr + roff));
                if let K::Ptr(pid) = k {
                    let name = POINTEES[pid as usize];
                    let info = if name == "GraphNode" {
                        let r = self.refine(name, v);
                        if r.is_none() && self.bad_nodes.len() < 40 {
                            let t = u16::from_le_bytes(
                                self.mem[v as usize..v as usize + 2].try_into().unwrap(),
                            );
                            self.bad_nodes.push((v, t));
                        }
                        r
                    } else {
                        pointee(pid)
                    };
                    if name == "ptr" {
                        let c = self.chunks[ci];
                        let mut n = 0;
                        while n < 64 && off + 4 * n + 4 <= c.len {
                            let Some(w) = rd(self.mem, v + 4 * n) else {
                                break;
                            };
                            if !matches!(self.word_target(w, false), Tgt::Heap(..) | Tgt::Global(_))
                            {
                                break;
                            }
                            n += 1;
                        }
                        if n > 0 && v % 4 == 0 {
                            self.push_region_n(v, PTRS, n, Slot::Chunk(ci), off);
                        }
                    }
                    if let Some((size, lay)) = info {
                        let c = self.chunks[ci];
                        if off + size <= c.len && v % 2 == 0 && self.plausible(name, v) {
                            self.push_region(v, lay, Slot::Chunk(ci), off);
                        }
                    }
                }
            }
        }
    }

    fn rodata_slot(&self, obj: Option<u16>, layout: u16, off: u32) -> bool {
        // arrays of uintptr_t (the segment table, the level script and geo layout
        // stacks) hold addresses of scripts and assets
        if only_words(layout) {
            return true;
        }
        Some(layout) == obj
            && (((OBJ_RAW..OBJ_RAW + 4 * 128).contains(&off)
                && (POINTER_SLOTS >> ((off - OBJ_RAW) / 4)) & 1 != 0)
                // bhvStack: addresses inside behavior scripts
                || (OBJ_BHV_STACK..OBJ_BHV_STACK + 32).contains(&off))
    }
}

/// Which variables the state has, in `VARS` order, as (index, address in
/// `build`'s memory, size, layout).
fn var_addr(build: Build, i: usize) -> (u32, u32, u16) {
    let v = &VARS[i];
    (if build == Build::Headless { v.1 } else { v.2 }, v.3, v.4)
}

/// The pools the render walk draws into (`gGfxAllocOnlyPool`, `gDisplayListHeap`):
/// not state, rebuilt every frame. Their addresses in `mem`.
pub fn draw_pools(mem: &[u8], build: Build) -> Vec<u32> {
    [":gGfxAllocOnlyPool"]
        .iter()
        .filter_map(|n| {
            let i = VARS.iter().position(|v| v.0.ends_with(n))?;
            rd(mem, var_addr(build, i).0).filter(|&p| p != 0)
        })
        .collect()
}

/// Why a state cannot be made portable, if it cannot: goddard (the intro's
/// Mario head and the like) keeps its own object graph in malloc blocks,
/// whose pointers are not described by types, so a state taken while it is
/// running does not translate. Its pool is only allocated while it runs.
pub fn unsupported(mem: &[u8], build: Build) -> Option<String> {
    let pool_at = |name: &str| {
        let i = VARS.iter().position(|v| v.0.ends_with(name))?;
        rd(mem, var_addr(build, i).0)
    };
    // The variable keeps pointing at the pool after it is freed, so look for
    // the pool in the main pool's list of blocks.
    let pool = pool_at(":sMemPoolForGoddard")?;
    if pool == 0 {
        return None;
    }
    let mut b = pool_at(":sPoolListHeadL")?;
    for _ in 0..100_000 {
        if b == 0 {
            return None;
        }
        if b + 12 == pool {
            return Some(
                "goddard (the intro's Mario head) is running; its state has no portable form"
                    .to_string(),
            );
        }
        b = rd(mem, b)?;
    }
    None
}

/// Finds the state in `mem`: the variables, and every malloc block reachable
/// from them by a pointer.
pub fn walk(mem: &[u8], build: Build) -> Result<Walked, String> {
    if let Some(why) = unsupported(mem, build) {
        return Err(why);
    }
    walk_unchecked(mem, build)
}

/// [`walk`] of a state that may be unsupported: what the game's own blocks
/// are, to reuse them.
#[allow(clippy::needless_range_loop)]
pub fn walk_unchecked(mem: &[u8], build: Build) -> Result<Walked, String> {
    let chunks = heap_chunks(mem, build)?;
    let mut w = Walk {
        build,
        mem,
        chunks,
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
            w.push_region(addr, layout, Slot::Var(i), 0);
        }
    }
    while let Some(ri) = w.queue.pop() {
        w.process(ri);
    }
    // Words no layout describes that hold exactly the address of a block (the
    // links of allocation pools, whose owners sit in structs reached only by
    // value): followed, and translated like pointers.
    let mut exact: Vec<(usize, u32, usize, u32)> = Vec::new(); // (chunk, offset, target chunk, offset in it)
    {
        let mut cover: HashMap<usize, Vec<bool>> = HashMap::new();
        for r in &w.regions {
            if let Slot::Chunk(ci) = r.slot {
                let c = cover
                    .entry(ci)
                    .or_insert_with(|| vec![false; w.chunks[ci].len as usize]);
                rleaves(r.layout, r.addr, r.count, &mut |a, k, _, _| {
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
        let mut done = 0;
        while done < w.order.len() {
            let ci = w.order[done];
            done += 1;
            let c = w.chunks[ci];
            let cov = cover.get(&ci);
            // The link of an allocation pool's blocks: the first word of a block
            // is the exact address of the pool's previous block. (Other words are
            // not looked at: in a block of s16 data, one in a few thousand looks
            // like an address.)
            let Some(v) = rd(mem, c.user) else { continue };
            if v == 0 || v % 4 != 0 {
                continue;
            }
            if cov.is_some_and(|x| x[0]) {
                continue;
            }
            if let Some(i) = w.chunk_of(v).filter(|&i| w.chunks[i].user == v && i != ci) {
                w.reach(i, (Slot::Chunk(ci), 0));
                exact.push((ci, 0, i, 0));
            }
        }
    }

    // Output buffers.
    let mut out = Walked::default();
    for i in 0..VARS.len() {
        let (addr, size, _) = var_addr(build, i);
        out.vars.push(Buf {
            data: mem[addr as usize..(addr + size) as usize].to_vec(),
            relocs: Vec::new(),
        });
    }
    let mut base = 0u32;
    for &ci in &w.order {
        let c = w.chunks[ci];
        out.chunk_addrs.push(c.user);
        out.chunk_lens.push(c.len);
        out.chunk_bases.push(base);
        out.chunks.push(Buf {
            data: mem[c.user as usize..(c.user + c.len) as usize].to_vec(),
            relocs: Vec::new(),
        });
        base += (c.len + 15) & !15;
    }
    for &(ci, o, ti, toff) in &exact {
        let buf = &mut out.chunks[w.reached[&ci]];
        let val = HEAP_BASE + out.chunk_bases[w.reached[&ti]] + toff;
        buf.data[o as usize..o as usize + 4].copy_from_slice(&val.to_le_bytes());
        buf.relocs.push((o, 0));
    }
    let regions = std::mem::take(&mut w.regions);
    out.found_by = w
        .found_by
        .iter()
        .map(|&(s, o)| match s {
            Slot::Var(i) => (VARS[i].0.to_string(), o),
            Slot::Chunk(c) => (format!("block@{:#x}", w.chunks[c].user), o),
        })
        .collect();
    let obj = object_layout();
    let pool = pool_layout();
    for r in &regions {
        let mut items: Vec<(u32, K, u16, u32)> = Vec::new();
        rleaves(r.layout, r.addr, r.count, &mut |a, k, l, o| {
            items.push((a, k, l, o))
        });
        for (a, k, l, o) in items {
            if w.skip.iter().any(|&(lo, hi)| a >= lo && a < hi) {
                // the display list pool (zeroed below)
                continue;
            }
            let Some(v) = rd(mem, a) else { continue };
            if let K::Ptr(pid) = k
                && is_render_list(pid)
            {
                let off = (a - r.addr + r.off) as usize;
                let buf = match r.slot {
                    Slot::Var(i) => &mut out.vars[i],
                    Slot::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
                };
                buf.data[off..off + 4].copy_from_slice(&[0; 4]);
                continue;
            }
            let canon: Option<(u32, u8)> = match k {
                K::Ptr(_) => match w.classify(v) {
                    Tgt::Null => None,
                    Tgt::Heap(ci, off) => w
                        .reached
                        .get(&ci)
                        .map(|&bi| (HEAP_BASE + out.chunk_bases[bi] + off, 0)),
                    Tgt::Global(g) => Some((g, 0)),
                    Tgt::Unresolved => {
                        note_unresolved(&mut out, r, a, v);
                        None
                    }
                },
                K::Word => match w.word_target(v, w.rodata_slot(obj, l, o)) {
                    Tgt::Heap(ci, off) => w
                        .reached
                        .get(&ci)
                        .map(|&bi| (HEAP_BASE + out.chunk_bases[bi] + off, 0)),
                    Tgt::Global(g) => Some((g, 0)),
                    _ => None,
                },
                K::Fn => {
                    if v == 0 {
                        None
                    } else {
                        match fn_to_canonical(build, v) {
                            Some(c) => Some((c, 1)),
                            None => {
                                note_unresolved(&mut out, r, a, v);
                                None
                            }
                        }
                    }
                }
                _ => None,
            };
            let Some((val, kind)) = canon else { continue };
            let off = a - r.addr + r.off;
            let buf = match r.slot {
                Slot::Var(i) => &mut out.vars[i],
                Slot::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
            };
            buf.data[off as usize..off as usize + 4].copy_from_slice(&val.to_le_bytes());
            if kind != 9 {
                buf.relocs.push((off, kind));
            }
        }
        // The pool's extra headless field and (below) the display list pool.
        if Some(r.layout) == pool {
            let off = r.off as usize;
            let buf = match r.slot {
                Slot::Var(i) => &mut out.vars[i],
                Slot::Chunk(ci) => &mut out.chunks[w.reached[&ci]],
            };
            if buf.data.len() >= off + 16 {
                buf.data[off + 12..off + 16].copy_from_slice(&[0; 4]);
            }
        }
    }
    // The display list pool is rebuilt every frame; its blocks are not state
    // (and live in different memory in the two builds).
    for p in draw_pools(mem, build) {
        let Some(ci) = w.chunk_of(p) else { continue };
        let Some(&bi) = w.reached.get(&ci) else {
            continue;
        };
        let off = (p - w.chunks[ci].user) as usize;
        let buf = &mut out.chunks[bi];
        if buf.data.len() >= off + 16 {
            buf.data[off..off + 16].copy_from_slice(&[0; 16]);
            buf.relocs
                .retain(|r| (r.0 as usize) < off || (r.0 as usize) >= off + 12);
        }
    }
    for b in out.vars.iter_mut().chain(out.chunks.iter_mut()) {
        // Pointers that ended up null (a render list, a cleared pool) need no translation.
        let data = &b.data;
        b.relocs
            .retain(|&(o, _)| data[o as usize..o as usize + 4] != [0; 4]);
        b.relocs.sort_unstable();
        b.relocs.dedup();
    }
    let _ = layout_size;
    Ok(out)
}

fn note_unresolved(out: &mut Walked, r: &Region, a: u32, v: u32) {
    out.unresolved_count += 1;
    if out.unresolved.len() < 200 {
        let what = match r.slot {
            Slot::Var(i) => VARS[i].0.to_string(),
            Slot::Chunk(c) => format!("chunk@{:#x}", c),
        };
        out.unresolved.push((
            format!(
                "{what} [region {} layout {} ({}) at +{:#x}]",
                r.slot_desc(),
                r.layout,
                layout_names(r.layout),
                r.off
            ),
            a - r.addr,
            v,
        ));
    }
}

/// The malloc blocks that are part of `mem`'s game state (address, usable
/// length): what a state loaded over it can reuse.
pub fn game_blocks(mem: &[u8], build: Build) -> Result<Vec<(u32, u32)>, String> {
    let w = walk_unchecked(mem, build)?;
    Ok(w.chunk_addrs.into_iter().zip(w.chunk_lens).collect())
}

/// The blocks of the display list pool (the rendering build's, which draw
/// from malloc): they are not part of a state, but loading one drops the
/// pool's list of them, so whoever loads it has to free them. Empty in the
/// headless build, whose pool is in an arena.
pub fn display_list_blocks(mem: &[u8], build: Build) -> Result<Vec<u32>, String> {
    let chunks = heap_chunks(mem, build)?;
    let mut out = Vec::new();
    let is_block = |b: u32| chunks.binary_search_by(|c| c.user.cmp(&b)).is_ok();
    for pool in draw_pools(mem, build) {
        let mut b = rd(mem, pool).unwrap_or(0);
        while b != 0 && is_block(b) && out.len() < 100_000 {
            out.push(b);
            b = rd(mem, b).unwrap_or(0);
        }
    }
    Ok(out)
}
