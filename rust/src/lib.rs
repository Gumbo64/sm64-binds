mod sm64;

pub use sm64::{
    Backend, FreezeMode, GamePad, GameState, PortableLoadReport, SM64Game, SM64GameGenerator, SaveState,
};
pub mod buttons;

/// Mupen64 savestates (`.st`): see [`mupen::MupenState`].
pub use sm64::mupen;

/// Portable save states, loadable in both the headless and the rendering build.
pub use sm64::portable;

/// The wasmer version used by [`SM64Game::wasm_parts`].
pub use wasmer;
