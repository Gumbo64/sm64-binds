mod sm64;

pub use sm64::{
    Backend, FreezeMode, GamePad, GameState, RngConfig, SM64Game, SM64GameGenerator, SaveState,
};
pub mod buttons;

/// Mupen64 savestates (`.st`): see [`mupen::MupenState`].
pub use sm64::mupen;

/// The wasmer version used by [`SM64Game::wasm_parts`].
pub use wasmer;
