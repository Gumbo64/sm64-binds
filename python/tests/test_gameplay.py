"""End-to-end tests that boot the real WASM build.

These need a US .z64 ROM; the ``rom_path`` fixture skips the whole module when
it is missing, so the suite still passes on a checkout without it.

The assertions are invariants (determinism, save-state fidelity, isolation
between instances) rather than exact gameplay positions: a fresh ROM boots into
the attract-mode demo, and whether Mario responds to input depends on which
screen the demo is on.
"""

import pytest

from conftest import WARMUP_FRAMES, input_script
from sm64_binds import (
    GamePad,
    RngConfig,
    SM64GameGenerator,
    buttons,
    check_hash,
    rom_to_wasm_bytes,
)


def pad_from(button, stick_x, stick_y):
    return GamePad(button, stick_x, stick_y)


# ---------------------------------------------------------------
# ROM validation
# ---------------------------------------------------------------


def test_rejects_an_invalid_rom():
    with pytest.raises(ValueError, match="Invalid ROM"):
        SM64GameGenerator.new(b"definitely not a US z64 ROM")


def test_rejects_an_empty_rom():
    with pytest.raises(ValueError, match="Invalid ROM"):
        SM64GameGenerator.new(b"")


def test_accepts_the_us_rom(rom_path):
    with open(rom_path, "rb") as f:
        rom = f.read()
    assert check_hash(rom)
    assert SM64GameGenerator.new(rom) is not None


def test_decoded_payload_is_a_wasm_module(rom_path):
    with open(rom_path, "rb") as f:
        wasm = rom_to_wasm_bytes(f.read())
    assert wasm[:4] == b"\x00asm", "decoded payload is not a WASM module"
    assert len(wasm) > 1_000_000, f"implausibly small: {len(wasm)} bytes"


def test_decoding_is_deterministic(rom_path):
    with open(rom_path, "rb") as f:
        rom = f.read()
    assert rom_to_wasm_bytes(rom) == rom_to_wasm_bytes(rom)


# ---------------------------------------------------------------
# Booting
# ---------------------------------------------------------------


def test_a_fresh_game_reports_an_empty_state(game):
    state = game.get_game_state()
    assert state.numStars == 0
    assert not state.has_won()
    assert all(isinstance(f, float) for f in state.pos)


def test_the_state_advances_over_time(game):
    before = repr(game.get_game_state())
    for _ in range(WARMUP_FRAMES):
        game.step_game(GamePad(0, 0, 0))
    assert repr(game.get_game_state()) != before


def test_health_and_lives_are_populated_once_mario_exists(played_game):
    state = played_game.get_game_state()
    # The demo starts a save with 4 lives; health is a 0..0x1000 power meter.
    assert state.lives == 4
    assert 1 <= state.health <= 0x1000
    assert all(f == f for f in state.pos), "position must be finite"


def test_stepping_is_stable_over_a_long_run(game):
    for button, stick_x, stick_y in input_script(5000):
        game.step_game(pad_from(button, stick_x, stick_y))
        state = game.get_game_state()
        assert all(f == f for f in state.pos), "position went non-finite"


def test_many_games_share_one_compiled_module(generator):
    # create_game must not recompile the module, so ten instances stay cheap.
    for _ in range(10):
        game = generator.create_game()
        game.step_game(GamePad())
        game.get_game_state()


# ---------------------------------------------------------------
# Determinism
# ---------------------------------------------------------------


def test_identical_inputs_produce_identical_states(generator):
    a = generator.create_game()
    b = generator.create_game()
    for frame, (button, stick_x, stick_y) in enumerate(input_script(2500)):
        pad = pad_from(button, stick_x, stick_y)
        a.step_game(pad)
        b.step_game(pad)
        assert repr(a.get_game_state()) == repr(b.get_game_state()), (
            f"games diverged at frame {frame}"
        )


def test_two_generators_agree_frame_for_frame(rom_path):
    first = SM64GameGenerator.from_file(rom_path)
    second = SM64GameGenerator.from_file(rom_path)
    a = first.create_game()
    b = second.create_game()
    for button, stick_x, stick_y in input_script(1500):
        pad = pad_from(button, stick_x, stick_y)
        a.step_game(pad)
        b.step_game(pad)
    assert repr(a.get_game_state()) == repr(b.get_game_state())


# ---------------------------------------------------------------
# Copying state
# ---------------------------------------------------------------


def test_copy_to_reproduces_every_state_field(generator, played_game):
    clone = generator.create_game()
    generator.copy_to(played_game, clone)
    assert repr(played_game.get_game_state()) == repr(clone.get_game_state())


def test_copy_returns_an_equivalent_clone(generator, played_game):
    clone = generator.copy(played_game)
    assert clone is not played_game
    assert repr(played_game.get_game_state()) == repr(clone.get_game_state())
    clone.step_game(GamePad())


