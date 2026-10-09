"""Mario 64 running in WebAssembly, driven through the WASM exports.

Layout note: every field of the WASM ``GameState`` struct is an ``s32``, so the
struct is 13 * 4 = 76 bytes with no padding. See
``sm64-port/src/pc/pc_main.c``.
"""

import ctypes
import hashlib
import io
import os
import struct
import zipfile

from wasmtime import Engine, Linker, Module, Store, WasiConfig

__all__ = [
    "GamePad",
    "GameState",
    "RngConfig",
    "SM64Game",
    "SM64GameGenerator",
    "check_hash",
    "rom_to_wasm_bytes",
    "unzip_bytes",
    "xor_bytes",
]

#: SHA-1 of a US ``.z64`` ROM, the only region this package supports.
ROM_HASH = "9bef1128717f958171a4afac3ed78ee2bb4e86ce"

#: Byte layout of the ``rng_pad`` export, mirroring ``OSContPad`` in
#: ``sm64-port/include/PR/os_cont.h``.
_GAME_PAD_STRUCT = struct.Struct("<Hb b")

#: Byte layout of the ``get_game_state`` export: 10 s32 scalars plus three
#: 3-float vectors, which is 13 * 4 = 76 bytes.
GAME_STATE_SIZE = 76
_GAME_STATE_STRUCT = struct.Struct("<i3f3f3f9i")


class GamePad:
    """A single frame of controller input."""

    __slots__ = ("button", "stick_x", "stick_y")

    def __init__(self, button=0, stick_x=0, stick_y=0):
        self.button = button
        self.stick_x = stick_x
        self.stick_y = stick_y

    @staticmethod
    def from_bytes(b):
        button, stick_x, stick_y = _GAME_PAD_STRUCT.unpack(bytes(b)[:4])
        return GamePad(button, stick_x, stick_y)

    def __eq__(self, other):
        if not isinstance(other, GamePad):
            return NotImplemented
        return (
            self.button == other.button
            and self.stick_x == other.stick_x
            and self.stick_y == other.stick_y
        )

    def __hash__(self):
        return hash((self.button, self.stick_x, self.stick_y))

    def __repr__(self):
        return (
            f"GamePad(button=0x{self.button:04x}, "
            f"stick_x={self.stick_x}, stick_y={self.stick_y})"
        )


class GameState:
    """A snapshot of the game, mirroring the WASM ``GameState`` struct."""

    __slots__ = (
        "numStars",
        "pos",
        "vel",
        "lakituPos",
        "lakituYaw",
        "inCredits",
        "courseNum",
        "actNum",
        "areaIndex",
        "levelNum",
        "lives",
        "coins",
        "health",
    )

    def __init__(self, data):
        values = _GAME_STATE_STRUCT.unpack(bytes(data)[:GAME_STATE_SIZE])

        self.numStars = values[0]
        self.pos = list(values[1:4])
        self.vel = list(values[4:7])
        self.lakituPos = list(values[7:10])
        (
            self.lakituYaw,
            self.inCredits,
            self.courseNum,
            self.actNum,
            self.areaIndex,
            self.levelNum,
            self.lives,
            self.coins,
            self.health,
        ) = values[10:]

    def has_won(self):
        return self.numStars > 0

    def __eq__(self, other):
        if not isinstance(other, GameState):
            return NotImplemented
        return all(
            getattr(self, name) == getattr(other, name) for name in self.__slots__
        )

    def __repr__(self):
        return (
            f"GameState(numStars={self.numStars}, pos={self.pos}, "
            f"levelNum={self.levelNum}, lives={self.lives}, "
            f"coins={self.coins}, health={self.health})"
        )


class RngConfig:
    """Tuning for the RNG that perturbs controller inputs."""

    __slots__ = (
        "window_length",
        "random_amount",
        "random_burst_length",
        "a_prob",
        "b_prob",
        "z_prob",
    )

    def __init__(
        self,
        window_length,
        random_amount,
        random_burst_length,
        a_prob,
        b_prob,
        z_prob,
    ):
        self.window_length = window_length
        self.random_amount = random_amount
        self.random_burst_length = random_burst_length
        self.a_prob = a_prob
        self.b_prob = b_prob
        self.z_prob = z_prob

    @staticmethod
    def default():
        """The configuration the WASM build starts with (random_action.c)."""
        return RngConfig(100, 5, 2, 0.5, 0.5, 0.2)

    def __repr__(self):
        fields = ", ".join(f"{name}={getattr(self, name)}" for name in self.__slots__)
        return f"RngConfig({fields})"


def check_hash(data: bytes) -> bool:
    return hashlib.sha1(data).hexdigest() == ROM_HASH


def unzip_bytes(zipped_bytes: bytes) -> bytes:
    """Extracts the single entry of a ZIP archive holding the WASM module."""
    with zipfile.ZipFile(io.BytesIO(zipped_bytes), "r") as z:
        return z.read(z.namelist()[0])


