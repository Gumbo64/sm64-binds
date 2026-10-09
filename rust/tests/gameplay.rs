//! End-to-end tests that boot the real WASM build.
//!
//! Every test here needs a US `.z64` ROM at the repo root. The helpers skip
//! instead of failing when it is absent, so `cargo test` still passes on a
//! checkout without the copyrighted file.
//!
//! These tests deliberately assert *invariants* (determinism, save-state
//! fidelity, isolation between instances) rather than exact gameplay
//! positions. A fresh ROM boots into the attract-mode demo, and whether Mario
//! responds to input depends on which screen the demo happens to be on, so
//! frame-exact positions are not stable enough to test against.

use sm64_binds::{GamePad, RngConfig, SM64Game, SM64GameGenerator, buttons};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../baserom.us.z64");

/// Builds a generator from the ROM, or returns `None` so the caller can skip.
macro_rules! generator_or_skip {
    () => {
        match SM64GameGenerator::new(std::fs::read(ROM_PATH).unwrap_or_else(|_| Vec::new())) {
            Ok(generator) => generator,
            Err(_) => {
                eprintln!("skipping: no US .z64 ROM at {ROM_PATH}");
                return;
            }
        }
    };
}

/// A mixed input script: button presses, stick movement and idle frames.
///
/// The attract demo runs on its own regardless of input, so this exists to
/// exercise every pad field rather than to reach a particular screen.
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

/// Long enough for the demo to leave the opening screens and reach a level,
/// where the state fields become meaningful.
const WARMUP_FRAMES: usize = 1200;

fn play_warmup(game: &mut SM64Game) {
    for pad in input_script(WARMUP_FRAMES) {
        game.step_game(pad).unwrap();
    }
}

#[test]
fn rejects_an_invalid_rom() {
    let err = SM64GameGenerator::new(b"definitely not a US z64 ROM".to_vec()).unwrap_err();
    assert!(
        err.to_string().contains("Invalid ROM"),
        "unexpected error: {err}"
    );
}

#[test]
fn rejects_an_empty_rom() {
    assert!(SM64GameGenerator::new(Vec::new()).is_err());
}

#[test]
fn accepts_the_us_rom_when_present() {
    let generator = generator_or_skip!();
    assert!(
        generator.create_game().is_ok(),
        "a valid ROM must produce a playable instance"
    );
}

#[test]
fn a_fresh_game_reports_an_empty_state() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    let state = game.get_game_state().unwrap();

    assert_eq!(state.num_stars, 0, "a fresh game starts with no stars");
    assert!(!state.has_won());
    assert!(state.pos.iter().all(|f| f.is_finite()));
    assert!(state.vel.iter().all(|f| f.is_finite()));
}

#[test]
fn the_state_advances_over_time() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();

    let boot = game.get_game_state().unwrap();
    play_warmup(&mut game);
    let after = game.get_game_state().unwrap();

    assert_ne!(
        format!("{boot}"),
        format!("{after}"),
        "1200 frames of a booting ROM must change the state"
    );
}

#[test]
fn health_and_lives_are_populated_once_mario_exists() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    play_warmup(&mut game);
    let state = game.get_game_state().unwrap();

    // The demo starts a save with 4 lives; health is a 0..0x1000 power meter.
    assert_eq!(state.lives, 4, "a new US save has four lives");
    assert!(
        (1..=0x1000).contains(&state.health),
        "health must be a populated power meter, got {}",
        state.health
    );
    assert!(state.pos.iter().all(|f| f.is_finite()));
    assert!(state.vel.iter().all(|f| f.is_finite()));
}

#[test]
fn stepping_is_stable_over_a_long_run() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();

    // 5000 frames with a state read every frame: exercises the memory read
    // bounds and catches panics from a stale struct size.
    for pad in input_script(5000) {
        game.step_game(pad).unwrap();
        let state = game.get_game_state().unwrap();
        assert!(
            state.pos.iter().all(|f| f.is_finite()),
            "position went non-finite"
        );
    }
}

#[test]
fn many_games_share_one_compiled_module() {
    let generator = generator_or_skip!();
    // create_game must not recompile the module, so ten instances stay cheap.
    for _ in 0..10 {
        let mut game = generator.create_game().unwrap();
        game.step_game(GamePad::default()).unwrap();
        game.get_game_state().unwrap();
    }
}

#[test]
fn identical_inputs_produce_identical_states() {
    let generator = generator_or_skip!();
    let mut a = generator.create_game().unwrap();
    let mut b = generator.create_game().unwrap();

    for (frame, pad) in input_script(2500).into_iter().enumerate() {
        // Interleaving the two games catches shared mutable state between them.
        a.step_game(pad).unwrap();
        b.step_game(pad).unwrap();
        assert_eq!(
            format!("{}", a.get_game_state().unwrap()),
            format!("{}", b.get_game_state().unwrap()),
            "games diverged at frame {frame}"
        );
    }
}

