//! A small, general-purpose wall-clock benchmark harness.
//!
//! Mirrors `python/benchs/harness.py`, so the Python and Rust bindings can be
//! compared case for case: a bench script declares cases (an operation, a unit
//! of work, a scaling factor) and this module handles batching, warm-up,
//! sampling statistics and subtractive ("derived") rows. Nothing here knows
//! about SM64.
//!
//! Timing per sampled case: three probes size the batch (how many calls fit
//! in `BATCH_TARGET`), `WARMUP` untimed calls, then repeated batches until
//! `target` elapses. Every batch yields one per-unit sample; the mean is what
//! gets reported, because derived rows subtract means from each other.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Seconds one batched sample should take, so the clock's own overhead stays
/// negligible even when timing single-digit-microsecond calls.
pub const BATCH_TARGET: Duration = Duration::from_millis(20);

/// Untimed calls before sampling starts, so caches warm up.
pub const WARMUP: u32 = 3;

/// How many probes size the batch calibration.
pub const PROBES: u32 = 3;

/// Reads `--target-secs <f>` (default 3.0) from the process arguments.
///
/// Kept hand-rolled rather than a clap-style parser: the benchmarks take a
/// ROM path positionally and are invoked via `make bench`, so a single flag
/// and one positional argument is the whole surface.
pub fn target_from_args(default_secs: f64) -> Duration {
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if arg != "--target-secs" {
            continue;
        }
        if let Some(v) = it.next().and_then(|s| s.parse::<f64>().ok()) {
            return Duration::from_secs_f64(v.clamp(0.05, 3600.0));
        }
    }
    Duration::from_secs_f64(default_secs)
}

/// The first positional argument: the ROM path, ignoring flags.
pub fn rom_path_from_args(default: &str) -> String {
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        if arg == "--target-secs" {
            let _ = it.next(); // the seconds value
            continue;
        }
        if arg.starts_with("--") {
            continue;
        }
        return arg;
    }
    default.to_string()
}

/// `12 ns`, `3.45 us`, `6.789 ms`, `1.234 s` — the column width a sampled
/// row can count on, so switching units never reshuffles the table.
pub fn fmt_seconds(secs: f64) -> String {
    let ns = secs * 1e9;
    if ns < 1_000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} us", ns / 1e3)
    } else if ns < 1_000_000_000.0 {
        format!("{:.3} ms", ns / 1e6)
    } else {
        format!("{:.3} s", ns / 1e9)
    }
}

fn fmt_rate(per_second: f64) -> String {
    if per_second < 1e3 {
        format!("{per_second:.0}/s")
    } else if per_second < 1e6 {
        format!("{:.1}k/s", per_second / 1e3)
    } else if per_second < 1e9 {
        format!("{:.1}M/s", per_second / 1e6)
    } else {
        format!("{:.1}G/s", per_second / 1e9)
    }
}

/// One case's samples, condensed to the numbers the table prints.
#[derive(Clone, Copy)]
struct Measurement {
    mean: f64,
    minimum: f64,
    samples: usize,
}

/// Times a sequence of cases and prints them as it goes.
///
/// Statistics deliberately favour the mean over the median: cheaper to
/// collect on a bench run, and derived cases need to subtract per-unit
/// means from each other.
pub struct Bencher {
    target: Duration,
    warmup: u32,
    pending: Option<String>,
    measurements: HashMap<String, Measurement>,
}

impl Bencher {
    pub fn new(target: Duration) -> Self {
        Bencher {
            target,
            warmup: WARMUP,
            pending: None,
            measurements: HashMap::new(),
        }
    }

    /// Arms a heading printed before the next measurement.
    pub fn section(&mut self, title: &str) {
        self.pending = Some(title.to_string());
    }

    /// Times a setup-style cost a single time (load, compile, ...).
    ///
    /// Things like module compilation run once per process, and re-running
    /// them to sample would take nearly as long as the benchmark itself. The
    /// single sample is exact (one operation, one clock read) but noisy —
    /// these rows are for magnitude, not comparisons.
    pub fn measure_once<F: FnOnce()>(&mut self, op: &str, run: F) -> f64 {
        self.flush_section();
        let start = Instant::now();
        run();
        let mean = start.elapsed().as_secs_f64();
        self.measurements.insert(
            op.to_string(),
            Measurement {
                mean,
                minimum: mean,
                samples: 0,
            },
        );
        self.print(op, mean, "call", "single run");
        mean
    }

