//! Portable save states (`SM64Game::export_portable` / `load_portable`): a state
//! exported at a point of a TAS movie, loaded into another game, goes on like the
//! original. Needs the ROM at `../baserom.us.z64` and `../tas/` (skipped without).
//!
//! The cross-build half (headless <-> rendering) needs a browser: see
//! `examples/portable_check.rs` and `javascript/e2e/`.

use sm64_binds::portable::Portable;
use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../baserom.us.z64");
const MOVIE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../tas/tasvideos-7310M-120star.m64"
);

macro_rules! setup_or_skip {
    () => {{
        let rom = std::fs::read(ROM_PATH).unwrap_or_default();
        let Ok(generator) = SM64GameGenerator::new(rom) else {
            eprintln!("skipping: no US .z64 ROM at {ROM_PATH}");
            return;
        };
        let Ok(movie) = std::fs::read(MOVIE) else {
            eprintln!("skipping: no movie at {MOVIE}");
            return;
        };
        (generator, pads(&movie))
    }};
}

fn pads(m64: &[u8]) -> Vec<GamePad> {
    let n = u32::from_le_bytes(m64[0x18..0x1C].try_into().unwrap()) as usize;
    m64[0x400..]
        .as_chunks::<4>()
        .0
        .iter()
        .take(n)
        .map(|c| GamePad::new(u16::from_be_bytes([c[0], c[1]]), c[2] as i8, c[3] as i8))
        .collect()
}

fn play(game: &mut SM64Game, pads: &[GamePad], frames: usize) {
    for p in &pads[..frames] {
        game.step_game(*p).unwrap();
    }
}

#[test]
fn a_loaded_state_goes_on_like_the_original() {
    let (generator, pads) = setup_or_skip!();
    // The file select menu, a level, another level much later, a long way in.
    for frame in [600, 1500, 12000, 30000] {
        let mut original = generator.create_game().unwrap();
        play(&mut original, &pads, frame);
        let state = original.export_portable().unwrap();
        let state = Portable::from_bytes(&state.to_bytes()).unwrap();

        // Into a game that is somewhere else entirely.
        let mut loaded = generator.create_game().unwrap();
        play(&mut loaded, &pads, 50);
        let report = loaded.load_portable(&state).unwrap();
        assert_eq!(report.untranslated, 0, "frame {frame}: {report:?}");

        for i in 0..300 {
            let pad = pads[frame + i];
            original.step_game(pad).unwrap();
            loaded.step_game(pad).unwrap();
            assert_eq!(
                original.get_game_state().unwrap(),
                loaded.get_game_state().unwrap(),
                "frame {frame}, {} frames after loading",
                i + 1
            );
            if i % 10 == 0 {
                let (a, b) = (
                    original.export_portable().unwrap().digest(),
                    loaded.export_portable().unwrap().digest(),
                );
                let differing: Vec<_> = (0..a.len())
                    .filter(|&v| a[v] != b[v])
                    .map(Portable::var_name)
                    // The render walk's leftovers differ with its cache's state.
                    .filter(|n| !n.contains("rendering_graph_node") && !n.contains("mario_misc"))
                    .collect();
                assert!(
                    differing.is_empty(),
                    "frame {frame}, {} frames after loading: {differing:?}",
                    i + 1
                );
            }
        }
    }
}

