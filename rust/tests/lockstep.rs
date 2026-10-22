//! Lockstep divergence tests for the headless build.
//!
//! Two instances of the **same** freshly built WASM are stepped with the same
//! input: a reference that renders and synthesises audio (as the old headless
//! build always did) and a candidate on the headless defaults (both off, and
//! the render walk then builds no display lists). After every frame, all of
//! linear memory must match except the stack and the data owned by the
//! renderer, the display lists and the audio engine, which are the only things
//! the defaults are allowed to change.
//!
//! They need the build products rather than the shipped payload, because the
//! link map is what says which bytes belong to the renderer and the audio
//! engine:
//!
//! ```text
//! make -C ../sm64-port TARGET_WEB=1 HEADLESS_VERSION=1
//! ```
//!
//! Without them, or without the ROM-free TAS files in `../tas`, the tests
//! skip. The quick test runs with `cargo test`; the full corpus is ignored by
//! default:
//!
//! ```text
//! cargo test --release --test lockstep -- --ignored --nocapture
//! ```

mod common;
use common::RawState;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sm64_binds::{GamePad, SM64Game, SM64GameGenerator};

const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

fn build_dir() -> PathBuf {
    std::env::var_os("SM64_HEADLESS_BUILD")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(MANIFEST).join("../sm64-port/build/us_web_headless"))
}

fn tas_path(name: &str) -> PathBuf {
    Path::new(MANIFEST).join("../tas").join(name)
}

/// Object files whose data the headless defaults may legitimately change.
/// `shadow.o` and `skybox.o` keep only what drawing the shadows and the sky
/// needs, which is not built when not drawing.
const EXCLUDED_OBJECTS: [&str; 6] = [
    "/src/pc/gfx/",
    "/src/audio/",
    "/src/pc/audio/",
    "/src/pc/mixer.o",
    "/src/game/shadow.o",
    "/src/game/skybox.o",
];

/// Individual symbols elsewhere that belong to the renderer or audio engine.
///
/// `gAudioRandom` lives in an excluded object anyway, but is listed because
/// game code does read it: all five reads (in `mario*.c`) only choose which
/// voice clip `play_sound` queues, so it never reaches game state. It stops
/// advancing when audio is off.
///
/// The rest change when the render walk builds no display lists:
/// - the display lists and their memory and cursors;
/// - the walk's matrix stacks: the float one is written before it is read
///   (only a shadow's slot is left out), the fixed point one only draws;
/// - `sFloorGeo`, which `find_floor_height_and_data` (also called by shadows)
///   fills before handing out its address, and which nothing reads later;
/// - the debug counters of floor checks, which only the debug display reads;
/// - the walk's cache of which subtrees only draw, filled when not drawing;
/// - `gCurrAnimAttribute`, which is left wherever the last object's animation
///   ended (a model that only draws is not walked); every read follows its
///   reset by `geo_set_animation_globals` or the held object's restore.
const EXCLUDED_SYMBOLS: [&str; 18] = [
    "gAudioHeap",
    "render_enabled",
    "audio_enabled",
    "gAudioRandom",
    "gGfxPools",
    "gDisplayListHeadInChunk",
    "gDisplayListEndInChunk",
    "sDrawArena",
    "sDrawArenaUsed",
    "sGfxCursor",
    "gMatStack",
    "gMatStackFixed",
    "sFloorGeo",
    "gNumCalls",
    "gNumFindFloorMisses",
    "sGeoCache",
    "sGeoCacheCount",
    "gCurrAnimAttribute",
];

/// The display lists' allocation-only pool (`gGfxAllocOnlyPool`) is on the
/// heap; its bookkeeping (last block, its size and how much of it is used)
/// depends on drawing.
const DRAW_POOL_BOOKKEEPING: usize = 12;

/// Audio-engine symbols whose values game code reads back, so they stay
/// compared even though their object files are excluded:
/// `get_current_background_music()` reads the queue to decide warp music.
const GAME_FACING_AUDIO: [&str; 2] = ["sBackgroundMusicQueue", "sBackgroundMusicQueueSize"];

