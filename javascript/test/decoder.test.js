// Tests for the pure controller/state decoders.
//
// These run under `node --test` with no browser or ROM: both classes only need
// an object exposing `mem_view(pointer, length)`, which the fake below fakes
// with a plain ArrayBuffer.

import assert from "node:assert/strict";
import { test } from "node:test";

import { GamePad, GamepadButtons, GAME_PAD_SIZE } from "../src/scripts/sm64/GamePad.js";
import { GameState, GAME_STATE_SIZE } from "../src/scripts/sm64/GameState.js";

/** Stands in for Sm64VisualEngine: exposes linear memory as a DataView. */
class FakeEngine {
    constructor(buffer) {
        this.buffer = buffer;
    }

    mem_view(pointer, length) {
        return new DataView(this.buffer, pointer, length);
    }
}

function pad_bytes(button, stick_x, stick_y, errnum = 0) {
    return Uint8Array.of(
        button & 0xff, (button >> 8) & 0xff,
        stick_x & 0xff, stick_y & 0xff, errnum,
    );
}

// ---------------------------------------------------------------
// GamePad
// ---------------------------------------------------------------

test("game pad layout is four bytes", () => {
    assert.equal(GAME_PAD_SIZE, 4);
});

test("from_pointer decodes little endian fields", () => {
    const bytes = pad_bytes(GamepadButtons.A_BUTTON, 80, -80);
    const engine = new FakeEngine(bytes.buffer);
    const pad = GamePad.from_pointer(0, engine);

    assert.equal(pad.button, GamepadButtons.A_BUTTON);
    assert.equal(pad.stick_x, 80);
    assert.equal(pad.stick_y, -80);
});

test("from_pointer honours the pointer offset", () => {
    const bytes = new Uint8Array(16);
    bytes.set(pad_bytes(GamepadButtons.Z_TRIG, -80, 40), 8);
    const engine = new FakeEngine(bytes.buffer);

    const pad = GamePad.from_pointer(8, engine);
    assert.ok(pad.equals(new GamePad(GamepadButtons.Z_TRIG, -80, 40)));
});

test("a new pad is neutral", () => {
    const pad = new GamePad();
    assert.equal(pad.button, 0);
    assert.equal(pad.stick_x, 0);
    assert.equal(pad.stick_y, 0);
});

test("equals compares all three fields", () => {
    const pad = new GamePad(GamepadButtons.A_BUTTON, 1, 2);
    assert.ok(pad.equals(new GamePad(GamepadButtons.A_BUTTON, 1, 2)));
    assert.ok(!pad.equals(new GamePad(GamepadButtons.B_BUTTON, 1, 2)));
    assert.ok(!pad.equals(new GamePad(GamepadButtons.A_BUTTON, 9, 2)));
    assert.ok(!pad.equals(new GamePad(GamepadButtons.A_BUTTON, 1, 9)));
    assert.ok(!pad.equals({ button: 0, stick_x: 0, stick_y: 0 }));
});

test("is_pressed only fires for held bits", () => {
    const pad = new GamePad(GamepadButtons.A_BUTTON, 0, 0);
    assert.ok(pad.is_pressed(GamepadButtons.A_BUTTON));
    assert.ok(!pad.is_pressed(GamepadButtons.B_BUTTON));
});

test("is_pressed works for every single-bit mask", () => {
    for (const mask of Object.values(GamepadButtons)) {
        const pad = new GamePad();
        pad.enable_button(mask);
        assert.ok(pad.is_pressed(mask), `0x${mask.toString(16)} should be pressed`);
    }
});

test("enable_button and disable_button round trip", () => {
    const pad = new GamePad();
    pad.enable_button(GamepadButtons.A_BUTTON);
    pad.enable_button(GamepadButtons.B_BUTTON);
    assert.equal(pad.button, GamepadButtons.A_BUTTON | GamepadButtons.B_BUTTON);

    pad.disable_button(GamepadButtons.A_BUTTON);
    assert.equal(pad.button, GamepadButtons.B_BUTTON);

    pad.disable_button(GamepadButtons.B_BUTTON);
    assert.equal(pad.button, 0);
});

test("clone copies all three fields", () => {
    const pad = new GamePad(GamepadButtons.START_BUTTON, -3, 4);
    const copy = pad.clone();
    assert.ok(pad.equals(copy));
    copy.enable_button(GamepadButtons.A_BUTTON);
    assert.ok(!pad.equals(copy), "the clone must be independent");
});

test("button masks are distinct single bits", () => {
    let seen = 0;
    for (const mask of Object.values(GamepadButtons)) {
        const bits = mask.toString(2).split("1").length - 1;
        assert.equal(bits, 1, `0x${mask.toString(16)} is not a single bit`);
        assert.equal(seen & mask, 0, `0x${mask.toString(16)} collides`);
        seen |= mask;
    }
});

// ---------------------------------------------------------------
// GameState
// ---------------------------------------------------------------

/** Builds the byte image the WASM export writes for a given state. */
function state_bytes(fields) {
    const buffer = new ArrayBuffer(GAME_STATE_SIZE);
    const view = new DataView(buffer);
    let offset = 0;

    const writeI32 = (v) => { view.setInt32(offset, v, true); offset += 4; };
    const writeVec3 = (v) => {
        for (const f of v) { view.setFloat32(offset, f, true); offset += 4; }
    };

    writeI32(fields.numStars);
    writeVec3(fields.pos);
    writeVec3(fields.vel);
    writeVec3(fields.lakituPos);
    writeI32(fields.lakituYaw);
    writeI32(fields.inCredits);
    writeI32(fields.courseNum);
    writeI32(fields.actNum);
    writeI32(fields.areaIndex);
    writeI32(fields.levelNum);
    writeI32(fields.lives);
    writeI32(fields.coins);
    writeI32(fields.health);

    return buffer;
}

