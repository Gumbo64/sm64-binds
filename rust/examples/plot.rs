//! Plots Mario's trajectory to an SVG file.
//!
//! Usage: `cargo run --release --example plot -- ../baserom.us.z64 out.svg`
//!
//! Written against the standard library only, so plotting does not pull a
//! plotting crate (and its system font libraries) into the test build.

use std::env;
use std::fs;
use std::process;

use sm64_binds::{GamePad, SM64GameGenerator, buttons};

/// Scales a game-space coordinate onto the SVG canvas.
struct Projection {
    centre: f32,
    scale: f32,
    width: f32,
    height: f32,
}

impl Projection {
    /// `world` is the half-width of the visible area in game units.
    fn new(points: &[[f32; 3]], size: f32, world: f32) -> Self {
        let sum_x: f32 = points.iter().map(|p| p[0]).sum();
        let sum_z: f32 = points.iter().map(|p| p[2]).sum();
        let n = points.len().max(1) as f32;
        Projection {
            centre: (sum_x / n + sum_z / n) / 2.0,
            scale: size / (2.0 * world),
            width: size,
            height: size,
        }
    }

    fn x(&self, value: f32) -> f32 {
        (value - self.centre) * self.scale + self.width / 2.0
    }

    fn y(&self, value: f32) -> f32 {
        self.height / 2.0 - (value - self.centre) * self.scale
    }
}

fn polyline(points: &[[f32; 3]], projection: &Projection, colour: &str, width: f32) -> String {
    let path: Vec<String> = points
        .iter()
        .map(|p| format!("{:.2},{:.2}", projection.x(p[0]), projection.y(p[2])))
        .collect();
    format!(
        r#"<polyline fill="none" stroke="{colour}" stroke-width="{width}" points="{}"/>"#,
        path.join(" ")
    )
}

fn svg(series: &[(&str, &[[f32; 3]])]) -> String {
    let mut out = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 800" width="800" height="800">"#,
    );
    out.push_str(r#"<rect width="800" height="800" fill="white"/>"#);

    let all: Vec<[f32; 3]> = series
        .iter()
        .flat_map(|(_, points)| points.iter().copied())
        .collect();
    let projection = Projection::new(&all, 800.0, 8000.0);

    for (colour, points) in series {
        if !points.is_empty() {
            out.push_str(&polyline(points, &projection, colour, 2.0));
        }
    }
    out.push_str("</svg>");
    out
}

fn main() {
    let mut args = env::args().skip(1);
    let rom_path = args.next().unwrap_or_else(|| "../baserom.us.z64".into());
    let out_path = args.next().unwrap_or_else(|| "output.svg".into());

    let rom = match fs::read(&rom_path) {
        Ok(rom) => rom,
        Err(err) => {
            eprintln!("could not read ROM {rom_path}: {err}");
            process::exit(1);
        }
    };
    let generator = match SM64GameGenerator::new(rom) {
        Ok(generator) => generator,
        Err(err) => {
            eprintln!("{err}");
            process::exit(1);
        }
    };

    let mut game = generator.create_game().expect("could not create the game");
    let mut clone = generator.create_game().expect("could not create the clone");

    let mut original: Vec<[f32; 3]> = Vec::new();
    let mut forked: Vec<[f32; 3]> = Vec::new();

    // Boot the attract demo, then branch the clone so the plot shows the
    // divergence between the two.
    for i in 0..1200 {
        let mut pad = GamePad::default();
        if i % 17 == 0 {
            pad.button = buttons::START_BUTTON;
        }
        game.step_game(pad).expect("step_game failed");
        original.push(game.get_game_state().expect("state failed").pos);
    }

    clone.copy_from(&game).expect("copy_from failed");
    for _ in 0..800 {
        clone
            .step_game(GamePad::new(0, -80, 80))
            .expect("step_game failed");
        forked.push(clone.get_game_state().expect("state failed").pos);
    }

    if let Err(err) = fs::write(&out_path, svg(&[("blue", &original), ("red", &forked)])) {
        eprintln!("could not write {out_path}: {err}");
        process::exit(1);
    }
    println!(
        "wrote {out_path} ({} and {} points)",
        original.len(),
        forked.len()
    );
}