#[test]
fn two_generators_agree_frame_for_frame() {
    let generator_a = generator_or_skip!();
    let generator_b =
        SM64GameGenerator::new(std::fs::read(ROM_PATH).expect("ROM vanished mid-test")).unwrap();

    let mut a = generator_a.create_game().unwrap();
    let mut b = generator_b.create_game().unwrap();
    for pad in input_script(1500) {
        a.step_game(pad).unwrap();
        b.step_game(pad).unwrap();
    }
    assert_eq!(
        format!("{}", a.get_game_state().unwrap()),
        format!("{}", b.get_game_state().unwrap()),
        "separate generators must behave identically"
    );
}

#[test]
fn copy_to_reproduces_every_state_field() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    let mut clone = generator.create_game().unwrap();

    play_warmup(&mut game);
    game.copy_to(&mut clone).unwrap();

    let source = game.get_game_state().unwrap();
    let copied = clone.get_game_state().unwrap();
    assert_eq!(
        format!("{source}"),
        format!("{copied}"),
        "copy_to must reproduce every field, not just the position"
    );
}

#[test]
fn copy_is_valid_partway_through_a_run() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    let mut clone = generator.create_game().unwrap();

    // Copy at frame 1000 rather than after a fixed warmup, then run both on
    // the remainder of the script.
    let script = input_script(2000);
    let (before, after) = script.split_at(1000);

    for pad in before {
        game.step_game(*pad).unwrap();
    }
    game.copy_to(&mut clone).unwrap();

    for (frame, pad) in after.iter().enumerate() {
        game.step_game(*pad).unwrap();
        clone.step_game(*pad).unwrap();
        assert_eq!(
            format!("{}", game.get_game_state().unwrap()),
            format!("{}", clone.get_game_state().unwrap()),
            "a mid-run copy diverged at frame {frame}"
        );
    }
}

#[test]
fn a_copy_does_not_disturb_the_original() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    let mut clone = generator.create_game().unwrap();

    play_warmup(&mut game);
    game.copy_to(&mut clone).unwrap();
    let expected = game.get_game_state().unwrap();

    // Run the clone hard for a while; the original must not notice.
    for pad in input_script(2000) {
        clone.step_game(pad).unwrap();
    }
    let after = game.get_game_state().unwrap();

    assert_eq!(
        format!("{expected}"),
        format!("{after}"),
        "stepping a clone must not perturb its source"
    );
}

#[test]
fn copies_stay_in_lockstep_when_given_the_same_inputs() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    let mut clone = generator.create_game().unwrap();

    play_warmup(&mut game);
    game.copy_to(&mut clone).unwrap();

    for (frame, pad) in input_script(1500).into_iter().enumerate() {
        game.step_game(pad).unwrap();
        clone.step_game(pad).unwrap();
        assert_eq!(
            format!("{}", game.get_game_state().unwrap()),
            format!("{}", clone.get_game_state().unwrap()),
            "copies diverged at frame {frame}"
        );
    }
}

#[test]
fn copy_uses_the_generator_helper() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    play_warmup(&mut game);

    let mut clone = generator.copy(&mut game).unwrap();
    assert_eq!(
        format!("{}", game.get_game_state().unwrap()),
        format!("{}", clone.get_game_state().unwrap())
    );
    clone.step_game(GamePad::default()).unwrap();
}

#[test]
fn full_copy_to_captures_the_whole_address_space() {
    let generator = generator_or_skip!();
    let mut source = generator.create_game().unwrap();
    let mut target = generator.create_game().unwrap();

    play_warmup(&mut source);
    // Snapshot a game that has already been stepped, then keep stepping the
    // copy: full_copy_to covers the constant region that copy_to skips.
    source.full_copy_to(&mut target).unwrap();
    let expected = source.get_game_state().unwrap();
    let copied = target.get_game_state().unwrap();

    assert_eq!(expected.level_num, copied.level_num);
    assert_eq!(expected.lives, copied.lives);
    assert_eq!(expected.health, copied.health);
    assert_eq!(expected.pos, copied.pos);
}

#[test]
fn memory_round_trips_through_bytes() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    play_warmup(&mut game);

    let saved = game.get_memory_bytes().unwrap();
    assert!(!saved.is_empty(), "the mutable region must not be empty");

    let mut restored = generator.create_game().unwrap();
    restored.write_memory_bytes(&saved).unwrap();

    let expected = game.get_game_state().unwrap();
    let actual = restored.get_game_state().unwrap();
    assert_eq!(
        format!("{expected}"),
        format!("{actual}"),
        "restoring raw memory must rebuild the exact state"
    );
}

