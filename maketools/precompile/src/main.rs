//! `sm64-precompile <code.wasm> <out.wasmu>`: compiles a module (the game's
//! code, its data stripped; see maketools/tools.py `precompile`) with wasmer's
//! LLVM backend and writes the serialized artifact the Rust crate loads with
//! a headless engine (rust/src/sm64/precompiled.rs).
//!
//! The target is baseline x86-64 (SSE2), so the artifact runs on any x86-64
//! CPU: the host's newer instructions are worth under 1% to the game.

use std::time::Instant;

use anyhow::{Context, Result, bail};
use wasmer::sys::{CompilerConfig, CpuFeature, LLVM, NativeEngineExt, Target, Triple};
use wasmer::{Engine, Module};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        bail!("usage: sm64-precompile <code.wasm> <out.wasmu>");
    };
    let code = std::fs::read(input).with_context(|| format!("reading {input}"))?;

    let mut llvm = LLVM::new();
    // Passes the memory base and global 0 (the stack pointer) between
    // functions instead of reloading them: about 15% on the game.
    llvm.enable_pass_params_opt();
    let target = Target::new(
        "x86_64-unknown-linux-gnu".parse::<Triple>().unwrap(),
        CpuFeature::SSE2.into(),
    );
    let features = llvm.default_features_for_target(&target);
    let engine = Engine::new(Box::new(llvm), target, features);

    let started = Instant::now();
    let module = Module::new(&engine, &code).context("compiling")?;
    let bytes = module.serialize()?;
    std::fs::write(output, &bytes).with_context(|| format!("writing {output}"))?;
    println!(
        "compiled {} bytes of code in {:.1?} to {} bytes",
        code.len(),
        started.elapsed(),
        bytes.len()
    );
    Ok(())
}
