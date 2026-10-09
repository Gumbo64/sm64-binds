#!/usr/bin/env python3
"""Extracts the single WASM entry from a ZIP archive."""

import sys

from tools import check_argc, unzip_file

if __name__ == "__main__":
    check_argc(sys.argv, 3, "<path_to_zip_file> <output_file>")
    unzip_file(sys.argv[1], sys.argv[2])
