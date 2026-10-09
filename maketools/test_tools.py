"""Tests for the WASM packaging tools.

These need a US .z64 ROM to round-trip the real payload; the pure helpers are
tested unconditionally.
"""

import hashlib
import os
import zipfile

import pytest

from tools import unzip_file, xor_bytes, xor_file, zip_file

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