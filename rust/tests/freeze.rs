//! Object freezing is an opt-in approximation; these check that it is
//! reported, carried with the state, and runs the game without failing.
//! The exact (unfrozen) path is covered by `lockstep.rs`.
//!
//! Like `lockstep.rs`, these use the freshly built headless WASM and skip
//! without it.

use std::path::Path;

use sm64_binds::{FreezeMode, GamePad, SM64GameGenerator};

const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

fn generator() -> Option<SM64GameGenerator> {
    let dir = std::env::var_os("SM64_HEADLESS_BUILD")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(MANIFEST).join("../sm64-port/build/us_web_headless"));
    let wasm = std::fs::read(dir.join("sm64.us.wasm")).ok()?;
    Some(SM64GameGenerator::from_wasm(wasm).unwrap())
}

fn tas(name: &str) -> Option<Vec<GamePad>> {
    let b = std::fs::read(Path::new(MANIFEST).join("../tas").join(name)).ok()?;
    let samples = u32::from_le_bytes(b[0x18..0x1c].try_into().ok()?) as usize;
    Some(
        b[0x400..]
            .as_chunks::<4>()
            .0
            .iter()
            .take(samples)
            .map(|c| GamePad {
                button: u16::from_be_bytes([c[0], c[1]]),
                stick_x: c[2] as i8,
                stick_y: c[3] as i8,
            })
            .collect(),
    )
}

macro_rules! or_skip {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => {
                eprintln!("skipping: headless build or TAS file missing");
                return;
            }
        }
    };
}

#[test]
fn freezing_is_reported_and_travels_with_the_state() {
    let generator = or_skip!(generator());
    let mut exact = generator.create_game().unwrap();
    let mut frozen = generator.create_game().unwrap();
    assert!(exact.is_exact().unwrap());

    frozen.set_object_freeze(FreezeMode::All).unwrap();
    assert!(!frozen.is_exact().unwrap());
    // Switching it off again does not make the game exact.
    frozen.set_object_freeze(FreezeMode::Off).unwrap();
    assert!(!frozen.is_exact().unwrap());

    // Copies, restores and save states carry the marker either way.
    let mut copy = generator.create_game().unwrap();
    copy.copy_from(&frozen).unwrap();
    assert!(!copy.is_exact().unwrap());
    copy.sync_from(&exact).unwrap();
    assert!(copy.is_exact().unwrap());
    copy.load_state(&frozen.save_state()).unwrap();
    assert!(!copy.is_exact().unwrap());
}

#[test]
fn frozen_game_plays_through_a_tas() {
    let generator = or_skip!(generator());
    let pads = or_skip!(tas("tasvideos-6943M-16star.m64"));
    let mut game = generator.create_game().unwrap();
    // Start freezing once the TAS is in the castle grounds.
    for pad in &pads[..2000] {
        game.step_game(*pad).unwrap();
    }
    game.set_object_freeze(FreezeMode::All).unwrap();
    for pad in &pads[2000..] {
        game.step_game(*pad).unwrap();
    }
    let state = game.get_game_state().unwrap();
    eprintln!(
        "frozen: finished on level {} with {} stars",
        state.level_num, state.num_stars
    );
}
