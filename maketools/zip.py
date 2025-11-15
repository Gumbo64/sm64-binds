import zipfile
import os
import sys

def compress_file(file_path, zip_file_path):
    """Compress a given file into a ZIP archive."""
    with zipfile.ZipFile(zip_file_path, 'w', zipfile.ZIP_DEFLATED) as zipf:
        zipf.write(file_path, os.path.basename(file_path))
    print(f"Compressed '{file_path}' into '{zip_file_path}'")

if __name__ == '__main__':
    # Check for the correct number of command-line arguments
    if len(sys.argv) != 3:
        print("Usage: python compress.py <path_to_file> <output_zip_file>")
        sys.exit(1)

    # Extract command-line arguments
    file_to_compress = sys.argv[1]
    output_zip_file = sys.argv[2]

    # Call the compress_file function
    compress_file(file_to_compress, output_zip_file)