def xor_bytes(data: bytes, key: bytes) -> bytes:
    """XORs ``data`` with ``key``, repeating the key as needed.

    Runs at C speed via big integer arithmetic instead of a Python loop.
    """
    key_len = len(key)
    if key_len == 0:
        raise ValueError("XOR key must not be empty")
    repeated_key = (key * (len(data) // key_len + 1))[: len(data)]
    result = int.from_bytes(data, "little") ^ int.from_bytes(repeated_key, "little")
    return result.to_bytes(len(data), "little")


_XOR_PATH = os.path.join(os.path.dirname(__file__), "pkg", "sm64_headless.us.wasm.zip.xor")


def rom_to_wasm_bytes(rom_bytes: bytes) -> bytes:
    """Recombines the distributed XOR payload with the ROM and unzips it."""
    with open(_XOR_PATH, "rb") as f:
        xor_data = f.read()
    return unzip_bytes(xor_bytes(xor_data, rom_bytes))


class SM64Game:
    """A single playable instance of the game."""

    def __init__(self, wasm_bytes):
        engine = Engine()
        module = Module(engine, wasm_bytes)
        self._init(engine, module)

    @classmethod
    def from_module(cls, engine, module):
        """Instantiates from an already compiled module (much faster)."""
        game = cls.__new__(cls)
        game._init(engine, module)
        return game

    def _init(self, engine, module):
        self.engine = engine
        self.module = module

        self.store = Store(engine)
        self.linker = Linker(engine)
        self.linker.define_wasi()

        wasi = WasiConfig()
        wasi.inherit_stdout()
        self.store.set_wasi(wasi)

        self.instance = self.linker.instantiate(self.store, module)

        self.using_rng_flag = False

        # Cache the exports so the hot paths don't look them up on every call.
        exports = self.instance.exports(self.store)
        self.memory = exports["memory"]
        self._step_game = exports["step_game"]
        self._get_game_state = exports["get_game_state"]
        self._rng_pad = exports["rng_pad"]
        self._set_rng_config = exports["set_rng_config"]
        self._set_rng_seed = exports["set_rng_seed"]

        exports["main_func"](self.store)

    # -----------------------------
    # RNG
    # -----------------------------

    def set_rng_config(self, cfg: RngConfig):
        self.using_rng_flag = True
        self._set_rng_config(
            self.store,
            cfg.window_length,
            cfg.random_amount,
            cfg.random_burst_length,
            cfg.a_prob,
            cfg.b_prob,
            cfg.z_prob,
        )

    def set_rng_seed(self, seed):
        self.using_rng_flag = True
        self._set_rng_seed(self.store, seed)

    # -----------------------------
    # Game stepping and state
    # -----------------------------

    def step_game(self, pad: GamePad):
        self._step_game(self.store, pad.button, pad.stick_x, pad.stick_y)

    def get_game_state(self) -> GameState:
        ptr = self._get_game_state(self.store)
        memory = self.memory.data_ptr(self.store)
        return GameState(memory[ptr : ptr + GAME_STATE_SIZE])

    def rng_pad(self, pad: GamePad) -> GamePad:
        ptr = self._rng_pad(self.store, pad.button, pad.stick_x, pad.stick_y)
        memory = self.memory.data_ptr(self.store)
        return GamePad.from_bytes(memory[ptr : ptr + 4])

    def using_rng(self):
        return self.using_rng_flag


class SM64GameGenerator:
    """Compiles the WASM module once so ``create_game`` stays cheap."""

    def __init__(self, wasm_bytes):
        self.wasm_bytes = wasm_bytes
        self.engine = Engine()
        self.module = Module(self.engine, wasm_bytes)

    @staticmethod
    def new(rom_bytes):
        if not check_hash(rom_bytes):
            raise ValueError("Invalid ROM. Should be US z64 (8MB).")
        return SM64GameGenerator(rom_to_wasm_bytes(rom_bytes))

    @staticmethod
    def from_file(path):
        with open(path, "rb") as f:
            return SM64GameGenerator.new(f.read())

    def create_game(self) -> SM64Game:
        return SM64Game.from_module(self.engine, self.module)

    # -----------------------------
    # Copying a game state
    # -----------------------------

    def copy(self, game: SM64Game) -> SM64Game:
        """Returns an exact clone of ``game``, leaving it untouched."""
        clone = self.create_game()
        self.copy_to(game, clone)
        return clone

    def copy_to(self, game: SM64Game, game_to: SM64Game):
        """Copies ``game``'s state into an existing ``game_to``.

        Reusing an instance saves a full instantiation.
        """
        src_ptr = game.memory.data_ptr(game.store)
        size = game.memory.data_len(game.store)

        dst_ptr = game_to.memory.data_ptr(game_to.store)
        dst_size = game_to.memory.data_len(game_to.store)

        ctypes.memmove(dst_ptr, src_ptr, min(size, dst_size))
        game_to.using_rng_flag = game.using_rng_flag
        return game_to