//! An object slot of a portable state: `portable_obj <state.p> <slot>`: its words, pointers marked.
use sm64_binds::portable::Portable;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = Portable::from_bytes(&std::fs::read(&a[1]).unwrap()).unwrap();
    if let Ok(vn) = std::env::var("VAR") {
        let i = (0..Portable::VAR_COUNT).find(|&i| Portable::var_name(i).ends_with(&vn)).unwrap();
        let v = &p.vars[i];
        for o in (0..v.data.len().min(0x100)).step_by(4) {
            println!("{o:#05x}: {:#010x}", u32::from_le_bytes(v.data[o..o + 4].try_into().unwrap()));
        }
        return;
    }
    let slot: u32 = a[2].parse().unwrap();
    let i = (0..Portable::VAR_COUNT).find(|&i| Portable::var_name(i).ends_with(":gObjectPool")).unwrap();
    let v = &p.vars[i];
    let base = slot * 0x260;
    for o in (0..0x260).step_by(4) {
        let w = u32::from_le_bytes(v.data[(base + o) as usize..(base + o + 4) as usize].try_into().unwrap());
        let r = v.relocs.iter().find(|r| r.0 == base + o).map(|r| if r.1 == 1 { " fn" } else { " ptr" }).unwrap_or("");
        println!("{o:#05x}: {w:#010x}{r}");
    }
}
