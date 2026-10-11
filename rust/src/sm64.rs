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

/// Linear memory is not one mutable region but two, with a constant gap
/// between them:
///
/// ```text
///   0x000000 ┌────────────────────────┐
///            │ stack      (mutable)   │  <- emscripten stack, rewritten every frame
///   0x010000 ├────────────────────────┤
///            │ read-only data          │  <- constant, excluded from save states
///   0x7a44ca ├────────────────────────┤
///            │ game heap and BSS       │  <- mutable, most of the save state
///            └────────────────────────┘
/// ```
///
pub const STACK_END: usize = 0x10000;

/// Start of the game's heap and BSS; everything above this is mutable.
pub const DATA0_SIZE: usize = 8013002;

/// The mutable regions of linear memory, as half-open `[start, end)` offsets.
///
/// `end` is clamped to the current size of linear memory when copying.
pub(crate) const MUTABLE_RANGES: [(usize, usize); 2] = [(0, STACK_END), (DATA0_SIZE, usize::MAX)];

impl SM64Game {
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
        self.expand_memory_to((DATA0_SIZE + save_state_length) as u64)
    }


    fn expand_memory_to(&mut self, total_bytes: u64) -> Result<(), Error> {
        if self.memory.view(&self.store).data_size() >= total_bytes {
            return Ok(());
        }
        self.memory.grow_at_least(&mut self.store, total_bytes)?;

        // grow_at_least rounds up to a page, but a runtime may refuse growth
        // entirely; re-check so no caller writes out of bounds.
        let actual = self.memory.view(&self.store).data_size();
        if actual < total_bytes {
            return Err(Error::msg(format!(
                "could not grow linear memory to {total_bytes} bytes (got {actual})"
            )));
        }
        Ok(())
    }

    /// The mutable ranges of a memory of `memory_size` bytes, with `usize::MAX`
    /// ends clamped to that size.
    fn mutable_ranges(&self, memory_size: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        MUTABLE_RANGES
            .iter()
            .copied()
            .map(move |(start, end)| (start, end.min(memory_size)))
            .filter(move |&(start, end)| end > start)
    }

    /// A snapshot of all of linear memory, read-only segment included.
    pub fn linear_memory(&self) -> Vec<u8> {
        let view = self.memory.view(&self.store);
        let len = view.data_size() as usize;
        // SAFETY: `data_ptr` is valid for `data_size` bytes while `view` is
        // alive, and `len` is `data_size`, so the slice is in bounds.
        unsafe { std::slice::from_raw_parts(view.data_ptr(), len) }.to_vec()
    }

    /// The number of bytes [`SM64Game::get_memory_bytes`] would return: the
    /// `STACK_END` bytes of stack plus everything from `DATA0_SIZE` to the end
    /// of linear memory.
    pub fn save_state_size(&self) -> usize {
        let size = self.memory.view(&self.store).data_size() as usize;
        self.mutable_ranges(size).map(|(a, b)| b - a).sum()
    }

    /// Serialises the mutable regions of this game into a save state.
    ///
    /// The layout is the mutable ranges concatenated: the `STACK_END` bytes of
    /// stack first, then everything from `DATA0_SIZE` onwards.
    pub fn get_memory_bytes(&self) -> Result<Vec<u8>, Error> {
        let view = self.memory.view(&self.store);
        let size = view.data_size() as usize;
        let total: usize = self.mutable_ranges(size).map(|(a, b)| b - a).sum();

        let mut out = Vec::with_capacity(total);
        for (start, end) in self.mutable_ranges(size) {
            // SAFETY: `data_ptr` is valid for `size` bytes while `view` is
            // alive, and each range is clamped to `size`, so the slice is in
            // bounds. It is copied out before the view is dropped.
            out.extend_from_slice(unsafe {
                std::slice::from_raw_parts(view.data_ptr().add(start), end - start)
            });
        }
        Ok(out)
    }

    /// Restores a save state produced by [`SM64Game::get_memory_bytes`].
    ///
    /// The save state must have come from a game with at least as much linear
    /// memory as this one; see [`SM64Game::save_state_size`].
    pub fn write_memory_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // Size the destination for the largest state it might hold, then place
        // the ranges. A short slice leaves the trailing range untouched.
        let stack = bytes.len().min(STACK_END);
        let heap = bytes.len().saturating_sub(STACK_END);
        self.expand_memory(heap)?;

        let view = self.memory.view(&self.store);

        // SAFETY: `expand_memory` verified linear memory reaches
        // `DATA0_SIZE + heap`, and `stack <= STACK_END` is always in bounds.
        // `bytes` is a separate allocation, so neither copy can overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), view.data_ptr(), stack);
            if heap > 0 {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr().add(STACK_END),
                    view.data_ptr().add(DATA0_SIZE),
                    heap,
                );
            }
        }
        Ok(())
    }

    /// Copies the mutable regions of this game into `game_to`, without
    /// materialising a save state in between.
    pub fn copy_to(&self, game_to: &mut SM64Game) -> Result<(), Error> {
        let src = self.memory.view(&self.store);
        let src_size = src.data_size() as usize;

        // The destination has to be at least as large as the source's mutable
        // ranges. The stack needs no growth, only the heap does. Growing can
        // move the destination's base pointer, so its view is taken after.
        let needed = src_size.saturating_sub(DATA0_SIZE);
        game_to.expand_memory(needed)?;

        let dst = game_to.memory.view(&game_to.store);
        let dst_size = dst.data_size() as usize;
        let src_ptr = src.data_ptr();
        let dst_ptr = dst.data_ptr();

        for &(start, end) in &MUTABLE_RANGES {
            let src_end = end.min(src_size);
            let dst_end = end.min(dst_size);
            let len = src_end.min(dst_end).saturating_sub(start);
            if len == 0 {
                continue;
            }
            // SAFETY: `len` bytes are readable from `start` in the source and
            // `expand_memory` guaranteed the same range is writable in the
            // destination. The two memories are distinct allocations, so the
            // ranges cannot overlap.
            unsafe {
                std::ptr::copy_nonoverlapping(src_ptr.add(start), dst_ptr.add(start), len);
            }
        }
        Ok(())
    }

    // Copies all of linear memory into `game_to`, read-only data included.
    // Simpler but worse performance.
    // pub fn full_copy_to(&self, game_to: &mut SM64Game) -> Result<(), Error> {
    //     let src = self.memory.view(&self.store);
    //     let len = src.data_size() as usize;

    //     game_to.expand_memory_to(len as u64)?;

    //     let dst = game_to.memory.view(&game_to.store);
    //     // SAFETY: as in `copy_to`, except that the whole address space is
    //     // copied rather than just the mutable region.
    //     unsafe {
    //         std::ptr::copy_nonoverlapping(src.data_ptr(), dst.data_ptr(), len);
    //     }
    //     Ok(())
    // }
}
