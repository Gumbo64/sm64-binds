use wasmer::*;

use anyhow::{Result, Error};
use std::convert::TryInto;

mod game_pad;
mod game_state;
mod rng_config;
mod util;

pub use game_pad::GamePad;
pub use game_state::GameState;
pub use rng_config::RngConfig;
use util::unzip_bytes;

#[derive(Debug, Clone)]
pub struct SM64GameGenerator {
    wasm_bytes: Vec<u8>,
}

impl SM64GameGenerator {
    pub fn new(rom_bytes: Vec<u8>) -> Result<Self, Error> {
        if !util::check_hash(&rom_bytes) {
            return Err(Error::msg("Invalid ROM. Should be a US copy, in z64 format (8.00MB)"));
        };
        let wasm_bytes = Self::rom_to_wasm_bytes(rom_bytes);

        Ok(SM64GameGenerator { wasm_bytes })
    }
    #[cfg(feature = "fs")]
    pub fn from_file(rom_path: &str) -> Result<Self, Error> {
        let rom_bytes = std::fs::read(rom_path)?;
        Self::new(rom_bytes)
    }
    pub fn create_game(&self) -> Result<SM64Game, Error>  {
        SM64Game::new(self.wasm_bytes.clone())
    }

    pub fn rom_to_wasm_bytes(rom_bytes: Vec<u8>) -> Vec<u8> {
        let rom_len = rom_bytes.len();
        const XOR_BYTES: &[u8] = include_bytes!("../pkg/sm64_headless.us.wasm.zip.xor");
        let mut wasm_zip_bytes: Vec<u8> = Vec::new();
        for i in 0..XOR_BYTES.len() {
            wasm_zip_bytes.push(XOR_BYTES[i] ^ rom_bytes[i % rom_len]);
        }

        unzip_bytes(&wasm_zip_bytes)
    }

    pub fn copy(&self, game: &mut SM64Game) -> Result<SM64Game, Error> {
        let mut cloned_game = self.create_game()?;
        
        game.copy_to(&mut cloned_game)?;

        Ok(cloned_game)
    }
}


pub struct SM64Game {
    store: Store,
    instance: Instance,
    using_rng: bool
}

impl SM64Game {
    // from using the twiggy tool (twiggy top sm64.us.wasm). Seems to be constant.
    // I believe this segment only has constants so no need to save/load it
    const DATA0_SIZE: usize = 8013002;
    pub fn new(wasm_bytes: Vec<u8>) -> Result<Self> {
        let using_rng = false;

        let mut store: Store = Store::default();
        let module = Module::new(&store, wasm_bytes)?;
        let import_object = imports! {
            "wasi_snapshot_preview1" => {
                "proc_exit" => Function::new_typed(&mut store, |_exit_code: i32| {
                    println!("proc_exit");
                }),
                "fd_write" => Function::new_typed(&mut store, |_fd: i32, _iovs: i32, _iovs_len: i32, _bytes_written: i32| -> i32 {
                    println!("fd_write");
                    0
                }),
                "fd_close" => Function::new_typed(&mut store, |_fd: i32| -> i32 {
                    println!("fd_close");
                    0
                }),
                "fd_seek" => Function::new_typed(&mut store, |_fd: i32, _offset: i64, _whence: i32, _new_offset: i32| -> i32 {
                    println!("fd_seek");
                    0
                }),
            },
        };

        let instance = Instance::new(&mut store, &module, &import_object).unwrap();


        let main_func: TypedFunction<(), ()> = instance.exports.get_typed_function::<(), ()>(&mut store, "main_func")?;
        main_func.call(&mut store)?;

        Ok(SM64Game {
            store,
            instance,
            using_rng,
        })
    }

    pub fn set_rng_config(&mut self, cfg: RngConfig) -> Result<()> {
        self.using_rng = true;
        let set_rng_config: TypedFunction<(u32, u32, u32, f32, f32, f32), ()> = self.instance.exports.get_typed_function::<(u32, u32, u32, f32, f32, f32), ()>(&mut self.store, "set_rng_config")?;
        set_rng_config.call(&mut self.store,
            cfg.window_length, cfg.random_amount, cfg.random_burst_length,  
            cfg.a_prob, cfg.b_prob, cfg.z_prob
        )?;
        Ok(())
    }

