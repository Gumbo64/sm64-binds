use anyhow::{Error, Result};
use wasmer::*;

mod game_pad;
mod game_state;
mod rng_config;
mod util;

pub use game_pad::{GAME_PAD_SIZE, GamePad};
pub use game_state::{GAME_STATE_SIZE, GameState};
pub use rng_config::RngConfig;
use util::{check_hash, unzip_bytes, xor_bytes};

/// The distributable WASM payload, XOR'd against a US `.z64` ROM.
const XOR_BYTES: &[u8] = include_bytes!("../pkg/sm64_headless.us.wasm.zip.xor");

/// Compiles the WASM module once so that every [`SM64Game`] it creates is a
/// cheap instantiation of shared, immutable code.
#[derive(Debug, Clone)]
pub struct SM64GameGenerator {
    engine: Engine,
    module: Module,
}

impl SM64GameGenerator {
    /// Builds a generator from a US `.z64` ROM (8.00 MB, big endian).
    pub fn new(rom_bytes: Vec<u8>) -> Result<Self, Error> {
        if !check_hash(&rom_bytes) {
            return Err(Error::msg(
                "Invalid ROM. Should be a US copy, in z64 format (8.00MB)",
            ));
        };
        let wasm_bytes = Self::rom_to_wasm_bytes(rom_bytes);

        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)?;

        Ok(SM64GameGenerator { engine, module })
    }

    #[cfg(feature = "fs")]
    pub fn from_file(rom_path: &str) -> Result<Self, Error> {
        let rom_bytes = std::fs::read(rom_path)?;
        Self::new(rom_bytes)
    }

    pub fn create_game(&self) -> Result<SM64Game, Error> {
        SM64Game::from_module(&self.engine, &self.module)
    }

    /// Recombines the distributed XOR payload with the ROM and unzips it.
    pub fn rom_to_wasm_bytes(rom_bytes: Vec<u8>) -> Vec<u8> {
        unzip_bytes(&xor_bytes(XOR_BYTES, &rom_bytes))
    }

    /// Clones `game`, returning the new instance. `game` is left untouched.
    pub fn copy(&self, game: &mut SM64Game) -> Result<SM64Game, Error> {
        let mut cloned_game = self.create_game()?;

        game.copy_to(&mut cloned_game)?;

        Ok(cloned_game)
    }
}

/// A single playable instance of the game.
pub struct SM64Game {
    store: Store,
    using_rng: bool,
    // Cached exports so the hot paths don't look them up on every call
    memory: Memory,
    step_game_fn: TypedFunction<(u32, i32, i32), ()>,
    get_game_state_fn: TypedFunction<(), i32>,
    rng_pad_fn: TypedFunction<(u32, i32, i32), i32>,
    set_rng_config_fn: TypedFunction<(u32, u32, u32, f32, f32, f32), ()>,
    set_rng_seed_fn: TypedFunction<u32, ()>,
}

impl SM64Game {
    // From using the twiggy tool (twiggy top sm64.us.wasm). Seems to be constant.
    // I believe this segment only has constants so no need to save/load it.
    const DATA0_SIZE: usize = 8013002;