/// One input section of the link map's data segment.
struct Section {
    start: usize,
    end: usize,
    name: String,
}

struct LinkMap {
    sections: Vec<Section>,
    rodata_start: usize,
    data_start: usize,
}

impl LinkMap {
    /// Parses `wasm-ld -Map` output. Input-section lines look like
    /// `  b06360  0  2  build/.../behavior_script.o:(.bss.gRandomSeed16)`.
    fn load(path: &Path) -> Option<LinkMap> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut sections = Vec::new();
        let (mut rodata_start, mut data_start) = (None, None);
        for line in text.lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() == 4 && cols[3] == ".rodata" {
                rodata_start = usize::from_str_radix(cols[0], 16).ok();
            }
            if cols.len() == 4 && cols[3] == ".data" {
                data_start = usize::from_str_radix(cols[0], 16).ok();
            }
            if cols.len() != 4 || !cols[3].contains(":(.") {
                continue;
            }
            let (Ok(start), Ok(size)) = (
                usize::from_str_radix(cols[0], 16),
                usize::from_str_radix(cols[2], 16),
            ) else {
                continue;
            };
            sections.push(Section {
                start,
                end: start + size,
                name: cols[3].to_string(),
            });
        }
        sections.sort_by_key(|s| s.start);
        Some(LinkMap {
            sections,
            rodata_start: rodata_start?,
            data_start: data_start?,
        })
    }

    /// What the comparison skips: sections that only drawing or audio use,
    /// and the stack (scratch, below `stack_end`).
    fn excluded(&self, stack_end: usize) -> Vec<(usize, usize)> {
        let mut ranges: Vec<(usize, usize)> = self
            .sections
            .iter()
            .filter(|s| {
                let is = |sym: &&str| s.name.ends_with(&format!(".{sym})"));
                (EXCLUDED_OBJECTS.iter().any(|o| s.name.contains(o))
                    || EXCLUDED_SYMBOLS.iter().any(is))
                    && !GAME_FACING_AUDIO.iter().any(is)
            })
            .map(|s| (s.start, s.end))
            .collect();
        ranges.push((0, stack_end));
        ranges.sort();
        ranges
    }

    /// The address of a data symbol.
    fn symbol(&self, name: &str) -> Option<usize> {
        let suffix = format!(".{name})");
        self.sections
            .iter()
            .find(|s| s.name.ends_with(&suffix))
            .map(|s| s.start)
    }

    fn describe(&self, addr: usize) -> String {
        match self.sections.iter().rev().find(|s| s.start <= addr) {
            Some(s) if addr < s.end => format!("{} +{:#x}", s.name, addr - s.start),
            Some(s) => format!("heap or padding after {} (+{:#x})", s.name, addr - s.end),
            None => "below the first section".to_string(),
        }
    }
}

/// The byte ranges compared each frame: everything outside the exclusions.
fn included_ranges(excluded: &[(usize, usize)], len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    for &(start, end) in excluded {
        if start > at {
            out.push((at, start.min(len)));
        }
        at = at.max(end);
    }
    if at < len {
        out.push((at, len));
    }
    out.retain(|(a, b)| b > a);
    out
}

/// Loads a Mupen64 `.m64` (header then 4 bytes per poll) or a raw pad file.
fn load_inputs(path: &Path) -> Option<Vec<GamePad>> {
    let bytes = std::fs::read(path).ok()?;
    let (body, count) = if bytes.starts_with(b"M64\x1a") {
        let samples = u32::from_le_bytes(bytes[0x18..0x1c].try_into().ok()?) as usize;
        (&bytes[0x400..], samples)
    } else {
        (&bytes[..], usize::MAX)
    };
    Some(
        body.as_chunks::<4>()
            .0
            .iter()
            .take(count)
            .map(|c| GamePad {
                button: u16::from_be_bytes([c[0], c[1]]),
                stick_x: c[2] as i8,
                stick_y: c[3] as i8,
            })
            .collect(),
    )
}

