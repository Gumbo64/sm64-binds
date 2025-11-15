use std::fs;
use std::io::{self, Read, Cursor, Write};
use sha1::{Sha1, Digest};

const ROM_HASH: &str = "9bef1128717f958171a4afac3ed78ee2bb4e86ce";

pub fn check_hash(file_path: &str) -> io::Result<bool> {
    let mut file = fs::File::open(file_path)?;
    let mut hasher = Sha1::new();
    
    // Read the file in chunks to handle large files efficiently
    let mut buffer = [0; 1024]; // 1KB buffer
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break; // End of file
        }
        hasher.update(&buffer[..bytes_read]);
    }

    // Finalize the hash and compare it to FIXED_HASH
    let hash_result = format!("{:x}", hasher.finalize());
    Ok(hash_result == ROM_HASH)
}

use flate2;
use flate2::read::GzDecoder;

macro_rules! include_bytes_compressed {
    ($path:expr) => {{
        // Load the original bytes
        let original_bytes = std::include_bytes!($path);
        
        // Compress the bytes
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(original_bytes).expect("Failed to compress data");
        let compressed_bytes = encoder.finish().expect("Failed to finish compression");

        // Return the compressed bytes
        compressed_bytes
    }};
}
pub fn get_xor_bytes() -> Vec<u8> {
    // Use a Cursor to wrap the byte vector
    let bytes = include_bytes_compressed!("../../../WASM_XOR/sm64_headless.us.wasm.xor");
    let mut d = GzDecoder::new(Cursor::new(bytes));
    let mut buffer = Vec::new();
    d.read_to_end(&mut buffer).expect("Failed to decompress data");
    buffer
}