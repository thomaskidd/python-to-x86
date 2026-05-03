# Spec: performance pillar

> Status: in progress.

The bench has had a "correctness" pillar since v0.1; this slice adds the **performance** pillar that the project charter (CLAUDE.md) commits to. The compiler is the system under test; Rust at `-O2` (or `cargo --release`) is the baseline.

## What this pillar measures

For each benchmark in `tests/performance/<name>/`, run an equivalent Python and Rust implementation on the same inputs, measure CPU time, and compute the ratio `python_time / rust_time`. Assert the ratio is below a configured threshold (the `max_ratio` field of `bench.toml`).

Initial threshold per benchmark: **2.0x** (loose, so v1 of the perf pillar lands without flakiness). The threshold is tightened toward 1.1x in subsequent slices as the compiler matures.

## Corpus layout

```
tests/performance/<name>/
    program.py       # Python implementation
    program.rs       # Rust implementation, idiomatic, not micro-optimized
    bench.toml       # tier, warmup, iterations, max_ratio, input
```

`bench.toml`:
```toml
tier = 3                 # bench is tier 3 by default
warmup = 5               # discarded warmup runs
iterations = 50          # measured runs (we report median)
max_ratio = 2.0          # python_time / rust_time must be ≤ this
[[input]]
type = "i64"
range = [10_000, 100_000]
```

`[[input]]` uses the same shape as `strategy.toml` from the correctness pillar (currently only `i64` with a required `range`).

## Bench command

The same `pyx86-bench` binary handles both pillars. New CLI flag:

```
pyx86-bench --tier 3 --jobs 4 --compiler "./target/release/pyx86" \
            [--rust-compiler "rustc"]   # default
```

When `--tier ≥ 3`, the bench discovers `tests/performance/`. Lower tiers skip it (the perf bench is too slow for tier 1/2 budgets).

Per-bench procedure:
1. Compile `program.py` with `--compiler` → `<scratch>/<name>-py.elf`.
2. Compile `program.rs` with `--rust-compiler -O <single-file>` → `<scratch>/<name>-rs.elf`. (We use single-file `rustc -O` not `cargo --release` to keep things simple; if a benchmark ever needs crates we'll switch.)
3. Generate **one** input set per the `[[input]]` section. Performance comparison uses a fixed input across all measurement runs (variance comes from the runtime itself, not from input variation).
4. For each implementation, run `warmup + iterations` total invocations. Discard the first `warmup`. For each measured run, capture wall-clock CPU time via `clock_gettime(CLOCK_PROCESS_CPUTIME_ID)`-equivalent (Rust `std::time::Instant` is monotonic and good enough; we record process-spawn-included time which slightly biases toward the slower of equivalent steady-state implementations — fine for the order-of-magnitude check we care about).
5. Take the **median** of the iteration times for each side. Compute `ratio = median_py / median_rs`.
6. Assert `ratio ≤ max_ratio`.

## Output

Bench summary now has two sections:

```
correctness:
  ok   …
performance:
  ok   loop_sum_perf      n=50000  py 12.3 ms / rs 8.7 ms   ratio 1.41x  (≤ 2.00x)
  FAIL fib_iter_perf      n=40     py 4.2 ms / rs 0.6 ms    ratio 7.00x  (> 2.00x)
24/24 correctness, 4/5 performance.
```

JSON output gains a `"performance"` array mirroring the human view.

## Initial benchmarks (4 programs, all in our int-only subset)

| Bench | Python | Equivalent Rust |
|---|---|---|
| `sum_iter_perf` | sum of 0..n via while loop | identical structure in Rust |
| `fib_iter_perf` | iterative Fibonacci(n) via two-var swap | same |
| `gcd_perf` | Euclidean algorithm | same |
| `factorial_perf` | n! up to n=12 (capped to fit in i64) | same |

All four work today in our v0.6 language. The `program.rs` for each is a single `fn main() { println!("{}", py_main(...)); }` with the same control flow as the Python version.

We deliberately start at small/medium problem sizes — the goal is to validate the perf-pillar **infrastructure**, not to win or lose the race yet. Tightening of `max_ratio` and adding harder benchmarks (mandelbrot, raytracer, sorting kernels) comes once the bench itself is solid.

### Known limitation: initial benchmarks are startup-dominated

The first run of this pillar showed all 4 benchmarks finishing in under 1 ms each, with py/rs ratios ~0.7x. That is **not the compiler beating clang/rustc** — it is the workload taking less time than process startup. LLVM's loop idiom recognition folds `sum 0..n` to a closed form, and the other workloads (factorial of 12, fib of 80) run in microseconds even without folding.

The ratio ≤ 2.0x assertion still pins a real property: pyx86's compile pipeline produces binaries whose process-start + workload time is no worse than 2x rustc's. But to *test the compiler's ability to keep up with rustc on hot inner loops*, the benchmarks need to do enough non-foldable work that startup time is a small fraction of total. A follow-up slice will:

- Replace foldable workloads (arithmetic-series sums) with linear-congruential-style chains LLVM can't fold.
- Increase iteration counts so total CPU time per run is in the 10–100 ms range, well above the ~1 ms process-startup floor.
- Add benchmarks across more code patterns (branch-heavy, modular-arithmetic, etc.).
- Then tighten `max_ratio` toward 1.1x.

This limitation is recorded so future contributors don't read the green ratios as "we're faster than Rust" — we are not, we are **roughly the same** at jobs where everything happens in startup.

## Tier assignment

Perf benches are **tier 3** by default. They run when `pyx86-bench --tier 3` is invoked. Tier 1 and 2 stay correctness-only, so the fast-iteration loop is unchanged.

Per CLAUDE.md, tier 3 has a 5-minute wall-clock budget. With 4 benchmarks × `(50 + 5)` runs × ~100 ms per run × 2 implementations ≈ 44 s. Plenty of headroom.

## What this pillar does **not** add

- LTO / PGO / native-cpu tuning. Rust uses `-O`. Python uses `pyx86 -o … --opt-level 2`.
- `cargo build --release` — single-file `rustc -O` for now.
- Statistical rigor beyond median (no confidence intervals, no Welch's t-test). Median is enough for the order-of-magnitude check we care about.
- Cross-platform — Linux x86-64 only.
- Memory / binary-size measurement.

## Files added

- `crates/pyx86_bench/src/main.rs` — perf section, `[[input]]` parsing reusing `ArgStrategy`, `program.rs` driver, median helper.
- `tests/performance/{sum_iter_perf, fib_iter_perf, gcd_perf, factorial_perf}/{program.py, program.rs, bench.toml}`.
- `specs/test-bench.md` updated to document the new procedure.
