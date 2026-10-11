"""Tests for the pure helpers, which need no ROM."""

import io
import struct
import zipfile

import pytest

from sm64_binds import (
    GAME_STATE_SIZE,
    GamePad,
    GameState,
    RngConfig,
    buttons,
    check_hash,
    rom_to_wasm_bytes,
    unzip_bytes,
    xor_bytes,
)


def zip_single(name, contents):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(name, contents)
    return buffer.getvalue()


# ---------------------------------------------------------------
# xor_bytes
# ---------------------------------------------------------------


def test_xor_with_a_single_byte_key():
    assert xor_bytes(b"\x00\x01\x02\xff", b"\xaa") == b"\xaa\xab\xa8\x55"


def test_xor_repeats_a_shorter_key():
    assert xor_bytes(b"\x01\x02\x03\x04\x05", b"\xff\x00") == bytes(
        [0xFE, 0x02, 0xFC, 0x04, 0xFA]
    )


def test_xor_uses_only_the_key_prefix_when_the_key_is_longer():
    assert xor_bytes(b"\x01\x02", b"\x09\x08\x07\x06") == bytes([0x01 ^ 9, 0x02 ^ 8])


def test_xor_with_empty_input():
    assert xor_bytes(b"", b"abc") == b""


def test_xor_is_its_own_inverse():
    data = bytes(range(256)) * 7
    key = b"sm64"
    assert xor_bytes(xor_bytes(data, key), key) == data


def test_xor_rejects_an_empty_key():
    with pytest.raises(ValueError):
        xor_bytes(b"abc", b"")


# ---------------------------------------------------------------
# unzip_bytes
# ---------------------------------------------------------------


def test_unzip_extracts_the_only_entry():
    payload = b"sm64.us.wasm contents"
    assert unzip_bytes(zip_single("sm64.us.wasm", payload)) == payload


def test_unzip_handles_an_empty_payload():
    assert unzip_bytes(zip_single("empty", b"")) == b""


def test_unzip_rejects_non_zip_input():
    with pytest.raises(zipfile.BadZipFile):
        unzip_bytes(b"definitely not a zip archive")


# ---------------------------------------------------------------
# check_hash
# ---------------------------------------------------------------


def test_check_hash_rejects_junk():
    assert not check_hash(b"")
    assert not check_hash(b"not a rom")


def test_check_hash_rejects_a_flipped_byte():
    # The real ROM's hash is checked in the integration tests; here we only
    # need to know that a wrong hash is reported as invalid.
    assert not check_hash(b"\x00" * 0x800000)


# ---------------------------------------------------------------
# GamePad
# ---------------------------------------------------------------


def test_gamepad_from_bytes_decodes_little_endian():
    # A_BUTTON is 0x8000, which is [0x00, 0x80] little endian.
    pad = GamePad.from_bytes(b"\x00\x80\x50\xb0")
    assert pad.button == buttons.A_BUTTON
    assert pad.stick_x == 80
    assert pad.stick_y == -80


def test_gamepad_from_bytes_ignores_extra_bytes():
    # The WASM export also writes errnum at offset 3 of a 4 byte struct.
    pad = GamePad.from_bytes(b"\x00\x80\x00\x00\xff\xff")
    assert pad == GamePad(buttons.A_BUTTON, 0, 0)


def test_gamepad_defaults_to_neutral():
    assert GamePad() == GamePad(0, 0, 0)


def test_gamepad_equality():
    assert GamePad(buttons.A_BUTTON, 1, 2) == GamePad(buttons.A_BUTTON, 1, 2)
    assert GamePad(buttons.A_BUTTON, 1, 2) != GamePad(buttons.B_BUTTON, 1, 2)
    assert GamePad(buttons.A_BUTTON, 1, 2) != GamePad(buttons.A_BUTTON, 9, 2)
    assert GamePad(buttons.A_BUTTON, 1, 2) != GamePad(buttons.A_BUTTON, 1, 9)


def test_gamepad_is_hashable():
    # __eq__ without __hash__ would make pads unhashable.
    assert len({GamePad(1, 2, 3), GamePad(1, 2, 3), GamePad(4, 5, 6)}) == 2


def test_gamepad_repr_is_readable():
    assert "0x8000" in repr(GamePad(buttons.A_BUTTON, 0, 0))


# ---------------------------------------------------------------
# GameState
# ---------------------------------------------------------------


