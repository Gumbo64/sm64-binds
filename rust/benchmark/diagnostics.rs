//! Diagnostics: why the `hot_paths` numbers look the way they do.
//!
//! Breaks down where `create_game` goes, what the fixed cost of a WASM call
//! is through this binding, and compares implementations of the save-state
//! copy: the crate's `copy_from` against a raw memcpy over the runtime's
//! linear memory. Wasmer is used directly rather than through the crate,
//! because the comparison needs access the public API does not expose.
//!
//! Run with: cargo run --release --bench diagnostics -- ../baserom.us.z64

use std::hint::black_box;
use std::io::Read;

use sm64_binds::{GamePad, SM64GameGenerator};
use wasmer::*;

mod harness;
use harness::{Bencher, rom_path_from_args, target_from_args};

/// Recombines the distributed XOR payload with the ROM and unzips it.
///
/// Same transform as `SM64GameGenerator::rom_to_wasm_bytes`, but kept
/// separate here so diagnostics hands raw WASM bytes straight to wasmer
/// without going through the crate's engine (whose compile timing belongs to
/// `hot_paths`' setup rows, not this binary).
fn rom_wasm(rom_path: &str) -> Vec<u8> {
    let rom = std::fs::read(rom_path).expect("no ROM");
    let xor: &[u8] = include_bytes!("../pkg/sm64_headless.us.wasm.zip.xor");
    let mut zipped = Vec::with_capacity(xor.len());
    for (i, byte) in xor.iter().enumerate() {
        zipped.push(byte ^ rom[i % rom.len()]);
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zipped)).unwrap();
    let mut out = Vec::new();
    archive.by_index(0).unwrap().read_to_end(&mut out).unwrap();
    out
}

/// Instantiates the game and boots it.
///
/// The game has to run `main_func` before it will accept input, so "usable
/// instance" and "instantiated instance" are not the same; the derived row
/// below splits them.
fn boot(engine: &Engine, module: &Module) -> (Store, Instance) {
    let mut store = Store::new(engine.clone());
    let imports = imports! {
        "wasi_snapshot_preview1" => {
            "proc_exit" => Function::new_typed(&mut store, |_c: i32| {}),
            "fd_write" => Function::new_typed(&mut store, |_a: i32, _b: i32, _c: i32, _d: i32| -> i32 { 0 }),
            "fd_close" => Function::new_typed(&mut store, |_a: i32| -> i32 { 0 }),
            "fd_seek" => Function::new_typed(&mut store, |_a: i32, _b: i64, _c: i32, _d: i32| -> i32 { 0 }),
        },
    };
    let instance = Instance::new(&mut store, module, &imports).unwrap();
    let main: TypedFunction<(), ()> = instance
        .exports
        .get_typed_function(&store, "main_func")
        .unwrap();
    main.call(&mut store).unwrap();
    (store, instance)
}

