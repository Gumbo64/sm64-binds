use std::fmt;

#[derive(Debug)]
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

impl GameState {
    pub fn new(data: &[u8]) -> GameState {
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
            }}",
            self.num_stars,
            self.pos[0], self.pos[1], self.pos[2],
            self.vel[0], self.vel[1], self.vel[2],
            self.lakitu_pos[0], self.lakitu_pos[1], self.lakitu_pos[2],
            self.lakitu_yaw,
            self.in_credits,
            self.course_num,
            self.act_num,
            self.area_index,
            self.level_num
        )
    }
}
