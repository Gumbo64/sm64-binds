//! Times the game's hot paths: stepping, save states and instantiation.
//!
//! Run with: cargo run --release --bench hot_paths -- ../baserom.us.z64
//!
//! The cases mirror `python/benchs/sm64_cases.py` case for case, so a number
//! in one binding can be set against the same number in the other. The shared
//! measurement machinery lives in `harness.rs` alongside this file.

use std::hint::black_box;

use sm64_binds::{GamePad, SM64GameGenerator, buttons};

mod harness;
use harness::{Bencher, rom_path_from_args, target_from_args};

/// One emulator session's worth of frames, shared by the amortised stepping
/// case and by the warm-up that puts the other cases in a playing state.
const FRAMES: usize = 2000;

/// A deterministic but varied input script: buttons and stick values cycle
/// with different moduli, so every frame presses something different while
/// every run of the script replays exactly the same presses. Deliberately
/// shared with the Python harness, so stepping numbers compare bindings, not
/// input styles.
fn input_script(frames: usize) -> Vec<GamePad> {
    (0..frames)
        .map(|i| {
            let mut pad = GamePad::default();
            if i % 2 == 0 {
                pad.button = buttons::A_BUTTON;
            }
            if i % 17 == 0 {
                pad.button |= buttons::START_BUTTON;
            }
            if i % 7 == 0 {
                pad.button |= buttons::Z_TRIG;
            }
            pad.stick_y = match i % 4 {
                0 => 80,
                1 => -80,
                _ => 0,
            };
            pad.stick_x = if i % 5 == 0 { -80 } else { 0 };
            pad
        })
        .collect()
}

fn main() {
    let mut b = Bencher::new(target_from_args(3.0));

    // ---------------------------------------------------------------
    b.section("setup (once per process)");
    let rom_path = rom_path_from_args("../baserom.us.z64");
    let mut generator_holder: Option<SM64GameGenerator> = None;
    // Module compilation happens once per process and costs more than
    // re-sampling could justify; both single-shot rows are magnitudes, not
    // comparisons.
    b.measure_once("SM64GameGenerator::new (sha1+xor+unzip+compile)", || {
        let rom = std::fs::read(&rom_path).expect("no ROM");
        generator_holder = Some(SM64GameGenerator::new(rom).expect("generator"));
    });
    let generator = generator_holder.expect("generator");
    // Measured once, and kept: the derived per-frame row below needs this
    // number as its subtraction term.
    b.measure_once("create_game (instantiate + main_func)", || {
        black_box(generator.create_game().expect("game"));
    });
    let mut game = generator.create_game().unwrap();

    // ---------------------------------------------------------------
    b.section("stepping");
    let script = input_script(FRAMES);
    let frames_case = format!("create_game + {FRAMES} frames");

    // Frame cost depends on what the emulated game is doing, so the game is
    // stepped through its intro first — otherwise the first samples are
    // title-screen frames and the later ones are gameplay frames.
    for pad in &script {
        game.step_game(*pad).unwrap();
    }
    // One binding-level step call on an otherwise steady input: the pure
    // per-frame number, with instantiations long since amortised away.
    b.measure("step_game (in game)", 1.0, "frame", || {
        game.step_game(GamePad::default()).unwrap();
    });

    // A whole session: one instantiation amortised over the frames played.
    // `units_per_block = FRAMES` converts the block into a per-frame figure;
    // the derived row then peels the instantiation back out.
    b.measure(&frames_case, FRAMES as f64, "frame", || {
        let mut fresh = generator.create_game().unwrap();
        for pad in &script {
            fresh.step_game(*pad).unwrap();
        }
        black_box(fresh);
    });
    b.derived(
        "step_game per frame",
        &frames_case,
        "create_game (instantiate + main_func)",
        1.0 / FRAMES as f64,
        "frame",
    );

    // ---------------------------------------------------------------
    b.section("per-call overhead");
    // Both are WASM-call-bound: the engine call into the game plus reading a
    // fixed-size blob back out of linear memory. `diagnostics` measures the
    // floor for this (an empty call) if the split ever needs investigating.
    b.measure("get_game_state", 1.0, "call", || {
        black_box(game.get_game_state().unwrap());
    });
    b.measure("rng_pad", 1.0, "call", || {
        black_box(game.rng_pad(GamePad::default()).unwrap());
    });

    // ---------------------------------------------------------------
    b.section("save states");
    for pad in &script {
        game.step_game(*pad).unwrap();
    }

    let save = game.get_memory_bytes().unwrap();
    b.note(&format!(
        "save state size: {:.2} MB (linear memory {:.2} MB)",
        save.len() as f64 / 1048576.0,
        // `save` holds the mutable ranges only; the read-only segment between
        // them (see `sm64.rs`) completes the address space.
        (save.len() + sm64_binds::DATA0_SIZE) as f64 / 1048576.0
    ));

    // Every save-state path, from the slowest (fresh instance + copy) to the
    // primitives they are built from. `copy` is `create_game` + `copy_to`,
    // which is why it dominates all the warm-target copies.
    let mut clone = generator.create_game().unwrap();
    b.measure("SM64GameGenerator::copy (clone)", 1.0, "call", || {
        black_box(generator.copy(&mut game).unwrap());
    });
    b.measure("copy_to (warm target)", 1.0, "call", || {
        game.copy_to(&mut clone).unwrap();
    });
    b.measure("full_copy_to (whole address space)", 1.0, "call", || {
        game.full_copy_to(&mut clone).unwrap();
    });
    b.measure("get_memory_bytes (alloc + copy)", 1.0, "call", || {
        black_box(game.get_memory_bytes().unwrap());
    });
    b.measure("write_memory_bytes (warm target)", 1.0, "call", || {
        clone.write_memory_bytes(&save).unwrap();
    });
}
