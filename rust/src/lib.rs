mod sm64;
pub use sm64::{SM64GameGenerator, SM64Game, GamePad, GameState, RandomConfig};
pub mod buttons;


#[cfg(test)]
mod tests {
    use anyhow::Error;
    use crate::{GamePad, RandomConfig, SM64GameGenerator, buttons};
    #[test]
    fn no_rng() -> Result<(), Error> {
        let game_gen = SM64GameGenerator::new(include_bytes!("../../baserom.us.z64").to_vec())?;
        // let game_gen = SM64GameGenerator::from_file("../baserom.us.z64")?;
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
    #[test]
    fn yes_rng() -> Result<(), Error> {
        let game_gen = SM64GameGenerator::new(include_bytes!("../../baserom.us.z64").to_vec())?;
        // let game_gen = SM64GameGenerator::from_file("../baserom.us.z64")?;
        let mut game = game_gen.create_game()?;

        let seed = 22;
        let cfg = RandomConfig::default();

        game.rng_init(seed, cfg)?;

        let mut i = 0;
        while i < 2000 {

            let mut button: u16 = 0;
            let stick_x: i8 = 0;
            let stick_y: i8 = 0;

            if (150 < i && i < 160) || (200 < i && i < 300) {
                button = buttons::START_BUTTON;
            }


            let mut pad = GamePad::new(button, stick_x, stick_y);
            pad = game.rng_pad(pad)?;
            game.step_game(pad)?;


            // let state = game.get_game_state()?;
            // println!("{}\n", state.to_string());
            print!("{:#?}\n", pad);
            i += 1;
        }

        Ok(())
    }
}
