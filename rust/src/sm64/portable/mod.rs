//! Portable save states: the game's state in a form either build can load.
//!
//! The headless WASM (`build/us_web_headless`, what Rust, Python and the nodes
//! run) and the rendering WASM (`build/us_web`, what the browser runs) are
//! linked from the same objects, but at different addresses, and their
//! mallocs hand out different blocks. A [`Portable`] state holds the game's
//! variables and the malloc blocks they reach, with every pointer and
//! function pointer replaced by a build-neutral one, so it loads in either
//! build and the game goes on identically. See `docs/portable-state.md`.
//!
//! This module only needs `std`: the same source is compiled to WASM for the
//! browser (`rust/portable-wasm`).

mod layout;
#[rustfmt::skip]
mod tables;
mod walk;

use tables::{POINTEES, VARS};
pub use walk::{
    Buf, Build, Walked, display_list_blocks, game_blocks, heap_all, heap_chunks, unsupported, walk,
};

const MAGIC: &[u8; 8] = b"SM64PORT";
/// Version 2 adds ghosts (the last `ghosts` blocks); version 1 states have none.
const VERSION: u32 = 2;

/// The game's state, translated to build-neutral form by [`export`].
#[derive(Debug, Clone)]
pub struct Portable {
    /// The build the state was exported from.
    pub source: Build,
    pub vars: Vec<Buf>,
    pub chunks: Vec<Buf>,
    pub chunk_bases: Vec<u32>,
    /// How many of the last `chunks` are ghosts: raw copies of freed memory that
    /// stale pointers point into. They are allocated and written on a load and
    /// never freed; no type walk touches them.
    pub ghosts: usize,
    /// The stale pointers translated into ghosts (variable or block, offset, original value).
    pub ghost_refs: Vec<(String, u32, u32)>,
    /// Pointers that had no translation and were set to null.
    pub unresolved: Vec<(String, u32, u32)>,
    pub unresolved_count: u32,
    /// Where each block was first found (for diagnostics).
    pub found_by: Vec<(String, u32)>,
}

/// A group of bytes to write into the target's memory.
pub type Patch = (u32, Vec<u8>);

/// A fingerprint of the tables (the variables and their order), so a state
/// is only loaded with the tables that wrote it.
fn tables_id() -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for v in VARS {
        for b in v.0.bytes().chain(v.3.to_le_bytes()) {
            h = (h ^ b as u32).wrapping_mul(0x01000193);
        }
    }
    h ^ POINTEES.len() as u32
}

/// Translates the state in `mem` (a copy of the linear memory of a `build`
/// game, at least up to its heap's top) to portable form.
pub fn export(mem: &[u8], build: Build) -> Result<Portable, String> {
    let w = walk(mem, build)?;
    Ok(Portable {
        source: build,
        vars: w.vars,
        chunks: w.chunks,
        chunk_bases: w.chunk_bases,
        ghosts: w.ghosts,
        ghost_refs: w.ghost_refs,
        unresolved: w.unresolved,
        unresolved_count: w.unresolved_count,
        found_by: w.found_by,
    })
}

fn put_buf(out: &mut Vec<u8>, b: &Buf) {
    out.extend_from_slice(&(b.data.len() as u32).to_le_bytes());
    out.extend_from_slice(&(b.relocs.len() as u32).to_le_bytes());
    out.extend_from_slice(&b.data);
    for &(o, k) in &b.relocs {
        out.extend_from_slice(&((o << 1) | k as u32).to_le_bytes());
    }
}

struct Reader<'a>(&'a [u8], usize);

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, String> {
        let b = self
            .0
            .get(self.1..self.1 + 4)
            .ok_or("portable state is truncated")?;
        self.1 += 4;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }
    fn bytes(&mut self, n: usize) -> Result<&[u8], String> {
        let b = self
            .0
            .get(self.1..self.1 + n)
            .ok_or("portable state is truncated")?;
        self.1 += n;
        Ok(b)
    }
    fn buf(&mut self) -> Result<Buf, String> {
        let len = self.u32()? as usize;
        let n = self.u32()? as usize;
        let data = self.bytes(len)?.to_vec();
        let mut relocs = Vec::with_capacity(n);
        for _ in 0..n {
            let v = self.u32()?;
            relocs.push((v >> 1, (v & 1) as u8));
        }
        Ok(Buf { data, relocs })
    }
}