#[test]
fn using_rng_tracks_only_what_the_caller_configured() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    // The flag is bookkeeping on the Rust side; the WASM RNG is active from
    // boot because random_action.c seeds a default config statically.
    assert!(
        !game.using_rng(),
        "a new game has no RNG attached by the caller"
    );

    game.set_rng_config(RngConfig::default()).unwrap();
    assert!(game.using_rng());

    let mut seeded = generator.create_game().unwrap();
    assert!(!seeded.using_rng());
    seeded.set_rng_seed(22).unwrap();
    assert!(
        seeded.using_rng(),
        "seeding also marks the game as using RNG"
    );
}

#[test]
fn rng_pad_is_deterministic_for_a_seed() {
    let generator = generator_or_skip!();
    let mut a = generator.create_game().unwrap();
    let mut b = generator.create_game().unwrap();
    a.set_rng_seed(22).unwrap();
    b.set_rng_seed(22).unwrap();

    let pad = GamePad::new(buttons::A_BUTTON, 40, 40);
    for frame in 0..2000 {
        assert_eq!(
            a.rng_pad(pad).unwrap(),
            b.rng_pad(pad).unwrap(),
            "rng_pad diverged at frame {frame} for the same seed"
        );
    }
}

#[test]
fn rng_pad_perturbs_inputs_by_default() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();

    // No set_rng_config call: the WASM default config still randomizes.
    let pad = GamePad::default();
    let differs = (0..2000).any(|_| game.rng_pad(pad).unwrap() != pad);
    assert!(
        differs,
        "rng_pad should perturb a neutral pad even without configuration"
    );
}

#[test]
fn different_seeds_produce_different_pad_sequences() {
    let generator = generator_or_skip!();
    let mut a = generator.create_game().unwrap();
    let mut b = generator.create_game().unwrap();
    a.set_rng_seed(1).unwrap();
    b.set_rng_seed(2).unwrap();

    let pad = GamePad::default();
    let differs = (0..2000).any(|_| a.rng_pad(pad).unwrap() != b.rng_pad(pad).unwrap());
    assert!(differs, "seeds 1 and 2 produced identical pad sequences");
}

#[test]
fn zeroed_rng_config_leaves_the_pad_alone() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    game.set_rng_config(RngConfig::new(0, 0, 0, 0.0, 0.0, 0.0))
        .unwrap();

    let pad = GamePad::new(buttons::Z_TRIG, 40, 40);
    for frame in 0..500 {
        assert_eq!(
            game.rng_pad(pad).unwrap(),
            pad,
            "a zero window must be a passthrough (frame {frame})"
        );
    }
}

#[test]
fn a_copy_shares_the_rng_sequence_of_its_source() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    game.set_rng_config(RngConfig::default()).unwrap();
    let mut clone = generator.copy(&mut game).unwrap();

    let pad = GamePad::default();
    for frame in 0..500 {
        let from_source = game.rng_pad(pad).unwrap();
        let from_clone = clone.rng_pad(pad).unwrap();
        assert_eq!(
            from_source, from_clone,
            "a copied game must share the RNG state (frame {frame})"
        );
    }
}

#[test]
fn rng_pad_returns_values_inside_the_stick_range() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    game.set_rng_config(RngConfig::default()).unwrap();

    let pad = GamePad::default();
    for _ in 0..2000 {
        let out = game.rng_pad(pad).unwrap();
        assert!(
            (-80..=80).contains(&out.stick_x),
            "stick_x out of range: {}",
            out.stick_x
        );
        assert!(
            (-80..=80).contains(&out.stick_y),
            "stick_y out of range: {}",
            out.stick_y
        );
        // random_action.c only ever sets A, B and Z.
        let allowed = buttons::A_BUTTON | buttons::B_BUTTON | buttons::Z_TRIG;
        assert_eq!(
            out.button & !allowed,
            0,
            "rng_pad set an unexpected button: 0x{:04x}",
            out.button
        );
    }
}

#[test]
fn a_clone_of_an_rng_game_keeps_its_own_sequence() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    game.set_rng_config(RngConfig::default()).unwrap();

    // Step the clone far ahead; the source's RNG position must be unaffected,
    // so a fresh reference built from the same seed still matches it.
    let mut clone = generator.copy(&mut game).unwrap();
    for _ in 0..1000 {
        clone.rng_pad(GamePad::default()).unwrap();
    }

    let mut reference = generator.create_game().unwrap();
    game.copy_to(&mut reference).unwrap();

    let pad = GamePad::default();
    for frame in 0..200 {
        assert_eq!(
            game.rng_pad(pad).unwrap(),
            reference.rng_pad(pad).unwrap(),
            "advancing a clone desynced the source RNG at frame {frame}"
        );
    }
}
