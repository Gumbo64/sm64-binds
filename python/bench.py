"""CLI entry point for the Python binding benchmarks.

Kept as a script at ``python/`` root because ``make bench-python`` invokes
the file path directly; the cases and the harness they run on live in
``benchs/``.

Run with: make bench-python
"""

from benchs.sm64_cases import main

if __name__ == "__main__":
    main()
