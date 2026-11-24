//! Differential fuzz test: is a game started from a translated portable state
//! gameplay-exact against the game it was exported from?
//!
//!   portable_fuzz <rom> <movie.m64> <seeds> <frames> <checkpoint>...
//!
//! For each checkpoint F: A = headless game replayed from boot to frame F;
//! B = a fresh headless game that loads A's portable state (export_portable ->
//! to_bytes -> from_bytes -> load_portable). If `$B2DIR/own_<F>.p` exists (a
//! state exported by the rendering build), B2 loads that too. Then, for each
//! seed, A and B (and B2) are driven with the same pseudo-random pad sequence
//! (style = seed % 5: random buttons+stick, running with jumps, mash A/B/Z,
//! idle, random with long holds) for <frames> frames, comparing every frame:
//!   gs     the GameState
//!   digest the per-variable digest of export_portable (pointer words skipped)
//!   exact  the whole portable state: variables and every reachable heap block,
//!          bytes and relocations (pointers compared by what they point to)
//! The line per pair gives the first frame of each kind and the variables.
//! Env: THREADS (default 4), EXACT=0 skips the per-frame export compare,
//! DETAIL=1 prints the differing words at the first exact difference,
//! STOP_AT_GS=0 keeps going after the first GameState difference,
//! MOVIE_FRAMES=n drives the first n frames with the movie's own pads
//! (then the random ones), SEED0 first seed, B2DIR (see above), EXACT_EVERY=n
//! (compare the whole exported state every n frames, default 1; GameState is
//! compared every frame), DETAIL_N words per differing variable, WATCH=<var suffix>
//! (print that variable's first word in both games when it changes), PROBE=<var>:<hex off>.
//!
//! "digest" is the main signal (variables, pointer words skipped); "exact" compares
//! blocks structurally (matched through pointers); "alias" = a stale pointer that only
//! looks like a pointer in the other heap; "natural" = variables that already differ
//! right after the load (render state) and are left out of the digest afterwards.

use sm64_binds::portable::{Buf, Portable};
use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

fn pads(path: &str) -> Vec<GamePad> {
    let m64 = std::fs::read(path).unwrap();
    let n = u32::from_le_bytes(m64[0x18..0x1C].try_into().unwrap()) as usize;
    let n = if n == 0 || n > (m64.len() - 0x400) / 4 { (m64.len() - 0x400) / 4 } else { n };
    m64[0x400..]
        .as_chunks::<4>()
        .0
        .iter()
        .take(n)
        .map(|c| GamePad {
            button: u16::from_be_bytes([c[0], c[1]]),
            stick_x: c[2] as i8,
            stick_y: c[3] as i8,
        })
        .collect()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next() % (hi - lo + 1) as u64) as i32
    }
}

const A_BTN: u16 = 0x8000;
const B_BTN: u16 = 0x4000;
const Z_BTN: u16 = 0x2000;
const START: u16 = 0x1000;
const L_BTN: u16 = 0x0020;
const R_BTN: u16 = 0x0010;
const C_BTNS: [u16; 4] = [0x0008, 0x0004, 0x0002, 0x0001];

fn stick(r: &mut Rng) -> (i8, i8) {
    match r.next() % 4 {
        0 => (0, 0),
        1 => (
            [-80, 0, 80][r.range(0, 2) as usize],
            [-80, 0, 80][r.range(0, 2) as usize],
        ),
        _ => (r.range(-80, 80) as i8, r.range(-80, 80) as i8),
    }
}

