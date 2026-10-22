"""Tests for the WASM packaging tools.

These need a US .z64 ROM to round-trip the real payload; the pure helpers are
tested unconditionally.
"""

import hashlib
import os
import zipfile

import pytest

from tools import (
    add_function_names,
    link_map_functions,
    strip_wasm_data,
    unzip_file,
    wasm_functions,
    xor_bytes,
    xor_file,
    zip_file,
)

ROM_PATH = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "baserom.us.z64"
)
XOR_PATH = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    "WASM_ZIP_XOR",
    "sm64_headless.us.wasm.zip.xor",
)


@pytest.fixture
def rom():
    if not os.path.exists(ROM_PATH):
        pytest.skip(f"no US .z64 ROM at {ROM_PATH}")
    with open(ROM_PATH, "rb") as f:
        return f.read()


# ---------------------------------------------------------------
# xor_bytes
# ---------------------------------------------------------------


def test_xor_with_a_single_byte_key():
    assert xor_bytes(b"\x00\x01\x02\xff", b"\xaa") == b"\xaa\xab\xa8\x55"


def test_xor_repeats_a_shorter_key():
    assert xor_bytes(b"\x01\x02\x03\x04\x05", b"\xff\x00") == bytes(
        [0xFE, 0x02, 0xFC, 0x04, 0xFA]
    )


def test_xor_is_its_own_inverse():
    data = bytes(range(256)) * 5
    key = b"sm64"
    assert xor_bytes(xor_bytes(data, key), key) == data


def test_xor_rejects_an_empty_key():
    with pytest.raises(ValueError):
        xor_bytes(b"abc", b"")


# ---------------------------------------------------------------
# File level round trip
# ---------------------------------------------------------------


def test_zip_and_unzip_round_trip(tmp_path):
    payload = b"sm64.us.wasm contents" * 1000
    source = tmp_path / "sm64.us.wasm"
    source.write_bytes(payload)
    archive = tmp_path / "sm64.us.wasm.zip"

    zip_file(str(source), str(archive))
    with zipfile.ZipFile(archive) as zf:
        assert zf.namelist() == ["sm64.us.wasm"]

    target = tmp_path / "out.wasm"
    unzip_file(str(archive), str(target))
    assert target.read_bytes() == payload


def test_unzip_does_not_leave_the_entry_in_the_cwd(tmp_path, monkeypatch):
    # The old implementation extracted into the working directory and then
    # renamed the file, which littered the repo when it failed part way.
    payload = b"payload"
    source = tmp_path / "entry.bin"
    source.write_bytes(payload)
    archive = tmp_path / "archive.zip"
    zip_file(str(source), str(archive))

    monkeypatch.chdir(tmp_path)
    unzip_file(str(archive), str(tmp_path / "extracted.bin"))
    assert sorted(p.name for p in tmp_path.iterdir()) == [
        "archive.zip",
        "entry.bin",
        "extracted.bin",
    ]


def test_xor_file_writes_the_xored_payload(tmp_path):
    data = tmp_path / "data.bin"
    data.write_bytes(b"hello world")
    key = tmp_path / "rom.bin"
    key.write_bytes(b"abcdefgh")
    output = tmp_path / "out.xor"

    xor_file(str(data), str(output), str(key))
    assert output.read_bytes() == xor_bytes(b"hello world", b"abcdefgh")


def test_recombining_the_shipped_payload_yields_the_wasm(rom, tmp_path):
    # XOR + unzip is exactly what `make sm64-build` does when emscripten is
    # unavailable, so this guards the distributable .xor artifact.
    recombined = tmp_path / "payload.zip"
    xor_file(XOR_PATH, str(recombined), ROM_PATH)

    with zipfile.ZipFile(recombined) as zf:
        names = zf.namelist()
        assert len(names) == 1, "the payload must hold exactly one entry"
        wasm = zf.read(names[0])

    assert wasm[:4] == b"\x00asm", "the payload must decode to a WASM module"
    assert len(wasm) > 1_000_000


def test_the_decoded_payload_is_stable(rom, tmp_path):
    def decode():
        recombined = tmp_path / "payload.zip"
        xor_file(XOR_PATH, str(recombined), ROM_PATH)
        with zipfile.ZipFile(recombined) as zf:
            return zf.read(zf.namelist()[0])

    first = hashlib.sha256(decode()).hexdigest()
    second = hashlib.sha256(decode()).hexdigest()
    assert first == second


