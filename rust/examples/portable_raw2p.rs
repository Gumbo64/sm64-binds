//! Exports a raw rendering-build save state ("SM64VISU", `save_state()` bytes, what
//! `portable_check quick` writes as raw_<frame>.bin) with the Rust translator, the same
//! one the browser runs as WASM: `portable_raw2p <raw.bin> <out.p>`. For re-making
//! a directory of own_<frame>.p states after a change to the translator.
use sm64_binds::portable::{Build, Portable, export};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let b = std::fs::read(&a[1]).unwrap();
    assert_eq!(&b[..8], b"SM64VISU");
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (stack, heap) = (u(32), u(36));
    let ds = Portable::data_start(Build::Visual) as usize;
    let mut mem = vec![0u8; ds + heap];
    mem[..stack].copy_from_slice(&b[44..44 + stack]);
    mem[ds..].copy_from_slice(&b[44 + stack..]);
    let p = export(&mem, Build::Visual).unwrap();
    println!(
        "{}: {} blocks, {} ghosts, {} unresolved",
        a[1],
        p.chunks.len(),
        p.ghosts,
        p.unresolved_count
    );
    std::fs::write(&a[2], p.to_bytes()).unwrap();
}