const SAMPLE = {
    numStars: 7,
    pos: [1.0, 2.0, 3.0],
    vel: [-1.5, 0.25, 100.0],
    lakituPos: [-64.0, 512.0, 0.5],
    lakituYaw: 4096,
    inCredits: 1,
    courseNum: 5,
    actNum: 2,
    areaIndex: 3,
    levelNum: 11,
    lives: 4,
    coins: 42,
    health: 255,
};

function parse(fields) {
    return new GameState(0, new FakeEngine(state_bytes(fields)));
}

test("game state layout is 76 bytes", () => {
    // 10 s32 scalars plus three 3-float vectors, with no padding.
    assert.equal(GAME_STATE_SIZE, 76);
});

test("reads every field", () => {
    const state = parse(SAMPLE);
    assert.equal(state.numStars, 7);
    assert.deepEqual(state.pos, [1.0, 2.0, 3.0]);
    assert.deepEqual(state.vel, [-1.5, 0.25, 100.0]);
    assert.deepEqual(state.lakituPos, [-64.0, 512.0, 0.5]);
    assert.equal(state.lakituYaw, 4096);
    assert.equal(state.inCredits, 1);
    assert.equal(state.courseNum, 5);
    assert.equal(state.actNum, 2);
    assert.equal(state.areaIndex, 3);
    assert.equal(state.levelNum, 11);
    assert.equal(state.lives, 4);
    assert.equal(state.coins, 42);
    assert.equal(state.health, 255);
});

test("reads the fields added in 1.0.24", () => {
    // levelNum/lives/coins/health were absent from the old 64 byte read.
    const state = parse(SAMPLE);
    assert.equal(state.levelNum, 11);
    assert.equal(state.lives, 4);
    assert.equal(state.coins, 42);
    assert.equal(state.health, 255);
});

test("honours the pointer offset", () => {
    // The WASM export returns a pointer into linear memory, so the struct is
    // not guaranteed to sit at offset 0.
    const struct = state_bytes(SAMPLE);
    const buffer = new Uint8Array(struct.byteLength + 64);
    buffer.set(new Uint8Array(struct), 64);

    const state = new GameState(64, new FakeEngine(buffer.buffer));
    assert.equal(state.coins, 42);
    assert.equal(state.health, 255);
    assert.equal(state.levelNum, 11);
});

test("has_won tracks numStars", () => {
    assert.ok(!parse({ ...SAMPLE, numStars: 0 }).has_won());
    assert.ok(parse({ ...SAMPLE, numStars: 1 }).has_won());
});

test("toString includes every field", () => {
    const text = parse(SAMPLE).toString();
    for (const field of [
        "numStars", "lakituYaw", "inCredits", "courseNum",
        "actNum", "areaIndex", "levelNum", "lives", "coins", "health",
    ]) {
        assert.ok(text.includes(field), `toString is missing ${field}`);
    }
});
// ---------------------------------------------------------------
// XOR (runs once per ROM upload, on the 6 MB shipped payload)
// ---------------------------------------------------------------

import { xorBytes } from "../src/scripts/xor.js";

test("xor with a single byte key", () => {
    assert.deepEqual(xorBytes(Uint8Array.of(0, 1, 2, 255), Uint8Array.of(0xaa)),
        Uint8Array.of(0xaa, 0xab, 0xa8, 0x55));
});

test("xor repeats a shorter key", () => {
    assert.deepEqual(xorBytes(Uint8Array.of(1, 2, 3, 4, 5), Uint8Array.of(0xff, 0)),
        Uint8Array.of(0xfe, 0x02, 0xfc, 0x04, 0xfa));
});

test("xor uses only the key prefix when the key is longer", () => {
    assert.deepEqual(xorBytes(Uint8Array.of(1, 2), Uint8Array.of(9, 8, 7, 6)),
        Uint8Array.of(1 ^ 9, 2 ^ 8));
});

test("xor with empty input", () => {
    assert.deepEqual(xorBytes(new Uint8Array(0), Uint8Array.of(1, 2, 3)), new Uint8Array(0));
});

test("xor is its own inverse", () => {
    const data = Uint8Array.from({ length: 300 }, (_, i) => i % 256);
    const key = Uint8Array.from([0xde, 0xad, 0xbe, 0xef]);
    assert.deepEqual(xorBytes(xorBytes(data, key), key), data);
});

test("xor rejects an empty key", () => {
    assert.throws(() => xorBytes(Uint8Array.of(1), new Uint8Array(0)));
});

test("xor matches a naive byte-by-byte reference", () => {
    // Guards the chunked implementation against an off-by-one at a key boundary.
    const data = Uint8Array.from({ length: 1000 }, (_, i) => (i * 31) % 256);
    for (const keyLength of [1, 2, 3, 7, 8, 17, 256]) {
        const key = Uint8Array.from({ length: keyLength }, (_, i) => i * 5);
        const expected = Uint8Array.from(data, (b, i) => b ^ key[i % keyLength]);
        assert.deepEqual(xorBytes(data, key), expected, `keyLength=${keyLength}`);
    }
});