    pub fn set_rng_seed(&mut self, seed: u32) -> Result<()> {
        self.using_rng = true;
        let set_rng_config: TypedFunction<u32, ()> = self.instance.exports.get_typed_function::<u32, ()>(&mut self.store, "set_rng_seed")?;
        set_rng_config.call(&mut self.store,
            seed
        )?;
        Ok(())
    }

    pub fn step_game(&mut self, pad: GamePad) -> Result<()> {
        let step_game: TypedFunction<(u32, i32, i32), ()> = self.instance.exports.get_typed_function::<(u32, i32, i32), ()>(&mut self.store, "step_game")?;
        step_game.call(&mut self.store, pad.button.into(), pad.stick_x.into(), pad.stick_y.into())?;
        Ok(())
    }

    pub fn get_game_state(&mut self) -> Result<GameState, Error> {
        let get_game_state: TypedFunction<(), i32> = self.instance.exports.get_typed_function::<(), i32>(&mut self.store, "get_game_state")?;
        let pointer = get_game_state.call(&mut self.store)?;

        let mut buffer: [u8; 64] = [0; 64];
        let memory = self.instance.exports.get_memory("memory")?;
        let view = memory.view(&self.store);
        view.read(pointer.try_into()?, &mut buffer)?;
        let state = GameState::new(&buffer);
        Ok(state)
    }

    pub fn rng_pad(&mut self, pad: GamePad) -> Result<GamePad, Error> {
        let rng_pad: TypedFunction<(u32, i32, i32), i32> = self.instance.exports.get_typed_function::<(u32, i32, i32), i32>(&mut self.store, "rng_pad")?;
        let pointer = rng_pad.call(&mut self.store, pad.button.into(), pad.stick_x.into(), pad.stick_y.into())?;

        let mut buffer: [u8; 4] = [0; 4];
        let memory = self.instance.exports.get_memory("memory")?;
        let view = memory.view(&self.store);
        view.read(pointer.try_into()?, &mut buffer)?;
        let pad = GamePad::from_bytes(&buffer);
        Ok(pad)
    }
    
    pub fn using_rng(&self) -> bool {
        self.using_rng
    }

    fn expand_memory(&mut self, save_state_length: usize) -> Result<(), Error> {
        // allocate more memory if necessary for the save state
        let memory = self.instance.exports.get_memory("memory")?;
        let required_total_bytes = (SM64Game::DATA0_SIZE + save_state_length) as u64;
        memory.grow_at_least(&mut self.store, required_total_bytes)?;
        Ok(())
    }

    pub fn get_memory_bytes(&self) -> Result<Vec<u8>, Error> {
        let memory = self.instance.exports.get_memory("memory")?;
        let view = memory.view(&self.store);

        let total_size = view.data_size();
        let buffer = view.copy_range_to_vec((SM64Game::DATA0_SIZE as u64)..total_size)?;
        Ok(buffer)
    }

    pub fn write_memory_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.expand_memory(bytes.len())?;

        let memory = self.instance.exports.get_memory("memory")?;
        
        let final_view = memory.view(&self.store);
        Ok(final_view.write(SM64Game::DATA0_SIZE as u64, &bytes)?)
    }

    pub fn copy_to(&self, game_to: &mut SM64Game) -> Result<(), Error> {
        let b = self.get_memory_bytes()?;
        game_to.write_memory_bytes(b.as_slice())?;
        Ok(())
    }
    
    pub fn full_copy_to(&self, game_to: &mut SM64Game) -> Result<(), Error> {
        let view_from = self.instance.exports.get_memory("memory")?.view(&self.store);
        let view_to = game_to.instance.exports.get_memory("memory")?.view(&mut game_to.store);

        Ok(view_from.copy_to_memory(view_from.data_size(), &view_to)?)
    }

}

