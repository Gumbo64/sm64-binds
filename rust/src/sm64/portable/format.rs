//! The byte format of a portable state ("SM64PORT").
//!
//! The layout is documented on [`Portable::to_bytes`]. Version 2 (current) adds
//! one header word, the number of ghosts (`ghosts.rs`); version 1 states (no
//! ghosts) still load. A state records the build it came from, that build's id
//! and a fingerprint of the tables ([`tables_id`]): a state is only loaded with
//! tables whose variables are the same, in the same order.
//!
//! Guarantees: `from_bytes(to_bytes(p))` gives back the state part of `p` (the
//! diagnostics are not stored), and `to_bytes` of that is the same bytes.
//!
//! Read next: `load.rs`.

use super::builds::Build;
use super::state::{Buf, Portable};
use super::tables::{POINTEES, VARS};

const MAGIC: &[u8; 8] = b"SM64PORT";
/// Version 2 adds ghosts (the last `ghosts` blocks); version 1 states have none.
const VERSION: u32 = 2;

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
}
