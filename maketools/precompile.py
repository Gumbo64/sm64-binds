#!/usr/bin/env python3
"""Precompiles the headless module's code for the Rust crate (rust/pkg/*.wasmu)."""

import sys

from tools import check_argc, precompile

if __name__ == "__main__":
    check_argc(sys.argv, 4, "<headless_wasm> <link_map> <out.wasmu>")
    precompile(sys.argv[1], sys.argv[2], sys.argv[3])
