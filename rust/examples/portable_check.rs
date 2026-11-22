//! Cross-build validation of portable states with a TAS movie.
//!
//! The headless build plays the movie to checkpoints; at each it exports a
//! portable state and records the next K frames (a digest of the game's
//! variables and its GameState per frame). The same is done by the rendering
//! build in a browser (`javascript/e2e/portable_check.mjs`). Then each side
//! loads the other's states and records the K frames that follow, and
//! `compare` says where any pair of recordings first differs.
//!
//!   portable_check record  <rom> <movie.m64> <dir> <K> <frame>...   headless: h_<frame>.p / h_<frame>.json
//!   portable_check load    <rom> <movie.m64> <dir> <K>              headless loads v_<frame>.p: hv_<frame>.json
//!   portable_check compare <dir> <K>                                the table
//!   portable_check scan    <rom> <movie.m64> <step>                 what happens when, to pick checkpoints
//!
//! Files in <dir>: `*.p` are portable states, `*.json` recordings:
//! {"frame": N, "frames": [{"gs": [...], "d": "<base64 of the u64 digests>"}]}.

use sm64_binds::portable::{Build, Portable};
use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};
use std::collections::BTreeSet;
use std::fmt::Write as _;

fn pads(path: &str) -> Vec<GamePad> {
    let m64 = std::fs::read(path).unwrap();
    let n = u32::from_le_bytes(m64[0x18..0x1C].try_into().unwrap()) as usize;
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

fn game_state(g: &mut SM64Game) -> Vec<f64> {
    let s = g.get_game_state().unwrap();
    let mut v = vec![s.num_stars as f64];
    v.extend(s.pos.iter().map(|x| *x as f64));
    v.extend(s.vel.iter().map(|x| *x as f64));
    v.extend(s.lakitu_pos.iter().map(|x| *x as f64));
    v.extend(
        [
            s.lakitu_yaw,
            s.in_credits,
            s.course_num,
            s.act_num,
            s.area_index,
            s.level_num,
            s.lives,
            s.coins,
            s.health,
        ]
        .iter()
        .map(|x| *x as f64),
    );
    v
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

fn unb64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    } as u32;
    let b = s.as_bytes();
    let mut out = Vec::new();
    for c in b.chunks(4) {
        let n = (val(c[0]) << 18)
            | (val(c[1]) << 12)
            | if c[2] != b'=' { val(c[2]) << 6 } else { 0 }
            | if c[3] != b'=' { val(c[3]) } else { 0 };
        out.push((n >> 16) as u8);
        if c[2] != b'=' {
            out.push((n >> 8) as u8);
        }
        if c[3] != b'=' {
            out.push(n as u8);
        }
    }
    out
}

fn record(g: &mut SM64Game, pads: &[GamePad], frame: usize, k: usize) -> String {
    let mut out = format!("{{\"frame\": {frame}, \"frames\": [");
    for i in 0..k {
        g.step_game(pads[frame + i]).unwrap();
        let d = g.export_portable().map(|p| p.digest()).unwrap_or_default();
        let bytes: Vec<u8> = d.iter().flat_map(|x| x.to_le_bytes()).collect();
        let gs = game_state(g);
        if i > 0 {
            out.push(',');
        }
        write!(out, "{{\"gs\": {:?}, \"d\": \"{}\"}}", gs, b64(&bytes)).unwrap();
    }
    out.push_str("]}");
    out
}

type Recording = (Vec<Vec<f64>>, Vec<Vec<u64>>);

fn read_recording(path: &str) -> Option<Recording> {
    // (JSON from JavaScript has no spaces after the colons, ours has)
    let text = std::fs::read_to_string(path).ok()?.replace(' ', "");
    let mut gs = vec![];
    let mut ds = vec![];
    for part in text.split("{\"gs\":[").skip(1) {
        let (nums, rest) = part.split_once(']')?;
        gs.push(
            nums.split(',')
                .map(|x| x.trim().parse::<f64>().unwrap_or(f64::NAN))
                .collect(),
        );
        let d = rest.split("\"d\":\"").nth(1)?.split('"').next()?;
        ds.push(
            unb64(d)
                .chunks(8)
                .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
                .collect(),
        );
    }
    Some((gs, ds))
}