def encode_state(fields):
    """Builds the byte image the WASM export would write."""
    (
        num_stars,
        pos,
        vel,
        lakitu_pos,
        lakitu_yaw,
        in_credits,
        course_num,
        act_num,
        area_index,
        level_num,
        lives,
        coins,
        health,
    ) = fields
    out = struct.pack("<i", num_stars)
    for vec in (pos, vel, lakitu_pos):
        out += struct.pack("<3f", *vec)
    out += struct.pack(
        "<9i",
        lakitu_yaw,
        in_credits,
        course_num,
        act_num,
        area_index,
        level_num,
        lives,
        coins,
        health,
    )
    return out


SAMPLE = (
    7,  # num_stars
    (1.0, 2.0, 3.0),  # pos
    (-1.5, 0.25, 100.0),  # vel
    (-64.0, 512.0, 0.5),  # lakitu_pos
    4096,  # lakitu_yaw
    1,  # in_credits
    5,  # course_num
    2,  # act_num
    3,  # area_index
    11,  # level_num
    4,  # lives
    42,  # coins
    255,  # health
)


def test_game_state_is_seventy_six_bytes():
    # 10 s32 scalars plus three 3-float vectors, with no padding.
    assert GAME_STATE_SIZE == 76
    assert len(encode_state(SAMPLE)) == GAME_STATE_SIZE


def test_game_state_reads_every_field():
    state = GameState(encode_state(SAMPLE))
    assert state.numStars == 7
    assert state.pos == [1.0, 2.0, 3.0]
    assert state.vel == [-1.5, 0.25, 100.0]
    assert state.lakituPos == [-64.0, 512.0, 0.5]
    assert state.lakituYaw == 4096
    assert state.inCredits == 1
    assert state.courseNum == 5
    assert state.actNum == 2
    assert state.areaIndex == 3
    assert state.levelNum == 11
    assert state.lives == 4
    assert state.coins == 42
    assert state.health == 255


def test_game_state_reads_the_fields_added_in_1_0_24():
    # levelNum/lives/coins/health were missing from the old 60 byte layout.
    state = GameState(encode_state(SAMPLE))
    assert (state.levelNum, state.lives, state.coins, state.health) == (11, 4, 42, 255)


def test_game_state_ignores_trailing_bytes():
    state = GameState(encode_state(SAMPLE) + b"\xaa" * 64)
    assert state.coins == 42


def test_game_state_has_won_tracks_num_stars():
    fields = list(SAMPLE)
    fields[0] = 0
    assert not GameState(encode_state(tuple(fields))).has_won()
    fields[0] = 1
    assert GameState(encode_state(tuple(fields))).has_won()


def test_game_state_equality_covers_every_field():
    assert GameState(encode_state(SAMPLE)) == GameState(encode_state(SAMPLE))
    other = list(SAMPLE)
    other[-1] = 254
    assert GameState(encode_state(SAMPLE)) != GameState(encode_state(tuple(other)))


def test_game_state_rejects_a_short_buffer():
    with pytest.raises(struct.error):
        GameState(b"\x00" * 60)


# ---------------------------------------------------------------
# RngConfig
# ---------------------------------------------------------------


def test_rng_config_default_matches_the_wasm_defaults():
    cfg = RngConfig.default()
    assert cfg.window_length == 100
    assert cfg.random_amount == 5
    assert cfg.random_burst_length == 2
    assert cfg.a_prob == 0.5
    assert cfg.b_prob == 0.5
    assert cfg.z_prob == 0.2


def test_rng_config_repr_lists_every_field():
    text = repr(RngConfig.default())
    for field in RngConfig.__slots__:
        assert field in text


# ---------------------------------------------------------------
# buttons
# ---------------------------------------------------------------


def test_button_masks_are_distinct_single_bits():
    seen = 0
    for mask in buttons.ALL:
        assert bin(mask).count("1") == 1, f"0x{mask:04x} is not a single bit"
        assert seen & mask == 0, f"0x{mask:04x} collides with an earlier mask"
        seen |= mask


def test_face_buttons_sit_in_the_high_byte():
    for mask in (
        buttons.A_BUTTON,
        buttons.B_BUTTON,
        buttons.Z_TRIG,
        buttons.START_BUTTON,
        buttons.U_JPAD,
        buttons.D_JPAD,
    ):
        assert mask & 0x00FF == 0, f"0x{mask:04x} sets a low byte bit"


def test_triggers_and_c_buttons_sit_in_the_low_byte():
    for mask in (
        buttons.L_TRIG,
        buttons.R_TRIG,
        buttons.U_CBUTTONS,
        buttons.L_CBUTTONS,
        buttons.R_CBUTTONS,
        buttons.D_CBUTTONS,
    ):
        assert mask >> 8 == 0, f"0x{mask:04x} should be in the low byte"