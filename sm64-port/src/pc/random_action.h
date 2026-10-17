#include <stdint.h>
#ifndef HEADLESS_VERSION
#include <SDL2/SDL_stdinc.h>
#endif
#include <stdint.h>
#include <ultra64.h>

void set_rng_seed(uint32_t seed);
void set_rng_config(uint32_t window_length, uint32_t random_amount, uint32_t random_burst_length, float A_prob, float B_prob, float Z_prob);
int is_random_action();
void rng_update(uint32_t input);
OSContPad *rng_pad(uint16_t button, uint8_t stick_x, uint8_t stick_y);
