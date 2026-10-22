//! `sync_from` must always leave a game byte-identical to its base,
//! whether it takes the dirty-page path or falls back to a full copy.
//!
//! Needs a US `.z64` ROM at the repo root, like the other game tests.

mod common;
use common::RawState;

use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../baserom.us.z64");
const TAS_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../tas/tasvideos-6943M-16star.m64"
);

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

/// xorshift32, for reproducible inputs and schedules.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn pad(&mut self) -> GamePad {
        GamePad {
            button: (self.next() & 0xe030) as u16
                & if self.next().is_multiple_of(4) {
                    0xffff
                } else {
                    0
                },
            stick_x: self.next() as i8,
            stick_y: self.next() as i8,
        }
    }
}

/// The 16-star TAS inputs, or random ones if the file is missing.
fn tas_inputs(frames: usize) -> Vec<GamePad> {
    match std::fs::read(TAS_PATH) {
        Ok(b) => b[0x400..]
            .as_chunks::<4>()
            .0
            .iter()
            .take(frames)
            .map(|c| GamePad {
                button: u16::from_be_bytes([c[0], c[1]]),
                stick_x: c[2] as i8,
                stick_y: c[3] as i8,
            })
            .collect(),
        Err(_) => {
            let mut rng = Rng(7);
            (0..frames).map(|_| rng.pad()).collect()
        }
    }
}

fn assert_same_memory(game: &SM64Game, base: &SM64Game, context: &str) {
    let (a, b) = (game.memory_copy(), base.memory_copy());
    assert_eq!(a.len(), b.len(), "{context}: memory sizes differ");
    if let Some(at) = a.iter().zip(&b).position(|(x, y)| x != y) {
        panic!("{context}: byte {at:#x} differs from the base after sync_from");
    }
}

#[test]
fn sync_from_matches_base_through_random_schedule() {
    let generator = generator_or_skip!();
    let mut base = generator.create_game().unwrap();
    // Into a level, so stepping touches real game state.
    for pad in tas_inputs(5000) {
        base.step_game(pad).unwrap();
    }
    let mut other = generator.copy_from(&base).unwrap();
    let mut game = generator.create_game().unwrap();
    let mut rng = Rng(0x1234_5678);
    let mut fast_restores = 0;

    for i in 0..3000 {
        let tracking_before = game.dirty_tracking_active();
        game.sync_from(&base).unwrap();
        if tracking_before {
            fast_restores += 1;
        }
        assert_same_memory(&game, &base, &format!("iteration {i}"));

        let frames = [0, 1, 1, 3, 10, 60, 600][rng.next() as usize % 7];
        for _ in 0..frames {
            game.step_game(rng.pad()).unwrap();
        }
        // Sometimes change the base, which must force a full copy next time.
        match rng.next() % 50 {
            0 => base.step_game(rng.pad()).unwrap(),
            1 => {
                other.step_game(rng.pad()).unwrap();
                base.sync_from(&other).unwrap();
            }
            2 => {
                let state = base.state_payload();
                base.load_payload(&state).unwrap();
            }
            _ => {}
        }
    }
    eprintln!(
        "dirty tracking active: {}, {fast_restores} restores with tracking",
        game.dirty_tracking_active()
    );
}

/// Two games synced from the same base must then behave identically.
#[test]
fn synced_game_steps_like_a_full_copy() {
    let generator = generator_or_skip!();
    let mut base = generator.create_game().unwrap();
    for pad in tas_inputs(4000) {
        base.step_game(pad).unwrap();
    }
    let mut synced = generator.create_game().unwrap();
    let mut rng = Rng(99);
    for round in 0..20 {
        synced.sync_from(&base).unwrap();
        let mut copied = generator.copy_from(&base).unwrap();
        for frame in 0..300 {
            let pad = rng.pad();
            synced.step_game(pad).unwrap();
            copied.step_game(pad).unwrap();
            assert_eq!(
                format!("{:?}", synced.get_game_state().unwrap()),
                format!("{:?}", copied.get_game_state().unwrap()),
                "round {round}, frame {frame}"
            );
        }
        assert_eq!(synced.memory_copy(), copied.memory_copy(), "round {round}");
    }
}

/// A destination whose memory grew must still be synced correctly.
#[test]
fn sync_from_handles_memory_growth() {
    let generator = generator_or_skip!();
    let mut base = generator.create_game().unwrap();
    for pad in tas_inputs(3000) {
        base.step_game(pad).unwrap();
    }
    let mut game = generator.create_game().unwrap();
    game.sync_from(&base).unwrap();
    game.sync_from(&base).unwrap();

    // Grow the base by one WASM page; both must end up the same size.
    let mut state = base.state_payload();
    state.extend(std::iter::repeat_n(0, 65536));
    base.load_payload(&state).unwrap();
    for _ in 0..3 {
        game.sync_from(&base).unwrap();
        assert_same_memory(&game, &base, "after growth");
        for _ in 0..30 {
            game.step_game(GamePad::default()).unwrap();
        }
    }
}
