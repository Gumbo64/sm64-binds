# N64 stack frames (dev tool)

Generates the `N64_FRAME(size);` lines in sm64-port (see
`sm64-port/include/n64_stack.h`). The sizes are the US ROM's IDO stack frames,
read from a matching build of the [decomp](https://github.com/n64decomp/sm64).

```sh
git clone https://github.com/n64decomp/sm64 decomp      # with core.autocrlf=false: IDO rejects CRLF
cp baserom.us.z64 decomp/ && make -C decomp VERSION=us COMPARE=1   # needs binutils-mips-linux-gnu
B=decomp/build/us
python3 mdebug.py $B > procs.json                     # every procedure's frame size, statics included
python3 callgraph.py $B procs.json > cg.json          # jal edges, jalr sites, address-taken in code
python3 taken.py $B procs.json > taken_data.json      # address-taken in data (behaviours, geo, scripts)
python3 needed.py cg.json taken_data.json > needed.json
python3 locate.py ../../sm64-port needed.json cg.json located.json   # report: all found, in ROM order
python3 insert.py ../../sm64-port located.json
```

`needed.py` keeps the procedures that can be on the stack at the model's three
places: `find_floor_from_list` (stores `z3` at `sp + 28`), the JRB sliding box
(reads `sp1E`, the low half of `sp + 28`) and `set_camera_mode` (its `sp` is
what `nop_update_water_camera` returns). An indirect call is assumed to reach
any address-taken procedure of its own module (game, Goddard or libultra and
audio). The render walk is left out (procedures only called from
`geo_process_root`'s walk): it was about half of the model's cost and never on
the stack at those places in any run. `geo_process_root` switches the model off
with `N64_STACK_OFF()` instead (add that by hand). In October 2026 that left
1,064 procedures (1,066 lines; two have `#ifdef` variants).

The game loop's `sp` (`0x80206DC8`) is `gThread5Stack + 0x2000`, minus 16 from
`osCreateThread`, minus `thread5_game_loop`'s 40-byte frame.

Recent decomp builds of `tools/ido-static-recomp` and `tools/armips` can fail
with new compilers. The prebuilt
[ido-static-recomp](https://github.com/decompals/ido-static-recomp/releases)
(IDO 5.3, Linux) works when set as `IDO_ROOT` in the decomp's Makefile, and
sm64-port's own `tools/armips` can be copied in. With `-j`, some
`levels/*/leveldata.elf` links may fail; running the same link again by hand
succeeds.
