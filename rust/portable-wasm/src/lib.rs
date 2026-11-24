//! The portable save state translator, as a WASM module for the browser
//! (`javascript/src/scripts/sm64/portable.js` loads it). It has no imports.
//!
//! The caller copies a game's memory into this module's memory
//! (`p_alloc` + a copy), calls an entry point, and reads the result back.
//! Calls return 0 or a count on success and -1 on failure, with the message
//! at `p_error_ptr`/`p_error_len`. `build` is 0 for the headless build and 1
//! for the rendering one.

#[path = "../../src/sm64/portable/mod.rs"]
#[allow(dead_code)]
mod portable;

use portable::{Build, Patch, Portable};

#[derive(Default)]
struct State {
    out: Vec<u8>,
    err: String,
    state: Option<Portable>,
    addrs: Vec<u32>,
    need: Vec<usize>,
    stale: Vec<u32>,
    patches: Vec<Patch>,
    bad: u32,
}

static mut STATE: Option<State> = None;

#[allow(static_mut_refs)]
fn st() -> &'static mut State {
    // SAFETY: the module is single threaded.
    unsafe { STATE.get_or_insert_with(State::default) }
}

fn build(b: u32) -> Build {
    if b == 0 {
        Build::Headless
    } else {
        Build::Visual
    }
}

fn fail(e: String) -> i32 {
    st().err = e;
    -1
}

/// Allocates `n` bytes for the caller to fill.
#[unsafe(no_mangle)]
pub extern "C" fn p_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[unsafe(no_mangle)]
pub extern "C" fn p_free(p: *mut u8, n: usize) {
    // SAFETY: `p` came from `p_alloc(n)`.
    unsafe { drop(Vec::from_raw_parts(p, 0, n.max(1))) };
}

#[unsafe(no_mangle)]
pub extern "C" fn p_error_ptr() -> *const u8 {
    st().err.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn p_error_len() -> usize {
    st().err.len()
}
#[unsafe(no_mangle)]
pub extern "C" fn p_result_ptr() -> *const u8 {
    st().out.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn p_result_len() -> usize {
    st().out.len()
}

/// Constants: 0 the allocator state's address in `build`, 1 where constant
/// data ends in `build`, 2 the visual build's texture cache address, 3 its
/// size, 4 and 5 the headless render walk cache's generation and count
/// addresses, 6 and 7 the visual build's renderer state's address and size.
#[unsafe(no_mangle)]
pub extern "C" fn p_const(what: u32, b: u32) -> u32 {
    match what {
        0 => Portable::gm(build(b)),
        1 => Portable::data_start(build(b)),
        2 => Portable::texture_cache().0,
        3 => Portable::texture_cache().1,
        4 => Portable::geo_cache()[0],
        5 => Portable::geo_cache()[1],
        6 => Portable::renderer().0,
        7 => Portable::renderer().1,
        _ => 0,
    }
}

/// The 40 hex digits of the build id (the SHA-1 of the WASM's code) the
/// tables are for, per build.
#[unsafe(no_mangle)]
pub extern "C" fn p_build_id(b: u32) -> *const u8 {
    build(b).build_id().as_ptr()
}

/// Exports the state of the game whose memory is at `mem`, as portable bytes
/// (`p_result_ptr`/`p_result_len`).
#[unsafe(no_mangle)]
pub extern "C" fn p_export(mem: *const u8, len: usize, b: u32) -> i32 {
    // SAFETY: the caller gives `len` readable bytes.
    let m = unsafe { std::slice::from_raw_parts(mem, len) };
    match portable::export(m, build(b)) {
        Ok(p) => {
            let s = st();
            s.out = p.to_bytes();
            let _ = s.state.take();
            s.out.len() as i32
        }
        Err(e) => fail(e),
    }
}

/// Like `p_export`, but the result (`p_result_ptr`) is [`Portable::digest`]:
/// the number of variables times 8 bytes.
#[unsafe(no_mangle)]
pub extern "C" fn p_digest(mem: *const u8, len: usize, b: u32) -> i32 {
    // SAFETY: the caller gives `len` readable bytes.
    let m = unsafe { std::slice::from_raw_parts(mem, len) };
    match portable::export(m, build(b)) {
        Ok(p) => {
            let s = st();
            s.out = p.digest().iter().flat_map(|d| d.to_le_bytes()).collect();
            s.out.len() as i32
        }
        Err(e) => fail(e),
    }
}

/// Starts loading portable bytes into the `target` build, whose memory is at
/// `mem`. Returns how many blocks the caller has to allocate in the target
/// (`p_need_size`, then `p_set_alloc`).
#[unsafe(no_mangle)]
pub extern "C" fn p_load_begin(
    state: *const u8,
    slen: usize,
    mem: *const u8,
    mlen: usize,
    target: u32,
) -> i32 {
    // SAFETY: the caller gives readable bytes.
    let (sb, m) = unsafe {
        (
            std::slice::from_raw_parts(state, slen),
            std::slice::from_raw_parts(mem, mlen),
        )
    };
    let p = match Portable::from_bytes(sb) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let t = build(target);
    let old = match portable::game_blocks(m, t) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let s = st();
    s.addrs = p.place(&old);
    s.need = (0..s.addrs.len()).filter(|&i| s.addrs[i] == 0).collect();
    s.stale = portable::display_list_blocks(m, t).unwrap_or_default();
    s.state = Some(p);
    s.need.len() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn p_need_size(i: usize) -> u32 {
    let s = st();
    s.state
        .as_ref()
        .map(|p| p.chunk_sizes()[s.need[i]])
        .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn p_set_alloc(i: usize, addr: u32) {
    let s = st();
    let at = s.need[i];
    s.addrs[at] = addr;
}

/// The blocks of the target's display list pool, to free (they are dropped
/// from the pool by the load).
#[unsafe(no_mangle)]
pub extern "C" fn p_stale_count() -> usize {
    st().stale.len()
}
#[unsafe(no_mangle)]
pub extern "C" fn p_stale(i: usize) -> u32 {
    st().stale[i]
}

/// Computes what to write into the target's memory; returns the number of
/// patches (`p_patch_addr`, `p_patch_ptr`, `p_patch_len`).
#[unsafe(no_mangle)]
pub extern "C" fn p_load_finish(target: u32) -> i32 {
    let s = st();
    let Some(p) = s.state.take() else {
        return fail("no state is being loaded".into());
    };
    match p.patches(build(target), &s.addrs) {
        Ok((patches, bad)) => {
            s.bad = bad;
            s.patches = patches;
            s.patches.len() as i32
        }
        Err(e) => fail(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn p_untranslated() -> u32 {
    st().bad
}
#[unsafe(no_mangle)]
pub extern "C" fn p_patch_addr(i: usize) -> u32 {
    st().patches[i].0
}
#[unsafe(no_mangle)]
pub extern "C" fn p_patch_ptr(i: usize) -> *const u8 {
    st().patches[i].1.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn p_patch_len(i: usize) -> usize {
    st().patches[i].1.len()
}
#[unsafe(no_mangle)]
pub extern "C" fn p_clear() {
    let s = st();
    s.patches = Vec::new();
    s.out = Vec::new();
    s.state = None;
}
