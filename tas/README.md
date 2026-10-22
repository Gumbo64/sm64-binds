# TAS input files

Tool-assisted runs of the US ROM, used by the Rust tests as long, realistic
input sequences covering most of the game. All are Mupen64 `.m64` files
(version 3, power-on start, one controller) downloaded from the TASVideos
publication pages listed below. Inputs start at offset `0x400`, 4 bytes per
input poll: big-endian buttons, then stick x and stick y. That is the same
layout as the bindings' raw `GamePad` files, and the same one upstream
sm64-port's `cont.m64` playback used.

| File | Publication | Category | Polls | Syncs here? |
|---|---|---|---|---|
| `tasvideos-2016M-0star.m64` | <https://tasvideos.org/2016M> | 0 stars | 8,772 | Yes: reaches Bowser 3 (level 34). |
| `tasvideos-6943M-16star.m64` | <https://tasvideos.org/6943M> | 16 stars | 23,248 | Yes: 16 stars, reaches Bowser 3. |
| `tasvideos-7310M-120star.m64` | <https://tasvideos.org/7310M> | 120 stars (current) | 128,808 | Yes: 120 stars, reaches Bowser 3. Takes Save & Quit after Big Boo's Haunt on purpose (the reload puts Mario next to the Vanish Cap Under the Moat entrance). |
| `tasvideos-2208M-120star.m64` | <https://tasvideos.org/2208M> | 120 stars (obsoleted) | 140,467 | Yes: 120 stars, reaches Bowser 3. |

Resets: the m64 header has no reset field. Mupen 1.0.3+ treats a sample with
both reserved bits set (`0x00C0` here) as a console reset. 2208M has such
samples (from poll 65,843) but they are not resets. They come with L+R+Start,
which sets those bits on a real controller; the movie predates that encoding,
and it keeps syncing past them.

The 16-star file has trailing input past the header's poll count; the tests
read only the first `samples` (header offset `0x18`) entries.

All four sync, and `rust/tests/lockstep.rs` checks that each still plays to
its end. The two 120-star runs used to desync here (at stars 33 and 62); the
causes were differences between a WASM build of the game and the N64, fixed in
sm64-port (see `../oracle/README.md` for the tool that found them):

- out-of-range float to `s16` conversions: the N64 wraps them (parallel
  universes rely on it), clang for WASM did not (`include/ido_convert.h`);
- objects allocated with `malloc`: the N64's pool of 240, which evicts
  unimportant objects when full, is part of the game (`MALLOC_OBJECTS`);
- the PC port widening the in-view test for widescreen, which decides which
  held objects' positions update (`obj_is_in_view`).

These files are TASVideos publications; check TASVideos' terms before
redistributing them.
