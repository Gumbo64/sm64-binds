//! The precompiled game (the `precompiled` feature, x86-64 Linux): the headless
//! WASM's code compiled ahead of time by wasmer's LLVM backend and shipped in
//! `pkg/`, loaded by an engine with no compiler. Made by `make sm64-build`
//! (`maketools/precompile.py`): the code, with its data stripped and named
//! from the link map, optimised by wasm-opt (which does the inlining wasmer's
//! LLVM backend cannot) and compiled for baseline x86-64.
//!
//! The artifact holds no data: a game's memory starts from the data segments
//! of the WASM decoded from the ROM, placed before `main_func`, and that WASM's
//! code must be the code the artifact was made from ([`load`]).

use anyhow::Result;
use wasmer::sys::NativeEngineExt;
use wasmer::{Engine, Module};

use super::wasm_module::BuildId;

/// wasmer's serialized module, which only this exact wasmer version can load.
static ARTIFACT: &[u8] = include_bytes!("../../pkg/sm64_headless.x86_64-linux.wasmu");

/// SHA-1 of the WASM code (the module without its data) it was made from.
const CODE_SHA1: &str = include_str!("../../pkg/sm64_headless.x86_64-linux.wasmu.sha1");

/// The precompiled module, or `None` if it was not made from the code with
/// this [`BuildId`].
pub(crate) fn load(build_id: &BuildId) -> Result<Option<(Engine, Module)>> {
    let hex: String = build_id.iter().map(|b| format!("{b:02x}")).collect();
    if hex != CODE_SHA1.trim() {
        return Ok(None);
    }
    let engine = Engine::headless();
    // SAFETY: the bytes are this crate's own artifact, written by
    // Module::serialize of the pinned wasmer version (maketools/precompile).
    let module = unsafe { Module::deserialize(&engine, ARTIFACT) }?;
    Ok(Some((engine, module)))
}
