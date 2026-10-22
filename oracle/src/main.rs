//! Lockstep oracle: our headless WASM against sm64_physics (emulator-verified),
//! same inputs, layout-independent state compared every frame.
use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};
use std::collections::HashMap;

#[repr(C)]
struct World(u8);
unsafe extern "C" {
    fn sm64_load_rom(rom: *const u8, size: usize) -> bool;
    fn sm64_world_create() -> *mut World;
    fn sm64_world_destroy(w: *mut World);
    fn sm64_world_enter(w: *const World);
    fn sm64_step(w: *mut World, input: u32);
    fn sm64_world_copy(dst: *mut World, src: *const World);
    fn oracle_dump(w: *const World, out: *mut u32, cap: usize) -> usize;
    fn oracle_behaviors(w: *const World, out: *mut usize, cap: usize) -> usize;
}

const PORT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../sm64-port");
const NUM_LISTS: usize = 13;
const OBJ_WORDS: usize = 17 + 0x50;

struct Map {
    syms: HashMap<String, u32>,
    by_addr: Vec<(u32, String)>,
}
impl Map {
    fn load(path: &str) -> Map {
        let mut syms = HashMap::new();
        let mut by_addr = Vec::new();
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let c: Vec<&str> = line.split_whitespace().collect();
            if c.len() == 4 && c[3].contains(":(.") {
                if let (Ok(a), Some(i)) = (u32::from_str_radix(c[0], 16), c[3].rfind(":(.")) {
                    let sec = &c[3][i + 3..c[3].len() - 1];
                    let name = sec.splitn(2, '.').nth(1).unwrap_or("").to_string();
                    if !name.is_empty() {
                        syms.insert(name.clone(), a);
                        by_addr.push((a, name));
                    }
                }
            }
        }
        by_addr.sort();
        Map { syms, by_addr }
    }
    fn sym(&self, n: &str) -> u32 {
        *self.syms.get(n).unwrap_or_else(|| panic!("no symbol {n}"))
    }
    fn name(&self, a: u32) -> String {
        match self.by_addr.binary_search_by_key(&a, |e| e.0) {
            Ok(i) => self.by_addr[i].1.clone(),
            Err(i) if i > 0 => {
                format!("{}+{:#x}", self.by_addr[i - 1].1, a - self.by_addr[i - 1].0)
            }
            _ => format!("{a:#x}"),
        }
    }
}

fn rd(g: &SM64Game, a: u32, n: usize) -> Vec<u8> {
    let mut b = vec![0; n];
    g.read_memory(a, &mut b).unwrap();
    b
}
fn u16at(b: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([b[o], b[o + 1]]) as u32
}
fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// Same words, same order as c/oracle_dump.c, from wasm32 (N64) layout. Also
/// returns each object's behavior name, for reports.
fn dump_wasm(g: &SM64Game, m: &Map, out: &mut Vec<u32>, bhvs: &mut Vec<String>) {
    out.clear();
    bhvs.clear();
    out.push(u32at(&rd(g, m.sym("gGlobalTimer"), 4), 0));
    out.push(u16at(&rd(g, m.sym("gRandomSeed16"), 2), 0));
    out.push(u16at(&rd(g, m.sym("gCurrLevelNum"), 2), 0));
    out.push(u16at(&rd(g, m.sym("gCurrAreaIndex"), 2), 0));
    let ms = rd(g, m.sym("gMarioStates"), 0xC8);
    out.extend([
        u32at(&ms, 0x0C),
        u16at(&ms, 0x18),
        u16at(&ms, 0x1A),
        u16at(&ms, 0x2C),
        u16at(&ms, 0x2E),
        u16at(&ms, 0x30),
        u16at(&ms, 0xAE),
        u16at(&ms, 0xAA),
        u16at(&ms, 0xA8),
    ]);
    for o in [0x3C, 0x40, 0x44, 0x48, 0x4C, 0x50, 0x54] {
        out.push(u32at(&ms, o));
    }
    let lists = m.sym("gObjectListArray");
    for l in 0..NUM_LISTS {
        let head = lists + (l * 0x68) as u32;
        let at = out.len();
        out.push(0);
        let mut node = u32at(&rd(g, head + 0x60, 4), 0);
        let mut count = 0;
        while node != head && count < 300 {
            let b = rd(g, node, 0x260);
            out.extend([
                u16at(&b, 0x74),
                u16at(&b, 0x38),
                u16at(&b, 0x40),
                u16at(&b, 0x1F4),
                u16at(&b, 0x1A),
                u16at(&b, 0x1C),
                u16at(&b, 0x1E),
            ]);
            for o in [0x20, 0x24, 0x28, 0x1F8, 0x1FC, 0x200, 0x204, 0x208, 0x70] {
                out.push(u32at(&b, o));
            }
            out.push(u16at(&b, 0x76));
            for i in 0..0x50 {
                out.push(u32at(&b, 0x88 + 4 * i));
            }
            bhvs.push(m.name(u32at(&b, 0x20C)));
            node = u32at(&b, 0x60);
            count += 1;
        }
        out[at] = count;
    }
    let lk = rd(g, m.sym("gLakituState"), 0xC0);
    for i in 0..0x30 {
        out.push(u32at(&lk, 4 * i));
    }
}

