#ifndef IDO_CONVERT_H
#define IDO_CONVERT_H

#include <PR/ultratypes.h>

// Float to integer conversions as IDO, the N64's compiler, made them. C leaves
// out-of-range conversions undefined; IDO truncated to a 32-bit integer and
// kept the low bits, so (s16) 197925.0f is 1317 (parallel universes rely on
// it). Clang for WASM keeps the value unwrapped instead. Unsigned targets went
// through IDO's unsigned conversion: a negative result became 0xFFFFFFFF.
static inline u32 ido_f64_to_u32(f64 x) {
    s32 t;
    if (x > -2147483649.0 && x < 2147483648.0) {
        t = (s32) x;
        return t < 0 ? 0xFFFFFFFF : (u32) t;
    }
    x -= 2147483648.0;
    if (x > -2147483649.0 && x < 2147483648.0) {
        t = (s32) x;
        return t < 0 ? 0xFFFFFFFF : (u32) t | 0x80000000;
    }
    return 0xFFFFFFFF;
}

#define IDO_S16(x) ((s16) (s32) (x))
#define IDO_S8(x) ((s8) (s32) (x))
#define IDO_U32(x) ido_f64_to_u32(x)
#define IDO_U16(x) ((u16) ido_f64_to_u32(x))
#define IDO_U8(x) ((u8) ido_f64_to_u32(x))
// For macros whose argument may be an integer or a float.
#define IDO_U16_ANY(x) _Generic((x), f32: IDO_U16(x), f64: IDO_U16(x), default: (u16) (x))

#endif // IDO_CONVERT_H
