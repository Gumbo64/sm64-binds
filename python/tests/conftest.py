"""Shared fixtures for the sm64_binds test suite."""

import os

import pytest

# A US .z64 ROM, required to boot the WASM module. The repo root is three
# levels up from this file; override with SM64_BINDS_ROM if it lives elsewhere.
_REPO_ROOT = os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
)
ROM_PATH = os.environ.get("SM64_BINDS_ROM", os.path.join(_REPO_ROOT, "baserom.us.z64"))

#: Long enough for the attract demo to leave the opening screens.
WARMUP_FRAMES = 1500


def input_script(frames):
    """A mixed input script exercising every pad field.

    The ROM boots into an attract-mode demo, so this exists to cover the pad
    fields rather than to reach a specific screen.
    """
    script = []
    for i in range(frames):
        button = 0
        if i % 2 == 0:
            button |= 0x8000  # A
        if i % 17 == 0:
            button |= 0x1000  # START
        if i % 7 == 0:
            button |= 0x2000  # Z
        stick_y = 80 if i % 4 == 0 else (-80 if i % 4 == 1 else 0)
        stick_x = -80 if i % 5 == 0 else 0
        script.append((button, stick_x, stick_y))
    return script


def pytest_report_header(config):
    if os.path.exists(ROM_PATH):
        return f"ROM: {ROM_PATH} (integration tests will run)"
    return f"ROM: not found at {ROM_PATH} (integration tests will skip)"


@pytest.fixture(scope="session")
def rom_path():
    if not os.path.exists(ROM_PATH):
        pytest.skip(f"no US .z64 ROM at {ROM_PATH}")
    return ROM_PATH


@pytest.fixture(scope="session")
def generator(rom_path):
    from sm64_binds import SM64GameGenerator

    return SM64GameGenerator.from_file(rom_path)


@pytest.fixture
def game(generator):
    """A freshly booted game."""
    return generator.create_game()


@pytest.fixture
def played_game(generator):
    """A game that has advanced far enough for its state to be meaningful."""
    from sm64_binds import GamePad

    instance = generator.create_game()
    for _ in range(WARMUP_FRAMES):
        instance.step_game(GamePad(0, 0, 0))
    return instance