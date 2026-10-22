//! [`SaveState`]: saving, turning into bytes and back, loading, and refusing
//! what is not a state of this build.
//!
//! Needs a US `.z64` ROM at the repo root, like the other game tests.

use sm64_binds::{GamePad, SM64Game, SM64GameGenerator, SaveState, buttons};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../baserom.us.z64");

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

fn pad(i: usize) -> GamePad {
    GamePad {
        button: if i.is_multiple_of(3) {
            buttons::A_BUTTON
        } else {
            0
        },
        stick_x: if i.is_multiple_of(5) { 60 } else { -20 },
        stick_y: (i % 7) as i8 * 10,
    }
}

fn play(game: &mut SM64Game, frames: std::ops::Range<usize>) {
    for i in frames {
        game.step_game(pad(i)).unwrap();
    }
}

/// The stack and heap, the parts a save state holds.
fn mutable(game: &SM64Game) -> (Vec<u8>, Vec<u8>) {
    let read_only = game.read_only_memory();
    game.with_memory(|m| (m[..read_only.start].to_vec(), m[read_only.end..].to_vec()))
}

#[test]
fn a_state_through_bytes_makes_another_game_the_same() {
    let generator = generator_or_skip!();
    let mut source = generator.create_game().unwrap();
    play(&mut source, 0..1500);

    let bytes = source.save_state().into_bytes();
    let mut target = generator.create_game().unwrap();
    target
        .load_state(&SaveState::from_bytes(bytes).unwrap())
        .unwrap();
    assert!(mutable(&source) == mutable(&target), "memory after loading");

    // And they play on identically.
    play(&mut source, 1500..1800);
    play(&mut target, 1500..1800);
    assert!(
        mutable(&source) == mutable(&target),
        "memory after playing on"
    );
    assert_eq!(
        format!("{:?}", source.get_game_state().unwrap()),
        format!("{:?}", target.get_game_state().unwrap())
    );
}

#[test]
fn as_bytes_and_from_bytes_round_trip() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    play(&mut game, 0..300);
    let state = game.save_state();
    let again = SaveState::from_bytes(state.as_bytes().to_vec()).unwrap();
    assert_eq!(state, again);
    assert!(format!("{state:?}").contains("SaveState"));
}

/// The bytes after the header are the stack, then everything after the
/// read-only memory.
#[test]
fn the_payload_is_the_stack_then_the_rest() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    play(&mut game, 0..300);
    let state = game.save_state();
    let (stack, rest) = mutable(&game);
    assert_eq!(&state.as_bytes()[44..], &[stack, rest].concat()[..]);
}

#[test]
fn from_bytes_refuses_what_is_not_a_state() {
    let generator = generator_or_skip!();
    let game = generator.create_game().unwrap();
    let good = game.save_state().into_bytes();

    let mut magic = good.clone();
    magic[0] ^= 1;
    assert!(SaveState::from_bytes(magic).is_err(), "bad magic");

    let mut version = good.clone();
    version[8] = 2;
    assert!(SaveState::from_bytes(version).is_err(), "unknown version");

    let truncated = good[..good.len() - 1].to_vec();
    assert!(SaveState::from_bytes(truncated).is_err(), "truncated");

    let mut longer = good.clone();
    longer.push(0);
    assert!(SaveState::from_bytes(longer).is_err(), "trailing bytes");

    assert!(SaveState::from_bytes(Vec::new()).is_err(), "empty");
    assert!(SaveState::from_bytes(good).is_ok());
}

/// A state of another build is refused and the game is left as it was.
#[test]
fn load_state_refuses_another_build() {
    let generator = generator_or_skip!();
    let mut source = generator.create_game().unwrap();
    play(&mut source, 0..600);
    let mut bytes = source.save_state().into_bytes();
    bytes[12] ^= 0xff; // the build id
    let foreign = SaveState::from_bytes(bytes).unwrap();

    let mut target = generator.create_game().unwrap();
    play(&mut target, 0..100);
    let before = mutable(&target);
    let error = target.load_state(&foreign).unwrap_err();
    assert!(error.to_string().contains("build"), "{error}");
    assert!(mutable(&target) == before, "the game was changed");
}
