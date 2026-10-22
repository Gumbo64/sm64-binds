//! Separates a headless WASM module into its code and its data.
//!
//! A WASM file is made of sections. Most of them are the code: functions,
//! types, imports, exports and so on. The data section is the game's starting
//! memory (the assets and tables that came from the ROM), as a list of
//! segments that say "put these bytes at this address".
//!
//! The precompiled module was compiled from the code alone, the same way
//! `maketools/tools.py` (`strip_wasm_data`) removes the data, so loading it
//! needs both halves back from the WASM:
//!
//! - the data, which is written into memory before the game starts, and
//! - the code's [`BuildId`], to check the precompiled module was made from
//!   this code. Save states are stamped with the same id.

use anyhow::{Error, Result};
use sha1::{Digest, Sha1};

/// Which build of the game a WASM is: the SHA-1 of its code (the module with
/// its data removed). Rebuilding `sm64-port` changes it; changing only the
/// data does not.
pub(super) type BuildId = [u8; 20];

/// A module's data segments, as (memory address, bytes to put there).
pub(super) type DataSegments = Vec<(usize, Vec<u8>)>;

// WASM section ids.
const START_SECTION: u8 = 8;
const DATA_SECTION: u8 = 11;
const DATA_COUNT_SECTION: u8 = 12;

/// Takes the data out of a WASM module. Returns the module without its data
/// sections (the code, byte for byte what `strip_wasm_data` writes) and the
/// data segments that were in them.
pub(super) fn separate_data(wasm: &[u8]) -> Result<(Vec<u8>, DataSegments)> {
    if !wasm.starts_with(b"\0asm") || wasm.len() < 8 {
        return Err(Error::msg("not a WASM module"));
    }
    // The 8-byte header (magic and version), then each section in order.
    let mut code = wasm[..8].to_vec();
    let mut data = Vec::new();
    let mut at = 8;
    while at < wasm.len() {
        let id = wasm[at];
        let mut body = at + 1;
        let size = read_leb128(wasm, &mut body)? as usize;
        let end = body + size;
        if end > wasm.len() {
            return Err(Error::msg("truncated WASM"));
        }
        match id {
            // A start function runs while the module is instantiated, before
            // the data has been written in. wasm-ld does not emit one.
            START_SECTION => return Err(Error::msg("unsupported start section")),
            DATA_SECTION => read_data_section(&wasm[body..end], &mut data)?,
            // Only the number of data segments: part of the data.
            DATA_COUNT_SECTION => {}
            _ => code.extend_from_slice(&wasm[at..end]),
        }
        at = end;
    }
    Ok((code, data))
}

/// The [`BuildId`] of `code`, a module returned by [`separate_data`].
pub(super) fn build_id(code: &[u8]) -> BuildId {
    Sha1::digest(code).into()
}

/// Adds the segments of a data section's body to `data`.
///
/// Only the form wasm-ld writes is accepted: active segments of memory 0 at a
/// constant address, `0x00 (i32.const address) end length bytes`.
fn read_data_section(section: &[u8], data: &mut DataSegments) -> Result<()> {
    let mut p = 0;
    for _ in 0..read_leb128(section, &mut p)? {
        const I32_CONST: u8 = 0x41;
        const END: u8 = 0x0b;
        if read_leb128(section, &mut p)? != 0 || section.get(p) != Some(&I32_CONST) {
            return Err(Error::msg("unsupported data segment"));
        }
        p += 1;
        // i32.const is signed; the address is its bits as unsigned.
        let address = read_sleb128(section, &mut p)? as u32 as usize;
        if section.get(p) != Some(&END) {
            return Err(Error::msg("unsupported data segment address"));
        }
        p += 1;
        let len = read_leb128(section, &mut p)? as usize;
        let bytes = section
            .get(p..p + len)
            .ok_or_else(|| Error::msg("truncated WASM"))?;
        data.push((address, bytes.to_vec()));
        p += len;
    }
    Ok(())
}

/// Reads an unsigned LEB128 integer (7 bits per byte, high bit set on all
/// but the last byte) at `*at`, and moves `*at` past it.
fn read_leb128(bytes: &[u8], at: &mut usize) -> Result<u64> {
    let (mut value, mut shift) = (0u64, 0);
    loop {
        let byte = *bytes.get(*at).ok_or_else(|| Error::msg("truncated WASM"))?;
        *at += 1;
        if shift >= 64 {
            return Err(Error::msg("malformed WASM integer"));
        }
        value |= u64::from(byte & 0x7f) << shift;
        shift += 7;
        if byte < 0x80 {
            return Ok(value);
        }
    }
}

/// Like [`read_leb128`], for a signed integer: sign-extended from bit 6 of
/// the last byte.
fn read_sleb128(bytes: &[u8], at: &mut usize) -> Result<i64> {
    let (mut value, mut shift) = (0i64, 0);
    loop {
        let byte = *bytes.get(*at).ok_or_else(|| Error::msg("truncated WASM"))?;
        *at += 1;
        if shift >= 64 {
            return Err(Error::msg("malformed WASM integer"));
        }
        value |= i64::from(byte & 0x7f) << shift;
        shift += 7;
        if byte < 0x80 {
            if shift < 64 && byte & 0x40 != 0 {
                value |= -1 << shift;
            }
            return Ok(value);
        }
    }
}
