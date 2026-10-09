use std::io::{Read, Cursor};
use sha1::{Sha1, Digest};
use zip::read::ZipArchive;

const ROM_HASH: &str = "9bef1128717f958171a4afac3ed78ee2bb4e86ce";

pub fn check_hash(data: &[u8]) -> bool {
    format!("{:x}", Sha1::digest(data)) == ROM_HASH
}

pub fn unzip_bytes(zip_bytes: &[u8]) -> Vec<u8> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).unwrap();

    // We expect only one file in the ZIP archive
    let mut file = archive.by_index(0).unwrap();

    let mut file_data = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut file_data).unwrap();

    file_data
}