"""Helpers shared by the maketools command line scripts."""

import os
import sys
import zipfile

__all__ = [
    "xor_bytes",
    "xor_file",
    "zip_file",
    "unzip_file",
    "check_argc",
]


def check_argc(argv, expected, usage):
    """Verifies the argument count, exiting with a usage message if wrong."""
    if len(argv) != expected:
        print(f"Usage: {argv[0]} {usage}", file=sys.stderr)
        sys.exit(1)


def xor_bytes(data: bytes, key: bytes) -> bytes:
    """XORs ``data`` with ``key``, repeating the key as needed.

    Runs at C speed via big integer arithmetic rather than a Python loop, which
    matters on the 6 MB payloads this repo ships.
    """
    key_len = len(key)
    if key_len == 0:
        raise ValueError("XOR key must not be empty")
    repeated_key = (key * (len(data) // key_len + 1))[: len(data)]
    result = int.from_bytes(data, "little") ^ int.from_bytes(repeated_key, "little")
    return result.to_bytes(len(data), "little")


def xor_file(input_path, output_path, key_path):
    with open(key_path, "rb") as key_file:
        key = key_file.read()
    with open(input_path, "rb") as input_file:
        data = input_file.read()
    with open(output_path, "wb") as output_file:
        output_file.write(xor_bytes(data, key))
    print(f"XOR'd '{input_path}' with '{key_path}' into '{output_path}'")


def zip_file(input_path, output_path):
    """Compresses a single file into a ZIP archive."""
    with zipfile.ZipFile(output_path, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.write(input_path, os.path.basename(input_path))
    print(f"Compressed '{input_path}' into '{output_path}'")


def unzip_file(zip_path, output_path):
    """Extracts the single entry of a ZIP archive to ``output_path``."""
    with zipfile.ZipFile(zip_path, "r") as archive:
        name = archive.namelist()[0]
        with archive.open(name) as source, open(output_path, "wb") as target:
            target.write(source.read())
    print(f"Extracted '{name}' from '{zip_path}' to '{output_path}'")