    /// Instantiates from a raw WASM module, bypassing ROM validation.
    pub fn new(wasm_bytes: impl AsRef<[u8]>) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm_bytes)?;
        Self::from_module(&engine, &module)
    }

    fn from_module(engine: &Engine, module: &Module) -> Result<Self> {
        let mut store = Store::new(engine.clone());
        let import_object = imports! {
            "wasi_snapshot_preview1" => {
                "proc_exit" => Function::new_typed(&mut store, |_exit_code: i32| {}),
                // The headless build must not try to talk to a terminal; the
                // return values keep libc's stdio happy.
                "fd_write" => Function::new_typed(&mut store, |_fd: i32, _iovs: i32, _iovs_len: i32, _bytes_written: i32| -> i32 { 0 }),
                "fd_close" => Function::new_typed(&mut store, |_fd: i32| -> i32 { 0 }),
                "fd_seek" => Function::new_typed(&mut store, |_fd: i32, _offset: i64, _whence: i32, _new_offset: i32| -> i32 { 0 }),
            },
        };

        let instance = Instance::new(&mut store, module, &import_object)?;
        let exports = &instance.exports;

        let memory = exports.get_memory("memory")?.clone();
        let step_game_fn = exports.get_typed_function(&store, "step_game")?;
        let get_game_state_fn = exports.get_typed_function(&store, "get_game_state")?;
        let rng_pad_fn = exports.get_typed_function(&store, "rng_pad")?;
        let set_rng_config_fn = exports.get_typed_function(&store, "set_rng_config")?;
        let set_rng_seed_fn = exports.get_typed_function(&store, "set_rng_seed")?;

        let main_func: TypedFunction<(), ()> = exports.get_typed_function(&store, "main_func")?;
        main_func.call(&mut store)?;

        Ok(SM64Game {
            store,
            using_rng: false,
            memory,
            step_game_fn,
            get_game_state_fn,
            rng_pad_fn,
            set_rng_config_fn,
            set_rng_seed_fn,
        })
    }

    pub fn set_rng_config(&mut self, cfg: RngConfig) -> Result<()> {
        self.using_rng = true;
        self.set_rng_config_fn.call(
            &mut self.store,
            cfg.window_length,
            cfg.random_amount,
            cfg.random_burst_length,
            cfg.a_prob,
            cfg.b_prob,
            cfg.z_prob,
        )?;
        Ok(())
    }

    pub fn set_rng_seed(&mut self, seed: u32) -> Result<()> {
        self.using_rng = true;
        self.set_rng_seed_fn.call(&mut self.store, seed)?;
        Ok(())
    }

    pub fn step_game(&mut self, pad: GamePad) -> Result<()> {
        self.step_game_fn.call(
            &mut self.store,
            pad.button.into(),
            pad.stick_x.into(),
            pad.stick_y.into(),
        )?;
        Ok(())
    }

    pub fn get_game_state(&mut self) -> Result<GameState, Error> {
        let pointer = self.get_game_state_fn.call(&mut self.store)?;

        let mut buffer: [u8; GAME_STATE_SIZE] = [0; GAME_STATE_SIZE];
        self.memory
            .view(&self.store)
            .read(pointer.try_into()?, &mut buffer)?;
        Ok(GameState::new(&buffer))
    }

    /// Reads the controller's current input, applying RNG if it is enabled.
    pub fn rng_pad(&mut self, pad: GamePad) -> Result<GamePad, Error> {
        let pointer = self.rng_pad_fn.call(
            &mut self.store,
            pad.button.into(),
            pad.stick_x.into(),
            pad.stick_y.into(),
        )?;

        let mut buffer: [u8; GAME_PAD_SIZE] = [0; GAME_PAD_SIZE];
        self.memory
            .view(&self.store)
            .read(pointer.try_into()?, &mut buffer)?;
        Ok(GamePad::from_bytes(&buffer))
    }

    pub fn using_rng(&self) -> bool {
        self.using_rng
    }

    fn expand_memory(&mut self, save_state_length: usize) -> Result<(), Error> {
        // allocate more memory if necessary for the save state
        let required_total_bytes = (SM64Game::DATA0_SIZE + save_state_length) as u64;
        self.memory
            .grow_at_least(&mut self.store, required_total_bytes)?;
        Ok(())
    }

    pub fn get_memory_bytes(&self) -> Result<Vec<u8>, Error> {
        let view = self.memory.view(&self.store);
        let buffer = view.copy_range_to_vec((SM64Game::DATA0_SIZE as u64)..view.data_size())?;
        Ok(buffer)
    }

    pub fn write_memory_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.expand_memory(bytes.len())?;

        Ok(self
            .memory
            .view(&self.store)
            .write(SM64Game::DATA0_SIZE as u64, bytes)?)
    }

    /// Copies the mutable region (everything after `DATA0_SIZE`) into `game_to`.
    pub fn copy_to(&self, game_to: &mut SM64Game) -> Result<(), Error> {
        let b = self.get_memory_bytes()?;
        game_to.write_memory_bytes(b.as_slice())?;
        Ok(())
    }

    /// Copies all of linear memory into `game_to`.
    pub fn full_copy_to(&self, game_to: &mut SM64Game) -> Result<(), Error> {
        let view_from = self.memory.view(&self.store);
        let view_to = game_to.memory.view(&game_to.store);

        Ok(view_from.copy_to_memory(view_from.data_size(), &view_to)?)
    }
}
