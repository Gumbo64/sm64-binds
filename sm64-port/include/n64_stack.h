#ifndef N64_STACK_H
#define N64_STACK_H

#include <PR/ultratypes.h>

// A model of the game thread's stack pointer on the N64 (US), for the places
// where the console's behaviour depends on what is left on its stack: the JRB
// sliding box reads an uninitialized local, and nop_update_water_camera returns
// nothing. Every function that can be on the stack at those places starts with
// N64_FRAME(size), the size of its stack frame in the US ROM. The sizes come
// from sm64-binds/oracle/n64stack, which reads them from an IDO build.
// The render walk (geo_process_*, shadows) is left out to save time: the model
// is off while it runs, so its floor checks are not recorded and a camera mode
// set by the camera update gets a wrong nop_update_water_camera value. Neither
// changed the game in any TAS or AI run.
#if defined(VERSION_US) && !defined(TARGET_N64)
#define N64_STACK_MODEL
#define N64_STACK_BOTTOM 0x80204E00 // gThread5Stack
#define N64_STACK_TOP 0x80206E00
#define N64_GAME_LOOP_SP 0x80206DC8 // in thread5_game_loop: top - 16 - its 40-byte frame

extern u32 gN64Sp;
void n64_stack_store(u32 addr, s32 value);
s32 n64_stack_load(u32 addr);

static inline void n64_frame_pop(u32 *size) {
    gN64Sp += *size;
}
#define N64_FRAME(size) \
    __attribute__((cleanup(n64_frame_pop))) u32 n64Frame = (gN64Sp -= (size), (size))

// Turns the model off until the end of the block: stores then miss the stack.
static inline u32 n64_stack_off(void) {
    u32 sp = gN64Sp;
    gN64Sp = 0;
    return sp;
}
static inline void n64_stack_restore(u32 *sp) {
    gN64Sp = *sp;
}
#define N64_STACK_OFF() __attribute__((cleanup(n64_stack_restore))) u32 n64Saved = n64_stack_off()
#else
#define N64_FRAME(size) struct N64FrameUnused
#define N64_STACK_OFF() struct N64FrameUnused
#endif

#endif // N64_STACK_H
