//! Reading (and, for tests and tools, writing) Mupen64 savestates.
//!
//! Two formats, both usually called `.st`, are recognised. The layouts are
//! documented byte for byte in `docs/mupen64-st.md`; in short:
//!
//! - **Mupen64-rr / mupen64-rr-lua** (the TAS emulator): no magic. The file is
//!   gzip (or, in newer rr-lua builds with `st_lz4` on, an LZ4 frame) around
//!   a 32-character ROM MD5, the RCP register blocks, 8 MB of RDRAM at offset
//!   0x1B0, SP memory, PIF RAM, the TLB lookup tables, the CPU registers, the
//!   TLB entries, the PC and the event queue, then (rr-lua) a movie block.
//! - **Mupen64Plus**: gzip around the magic `M64+SAVE`, a big-endian version,
//!   the ROM MD5, the register blocks, and 8 MB of RDRAM at offset 0x1BC.
//!
//! Both store RDRAM as the emulator's host-order (little-endian) `u32` words.
//! [`MupenState`] keeps it in N64 (big-endian) byte order instead, so N64
//! address `0x80000000 + i` is byte `i`.

use std::io::Read;

use anyhow::{Error, Result, bail};

/// Bytes of RDRAM both formats store (the 8 MB of an Expansion Pak, even
/// when the game only uses 4 MB).
pub const RDRAM_SIZE: usize = 0x80_0000;

/// The MD5 of the US `.z64` ROM, as both emulators write it (upper case hex).
pub const US_ROM_MD5: &str = "20B854B239203BAF6C961B850A4A51A2";

const M64P_MAGIC: &[u8; 8] = b"M64+SAVE";

/// Mupen64Plus: everything before the RDRAM is 44 header bytes and 400
/// register bytes, and the fixed part ends after `vi_field`.
const M64P_RDRAM: usize = 0x1BC;
const M64P_FIXED_LEN: usize = 16_788_288;

/// Mupen64-rr: the RDRAM follows 0x190 bytes of register blocks, and the
/// fixed part (up to and including `vi_field`) is 0xA02BB4 bytes.
const RR_RDRAM: usize = 0x1B0;
const RR_FIXED_LEN: usize = 0xA0_2BB4;
/// rr's LUT tables are 0x100000 *bytes* each (m64p's are that many words).
const RR_LUT_BYTES: usize = 0x10_0000;

/// Which emulator wrote a [`MupenState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MupenFormat {
    /// Mupen64-rr or mupen64-rr-lua (no version field in the file).
    Rr,
    /// Mupen64Plus, with the file's version (e.g. `0x00010200` for 1.2).
    Mupen64Plus { version: u32 },
}

/// How the file was compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Lz4,
}

/// The movie block mupen64-rr-lua appends when a movie was playing or
/// recording (its "freeze" data). `current_sample` is the number of input
/// samples (controller polls) played so far: the movie frame of the state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RrMovie {
    pub uid: u32,
    pub current_sample: u32,
    pub current_vi: u32,
    pub length_samples: u32,
    /// The movie's inputs up to `length_samples`, one `u32` per sample
    /// (Mupen's `BUTTONS` layout, as in an `.m64`).
    pub inputs: Vec<u32>,
}

/// A parsed Mupen64 savestate: the N64's RDRAM and CPU state.
#[derive(Clone)]
pub struct MupenState {
    pub format: MupenFormat,
    pub compression: Compression,
    /// The ROM's MD5 as stored in the state (32 hex characters).
    pub rom_md5: String,
    /// RDRAM in N64 byte order: byte `i` is physical address `i`.
    rdram: Vec<u8>,
    /// General purpose registers.
    pub gpr: [u64; 32],
    /// Coprocessor 0 registers (Count is 9, Status 12, EPC 14).
    pub cp0: [u32; 32],
    /// Floating point registers, as 64-bit values.
    pub fpr: [u64; 32],
    pub fcr31: u32,
    pub lo: u64,
    pub hi: u64,
    pub pc: u32,
    pub next_interrupt: u32,
    pub vi_field: u32,
    /// The event queue, as (event type, count) pairs.
    pub events: Vec<(u32, u32)>,
    /// rr-lua's movie block, if the state has one.
    pub movie: Option<RrMovie>,
}

impl std::fmt::Debug for MupenState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MupenState")
            .field("format", &self.format)
            .field("compression", &self.compression)
            .field("rom_md5", &self.rom_md5)
            .field("pc", &format_args!("{:#010x}", self.pc))
            .field("events", &self.events.len())
            .field("movie", &self.movie.as_ref().map(|m| m.current_sample))
            .finish()
    }
}

