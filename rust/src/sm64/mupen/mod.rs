//! Mupen64 savestates (`.st`): parsing them ([`MupenState`], `st.rs`) and
//! loading the game state they hold into a port game (`load.rs`).

mod layout;
mod load;
mod st;
#[rustfmt::skip]
mod tables;
mod translate;

pub use load::MupenLoadReport;
pub use st::{Compression, MupenFormat, MupenState, RDRAM_SIZE, RrMovie, US_ROM_MD5};

/// The variables [`crate::SM64Game::load_mupen_state`] copies, as (name, port
/// address, size): for checking a loaded game against another one.
pub fn copied_variables() -> Vec<(&'static str, u32, u32)> {
    tables::VARS
        .iter()
        .filter(|v| v.copy)
        .map(|v| (v.name, v.port, v.size))
        .collect()
}

/// The port address and size of a variable both builds have, by name.
pub fn port_variable(name: &str) -> Option<(u32, u32)> {
    layout::var(name).map(|v| (v.port, v.size))
}

/// Every variable both builds have (copied or not), as (name, port address,
/// size, copied): for diagnostics.
pub fn all_variables() -> Vec<(&'static str, u32, u32, bool)> {
    tables::VARS
        .iter()
        .map(|v| (v.name, v.port, v.size, v.copy))
        .collect()
}
