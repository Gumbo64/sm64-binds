use std::{env, fs, io};
use std::path::Path;

use zip::ZipArchive;

// download the XOR file
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=pkg/sm64_headless.us.wasm.xor.zip");
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let binding = Path::new(&out_dir).join("sm64_headless.us.wasm.xor");
    let output_path = binding.as_path();

    // extract pkg/sm64_headless.us.wasm.xor.zip to output_path
    // Open the ZIP file
    let zip_file_path = Path::new("pkg/sm64_headless.us.wasm.xor.zip");
    let zip_file = std::fs::File::open(zip_file_path)?;
    
    // Create a ZipArchive
    let mut archive = ZipArchive::new(zip_file)?;

    // Loop through entries in the zip
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if file.name() == "sm64_headless.us.wasm.xor" {
            // Write the file to the specified output path
            let mut output_file = fs::File::create(&output_path)?;
            io::copy(&mut file, &mut output_file)?;
            break; // Exit after extracting the desired file
        }
    }
    Ok(())
}