/// Reads little-endian values from a byte slice at a cursor.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], at: usize) -> Self {
        Cursor { bytes, at }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
        let Some(end) = end else {
            bail!("savestate truncated at {:#x}", self.at);
        };
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
}

/// Undoes the file's compression. Gzip and LZ4 frames are recognised by their
/// magic; anything else is taken as uncompressed.
pub fn decompress(bytes: &[u8]) -> Result<(Vec<u8>, Compression)> {
    // A generous cap: the largest states (rr-lua with a long movie and a
    // screenshot) are a few tens of MB.
    const LIMIT: u64 = 256 << 20;
    if bytes.starts_with(&[0x1F, 0x8B]) {
        let mut out = Vec::with_capacity(M64P_FIXED_LEN + 8192);
        flate2::read::MultiGzDecoder::new(bytes)
            .take(LIMIT)
            .read_to_end(&mut out)
            .map_err(|e| Error::msg(format!("bad gzip savestate: {e}")))?;
        Ok((out, Compression::Gzip))
    } else if bytes.starts_with(&[0x04, 0x22, 0x4D, 0x18]) {
        let mut out = Vec::with_capacity(RR_FIXED_LEN + 8192);
        lz4_flex::frame::FrameDecoder::new(bytes)
            .take(LIMIT)
            .read_to_end(&mut out)
            .map_err(|e| Error::msg(format!("bad LZ4 savestate: {e}")))?;
        Ok((out, Compression::Lz4))
    } else {
        Ok((bytes.to_vec(), Compression::None))
    }
}

/// N64 byte order from the file's little-endian words.
fn rdram_from_words(words: &[u8]) -> Vec<u8> {
    let mut out = words.to_vec();
    for w in out.as_chunks_mut::<4>().0 {
        w.reverse();
    }
    out
}

fn md5_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_string()
}

impl MupenState {
    /// Parses a savestate file's bytes (compressed or not), detecting the
    /// format. Treats the input as untrusted: every read is bounds checked.
    pub fn from_bytes(file: &[u8]) -> Result<MupenState> {
        let (raw, compression) = decompress(file)?;
        if raw.starts_with(M64P_MAGIC) {
            Self::parse_m64p(&raw, compression)
        } else if raw.starts_with(&[0xC8, 0xA6, 0xD8, 0x23]) || file.starts_with(b"PK\x03\x04") {
            bail!(
                "Project64-format states (.pj / zip) are not supported; save a Mupen64Plus .st instead"
            )
        } else if raw.len() >= RR_FIXED_LEN && raw[..32].iter().all(u8::is_ascii_hexdigit) {
            Self::parse_rr(&raw, compression)
        } else {
            bail!(
                "not a Mupen64 savestate ({} bytes after decompression, no M64+SAVE magic or ROM MD5)",
                raw.len()
            )
        }
    }

    #[cfg(feature = "fs")]
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<MupenState> {
        Self::from_bytes(&std::fs::read(path)?)
    }

    fn parse_m64p(raw: &[u8], compression: Compression) -> Result<MupenState> {
        if raw.len() < M64P_FIXED_LEN {
            bail!("Mupen64Plus savestate truncated ({} bytes)", raw.len());
        }
        let version = u32::from_be_bytes(raw[8..12].try_into().unwrap());
        if version >> 16 != 1 {
            bail!("Mupen64Plus savestate version {version:#010x} is not 1.x");
        }
        let rom_md5 = md5_string(&raw[12..44]);
        let rdram = rdram_from_words(&raw[M64P_RDRAM..M64P_RDRAM + RDRAM_SIZE]);
        let mut c = Cursor::new(raw, M64P_RDRAM + RDRAM_SIZE);
        c.skip(0x2000)?; // SP DMEM + IMEM
        c.skip(0x40)?; // PIF RAM
        c.skip(4 + 20)?; // use_flashram, old flashram state
        c.skip(2 * 0x10_0000 * 4)?; // TLB LUT_r, LUT_w (u32 each)
        let mut s = Self::read_cpu(&mut c, rdram, rom_md5, compression, false)?;
        s.format = MupenFormat::Mupen64Plus { version };
        // The event queue: 1024 bytes of (type, count) pairs (1.1 and
        // later; 1.0 wrote only up to the terminator).
        let queue = &raw[M64P_FIXED_LEN..raw.len().min(M64P_FIXED_LEN + 1024)];
        s.events = read_events(queue);
        Ok(s)
    }

