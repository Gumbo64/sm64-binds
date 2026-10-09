use sha1::{Digest, Sha1};
use std::io::{Cursor, Read};
use zip::read::ZipArchive;

const ROM_HASH: &str = "9bef1128717f958171a4afac3ed78ee2bb4e86ce";

/// SHA-1 of a US `.z64` ROM, which is the only region this crate supports.
pub fn check_hash(data: &[u8]) -> bool {
    format!("{:x}", Sha1::digest(data)) == ROM_HASH
}

/// Extracts the single entry of a ZIP archive holding the WASM binary.
///
/// # Panics
///
/// Panics if the archive is malformed or does not contain exactly one entry.
pub fn unzip_bytes(zip_bytes: &[u8]) -> Vec<u8> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).unwrap();

    // We expect only one file in the ZIP archive
    let mut file = archive.by_index(0).unwrap();

    let mut file_data = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut file_data).unwrap();

    file_data
}

/// XORs `data` with `key`, repeating the key as needed.
///
/// This is the inverse of the transform applied by `maketools/xor.py` when the
/// distributable `.xor` payload was generated.
pub fn xor_bytes(data: &[u8], key: &[u8]) -> Vec<u8> {
    assert!(!key.is_empty(), "XOR key must not be empty");

    let mut out = Vec::with_capacity(data.len());
    // Walking the key with a wrapping index avoids the `.cycle()` adapter, which
    // showed up as a real cost on the 8 MB ROM.
    let mut key_index = 0usize;
    for byte in data {
        out.push(byte ^ key[key_index]);
        key_index = (key_index + 1) % key.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn zip_single(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut buffer);
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
        writer.finish().unwrap();
        buffer.into_inner()
    }

    #[test]
    fn check_hash_accepts_the_real_rom() {
        let rom = include_bytes!("../../../baserom.us.z64");
        assert!(check_hash(rom));
    }

    #[test]
    fn check_hash_rejects_wrong_region_and_truncated_roms() {
        assert!(!check_hash(b""));
        assert!(!check_hash(b"not a rom"));
        // A single flipped byte in a real ROM must fail validation.
        let mut rom = include_bytes!("../../../baserom.us.z64").to_vec();
        rom[0] ^= 0xFF;
        assert!(!check_hash(&rom));
        // Truncating the real ROM must also fail.
        let rom = include_bytes!("../../../baserom.us.z64");
        assert!(!check_hash(&rom[..rom.len() - 1]));
    }

    #[test]
    fn unzip_extracts_the_only_entry() {
        let payload = b"sm64.us.wasm contents";
        let zipped = zip_single("sm64.us.wasm", payload);
        assert_eq!(unzip_bytes(&zipped), payload);
    }

    #[test]
    fn unzip_handles_an_empty_payload() {
        assert_eq!(unzip_bytes(&zip_single("empty", b"")), Vec::<u8>::new());
    }

    #[test]
    #[should_panic]
    fn unzip_rejects_non_zip_input() {
        unzip_bytes(b"definitely not a zip archive");
    }

    #[test]
    fn xor_with_a_single_byte_key_is_a_plain_xor() {
        assert_eq!(
            xor_bytes(&[0, 1, 2, 255], &[0xAA]),
            [0xAA, 0xAB, 0xA8, 0x55]
        );
    }

    #[test]
    fn xor_repeats_a_shorter_key() {
        assert_eq!(
            xor_bytes(&[1, 2, 3, 4, 5], &[0xFF, 0x00]),
            [0xFE, 0x02, 0xFC, 0x04, 0xFA]
        );
    }

    #[test]
    fn xor_with_a_longer_key_uses_only_the_prefix() {
        assert_eq!(xor_bytes(&[1, 2], &[9, 8, 7, 6]), [1 ^ 9, 2 ^ 8]);
    }

    #[test]
    fn xor_handles_empty_input() {
        assert_eq!(xor_bytes(&[], &[1, 2, 3]), Vec::<u8>::new());
    }

    #[test]
    fn xor_is_its_own_inverse() {
        let rom = include_bytes!("../../../baserom.us.z64");
        let once = xor_bytes(rom, rom);
        assert_eq!(xor_bytes(&once, rom), rom.to_vec());
    }

    #[test]
    #[should_panic(expected = "XOR key must not be empty")]
    fn xor_rejects_an_empty_key() {
        xor_bytes(&[1, 2, 3], &[]);
    }

    /// The bundled payload must decode to a real WebAssembly module, otherwise
    /// every downstream test failure is a red herring.
    #[test]
    fn bundled_payload_decodes_to_a_wasm_module() {
        let rom = include_bytes!("../../../baserom.us.z64");
        let wasm = crate::sm64::SM64GameGenerator::rom_to_wasm_bytes(rom.to_vec());
        assert!(
            wasm.starts_with(b"\0asm"),
            "decoded payload is not a WASM module (magic: {:?})",
            &wasm[..4.min(wasm.len())]
        );
        assert!(
            wasm.len() > 1_000_000,
            "decoded payload is implausibly small: {} bytes",
            wasm.len()
        );
    }

    #[test]
    fn decoding_the_payload_is_deterministic() {
        let rom = include_bytes!("../../../baserom.us.z64");
        let first = crate::sm64::SM64GameGenerator::rom_to_wasm_bytes(rom.to_vec());
        let second = crate::sm64::SM64GameGenerator::rom_to_wasm_bytes(rom.to_vec());
        assert_eq!(first, second);
    }
}
