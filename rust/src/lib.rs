mod sm64;
pub use sm64::{SM64GameGenerator, SM64Game, GamePad, GameState, RandomConfig};
pub mod buttons;


#[cfg(test)]
mod tests {
    use wasmtime::Error;
    use crate::{SM64GameGenerator, buttons, GamePad};
    #[test]
    fn main() -> Result<(), Error> {
        // let game_gen = SM64GameGenerator::from_rom_bytes(include_bytes!("../../baserom.us.z64").to_vec())?;
        let game_gen = SM64GameGenerator::new("../baserom.us.z64")?;


        let mut game = game_gen.create_game()?;

        let mut i = 0;
        while i < 2000 {

            let mut button: u16 = 0;
            let stick_x: i8 = 0;
            let mut stick_y: i8 = 0;

            if (150 < i && i < 160) || (200 < i && i < 300) {
                button = buttons::START_BUTTON;
            }

            if i > 300 {
                stick_y = 80;
            }

            if i % 2 == 0 {
                button = buttons::A_BUTTON;
            }

            let pad = GamePad::new(button, stick_x, stick_y);
            game.step_game(pad)?;


            let state = game.get_game_state()?;
            println!("{}\n", state.to_string());
            i += 1;
        }

        Ok(())
    }
}
