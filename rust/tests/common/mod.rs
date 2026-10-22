//! Raw save-state payloads for tests that edit, truncate or inflate them.
//!
//! A [`SaveState`] is a 44-byte header and then the payload: the stack, then
//! everything after the read-only memory (see `src/sm64/save_state.rs`).

#![allow(dead_code)]

use anyhow::Result;
use sm64_binds::{SM64Game, SaveState};

const HEADER_LEN: usize = 44;

pub trait RawState {
    /// The payload of this game's save state.
    fn state_payload(&self) -> Vec<u8>;
    /// Loads `payload` (laid out as [`RawState::state_payload`]) through a
    /// save state with a matching header.
    fn load_payload(&mut self, payload: &[u8]) -> Result<()>;
    /// A copy of all of linear memory.
    fn memory_copy(&self) -> Vec<u8>;
}

impl RawState for SM64Game {
    fn state_payload(&self) -> Vec<u8> {
        self.save_state().as_bytes()[HEADER_LEN..].to_vec()
    }

    fn load_payload(&mut self, payload: &[u8]) -> Result<()> {
        let mut bytes = self.save_state().into_bytes();
        bytes.truncate(HEADER_LEN);
        let stack = u32::from_le_bytes(bytes[32..36].try_into().unwrap()) as usize;
        let heap = payload
            .len()
            .checked_sub(stack)
            .expect("payload shorter than the stack");
        bytes[36..44].copy_from_slice(&(heap as u64).to_le_bytes());
        bytes.extend_from_slice(payload);
        self.load_state(&SaveState::from_bytes(bytes)?)
    }

    fn memory_copy(&self) -> Vec<u8> {
        self.with_memory(|memory| memory.to_vec())
    }
}
