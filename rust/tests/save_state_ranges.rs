//! Checks the mutable-range constants that define what a save state captures.
//!
//! A save state copies the stack and everything from `DATA0_SIZE` onwards, and
//! skips the segment between them. That skip is only safe while the skipped
//! segment really is constant, so this test boots the real WASM, plays several
//! input styles, and fails if a single byte below `DATA0_SIZE` moves.
//!
//! Both constants were derived by measurement rather than assumption; this is
//! the test that keeps them honest if the WASM is ever rebuilt.
//!
//! The read-only segment is compared on *every* frame, so a byte written and
//! restored within a single frame is still caught.

use sm64_binds::buttons;
use sm64_binds::{DATA0_SIZE, STACK_END};
use sm64_binds::{GamePad, SM64GameGenerator};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../baserom.us.z64");

/// Distinct styles of play, so the check is not tuned to one input pattern.
const STYLES: [&str; 4] = ["neutral", "menu mash", "button sweep", "walking"];

/// Frames per style for the fast test.
///
/// Kept modest because every frame snapshots 15 MB of linear memory. Measured
/// dirty-byte counts converge within ~10 frames (10 frames surface 4303 of the
/// ~4600 bytes that ever move), so this is generous; `--ignored` runs 25x
/// further for a deep sweep.
const FRAMES_PER_STYLE: usize = 60;

/// A single frame of input for the given style.
fn input(style: &str, frame: usize) -> GamePad {
    match style {
        "neutral" => GamePad::default(),
        "menu mash" => {
            let button = match frame % 41 {
                0 => buttons::START_BUTTON,
                7 => buttons::A_BUTTON,
                13 => buttons::B_BUTTON,
                19 => buttons::Z_TRIG,
                25 => buttons::U_JPAD,
                31 => buttons::D_JPAD,
                37 => buttons::R_CBUTTONS,
                _ => 0,
            };
            GamePad::new(button, 0, 0)
        }
        "button sweep" => GamePad::new(
            [
                buttons::A_BUTTON,
                buttons::B_BUTTON,
                buttons::Z_TRIG,
                buttons::START_BUTTON,
                buttons::U_JPAD,
                buttons::D_JPAD,
                buttons::R_JPAD,
                buttons::L_JPAD,
            ][frame % 8],
            // Cast to i32 before scaling: `frame` is a usize, so the
            // subtraction below would underflow, and scaling an i8 would
            // overflow. Both panic in a debug build and wrap in release.
            ((((frame / 7) % 5) as i32) * 40 - 80) as i8,
            ((((frame / 3) % 3) as i32) * 80 - 80) as i8,
        ),
        _ => GamePad::new(
            if frame.is_multiple_of(2) {
                buttons::A_BUTTON
            } else {
                0
            },
            if frame.is_multiple_of(5) { -80 } else { 0 },
            if frame.is_multiple_of(3) { 80 } else { -80 },
        ),
    }
}

fn generator() -> Option<SM64GameGenerator> {
    match SM64GameGenerator::new(std::fs::read(ROM_PATH).unwrap_or_default()) {
        Ok(generator) => Some(generator),
        Err(_) => {
            eprintln!("skipping: no US .z64 ROM at {ROM_PATH}");
            None
        }
    }
}

/// Plays `frames` frames of `style`, returning the first address that moved
/// inside the skipped segment, if any.
///
/// Compares on every frame rather than sampling, so a byte written and
/// restored within one frame is still caught. The per-frame cost is dominated
/// by snapshotting linear memory, which is why `frames` stays modest.
fn first_escaped_byte(generator: &SM64GameGenerator, style: &str, frames: usize) -> Option<usize> {
    let mut game = generator.create_game().unwrap();
    let baseline = game.linear_memory();

    for frame in 0..frames {
        game.step_game(input(style, frame)).unwrap();
        let current = game.linear_memory();
        if let Some(offset) = current[STACK_END..DATA0_SIZE]
            .iter()
            .zip(&baseline[STACK_END..DATA0_SIZE])
            .position(|(a, b)| a != b)
        {
            return Some(STACK_END + offset);
        }
    }
    None
}

#[test]
fn the_read_only_segment_never_changes() {
    let Some(generator) = generator() else { return };

    for style in STYLES {
        if let Some(address) = first_escaped_byte(&generator, style, FRAMES_PER_STYLE) {
            panic!(
                "linear memory at 0x{address:x} is inside the read-only segment \
                 [0x{STACK_END:x}, 0x{DATA0_SIZE:x}) but changed while playing '{style}'. \
                 A save state skips that range, so the constants in sm64.rs are stale \
                 and need re-measuring."
            );
        }
    }
}

/// Guards the test above against passing for the wrong reason.
///
/// If `STACK_END` were moved to 0, or the game stopped using the stack, the
/// check would pass while covering nothing. So assert the ranges around the
/// skipped segment really do change during play.
#[test]
fn the_skipped_segment_is_surrounded_by_memory_that_does_change() {
    let Some(generator) = generator() else { return };

    let mut game = generator.create_game().unwrap();
    let boot = game.linear_memory();
    for frame in 0..200 {
        game.step_game(input("button sweep", frame)).unwrap();
    }
    let played = game.linear_memory();

    let stack_changed = boot[..STACK_END] != played[..STACK_END];
    let heap_changed = boot[DATA0_SIZE..] != played[DATA0_SIZE..];
    let gap_unchanged = boot[STACK_END..DATA0_SIZE] == played[STACK_END..DATA0_SIZE];

    assert!(
        stack_changed,
        "the stack never changed, so the read-only check covers nothing"
    );
    assert!(
        heap_changed,
        "the region above DATA0_SIZE never changed, so the game is not running"
    );
    assert!(
        gap_unchanged,
        "the segment between them changed, which the fast test also catches"
    );
}

/// A longer, wider sweep. Ignored by default because it takes several minutes;
/// run it after rebuilding the WASM, or when changing the mutable ranges:
///
/// ```text
/// cargo test --release --test save_state_ranges -- --ignored --nocapture
/// ```
#[test]
#[ignore = "slow sweep; run manually after rebuilding the WASM"]
fn deep_sweep() {
    let Some(generator) = generator() else { return };

    for style in STYLES {
        let started = std::time::Instant::now();
        let escaped = first_escaped_byte(&generator, style, 3000);
        println!(
            "  {style:<14} 3000 frames in {:.1}s -> {}",
            started.elapsed().as_secs_f64(),
            escaped.map_or("clean".to_string(), |a| format!("0x{a:x} MOVED"))
        );
        assert_eq!(escaped, None, "'{style}' wrote into the read-only segment");
    }
}
