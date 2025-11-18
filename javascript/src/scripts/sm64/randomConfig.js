function make_config(max_name_length, max_solution_time, max_window_length, max_random_action, A_prob, B_prob, Z_prob) {
    return {
        "max_name_length": max_name_length,
        "max_solution_time": max_solution_time,
        "max_window_length": max_window_length,
        "max_random_action": max_random_action,
        "A_prob": A_prob,
        "B_prob": B_prob,
        "Z_prob": Z_prob,
    }
}

export {make_config};