#[test]
fn bytes_round_trip_and_garbage_is_refused() {
    let (generator, pads) = setup_or_skip!();
    let mut game = generator.create_game().unwrap();
    play(&mut game, &pads, 700);
    let state = game.export_portable().unwrap();
    let bytes = state.to_bytes();
    assert_eq!(&bytes[..8], b"SM64PORT");
    let back = Portable::from_bytes(&bytes).unwrap();
    assert_eq!(back.to_bytes(), bytes);
    assert!(Portable::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    assert!(Portable::from_bytes(&bytes[..40]).is_err());
    let mut wrong = bytes.clone();
    wrong[0] = b'X';
    assert!(Portable::from_bytes(&wrong).is_err());
    let mut version = bytes.clone();
    version[8] = 9;
    assert!(Portable::from_bytes(&version).is_err());
}

#[test]
fn the_intro_has_no_portable_state() {
    let (generator, pads) = setup_or_skip!();
    let mut game = generator.create_game().unwrap();
    play(&mut game, &pads, 100);
    let err = game.export_portable().unwrap_err().to_string();
    assert!(err.contains("goddard"), "{err}");
}

#[test]
fn loading_twice_reuses_the_blocks_it_made() {
    let (generator, pads) = setup_or_skip!();
    let mut source = generator.create_game().unwrap();
    play(&mut source, &pads, 12000);
    let state = source.export_portable().unwrap();
    let mut game = generator.create_game().unwrap();
    play(&mut game, &pads, 80);
    game.load_portable(&state).unwrap();
    let size = game.with_memory(|m| m.len());
    // The second load fits in the blocks the first one used: memory does not grow.
    let report = game.load_portable(&state).unwrap();
    assert_eq!(report.reused, report.blocks, "{report:?}");
    assert_eq!(game.with_memory(|m| m.len()), size);
}

fn word(game: &SM64Game, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    game.read_memory(addr, &mut b).unwrap();
    u32::from_le_bytes(b)
}

fn bytes(game: &SM64Game, addr: u32, n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    game.read_memory(addr, &mut b).unwrap();
    b
}

/// A pointer into freed memory (an object's floor after the surface was freed) keeps
/// reading what the original game reads there: a ghost, a raw copy of the memory around
/// it, is allocated and never freed, and the pointer points into it.
#[test]
fn stale_pointers_keep_reading_what_the_original_reads() {
    let (generator, pads) = setup_or_skip!();
    // Frame 41000 has a stale `sObjFloor`.
    let frame = 41000;
    let mut original = generator.create_game().unwrap();
    play(&mut original, &pads, frame);
    let state = original.export_portable().unwrap();
    assert!(
        state.ghosts > 0 && !state.ghost_refs.is_empty(),
        "no stale pointer to test"
    );
    assert!(state.ghost_bytes() < 4096, "ghosts are windows, not whole chunks");
    let state = Portable::from_bytes(&state.to_bytes()).unwrap();
    assert!(state.ghosts > 0);
    let slot = (0..Portable::VAR_COUNT)
        .find(|&i| Portable::var_name(i).ends_with(":sObjFloor"))
        .map(|i| Portable::var_at(i, sm64_binds::portable::Build::Headless).0)
        .unwrap();

    // Into a fresh game and into one that has played.
    for pre in [0, 5000] {
        let mut original = generator.create_game().unwrap();
        play(&mut original, &pads, frame);
        let mut loaded = generator.create_game().unwrap();
        play(&mut loaded, &pads, pre);
        let report = loaded.load_portable(&state).unwrap();
        assert_eq!(report.untranslated, 0);
        assert_eq!(report.ghost_bytes, state.ghost_bytes());
        for i in 0..200 {
            let (po, pl) = (word(&original, slot), word(&loaded, slot));
            if po != 0 {
                assert_ne!(pl, 0);
                assert_eq!(
                    bytes(&original, po, 0x30),
                    bytes(&loaded, pl, 0x30),
                    "{i} frames after the load"
                );
            }
            let pad = pads[frame + i];
            original.step_game(pad).unwrap();
            loaded.step_game(pad).unwrap();
        }
    }
}

/// States of format version 1 (no ghosts) are still read.
#[test]
fn version_1_states_load() {
    let (generator, pads) = setup_or_skip!();
    let mut game = generator.create_game().unwrap();
    play(&mut game, &pads, 12000);
    let state = game.export_portable().unwrap();
    assert_eq!(state.ghosts, 0);
    let mut v1 = state.to_bytes();
    assert_eq!(&v1[8..12], &2u32.to_le_bytes());
    v1[8..12].copy_from_slice(&1u32.to_le_bytes());
    v1.drain(48..52);
    let old = Portable::from_bytes(&v1).unwrap();
    assert_eq!(old.ghosts, 0);
    assert_eq!(old.chunks.len(), state.chunks.len());
    let mut other = generator.create_game().unwrap();
    play(&mut other, &pads, 60);
    other.load_portable(&old).unwrap();
}
