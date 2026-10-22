//! [`SaveState`]: a game's state as bytes, to keep on disk or send elsewhere.
//!
//! The bytes are a header and then the mutable regions of linear memory (see
//! `memory.rs`): the stack before the read-only data, then everything after
//! it (called the heap here):
//!
//! ```text
//!   offset  size  field
//!        0     8  magic "SM64SAVE"
//!        8     4  format version (little endian), currently 1
//!       12    20  build id: SHA-1 of the WASM code (the module without data)
//!       32     4  stack length (little endian)
//!       36     8  heap length (little endian)
//!       44     -  stack bytes, then heap bytes
//! ```
//!
//! A state only means something to the build that made it (addresses and
//! layouts differ between builds), so loading checks the build id.

use anyhow::{Error, Result};

use super::SM64Game;
use super::wasm_module::BuildId;

const MAGIC: &[u8; 8] = b"SM64SAVE";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 44;

/// A game's state, made by [`SM64Game::save_state`] and loaded by
/// [`SM64Game::load_state`]. Turn it into bytes with [`SaveState::as_bytes`]
/// to store or send it, and back with [`SaveState::from_bytes`].
///
/// For branching games within a process, [`SM64Game::copy_from`] and
/// [`SM64Game::sync_from`] are faster: they copy memory directly.
#[derive(Clone, PartialEq, Eq)]
pub struct SaveState {
    bytes: Vec<u8>,
}

impl std::fmt::Debug for SaveState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveState")
            .field("build", &hex(self.build_id()))
            .field("len", &self.bytes.len())
            .finish()
    }
}

impl SaveState {
    fn new(build_id: &BuildId, stack: &[u8], heap: &[u8]) -> SaveState {
        let mut bytes = Vec::with_capacity(HEADER_LEN + stack.len() + heap.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(build_id);
        bytes.extend_from_slice(&(stack.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(heap.len() as u64).to_le_bytes());
        bytes.extend_from_slice(stack);
        bytes.extend_from_slice(heap);
        SaveState { bytes }
    }

    /// The state as bytes, header included: what [`SaveState::from_bytes`]
    /// takes back.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Like [`SaveState::as_bytes`], without copying.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// A state from bytes written by [`SaveState::as_bytes`]. Fails if they
    /// are not a save state of this format, or are truncated; whether it is
    /// for the right build is checked by [`SM64Game::load_state`].
    pub fn from_bytes(bytes: Vec<u8>) -> Result<SaveState> {
        if bytes.len() < HEADER_LEN || &bytes[..8] != MAGIC {
            return Err(Error::msg("not an sm64-binds save state"));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(Error::msg(format!(
                "save state format version {version}, expected {VERSION}"
            )));
        }
        let stack = u64::from(u32::from_le_bytes(bytes[32..36].try_into().unwrap()));
        let heap = u64::from_le_bytes(bytes[36..44].try_into().unwrap());
        let expected = (HEADER_LEN as u64 + stack).saturating_add(heap);
        if bytes.len() as u64 != expected {
            return Err(Error::msg(format!(
                "save state is {} bytes, its header says {expected}",
                bytes.len()
            )));
        }
        Ok(SaveState { bytes })
    }

    fn build_id(&self) -> &[u8] {
        &self.bytes[12..32]
    }

    fn stack_len(&self) -> usize {
        u32::from_le_bytes(self.bytes[32..36].try_into().unwrap()) as usize
    }

    /// The stack bytes, then the heap bytes.
    fn memory(&self) -> &[u8] {
        &self.bytes[HEADER_LEN..]
    }
}

impl SM64Game {
    /// This game's state: its stack and heap, which are everything that
    /// changes as it plays.
    pub fn save_state(&self) -> SaveState {
        self.with_memory(|memory| {
            let (stack, heap) = self.mutable_parts(memory);
            SaveState::new(&self.build_id, stack, heap)
        })
    }

    /// Makes this game the one `state` was saved from. Fails, leaving the game
    /// as it was, if `state` came from a different build of the game.
    pub fn load_state(&mut self, state: &SaveState) -> Result<()> {
        if state.build_id() != self.build_id {
            return Err(Error::msg(format!(
                "save state is from build {}, this game is build {}",
                hex(state.build_id()),
                hex(&self.build_id)
            )));
        }
        if state.stack_len() != self.layout.stack_len() {
            return Err(Error::msg(format!(
                "save state has a {} byte stack, this game's is {}",
                state.stack_len(),
                self.layout.stack_len()
            )));
        }
        self.write_mutable_bytes(state.memory())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
