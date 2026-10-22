//! The precompiled module (`pkg/*.wasmu`, the default on x86-64 Linux) against
//! Cranelift: the same WASM, the same input, and all of linear memory equal,
//! stack included.
//!
//! Needs the headless build the module was precompiled from (see
//! `tests/lockstep.rs`); skips without it or without the TAS files.
#![cfg(all(
    feature = "precompiled",
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "gnu"
))]

use std::path::{Path, PathBuf};

use sm64_binds::{Backend, GamePad, SM64Game, SM64GameGenerator};

const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

fn wasm() -> Option<Vec<u8>> {
    let dir = std::env::var_os("SM64_HEADLESS_BUILD")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(MANIFEST).join("../sm64-port/build/us_web_headless"));
    std::fs::read(dir.join("sm64.us.wasm")).ok()
}

/// Generators of the build: Cranelift and precompiled. `None` (skip) if there
/// is no build or no precompiled module for it.
struct Generators {
    wasm: Vec<u8>,
    cranelift: SM64GameGenerator,
    precompiled: SM64GameGenerator,
}

fn setup() -> Option<Generators> {
    let Some(wasm) = wasm() else {
        eprintln!("skipping: no headless build");
        return None;
    };
    let precompiled = match SM64GameGenerator::from_wasm_with(&wasm, Backend::Precompiled) {
        Ok(generator) => generator,
        Err(e) => {
            eprintln!("skipping: {e}");
            return None;
        }
    };
    let cranelift = SM64GameGenerator::from_wasm_with(&wasm, Backend::Cranelift).unwrap();
    Some(Generators {
        wasm,
        cranelift,
        precompiled,
    })
}

fn movie(path: &Path) -> Vec<GamePad> {
    let bytes = std::fs::read(path).unwrap();
    let count = u32::from_le_bytes(bytes[0x18..0x1c].try_into().unwrap()) as usize;
    bytes[0x400..]
        .as_chunks::<4>()
        .0
        .iter()
        .take(count)
        .map(|c| GamePad {
            button: u16::from_be_bytes([c[0], c[1]]),
            stick_x: c[2] as i8,
            stick_y: c[3] as i8,
        })
        .collect()
}

fn movies() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(Path::new(MANIFEST).join("../tas"))
        .map(|dir| {
            dir.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "m64"))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
}

fn same_memory(a: &SM64Game, b: &SM64Game) -> Option<usize> {
    a.with_memory(|ma| {
        b.with_memory(|mb| {
            if ma.len() != mb.len() {
                return Some(ma.len().min(mb.len()));
            }
            ma.iter().zip(mb).position(|(x, y)| x != y)
        })
    })
}

/// Plays `pads` on both, comparing memory every `every` frames and at the end.
fn play(
    label: &str,
    cranelift: &mut SM64Game,
    precompiled: &mut SM64Game,
    pads: &[GamePad],
    every: usize,
) {
    for (frame, pad) in pads.iter().enumerate() {
        cranelift.step_game(*pad).unwrap();
        precompiled.step_game(*pad).unwrap();
        if (frame % every == 0 || frame + 1 == pads.len())
            && let Some(at) = same_memory(cranelift, precompiled)
        {
            panic!("{label}: memory differs at {at:#x} after frame {frame}");
        }
    }
}

#[test]
fn auto_uses_the_precompiled_module_for_its_build() {
    let Some(g) = setup() else {
        return;
    };
    assert_eq!(
        g.precompiled.create_game().unwrap().backend(),
        "precompiled"
    );
    assert_eq!(g.cranelift.create_game().unwrap().backend(), "cranelift");
    if std::env::var_os("SM64_BACKEND").is_none() {
        let auto = SM64GameGenerator::from_wasm(&g.wasm).unwrap();
        assert_eq!(auto.create_game().unwrap().backend(), "precompiled");
    }
}

