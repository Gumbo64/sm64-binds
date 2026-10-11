"""Mario 64 running in WebAssembly, driven through the WASM exports."""

from .Sm64Game import (
    DATA0_SIZE,
    GAME_STATE_SIZE,
    MUTABLE_RANGES,
    STACK_END,
    GamePad,
    GameState,
    RngConfig,
    SM64Game,
    SM64GameGenerator,
    check_hash,
    rom_to_wasm_bytes,
    unzip_bytes,
    xor_bytes,
)
from . import buttons

__all__ = [
    "DATA0_SIZE",
    "GAME_STATE_SIZE",
    "MUTABLE_RANGES",
    "STACK_END",
    "GamePad",
    "GameState",
    "RngConfig",
    "SM64Game",
    "SM64GameGenerator",
    "buttons",
    "check_hash",
    "rom_to_wasm_bytes",
    "unzip_bytes",
    "xor_bytes",
]