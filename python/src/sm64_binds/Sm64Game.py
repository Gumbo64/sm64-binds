import struct
import zipfile
import io
import os
import ctypes
import hashlib
from wasmtime import Engine, Store, Module, Linker, WasiConfig
# ---------------------------------------------------------
# GamePad and GameState Python versions
# ---------------------------------------------------------

class GamePad:
    def __init__(self, button=0, stick_x=0, stick_y=0):
        self.button = button
        self.stick_x = stick_x
        self.stick_y = stick_y

    @staticmethod
    def from_bytes(b):
        b = bytes(b)
        button = (b[0] << 8) | b[1]
        stick_x = struct.unpack("b", b[2:3])[0]
        stick_y = struct.unpack("b", b[3:4])[0]
        return GamePad(button, stick_x, stick_y)

# The 60 byte layout produced by the WASM get_game_state export
# (16 bit fields are padded out to 4 bytes by the WASM side)
_GAME_STATE_STRUCT = struct.Struct("<H2x3f3f3fH2xB3xH2xH2xH2x")

class GameState:
    def __init__(self, data):
        fields = _GAME_STATE_STRUCT.unpack(bytes(data))
        self.numStars = fields[0]
        self.pos = list(fields[1:4])
        self.vel = list(fields[4:7])
        self.lakituPos = list(fields[7:10])
        (self.lakituYaw, self.inCredits, self.courseNum,
         self.actNum, self.areaIndex) = fields[10:]

# ---------------------------------------------------------
# Utility: unzip and XOR like Rust
# ---------------------------------------------------------

def unzip_bytes(zipped_bytes):
    with zipfile.ZipFile(io.BytesIO(zipped_bytes), 'r') as z:
        return z.read(z.namelist()[0])

ROM_HASH = "9bef1128717f958171a4afac3ed78ee2bb4e86ce"
def check_hash(data: bytes) -> bool:
    return hashlib.sha1(data).hexdigest() == ROM_HASH


def xor_bytes(data: bytes, key: bytes) -> bytes:
    """XOR data with the key, repeating the key as needed (runs at C speed)."""
    key_len = len(key)
    repeated_key = (key * (len(data) // key_len + 1))[:len(data)]
    result = int.from_bytes(data, "little") ^ int.from_bytes(repeated_key, "little")
    return result.to_bytes(len(data), "little")

# ---------------------------------------------------------
# RngConfig class matching Rust
# ---------------------------------------------------------

class RngConfig:
    def __init__(self, window_length, random_amount, random_burst_length,
                 a_prob, b_prob, z_prob):
        self.window_length = window_length
        self.random_amount = random_amount
        self.random_burst_length = random_burst_length
        self.a_prob = a_prob
        self.b_prob = b_prob
        self.z_prob = z_prob


# ---------------------------------------------------------
# SM64Game (Python match of Rust version)
# ---------------------------------------------------------

class SM64Game:
    def __init__(self, wasm_bytes):
        engine = Engine()
        module = Module(engine, wasm_bytes)
        self._init(engine, module)

    @classmethod
    def from_module(cls, engine, module):
        """Instantiate from an already compiled module (much faster)."""
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

        # cache the exports so the hot paths don't look them up on every call
        exports = self.instance.exports(self.store)
        self.memory = exports["memory"]
        self._step_game = exports["step_game"]
        self._get_game_state = exports["get_game_state"]
        self._rng_pad = exports["rng_pad"]
        self._set_rng_config = exports["set_rng_config"]
        self._set_rng_seed = exports["set_rng_seed"]

        # call main_func like Rust does
        exports["main_func"](self.store)

    # -----------------------------------------------------
    # RNG functions
    # -----------------------------------------------------

    def set_rng_config(self, cfg: RngConfig):
        self.using_rng_flag = True
        self._set_rng_config(self.store,
            cfg.window_length,
            cfg.random_amount,
            cfg.random_burst_length,
            cfg.a_prob,
            cfg.b_prob,
            cfg.z_prob
        )

    def set_rng_seed(self, seed):
        self.using_rng_flag = True
        self._set_rng_seed(self.store, seed)

    # -----------------------------------------------------
    # Game stepping + state
    # -----------------------------------------------------

    def step_game(self, pad: GamePad):
        self._step_game(self.store, pad.button, pad.stick_x, pad.stick_y)

    def get_game_state(self):
        ptr = self._get_game_state(self.store)

        raw = self.memory.data_ptr(self.store)[ptr:ptr+60]
        return GameState(raw)

    def rng_pad(self, pad: GamePad):
        ptr = self._rng_pad(self.store, pad.button, pad.stick_x, pad.stick_y)

        raw = self.memory.data_ptr(self.store)[ptr:ptr+4]
        return GamePad.from_bytes(raw)

    def using_rng(self):
        return self.using_rng_flag

# ---------------------------------------------------------
# SM64GameGenerator (Python match of Rust version)
# ---------------------------------------------------------

class SM64GameGenerator:
    def __init__(self, wasm_bytes):
        self.wasm_bytes = wasm_bytes
        # The module is compiled once and instantiated for every game,
        # making create_game cheap
        self.engine = Engine()
        self.module = Module(self.engine, wasm_bytes)

    @staticmethod
    def new(rom_bytes):
        if not check_hash(rom_bytes):
            raise ValueError("Invalid ROM. Should be US z64 (8MB).")

        wasm_bytes = SM64GameGenerator.rom_to_wasm_bytes(rom_bytes)
        return SM64GameGenerator(wasm_bytes)

    @staticmethod
    def from_file(path):
        with open(path, "rb") as f:
            return SM64GameGenerator.new(f.read())

    @staticmethod
    def rom_to_wasm_bytes(rom_bytes):
        # load XOR bytes exactly like Rust include_bytes!
        base_dir = os.path.dirname(__file__)
        xor_path = os.path.join(base_dir, "pkg", "sm64_headless.us.wasm.zip.xor")

        with open(xor_path, "rb") as f:
            xor_data = f.read()

        return unzip_bytes(xor_bytes(xor_data, rom_bytes))

    def create_game(self):
        return SM64Game.from_module(self.engine, self.module)

    # -----------------------------
    # Copy a game state
    # -----------------------------

    def copy(self, game: SM64Game) -> SM64Game:
        """
        Creates an exact clone of the given SM64Game instance.
        """
        clone = self.create_game()
        self.copy_to(game, clone)
        return clone

    def copy_to(self, game: SM64Game, game_to: SM64Game):
        """
        If you already have a SM64Game instance to copy memory to, then this saves a LOT of instantiation time
        """
        # Source memory
        src_mem = game.memory
        src_store = game.store
        src_ptr = src_mem.data_ptr(src_store)
        size = src_mem.data_len(src_store)

        # Destination memory
        dst_mem = game_to.memory
        dst_store = game_to.store
        dst_ptr = dst_mem.data_ptr(dst_store)
        dst_size = dst_mem.data_len(dst_store)

        # Copy linear memory
        copy_size = size if size <= dst_size else dst_size
        ctypes.memmove(dst_ptr, src_ptr, copy_size)

        # Copy RNG flag
        game_to.using_rng_flag = game.using_rng_flag

        return game_to
