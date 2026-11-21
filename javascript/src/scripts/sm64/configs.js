class GameConfig {
    constructor(max_solution_time) {
        this.max_solution_time = max_solution_time;
    }

    static defaultConfig() {
        return new GameConfig(NaN);
    }

    has_max_solution_time() {
        return !isNaN(this.max_solution_time);
    }
}

export { GameConfig };
