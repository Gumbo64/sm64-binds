# N64 oracle (dev tool)

Steps the headless WASM (`sm64-port/build/us_web_headless`) in lockstep with
[sm64_physics](https://github.com/Teero888/sm64_physics), a native build of the
game that its author checks against mupen64plus at every controller poll. After
every frame it compares, layout-independently: the RNG seed, timer, level and
area; Mario's state; every object in every object list (flags, animation,
position, hitbox, collisions and its 0x50 fields, minus pointer fields); and
`gLakituState`. It stops at the first difference in Mario, the globals or the
camera and prints the object differences of the frames before it.

sm64_physics is AGPL-3.0. Nothing of it is in this repository: build it
yourself and point this tool at it. The tool is never shipped or linked into the
packages.

```sh
git clone https://github.com/Teero888/sm64_physics
cmake -S sm64_physics -B sm64_physics/build-us -DCMAKE_BUILD_TYPE=Release -DSM64_VERSION=us
cmake --build sm64_physics/build-us -j
make -C ../sm64-port TARGET_WEB=1 HEADLESS_VERSION=1
SM64_PHYSICS_DIR=$PWD/sm64_physics cargo run --release            # the four TASes in ../tas
SM64_PHYSICS_DIR=$PWD/sm64_physics cargo run --release -- /path/to/run.m64
```

Options (environment variables):

- `FUZZ=N`: every N frames of each movie, branch both games and play 600
  frames of random input on the copies.
- `SOFT=1`: also list the first object-field differences of each kind.
- `TRACE=a..b`: print the compared globals and Mario on frames a to b;
  with `LIST=n`, also the objects of object list n.

Use a separate `CARGO_TARGET_DIR` from `../rust`: its release profile uses LTO
and the two builds otherwise trip over each other's dependency artifacts.