const GS_NAMES: [&str; 18] = [
    "stars",
    "pos.x",
    "pos.y",
    "pos.z",
    "vel.x",
    "vel.y",
    "vel.z",
    "lakitu.x",
    "lakitu.y",
    "lakitu.z",
    "lakitu_yaw",
    "in_credits",
    "course",
    "act",
    "area",
    "level",
    "lives",
    "coins/health",
];

/// The first frame where two recordings differ, described; variables named in
/// `ignore` are not compared. `natural` collects the variables that differ
/// (for working out which only differ between the builds).
fn first_difference(a: &str, b: &str, ignore: &BTreeSet<&'static str>, natural: &mut BTreeSet<&'static str>) -> Option<String> {
    let (ga, da) = read_recording(a)?;
    let (gb, db) = read_recording(b)?;
    if ga.is_empty() || gb.is_empty() {
        return Some("empty recording".into());
    }
    let mut first = None;
    for i in 0..ga.len().min(gb.len()) {
        let gd: Vec<String> = (0..ga[i].len().min(gb[i].len())).filter(|&j| ga[i][j] != gb[i][j]).map(|j| format!("{}: {} vs {}", GS_NAMES.get(j).unwrap_or(&"coins"), ga[i][j], gb[i][j])).collect();
        let all: Vec<&'static str> = (0..da[i].len().min(db[i].len())).filter(|&j| da[i][j] != db[i][j]).map(Portable::var_name).collect();
        natural.extend(all.iter().copied());
        let vd: Vec<&&str> = all.iter().filter(|n| !ignore.contains(*n)).collect();
        if first.is_none() && (!gd.is_empty() || !vd.is_empty()) {
            let mut s = format!("frame +{}: ", i + 1);
            if !gd.is_empty() {
                write!(s, "GameState {}; ", gd.join(", ")).unwrap();
            }
            if !vd.is_empty() {
                write!(s, "{} variables differ: {}", vd.len(), vd.iter().take(6).map(|n| **n).collect::<Vec<_>>().join(", ")).unwrap();
            }
            first = Some(s);
        }
    }
    first
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).map(String::as_str).unwrap_or("");
    match mode {
        "scan" => {
            let g = SM64GameGenerator::from_file(&a[2]).unwrap();
            let p = pads(&a[3]);
            let step: usize = a[4].parse().unwrap();
            let mut game = g.create_game().unwrap();
            for (i, pad) in p.iter().enumerate() {
                game.step_game(*pad).unwrap();
                if i % step == 0 {
                    let s = game.get_game_state().unwrap();
                    let st = game.export_portable();
                    // Mario's action, and how many objects are active
                    let mario = game.call_export_u32("get_mario_state").unwrap();
                    let mut b = [0u8; 4];
                    game.read_memory(mario + 0x0C, &mut b).unwrap();
                    let action = u32::from_le_bytes(b);
                    let pool = (0..Portable::VAR_COUNT)
                        .find(|&v| Portable::var_name(v).ends_with(":gObjectPool"))
                        .map(|v| Portable::var_at(v, Build::Headless).0)
                        .unwrap_or(0);
                    let objects = (0..240u32)
                        .filter(|o| {
                            let mut a = [0u8; 2];
                            game.read_memory(pool + o * 0x260 + 0x74, &mut a).unwrap();
                            i16::from_le_bytes(a) != 0
                        })
                        .count();
                    println!(
                        "{i}: level {} area {} stars {} coins {} health {:#x} action {action:#x} objects {objects} pos {:?} vel.y {}{}",
                        s.level_num,
                        s.area_index,
                        s.num_stars,
                        s.coins,
                        s.health,
                        s.pos,
                        s.vel[1],
                        if st.is_err() {
                            " (no portable form)"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
        "record" => {
            let g = SM64GameGenerator::from_file(&a[2]).unwrap();
            let p = pads(&a[3]);
            let dir = &a[4];
            let k: usize = a[5].parse().unwrap();
            let mut frames: Vec<usize> = a[6..].iter().map(|s| s.parse().unwrap()).collect();
            frames.sort();
            let mut game = g.create_game().unwrap();
            let mut at = 0;
            for f in frames {
                while at < f {
                    game.step_game(p[at]).unwrap();
                    at += 1;
                }
                match game.export_portable() {
                    Ok(st) => std::fs::write(format!("{dir}/h_{f}.p"), st.to_bytes()).unwrap(),
                    Err(e) => {
                        println!("h_{f}: no portable state: {e}");
                        continue;
                    }
                }
                // the recording continues the replay
                std::fs::write(format!("{dir}/h_{f}.json"), record(&mut game, &p, f, k)).unwrap();
                at += k;
                println!("h_{f}: recorded");
            }
        }
        "load" => {
            let g = SM64GameGenerator::from_file(&a[2]).unwrap();
            let p = pads(&a[3]);
            let dir = &a[4];
            let k: usize = a[5].parse().unwrap();
            let mut files: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            files.sort();
            for f in files {
                let name = f.file_name().unwrap().to_string_lossy().to_string();
                let Some(frame) = name.strip_prefix("v_").and_then(|r| r.strip_suffix(".p")) else {
                    continue;
                };
                let frame: usize = frame.parse().unwrap();
                let st = Portable::from_bytes(&std::fs::read(&f).unwrap()).unwrap();
                let mut game = g.create_game().unwrap();
                match game.load_portable(&st) {
                    Ok(r) => println!("hv_{frame}: loaded ({r:?})"),
                    Err(e) => {
                        println!("hv_{frame}: load failed: {e}");
                        continue;
                    }
                }
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    record(&mut game, &p, frame, k)
                })) {
                    Ok(json) => std::fs::write(format!("{dir}/hv_{frame}.json"), json).unwrap(),
                    Err(_) => println!("hv_{frame}: the loaded game crashed"),
                }
            }
        }
        "compare" => {
            let dir = &a[2];
            let mut frames: Vec<usize> = std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|e| {
                    let n = e.unwrap().file_name().to_string_lossy().to_string();
                    n.strip_prefix("h_")?.strip_suffix(".json")?.parse().ok()
                })
                .collect();
            frames.sort();
            // First pass: which variables differ between the builds' own runs (render
            // state: the headless build does not draw), to leave out of the rest.
            let mut natural = BTreeSet::new();
            let none = BTreeSet::new();
            for f in &frames {
                first_difference(&format!("{dir}/h_{f}.json"), &format!("{dir}/v_{f}.json"), &none, &mut natural);
            }
            println!("Variables that differ between the builds' own runs (render state), left out: {natural:?}\n");
            let show = |r: Option<String>, exists: bool| if !exists { "missing".to_string() } else { r.map(|d| format!("DIFFERS at {d}")).unwrap_or_else(|| "identical".into()) };
            println!("| checkpoint | builds from boot (headless vs rendering) | headless -> rendering (rendering run vs the loaded one) | rendering -> headless (headless run vs the loaded one) |");
            println!("|---|---|---|---|");
            let mut sink = BTreeSet::new();
            for f in frames {
                let p = |n: &str| format!("{dir}/{n}_{f}.json");
                let ex = |n: &str| std::path::Path::new(&p(n)).exists();
                let boot = show(first_difference(&p("h"), &p("v"), &natural, &mut sink), ex("v"));
                let h2v = show(first_difference(&p("v"), &p("vh"), &natural, &mut sink), ex("vh") && ex("v"));
                let v2h = show(first_difference(&p("h"), &p("hv"), &natural, &mut sink), ex("hv"));
                println!("| {f} | {boot} | {h2v} | {v2h} |");
            }
        }
        _ => eprintln!("usage: see the file's documentation"),
    }
    let _ = Build::Headless;
}
