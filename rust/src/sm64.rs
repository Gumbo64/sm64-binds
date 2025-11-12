use wasmtime::*;

use std::convert::TryInto;

mod game_pad;
mod game_state;
mod rng_config;
mod util;

pub use game_pad::GamePad;
pub use game_state::GameState;
pub use rng_config::RandomConfig;

pub struct SM64GameGenerator {
    wasm_bytes: Vec<u8>,
}

impl SM64GameGenerator {
    #[cfg(feature = "fs")]
    pub fn new(rom_path: &str) -> Result<Self, Error> {
        match util::check_hash(rom_path) {
            Ok(_) => {},
            Err(_) => {return Err(Error::msg("Invalid ROM. Should be a US copy, in z64 format (8.00MB)"));}
        };
        let rom_bytes = std::fs::read(rom_path)?;
        Self::from_rom_bytes(rom_bytes)
    }

    pub fn from_rom_bytes(rom_bytes: Vec<u8>) -> Result<Self, Error> {
        let wasm_bytes = Self::rom_to_wasm_bytes(rom_bytes);
        Ok(SM64GameGenerator { wasm_bytes })
    }

    pub fn create_game(&self, cfg: Option<RandomConfig>) -> Result<SM64Game, Error>  {
        SM64Game::new(self.wasm_bytes.clone(), cfg)
    }

    pub fn rom_to_wasm_bytes(rom_bytes: Vec<u8>) -> Vec<u8> {
        let rom_len = rom_bytes.len();
        let xor_bytes = include_bytes!("../../WASM_XOR/sm64_headless.us.wasm.xor");
        let mut wasm_bytes: Vec<u8> = Vec::new();
        for i in 0..xor_bytes.len() {
            wasm_bytes.push(xor_bytes[i] ^ rom_bytes[i % rom_len]);
        }
        wasm_bytes
    }
}


pub struct SM64Game {
    store: Store<u32>,
    instance: Instance,
    memory: Memory,
    using_rng: bool
}

impl SM64Game {
    pub fn new(wasm_bytes: Vec<u8>, cfg: Option<RandomConfig>) -> Result<Self, Error> {
        let engine = Engine::default();
        let mut store: Store<u32> = Store::new(&engine, 4);
        let module = Module::new(&engine, wasm_bytes)?;
        
        let mut linker = Linker::new(&engine);
        linker.define_unknown_imports_as_traps(&module)?;
        
        let instance = linker.instantiate(&mut store, &module)?;
        let memory = instance.get_memory(&mut store, "memory").unwrap();

        let using_rng;
        if let Some(c) = cfg {
            using_rng = true;
            let rng_init = instance.get_typed_func::<(u32, u32, u32, f32, f32, f32), ()>(&mut store, "rng_init")?;
            rng_init.call(&mut store, (
                c.seed, c.max_random_action, c.max_window_length, 
                c.a_prob, c.b_prob, c.z_prob)
            )?;
        } else {
            using_rng = false;
        }

        let main_func = instance.get_typed_func::<(), ()>(&mut store, "main_func")?;
        main_func.call(&mut store, ())?;

        Ok(SM64Game {
            store,
            instance,
            memory,
            using_rng,
        })
    }


    pub fn step_game(&mut self, pad: GamePad) -> Result<(), Error> {
        let step_game = self.instance.get_typed_func::<(u32, i32, i32), ()>(&mut self.store, "step_game")?;
        step_game.call(&mut self.store, (pad.button.into(), pad.stick_x.into(), pad.stick_y.into()))?;
        Ok(())
    }

    pub fn get_game_state(&mut self) -> Result<GameState, Error> {
        let get_game_state = self.instance.get_typed_func::<(), i32>(&mut self.store, "get_game_state")?;
        let pointer = get_game_state.call(&mut self.store, ())?;
        let mut buffer: [u8; 60] = [0; 60];
        self.memory.read(&mut self.store, pointer.try_into()?, &mut buffer)?;
        let state = GameState::new(&buffer);
        Ok(state)
    }

    pub fn rng_pad(&mut self, pad: GamePad) -> Result<GamePad, Error> {
        let rng_pad = self.instance.get_typed_func::<(u32, i32, i32), i32>(&mut self.store, "rng_pad")?;
        let pointer = rng_pad.call(&mut self.store, (pad.button.into(), pad.stick_x.into(), pad.stick_y.into()))?;
        let mut buffer: [u8; 4] = [0; 4];
        self.memory.read(&mut self.store, pointer.try_into()?, &mut buffer)?;
        let pad = GamePad::from_bytes(&buffer);
        Ok(pad)
    }
    pub fn using_rng(&self) -> bool {
        self.using_rng
    }
}