    /// Samples `run`, which must perform `units_per_block` units of work.
    ///
    /// Measured bodies vary hugely in single-call cost: `get_game_state` is
    /// tens of nanoseconds, a 2000-frame block is hundreds of milliseconds.
    /// `units_per_block` states how much work one `run` call performs — one
    /// unit by default, or 2000 for a whole session block — so every result
    /// comes out per-unit and rows stay comparable across that range.
    /// Batching (`BATCH_TARGET`) keeps the clock's own read cost from
    /// engulfing fast ops: many short `run` calls are timed together, then
    /// divided back per unit.
    pub fn measure<F: FnMut()>(
        &mut self,
        op: &str,
        units_per_block: f64,
        unit: &str,
        mut run: F,
    ) -> f64 {
        self.flush_section();

        // Calibrate how many calls make one well-timed batch. The minimum of
        // three probes, so a stray scheduler interrupt can not inflate the
        // batch and drag every sample's resolution down with it.
        let mut probe = f64::INFINITY;
        for _ in 0..PROBES {
            let start = Instant::now();
            run();
            probe = probe.min(start.elapsed().as_secs_f64());
        }
        let blocks_that_fit = if probe > 0.0 {
            (BATCH_TARGET.as_secs_f64() / probe).round().max(1.0) as u64
        } else {
            1
        };

        for _ in 0..self.warmup {
            run();
        }

        let mut samples = Vec::new();
        let start = Instant::now();
        while start.elapsed() < self.target {
            let t0 = Instant::now();
            for _ in 0..blocks_that_fit {
                run();
            }
            samples.push(t0.elapsed().as_secs_f64() / (blocks_that_fit as f64 * units_per_block));
        }
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let m = Measurement {
            mean,
            minimum: samples.iter().cloned().fold(f64::INFINITY, f64::min),
            samples: samples.len(),
        };
        self.measurements.insert(op.to_string(), m);
        self.print(
            op,
            mean,
            unit,
            &format!("{} samples, min {}", m.samples, fmt_seconds(m.minimum)),
        );
        mean
    }

    /// Adds a subtractive row: mean unit time of `base` minus `scale` x `minus`.
    ///
    /// `scale` is how much of the `minus` op each unit of the base op
    /// consumes. The classic use: one N-frame block contains one game
    /// instantiation, so its "per frame excluding create_game" row subtracts
    /// `scale = 1/N` of a create_game.
    ///
    /// The result is only as trustworthy as the difference between two noisy
    /// means — when it approaches zero or goes negative, the subtraction is
    /// distinguishing noise, not work.
    pub fn derived(&mut self, op: &str, base: &str, minus: &str, scale: f64, unit: &str) -> f64 {
        self.flush_section();
        let scale = scale.max(0.0);
        let result = self
            .measurements
            .get(base)
            .map(|b| b.mean)
            .unwrap_or_else(|| absent(base));
        let sub = self
            .measurements
            .get(minus)
            .map(|m| m.mean)
            .unwrap_or_else(|| absent(minus));
        let mean = result - scale * sub;
        self.print(
            op,
            mean,
            unit,
            &format!("{base} minus {scale:.6} x {minus}"),
        );
        mean
    }

    /// Prints an informational line outside the table (sizes, casework).
    pub fn note(&mut self, text: &str) {
        self.flush_section();
        println!("  {text}");
    }

    fn print(&self, op: &str, mean: f64, unit: &str, sublabel: &str) {
        let rate = if mean > 0.0 {
            fmt_rate(1.0 / mean)
        } else {
            "-/s".to_string()
        };
        println!(
            "  {op:<44} {:>10} {:<7} {:>9}  ({sublabel})",
            fmt_seconds(mean),
            unit,
            rate
        );
    }

    fn flush_section(&mut self) {
        if let Some(title) = self.pending.take() {
            println!("\n=== {title} ===");
        }
    }
}

fn absent(name: &str) -> f64 {
    panic!("no measurement recorded for '{name}'");
}
