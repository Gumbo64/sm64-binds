use anyhow::{Error, Result};
use wasmer::*;

mod dirty;
mod game_pad;
mod game_state;
mod generator;
mod memory;
#[cfg(all(
    feature = "precompiled",
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "gnu"
))]
mod precompiled;
mod rng_config;
mod save_state;
mod sync;
mod util;
mod wasm_module;

pub use game_pad::{GAME_PAD_SIZE, GamePad};
pub use game_state::{GAME_STATE_SIZE, GameState};
pub use generator::{Backend, SM64GameGenerator};
pub use rng_config::RngConfig;
pub use save_state::SaveState;

use generator::GameSource;
use memory::MemoryLayout;
use sync::SyncState;
use wasm_module::BuildId;

/// How [`SM64Game::set_object_freeze`] freezes objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeMode {
    /// Exact play (the default).
    Off,
    /// Every object except Mario and doors, using the game's own time stop.
    All,
}

/// A single playable instance of the game, made by
/// [`SM64GameGenerator::create_game`] (`sm64/generator.rs`).
///
/// Its memory is handled in `sm64/memory.rs` (reads, copies),
/// `sm64/save_state.rs` ([`SaveState`]) and `sm64/sync.rs`
/// ([`SM64Game::sync_from`]).
pub struct SM64Game {
    store: Store,
    exports: Exports,
    precompiled: bool,
    /// Which build of the game this is, for [`SaveState`]s.
    build_id: BuildId,
    /// Which parts of memory are constant.
    layout: MemoryLayout,
    using_rng: bool,
    sync: SyncState,
}

/// The instance and its exports, cached so the hot paths don't look them up on
/// every call.
struct Exports {
    /// Kept so optional exports can be looked up by name without being
    /// required at load time.
    instance: Instance,
    memory: Memory,
    step_game: TypedFunction<(u32, i32, i32), ()>,
    get_game_state: TypedFunction<(), i32>,
    rng_pad: TypedFunction<(u32, i32, i32), i32>,
    set_rng_config: TypedFunction<(u32, u32, u32, f32, f32, f32), ()>,
    set_rng_seed: TypedFunction<u32, ()>,
}

impl SM64Game {
    /// A new game from `source`: [`SM64GameGenerator::create_game`].
    fn from_source(source: &GameSource) -> Result<Self> {
        let mut store = Store::new(source.engine.clone());
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

        let instance = Instance::new(&mut store, &source.module, &import_object)?;
        let exports = &instance.exports;

        let memory = exports.get_memory("memory")?.clone();
        if let Some(segments) = &source.segments {
            // What instantiating the full WASM would have placed.
            let view = memory.view(&store);
            for (offset, bytes) in segments.iter() {
                view.write(*offset as u64, bytes)?;
            }
        }
        let step_game = exports.get_typed_function(&store, "step_game")?;
        let get_game_state = exports.get_typed_function(&store, "get_game_state")?;
        let rng_pad = exports.get_typed_function(&store, "rng_pad")?;
        let set_rng_config = exports.get_typed_function(&store, "set_rng_config")?;
        let set_rng_seed = exports.get_typed_function(&store, "set_rng_seed")?;

        let main_func: TypedFunction<(), ()> = exports.get_typed_function(&store, "main_func")?;
        main_func.call(&mut store)?;

        Ok(SM64Game {
            store,
            exports: Exports {
                instance,
                memory,
                step_game,
                get_game_state,
                rng_pad,
                set_rng_config,
                set_rng_seed,
            },
            precompiled: source.segments.is_some(),
            build_id: source.build_id,
            layout: source.layout,
            using_rng: false,
            sync: SyncState::new(),
        })
    }

