//! Pointers found in object slots that no object field declares as a pointer:
//! `portable_slots <state.p>...`.
use sm64_binds::portable::Portable;
const POINTER_SLOTS: u128 = 0x42000000004778000000;
fn main() {
    let i = (0..Portable::VAR_COUNT).find(|&i| Portable::var_name(i).ends_with(":gObjectPool")).unwrap();
    for f in std::env::args().skip(1) {
        let p = Portable::from_bytes(&std::fs::read(&f).unwrap()).unwrap();
        let v = &p.vars[i];
        let mut n = 0;
        for &(o, k) in &v.relocs {
            let (slot, field) = (o / 0x260, o % 0x260);
            if !(0x88..0x88 + 0x50 * 4).contains(&field) { continue; }
            let idx = (field - 0x88) / 4;
            if POINTER_SLOTS >> idx & 1 != 0 { continue; }
            let w = u32::from_le_bytes(v.data[o as usize..o as usize + 4].try_into().unwrap());
            n += 1;
            println!("{f}: object {slot} field {idx:#x} (+{field:#x}) kind {k} value {w:#x}");
        }
        println!("{f}: {n}");
    }
}