fn main() {
    let mut b = Bencher::new(target_from_args(1.5));
    let engine = Engine::default();

    // ---------------------------------------------------------------
    b.section("wasm payload (once per process)");
    let rom_path = rom_path_from_args("../baserom.us.z64");
    let mut wasm_holder: Option<Vec<u8>> = None;
    b.measure_once("rom_wasm (xor + unzip)", || {
        wasm_holder = Some(rom_wasm(&rom_path));
    });
    let wasm = wasm_holder.unwrap();
    let module = Module::new(&engine, &wasm).unwrap();
    // Where the mutable memory after the read-only data starts.
    let read_only_end = SM64GameGenerator::from_wasm(&wasm)
        .and_then(|g| g.create_game())
        .unwrap()
        .read_only_memory()
        .end;

    // --- Where create_game's time goes --------------------------------
    // The engine (and thus compilation) is shared: both rows instantiate a
    // compiled `module` into a fresh `Store`, so the gap between them is
    // purely per-instance cost.
    b.section("instantiation");
    b.measure("Instance::new (fresh store)", 1.0, "call", || {
        let mut store = Store::new(engine.clone());
        let imports = imports! {
            "wasi_snapshot_preview1" => {
                "proc_exit" => Function::new_typed(&mut store, |_c: i32| {}),
                "fd_write" => Function::new_typed(&mut store, |_a: i32, _b: i32, _c: i32, _d: i32| -> i32 { 0 }),
                "fd_close" => Function::new_typed(&mut store, |_a: i32| -> i32 { 0 }),
                "fd_seek" => Function::new_typed(&mut store, |_a: i32, _b: i64, _c: i32, _d: i32| -> i32 { 0 }),
            },
        };
        black_box(Instance::new(&mut store, &module, &imports).unwrap());
    });
    b.measure("Instance::new + main_func (boot)", 1.0, "call", || {
        black_box(boot(&engine, &module));
    });
    b.derived(
        "main_func alone (the boot cost)",
        "Instance::new + main_func (boot)",
        "Instance::new (fresh store)",
        1.0,
        "call",
    );

    // --- Fixed cost of a call through the binding ---------------------
    b.section("per-call overhead");
    // The smallest module wasmer will run: `f` is the identity on i32. Every
    // step_game/get_game_state pays this much WASM-call overhead before any
    // game logic runs, which pins down how much of a frame the binding itself
    // can possibly own.
    //
    // (module (func (export "f") (param i32) (result i32) local.get 0))
    let tiny: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // magic + version
        0x01, 0x06, 0x01, 0x60, 0x01, 0x7f, 0x01, 0x7f, // type: (i32)->i32
        0x03, 0x02, 0x01, 0x00, // function 0 : type 0
        0x07, 0x05, 0x01, 0x01, b'f', 0x00, 0x00, // export "f"
        0x0a, 0x06, 0x01, 0x04, 0x00, 0x20, 0x00, 0x0b, // body: local.get 0
    ];
    let tiny_module = Module::new(&engine, tiny).unwrap();
    let mut tiny_store = Store::new(engine.clone());
    let tiny_instance = Instance::new(&mut tiny_store, &tiny_module, &Imports::new()).unwrap();
    let tiny_fn: TypedFunction<i32, i32> = tiny_instance
        .exports
        .get_typed_function(&tiny_store, "f")
        .unwrap();
    b.measure("empty WASM call through this API", 1.0, "call", || {
        black_box(tiny_fn.call(&mut tiny_store, 1).unwrap());
    });

    // --- Save-state copy implementations ------------------------------
    // Three ways to move the mutable bytes around, from fastest (a real
    // memcpy) through the crate's `copy_from` down to what `copy_from` used to
    // do before it got the bulk-memcpy treatment. `raw` minus `old` is the
    // runtime-accessor overhead the crate no longer pays.
    b.section("save-state copy implementations");
    let (mut store, instance) = boot(&engine, &module);
    let memory = instance.exports.get_memory("memory").unwrap().clone();
    let step: TypedFunction<(u32, i32, i32), ()> = instance
        .exports
        .get_typed_function(&store, "step_game")
        .unwrap();
    // Step the game so it is in a playing state rather than freshly booted;
    // the copies below move bytes around, and those bytes should look like a
    // session's heap, not a boot-time heap.
    for _ in 0..1200 {
        step.call(&mut store, 0x8000, 0, 80).unwrap();
    }

    let view: MemoryView = memory.view(&store);
    let size = view.data_size() as usize;
    let copy_len = size - read_only_end;
    b.note(&format!(
        "the mutable part is {:.2} MB of {:.2} MB linear memory",
        copy_len as f64 / 1048576.0,
        size as f64 / 1048576.0
    ));

    let offset = read_only_end;
    let ptr = view.data_ptr();

    // What the crate does now: one memcpy of everything mutable.
    let mut scratch = vec![0u8; copy_len];
    b.measure("raw memcpy over data_ptr", 1.0, "call", || {
        // SAFETY: `ptr` is valid for `size` bytes while `view` is alive, and
        // `scratch` is a separate allocation of the matching length.
        unsafe {
            std::ptr::copy_nonoverlapping(ptr.add(offset), scratch.as_mut_ptr(), scratch.len());
        }
        black_box(&scratch);
    });

    // What the crate used to do: go through the runtime's own accessors,
    // copying the ranges out into a fresh Vec and then back in. Two full
    // passes and an allocation per save were the old cost.
    b.measure(
        "copy_range_to_vec + write (the old path)",
        1.0,
        "call",
        || {
            let buffer = view.copy_range_to_vec(offset as u64..size as u64).unwrap();
            memory.view(&store).write(offset as u64, &buffer).unwrap();
            black_box(&buffer);
        },
    );
    b.derived(
        "what the old path wasted",
        "copy_range_to_vec + write (the old path)",
        "raw memcpy over data_ptr",
        1.0,
        "call",
    );

    // The same data flow through the public API, tying this binary's numbers
    // back to `hot_paths`: with both sides warmed, `copy_from` should sit near
    // the raw memcpy.
    let rom = std::fs::read(&rom_path).expect("no ROM");
    let crate_generator = SM64GameGenerator::new(rom).expect("generator");
    let mut source_game = crate_generator.create_game().unwrap();
    let mut target_game = crate_generator.create_game().unwrap();
    let warm_pad = GamePad::default();
    for _ in 0..1200 {
        source_game.step_game(warm_pad).unwrap();
    }
    b.measure("SM64Game::copy_from (public API)", 1.0, "call", || {
        target_game.copy_from(&source_game).unwrap();
        black_box(());
    });
}
