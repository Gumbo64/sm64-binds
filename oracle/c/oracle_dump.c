// Layout-independent dump of the state the oracle compares (native side).
#include <stdint.h>
#include <stddef.h>
#include "sm64_physics.h"
#include "types.h"
#include "game/object_list_processor.h"
#include "game/level_update.h"
#include "game/game_init.h"
#include "game/area.h"
#include "engine/behavior_script.h"
#include "game/camera.h"
// gRandomSeed16 is static: same section as gGlobalTimer, at SEED_OFFSET from it (build.rs).
#ifndef SEED_OFFSET
#error SEED_OFFSET
#endif

#define V(x) (*(__typeof__(&(x))) sm64_world_variable(w, &(x)))

static size_t put_obj(uint32_t *o, size_t n, const struct Object *obj) {
    union { float f; uint32_t u; } c;
#define F(x) (c.f = (x), o[n++] = c.u)
    o[n++] = (uint16_t) obj->activeFlags;
    o[n++] = (uint16_t) obj->header.gfx.animInfo.animID;
    o[n++] = (uint16_t) obj->header.gfx.animInfo.animFrame;
    o[n++] = (uint16_t) obj->bhvDelayTimer;
    for (int i = 0; i < 3; i++) o[n++] = (uint16_t) obj->header.gfx.angle[i];
    for (int i = 0; i < 3; i++) F(obj->header.gfx.pos[i]);
    F(obj->hitboxRadius); F(obj->hitboxHeight); F(obj->hurtboxRadius); F(obj->hurtboxHeight); F(obj->hitboxDownOffset);
    o[n++] = obj->collidedObjInteractTypes;
    o[n++] = (uint16_t) obj->numCollidedObjs;
    for (int i = 0; i < 0x50; i++) o[n++] = obj->rawData.asU32[i];
    return n;
#undef F
}

size_t oracle_dump(const sm64_world *w, uint32_t *o, size_t cap) {
    size_t n = 0;
    union { float f; uint32_t u; } c;
#define F(x) (c.f = (x), o[n++] = c.u)
    o[n++] = V(gGlobalTimer);
    o[n++] = *(const uint16_t *) ((const char *) &V(gGlobalTimer) + SEED_OFFSET);
    o[n++] = (uint16_t) V(gCurrLevelNum);
    o[n++] = (uint16_t) V(gCurrAreaIndex);
    const struct MarioState *m = &V(gMarioStates)[0];
    o[n++] = m->action; o[n++] = m->actionState; o[n++] = m->actionTimer;
    for (int i = 0; i < 3; i++) o[n++] = (uint16_t) m->faceAngle[i];
    o[n++] = (uint16_t) m->health; o[n++] = (uint16_t) m->numStars; o[n++] = (uint16_t) m->numCoins;
    for (int i = 0; i < 3; i++) F(m->pos[i]);
    for (int i = 0; i < 3; i++) F(m->vel[i]);
    F(m->forwardVel);
    struct ObjectNode *lists = V(gObjectListArray);
    for (int l = 0; l < NUM_OBJ_LISTS; l++) {
        size_t countAt = n++;
        uint32_t count = 0;
        struct ObjectNode *head = &lists[l];
        for (struct ObjectNode *node = head->next; node != head && n + 100 < cap; node = node->next) {
            n = put_obj(o, n, (const struct Object *) node);
            count++;
        }
        o[countAt] = count;
    }
    const uint32_t *lakitu = (const uint32_t *) &V(gLakituState);
    for (size_t i = 0; i < sizeof(struct LakituState) / 4; i++) o[n++] = lakitu[i];
    return n;
#undef F
}

// Each object's behavior script address, list by list, for reports.
size_t oracle_behaviors(const sm64_world *w, uintptr_t *o, size_t cap) {
    size_t n = 0;
    struct ObjectNode *lists = V(gObjectListArray);
    for (int l = 0; l < NUM_OBJ_LISTS; l++) {
        struct ObjectNode *head = &lists[l];
        for (struct ObjectNode *node = head->next; node != head && n < cap; node = node->next) {
            o[n++] = (uintptr_t) ((const struct Object *) node)->behavior;
        }
    }
    return n;
}
