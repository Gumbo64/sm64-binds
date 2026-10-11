#!/usr/bin/env python3
"""Checks that every language package carries the same distributable artifacts.

``make sm64-build`` copies the payloads out of ``WASM_ZIP_XOR/`` and
``WASM_JS/`` into the per-language package directories. Nothing forces that
copy to happen before a release, so a stale payload would only surface for
users, at runtime, as a broken install.

Exits non-zero if any packaged artifact differs from its source.
"""

import hashlib
import pathlib
import sys

# (source, packaged) pairs that must be byte identical.
PAIRS = [
    (
        "WASM_ZIP_XOR/sm64_headless.us.wasm.zip.xor",
        "rust/pkg/sm64_headless.us.wasm.zip.xor",
    ),
    (
        "WASM_ZIP_XOR/sm64_headless.us.wasm.zip.xor",
        "python/src/sm64_binds/pkg/sm64_headless.us.wasm.zip.xor",
    ),
    (
        "WASM_ZIP_XOR/sm64.us.wasm.zip.xor",
        # Named .png because webpack rejects .xor files.
        "javascript/src/scripts/sm64/pkg/sm64.us.wasm.zip.xor.png",
    ),
    (
        "WASM_JS/sm64.us.js",
        "javascript/src/scripts/sm64/pkg/sm64.us.js",
    ),
]

def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    failures = []

    for source, packaged in PAIRS:
        src = root / source
        pkg = root / packaged
        if not src.exists():
            failures.append(f"{source} is missing")
            continue
        if not pkg.exists():
            failures.append(f"{packaged} is missing")
            continue
        if sha256(src) != sha256(pkg):
            failures.append(f"{packaged} differs from {source}")
            print(f"  MISMATCH  {packaged}")
        else:
            print(f"  ok        {packaged}")

    # A truncated payload would only surface for users at runtime, so catch a
    # suspiciously small one here. maketools/test_tools.py does the full
    # recombine when a ROM is present.
    for packaged in (
        "rust/pkg/sm64_headless.us.wasm.zip.xor",
        "python/src/sm64_binds/pkg/sm64_headless.us.wasm.zip.xor",
    ):
        path = root / packaged
        if path.exists() and path.stat().st_size < 1_000_000:
            failures.append(f"{packaged} is implausibly small")


    if failures:
        print("\nArtifact check failed:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        print(
            "\nRun `make sm64-build` to redistribute the payloads.",
            file=sys.stderr,
        )
        return 1

    print("\nAll packaged artifacts match their sources.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
