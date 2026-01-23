mod sm64;
pub use sm64::{SM64GameGenerator, SM64Game, GamePad, GameState, RngConfig};
pub mod buttons;


#[cfg(test)]
mod tests {
    use anyhow::Error;
    use crate::{GamePad, SM64GameGenerator, buttons};
    use plotters::prelude::*;

    fn visualize_positions(positions1: &Vec<[f32; 3]>, positions2: &Vec<[f32; 3]>) -> Result<(), Box<dyn std::error::Error>> {
        
        // Create a drawing area
        let root = BitMapBackend::new("output.png", (640, 480)).into_drawing_area();
        root.fill(&WHITE).unwrap();

        // Define a coordinate system
        let mut chart = ChartBuilder::on(&root)
            .caption("Two Datasets Plot", ("sans-serif", 50).into_font())
            .margin(5)
            .x_label_area_size(30)
            .y_label_area_size(30)
            .build_cartesian_2d(-8000.0..8000.0, -8000.0..8000.0) // Adjust limits as needed
            .unwrap();

        // Draw the axes
        chart.configure_mesh().draw().unwrap();

        // Plot the first dataset
        let series1: Vec<(f64, f64)> = positions1.iter().map(|&[x, _, y]| (x.into(), y.into())).collect();
        chart.draw_series(LineSeries::new(series1, &RED)).unwrap()
            .label("Dataset 1")
            .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &RED));

        // Plot the second dataset
        let series2: Vec<(f64, f64)> = positions2.iter().map(|&[x, _, y]| (x.into(), y.into())).collect();
        chart.draw_series(LineSeries::new(series2, &BLUE)).unwrap()
            .label("Dataset 2")
            .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &BLUE));

        // Draw the legend
        chart.configure_series_labels().border_style(&BLACK).draw().unwrap();
        Ok(())
    }

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
        // let cfg = RngConfig::default();

        // game.set_rng_config(cfg)?;
        game.set_rng_seed(seed)?;

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
    #[test]
    fn mem_test() -> Result<(), Box<dyn std::error::Error>> {
        // Load ROM and initialize game
        let game_gen = SM64GameGenerator::new(include_bytes!("../../baserom.us.z64").to_vec())?;

        let mut game = game_gen.create_game()?;
        let mut game2 = game_gen.create_game()?;

        let mut positions_list = Vec::new();
        let mut positions_clone_list = Vec::new();

        // Gameplay loop
        for i in 0..3000 { // Iterate through the gameplay logic            
            // Override logic similar to your Python script
            let mut button = 0;
            let mut sticky = 0;

            if (150 < i && i < 160) || (200 < i && i < 300) {
                button = buttons::START_BUTTON;
            }

            if i > 300 {
                sticky = 80;
            }

            if i % 2 == 0 {
                button = buttons::A_BUTTON; // Press A
            }

            let pad = GamePad::new(button, 0, sticky);

            if i == 1760 {
                game_gen.copy_to(&game, &mut game2)?; // Copy state
            }

            if i >= 1760 {
                
                let mut button2 = 0;
                if button2 % 8 == 0 {
                    button2 = buttons::A_BUTTON;
                }
                let pad2 = GamePad::new(button2, 40, 80);
                game2.step_game(pad2)?;
                let state2 = game2.get_game_state()?;
                positions_clone_list.push(state2.pos);
            }

            // Step game logic
            game.step_game(pad)?;

            // Read and store state
            let state = game.get_game_state()?;
            positions_list.push(state.pos);
        }

        // Visualization logic: here you'd implement the equivalent to create_3d_rotation_gif_two_colors.
        visualize_positions(&positions_list, &positions_clone_list)?;
        Ok(())
    }
}
