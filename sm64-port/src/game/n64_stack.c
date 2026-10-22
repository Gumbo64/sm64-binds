#include "n64_stack.h"

#ifdef N64_STACK_MODEL
u32 gN64Sp;

// The game thread's stack, holding only the values the model writes.
static s32 sN64StackWords[(N64_STACK_TOP - N64_STACK_BOTTOM) / 4];

void n64_stack_store(u32 addr, s32 value) {
    if (addr - N64_STACK_BOTTOM < sizeof(sN64StackWords)) {
        sN64StackWords[(addr - N64_STACK_BOTTOM) / 4] = value;
    }
}

s32 n64_stack_load(u32 addr) {
    if (addr - N64_STACK_BOTTOM < sizeof(sN64StackWords)) {
        return sN64StackWords[(addr - N64_STACK_BOTTOM) / 4];
    }
    return 0;
}
#endif
