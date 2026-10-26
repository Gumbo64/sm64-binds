//! Mupen64 savestates: refusing what cannot be loaded, and (with real
//! states) loading them into the port and staying in sync with a movie.
//!
//! The real-state test needs `SM64_MUPEN_STATES` (a directory of `.st` files
//! made from the movie, Mupen64-rr or Mupen64Plus) and `SM64_MUPEN_M64` (the
//! `.m64` they were made from); without them it is skipped. States hold RDRAM
//! derived from the ROM, so none ship with the crate.

use sm64_binds::mupen::{MupenFormat, MupenState, port_variable};
use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};

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

fn u32_var(g: &SM64Game, name: &str, offset: u32) -> u32 {
    let (a, _) = port_variable(name).unwrap();
    let mut b = [0; 4];
    g.read_memory(a + offset, &mut b).unwrap();
    u32::from_le_bytes(b)
}

/// Mario's action, position and velocity, bit for bit.
fn mario(g: &SM64Game) -> Vec<u32> {
    [0x0C, 0x3C, 0x40, 0x44, 0x48, 0x4C, 0x50, 0x54]
        .iter()
        .map(|&o| u32_var(g, "gMarioStates", o))
        .collect()
}

#[test]
fn refuses_another_area_and_leaves_the_game_alone() {
    let generator = generator_or_skip!();
    let mut game = generator.create_game().unwrap();
    for _ in 0..10 {
        game.step_game(GamePad::default()).unwrap();
    }
    // A synthetic state in BoB (level 9, area 1).
    let mut rdram = vec![0u8; 0x80_0000];
    rdram[0x32DDF8..0x32DDFA].copy_from_slice(&9i16.to_be_bytes());
    rdram[0x33BACA..0x33BACC].copy_from_slice(&1i16.to_be_bytes());
    let st = MupenState::from_rdram(MupenFormat::Rr, &rdram);
    let st = MupenState::from_bytes(&st.to_gzip_bytes()).unwrap();
    assert_eq!((st.level(), st.area()), (Some(9), Some(1)));
    let before = game.save_state();
    let err = game.load_mupen_state(&st).unwrap_err().to_string();
    assert!(err.contains("level 9 area 1"), "{err}");
    assert_eq!(game.save_state(), before);
    // Warping needs a game in a level: this one is still at the title.
    assert!(game.warp_to_mupen_area(&st, 10).is_err());
}

#[test]
fn real_states_load_and_stay_in_sync() {
    let generator = generator_or_skip!();
    let (Ok(dir), Ok(movie)) = (
        std::env::var("SM64_MUPEN_STATES"),
        std::env::var("SM64_MUPEN_M64"),
    ) else {
        eprintln!("skipping: set SM64_MUPEN_STATES and SM64_MUPEN_M64");
        return;
    };
    let m64 = std::fs::read(movie).unwrap();
    let n = u32::from_le_bytes(m64[0x18..0x1C].try_into().unwrap()) as usize;
    let pads: Vec<GamePad> = m64[0x400..]
        .as_chunks::<4>()
        .0
        .iter()
        .take(n)
        .map(|c| GamePad {
            button: u16::from_be_bytes([c[0], c[1]]),
            stick_x: c[2] as i8,
            stick_y: c[3] as i8,
        })
        .collect();
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.retain(|p| p.extension().is_some_and(|e| e == "st"));
    paths.sort();
    let mut tested = 0;
    for path in paths {
        let st = MupenState::from_file(&path).unwrap();
        let timer = st.global_timer().unwrap();
        // The port's frame for the state: gGlobalTimer is one ahead.
        let frame = timer as usize - 1;
        if st.level().unwrap_or(0) <= 1 || frame + 300 > pads.len() {
            continue; // menus are not transplanted
        }
        let mut reference = generator.create_game().unwrap();
        for p in &pads[..frame] {
            reference.step_game(*p).unwrap();
        }
        assert_eq!(u32_var(&reference, "gGlobalTimer", 0), timer, "{path:?}");
        // From 100 frames before (warping if that is another area).
        let mut game = generator.create_game().unwrap();
        for p in &pads[..frame - 100] {
            game.step_game(*p).unwrap();
        }
        if game.warp_to_mupen_area(&st, 600).is_err() {
            continue;
        }
        let report = game.load_mupen_state(&st).unwrap();
        eprintln!("{path:?}: {report}");
        assert_eq!(
            mario(&game),
            mario(&reference),
            "{path:?}: Mario after loading"
        );
        for p in &pads[frame..frame + 300] {
            reference.step_game(*p).unwrap();
            game.step_game(*p).unwrap();
        }
        assert_eq!(
            mario(&game),
            mario(&reference),
            "{path:?}: Mario 300 frames after loading"
        );
        tested += 1;
    }
    assert!(tested > 0, "no in-level states to test");
}