const HEAD: [&str; 20] = [
    "gGlobalTimer",
    "gRandomSeed16",
    "gCurrLevelNum",
    "gCurrAreaIndex",
    "m.action",
    "m.actionState",
    "m.actionTimer",
    "m.faceAngle[0]",
    "m.faceAngle[1]",
    "m.faceAngle[2]",
    "m.health",
    "m.numStars",
    "m.numCoins",
    "m.pos[0]",
    "m.pos[1]",
    "m.pos[2]",
    "m.vel[0]",
    "m.vel[1]",
    "m.vel[2]",
    "m.forwardVel",
];
const OBJ: [&str; 17] = [
    "activeFlags",
    "animID",
    "animFrame",
    "bhvDelayTimer",
    "angle[0]",
    "angle[1]",
    "angle[2]",
    "gfx.pos[0]",
    "gfx.pos[1]",
    "gfx.pos[2]",
    "hitboxRadius",
    "hitboxHeight",
    "hurtboxRadius",
    "hurtboxHeight",
    "hitboxDownOffset",
    "collidedObjInteractTypes",
    "numCollidedObjs",
];

/// Labels for each word of a dump (walked from the WASM side's counts).
fn labels(d: &[u32], bhvs: &[String]) -> Vec<String> {
    let mut v: Vec<String> = HEAD.iter().map(|s| s.to_string()).collect();
    let mut i = HEAD.len();
    let mut k = 0;
    for l in 0..NUM_LISTS {
        if i >= d.len() {
            break;
        }
        let n = d[i] as usize;
        v.push(format!("list{l}.count"));
        i += 1;
        for j in 0..n {
            let b = bhvs.get(k).cloned().unwrap_or_default();
            k += 1;
            for f in OBJ {
                v.push(format!("list{l}[{j}] {b} {f}"));
            }
            for r in 0..0x50 {
                v.push(format!("list{l}[{j}] {b} raw[{r:#x}]"));
            }
            i += OBJ_WORDS;
        }
    }
    for k in 0..0x30 {
        v.push(format!("gLakituState+{:#x}", 4 * k));
    }
    v
}

/// Zeroes the rawData slots that some behavior uses as a pointer: on a 64-bit
/// host they live in a separate array.
fn mask(d: &mut [u32], ptr: &[bool; 0x50]) {
    let mut i = HEAD.len();
    for _ in 0..NUM_LISTS {
        if i >= d.len() {
            return;
        }
        let n = d[i] as usize;
        i += 1;
        for _ in 0..n {
            if i + OBJ_WORDS > d.len() {
                return;
            }
            for r in 0..0x50 {
                if ptr[r] {
                    d[i + 17 + r] = 0;
                }
            }
            i += OBJ_WORDS;
        }
    }
}