    /// Which way this game's code was compiled: `"precompiled"` (ahead of
    /// time, shipped in the crate) or `"cranelift"` (when it was loaded).
    pub fn backend(&self) -> &'static str {
        if self.precompiled {
            "precompiled"
        } else {
            "cranelift"
        }
    }

    /// The store and exports, for running the game's code or growing its
    /// memory. Counts as a change to the game's memory, for
    /// [`SM64Game::sync_from`]: every call into the game goes through here.
    fn parts_mut(&mut self) -> (&mut Store, &Exports) {
        self.sync.touch();
        (&mut self.store, &self.exports)
    }

    pub fn step_game(&mut self, pad: GamePad) -> Result<()> {
        let (store, exports) = self.parts_mut();
        exports.step_game.call(
            store,
            pad.button.into(),
            pad.stick_x.into(),
            pad.stick_y.into(),
        )?;
        Ok(())
    }

    pub fn get_game_state(&mut self) -> Result<GameState, Error> {
        let (store, exports) = self.parts_mut();
        let pointer = exports.get_game_state.call(store)? as u32;

        let mut buffer: [u8; GAME_STATE_SIZE] = [0; GAME_STATE_SIZE];
        self.read_memory(pointer, &mut buffer)?;
        Ok(GameState::new(&buffer))
    }

    /// Reads the controller's current input, applying RNG if it is enabled.
    pub fn rng_pad(&mut self, pad: GamePad) -> Result<GamePad, Error> {
        let (store, exports) = self.parts_mut();
        let pointer = exports.rng_pad.call(
            store,
            pad.button.into(),
            pad.stick_x.into(),
            pad.stick_y.into(),
        )? as u32;

        let mut buffer: [u8; GAME_PAD_SIZE] = [0; GAME_PAD_SIZE];
        self.read_memory(pointer, &mut buffer)?;
        Ok(GamePad::from_bytes(&buffer))
    }

    pub fn set_rng_config(&mut self, cfg: RngConfig) -> Result<()> {
        self.using_rng = true;
        let (store, exports) = self.parts_mut();
        exports.set_rng_config.call(
            store,
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
        let (store, exports) = self.parts_mut();
        exports.set_rng_seed.call(store, seed)?;
        Ok(())
    }

    pub fn using_rng(&self) -> bool {
        self.using_rng
    }

    /// Turns the display-list interpreter on or off.
    ///
    /// Headless builds start with it off: the game never reads its output, so
    /// it only costs time. Turning it on is for checking exactly that. The
    /// setting lives in linear memory, so copies and save states carry it.
    /// Fails on WASM built before the export existed.
    pub fn set_render_enabled(&mut self, enabled: bool) -> Result<()> {
        self.call_i32("set_render_enabled", enabled.into())
    }

    /// Turns audio synthesis on or off. Headless builds start with it off.
    ///
    /// Like [`SM64Game::set_render_enabled`], the setting is part of the
    /// game's memory and so travels with copies and save states.
    pub fn set_audio_enabled(&mut self, enabled: bool) -> Result<()> {
        self.call_i32("set_audio_enabled", enabled.into())
    }

    /// Freezes objects so frames are cheaper, at the cost of exactness.
    ///
    /// A frozen game no longer plays like the real one: frozen enemies can't
    /// be defeated, `!` boxes and red coins don't spawn stars, and so on. Use
    /// it to screen candidates, then replay the ones that matter on an exact
    /// game. Switching back to [`FreezeMode::Off`] does not make a game exact
    /// again. The mode is stored in linear memory, so copies inherit it.
    /// Needs a headless build with the `set_object_freeze` export.
    pub fn set_object_freeze(&mut self, mode: FreezeMode) -> Result<()> {
        let code = match mode {
            FreezeMode::Off => 0,
            FreezeMode::All => 1,
        };
        self.call_i32("set_object_freeze", code)
    }

    /// False once objects have been frozen in this game, or in the game (or
    /// save state) it was copied from. The marker lives in linear memory, so
    /// it travels with the state. Builds without freezing are always exact.
    pub fn is_exact(&mut self) -> Result<bool> {
        if !self.has_export("objects_ever_frozen") {
            return Ok(true);
        }
        Ok(self.call_export_u32("objects_ever_frozen")? == 0)
    }

    /// Calls an export that takes an i32 and returns nothing.
    fn call_i32(&mut self, name: &str, value: i32) -> Result<()> {
        let (store, exports) = self.parts_mut();
        let f: TypedFunction<i32, ()> = exports.instance.exports.get_typed_function(store, name)?;
        f.call(store, value)?;
        Ok(())
    }

    /// Calls a zero-argument export that returns an `i32` (typically a
    /// pointer into linear memory) and returns it as a `u32`.
    ///
    /// Looked up by name on every call, so this is for occasional use (such as
    /// `get_mario_state`), not hot paths. Fails if the WASM lacks the export.
    pub fn call_export_u32(&mut self, name: &str) -> Result<u32> {
        let (store, exports) = self.parts_mut();
        let f: TypedFunction<(), i32> = exports.instance.exports.get_typed_function(store, name)?;
        Ok(f.call(store)? as u32)
    }

    /// Whether the loaded WASM has an export called `name`.
    pub fn has_export(&self, name: &str) -> bool {
        self.exports.instance.exports.get_extern(name).is_some()
    }

    /// Low-level escape hatch: the wasmer store and instance, for calling
    /// exports with signatures the typed methods above don't cover. Re-exported as
    /// `sm64_binds::wasmer` so callers use the same wasmer version.
    pub fn wasm_parts(&mut self) -> (&mut Store, &Instance, &Memory) {
        let (store, exports) = self.parts_mut();
        (store, &exports.instance, &exports.memory)
    }
}
