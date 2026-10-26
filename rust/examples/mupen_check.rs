//! Loads a Mupen64 savestate into the port and checks it against the port's
//! own replay of the same movie.
//!
//! cargo run --release --example mupen_check -- <rom> <movie.m64> <state.st> [earlier frames] [frames after]
//!
//! 1. Parses the state and prints what it holds.
//! 2. Replays the movie in the port to the state's frame (rr states record it;
//!    otherwise the frame whose gGlobalTimer matches) as the reference.
//! 3. Replays a second game to `earlier` frames before that (default 30, in the
//!    same area), loads the state into it, and compares every copied variable
//!    and Mario with the reference, then keeps playing both for 600 frames (or
//!    `frames after`) and reports the first frame where Mario differs.
//!
//! Environment: `MUPEN_FROM=<frame>` starts the second game from that frame
//! instead (warping to the state's area); `MUPEN_LATER=<st>` also plays both to
//! a later state of the same movie and compares with it (the emulator itself);
//! `MUPEN_DETAIL`, `MUPEN_WATCH=<n>`, `MUPEN_ALL`, `MUPEN_ENVFX` and
//! `MUPEN_DUMP=<dir>` print or dump more for debugging.

use anyhow::{Result, bail};
use sm64_binds::mupen::{MupenState, copied_variables, port_variable};
use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};

fn pads(path: &str) -> Result<Vec<GamePad>> {
    let b = std::fs::read(path)?;
    if b.len() < 0x400 || &b[..4] != b"M64\x1a" {
        bail!("not an m64");
    }
    let n = u32::from_le_bytes(b[0x18..0x1C].try_into().unwrap()) as usize;
    Ok(b[0x400..]
        .as_chunks::<4>()
        .0
        .iter()
        .take(n)
        .map(|c| GamePad {
            button: u16::from_be_bytes([c[0], c[1]]),
            stick_x: c[2] as i8,
            stick_y: c[3] as i8,
        })
        .collect())
}

