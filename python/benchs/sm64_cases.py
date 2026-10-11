"""SM64 cases for the generic benchmark harness (see ``benchs/harness.py``).

Every hot path of the Python binding, phrased as harness cases: setup costs,
stepping (warm game, amortised with instantiation), per-call overhead, and
the save-state copy paths. The cases mirror ``rust/benchmark/hot_paths.rs``
case for case, so a number in one binding can be set against the same number
in the other.
"""

from sm64_binds import GamePad, SM64GameGenerator, buttons

from benchs.harness import TARGET_SECONDS, Bencher, parse_args

#: One emulator session's worth of frames, shared by the amortised stepping
#: case and by the warm-up that puts the other cases in a playing state.
FRAMES = 2000

#: A pad with A held steady: an unchanging input keeps successive samples in
#: the same scene, which matters because frame cost varies with it.
HOLD_A = GamePad(0x8000, 0, 0)


def input_script(frames: int) -> list[GamePad]:
    """A deterministic but varied input script: buttons and stick values
    cycle with different moduli, so every frame presses something different
    while every run of the script replays exactly the same presses.
    Deliberately shared with the Rust ``hot_paths`` bench, so stepping
    numbers compare bindings, not input styles."""
    pads = []
    for i in range(frames):
        pad = GamePad()
        if i % 2 == 0:
            pad.button = buttons.A_BUTTON
        if i % 17 == 0:
            pad.button |= buttons.START_BUTTON
        if i % 7 == 0:
            pad.button |= buttons.Z_TRIG
        pad.stick_y = (80, -80, 0, 0)[i % 4]
        pad.stick_x = -80 if i % 5 == 0 else 0
        pads.append(pad)
    return pads


def run_bench(rom_path: str, *, target: float = TARGET_SECONDS, frames: int = FRAMES) -> None:
    b = Bencher(target=target)

    # ------------------------------------------------------------------
    b.section("setup (once per process)")
    holder: dict[str, SM64GameGenerator] = {}

    def load_generator() -> None:
        holder["gen"] = SM64GameGenerator.from_file(rom_path)

    # Module compilation happens once per process and costs more than
    # re-sampling could justify; both single-shot rows are magnitudes, not
    # comparisons.
    b.measure_once("SM64GameGenerator.from_file", load_generator)
    generator = holder["gen"]
    # Measured once, and kept: the derived per-frame row below needs this
    # number as its subtraction term.
    b.measure_once("create_game", lambda: generator.create_game())
    game = generator.create_game()

    # ------------------------------------------------------------------
    b.section("stepping")
    script = input_script(frames)

    # Frame cost depends on what the emulated game is doing, so the game is
    # stepped through its intro first — otherwise the first samples are
    # title-screen frames and the later ones are gameplay frames.
    for pad in script:
        game.step_game(pad)

    # One binding-level step call on an otherwise steady input: the pure
    # per-frame number, with instantiations long since amortised away.
    b.measure("step_game (in game)", lambda: game.step_game(HOLD_A), unit="frame")

    # A whole session: one instantiation amortised over the frames played.
    # ``units_per_block`` converts the block into a per-frame figure; the
    # derived row then peels the instantiation back out.
    def play_from_scratch() -> None:
        g = generator.create_game()
        for pad in script:
            g.step_game(pad)

    b.measure(
        f"create_game + {frames} frames", play_from_scratch, units_per_block=frames, unit="frame"
    )
    b.derived(
        "step_game per frame",
        f"create_game + {frames} frames",
        "create_game",
        scale=1.0 / frames,
        unit="frame",
    )
    # ------------------------------------------------------------------
    b.section("per-call overhead")
    # Both are WASM-call-bound: the engine call into the game plus reading a
    # fixed-size blob back out of the runtime's memory. The Python binding
    # reads the return pointer through ``data_ptr`` rather than an export's
    # write-back, so it is a touch slower than the Rust equivalent.
    b.measure("get_game_state", lambda: game.get_game_state())
    b.measure("rng_pad", lambda: game.rng_pad(HOLD_A))

    # ------------------------------------------------------------------
    b.section("save states")
    # The Python binding copies by memmoving between the runtime's memory
    # base addresses and has no serialise API (no equivalent of the Rust
    # ``get_memory_bytes``), so only the copy paths are measured here.
    b.note("(python memmoves directly; there is no serialise API)")
    clone = generator.create_game()
    # ``generator.copy`` is ``create_game`` + ``copy_to``, which is why it
    # dominates the warm-target copies.
    b.measure("generator.copy (fresh clone)", lambda: generator.copy(game))
    b.measure("copy_to (warm target)", lambda: generator.copy_to(game, clone))
    b.measure("full_copy_to (warm target)", lambda: generator.full_copy_to(game, clone))


def main(argv=None) -> None:
    args = parse_args(argv)
    run_bench(args.rom, target=args.target_secs, frames=args.frames)


if __name__ == "__main__":
    main()
