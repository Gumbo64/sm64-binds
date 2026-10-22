//! Where games come from: the ROM turned back into the headless WASM (or a
//! WASM given directly), and how its code is compiled ([`Backend`]: Cranelift,
//! or the precompiled module in `pkg/`). The only way to make an [`SM64Game`].

use anyhow::{Error, Result};
use wasmer::{Engine, Module};

use super::SM64Game;
use super::memory::MemoryLayout;
use super::util::{check_hash, unzip_bytes, xor_bytes};
use super::wasm_module::{self, BuildId, DataSegments};

/// The distributable WASM payload, XOR'd against a US `.z64` ROM.
const XOR_BYTES: &[u8] = include_bytes!("../../pkg/sm64_headless.us.wasm.zip.xor");

/// How a generator's games run the game's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    /// The precompiled module when it was made from this WASM's code (x86-64
    /// Linux with the `precompiled` feature), Cranelift otherwise. The
    /// `SM64_BACKEND` environment variable (`cranelift` or `precompiled`) can
    /// override this choice.
    #[default]
    Auto,
    /// Compile the WASM with Cranelift when the generator is made.
    Cranelift,
    /// The precompiled module; making the generator fails if it is not for
    /// this platform or was made from a different WASM.
    Precompiled,
}

/// Compiles the WASM module once so that every [`SM64Game`] it creates is a
/// cheap instantiation of shared, immutable code.
#[derive(Debug, Clone)]
pub struct SM64GameGenerator {
    source: GameSource,
}

impl SM64GameGenerator {
    /// Builds a generator from a US `.z64` ROM (8.00 MB, big endian).
    pub fn new(rom_bytes: Vec<u8>) -> Result<Self, Error> {
        if !check_hash(&rom_bytes) {
            return Err(Error::msg(
                "Invalid ROM. Should be a US copy, in z64 format (8.00MB)",
            ));
        };
        Self::from_wasm(Self::rom_to_wasm_bytes(rom_bytes))
    }

    #[cfg(feature = "fs")]
    pub fn from_file(rom_path: &str) -> Result<Self, Error> {
        let rom_bytes = std::fs::read(rom_path)?;
        Self::new(rom_bytes)
    }

    /// Builds a generator from a headless WASM (such as a fresh build of
    /// `sm64-port`), bypassing ROM validation.
    pub fn from_wasm(wasm_bytes: impl AsRef<[u8]>) -> Result<Self, Error> {
        Self::from_wasm_with(wasm_bytes, Backend::Auto)
    }

    /// Like [`SM64GameGenerator::from_wasm`], with a selected `backend` rather than [`Backend::Auto`].
    pub fn from_wasm_with(wasm_bytes: impl AsRef<[u8]>, backend: Backend) -> Result<Self, Error> {
        let wasm = wasm_bytes.as_ref();
        let source = match backend {
            Backend::Auto => GameSource::for_wasm(wasm)?,
            Backend::Cranelift => GameSource::cranelift(wasm)?,
            Backend::Precompiled => GameSource::precompiled(wasm)?.ok_or_else(|| {
                Error::msg("the precompiled module is not for this platform or this WASM")
            })?,
        };
        Ok(SM64GameGenerator { source })
    }

    pub fn create_game(&self) -> Result<SM64Game, Error> {
        SM64Game::from_source(&self.source)
    }

    /// Recombines the distributed XOR payload with the ROM and unzips it.
    pub fn rom_to_wasm_bytes(rom_bytes: Vec<u8>) -> Vec<u8> {
        unzip_bytes(&xor_bytes(XOR_BYTES, &rom_bytes))
    }

    /// A new game that is a copy of `game`.
    pub fn copy_from(&self, game: &SM64Game) -> Result<SM64Game, Error> {
        let mut copy = self.create_game()?;
        copy.copy_from(game)?;
        Ok(copy)
    }
}

/// What games are made from: the WASM compiled at load time by wasmer
/// (Cranelift), or the precompiled module shipped in `pkg/` and the WASM's
/// data for it.
#[derive(Debug, Clone)]
pub(super) struct GameSource {
    pub(super) engine: Engine,
    pub(super) module: Module,
    /// For the precompiled module, which has no data: the WASM's data,
    /// written into memory before `main_func`.
    pub(super) segments: Option<std::sync::Arc<DataSegments>>,
    /// The WASM's [`BuildId`], which save states are stamped with.
    pub(super) build_id: BuildId,
    /// Where its memory is constant, from its data.
    pub(super) layout: MemoryLayout,
}

impl GameSource {
    /// [`Backend::Auto`]: the precompiled module when it was made from this
    /// WASM's code, Cranelift otherwise, unless `SM64_BACKEND` asks for
    /// `cranelift`, or for `precompiled`, which then fails when it is not
    /// possible.
    fn for_wasm(wasm: &[u8]) -> Result<GameSource> {
        let wanted = std::env::var("SM64_BACKEND").ok();
        match wanted.as_deref() {
            None | Some("cranelift") | Some("precompiled") => {}
            Some(other) => {
                return Err(Error::msg(format!(
                    "SM64_BACKEND={other}: expected cranelift or precompiled"
                )));
            }
        }
        if wanted.as_deref() != Some("cranelift") {
            if let Some(source) = Self::precompiled(wasm)? {
                return Ok(source);
            }
            if wanted.as_deref() == Some("precompiled") {
                return Err(Error::msg(
                    "SM64_BACKEND=precompiled, but the precompiled module is not for this \
                     platform or was made from a different WASM",
                ));
            }
        }
        Self::cranelift(wasm)
    }

    fn cranelift(wasm: &[u8]) -> Result<GameSource> {
        let (code, data) = wasm_module::separate_data(wasm)?;
        let engine = Engine::default();
        let module = Module::new(&engine, wasm)?;
        Ok(GameSource {
            engine,
            module,
            segments: None,
            build_id: wasm_module::build_id(&code),
            layout: MemoryLayout::from_data(&data),
        })
    }

    /// The precompiled module for `wasm`, if there is one for it.
    fn precompiled(wasm: &[u8]) -> Result<Option<GameSource>> {
        #[cfg(all(
            feature = "precompiled",
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        ))]
        {
            let (code, segments) = wasm_module::separate_data(wasm)?;
            let build_id = wasm_module::build_id(&code);
            Ok(
                super::precompiled::load(&build_id)?.map(|(engine, module)| GameSource {
                    engine,
                    module,
                    layout: MemoryLayout::from_data(&segments),
                    segments: Some(std::sync::Arc::new(segments)),
                    build_id,
                }),
            )
        }
        #[cfg(not(all(
            feature = "precompiled",
            target_arch = "x86_64",
            target_os = "linux",
            target_env = "gnu"
        )))]
        {
            let _ = wasm;
            Ok(None)
        }
    }
}
