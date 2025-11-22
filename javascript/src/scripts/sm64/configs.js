class RngConfig {
    constructor(window_length, random_amount, random_burst_length, A_prob, B_prob, Z_prob) {
        this.window_length = window_length;
        this.random_amount = random_amount;
        this.random_burst_length = random_burst_length;
        this.A_prob = A_prob;
        this.B_prob = B_prob;
        this.Z_prob = Z_prob;
    }
}

class GameConfig {
    constructor(max_solution_time, random_config) {
        this.max_solution_time = max_solution_time;
        this.random_config = random_config;
    }

    static defaultConfig() {
        return new GameConfig(NaN, null);
    }

    has_max_solution_time() {
        return !isNaN(this.max_solution_time);
    }

    has_rng() {
        return this.random_config !== null;
    }
}

export { GameConfig, RngConfig };