fn read(g: &SM64Game, name: &str) -> Vec<u8> {
    let (a, s) = port_variable(name).unwrap();
    let mut b = vec![0; s as usize];
    g.read_memory(a, &mut b).unwrap();
    b
}
fn f32s(b: &[u8], o: usize, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| f32::from_le_bytes(b[o + 4 * i..o + 4 * i + 4].try_into().unwrap()))
        .collect()
}
fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn mario(g: &SM64Game) -> (u32, Vec<f32>, Vec<f32>, f32) {
    let m = read(g, "gMarioStates");
    (
        u32at(&m, 0x0C),
        f32s(&m, 0x3C, 3),
        f32s(&m, 0x48, 3),
        f32s(&m, 0x54, 1)[0],
    )
}
fn timer(g: &SM64Game) -> u32 {
    u32at(&read(g, "gGlobalTimer"), 0)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        bail!("usage: mupen_check <rom> <movie.m64> <state.st> [earlier frames] [frames after]");
    }
    let st = MupenState::from_file(&args[3])?;
    let earlier: usize = args.get(4).map_or(30, |s| s.parse().unwrap());
    let after: usize = args.get(5).map_or(600, |s| s.parse().unwrap());
    let ms = st.mario_state_bytes().unwrap();
    println!(
        "{:?}\n  level {:?} area {:?} timer {:?} mario action {:#x} pos {:?} vel {:?}",
        st,
        st.level(),
        st.area(),
        st.global_timer(),
        u32at(&ms, 0x0C),
        f32s(&ms, 0x3C, 3),
        f32s(&ms, 0x48, 3)
    );
    let pads = pads(&args[2])?;
    let generator = SM64GameGenerator::from_file(&args[1])?;
    let timer_n64 = st.global_timer().unwrap();

    // The reference: the port at the state's frame.
    let mut reference = generator.create_game()?;
    let mut frame = 0usize;
    while timer(&reference) < timer_n64 && frame < pads.len() {
        reference.step_game(pads[frame])?;
        frame += 1;
    }
    let (a, p, v, f) = mario(&reference);
    println!(
        "port after {frame} frames: timer {} action {a:#x} pos {p:?} vel {v:?} fvel {f}{}",
        timer(&reference),
        st.movie.as_ref().map_or(String::new(), |m| format!(
            " (rr movie sample {})",
            m.current_sample
        ))
    );

    // Same frame: loading should change little.
    let mut same = generator.copy_from(&reference)?;
    let r = same.load_mupen_state(&st)?;
    println!("load into the same frame:\n{r}");
    diff(&reference, &same, "same frame");

    if let Ok(dir) = std::env::var("MUPEN_DUMP") {
        // Load into frame-earlier and frame-earlier+1 and dump both memories.
        for (k, e) in [earlier, earlier - 1].into_iter().enumerate() {
            let mut g = generator.create_game()?;
            for pad in &pads[..frame - e] {
                g.step_game(*pad)?;
            }
            g.warp_to_mupen_area(&st, 600)?;
            g.load_mupen_state(&st)?;
            g.with_memory(|m| std::fs::write(format!("{dir}/mem{k}.bin"), m))?;
        }
    }

    // Earlier frame, then load.
    let mut game = generator.create_game()?;
    for pad in &pads[..frame - earlier] {
        game.step_game(*pad)?;
    }
    if std::env::var("MUPEN_FROM").is_ok() {
        // Start from another frame instead (e.g. another level), warping.
        let from: usize = std::env::var("MUPEN_FROM").unwrap().parse()?;
        game = generator.create_game()?;
        for pad in &pads[..from] {
            game.step_game(*pad)?;
        }
        println!(
            "warped from frame {from} in {} frames",
            game.warp_to_mupen_area(&st, 600)?
        );
    }
    let warp = game.warp_to_mupen_area(&st, 600)?;
    if warp > 0 {
        println!("warped to the state's area in {warp} frames");
    }
    let r = game.load_mupen_state(&st)?;
    println!("load into {earlier} frames earlier:\n{r}");
    for (where_, at, v) in r
        .kept
        .iter()
        .filter(|k| !k.0.starts_with("credits") && !k.0.starts_with("sStarSelectorModels"))
        .take(80)
    {
        println!("  kept {where_} ({at:#x}) = {v:#010x}");
    }
    diff(&reference, &game, "earlier");
    if std::env::var("MUPEN_ENVFX").is_ok() {
        println!("envfx: {:?}", r.envfx);
        let buf = |g: &SM64Game| u32at(&read(g, "gEnvFxBuffer"), 0);
        let (a, b) = (buf(&reference), buf(&game));
        let (mut x, mut y) = (vec![0; 56 * 140], vec![0; 56 * 140]);
        reference.read_memory(a, &mut x)?;
        game.read_memory(b, &mut y)?;
        let bad: Vec<usize> = (0..x.len() / 4)
            .filter(|&k| x[4 * k..4 * k + 4] != y[4 * k..4 * k + 4])
            .collect();
        println!(
            "envfx buffers {a:#x} {b:#x}: {} words differ, first {:?}",
            bad.len(),
            &bad[..bad.len().min(10)]
        );
    }
    let mut first = None;
    for (i, pad) in pads[frame..(frame + after).min(pads.len())]
        .iter()
        .enumerate()
    {
        reference.step_game(*pad)?;
        game.step_game(*pad)?;
        if first.is_none()
            && std::env::var("MUPEN_WATCH").is_ok_and(|w| w.parse::<usize>().unwrap() == i + 1)
        {
            diff(
                &reference,
                &game,
                &format!("{} frames after loading", i + 1),
            );
            let cam = |g: &SM64Game| {
                let mut b = vec![0u8; 108];
                g.read_memory(u32at(&read(g, "gCamera"), 0), &mut b)
                    .unwrap();
                b
            };
            let (x, y) = (cam(&reference), cam(&game));
            let bad: Vec<usize> = (0..27)
                .filter(|&k| x[4 * k..4 * k + 4] != y[4 * k..4 * k + 4])
                .map(|k| 4 * k)
                .collect();
            println!("  camera struct: differing offsets {bad:x?}");
        }
        if first.is_none() && mario(&reference) != mario(&game) {
            first = Some(i + 1);
            println!(
                "  first Mario difference {} frames after loading:\n    ref  {:?}\n    load {:?}",
                i + 1,
                mario(&reference),
                mario(&game)
            );
        }
    }
    diff(&reference, &game, &format!("{after} frames after loading"));
    if let Ok(later) = std::env::var("MUPEN_LATER") {
        // Play both on to a later state of the same movie and compare each
        // with what the emulator had there.
        let later = MupenState::from_file(&later)?;
        let target = later.global_timer().unwrap();
        let mut k = frame + after;
        while timer(&game) < target && k < pads.len() {
            reference.step_game(pads[k])?;
            game.step_game(pads[k])?;
            k += 1;
        }
        let m = later.mario_state_bytes().unwrap();
        let truth = (
            u32at(&m, 0x0C),
            f32s(&m, 0x3C, 3),
            f32s(&m, 0x48, 3),
            f32s(&m, 0x54, 1)[0],
        );
        println!(
            "at the later state's frame (timer {target}):\n  emulator {truth:?}\n  ref      {:?}\n  loaded   {:?}",
            mario(&reference),
            mario(&game)
        );
    }
    let (a, p, ..) = mario(&reference);
    let (b, q, ..) = mario(&game);
    println!(
        "after {after} more frames: {} (ref action {a:#x} pos {p:?}, loaded action {b:#x} pos {q:?})",
        first.map_or("in sync".to_string(), |f| format!("diverged at +{f}"))
    );
    Ok(())
}