impl Portable {
    /// The state as bytes:
    ///
    /// ```text
    ///   0   8  magic "SM64PORT"
    ///   8   4  format version (little endian), currently 2
    ///  12   4  source build: 0 headless, 1 visual
    ///  16  20  source build id (SHA-1 of the source WASM's code)
    ///  36   4  tables id (which variables, in which order)
    ///  40   4  number of variables
    ///  44   4  number of malloc blocks (the ghosts included)
    ///  48   4  how many of the last blocks are ghosts (version 2; version 1 has no such field)
    ///  52   -  the variables, then the blocks; each is
    ///             u32 length, u32 number of relocations, the bytes,
    ///             then the relocations as u32 (offset << 1 | 1 for a
    ///             function pointer, 0 for a pointer)
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.source.index() as u32).to_le_bytes());
        let id = self.source.build_id();
        for i in 0..20 {
            out.push(u8::from_str_radix(&id[2 * i..2 * i + 2], 16).unwrap_or(0));
        }
        out.extend_from_slice(&tables_id().to_le_bytes());
        out.extend_from_slice(&(self.vars.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.ghosts as u32).to_le_bytes());
        for b in self.vars.iter().chain(&self.chunks) {
            put_buf(&mut out, b);
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Portable, String> {
        if bytes.len() < 48 || &bytes[..8] != MAGIC {
            return Err("not a portable sm64 state".into());
        }
        let mut r = Reader(bytes, 8);
        let version = r.u32()?;
        if version != VERSION && version != 1 {
            return Err(format!(
                "portable state format version {version}, expected 1 or {VERSION}"
            ));
        }
        let source = match r.u32()? {
            0 => Build::Headless,
            1 => Build::Visual,
            n => return Err(format!("unknown source build {n}")),
        };
        r.bytes(20)?;
        if r.u32()? != tables_id() {
            return Err("portable state was made with different tables".into());
        }
        let nv = r.u32()? as usize;
        let nc = r.u32()? as usize;
        if nv != VARS.len() {
            return Err(format!(
                "portable state has {nv} variables, expected {}",
                VARS.len()
            ));
        }
        let ghosts = if version >= 2 { r.u32()? as usize } else { 0 };
        if ghosts > nc {
            return Err("portable state has more ghosts than blocks".into());
        }
        let vars = (0..nv).map(|_| r.buf()).collect::<Result<Vec<_>, _>>()?;
        let chunks = (0..nc).map(|_| r.buf()).collect::<Result<Vec<_>, _>>()?;
        let mut chunk_bases = Vec::new();
        let mut base = 0u32;
        for c in &chunks {
            chunk_bases.push(base);
            base += (c.data.len() as u32 + 15) & !15;
        }
        Ok(Portable {
            source,
            vars,
            chunks,
            chunk_bases,
            ghosts,
            ghost_refs: Vec::new(),
            unresolved: Vec::new(),
            unresolved_count: 0,
            found_by: Vec::new(),
        })
    }

    /// The bytes of the ghosts: what a load leaks.
    pub fn ghost_bytes(&self) -> usize {
        self.chunks[self.chunks.len() - self.ghosts..]
            .iter()
            .map(|c| c.data.len())
            .sum()
    }

    /// The malloc sizes to allocate in the target, one per block, in order.
    pub fn chunk_sizes(&self) -> Vec<u32> {
        self.chunks.iter().map(|c| c.data.len() as u32).collect()
    }

    /// Where each block of this state goes in the target: the target's own
    /// game blocks (`old`, from [`game_blocks`]) are reused, best fit first
    /// (a block that is freed later is freed whole, so reusing one needs no
    /// allocator call); 0 for the blocks that fit none, which the caller must
    /// allocate (`malloc` of the size from [`Portable::chunk_sizes`]).
    pub fn place(&self, old: &[(u32, u32)]) -> Vec<u32> {
        let mut free: Vec<(u32, u32)> = old.iter().map(|&(a, l)| (l, a)).collect();
        free.sort_unstable();
        // Ghosts are always allocated: the game never frees them.
        let mut order: Vec<usize> = (0..self.chunks.len() - self.ghosts).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.chunks[i].data.len()));
        let mut addrs = vec![0; self.chunks.len()];
        for i in order {
            let need = self.chunks[i].data.len() as u32;
            let at = free.partition_point(|f| f.0 < need);
            if at < free.len() {
                addrs[i] = free.remove(at).1;
            }
        }
        addrs
    }

    fn pointer_to(&self, target: Build, v: u32, addrs: &[u32], bad: &mut u32) -> u32 {
        if v >= walk::HEAP_BASE {
            let rel = v - walk::HEAP_BASE;
            let i = self.chunk_bases.partition_point(|&b| b <= rel);
            if let Some(i) = i.checked_sub(1)
                && let Some(&a) = addrs.get(i)
            {
                return a + (rel - self.chunk_bases[i]);
            }
        } else if let Some(a) = walk::from_canonical_global(target, v) {
            return a;
        }
        *bad += 1;
        0
    }

    fn translated(&self, target: Build, b: &Buf, addrs: &[u32], bad: &mut u32) -> Vec<u8> {
        let mut data = b.data.clone();
        for &(o, kind) in &b.relocs {
            let o = o as usize;
            let v = u32::from_le_bytes(data[o..o + 4].try_into().unwrap());
            let t = if kind == 1 {
                walk::fn_from_canonical(target, v).unwrap_or_else(|| {
                    *bad += 1;
                    0
                })
            } else {
                self.pointer_to(target, v, addrs, bad)
            };
            data[o..o + 4].copy_from_slice(&t.to_le_bytes());
        }
        data
    }

    /// The pointers this state has that `target` has no equivalent of, as (variable or
    /// block, offset, portable value): what [`Portable::patches`] counts and nulls.
    pub fn untranslatable(&self, target: Build, chunk_addrs: &[u32]) -> Vec<(String, u32, u32)> {
        let mut out = Vec::new();
        let mut one = |name: String, b: &Buf| {
            for &(o, kind) in &b.relocs {
                let v = u32::from_le_bytes(b.data[o as usize..o as usize + 4].try_into().unwrap());
                let mut bad = 0;
                if kind == 1 {
                    if walk::fn_from_canonical(target, v).is_none() {
                        bad = 1;
                    }
                } else {
                    self.pointer_to(target, v, chunk_addrs, &mut bad);
                }
                if bad > 0 {
                    out.push((name.clone(), o, v));
                }
            }
        };
        for (i, b) in self.vars.iter().enumerate() {
            one(VARS[i].0.to_string(), b);
        }
        for (i, b) in self.chunks.iter().enumerate() {
            one(format!("block {i}"), b);
        }
        out
    }

    /// What to write into `target`'s memory to load this state, once the
    /// blocks have been allocated there (`chunk_addrs[i]` holds
    /// [`Portable::chunk_sizes`]`()[i]` bytes). Also returns how many
    /// pointers had no translation (set to null).
    pub fn patches(&self, target: Build, chunk_addrs: &[u32]) -> Result<(Vec<Patch>, u32), String> {
        if chunk_addrs.len() != self.chunks.len() {
            return Err("wrong number of block addresses".into());
        }
        let mut bad = 0;
        let mut out: Vec<Patch> = Vec::new();
        for (i, b) in self.vars.iter().enumerate() {
            if b.data.is_empty() {
                continue;
            }
            let a = if target == Build::Headless {
                VARS[i].1
            } else {
                VARS[i].2
            };
            out.push((a, self.translated(target, b, chunk_addrs, &mut bad)));
        }
        for (i, b) in self.chunks.iter().enumerate() {
            out.push((
                chunk_addrs[i],
                self.translated(target, b, chunk_addrs, &mut bad),
            ));
        }
        // The display list pools start empty (their blocks are not state), and the
        // headless build draws into an arena of its own: its pools are marked.
        for name in [":gGfxAllocOnlyPool"] {
            let Some(i) = VARS.iter().position(|v| v.0.ends_with(name)) else {
                continue;
            };
            let at = if target == Build::Headless {
                VARS[i].1
            } else {
                VARS[i].2
            };
            let Some((_, d)) = out.iter().find(|p| p.0 == at) else {
                continue;
            };
            let p = u32::from_le_bytes(d[..4].try_into().unwrap());
            let inside = chunk_addrs
                .iter()
                .zip(&self.chunks)
                .any(|(&a, c)| p >= a && p + 16 <= a + c.data.len() as u32);
            if p != 0 && inside {
                out.push((p, vec![0; 12]));
                if target == Build::Headless {
                    out.push((p + 12, 1u32.to_le_bytes().to_vec()));
                }
            }
        }
        Ok((out, bad))
    }

    /// Where the headless build's render walk cache is, for invalidating it
    /// after a load: (generation address, count address).
    pub fn geo_cache() -> [u32; 2] {
        tables::GEO_CACHE
    }

    /// Where constant data ends in `build` (its `.data` starts): memory below
    /// it is the stack and read-only data, which no state includes.
    pub fn data_start(build: Build) -> u32 {
        tables::DATA_STARTS[build.index()]
    }

    /// The rendering build's texture cache (address, size), to reset after a
    /// state is loaded: it maps texture addresses to GL textures.
    pub fn texture_cache() -> (u32, u32) {
        tables::TEXTURE_CACHE
    }

    /// The rendering build's renderer state (address, size): what it knows of
    /// the live GL context (the GL state it mirrors, shader programs, the
    /// texture cache, the window's size). It is not part of the game, so a
    /// load keeps it, whatever the loaded memory held there.
    pub fn renderer() -> (u32, u32) {
        tables::RENDERER
    }

    /// The addresses of the allocator's state, per build.
    pub fn gm(build: Build) -> u32 {
        tables::GM[build.index()]
    }

    /// The address and size of variable `i` in `build`'s memory.
    pub fn var_at(i: usize, build: Build) -> (u32, u32) {
        let v = &VARS[i];
        (if build == Build::Headless { v.1 } else { v.2 }, v.3)
    }

    /// How many variables a state has.
    pub const VAR_COUNT: usize = VARS.len();

    /// Names of the variables, for diagnostics.
    pub fn var_name(i: usize) -> &'static str {
        VARS[i].0
    }

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