fn pointer_slots() -> [bool; 0x50] {
    let text = std::fs::read_to_string(format!("{PORT}/include/object_fields.h")).unwrap();
    let mut p = [false; 0x50];
    for kind in [
        "S16P",
        "S32P",
        "ANIMS",
        "WAYPOINT",
        "CHAIN_SEGMENT",
        "OBJ",
        "SURFACE",
        "VPTR",
        "CVPTR",
    ] {
        let pat = format!("OBJECT_FIELD_{kind}(");
        for (pos, _) in text.match_indices(&pat) {
            let rest = &text[pos + pat.len()..];
            let num = &rest[..rest.find(')').unwrap()];
            if let Ok(v) = usize::from_str_radix(num.trim_start_matches("0x"), 16) {
                if v < 0x50 {
                    p[v] = true;
                }
            }
        }
    }
    p
}

/// Names of the native build's behavior scripts, from this executable's symbols.
fn native_names() -> (Vec<(usize, String)>, isize) {
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new("nm").arg(&exe).output().unwrap();
    let mut v = Vec::new();
    let mut dump_at = 0usize;
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let c: Vec<&str> = l.split_whitespace().collect();
        if c.len() != 3 {
            continue;
        }
        let Ok(a) = usize::from_str_radix(c[0], 16) else {
            continue;
        };
        if c[2] == "oracle_dump" {
            dump_at = a;
        }
        if c[2].starts_with("bhv") {
            v.push((a, c[2].to_string()));
        }
    }
    v.sort();
    (v, oracle_dump as usize as isize - dump_at as isize)
}

fn fmt(v: Option<&u32>) -> String {
    v.map_or("-".into(), |v| format!("{v:#x} ({:e})", f32::from_bits(*v)))
}