/// `n` pads of style `style` from seed `seed`.
fn gen_pads(seed: u64, style: u64, n: usize) -> Vec<GamePad> {
    let mut r = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xD1B54A32D192ED03);
    for _ in 0..4 {
        r.next();
    }
    let mut out = Vec::with_capacity(n);
    let mut cur = GamePad::default();
    let mut left = 0;
    let mut dir = r.range(-128, 127);
    while out.len() < n {
        if left == 0 {
            match style {
                0 => {
                    left = r.range(1, 30);
                    let mut b = 0;
                    for &m in &[A_BTN, B_BTN, Z_BTN, L_BTN, R_BTN, C_BTNS[0], C_BTNS[1], C_BTNS[2], C_BTNS[3]] {
                        if r.next() % 4 == 0 {
                            b |= m;
                        }
                    }
                    if r.next() % 40 == 0 {
                        b |= START;
                    }
                    let (x, y) = stick(&mut r);
                    cur = GamePad::new(b, x, y);
                }
                1 => {
                    left = r.range(1, 30);
                    if r.next() % 8 == 0 {
                        dir = r.range(-128, 127);
                    }
                    let a = ((dir as f64) * std::f64::consts::PI / 128.0).sin();
                    let c = ((dir as f64) * std::f64::consts::PI / 128.0).cos();
                    let mag = if r.next() % 6 == 0 { r.range(20, 60) } else { 80 } as f64;
                    let mut b = 0;
                    if r.next() % 3 == 0 {
                        b |= A_BTN;
                    }
                    if r.next() % 12 == 0 {
                        b |= B_BTN;
                    }
                    if r.next() % 20 == 0 {
                        b |= Z_BTN;
                    }
                    if r.next() % 15 == 0 {
                        b |= C_BTNS[r.range(0, 3) as usize];
                    }
                    cur = GamePad::new(b, (a * mag) as i8, (c * mag) as i8);
                }
                2 => {
                    left = r.range(1, 4);
                    let mut b = 0;
                    for &m in &[A_BTN, B_BTN, Z_BTN] {
                        if r.next() % 2 == 0 {
                            b |= m;
                        }
                    }
                    let (x, y) = stick(&mut r);
                    cur = GamePad::new(b, x, y);
                }
                3 => {
                    left = r.range(20, 200);
                    cur = if r.next() % 5 == 0 {
                        GamePad::new(if r.next() % 2 == 0 { A_BTN } else { 0 }, 0, r.range(-80, 80) as i8)
                    } else {
                        GamePad::default()
                    };
                }
                _ => {
                    left = r.range(30, 150);
                    let mut b = 0;
                    for &m in &[A_BTN, B_BTN, Z_BTN, R_BTN, C_BTNS[0], C_BTNS[1], C_BTNS[2], C_BTNS[3]] {
                        if r.next() % 5 == 0 {
                            b |= m;
                        }
                    }
                    let (x, y) = stick(&mut r);
                    cur = GamePad::new(b, x, y);
                }
            }
        }
        left -= 1;
        out.push(cur);
    }
    out
}

fn gs(g: &mut SM64Game) -> Vec<u32> {
    let s = g.get_game_state().unwrap();
    let mut v = vec![s.num_stars as u32];
    v.extend(s.pos.iter().map(|x| x.to_bits()));
    v.extend(s.vel.iter().map(|x| x.to_bits()));
    v.extend(s.lakitu_pos.iter().map(|x| x.to_bits()));
    v.extend(
        [s.lakitu_yaw, s.in_credits, s.course_num, s.act_num, s.area_index, s.level_num, s.lives, s.coins, s.health]
            .iter()
            .map(|x| *x as u32),
    );
    v
}

const GS_NAMES: [&str; 18] = [
    "stars", "pos.x", "pos.y", "pos.z", "vel.x", "vel.y", "vel.z", "lakitu.x", "lakitu.y",
    "lakitu.z", "lakitu_yaw", "in_credits", "course", "act", "area", "level", "lives", "coins/health",
];

const HEAP_BASE: u32 = 0x4000_0000;

fn word(d: &[u8], i: usize) -> u32 {
    let mut w = [0u8; 4];
    let e = (i * 4 + 4).min(d.len());
    w[..e - i * 4].copy_from_slice(&d[i * 4..e]);
    u32::from_le_bytes(w)
}

fn decode(p: &Portable, v: u32) -> Option<(usize, u32)> {
    if v < HEAP_BASE {
        return None;
    }
    let rel = v - HEAP_BASE;
    let k = p.chunk_bases.partition_point(|&x| x <= rel);
    (k > 0).then(|| (k - 1, rel - p.chunk_bases[k - 1]))
}

