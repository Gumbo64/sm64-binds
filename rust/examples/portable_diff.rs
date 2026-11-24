//! How two portable states differ: `portable_diff <a.p> <b.p> [max lines]`.
//!
//! Pointer-aware: heap blocks are paired through the pointers that reach them (the two states
//! may number their blocks differently), then every word is compared: plain values must be
//! equal, global pointers equal, heap pointers must reach paired blocks at the same offset.
use sm64_binds::portable::{Buf, Portable};
use std::collections::{BTreeMap, HashMap, VecDeque};

const HEAP: u32 = 0x4000_0000;

fn block(p: &Portable, v: u32) -> Option<(usize, u32)> {
    let rel = v.checked_sub(HEAP)?;
    let i = p.chunk_bases.partition_point(|&b| b <= rel).checked_sub(1)?;
    Some((i, rel - p.chunk_bases[i]))
}

fn word(b: &Buf, o: usize) -> u32 {
    let mut w = [0u8; 4];
    let e = (o + 4).min(b.data.len());
    w[..e - o].copy_from_slice(&b.data[o..e]);
    u32::from_le_bytes(w)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let load = |p: &str| Portable::from_bytes(&std::fs::read(p).expect(p)).expect(p);
    let (a, b) = (load(&args[1]), load(&args[2]));
    let max: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(40);
    let (da, db) = (a.digest(), b.digest());
    let names: Vec<_> = (0..da.len().min(db.len())).filter(|&i| da[i] != db[i]).map(Portable::var_name).collect();
    println!("digest: {} of {} variables differ: {}", names.len(), da.len(), names.join(", "));

    // The variables' plain words (not pointers): the game's values.
    for i in 0..a.vars.len().min(b.vars.len()) {
        let (x, y) = (&a.vars[i], &b.vars[i]);
        let r: std::collections::HashSet<u32> = x.relocs.iter().chain(y.relocs.iter()).map(|r| r.0).collect();
        let mut w = Vec::new();
        for o in (0..x.data.len().min(y.data.len())).step_by(4) {
            if !r.contains(&(o as u32)) && word(x, o) != word(y, o) {
                w.push(format!("+{o:#x}: {:#x} vs {:#x}", word(x, o), word(y, o)));
            }
        }
        if !w.is_empty() {
            println!("value {}: {} words: {}", Portable::var_name(i), w.len(), w.iter().take(12).cloned().collect::<Vec<_>>().join(", "));
        }
    }
    // What each pointer of the variables points to: the 48 bytes there (pointer words
    // skipped) must be equal; global pointers must be equal.
    let window = |p: &Portable, v: u32| -> Option<Vec<Option<u32>>> {
        let (i, o) = block(p, v)?;
        let c = &p.chunks[i];
        let r: std::collections::HashSet<u32> = c.relocs.iter().map(|r| r.0).collect();
        Some((0..12).map(|k| o as usize + 4 * k).filter(|&q| q + 4 <= c.data.len()).map(|q| if r.contains(&(q as u32)) { None } else { Some(word(c, q)) }).collect())
    };
    let mut bad = 0;
    for i in 0..a.vars.len().min(b.vars.len()) {
        let (x, y) = (&a.vars[i], &b.vars[i]);
        let ry: HashMap<u32, u8> = y.relocs.iter().map(|r| (r.0, r.1)).collect();
        for &(o, k) in &x.relocs {
            if ry.get(&o) != Some(&k) || k != 0 {
                continue;
            }
            let (wx, wy) = (word(x, o as usize), word(y, o as usize));
            let msg = match (wx >= HEAP, wy >= HEAP) {
                (false, false) if wx != wy => Some(format!("global {wx:#x} vs {wy:#x}")),
                (true, true) => {
                    let (cx, cy) = (window(&a, wx), window(&b, wy));
                    (cx != cy).then(|| format!("pointee differs: {cx:x?} vs {cy:x?}"))
                }
                (false, false) => None,
                _ => Some(format!("{wx:#x} vs {wy:#x}")),
            };
            if let Some(m) = msg {
                bad += 1;
                if bad <= max {
                    println!("pointer {}+{o:#x}: {m}", Portable::var_name(i));
                }
            }
        }
    }
    println!("{bad} pointers of the variables differ");
    // BLOCK_OF=<variable suffix>: compare the whole block the variable's first word points to.
    if let Ok(vn) = std::env::var("BLOCK_OF") {
        let i = (0..Portable::VAR_COUNT).find(|&i| Portable::var_name(i).ends_with(&vn)).unwrap();
        let (pa, pb) = (word(&a.vars[i], 0), word(&b.vars[i], 0));
        let ((ia, oa), (ib, ob)) = (block(&a, pa).unwrap(), block(&b, pb).unwrap());
        let (x, y) = (&a.chunks[ia], &b.chunks[ib]);
        println!("{vn}: block {ia}+{oa:#x} ({} bytes) vs block {ib}+{ob:#x} ({} bytes)", x.data.len(), y.data.len());
        let n = (x.data.len() - oa as usize).min(y.data.len() - ob as usize);
        let (rx, ry): (std::collections::HashSet<u32>, std::collections::HashSet<u32>) = (x.relocs.iter().map(|r| r.0).collect(), y.relocs.iter().map(|r| r.0).collect());
        let mut d = 0;
        for k in (0..n).step_by(4) {
            let (qa, qb) = (oa as usize + k, ob as usize + k);
            let (wa, wb) = (word(x, qa), word(y, qb));
            let (pa, pb) = (rx.contains(&(qa as u32)), ry.contains(&(qb as u32)));
            if pa != pb || (!pa && wa != wb) {
                d += 1;
                if d <= 12 { println!("  +{k:#x}: {wa:#x}{} vs {wb:#x}{}", if pa {" ptr"} else {""}, if pb {" ptr"} else {""}); }
            }
        }
        println!("  {d} words differ of {}", n / 4);
        let show = |p: &Portable, c: &sm64_binds::portable::Buf, o: u32| {
            let mut v: Vec<String> = c.relocs.iter().filter(|r| r.0 >= o).take(8).map(|r| {
                let w = word(c, r.0 as usize);
                match block(p, w) { Some((bi, bo)) => format!("+{:#x}->block{bi}+{bo:#x}", r.0 - o), None => format!("+{:#x}->{w:#x}", r.0 - o) }
            }).collect();
            v.truncate(8);
            v.join(" ")
        };
        println!("  pointers A: {}", show(&a, x, oa));
        println!("  pointers B: {}", show(&b, y, ob));
    }
    if std::env::var("VALUES_ONLY").is_ok() {
        return;
    }
    // Pair blocks breadth first from the variables.
    let mut pair: HashMap<usize, usize> = HashMap::new();
    let mut queue: VecDeque<(String, *const Buf, *const Buf)> = VecDeque::new();
    for i in 0..a.vars.len().min(b.vars.len()) {
        queue.push_back((Portable::var_name(i).to_string(), &a.vars[i], &b.vars[i]));
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    while let Some((name, x, y)) = queue.pop_front() {
        // SAFETY: the pointers are into `a` and `b`, which outlive the loop.
        let (x, y) = unsafe { (&*x, &*y) };
        let rx: HashMap<u32, u8> = x.relocs.iter().map(|r| (r.0, r.1)).collect();
        let ry: HashMap<u32, u8> = y.relocs.iter().map(|r| (r.0, r.1)).collect();
        let n = x.data.len().min(y.data.len());
        let mut msgs = Vec::new();
        for o in (0..n).step_by(4) {
            let (wx, wy) = (word(x, o), word(y, o));
            let (kx, ky) = (rx.get(&(o as u32)), ry.get(&(o as u32)));
            if kx.is_none() && ky.is_none() {
                if wx != wy {
                    msgs.push(format!("+{o:#x}: {wx:#x} vs {wy:#x}"));
                }
                continue;
            }
            match (block(&a, wx), block(&b, wy)) {
                (Some((ia, oa)), Some((ib, ob))) if kx == Some(&0) && ky == Some(&0) => {
                    if oa != ob {
                        msgs.push(format!("+{o:#x}: ptr block{ia}+{oa:#x} vs block{ib}+{ob:#x}"));
                    } else if let Some(&p) = pair.get(&ia) {
                        if p != ib {
                            msgs.push(format!("+{o:#x}: ptr block{ia} (paired with {p}) vs block{ib}"));
                        }
                    } else {
                        pair.insert(ia, ib);
                        if seen.insert(ia) {
                            queue.push_back((format!("block{ia}<{name}+{o:#x}"), &a.chunks[ia], &b.chunks[ib]));
                        }
                    }
                }
                _ => {
                    if wx != wy || kx != ky {
                        msgs.push(format!("+{o:#x}: {wx:#x} ({kx:?}) vs {wy:#x} ({ky:?})"));
                    }
                }
            }
        }
        if !msgs.is_empty() {
            out.insert(name, msgs);
        }
    }
    println!("{} of {} blocks paired; {} items differ", pair.len(), a.chunks.len(), out.len());
    for (k, v) in out.iter().take(max) {
        println!("  {k}: {} words: {}", v.len(), v.iter().take(6).cloned().collect::<Vec<_>>().join(", "));
    }
}