/// A borrowed view of a game's linear memory.
fn memory(game: &mut SM64Game) -> &[u8] {
    let (ptr, len) = game.with_memory(|memory| (memory.as_ptr(), memory.len()));
    // SAFETY: the memory stays where it is and the same size until the game is
    // next used, which the mutable borrow rules out while the slice lives.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

struct Harness {
    generator: SM64GameGenerator,
    map: LinkMap,
    excluded: Vec<(usize, usize)>,
    /// Where `gGfxAllocOnlyPool` is: the pool it points to is excluded too.
    draw_pool: Option<usize>,
}

impl Harness {
    fn load() -> Option<Harness> {
        let dir = build_dir();
        let wasm = std::fs::read(dir.join("sm64.us.wasm")).ok()?;
        let generator = SM64GameGenerator::from_wasm(&wasm).unwrap();
        let map = LinkMap::load(&dir.join("sm64.map"))?;
        let stack_end = generator.create_game().unwrap().read_only_memory().start;
        let excluded = map.excluded(stack_end);
        let draw_pool = map.symbol("gGfxAllocOnlyPool");
        Some(Harness {
            generator,
            map,
            excluded,
            draw_pool,
        })
    }

    fn reference(&self) -> SM64Game {
        let mut game = self.generator.create_game().unwrap();
        game.set_render_enabled(true).unwrap();
        game.set_audio_enabled(true).unwrap();
        game
    }

    fn candidate(&self) -> SM64Game {
        self.generator.create_game().unwrap()
    }

    /// Returns a description of the first difference, if any.
    fn compare(&self, reference: &mut SM64Game, candidate: &mut SM64Game) -> Option<String> {
        let a = reference.get_game_state().unwrap();
        let b = candidate.get_game_state().unwrap();
        if format!("{a:?}") != format!("{b:?}") {
            return Some(format!(
                "GameState differs:\n  reference {a:?}\n  candidate {b:?}"
            ));
        }
        let ma = memory(reference);
        let mb = memory(candidate);
        if ma.len() != mb.len() {
            return Some(format!("memory sizes differ: {} vs {}", ma.len(), mb.len()));
        }
        let mut excluded = self.excluded.clone();
        if let Some(at) = self.draw_pool {
            let pool = |m: &[u8]| u32::from_le_bytes(m[at..at + 4].try_into().unwrap()) as usize;
            if pool(ma) != pool(mb) {
                return Some(format!(
                    "gGfxAllocOnlyPool differs: {:#x} vs {:#x}",
                    pool(ma),
                    pool(mb)
                ));
            }
            excluded.push((pool(ma), pool(ma) + DRAW_POOL_BOOKKEEPING));
            excluded.sort();
        }
        for (start, end) in included_ranges(&excluded, ma.len()) {
            if ma[start..end] != mb[start..end] {
                let at = start
                    + ma[start..end]
                        .iter()
                        .zip(&mb[start..end])
                        .position(|(x, y)| x != y)
                        .unwrap();
                return Some(format!(
                    "memory differs at {at:#x} ({}): reference {:#04x}, candidate {:#04x}",
                    self.map.describe(at),
                    ma[at],
                    mb[at]
                ));
            }
        }
        None
    }

    /// Steps both games through `pads`, comparing after every frame.
    /// `read_only` is a snapshot of the read-only memory, which must never change.
    fn run(
        &self,
        label: &str,
        reference: &mut SM64Game,
        candidate: &mut SM64Game,
        pads: &[GamePad],
        read_only: &[u8],
        levels: &mut BTreeSet<i32>,
    ) {
        for (frame, pad) in pads.iter().enumerate() {
            reference.step_game(*pad).unwrap();
            candidate.step_game(*pad).unwrap();
            if let Some(diff) = self.compare(reference, candidate) {
                panic!("{label}: diverged after frame {frame}: {diff}");
            }
            if frame % 256 == 0 || frame + 1 == pads.len() {
                let range = candidate.read_only_memory();
                assert!(
                    memory(candidate)[range] == *read_only,
                    "{label}: frame {frame}: read-only memory changed, so save states would miss it"
                );
                levels.insert(candidate.get_game_state().unwrap().level_num);
            }
        }
    }
}

/// xorshift32, so fuzz inputs are reproducible without extra dependencies.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    /// A random pad: every button can appear, A/B/Z most often.
    fn pad(&mut self) -> GamePad {
        let mut button = 0u16;
        for (bit, per_mille) in [
            (0x8000, 60), // A
            (0x4000, 40), // B
            (0x2000, 40), // Z
            (0x1000, 4),  // Start
            (0x0020, 10), // L
            (0x0010, 20), // R
            (0x000f, 20), // C buttons
            (0x0f00, 5),  // D-pad
        ] {
            if self.next() % 1000 < per_mille {
                button |= bit & (self.next() as u16 | 0x8421);
            }
        }
        GamePad {
            button,
            stick_x: self.next() as i8,
            stick_y: self.next() as i8,
        }
    }
}

