#include "random_action.h"
#include <stdint.h>

typedef struct {
    uint32_t window_length;
    uint32_t random_amount;
    uint32_t random_burst_length;
    float A_prob;
    float B_prob;
    float Z_prob;
} RngConfig;

static RngConfig rng = {
     100, 5, 2, 0.5, 0.5, 0.2
};

static uint32_t random_seed = 22;

uint32_t rng_next() {
    // this is xorshift32
    random_seed ^= (random_seed << 13);
    random_seed ^= (random_seed >> 17);
    random_seed ^= (random_seed << 5);
    return random_seed;
}

float rng_next_prob() {
    // between 0 and 1 
    return rng_next() / (float)UINT32_MAX;
}
int8_t rng_next_stick() {
    // between -80 and 80 inclusive
    return (int8_t)(rng_next() % 161) - 80;
}

OSContPad *rng_pad(uint16_t button, uint8_t stick_x, uint8_t stick_y) {
    static OSContPad pad = {0};
    pad.errnum = 0;

    if (!is_random_action()) {
        pad.button = button;
        pad.stick_x = stick_x;
        pad.stick_y = stick_y;
        pad.errnum = 1;

        return &pad;
    }

    pad.button = 0;
    pad.button |= A_BUTTON * (rng_next_prob() < rng.A_prob);
    pad.button |= B_BUTTON * (rng_next_prob() < rng.B_prob);
    pad.button |= Z_TRIG * (rng_next_prob() < rng.Z_prob);

    pad.stick_x = rng_next_stick();
    pad.stick_y = rng_next_stick();

    return &pad;
}

void rng_update(uint32_t input) {
    random_seed = rng_next() ^ input;
    random_seed = rng_next();
}

void set_rng_seed(uint32_t seed) {
    random_seed = seed;
}

void set_rng_config(uint32_t window_length, uint32_t random_amount, uint32_t random_burst_length, float A_prob, float B_prob, float Z_prob) {
    rng.window_length = window_length;
    rng.random_amount = random_amount;
    rng.random_burst_length = random_burst_length;
    rng.A_prob = A_prob;
    rng.B_prob = B_prob;
    rng.Z_prob = Z_prob;
}

int is_random_action() {
    static uint32_t cur_window_length = 0;
    static uint32_t cur_random_amount = 0;
    static uint32_t cur_burst_count = 0;

    if (cur_burst_count > 0) {
        cur_burst_count -= 1;
        return 1;
    }

    if (cur_window_length == 0) {
        // Reset the window length and random action amount
        cur_window_length = rng.window_length;
        cur_random_amount = rng.random_amount;
    }

    double prob_random = (double)cur_random_amount / (double)cur_window_length;
    float r = rng_next_prob();
    int is_random = r < prob_random;
    if (is_random) {
        cur_random_amount -= 1;
        cur_burst_count = rng.random_burst_length - 1; // one is used in this function call
    }
    
    cur_window_length -= 1;

    return is_random;
}