def test_a_copy_does_not_disturb_the_original(generator, played_game):
    clone = generator.copy(played_game)
    expected = repr(played_game.get_game_state())
    for button, stick_x, stick_y in input_script(2000):
        clone.step_game(pad_from(button, stick_x, stick_y))
    assert repr(played_game.get_game_state()) == expected


def test_copies_stay_in_lockstep_when_given_the_same_inputs(
    generator, played_game
):
    clone = generator.copy(played_game)
    for frame, (button, stick_x, stick_y) in enumerate(input_script(1500)):
        pad = pad_from(button, stick_x, stick_y)
        played_game.step_game(pad)
        clone.step_game(pad)
        assert (
            repr(played_game.get_game_state()) == repr(clone.get_game_state())
        ), f"copies diverged at frame {frame}"


def test_copy_is_valid_partway_through_a_run(generator):
    game = generator.create_game()
    clone = generator.create_game()
    script = input_script(2000)

    for button, stick_x, stick_y in script[:1000]:
        game.step_game(pad_from(button, stick_x, stick_y))
    generator.copy_to(game, clone)

    for frame, (button, stick_x, stick_y) in enumerate(script[1000:]):
        pad = pad_from(button, stick_x, stick_y)
        game.step_game(pad)
        clone.step_game(pad)
        assert (
            repr(game.get_game_state()) == repr(clone.get_game_state())
        ), f"a mid-run copy diverged at frame {frame}"


# ---------------------------------------------------------------
# RNG
# ---------------------------------------------------------------


def test_using_rng_tracks_only_what_the_caller_configured(generator):
    game = generator.create_game()
    # The flag is bookkeeping on the Python side; the WASM RNG is active from
    # boot because random_action.c seeds a default config statically.
    assert not game.using_rng()

    game.set_rng_config(RngConfig.default())
    assert game.using_rng()

    seeded = generator.create_game()
    assert not seeded.using_rng()
    seeded.set_rng_seed(22)
    assert seeded.using_rng()


def test_rng_pad_is_deterministic_for_a_seed(generator):
    a = generator.create_game()
    b = generator.create_game()
    a.set_rng_seed(22)
    b.set_rng_seed(22)

    pad = GamePad(buttons.A_BUTTON, 40, 40)
    for frame in range(2000):
        assert a.rng_pad(pad) == b.rng_pad(pad), (
            f"rng_pad diverged at frame {frame} for the same seed"
        )


def test_rng_pad_perturbs_inputs_by_default(game):
    # No set_rng_config call: the WASM default config still randomizes.
    pad = GamePad()
    assert any(game.rng_pad(pad) != pad for _ in range(2000))


def test_different_seeds_produce_different_pad_sequences(generator):
    a = generator.create_game()
    b = generator.create_game()
    a.set_rng_seed(1)
    b.set_rng_seed(2)

    pad = GamePad()
    assert any(a.rng_pad(pad) != b.rng_pad(pad) for _ in range(2000))


def test_zeroed_rng_config_leaves_the_pad_alone(generator):
    game = generator.create_game()
    game.set_rng_config(RngConfig(0, 0, 0, 0.0, 0.0, 0.0))
    pad = GamePad(buttons.Z_TRIG, 40, 40)
    for frame in range(500):
        assert game.rng_pad(pad) == pad, f"passthrough failed at frame {frame}"


def test_a_copy_shares_the_rng_sequence_of_its_source(generator):
    game = generator.create_game()
    game.set_rng_config(RngConfig.default())
    clone = generator.copy(game)

    pad = GamePad()
    for frame in range(500):
        assert game.rng_pad(pad) == clone.rng_pad(pad), (
            f"a copied game must share the RNG state (frame {frame})"
        )


def test_rng_pad_returns_values_inside_the_stick_range(generator):
    game = generator.create_game()
    game.set_rng_config(RngConfig.default())

    pad = GamePad()
    allowed = buttons.A_BUTTON | buttons.B_BUTTON | buttons.Z_TRIG
    for _ in range(2000):
        out = game.rng_pad(pad)
        assert -80 <= out.stick_x <= 80, f"stick_x out of range: {out.stick_x}"
        assert -80 <= out.stick_y <= 80, f"stick_y out of range: {out.stick_y}"
        assert out.button & ~allowed == 0, (
            f"rng_pad set an unexpected button: 0x{out.button:04x}"
        )


def test_a_clone_keeps_its_own_rng_sequence(generator):
    game = generator.create_game()
    game.set_rng_config(RngConfig.default())

    clone = generator.copy(game)
    for _ in range(1000):
        clone.rng_pad(GamePad())

    reference = generator.create_game()
    generator.copy_to(game, reference)

    pad = GamePad()
    for frame in range(200):
        assert game.rng_pad(pad) == reference.rng_pad(pad), (
            f"advancing a clone desynced the source RNG at frame {frame}"
        )