/// Copied variables that differ between two games, by bytes.
fn diff(a: &SM64Game, b: &SM64Game, label: &str) {
    let mut bad = Vec::new();
    let all = std::env::var("MUPEN_ALL").is_ok();
    let vars: Vec<(&str, u32, u32)> = if all {
        sm64_binds::mupen::all_variables()
            .into_iter()
            .map(|(n, a, s, _)| (n, a, s))
            .collect()
    } else {
        copied_variables()
    };
    for (name, addr, size) in vars {
        let (mut x, mut y) = (vec![0; size as usize], vec![0; size as usize]);
        a.read_memory(addr, &mut x).unwrap();
        b.read_memory(addr, &mut y).unwrap();
        let n = x.iter().zip(&y).filter(|(p, q)| p != q).count();
        if n > 0 {
            let first = x.iter().zip(&y).position(|(p, q)| p != q).unwrap();
            bad.push((name, n, size, first));
        }
    }
    println!(
        "{label}: {} of {} copied variables differ from the reference",
        bad.len(),
        copied_variables().len()
    );
    for (name, n, size, first) in bad.iter().take(60) {
        println!("  {name}: {n}/{size} bytes, first at +{first:#x}");
    }
    if std::env::var("MUPEN_DETAIL").is_ok() {
        for (name, ..) in &bad {
            let x = read(a, name);
            let y = read(b, name);
            let mut shown = 0;
            let mut o = 0;
            while o + 4 <= x.len() && shown < 24 {
                if x[o..o + 4] != y[o..o + 4] {
                    let (wx, wy) = (u32at(&x, o), u32at(&y, o));
                    let slot = if *name == "gObjectPool" {
                        format!(" slot {} +{:#x}", o / 0x260, o % 0x260)
                    } else {
                        String::new()
                    };
                    println!("    {name}+{o:#x}{slot}: ref {wx:#010x} loaded {wy:#010x}");
                    shown += 1;
                }
                o += 4;
            }
        }
    }
}
