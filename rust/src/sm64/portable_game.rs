//! [`SM64Game`] and portable states: exporting the headless game's state in a
//! form the rendering build loads too, and loading one from either build.

use anyhow::{Error, Result};
use wasmer::TypedFunction;

use super::SM64Game;
use super::portable::{self, Build, Portable};

/// What [`SM64Game::load_portable`] did.
#[derive(Debug, Clone, Default)]
pub struct PortableLoadReport {
    /// Blocks the state needed, of which this many reused the game's own.
    pub blocks: usize,
    pub reused: usize,
    /// Bytes written.
    pub bytes: usize,
    /// Pointers the state had that this build has no equivalent of (set to
    /// null): zero for states from a build of the same sources.
    pub untranslated: u32,
}

impl SM64Game {
    /// This game's state in portable form: what [`SM64Game::load_portable`]
    /// of any build loads, so the game goes on identically.
    pub fn export_portable(&self) -> Result<Portable> {
        self.with_memory(|m| portable::export(m, Build::Headless))
            .map_err(Error::msg)
    }

    /// Makes this game the one `state` was exported from, by either build.
    ///
    /// The game's variables are overwritten, and its heap blocks are reused
    /// (or, for the ones that do not fit, allocated with the game's `malloc`)
    /// to hold the state's. Display lists, audio output and the like are not
    /// part of a state: the next frame rebuilds them.
    pub fn load_portable(&mut self, state: &Portable) -> Result<PortableLoadReport> {
        let old = self
            .with_memory(|m| portable::game_blocks(m, Build::Headless))
            .map_err(Error::msg)?;
        let addrs = state.place(&old);
        self.load_portable_at(state, addrs)
    }

    /// [`SM64Game::load_portable`] with the blocks' addresses given (0 to
    /// have one allocated): for tests.
    #[doc(hidden)]
    pub fn load_portable_at(
        &mut self,
        state: &Portable,
        mut addrs: Vec<u32>,
    ) -> Result<PortableLoadReport> {
        let sizes = state.chunk_sizes();
        let reused = addrs.iter().filter(|&&a| a != 0).count();
        for (i, a) in addrs.iter_mut().enumerate() {
            if *a == 0 {
                let (store, exports) = self.parts_mut();
                let malloc: TypedFunction<i32, i32> = exports
                    .instance
                    .exports
                    .get_typed_function(store, "malloc")?;
                *a = malloc.call(store, sizes[i] as i32)? as u32;
                if *a == 0 {
                    return Err(Error::msg("malloc failed while loading a portable state"));
                }
            }
        }
        let (patches, bad) = state.patches(Build::Headless, &addrs).map_err(Error::msg)?;
        if bad > 0 && std::env::var("SM64_PORTABLE_DEBUG").is_ok() {
            eprintln!(
                "untranslatable pointers: {:?}",
                state.untranslatable(Build::Headless, &addrs)
            );
        }
        let [gen_at, count_at] = Portable::geo_cache();
        let bytes = self.with_memory_mut(|mem| -> Result<usize, Error> {
            let mut bytes = 0;
            for (a, data) in &patches {
                let a = *a as usize;
                let dst = mem.get_mut(a..a + data.len()).ok_or_else(|| Error::msg(format!("patch at {a:#x} is outside memory")))?;
                dst.copy_from_slice(data);
                bytes += data.len();
            }
            // The render walk's cache holds nodes of the old state: invalidate it.
            let (g, c) = (gen_at as usize, count_at as usize);
            let gen_v = u32::from_le_bytes(mem[g..g + 4].try_into().unwrap()).wrapping_add(1);
            mem[g..g + 4].copy_from_slice(&gen_v.to_le_bytes());
            mem[c..c + 4].copy_from_slice(&0u32.to_le_bytes());
            Ok(bytes)
        })?;
        Ok(PortableLoadReport {
            blocks: addrs.len(),
            reused,
            bytes,
            untranslated: bad,
        })
    }
}