#[test]
fn precompiled_matches_cranelift_on_the_tas_runs() {
    let Some(g) = setup() else {
        return;
    };
    let cranelift = g.cranelift.create_game().unwrap();
    let precompiled = g.precompiled.create_game().unwrap();
    assert_eq!(
        same_memory(&cranelift, &precompiled),
        None,
        "after main_func"
    );

    let paths = movies();
    if paths.is_empty() {
        eprintln!("skipping the movies: none in ../tas");
    }
    for path in paths {
        let label = path.file_name().unwrap().to_string_lossy().into_owned();
        let pads = movie(&path);
        let mut cranelift = g.cranelift.create_game().unwrap();
        let mut precompiled = g.precompiled.create_game().unwrap();
        play(&label, &mut cranelift, &mut precompiled, &pads, 16);
        let state = precompiled.get_game_state().unwrap();
        eprintln!(
            "{label}: {} frames equal, {} stars",
            pads.len(),
            state.num_stars
        );
    }
}

/// With rendering and audio on, which run code the headless defaults skip.
#[test]
fn precompiled_matches_cranelift_rendering() {
    let Some(g) = setup() else { return };
    let path = Path::new(MANIFEST).join("../tas/tasvideos-6943M-16star.m64");
    if !path.exists() {
        eprintln!("skipping: no {}", path.display());
        return;
    }
    let pads = movie(&path);
    let mut cranelift = g.cranelift.create_game().unwrap();
    let mut precompiled = g.precompiled.create_game().unwrap();
    for game in [&mut cranelift, &mut precompiled] {
        game.set_render_enabled(true).unwrap();
        game.set_audio_enabled(true).unwrap();
    }
    play("rendering", &mut cranelift, &mut precompiled, &pads, 64);
}

/// Save states and restores carry over between the two.
#[test]
fn precompiled_save_states_and_syncs() {
    let Some(g) = setup() else {
        return;
    };
    let mut base = g.precompiled.create_game().unwrap();
    let path = Path::new(MANIFEST).join("../tas/tasvideos-6943M-16star.m64");
    if !path.exists() {
        eprintln!("skipping: no {}", path.display());
        return;
    }
    let pads = movie(&path);
    for pad in &pads[..5000] {
        base.step_game(*pad).unwrap();
    }

    // A Cranelift game from the precompiled one's save state...
    let mut reference = g.cranelift.create_game().unwrap();
    reference.load_state(&base.save_state()).unwrap();
    // ...and a precompiled branch synced from it, twice (the second sync
    // takes the dirty-page path where it is available).
    let mut branch = g.precompiled.create_game().unwrap();
    for round in 0..2 {
        branch.sync_from(&base).unwrap();
        let mut cranelift = g.cranelift.create_game().unwrap();
        cranelift.copy_from(&reference).unwrap();
        play(
            &format!("branch {round}"),
            &mut cranelift,
            &mut branch,
            &pads[5000..7000],
            50,
        );
    }
}

/// An access outside memory is an error, as under Cranelift, and the process
/// carries on.
#[test]
fn precompiled_traps_cleanly() {
    let Some(g) = setup() else {
        return;
    };
    let mut game = g.precompiled.create_game().unwrap();
    let path = Path::new(MANIFEST).join("../tas/tasvideos-6943M-16star.m64");
    if !path.exists() {
        eprintln!("skipping: no {}", path.display());
        return;
    }
    // In the castle grounds, where Mario's object is updated every frame.
    let pads = movie(&path);
    for pad in &pads[..2000] {
        game.step_game(*pad).unwrap();
    }
    // MarioState.marioObj, pointed far outside memory.
    let mario = game.call_export_u32("get_mario_state").unwrap();
    let (store, _, memory) = game.wasm_parts();
    memory
        .view(store)
        .write(u64::from(mario) + 0x88, &0xFFFF_FF00u32.to_le_bytes())
        .unwrap();
    let error = game.step_game(pads[2000]).unwrap_err();
    assert!(format!("{error:#}").contains("out of bounds"), "{error:#}");

    let mut fresh = g.precompiled.create_game().unwrap();
    for _ in 0..200 {
        fresh.step_game(GamePad::default()).unwrap();
    }
}
