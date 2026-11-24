//! What a raw rendering-build save state's export does with one word: `portable_probe <raw> <addr>`.
use sm64_binds::portable::{Build, Portable, walk};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).unwrap();
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (stack, heap) = (u(32), u(36));
    let ds = Portable::data_start(Build::Visual) as usize;
    let mut mem = vec![0u8; ds + heap];
    mem[..stack].copy_from_slice(&b[44..44 + stack]);
    mem[ds..].copy_from_slice(&b[44 + stack..]);
    let addr = u32::from_str_radix(a[2].trim_start_matches("0x"), 16).unwrap();
    let w = walk(&mem, Build::Visual).unwrap();
    let rd = |x: u32| u32::from_le_bytes(mem[x as usize..x as usize + 4].try_into().unwrap());
    println!("memory {addr:#x} = {:#x}", rd(addr));
    for (i, (&ca, &cl)) in w.chunk_addrs.iter().zip(&w.chunk_lens).enumerate() {
        if addr >= ca && addr < ca + cl {
            let off = addr - ca;
            let buf = &w.chunks[i];
            let v = u32::from_le_bytes(buf.data[off as usize..off as usize + 4].try_into().unwrap());
            let r = buf.relocs.iter().find(|r| r.0 == off);
            println!("in block {i} @{ca:#x}+{off:#x} (found by {:?}): exported {v:#x}, reloc {r:?}", w.found_by.get(i));
            let near: Vec<_> = buf.relocs.iter().filter(|r| r.0 + 0x40 >= off && r.0 <= off + 0x40).collect();
            println!("relocs nearby: {near:?}");
        }
    }
}
