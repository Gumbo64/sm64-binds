use std::fs;
use std::io::{self, Read, Cursor, Write};
use sha1::{Sha1, Digest};
use zip::read::ZipArchive;

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


pub fn unzip_bytes(zip_bytes: &[u8]) -> Vec<u8> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).unwrap();

    // We expect only one file in the ZIP archive
    let mut file = archive.by_index(0).unwrap();
    
    let mut file_data = Vec::new();
    file.read_to_end(&mut file_data).unwrap();

    file_data
}