"""Mario 64 running in WebAssembly, driven through the WASM exports."""

from .Sm64Game import (
    GAME_STATE_SIZE,
    GamePad,
    GameState,
    RngConfig,
    SM64Game,
    SM64GameGenerator,
    check_hash,
    read_only_memory,
    rom_to_wasm_bytes,
    unzip_bytes,
    xor_bytes,
)
from . import buttons

__all__ = [
    "GAME_STATE_SIZE",
    "GamePad",
    "GameState",
    "RngConfig",
    "SM64Game",
    "SM64GameGenerator",
    "buttons",
    "check_hash",
    "read_only_memory",
    "rom_to_wasm_bytes",
    "unzip_bytes",
    "xor_bytes",
]