    fn parse_rr(raw: &[u8], compression: Compression) -> Result<MupenState> {
        let rom_md5 = md5_string(&raw[..32]);
        let rdram = rdram_from_words(&raw[RR_RDRAM..RR_RDRAM + RDRAM_SIZE]);
        let mut c = Cursor::new(raw, RR_RDRAM + RDRAM_SIZE);
        c.skip(0x2000)?; // SP DMEM + IMEM
        c.skip(0x40)?; // PIF RAM
        c.skip(24)?; // flashram
        c.skip(2 * RR_LUT_BYTES)?;
        let mut s = Self::read_cpu(&mut c, rdram, rom_md5, compression, true)?;
        debug_assert_eq!(c.at, RR_FIXED_LEN);
        // The event queue, up to and including the 0xFFFFFFFF terminator,
        // then (rr-lua, and rr 1.1+) whether a movie block follows.
        let mut events = Vec::new();
        loop {
            let ty = c.u32()?;
            if ty == 0xFFFF_FFFF {
                break;
            }
            events.push((ty, c.u32()?));
            if events.len() > 128 {
                bail!("rr savestate event queue has no terminator");
            }
        }
        s.events = events;
        if c.at + 4 <= raw.len() && c.u32()? != 0 {
            let _size = c.u32()?;
            let uid = c.u32()?;
            let current_sample = c.u32()?;
            let current_vi = c.u32()?;
            let length_samples = c.u32()?;
            let n = (length_samples as usize).saturating_add(1);
            let inputs = c
                .take(n.saturating_mul(4))
                .map(|b| {
                    b.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|w| u32::from_le_bytes(*w))
                        .collect()
                })
                .unwrap_or_default();
            s.movie = Some(RrMovie {
                uid,
                current_sample,
                current_vi,
                length_samples,
                inputs,
            });
        }
        Ok(s)
    }

    /// The CPU block common to both formats, from `llbit` to `vi_field`.
    /// rr writes each CP0 register as 8 bytes (value, then 4 unused).
    fn read_cpu(
        c: &mut Cursor,
        rdram: Vec<u8>,
        rom_md5: String,
        compression: Compression,
        rr: bool,
    ) -> Result<MupenState> {
        let _llbit = c.u32()?;
        let mut gpr = [0u64; 32];
        for r in &mut gpr {
            *r = c.u64()?;
        }
        let mut cp0 = [0u32; 32];
        for r in &mut cp0 {
            *r = c.u32()?;
            if rr {
                c.skip(4)?;
            }
        }
        let lo = c.u64()?;
        let hi = c.u64()?;
        let mut fpr = [0u64; 32];
        for r in &mut fpr {
            *r = c.u64()?;
        }
        let _fcr0 = c.u32()?;
        let fcr31 = c.u32()?;
        c.skip(32 * 52)?; // TLB entries
        let pc = c.u32()?;
        let next_interrupt = c.u32()?;
        let _next_vi = c.u32()?;
        let vi_field = c.u32()?;
        Ok(MupenState {
            format: MupenFormat::Rr,
            compression,
            rom_md5,
            rdram,
            gpr,
            cp0,
            fpr,
            fcr31,
            lo,
            hi,
            pc,
            next_interrupt,
            vi_field,
            events: Vec::new(),
            movie: None,
        })
    }

    /// Whether the state is from the US ROM, by the MD5 it stores.
    pub fn is_us_rom(&self) -> bool {
        self.rom_md5.eq_ignore_ascii_case(US_ROM_MD5)
    }

    /// All of RDRAM, in N64 (big-endian) byte order.
    pub fn rdram(&self) -> &[u8] {
        &self.rdram
    }

    /// `len` bytes at N64 address `addr` (KSEG0/KSEG1 virtual or physical).
    pub fn bytes(&self, addr: u32, len: usize) -> Option<&[u8]> {
        let p = (addr & 0x1FFF_FFFF) as usize;
        self.rdram.get(p..p.checked_add(len)?)
    }
    pub fn u8(&self, addr: u32) -> Option<u8> {
        self.bytes(addr, 1).map(|b| b[0])
    }
    pub fn u16(&self, addr: u32) -> Option<u16> {
        self.bytes(addr, 2)
            .map(|b| u16::from_be_bytes(b.try_into().unwrap()))
    }
    pub fn s16(&self, addr: u32) -> Option<i16> {
        self.u16(addr).map(|v| v as i16)
    }
    pub fn u32(&self, addr: u32) -> Option<u32> {
        self.bytes(addr, 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    }
    pub fn f32(&self, addr: u32) -> Option<f32> {
        self.u32(addr).map(f32::from_bits)
    }

    /// A state with this RDRAM (N64 byte order, padded or cut to 8 MB) and
    /// zeroed CPU registers, for tests and tools that make synthetic states.
    pub fn from_rdram(format: MupenFormat, rdram: &[u8]) -> MupenState {
        let mut r = rdram.to_vec();
        r.resize(RDRAM_SIZE, 0);
        MupenState {
            format,
            compression: Compression::Gzip,
            rom_md5: US_ROM_MD5.to_string(),
            rdram: r,
            gpr: [0; 32],
            cp0: [0; 32],
            fpr: [0; 32],
            fcr31: 0,
            lo: 0,
            hi: 0,
            pc: 0x8000_0000,
            next_interrupt: 0,
            vi_field: 0,
            events: Vec::new(),
            movie: None,
        }
    }

    /// Mutable RDRAM, for building synthetic states.
    pub fn rdram_mut(&mut self) -> &mut [u8] {
        &mut self.rdram
    }

    /// The state as an uncompressed file in its format, with the CPU
    /// registers, PC and event queue it holds and every other register block
    /// zeroed. Emulators would not run it as is (the RCP registers are
    /// missing); it is for round-trip tests and synthetic loader tests.
    pub fn to_uncompressed_bytes(&self) -> Vec<u8> {
        let rr = matches!(self.format, MupenFormat::Rr);
        let mut b = Vec::with_capacity(M64P_FIXED_LEN + 8192);
        let mut md5 = self.rom_md5.as_bytes().to_vec();
        md5.resize(32, 0);
        if let MupenFormat::Mupen64Plus { version } = self.format {
            b.extend_from_slice(M64P_MAGIC);
            b.extend_from_slice(&version.to_be_bytes());
            b.extend_from_slice(&md5);
            b.resize(M64P_RDRAM, 0);
        } else {
            b.extend_from_slice(&md5);
            b.resize(RR_RDRAM, 0);
        }
        for w in self.rdram.as_chunks::<4>().0 {
            b.extend_from_slice(&[w[3], w[2], w[1], w[0]]);
        }
        b.resize(b.len() + 0x2000 + 0x40 + 24, 0); // SP memory, PIF RAM, flashram
        b.resize(
            b.len() + 2 * if rr { RR_LUT_BYTES } else { 0x10_0000 * 4 },
            0,
        );
        b.extend_from_slice(&0u32.to_le_bytes()); // llbit
        for r in self.gpr {
            b.extend_from_slice(&r.to_le_bytes());
        }
        for r in self.cp0 {
            b.extend_from_slice(&r.to_le_bytes());
            if rr {
                b.extend_from_slice(&[0; 4]);
            }
        }
        b.extend_from_slice(&self.lo.to_le_bytes());
        b.extend_from_slice(&self.hi.to_le_bytes());
        for r in self.fpr {
            b.extend_from_slice(&r.to_le_bytes());
        }
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&self.fcr31.to_le_bytes());
        b.resize(b.len() + 32 * 52, 0);
        for v in [self.pc, self.next_interrupt, 0, self.vi_field] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let mut queue = Vec::new();
        for &(t, n) in &self.events {
            queue.extend_from_slice(&t.to_le_bytes());
            queue.extend_from_slice(&n.to_le_bytes());
        }
        queue.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        if rr {
            b.extend_from_slice(&queue);
            match &self.movie {
                None => b.extend_from_slice(&0u32.to_le_bytes()),
                Some(m) => {
                    b.extend_from_slice(&1u32.to_le_bytes());
                    let size = 4 * (m.inputs.len() as u32) + 20;
                    for v in [
                        size,
                        m.uid,
                        m.current_sample,
                        m.current_vi,
                        m.length_samples,
                    ] {
                        b.extend_from_slice(&v.to_le_bytes());
                    }
                    let mut inputs = m.inputs.clone();
                    inputs.resize(m.length_samples as usize + 1, 0);
                    for v in inputs {
                        b.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        } else {
            debug_assert_eq!(b.len(), M64P_FIXED_LEN);
            queue.resize(1024, 0);
            b.extend_from_slice(&queue);
            b.extend_from_slice(&[0; 4]); // using_tlb
            b.extend_from_slice(&[0; 4096]); // 1.2+ extra state
        }
        b
    }

    /// [`MupenState::to_uncompressed_bytes`], gzip-compressed like the
    /// emulators write it.
    pub fn to_gzip_bytes(&self) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(&self.to_uncompressed_bytes()).unwrap();
        e.finish().unwrap()
    }
}

/// (type, count) pairs up to the 0xFFFFFFFF terminator.
fn read_events(queue: &[u8]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for e in queue.as_chunks::<8>().0 {
        let ty = u32::from_le_bytes(e[..4].try_into().unwrap());
        if ty == 0xFFFF_FFFF {
            break;
        }
        out.push((ty, u32::from_le_bytes(e[4..].try_into().unwrap())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(format: MupenFormat) -> MupenState {
        let mut s = MupenState::from_rdram(format, &[]);
        // gGlobalTimer at 0x8032D5D4, big endian.
        s.rdram_mut()[0x32D5D4..0x32D5D8].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        s.gpr[29] = 0xFFFF_FFFF_8020_0000;
        s.cp0[9] = 77;
        s.pc = 0x8024_6000;
        s.events = vec![(1, 500), (8, 900)];
        s
    }

    #[test]
    fn m64p_round_trip() {
        let s = sample(MupenFormat::Mupen64Plus {
            version: 0x0001_0200,
        });
        let raw = s.to_uncompressed_bytes();
        assert_eq!(&raw[..8], b"M64+SAVE");
        // RDRAM word for 0x8032D5D4 stored little endian at 0x1BC + offset.
        assert_eq!(&raw[0x1BC + 0x32D5D4..][..4], &0x1234_5678u32.to_le_bytes());
        for bytes in [raw.clone(), s.to_gzip_bytes()] {
            let p = MupenState::from_bytes(&bytes).unwrap();
            assert_eq!(
                p.format,
                MupenFormat::Mupen64Plus {
                    version: 0x0001_0200
                }
            );
            assert_eq!(p.u32(0x8032_D5D4), Some(0x1234_5678));
            assert_eq!(p.u16(0x8032_D5D4), Some(0x1234));
            assert_eq!(p.gpr[29], 0xFFFF_FFFF_8020_0000);
            assert_eq!(p.cp0[9], 77);
            assert_eq!(p.pc, 0x8024_6000);
            assert_eq!(p.events, vec![(1, 500), (8, 900)]);
            assert!(p.is_us_rom());
        }
    }

    #[test]
    fn rr_round_trip_with_movie() {
        let mut s = sample(MupenFormat::Rr);
        s.movie = Some(RrMovie {
            uid: 0xABCD,
            current_sample: 3,
            current_vi: 7,
            length_samples: 4,
            inputs: vec![1, 2, 3, 4, 5],
        });
        let raw = s.to_uncompressed_bytes();
        assert_eq!(&raw[0x1B0 + 0x32D5D4..][..4], &0x1234_5678u32.to_le_bytes());
        // The PC sits 16 bytes before the end of the fixed part.
        assert_eq!(
            &raw[RR_FIXED_LEN - 16..][..4],
            &0x8024_6000u32.to_le_bytes()
        );
        let p = MupenState::from_bytes(&s.to_gzip_bytes()).unwrap();
        assert_eq!(p.format, MupenFormat::Rr);
        assert_eq!(p.compression, Compression::Gzip);
        assert_eq!(p.u32(0x8032_D5D4), Some(0x1234_5678));
        assert_eq!(p.pc, 0x8024_6000);
        assert_eq!(p.cp0[9], 77);
        assert_eq!(p.events, vec![(1, 500), (8, 900)]);
        assert_eq!(p.movie, s.movie);
    }

    #[test]
    fn rr_lz4() {
        use std::io::Write;
        let s = sample(MupenFormat::Rr);
        let mut e = lz4_flex::frame::FrameEncoder::new(Vec::new());
        e.write_all(&s.to_uncompressed_bytes()).unwrap();
        let p = MupenState::from_bytes(&e.finish().unwrap()).unwrap();
        assert_eq!(p.compression, Compression::Lz4);
        assert_eq!(p.u32(0x8032_D5D4), Some(0x1234_5678));
    }

    #[test]
    fn rejects_garbage_and_truncation() {
        assert!(MupenState::from_bytes(b"hello").is_err());
        let s = sample(MupenFormat::Rr);
        let raw = s.to_uncompressed_bytes();
        assert!(MupenState::from_bytes(&raw[..RR_FIXED_LEN - 1]).is_err());
        let s = sample(MupenFormat::Mupen64Plus {
            version: 0x0001_0200,
        });
        let raw = s.to_uncompressed_bytes();
        assert!(MupenState::from_bytes(&raw[..M64P_FIXED_LEN - 1]).is_err());
    }
}
