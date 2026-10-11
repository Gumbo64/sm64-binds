# Benchmarks

Standalone timing programs, not cargo's `libtest` benchmarks — each boots its
own game instance and prints a comparison table. Both share the same general
harness, `benchmark/harness.rs` (mirrored by `python/benchs/harness.py` for
the Python binding): cases are declared with an operation name, a unit of
work and a scaling factor, and the harness handles calibration, batching,
warm-up, statistics and derived (subtractive) rows.

| Benchmark | What it measures |
| --- | --- |
| `hot_paths` | Stepping, `get_game_state`, `rng_pad`, and every save-state path. The one to run when changing anything on a hot path. |
| `diagnostics` | Why those numbers look the way they do: game instantiation phases, the fixed cost of a WASM call, and `copy_to` compared against raw runtime memory access. Uses wasmer directly, because the comparison needs access the public API does not expose. |

## Running

Both need a US `.z64` ROM at the repo root.

```
cargo bench --bench hot_paths -- ../baserom.us.z64
cargo bench --bench diagnostics -- ../baserom.us.z64
```

`--target-secs <f>` changes the sampling budget per case (default 3.0 s for
`hot_paths`, 1.5 s for `diagnostics`). These are cargo bench targets with
`harness = false`, so `cargo bench` runs each one's `main()` directly. The
`bench` profile inherits `release` from `rust/.cargo/config.toml`, which
keeps the numbers comparable with the code that ships.

Or from the repo root:

```
make bench
```

The Python binding's equivalent:

```
make bench-python
```

## Reading the numbers

They are wall-clock timings on one machine, so treat them as relative and
compare rows within a table, not across machines. Expect a few percent of
noise between runs on a shared container; each sampled case prints its
sample count and the fastest batch it saw.

Always `--release`. A debug build runs the emulated game several times slower
and the ratios stop meaning anything.

## Where these are wired in

`make bench` and `make lint` cover these, and `cargo clippy --all-targets`
type-checks them. They are deliberately not part of `cargo test`: they assert
nothing and need a ROM, so they would only slow the suite down.
