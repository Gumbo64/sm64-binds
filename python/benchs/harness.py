"""A small, general-purpose wall-clock benchmark harness.

Mirrors ``rust/benchmark/harness.rs``, so the Python and Rust bindings can be
compared case for case: a bench script declares cases (an operation, a unit
of work, a scaling factor) and this module handles batching, warm-up,
sampling statistics and subtractive ("derived") rows. Nothing here knows
about SM64.

Timing per sampled case: three probes size the batch (how many calls fit in
``BATCH_TARGET``), ``WARMUP`` untimed calls, then repeated batches until
``target`` elapses. Every batch yields one per-unit sample; the mean is what
gets reported, because derived rows subtract means from each other.
"""

from __future__ import annotations

import argparse
import statistics
import time
from dataclasses import dataclass
from typing import Callable

#: Seconds one batched sample should take, so the clock's own overhead stays
#: negligible even when timing single-digit-microsecond calls.
BATCH_TARGET = 0.02

#: Untimed calls before sampling starts, so caches warm up.
WARMUP = 3

#: How many probes size the batch calibration.
PROBES = 3

#: Default sampling budget per case.
TARGET_SECONDS = 2.0


def fmt_seconds(seconds: float) -> str:
    """``12 ns``, ``3.45 us``, ``6.789 ms`` — fixed column units, so a row
    that switches unit never reshuffles the table."""
    ns = seconds * 1e9
    if ns < 1_000:
        return f"{ns:.0f} ns"
    if ns < 1_000_000:
        return f"{ns / 1e3:.2f} us"
    if ns < 1_000_000_000:
        return f"{ns / 1e6:.3f} ms"
    return f"{ns / 1e9:.3f} s"


def fmt_rate(units_per_second: float) -> str:
    """Human-scale ``/s`` figure matching :func:`fmt_seconds`."""
    if units_per_second < 1e3:
        return f"{units_per_second:.0f}/s"
    if units_per_second < 1e6:
        return f"{units_per_second / 1e3:.1f}k/s"
    if units_per_second < 1e9:
        return f"{units_per_second / 1e6:.1f}M/s"
    return f"{units_per_second / 1e9:.1f}G/s"


def time_batch(run: Callable[[], None], blocks: int) -> float:
    """Seconds for one batch of ``blocks`` invocations of ``run``."""
    start = time.perf_counter()
    for _ in range(blocks):
        run()
    return time.perf_counter() - start


@dataclass
class Measurement:
    """One case's samples, condensed to the numbers the table prints."""

    op: str
    unit: str
    mean: float     # seconds per unit
    minimum: float
    median: float
    samples: int


class Bencher:
    """Times a sequence of cases and prints them as it goes.

    Statistics deliberately favour the mean over the median: cheaper to
    collect on a bench run, and derived cases need to subtract per-unit
    means from each other.
    """

    def __init__(self, target: float = TARGET_SECONDS, warmup: int = WARMUP):
        self.target = target
        self.warmup = warmup
        self._pending: str | None = None
        self._measurements: dict[str, Measurement] = {}

    def section(self, title: str) -> None:
        """Arms a heading printed before the next measurement."""
        self._pending = title

    def measure_once(self, op: str, run: Callable[[], None], *, unit: str = "call") -> float:
        """Times a setup-style cost a single time (load, compile, ...).

        Things like module compilation run once per process, and re-running
        them to sample would take nearly as long as the benchmark itself. The
        single sample is exact (one operation, one clock read) but noisy —
        these rows are for magnitude, not comparisons.
        """
        self._flush_section()
        start = time.perf_counter()
        run()
        mean = time.perf_counter() - start
        self._measurements[op] = Measurement(op, unit, mean, mean, mean, 0)
        self._print(op, mean, unit, "single run")
        return mean

    def measure(
        self,
        op: str,
        run: Callable[[], None],
        *,
        units_per_block: float = 1.0,
        unit: str = "call",
    ) -> float:
        """Samples ``run``, which must perform ``units_per_block`` units of work.

        Measured bodies vary hugely in single-call cost: ``get_game_state``
        is tens of nanoseconds, a 2000-frame block is hundreds of
        milliseconds. ``units_per_block`` states how much work one ``run``
        call performs — one unit by default, or 2000 for a whole session
        block — so every result comes out per-unit and rows stay comparable
        across that range. Batching (``BATCH_TARGET``) keeps the clock's own
        read cost from engulfing fast ops: many short ``run`` calls are timed
        together, then divided back per unit.
        """
        self._flush_section()

        # Calibrate how many calls make one well-timed batch. The minimum of
        # three probes, so a stray scheduler interrupt can not inflate the
        # batch and drag every sample's resolution down with it.
        probe = min(time_batch(run, 1) for _ in range(PROBES))
        blocks_that_fit = max(1, round(BATCH_TARGET / probe)) if probe > 0 else 1

        for _ in range(self.warmup):
            run()

        samples = []
        start = time.perf_counter()
        while time.perf_counter() - start < self.target:
            elapsed = time_batch(run, blocks_that_fit)
            samples.append(elapsed / (blocks_that_fit * units_per_block))

        self._measurements[op] = Measurement(
            op,
            unit,
            statistics.mean(samples),
            min(samples),
            statistics.median(samples),
            len(samples),
        )
        m = self._measurements[op]
        self._print(op, m.mean, unit, f"{m.samples} samples, min {fmt_seconds(m.minimum)}")
        return m.mean

    def derived(
        self,
        op: str,
        base: str,
        minus: str,
        *,
        scale: float = 1.0,
        unit: str = "",
    ) -> float:
        """Adds a subtractive row: mean unit time of ``base`` minus ``scale`` x ``minus``.

        ``scale`` is how much of the ``minus`` op each unit of the base op
        consumes. The classic use: one N-frame block contains one game
        instantiation, so its "per frame excluding create_game" row subtracts
        ``scale=1/N`` of a create_game.

        The result is only as trustworthy as the difference between two noisy
        means — when it approaches zero or goes negative, the subtraction is
        distinguishing noise, not work.
        """
        self._flush_section()
        base_m = self._measurements[base]
        minus_m = self._measurements[minus]
        mean = base_m.mean - scale * minus_m.mean
        unit = unit or base_m.unit
        self._print(op, mean, unit, f"{base} minus {scale:.6} x {minus}")
        return mean

    def note(self, text: str) -> None:
        """Prints an informational line outside the table (sizes, notes)."""
        self._flush_section()
        print(f"  {text}")

    def _print(self, op: str, mean: float, unit: str, sublabel: str) -> None:
        rate = fmt_rate(1.0 / mean) if mean > 0 else "-"
        print(f"  {op:<44} {fmt_seconds(mean):>10} {unit:<7} {rate:>9}  ({sublabel})")

    def _flush_section(self) -> None:
        if self._pending is not None:
            print(f"\n=== {self._pending} ===")
            self._pending = None


def parse_args(argv=None, default_rom: str = "baserom.us.z64"):
    """The bench CLI: a ROM path positionally, ``--target-secs`` and
    ``--frames`` as the only flags — mirroring the Rust benches."""
    parser = argparse.ArgumentParser(description="Timing harness")
    parser.add_argument("rom", nargs="?", default=default_rom, help="path to a US .z64 ROM")
    parser.add_argument("--target-secs", type=float, default=TARGET_SECONDS, help="seconds per case")
    parser.add_argument("--frames", type=int, default=2000, help="frames per amortized stepping case")
    return parser.parse_args(argv)