macro_rules! harness_or_skip {
    () => {
        match Harness::load() {
            Some(h) => h,
            None => {
                eprintln!(
                    "skipping: no headless build and link map in {}",
                    build_dir().display()
                );
                return;
            }
        }
    };
}

fn read_only_snapshot(game: &mut SM64Game) -> Vec<u8> {
    let range = game.read_only_memory();
    memory(game)[range].to_vec()
}

#[test]
fn read_only_memory_is_exactly_rodata() {
    let harness = harness_or_skip!();
    let game = harness.candidate();
    assert_eq!(
        game.read_only_memory(),
        harness.map.rodata_start..harness.map.data_start,
        "the read-only range read from the WASM is not .rodata in the link map"
    );
}

/// The quick check: the 0-star TAS, about 8.8k frames.
#[test]
fn headless_defaults_match_rendering_on_zero_star_tas() {
    let harness = harness_or_skip!();
    let Some(pads) = load_inputs(&tas_path("tasvideos-2016M-0star.m64")) else {
        eprintln!("skipping: TAS file missing");
        return;
    };
    let (mut reference, mut candidate) = (harness.reference(), harness.candidate());
    let read_only = read_only_snapshot(&mut candidate);
    let mut levels = BTreeSet::new();
    harness.run(
        "0-star TAS",
        &mut reference,
        &mut candidate,
        &pads,
        &read_only,
        &mut levels,
    );
}

/// The full corpus: every TAS, the AI runs if present, and random branches.
#[test]
#[ignore = "takes minutes; run with --release -- --ignored"]
fn headless_defaults_match_rendering_on_full_corpus() {
    let harness = harness_or_skip!();
    let mut files: Vec<PathBuf> = [
        "tasvideos-2016M-0star.m64",
        "tasvideos-6943M-16star.m64",
        "tasvideos-7310M-120star.m64",
        "tasvideos-2208M-120star.m64",
    ]
    .iter()
    .map(|n| tas_path(n))
    .collect();
    for dir in ["runs", "runs_interesting"] {
        let dir = Path::new(MANIFEST).join("../../sm64-AI-binds").join(dir);
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "m64"))
                .collect();
            found.sort();
            files.extend(found);
        }
    }

    let levels = std::sync::Mutex::new(BTreeSet::new());
    let frames = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for file in &files {
            let (harness, levels, frames) = (&harness, &levels, &frames);
            scope.spawn(move || {
                let label = file.file_name().unwrap().to_string_lossy().into_owned();
                let Some(pads) = load_inputs(file) else {
                    return;
                };
                let (mut reference, mut candidate) = (harness.reference(), harness.candidate());
                let read_only = read_only_snapshot(&mut candidate);
                let mut seen = BTreeSet::new();

                // Replay the file, branching into random inputs every 4000
                // frames. Each branch starts from copies of both games, so it
                // also checks that copies carry the render and audio settings.
                for (chunk_index, chunk) in pads.chunks(4000).enumerate() {
                    harness.run(
                        &label,
                        &mut reference,
                        &mut candidate,
                        chunk,
                        &read_only,
                        &mut seen,
                    );
                    let (mut ref_branch, mut cand_branch) =
                        (harness.reference(), harness.candidate());
                    ref_branch.set_render_enabled(false).unwrap();
                    ref_branch.set_audio_enabled(false).unwrap();
                    cand_branch.set_render_enabled(true).unwrap();
                    cand_branch.set_audio_enabled(true).unwrap();
                    ref_branch.copy_from(&reference).unwrap();
                    cand_branch.copy_from(&candidate).unwrap();
                    let mut rng =
                        Rng(0x9e37_79b9 ^ (chunk_index as u32 + 1).wrapping_mul(0x85eb_ca6b));
                    let random: Vec<GamePad> = (0..600).map(|_| rng.pad()).collect();
                    let branch_label = format!("{label} branch {chunk_index}");
                    harness.run(
                        &branch_label,
                        &mut ref_branch,
                        &mut cand_branch,
                        &random,
                        &read_only,
                        &mut seen,
                    );
                }
                frames.fetch_add(
                    pads.len() + pads.len().div_ceil(4000) * 600,
                    std::sync::atomic::Ordering::Relaxed,
                );
                eprintln!(
                    "{label}: ok ({} frames + branches), levels {seen:?}",
                    pads.len()
                );
                levels.lock().unwrap().extend(seen);
            });
        }
    });
    eprintln!(
        "all {} files matched over {} frames; levels visited: {:?}",
        files.len(),
        frames.into_inner(),
        levels.into_inner().unwrap()
    );
}

