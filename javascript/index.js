import FileUpload from "./src/components/FileUpload";
import { GameConfig, RngConfig } from "./src/scripts/sm64/configs";
import { Sm64VisualEngine, GamepadButtons, GamePad, GameState } from "./src/scripts/sm64/Sm64Game";
import { getROM } from "./src/scripts/fileUpload";

// for playback
const MIN_SPEED = 2;
const MAX_SPEED = 1000;

async function sm64_test(canvas) {
    let [engine, success] = await sm64_playback(canvas);

    let speed = 1;
    function get_speed() {return speed}

    let i = 0;
    function step() {

        // true controller pad, other pad can change via RNG
        let pad = new GamePad(0,0,0);
        if ( (150 < i && i < 160) || (200 < i && i < 300) ) {
            pad.button = GamepadButtons.START_BUTTON;
        }
        if (i > 300) {
            pad.stick_y = 80;
        }
        if (i % 2 == 0) {
            pad.button = GamepadButtons.A_BUTTON;
        }
        
        engine.step_game(pad);

        let state = engine.get_game_state();
        console.log(state.pos)
        let c_pad = engine.get_controller_pad();
        if (c_pad.is_pressed(GamepadButtons.U_JPAD)) {
            return true;
        }

        if (c_pad.is_pressed(GamepadButtons.D_JPAD)) {
            speed = 10;
            engine.set_audio_enabled(0);
        } else {
            speed = 1;
            engine.set_audio_enabled(1);
        }
        i += 1;

        return false;
    }
    
    await iter_with_anim_frame(step, get_speed);

    return [success, solution];

}
async function iter_with_anim_frame(func, get_speed, kill_signal= async () => {false}) {
    let done = false;
    let killed = false;

    let target_time = 0;
    const loop = async (time) => {
        time *= 0.03; // milliseconds to frame count (33.333 ms -> 1)
        if (time >= target_time + 100.0) {
            // We are lagging 100 frames behind, probably due to coming back after inactivity,
            // so reset, with a small margin to avoid potential jitter later.
            target_time = time - 0.010;
        }
        while (time >= target_time + 1.0/get_speed()) {
            done = func();
            killed = await kill_signal();
            if (done || killed) return;
            target_time = target_time + 1.0/get_speed();
        }
        requestAnimationFrame(loop);
    }
    requestAnimationFrame(loop);
    

    return new Promise((resolve, reject) => {
        const checkEndCondition = async () => {
            if (killed) {                
                reject();
            } else if (done) {
                resolve();
            } else {
                setTimeout(checkEndCondition, 100); // Polling every 100ms to check end conditions
            }
        };
        checkEndCondition();
    });
}

async function sm64_evaluate(canvas, solution = [], seed, game_config = GameConfig.defaultConfig(), kill_signal = async () => {false}, has_won = (state) => {false}) {
    let [_, success] = sm64_playback(canvas, solution, false, seed, game_config, MIN_SPEED, MAX_SPEED, kill_signal, has_won);
    return success;
}

async function sm64_playback(canvas, inputs, controllable=false, seed=NaN, game_config = GameConfig.defaultConfig(), min_speed=MIN_SPEED, max_speed=MAX_SPEED, kill_signal = async () => {false}, has_won = (state) => {false}, engine=null) {
    let success = false;
    
    if (engine === null) {
        canvas.id = "canvas";
        engine = await Sm64VisualEngine.create(canvas, seed, game_config.rng_config);
        canvas.id = "";
    }


    let speed = max_speed;
    function get_speed() {return speed}
    if (speed != 1) engine.set_audio_enabled(0);

    // Loop through all frames
    let i = 0;
    function step() {
        if (i >= inputs.length) {
            success = true; 
            return [engine, success];
        }
        let pad = inputs[i];
        i+=1;

        // assert that the playback matches the seed
        let r_pad = engine.rng_pad(pad);
        if (game_config.has_rng() && !r_pad.equals(pad)) {
            throw new Error("Replay inputs do not match seed")
        }
        
        // playback
        engine.step_game(pad);

        // check if won
        let state = engine.get_game_state();
        if (has_won(state)) {
            success = true;
            return [engine, success];
        }

        if (controllable) {
            // allow special inputs
            let c_pad = engine.get_controller_pad();
            if (c_pad.is_pressed(GamepadButtons.START_BUTTON)) {
                inputs.length = i;
                success = true;
                return [engine, success];
            }
            let cut_amount = 10 * min_speed * 30 // 10 seconds before
            if (i > inputs.length - cut_amount) {
                speed = min_speed;
            }
        }
        success = false
        return [engine, success];
    }
    
    await iter_with_anim_frame(step, get_speed, kill_signal);
    
    engine.set_audio_enabled(1);
    return [engine, success];
}


async function record_single(canvas, starting_solution = [], seed=NaN, game_config = GameConfig.defaultConfig(), kill_signal = async () => {false}, has_won = (state) => {false}) {
    let [engine, success] = await sm64_playback(canvas, starting_solution, true, seed, game_config, MIN_SPEED, MAX_SPEED, kill_signal, has_won);
    if (success) return [success, solution];

    let solution = Array.from(starting_solution); // shallow copy, use the same pad objects but different array

    let cancel_start_press = true;

    let speed = 1;
    function get_speed() {return speed}

    // Loop through all frames
    let i = starting_solution.length;
    function step() {
        if (game_config.has_max_solution_time() && i >= game_config.max_solution_time) return true;
        i+=1;

        let c_pad = engine.get_controller_pad();

        // don't press start again after interrupting playback, unless it has released for at least one frame
        if (cancel_start_press) {
            if (c_pad.is_pressed(GamepadButtons.START_BUTTON)) {
                c_pad.disable_button(GamepadButtons.START_BUTTON)
            } else {
                cancel_start_press = false;
            }
        }

        // true controller pad, other pad can change via RNG

        let pad = c_pad.clone();
        if (game_config.has_rng()) pad = engine.rng_pad(pad);
        
        engine.step_game(pad);
        solution.push(pad);

        let state = engine.get_game_state();
        // console.log(state.toString());

        // check if won
        if (has_won(state)) {
            success = true;
            return true;
        }

        if (c_pad.is_pressed(GamepadButtons.U_JPAD)) {
            return true;
        }

        if (c_pad.is_pressed(GamepadButtons.D_JPAD)) {
            speed = 10;
            engine.set_audio_enabled(0);
        } else {
            speed = 1;
            engine.set_audio_enabled(1);
        }
        return false;
    }
    
    await iter_with_anim_frame(step, get_speed, kill_signal);

    return [success, solution];

}

async function sm64_record(canvas, seed=NaN, game_config = GameConfig.defaultConfig(), kill_signal = async () => {false}, has_won = (state) => {false}) {
    let success = false;
    let starting_solution = [];

    while (!success) {
        [success, starting_solution] = await record_single(canvas, starting_solution, seed, game_config, kill_signal, has_won);
    }
    return starting_solution;
}

export { sm64_record, sm64_evaluate, sm64_playback, sm64_test, GameConfig, RngConfig, GamePad, GamepadButtons, GameState, FileUpload, getROM };