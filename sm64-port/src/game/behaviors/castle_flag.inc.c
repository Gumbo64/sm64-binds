// castle_flag.inc.c

void bhv_castle_flag_init(void) {
    o->header.gfx.animInfo.animFrame = IDO_S16(random_float() * 28.0f);
}
