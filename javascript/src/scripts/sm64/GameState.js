// A snapshot of the game, mirroring `struct GameState` in
// sm64-port/src/pc/pc_main.c. Every scalar there is an s32, so the struct is
// 13 * 4 = 76 bytes with no padding.

/** Byte layout of the `get_game_state` export. */
export const GAME_STATE_SIZE = 76;

export class GameState {
    /**
     * Reads the 76 byte little endian struct the WASM export writes.
     *
     * `engine` only has to provide `mem_view(pointer, length)`; in the browser
     * that is backed by the WebAssembly.Memory buffer.
     */
    constructor(pointer, engine) {
        const view = engine.mem_view(pointer, GAME_STATE_SIZE);

        let offset = 0;
        const readI32 = () => {
            const value = view.getInt32(offset, true);
            offset += 4;
            return value;
        };
        const readVec3 = () => {
            const vec = [
                view.getFloat32(offset, true),
                view.getFloat32(offset + 4, true),
                view.getFloat32(offset + 8, true),
            ];
            offset += 12;
            return vec;
        };

        this.numStars = readI32();
        this.pos = readVec3();
        this.vel = readVec3();
        this.lakituPos = readVec3();
        this.lakituYaw = readI32();
        this.inCredits = readI32();
        this.courseNum = readI32();
        this.actNum = readI32();
        this.areaIndex = readI32();
        this.levelNum = readI32();
        this.lives = readI32();
        this.coins = readI32();
        this.health = readI32();
    }

    has_won() {
        return this.numStars > 0;
    }

    toString() {
        return `GameState {
            numStars: ${this.numStars},
            position: (${this.pos.join(", ")}),
            velocity: (${this.vel.join(", ")}),
            lakituPosition: (${this.lakituPos.join(", ")}),
            lakituYaw: ${this.lakituYaw},
            inCredits: ${this.inCredits},
            courseNum: ${this.courseNum},
            actNum: ${this.actNum},
            areaIndex: ${this.areaIndex},
            levelNum: ${this.levelNum},
            lives: ${this.lives},
            coins: ${this.coins},
            health: ${this.health}
        }`;
    }
}