fn tas(p: &str) -> (Vec<GamePad>, Vec<u32>) {
    let b = std::fs::read(p).unwrap();
    let (body, n) = if b.starts_with(b"M64\x1a") {
        (
            &b[0x400..],
            u32::from_le_bytes(b[0x18..0x1c].try_into().unwrap()) as usize,
        )
    } else {
        (&b[..], usize::MAX)
    };
    let c: Vec<&[u8]> = body.chunks_exact(4).take(n).collect();
    (
        c.iter()
            .map(|c| GamePad {
                button: u16::from_be_bytes([c[0], c[1]]),
                stick_x: c[2] as i8,
                stick_y: c[3] as i8,
            })
            .collect(),
        c.iter()
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Branches both games every `every` frames of a movie and plays `len` frames of
/// random input on the copies, comparing every frame. Returns the first failure.
fn fuzz(
    generator: &SM64GameGenerator,
    map: &Map,
    ptr: &[bool; 0x50],
    raw: &[u32],
    pads: &[GamePad],
    every: usize,
    len: usize,
    seed: u32,
) -> Result<usize, String> {
    let mut g = generator.create_game().unwrap();
    let mut b = generator.create_game().unwrap();
    let w = unsafe { sm64_world_create() };
    let wb = unsafe { sm64_world_create() };
    let mut rng = seed;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    let (mut a, mut bh, mut nb) = (Vec::new(), Vec::new(), vec![0u32; 1 << 20]);
    let mut branches = 0;
    for f in 0..pads.len() {
        unsafe { sm64_world_enter(w) };
        g.step_game(pads[f]).unwrap();
        unsafe { sm64_step(w, raw[f]) };
        if f % every != every - 1 {
            continue;
        }
        branches += 1;
        b.copy_from(&g).unwrap();
        unsafe {
            sm64_world_copy(wb, w);
            sm64_world_enter(wb)
        };
        for k in 0..len {
            let r = next();
            // Buttons: A, B, Z, C, R, start rarely; the stick anywhere.
            let mut button = (r & 0xC000 | (r >> 8) & 0x2000) as u16;
            if r & 0x3f0000 == 0 {
                button |= 0x1000;
            }
            if (r >> 22) & 7 == 0 {
                button |= 0x000F & (r >> 26) as u16;
            }
            if (r >> 25) & 7 == 0 {
                button |= 0x0010;
            }
            let (sx, sy) = ((next() & 0xff) as u8 as i8, (next() & 0xff) as u8 as i8);
            let pad = GamePad {
                button,
                stick_x: sx,
                stick_y: sy,
            };
            let input = u32::from_le_bytes([(button >> 8) as u8, button as u8, sx as u8, sy as u8]);
            b.step_game(pad).unwrap();
            unsafe { sm64_step(wb, input) };
            dump_wasm(&b, map, &mut a, &mut bh);
            let n = unsafe { oracle_dump(wb, nb.as_mut_ptr(), nb.len()) };
            let d = &mut nb[..n];
            mask(&mut a, ptr);
            mask(d, ptr);
            if a[..] != d[..] {
                let lab = labels(&a, &bh);
                let first = (0..a.len().max(d.len()))
                    .find(|&i| a.get(i) != d.get(i))
                    .unwrap();
                return Err(format!(
                    "branch at frame {f}, random frame {k}: {} ours {} n64 {}",
                    lab.get(first).cloned().unwrap_or_default(),
                    fmt(a.get(first)),
                    fmt(d.get(first))
                ));
            }
        }
    }
    unsafe {
        sm64_world_destroy(w);
        sm64_world_destroy(wb)
    };
    Ok(branches)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wasm = std::fs::read(format!("{PORT}/build/us_web_headless/sm64.us.wasm")).unwrap();
    let generator = SM64GameGenerator::from_wasm(&wasm).unwrap();
    let map = Map::load(&format!("{PORT}/build/us_web_headless/sm64.map"));
    let rom = std::fs::read(format!("{PORT}/baserom.us.z64")).unwrap();
    assert!(unsafe { sm64_load_rom(rom.as_ptr(), rom.len()) });
    let ptr = pointer_slots();
    let names: Vec<String> = if args.is_empty() {
        [
            "tasvideos-2016M-0star.m64",
            "tasvideos-6943M-16star.m64",
            "tasvideos-7310M-120star.m64",
            "tasvideos-2208M-120star.m64",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    } else {
        args
    };
    let trace: Option<(usize, usize)> = std::env::var("TRACE").ok().map(|t| {
        let (a, b) = t.split_once("..").unwrap();
        (a.parse().unwrap(), b.parse().unwrap())
    });
    if let Ok(every) = std::env::var("FUZZ") {
        let every: usize = every.parse().unwrap();
        for name in &names {
            let (pads, raw) = tas(&if name.starts_with('/') {
                name.clone()
            } else {
                format!("{}/../tas/{name}", env!("CARGO_MANIFEST_DIR"))
            });
            match fuzz(&generator, &map, &ptr, &raw, &pads, every, 600, 0x9e3779b9) {
                Ok(n) => println!("{name}: {n} random branches x 600 frames identical"),
                Err(e) => println!("{name}: {e}"),
            }
        }
        return;
    }
    let (bhv_names, bias) = native_names();
    let mut a = Vec::new();
    let mut bhvs = Vec::new();
    let mut nb = vec![0u32; 1 << 20];
    for name in names {
        let (pads, raw) = tas(&if name.starts_with('/') {
            name.clone()
        } else {
            format!("{}/../tas/{name}", env!("CARGO_MANIFEST_DIR"))
        });
        let mut g = generator.create_game().unwrap();
        let w = unsafe { sm64_world_create() };
        unsafe { sm64_world_enter(w) };
        let mut result = None;
        let mut seen = std::collections::HashSet::new();
        let mut soft: Vec<String> = Vec::new();
        let mut shown_lists = false;
        let mut recent: std::collections::VecDeque<Vec<String>> = Default::default();
        for f in 0..pads.len() {
            g.step_game(pads[f]).unwrap();
            unsafe { sm64_step(w, raw[f]) };
            dump_wasm(&g, &map, &mut a, &mut bhvs);
            let n = unsafe { oracle_dump(w, nb.as_mut_ptr(), nb.len()) };
            let b = &mut nb[..n];
            mask(&mut a, &ptr);
            mask(b, &ptr);
            if let Some((lo, hi)) = trace {
                if f >= lo && f <= hi {
                    let tot = |d: &[u32]| {
                        let mut i = HEAD.len();
                        let mut t = 0;
                        for _ in 0..NUM_LISTS {
                            if i >= d.len() {
                                break;
                            }
                            let n = d[i] as usize;
                            t += n;
                            i += 1 + n * OBJ_WORDS;
                        }
                        t
                    };
                    println!(
                        "  frame {f}: input {:08x} objects ours {} n64 {}",
                        raw[f],
                        tot(&a),
                        tot(b)
                    );
                    for i in 0..HEAD.len() {
                        println!(
                            "    {:<16} ours {:<28} n64 {}",
                            HEAD[i],
                            fmt(a.get(i)),
                            fmt(b.get(i))
                        );
                    }
                    if std::env::var_os("DBGZ3").is_some() {
                        let n = u32at(&rd(&g, map.sym("gDbgZ3N"), 4), 0) as usize;
                        let at = u32at(&rd(&g, map.sym("gDbgBoxAt"), 4), 0) as i32;
                        let z = rd(&g, map.sym("gDbgZ3"), 4 * n);
                        let c = rd(&g, map.sym("gDbgZ3Ctx"), 4 * n);
                        println!("  z3 log: {n} calls, box read at {at}");
                        for i in 0..n {
                            let v = u32at(&z, 4 * i) as i32;
                            println!(
                                "    {i:4} z3 {v:6} ({:#06x}) {}",
                                v as u32 & 0xffff,
                                map.name(u32at(&c, 4 * i))
                            );
                        }
                    }
                    if let Ok(l) = std::env::var("LIST") {
                        let l: usize = l.parse().unwrap();
                        let mut ptrs = vec![0usize; 4096];
                        let k = unsafe { oracle_behaviors(w, ptrs.as_mut_ptr(), ptrs.len()) };
                        let nat: Vec<String> = ptrs[..k]
                            .iter()
                            .map(|p| {
                                let q = (*p as isize - bias) as usize;
                                match bhv_names.binary_search_by_key(&q, |e| e.0) {
                                    Ok(i) => bhv_names[i].1.clone(),
                                    Err(_) => format!("{q:#x}"),
                                }
                            })
                            .collect();
                        for (side, d, nm) in [
                            (
                                "ours",
                                &a[..],
                                &bhvs
                                    .iter()
                                    .map(|s| s.trim_start_matches("rel.ro.").to_string())
                                    .collect::<Vec<_>>()[..],
                            ),
                            ("n64 ", &b[..], &nat[..]),
                        ] {
                            let mut i = HEAD.len();
                            let mut k = 0;
                            for li in 0..NUM_LISTS {
                                let n = d[i] as usize;
                                i += 1;
                                for j in 0..n {
                                    if li == l {
                                        let o = &d[i..i + OBJ_WORDS];
                                        let r = |x: usize| f32::from_bits(o[17 + x]);
                                        println!(
                                            "    {side} list{l}[{j}] {:<28} pos ({:.3}, {:.3}, {:.3}) velY {:.3} timer {} active {:#x}",
                                            nm.get(k).cloned().unwrap_or_default(),
                                            r(6),
                                            r(7),
                                            r(8),
                                            r(0xA),
                                            o[17 + 0x33] as i32,
                                            o[0]
                                        );
                                    }
                                    i += OBJ_WORDS;
                                    k += 1;
                                }
                            }
                        }
                    }
                }
            }
            if a[..] != b[..] {
                let lab = labels(&a, &bhvs);
                let n = a.len().max(b.len());
                let lak = a.len().saturating_sub(0x30);
                let hard = (0..HEAD.len()).any(|i| a.get(i) != b.get(i))
                    || (a.len() == b.len() && a[lak..] != b[lak..]);
                let mut this_frame = Vec::new();
                for i in 0..n {
                    if a.get(i) == b.get(i) {
                        continue;
                    }
                    let l = lab.get(i).cloned().unwrap_or(format!("word {i}"));
                    // Kind: behavior + field, without the list position.
                    let kind = l.split_once(' ').map_or(l.clone(), |x| x.1.to_string());
                    if !l.contains("animFrame") && !l.contains("animID") && this_frame.len() < 25 {
                        this_frame.push(format!(
                            "    frame {f:>6}: {l:<58} ours {} n64 {}",
                            fmt(a.get(i)),
                            fmt(b.get(i))
                        ));
                    }
                    if seen.insert(kind.clone()) && seen.len() < 400 {
                        soft.push(format!(
                            "    frame {f:>6}: {l:<58} ours {} n64 {}",
                            fmt(a.get(i)),
                            fmt(b.get(i))
                        ));
                    }
                    if i >= HEAD.len() && l.ends_with("count") {
                        /* lists diverge: labels after this are unreliable */
                        break;
                    }
                }
                recent.push_back(this_frame);
                if recent.len() > 3 {
                    recent.pop_front();
                }
                if recent
                    .back()
                    .is_some_and(|fr| fr.iter().any(|l| l.contains("count")))
                    && !shown_lists
                {
                    shown_lists = true;
                    let mut ptrs = vec![0usize; 4096];
                    let k = unsafe { oracle_behaviors(w, ptrs.as_mut_ptr(), ptrs.len()) };
                    let nat: Vec<String> = ptrs[..k]
                        .iter()
                        .map(|p| {
                            let q = (*p as isize - bias) as usize;
                            match bhv_names.binary_search_by_key(&q, |e| e.0) {
                                Ok(i) => bhv_names[i].1.clone(),
                                Err(_) => format!("{q:#x}"),
                            }
                        })
                        .collect();
                    let strip = |s: &String| s.trim_start_matches("rel.ro.").to_string();
                    let ours: Vec<String> = bhvs.iter().map(strip).collect();
                    println!(
                        "  frame {f}: objects only in ours: {:?}",
                        ours.iter()
                            .filter(|b| ours.iter().filter(|x| x == b).count()
                                > nat.iter().filter(|x| x == b).count())
                            .collect::<std::collections::BTreeSet<_>>()
                    );
                    println!(
                        "  frame {f}: objects only on n64: {:?}",
                        nat.iter()
                            .filter(|b| nat.iter().filter(|x| x == b).count()
                                > ours.iter().filter(|x| x == b).count())
                            .collect::<std::collections::BTreeSet<_>>()
                    );
                }
                if hard {
                    let diffs: Vec<String> = (0..HEAD.len())
                        .chain(lak..a.len())
                        .filter(|&i| a.get(i) != b.get(i))
                        .map(|i| {
                            format!(
                                "    {:<20} ours {} n64 {}",
                                lab.get(i).cloned().unwrap_or_default(),
                                fmt(a.get(i)),
                                fmt(b.get(i))
                            )
                        })
                        .collect();
                    result = Some((f, diffs));
                    break;
                }
            }
        }
        if std::env::var_os("SOFT").is_some() {
            println!("{name}: first object-field differences:");
            for l in &soft {
                println!("{l}");
            }
        }
        let st = g.get_game_state().unwrap();
        match result {
            None => println!(
                "{name}: identical for all {} frames (level {}, stars {})",
                pads.len(),
                st.level_num,
                st.num_stars
            ),
            Some((f, d)) => {
                println!("  object differences in the last frames before it:");
                for fr in &recent {
                    for l in fr {
                        println!("{l}");
                    }
                }
                println!(
                    "{name}: first difference after frame {f} (input index), level {} area {}, stars {}:",
                    st.level_num, st.area_index, st.num_stars
                );
                for l in d {
                    println!("{l}");
                }
            }
        }
        unsafe { sm64_world_destroy(w) };
    }
}
