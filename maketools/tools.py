"""Helpers shared by the maketools command line scripts."""

import hashlib
import os
import re
import shutil
import subprocess
import sys
import zipfile

__all__ = [
    "xor_bytes",
    "xor_file",
    "zip_file",
    "unzip_file",
    "check_argc",
    "strip_wasm_data",
    "add_function_names",
    "precompile",
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

def _read_leb128(data: bytes, at: int):
    """Reads an unsigned LEB128 number, returning it and the offset after it."""
    value = shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, at


def strip_wasm_data(data: bytes) -> bytes:
    """A WASM module without its data (and data count) sections.

    What is left is the code: the memory's initial contents, which hold the
    game's assets, are not. The Rust bindings strip the decoded module the same
    way to check it matches the precompiled code (`src/sm64/precompiled.rs`).
    """
    if data[:4] != b"\0asm":
        raise ValueError("not a WASM module")
    out = bytearray(data[:8])
    at = 8
    while at < len(data):
        section_id = data[at]
        size, body = _read_leb128(data, at + 1)
        if section_id not in (11, 12):
            out += data[at : body + size]
        at = body + size
    return bytes(out)


def _write_leb128(n: int) -> bytes:
    out = bytearray()
    while True:
        byte = n & 0x7F
        n >>= 7
        if n:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def _sections(data: bytes):
    """(id, start, body start, end) of each section."""
    at = 8
    while at < len(data):
        size, body = _read_leb128(data, at + 1)
        yield data[at], at, body, body + size
        at = body + size


def _skip_limits(data: bytes, at: int) -> int:
    flags, at = _read_leb128(data, at)
    _, at = _read_leb128(data, at)
    if flags & 1:
        _, at = _read_leb128(data, at)
    return at


def wasm_functions(data: bytes):
    """The number of imported functions, and (offset, size) of each code body."""
    imported = 0
    bodies = []
    for section_id, _, body, _ in _sections(data):
        if section_id == 2:
            count, at = _read_leb128(data, body)
            for _ in range(count):
                for _ in range(2):  # module and field names
                    length, at = _read_leb128(data, at)
                    at += length
                kind = data[at]
                at += 1
                if kind == 0:  # function: type index
                    imported += 1
                    _, at = _read_leb128(data, at)
                elif kind == 1:  # table: reference type, limits
                    at = _skip_limits(data, at + 1)
                elif kind == 2:  # memory: limits
                    at = _skip_limits(data, at)
                elif kind == 3:  # global: value type, mutability
                    at += 2
                else:
                    raise ValueError(f"unsupported import kind {kind}")
        elif section_id == 10:
            count, at = _read_leb128(data, body)
            for _ in range(count):
                size, start = _read_leb128(data, at)
                bodies.append((at, start + size - at))
                at = start + size
    return imported, bodies


def link_map_functions(map_text: str):
    """(offset, size, name) of each function in a wasm-ld link map's CODE section."""
    functions = []
    in_code = False
    for line in map_text.splitlines():
        # Output sections: "-  <offset>  <size> CODE", the name one space after
        # the size (symbols, also uppercase at times, are indented further).
        section = re.match(r"\s*-\s+[0-9a-f]+\s+[0-9a-f]+ (\S+)$", line)
        if section:
            in_code = section.group(1) == "CODE"
            continue
        # Input sections: "-  <offset>  <size>  <file>:(<name>)".
        m = re.match(r"\s*-\s+([0-9a-f]+)\s+([0-9a-f]+)\s+\S*:\((.+)\)\s*$", line)
        if in_code and m:
            functions.append((int(m.group(1), 16), int(m.group(2), 16), m.group(3)))
    return functions


def add_function_names(data: bytes, map_text: str) -> bytes:
    """``data`` with a name section naming its functions from the link map.

    The map must describe exactly this module: as many functions, each the
    same size, all at one constant offset from where the map puts them
    (emscripten can re-encode a section header after linking).
    """
    imported, bodies = wasm_functions(data)
    functions = link_map_functions(map_text)
    if len(functions) != len(bodies):
        raise ValueError(f"the map has {len(functions)} functions, the module {len(bodies)}")
    shifts = {offset - map_offset for (offset, _), (map_offset, _, _) in zip(bodies, functions)}
    if len(shifts) != 1 or any(size != map_size for (_, size), (_, map_size, _) in zip(bodies, functions)):
        raise ValueError("the map's functions are not the module's")

    entries = bytearray(_write_leb128(len(functions)))
    for i, (_, _, name) in enumerate(functions):
        encoded = name.encode()
        entries += _write_leb128(imported + i) + _write_leb128(len(encoded)) + encoded
    payload = b"\x04name" + b"\x01" + _write_leb128(len(entries)) + entries
    names = b"\x00" + _write_leb128(len(payload)) + payload

    out = bytearray(data[:8])
    for section_id, start, body, end in _sections(data):
        if section_id == 0:
            length, at = _read_leb128(data, body)
            if data[at : at + length] == b"name":
                continue  # replaced by the new one
        out += data[start:end]
    return bytes(out + names)


# wasm-opt's settings for the precompiled module. wasmer's LLVM backend compiles
# each function on its own, so inlining has to happen here; -g keeps the names.
WASM_OPT_FLAGS = [
    "-O4", "-g",
    "--always-inline-max-function-size=40",
    "--flexible-inline-max-function-size=200",
    "--one-caller-inline-max-function-size=-1",
    "--inline-functions-with-loops",
    "--partial-inlining-ifs=4",
    "--converge",
]


def _wasm_opt():
    """$WASM_OPT, else the wasm-opt that ships with emcc (next to its clang,
    in emsdk's upstream/bin: the one the game is built with), else PATH's."""
    if os.environ.get("WASM_OPT"):
        return os.environ["WASM_OPT"]
    if shutil.which("emcc"):
        version = subprocess.run(["emcc", "-v"], capture_output=True, text=True)
        m = re.search(r"^InstalledDir: (.+)$", version.stderr + version.stdout, re.MULTILINE)
        if m and os.path.exists(os.path.join(m.group(1).strip(), "wasm-opt")):
            return os.path.join(m.group(1).strip(), "wasm-opt")
    return shutil.which("wasm-opt") or "wasm-opt"


def precompile(wasm_path, map_path, out_path):
    """Compiles the headless module's code for the Rust crate's precompiled path.

    The code (``strip_wasm_data``), named from the link map, optimised by
    wasm-opt (``WASM_OPT_FLAGS``) and compiled by wasmer's LLVM backend for
    baseline x86-64 (``maketools/precompile``). Writes ``out_path`` and
    ``out_path``.sha1, the SHA-1 of the code before optimising: the bindings
    load the artifact only for a WASM whose code has that hash.
    """
    with open(wasm_path, "rb") as f:
        code = strip_wasm_data(f.read())
    with open(map_path) as f:
        named = add_function_names(code, f.read())

    here = os.path.dirname(os.path.abspath(__file__))
    work = out_path + ".work"
    os.makedirs(work, exist_ok=True)
    try:
        named_path = os.path.join(work, "code.wasm")
        optimised_path = os.path.join(work, "code.opt.wasm")
        with open(named_path, "wb") as f:
            f.write(named)
        subprocess.run([_wasm_opt(), named_path, *WASM_OPT_FLAGS, "-o", optimised_path], check=True)
        cargo = os.environ.get("CARGO", "cargo")
        env = dict(os.environ)
        env.setdefault("LLVM_SYS_180_PREFIX", "/usr/lib/llvm-18")
        subprocess.run(
            [cargo, "run", "--release", "--quiet",
             "--manifest-path", os.path.join(here, "precompile", "Cargo.toml"),
             "--", optimised_path, out_path],
            check=True,
            env=env,
        )
    finally:
        shutil.rmtree(work, ignore_errors=True)
    with open(out_path + ".sha1", "w") as f:
        f.write(hashlib.sha1(code).hexdigest() + "\n")
    print(f"Precompiled '{wasm_path}' to '{out_path}'")
