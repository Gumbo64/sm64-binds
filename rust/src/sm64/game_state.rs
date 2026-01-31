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
}

impl GameState {
    pub fn new(data: &[u8]) -> GameState {
        let mut offset = 0;

        let num_stars = i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        offset += 4;

        let pos = [
            f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()),
            f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()),
            f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()),
        ];
        offset += 12;

        let vel = [
            f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()),
            f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()),
            f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()),
        ];
        offset += 12;

        let lakitu_pos = [
            f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()),
            f32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()),
            f32::from_le_bytes(data[offset + 8..offset + 12].try_into().unwrap()),
        ];
        offset += 12;

        let lakitu_yaw = i32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;

        let in_credits = i32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4; 

        let course_num = i32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;

        let act_num = i32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;

        let area_index = i32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        offset += 4;
        let level_num = i32::from_le_bytes(data[offset..offset+4].try_into().unwrap());
        // offset += 4;

        // Construct GameState
        GameState {
            num_stars,
            pos,
            vel,
            lakitu_pos,
            lakitu_yaw,
            in_credits,
            course_num,
            act_num,
            area_index,
            level_num
        }
    }

    pub fn has_won(&self) -> bool {
        self.num_stars > 0
    }

    pub fn to_string(&self) -> String {
        format!(
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