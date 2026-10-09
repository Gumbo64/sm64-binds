#!/usr/bin/env python3
"""XORs a file with a key, producing the distributable .xor payload."""

import sys

from tools import check_argc, xor_file

if __name__ == "__main__":
    check_argc(sys.argv, 4, "<input_file> <output_file> <key_file>")
    xor_file(sys.argv[1], sys.argv[2], sys.argv[3])
