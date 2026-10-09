#!/usr/bin/env python3
"""Compresses a WASM build into the ZIP the bindings expect."""

import sys

from tools import check_argc, zip_file

if __name__ == "__main__":
    check_argc(sys.argv, 3, "<path_to_file> <output_zip_file>")
    zip_file(sys.argv[1], sys.argv[2])