/// The harness must notice a one-byte change in game state and name it, and
/// the exclusions must stay a small, known part of memory.
#[test]
fn harness_detects_injected_divergence() {
    let harness = harness_or_skip!();
    let excluded: usize = harness.excluded.iter().map(|(a, b)| b - a).sum();
    eprintln!(
        "excluded from comparison: {excluded} bytes in {} ranges",
        harness.excluded.len()
    );
    assert!(excluded < 1 << 20, "exclusions grew to {excluded} bytes");
    for sym in GAME_FACING_AUDIO {
        let section = harness
            .map
            .sections
            .iter()
            .find(|s| s.name.ends_with(&format!(".{sym})")));
        let section = section.unwrap_or_else(|| panic!("{sym} missing from the link map"));
        assert!(
            !harness
                .excluded
                .iter()
                .any(|&(a, b)| a <= section.start && section.start < b),
            "{sym} is excluded"
        );
    }

    let seed = harness
        .map
        .sections
        .iter()
        .find(|s| s.name.ends_with("(.bss.gRandomSeed16)"))
        .expect("gRandomSeed16 in map");
    let (mut reference, mut candidate) = (harness.reference(), harness.candidate());
    for _ in 0..100 {
        reference.step_game(GamePad::default()).unwrap();
        candidate.step_game(GamePad::default()).unwrap();
    }
    assert_eq!(harness.compare(&mut reference, &mut candidate), None);
    // Flip a bit through a save state: the stack, then everything after the
    // read-only memory.
    let read_only = candidate.read_only_memory();
    let mut state = candidate.state_payload();
    state[read_only.start + seed.start - read_only.end] ^= 1;
    candidate.load_payload(&state).unwrap();
    let diff = harness
        .compare(&mut reference, &mut candidate)
        .expect("injected change not detected");
    assert!(diff.contains("gRandomSeed16"), "{diff}");
}

/// Every TAS run must play through to its end, as on the console.
#[test]
fn tas_runs_finish() {
    let harness = harness_or_skip!();
    for (file, stars, final_level) in [
        ("tasvideos-2016M-0star.m64", 0, 34),
        ("tasvideos-6943M-16star.m64", 16, 34),
        ("tasvideos-7310M-120star.m64", 120, 34),
        ("tasvideos-2208M-120star.m64", 120, 34),
    ] {
        let Some(pads) = load_inputs(&tas_path(file)) else {
            continue;
        };
        let mut game = harness.candidate();
        for pad in &pads {
            game.step_game(*pad).unwrap();
        }
        let state = game.get_game_state().unwrap();
        assert_eq!(
            (state.num_stars, state.level_num),
            (stars, final_level),
            "{file}: {state:?}"
        );
    }
}