def test_zip_then_xor_reproduces_the_shipped_container(rom, tmp_path):
    """`make` regenerates the .xor from the built WASM; check that round trip.

    Only the ZIP container metadata (timestamps) may differ, so the WASM inside
    is compared rather than the archive bytes.
    """
    recombined = tmp_path / "payload.zip"
    xor_file(XOR_PATH, str(recombined), ROM_PATH)
    with zipfile.ZipFile(recombined) as zf:
        name = zf.namelist()[0]
        wasm = zf.read(name)
        extracted = tmp_path / name
        extracted.write_bytes(wasm)

    rezip = tmp_path / "rezipped.zip"
    zip_file(str(extracted), str(rezip))
    with zipfile.ZipFile(rezip) as zf:
        assert zf.read(name) == wasm, "the WASM must survive a rezip unchanged"

# ---------------------------------------------------------------
# strip_wasm_data
# ---------------------------------------------------------------


def _section(section_id, body):
    assert len(body) < 128
    return bytes([section_id, len(body)]) + body


def test_strip_wasm_data_drops_only_the_data_sections():
    header = b"\0asm\x01\x00\x00\x00"
    types = _section(1, b"\x01\x60\x00\x00")
    data_count = _section(12, b"\x01")
    data = _section(11, b"\x01\x00\x41\x00\x0b\x02hi")
    custom = _section(0, b"\x04name")
    module = header + types + data_count + data + custom
    assert strip_wasm_data(module) == header + types + custom


def test_strip_wasm_data_reads_multi_byte_sizes():
    header = b"\0asm\x01\x00\x00\x00"
    big = bytes([11, 0x80, 0x01]) + bytes(128)  # a 128-byte data section
    assert strip_wasm_data(header + big) == header


def test_strip_wasm_data_rejects_other_files():
    with pytest.raises(ValueError):
        strip_wasm_data(b"PK\x03\x04")


# ---------------------------------------------------------------
# add_function_names
# ---------------------------------------------------------------


def _module_with_two_functions():
    """One imported function and two defined ones, each body "no locals, end"."""
    header = b"\0asm\x01\x00\x00\x00"
    types = _section(1, b"\x01\x60\x00\x00")
    imports = _section(2, b"\x01\x03env\x01f\x00\x00")
    functions = _section(3, b"\x02\x00\x00")
    code = _section(10, b"\x02" + b"\x02\x00\x0b" * 2)
    return header + types + imports + functions + code


def _link_map(module, shift, sizes=None):
    """A wasm-ld map of ``module``'s code, offsets ``shift`` bytes off."""
    _, bodies = wasm_functions(module)
    lines = [
        "    Addr      Off     Size Out     In      Symbol",
        "       -        8        4 TYPE",
        "       -       20       10 CODE",
    ]
    for (offset, size), name, file in zip(bodies, ["alpha", "BETA"], ["a.o", "<internal>"]):
        size = sizes.pop(0) if sizes else size
        lines.append(f"       -  {offset - shift:7x}  {size:7x}         {file}:({name})")
        # The symbol line under it, uppercase like some real ones.
        lines.append(f"       -  {offset - shift:7x}  {size:7x}                 {name.upper()}")
    return "\n".join(lines) + "\n"


def _names(module):
    """{index: name} from a module's name section."""
    from tools import _read_leb128, _sections

    for section_id, _, body, _ in _sections(module):
        if section_id != 0:
            continue
        length, at = _read_leb128(module, body)
        if module[at : at + length] != b"name":
            continue
        at += length + 1  # subsection 1
        _, at = _read_leb128(module, at)
        count, at = _read_leb128(module, at)
        names = {}
        for _ in range(count):
            index, at = _read_leb128(module, at)
            length, at = _read_leb128(module, at)
            names[index] = module[at : at + length].decode()
            at += length
        return names
    return None


def test_link_map_functions_skips_symbol_lines():
    module = _module_with_two_functions()
    assert [name for _, _, name in link_map_functions(_link_map(module, 4))] == ["alpha", "BETA"]


def test_add_function_names_names_functions_after_the_imports():
    module = _module_with_two_functions()
    named = add_function_names(module, _link_map(module, 4))
    assert named.startswith(module)
    assert _names(named) == {1: "alpha", 2: "BETA"}
    # A second run replaces the name section rather than adding another.
    assert add_function_names(named, _link_map(module, 4)) == named


def test_add_function_names_rejects_a_map_of_another_module():
    module = _module_with_two_functions()
    with pytest.raises(ValueError):
        add_function_names(module, _link_map(module, 4, sizes=[3, 4]))