/// Structural comparison of two exports: block numbers and addresses differ
/// between a played game and a loaded one (sizes, discovery order), so the
/// blocks are matched through the pointers that lead to them, starting from
/// the variables. A difference is: a non-pointer word that differs, a pointer
/// to a global/function that differs, a word that is a pointer on one side
/// only, two pointers whose targets differ in offset or were already matched
/// to other blocks, or a block reached from one side only. Blocks are compared
/// over the shorter length (the loaded game's blocks may be larger).
/// Returns (names of what differs, detail lines).
fn exact_diff(a: &Portable, b: &Portable, detail: bool) -> (Vec<String>, Vec<String>) {
    let mut names = vec![];
    let mut det = vec![];
    let mut ab: std::collections::HashMap<usize, usize> = Default::default();
    let mut ba: std::collections::HashMap<usize, usize> = Default::default();
    let mut queue: Vec<(String, &Buf, &Buf, bool)> = vec![];
    for (i, (x, y)) in a.vars.iter().zip(&b.vars).enumerate() {
        if Portable::var_name(i).ends_with("gDisplayListHeap") {
            continue; // the display list arena cursor: render state, rebuilt every frame
        }
        queue.push((Portable::var_name(i).to_string(), x, y, false));
    }
    let mut qi = 0;
    while qi < queue.len() {
        let (name, x, y, block) = (queue[qi].0.clone(), queue[qi].1, queue[qi].2, queue[qi].3);
        qi += 1;
        let n = if block { x.data.len().min(y.data.len()) } else { x.data.len() };
        let mut diffs: Vec<String> = vec![];
        if !block && x.data.len() != y.data.len() {
            diffs.push(format!("length {} vs {}", x.data.len(), y.data.len()));
        }
        // Relocations can sit at any byte offset (pointers inside s16 data), so
        // compare them as sets and the bytes they cover separately.
        let ra: std::collections::BTreeMap<u32, u8> = x.relocs.iter().map(|r| (r.0, r.1)).collect();
        let rb: std::collections::BTreeMap<u32, u8> = y.relocs.iter().map(|r| (r.0, r.1)).collect();
        let mut covered = vec![false; n + 4];
        for &o in ra.keys().chain(rb.keys()) {
            for k in 0..4 {
                if (o as usize + k) < covered.len() {
                    covered[o as usize + k] = true;
                }
            }
        }
        for (o, (ka, kb)) in ra
            .keys()
            .chain(rb.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|o| (*o, (ra.get(o).copied(), rb.get(o).copied())))
        {
            if (o as usize + 4) > n {
                continue;
            }
            let rd = |d: &[u8]| u32::from_le_bytes(d[o as usize..o as usize + 4].try_into().unwrap());
            let (wa, wb) = (rd(&x.data), rd(&y.data));
            match (ka, kb) {
                (Some(ka), Some(kb)) if ka == kb => {
                    if let (Some((ia, oa)), Some((ib, ob))) =
                        (decode(a, wa).filter(|_| ka == 0), decode(b, wb).filter(|_| kb == 0))
                    {
                        let mut bad = oa != ob;
                        match (ab.get(&ia), ba.get(&ib)) {
                            (None, None) => {
                                ab.insert(ia, ib);
                                ba.insert(ib, ia);
                                queue.push((
                                    format!("block{ia}/{ib}({})", a.found_by[ia].0),
                                    &a.chunks[ia],
                                    &b.chunks[ib],
                                    true,
                                ));
                            }
                            (Some(&j), Some(&k)) => bad |= j != ib || k != ia,
                            _ => bad = true,
                        }
                        if bad {
                            diffs.push(format!("+{o:#x}: ptr to blk{ia}+{oa:#x} vs blk{ib}+{ob:#x}"));
                        }
                    } else if wa != wb {
                        diffs.push(format!("+{o:#x}: {wa:#x} vs {wb:#x} (ptr)"));
                    }
                }
                (ka, kb) => diffs.push(format!("+{o:#x}: reloc {ka:?} vs {kb:?} ({wa:#x} vs {wb:#x})")),
            }
        }
        {
            let m = n.min(x.data.len()).min(y.data.len());
            let mut i = 0;
            while i < m {
                if !covered[i] && x.data[i] != y.data[i] {
                    let e = (i + 4).min(m);
                    diffs.push(format!("+{i:#x}: {:02x?} vs {:02x?}", &x.data[i..e], &y.data[i..e]));
                    i = e;
                } else {
                    i += 1;
                }
            }
        }
        if !diffs.is_empty() {
            if detail {
                det.push(format!(
                    "{name}: {} words differ: {}",
                    diffs.len(),
                    diffs
                        .iter()
                        .take(std::env::var("DETAIL_N").ok().and_then(|s| s.parse().ok()).unwrap_or(6))
                        .map(|d| {
                            // object pool words: say which object and field
                            let off = d.strip_prefix("+0x").and_then(|r| r.split(':').next()).and_then(|h| usize::from_str_radix(h, 16).ok());
                            match off {
                                Some(off) if name.ends_with("gObjectPool") => format!("obj{}+{:#x}{}", off / 0x260, off % 0x260, d.split_once(':').map(|x| format!(":{}", x.1)).unwrap_or_default()),
                                _ => d.clone(),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            names.push(if diffs.iter().all(|d| d.contains("reloc ")) { format!("{name}[alias]") } else { name });
        }
    }
    let ua: Vec<usize> = (0..a.chunks.len()).filter(|i| !ab.contains_key(i)).collect();
    let ub: Vec<usize> = (0..b.chunks.len()).filter(|i| !ba.contains_key(i)).collect();
    if ua.len() + ub.len() > 0 {
        names.push(format!("unmatched-blocks(A{},B{})", ua.len(), ub.len()));
        if detail {
            for &i in &ua {
                det.push(format!("A-only blk{i} {}b {:?}", a.chunks[i].data.len(), a.found_by[i]));
            }
            for &i in &ub {
                det.push(format!("B-only blk{i} {}b {:?}", b.chunks[i].data.len(), b.found_by[i]));
            }
        }
    }
    (names, det)
}

#[derive(Default, Debug)]
struct Outcome {
    frames: usize,
    gs: Option<(usize, String)>,
    digest: Option<(usize, Vec<String>)>,
    exact: Option<(usize, Vec<String>, Vec<String>)>,
    alias: Option<Vec<String>>,
    natural: Vec<String>,
    crash: Option<(usize, String, bool)>,
    digest_det: Vec<String>,
}

fn run_pair(a: &mut SM64Game, b: &mut SM64Game, pads: &[GamePad], exact: bool, detail: bool, stop_gs: bool) -> Outcome {
    let every: usize = std::env::var("EXACT_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut o = Outcome::default();
    // frame 0: right after the load, before any step
    if exact {
        if let (Ok(pa), Ok(pb)) = (a.export_portable(), b.export_portable()) {
            record_export_diff(&mut o, 0, &pa, &pb, detail);
            // what already differs right after the load is state the load does not carry
            // (render caches, matrices): not compared afterwards
            if let Some((_, n)) = o.digest.take() {
                o.natural = n;
            }
        }
    }
    for (i, pad) in pads.iter().enumerate() {
        let ra = a.step_game(*pad);
        let rb = b.step_game(*pad);
        o.frames = i + 1;
        match (ra, rb) {
            (Err(e), Ok(_)) => {
                o.crash = Some((i + 1, format!("{e:?}"), false));
                break;
            }
            (Ok(_), Err(e)) => {
                o.crash = Some((i + 1, format!("{e:?}"), true));
                break;
            }
            (Err(_), Err(_)) => break,
            _ => {}
        }
        let (ga, gb) = (gs(a), gs(b));
        if let Ok(w) = std::env::var("WATCH") {
            // WATCH=var-name-suffix: print the first word of that variable in both games when it changes in either
            let vi = (0..Portable::VAR_COUNT).find(|&k| Portable::var_name(k).ends_with(w.as_str())).unwrap();
            let (va, _) = Portable::var_at(vi, sm64_binds::portable::Build::Headless);
            let rd = |g: &SM64Game| g.with_memory(|m| u32::from_le_bytes(m[va as usize..va as usize + 4].try_into().unwrap()));
            let (x, y) = (rd(a), rd(b));
            thread_local!(static LAST: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((u32::MAX, 0)) });
            if LAST.with(|l| l.get()) != (x, y) {
                println!("WATCH {w} frame {}: A {x:#x} B {y:#x}", i + 1);
                LAST.with(|l| l.set((x, y)));
            }
        }
        let mut stop = false;
        if ga != gb && o.gs.is_none() {
            let d: Vec<String> = (0..ga.len())
                .filter(|&j| ga[j] != gb[j])
                .map(|j| {
                    let f = |v: u32| if (1..=9).contains(&j) { f32::from_bits(v) as f64 } else { v as i32 as f64 };
                    format!("{}: {} vs {}", GS_NAMES[j], f(ga[j]), f(gb[j]))
                })
                .collect();
            o.gs = Some((i + 1, d.join(", ")));
            stop = stop_gs;
        }
        if ((exact && (i + 1) % every == 0) || stop || o.gs.is_some()) && !(o.exact.is_some() && o.digest.is_some()) {
            match (a.export_portable(), b.export_portable()) {
                (Ok(pa), Ok(pb)) => {
                    let had = o.digest.is_some();
                    record_export_diff(&mut o, i + 1, &pa, &pb, detail);
                    if !had && o.digest.is_some() {
                        o.digest_det.push(format!("[at digest frame: level {} area {} act {} course {}]", ga[15], ga[14], ga[13], ga[12]));
                    }
                }
                (Err(_), Err(_)) => {}
                (Err(e), _) | (_, Err(e)) => {
                    if o.exact.is_none() {
                        o.exact = Some((i + 1, vec![format!("export failed on one side: {e}")], vec![]));
                    }
                }
            }
        }
        if stop {
            break;
        }
    }
    o
}

fn record_export_diff(o: &mut Outcome, frame: usize, pa: &Portable, pb: &Portable, detail: bool) {
    if o.digest.is_none() {
        let (da, db) = (pa.digest(), pb.digest());
        let names: Vec<String> = (0..da.len().min(db.len()))
            .filter(|&j| da[j] != db[j])
            .map(|j| Portable::var_name(j).to_string())
            .filter(|n| !o.natural.contains(n))
            .collect();
        if !names.is_empty() {
            if detail {
                o.digest_det = exact_diff(pa, pb, true)
                    .1
                    .into_iter()
                    .filter(|l| names.iter().any(|n| l.starts_with(n.as_str())))
                    .collect();
            }
            o.digest = Some((frame, names));
        }
    }
    if o.exact.is_none() {
        let (names, words) = exact_diff(pa, pb, detail);
        // "[alias]": only whether a word counts as a pointer differs: a stale
        // pointer of the played game (raw, no block) that, in the loaded game's
        // different heap, lands inside a live block.
        let (alias, real): (Vec<String>, Vec<String>) = names.into_iter().partition(|n| n.ends_with("[alias]"));
        if o.alias.is_none() && !alias.is_empty() {
            o.alias = Some(alias);
        }
        if !real.is_empty() {
            o.exact = Some((frame, real, words));
        }
    }
}

fn fmt_names(v: &[String]) -> String {
    let mut s = v.iter().take(8).cloned().collect::<Vec<_>>().join(",");
    if v.len() > 8 {
        s += &format!(",...(+{})", v.len() - 8);
    }
    s
}

struct Row {
    f: usize,
    which: &'static str,
    o: Outcome,
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 6 {
        eprintln!("usage: portable_fuzz <rom> <movie.m64> <seeds> <frames> <checkpoint>...");
        std::process::exit(2);
    }
    let gen_ = SM64GameGenerator::from_file(&a[1]).unwrap();
    let movie_path = a[2].clone();
    let movie = std::path::Path::new(&movie_path).file_name().unwrap().to_string_lossy().to_string();
    let p = pads(&movie_path);
    let seeds: u64 = a[3].parse().unwrap();
    let nframes: usize = a[4].parse().unwrap();
    let cps: Vec<usize> = a[5..].iter().map(|s| s.parse().unwrap()).collect();
    let env_usize = |k: &str, d: usize| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
    let threads = env_usize("THREADS", 4);
    let movie_frames = env_usize("MOVIE_FRAMES", 0);
    let exact = std::env::var("EXACT").map(|v| v != "0").unwrap_or(true);
    let detail = std::env::var("DETAIL").is_ok();
    let stop_gs = std::env::var("STOP_AT_GS").map(|v| v != "0").unwrap_or(true);
    let b2dir = std::env::var("B2DIR").ok();
    let seed0 = env_usize("SEED0", 0) as u64;

    let next = AtomicUsize::new(0);
    let rows: Mutex<Vec<Row>> = Mutex::new(vec![]);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    if k >= cps.len() {
                        break;
                    }
                    let f = cps[k];
                    let mut ga = gen_.create_game().unwrap();
                    for pad in &p[..f.min(p.len())] {
                        ga.step_game(*pad).unwrap();
                    }
                    let sa = ga.save_state();
                    let st = match ga.export_portable() {
                        Ok(s) => s,
                        Err(e) => {
                            println!("CHECKPOINT movie={movie} F={f} no portable state: {e}");
                            continue;
                        }
                    };
                    let st = Portable::from_bytes(&st.to_bytes()).unwrap();
                    let mut variants: Vec<(&'static str, Portable)> = vec![("B", st)];
                    if let Some(d) = &b2dir {
                        if let Ok(bytes) = std::fs::read(format!("{d}/own_{f}.p")) {
                            variants.push(("B2", Portable::from_bytes(&bytes).unwrap()));
                        }
                    }
                    for (which, st) in variants {
                        let mut gb = gen_.create_game().unwrap();
                        match gb.load_portable(&st) {
                            Ok(r) => {
                                if r.untranslated > 0 {
                                    println!("CHECKPOINT movie={movie} F={f} {which}: {} untranslated pointers", r.untranslated);
                                }
                            }
                            Err(e) => {
                                println!("CHECKPOINT movie={movie} F={f} {which}: load failed: {e}");
                                continue;
                            }
                        }
                        let sb = gb.save_state();
                        // PROBE=<var name suffix>:<offset hex>: print the words of what that
                        // variable's pointer points to, in both games, after the load
                        if let Ok(pr) = std::env::var("PROBE") {
                            let (vn, off) = pr.split_once(':').unwrap();
                            let off = u32::from_str_radix(off, 16).unwrap();
                            let vi = (0..Portable::VAR_COUNT).find(|&i| Portable::var_name(i).ends_with(vn)).unwrap();
                            for (nm, g) in [("A", &ga), ("B", &gb)] {
                                g.with_memory(|m| {
                                    let (va, _) = Portable::var_at(vi, sm64_binds::portable::Build::Headless);
                                    let rd = |x: u32| u32::from_le_bytes(m[x as usize..x as usize + 4].try_into().unwrap());
                                    let p = if vn.ends_with("gObjectPool") { va } else { rd(va) };
                                    let ws: Vec<String> = (0..12).map(|k| format!("{:08x}", rd(p + off + 4 * k))).collect();
                                    println!("PROBE {nm}: {vn} = {p:#x}; +{off:#x}: {}", ws.join(" "));
                                });
                            }
                        }
                        for seed in seed0..seed0 + seeds {
                            let style = seed % 5;
                            let mut pp: Vec<GamePad> = (f..(f + movie_frames).min(p.len())).map(|i| p[i]).collect();
                            let rest = nframes.saturating_sub(pp.len());
                            pp.extend(gen_pads(seed * 1000 + f as u64, style, rest));
                            pp.truncate(nframes);
                            ga.load_state(&sa).unwrap();
                            gb.load_state(&sb).unwrap();
                            let o = run_pair(&mut ga, &mut gb, &pp, exact, detail, stop_gs);
                            let line = format!(
                                "PAIR movie={movie} F={f} seed={seed} style={style} which={which} frames={} gs={} digest={} exact={} alias={} natural={}{}",
                                o.frames,
                                o.gs.as_ref().map(|g| format!("@{} [{}]", g.0, g.1)).unwrap_or("-".into()),
                                o.digest.as_ref().map(|g| format!("@{} [{}]", g.0, fmt_names(&g.1))).unwrap_or("-".into()),
                                o.exact.as_ref().map(|g| format!("@{} [{}]", g.0, fmt_names(&g.1))).unwrap_or("-".into()),
                                o.alias.as_ref().map(|a| a.len().to_string()).unwrap_or("-".into()),
                                o.natural.len(),
                                o.crash
                                    .as_ref()
                                    .map(|c| format!(" CRASH@{} side={} {}", c.0, if c.2 { "B" } else { "A" }, c.1))
                                    .unwrap_or_default(),
                            );
                            println!("{line}");
                            for w in &o.digest_det {
                                println!("  DDETAIL {w}");
                            }
                            if let Some(e) = &o.exact {
                                for w in &e.2 {
                                    println!("  DETAIL {w}");
                                }
                            }
                            rows.lock().unwrap().push(Row { f, which, o });
                        }
                    }
                }
            });
        }
    });
    let rows = rows.into_inner().unwrap();
    println!("\n== SUMMARY movie={movie} frames={nframes} ==");
    for which in ["B", "B2"] {
        let r: Vec<&Row> = rows.iter().filter(|r| r.which == which).collect();
        if r.is_empty() {
            continue;
        }
        let cnt = |f: &dyn Fn(&Row) -> bool| r.iter().filter(|x| f(x)).count();
        println!(
            "{which}: pairs={} gs_diverged={} digest_diverged={} exact_diverged={} crashed={}",
            r.len(),
            cnt(&|x| x.o.gs.is_some()),
            cnt(&|x| x.o.digest.is_some()),
            cnt(&|x| x.o.exact.is_some()),
            cnt(&|x| x.o.crash.is_some())
        );
        let mut firsts: Vec<usize> = r.iter().filter_map(|r| r.o.gs.as_ref().map(|x| x.0)).collect();
        firsts.sort();
        if !firsts.is_empty() {
            println!("  first gs-divergent frames: {firsts:?}");
        }
        let mut by: std::collections::BTreeMap<usize, usize> = Default::default();
        for r in &r {
            if r.o.gs.is_some() {
                *by.entry(r.f).or_default() += 1;
            }
        }
        if !by.is_empty() {
            println!("  gs divergences by checkpoint: {by:?}");
        }
    }
}
