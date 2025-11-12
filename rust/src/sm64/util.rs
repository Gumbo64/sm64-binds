use std::fs;
use std::io::{self, Read};
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
