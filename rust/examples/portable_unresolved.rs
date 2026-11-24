//! Unresolved pointers of an export: `portable_unresolved headless <rom> <m64> <frame>` or
//! `portable_unresolved raw <raw visual snapshot>`.
use sm64_binds::portable::{Build, Portable, export};
use sm64_binds::{GamePad, SM64GameGenerator};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p: Portable = if a[1] == "headless" {
        let b = std::fs::read(&a[3]).unwrap();
        let n = u32::from_le_bytes(b[0x18..0x1c].try_into().unwrap()) as usize;
        let f: usize = a[4].parse().unwrap();
        let g = SM64GameGenerator::from_file(&a[2]).unwrap();
        let mut game = g.create_game().unwrap();
        for i in 0..f.min(n) {
            let o = 0x400 + 4 * i;
            game.step_game(GamePad::new(u16::from_be_bytes([b[o], b[o + 1]]), b[o + 2] as i8, b[o + 3] as i8)).unwrap();
        }
        game.export_portable().unwrap()
    } else {
        let b = std::fs::read(&a[2]).unwrap();
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
        let ds = Portable::data_start(Build::Visual) as usize;
        let mut mem = vec![0u8; ds + u(36)];
        mem[..u(32)].copy_from_slice(&b[44..44 + u(32)]);
        mem[ds..].copy_from_slice(&b[44 + u(32)..]);
        export(&mem, Build::Visual).unwrap()
    };
    println!("unresolved {} ({} blocks)", p.unresolved_count, p.chunks.len());
    for u in p.unresolved.iter().take(20) {
        println!("  {} +{:#x} = {:#x}", u.0, u.1, u.2);
    }
}
