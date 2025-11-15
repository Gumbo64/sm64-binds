import zipfile
import os
import sys

def decompress_file(zip_file_path, output_file_path):
    """Decompress the single file from a ZIP archive into the specified output file."""
    with zipfile.ZipFile(zip_file_path, 'r') as zipf:
        # Extract the single file
        file_name = zipf.namelist()[0]  # Get the first file name in the ZIP
        zipf.extract(file_name)

        # Move the extracted file to the specified output path
        os.rename(file_name, output_file_path)
    
    print(f"Extracted '{file_name}' from '{zip_file_path}' to '{output_file_path}'")

if __name__ == '__main__':
    # Check for the correct number of command-line arguments
    if len(sys.argv) != 3:
        print("Usage: python decompress.py <path_to_zip_file> <output_file>")
        sys.exit(1)

    # Extract command-line arguments
    zip_file_to_extract = sys.argv[1]
    output_file_path = sys.argv[2]

    # Call the decompress_file function
    decompress_file(zip_file_to_extract, output_file_path)
