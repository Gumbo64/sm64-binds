//! dlmalloc's heap.

use super::builds::Build;
use super::tables::*;

/// A malloc block in use: its user address and usable length.
#[derive(Debug, Clone, Copy)]
pub struct Chunk {
    pub user: u32,
    pub len: u32,
}

pub(super) fn rd(mem: &[u8], a: u32) -> Option<u32> {
    Some(u32::from_le_bytes(
        mem.get(a as usize..a as usize + 4)?.try_into().ok()?,
    ))
}

/// Every malloc block in use, found by walking dlmalloc's chunks from the
/// start of its heap to its top.
pub fn heap_chunks(mem: &[u8], build: Build) -> Result<Vec<Chunk>, String> {
    Ok(heap_all(mem, build)?
        .into_iter()
        .filter(|c| c.2)
        .map(|c| Chunk {
            user: c.0 + 8,
            len: c.1 - 4,
        })
        .collect())
}

/// Every dlmalloc chunk, in use or free: (chunk address, size, in use).
pub fn heap_all(mem: &[u8], build: Build) -> Result<Vec<(u32, u32, bool)>, String> {
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
        out.push((c, size, next_head & 1 != 0));
        c = next;
    }
    if c != top {
        return Err(format!(
            "heap walk ended at {c:#x}, not at its top {top:#x}"
        ));
    }
    Ok(out)
}
