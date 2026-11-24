//! The malloc blocks in use that a state does not reach: `portable_unreached <raw visual save state>`
//! (the rendering build's own save_state bytes, "SM64VISU").
use sm64_binds::portable::{Build, Portable, display_list_blocks, heap_chunks, walk};
fn main() {
    let load = |f: &str| -> Vec<u8> {
        let b = std::fs::read(f).unwrap();
        assert_eq!(&b[..8], b"SM64VISU");
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
        let (stack, heap) = (u(32), u(36));
        let ds = Portable::data_start(Build::Visual) as usize;
        let mut mem = vec![0u8; ds + heap];
        mem[..stack].copy_from_slice(&b[44..44 + stack]);
        mem[ds..].copy_from_slice(&b[44 + stack..]);
        mem
    };
    let boot = std::env::args().nth(2).map(|f| load(&f));
    let f = std::env::args().nth(1).unwrap();
    let b = std::fs::read(&f).unwrap();
    assert_eq!(&b[..8], b"SM64VISU");
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (stack, heap) = (u(32), u(36));
    let ds = Portable::data_start(Build::Visual) as usize;
    let mut mem = vec![0u8; ds + heap];
    mem[..stack].copy_from_slice(&b[44..44 + stack]);
    mem[ds..].copy_from_slice(&b[44 + stack..]);
    let chunks = heap_chunks(&mem, Build::Visual).unwrap();
    let w = walk(&mem, Build::Visual).unwrap();
    let dl: std::collections::HashSet<u32> = display_list_blocks(&mem, Build::Visual)
        .unwrap()
        .into_iter()
        .collect();
    let reached: std::collections::HashSet<u32> = w.chunk_addrs.iter().copied().collect();
    let mut n = 0;
    for c in &chunks {
        if reached.contains(&c.user) || dl.contains(&c.user) {
            continue;
        }
        if let Some(bm) = &boot {
            let r = c.user as usize..(c.user + c.len) as usize;
            if bm.get(r.clone()) == mem.get(r) {
                continue;
            }
        }
        n += 1;
        let head: Vec<String> = (0..8)
            .map(|k| {
                format!(
                    "{:08x}",
                    u32::from_le_bytes(
                        mem[(c.user + 4 * k) as usize..(c.user + 4 * k + 4) as usize]
                            .try_into()
                            .unwrap()
                    )
                )
            })
            .collect();
        println!(
            "unreached {:#x} len {:#x}: {}",
            c.user,
            c.len,
            head.join(" ")
        );
    }
    // Words of reached blocks that point into a block in use but are not translated.
    let inside = |v: u32| {
        chunks
            .iter()
            .find(|c| v >= c.user && v < c.user + c.len)
            .map(|c| (c.user, v - c.user))
    };
    let mut hits = 0;
    for (i, buf) in w.chunks.iter().enumerate() {
        let rel: std::collections::HashSet<u32> = buf.relocs.iter().map(|r| r.0).collect();
        let mut here = Vec::new();
        for o in (0..buf.data.len().saturating_sub(3)).step_by(4) {
            if rel.contains(&(o as u32)) {
                continue;
            }
            let v = u32::from_le_bytes(buf.data[o..o + 4].try_into().unwrap());
            if let Some((blk, off)) = inside(v) {
                if v % 4 == 0 && v >= 0xc50000 {
                    here.push(format!("+{o:#x}={v:#x}(blk {blk:#x}+{off:#x})"));
                }
            }
        }
        if !here.is_empty() {
            hits += here.len();
            println!(
                "block {i} @{:#x} len {:#x} found by {:?}: {} untranslated: {}",
                w.chunk_addrs[i],
                w.chunk_lens[i],
                w.found_by.get(i),
                here.len(),
                here.iter().take(5).cloned().collect::<Vec<_>>().join(" ")
            );
        }
    }
    println!("{hits} untranslated words point into blocks");
    println!(
        "{} blocks in use, {} reached, {} display list, {} unreached",
        chunks.len(),
        reached.len(),
        dl.len(),
        n
    );
}
