use std::fmt;

/// Layout produced by the `get_game_state` WASM export.
///
/// Mirrors `struct GameState` in `sm64-port/src/pc/pc_main.c`: every scalar is
/// an `s32`, so the struct is 13 * 4 = 76 bytes with no padding.
pub const GAME_STATE_SIZE: usize = 76;

#[derive(Debug, Clone, PartialEq)]
pub struct GameState {
    pub num_stars: i32,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub lakitu_pos: [f32; 3],
    pub lakitu_yaw: i32,
    pub in_credits: i32,
    pub course_num: i32,
    pub act_num: i32,
    pub area_index: i32,
    pub level_num: i32,
    pub lives: i32,
    pub coins: i32,
    pub health: i32,
}

impl GameState {
    /// Decodes the 76 byte little endian struct written by the WASM export.
    ///
    /// # Panics
    ///
    /// Panics if `data` is shorter than [`GAME_STATE_SIZE`].
    pub fn new(data: &[u8]) -> GameState {
        assert!(
            data.len() >= GAME_STATE_SIZE,
            "game state needs {GAME_STATE_SIZE} bytes, got {}",
            data.len()
        );

        let mut offset = 0;
        GameState {
            num_stars: read_i32(data, &mut offset),
            pos: read_vec3(data, &mut offset),
            vel: read_vec3(data, &mut offset),
            lakitu_pos: read_vec3(data, &mut offset),
            lakitu_yaw: read_i32(data, &mut offset),
            in_credits: read_i32(data, &mut offset),
            course_num: read_i32(data, &mut offset),
            act_num: read_i32(data, &mut offset),
            area_index: read_i32(data, &mut offset),
            level_num: read_i32(data, &mut offset),
            lives: read_i32(data, &mut offset),
            coins: read_i32(data, &mut offset),
            health: read_i32(data, &mut offset),
        }
    }

    pub fn has_won(&self) -> bool {
        self.num_stars > 0
    }
}

fn read_i32(data: &[u8], offset: &mut usize) -> i32 {
    let value = i32::from_le_bytes(data[*offset..*offset + 4].try_into().unwrap());
    *offset += 4;
    value
}

fn read_vec3(data: &[u8], offset: &mut usize) -> [f32; 3] {
    let vec = [
        f32::from_le_bytes(data[*offset..*offset + 4].try_into().unwrap()),
        f32::from_le_bytes(data[*offset + 4..*offset + 8].try_into().unwrap()),
        f32::from_le_bytes(data[*offset + 8..*offset + 12].try_into().unwrap()),
    ];
    *offset += 12;
    vec
}

impl fmt::Display for GameState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GameState {{\n\
                numStars: {},\n\
                position: ({}, {}, {}),\n\
                velocity: ({}, {}, {}),\n\
                lakituPosition: ({}, {}, {}),\n\
                lakituYaw: {},\n\
                inCredits: {},\n\
                courseNum: {},\n\
                actNum: {},\n\
                areaIndex: {},\n\
                levelNum: {},\n\
                lives: {},\n\
                coins: {},\n\
                health: {},\n\
            }}",
            self.num_stars,
            self.pos[0],
            self.pos[1],
            self.pos[2],
            self.vel[0],
            self.vel[1],
            self.vel[2],
            self.lakitu_pos[0],
            self.lakitu_pos[1],
            self.lakitu_pos[2],
            self.lakitu_yaw,
            self.in_credits,
            self.course_num,
            self.act_num,
            self.area_index,
            self.level_num,
            self.lives,
            self.coins,
            self.health,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the little endian byte image the WASM export would write.
    fn encode(state: &GameState) -> Vec<u8> {
        fn push_i32(out: &mut Vec<u8>, v: i32) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        fn push_vec3(out: &mut Vec<u8>, v: &[f32; 3]) {
            for f in v {
                push_i32(out, f.to_bits() as i32);
            }
        }

        let mut out = Vec::with_capacity(GAME_STATE_SIZE);
        push_i32(&mut out, state.num_stars);
        push_vec3(&mut out, &state.pos);
        push_vec3(&mut out, &state.vel);
        push_vec3(&mut out, &state.lakitu_pos);
        push_i32(&mut out, state.lakitu_yaw);
        push_i32(&mut out, state.in_credits);
        push_i32(&mut out, state.course_num);
        push_i32(&mut out, state.act_num);
        push_i32(&mut out, state.area_index);
        push_i32(&mut out, state.level_num);
        push_i32(&mut out, state.lives);
        push_i32(&mut out, state.coins);
        push_i32(&mut out, state.health);
        out
    }

    fn sample() -> GameState {
        GameState {
            num_stars: 7,
            pos: [1.0, 2.0, 3.0],
            vel: [-1.5, 0.25, 100.0],
            lakitu_pos: [-64.0, 512.0, 0.5],
            lakitu_yaw: 4096,
            in_credits: 1,
            course_num: 5,
            act_num: 2,
            area_index: 3,
            level_num: 11,
            lives: 4,
            coins: 42,
            health: 255,
        }
    }

    #[test]
    fn struct_is_seventy_six_bytes() {
        // Every field is an s32 or three of them, so 13 * 4 with no padding.
        assert_eq!(GAME_STATE_SIZE, 76);
        assert_eq!(encode(&sample()).len(), GAME_STATE_SIZE);
    }

    #[test]
    fn round_trips_every_field() {
        let state = sample();
        assert_eq!(GameState::new(&encode(&state)), state);
    }

    #[test]
    fn reads_fields_at_their_documented_offsets() {
        let bytes = encode(&sample());

        assert_eq!(&bytes[0..4], &7i32.to_le_bytes(), "numStars");
        assert_eq!(
            GameState::new(&bytes).pos,
            [1.0, 2.0, 3.0],
            "pos occupies bytes 4..16"
        );
        assert_eq!(
            GameState::new(&bytes).level_num,
            11,
            "level_num sits after area_index"
        );
        assert_eq!(
            GameState::new(&bytes).health,
            255,
            "health is the last field"
        );
    }

    #[test]
    fn ignores_trailing_bytes() {
        // The memory view is larger than the struct, so a short read must work.
        let mut bytes = encode(&sample());
        bytes.extend_from_slice(&[0xAA; 64]);
        assert_eq!(GameState::new(&bytes), sample());
    }

    #[test]
    #[should_panic(expected = "game state needs 76 bytes")]
    fn rejects_short_buffers() {
        GameState::new(&[0u8; 60]);
    }

    #[test]
    fn has_won_tracks_num_stars() {
        let mut state = sample();
        state.num_stars = 0;
        assert!(!state.has_won());
        state.num_stars = 1;
        assert!(state.has_won());
    }

    #[test]
    fn display_includes_every_field() {
        let text = sample().to_string();
        for field in [
            "numStars",
            "lakituYaw",
            "inCredits",
            "courseNum",
            "actNum",
            "areaIndex",
            "levelNum",
            "lives",
            "coins",
            "health",
        ] {
            assert!(text.contains(field), "Display is missing {field}");
        }
    }
}
