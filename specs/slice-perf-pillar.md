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

## Initial benchmarks (3 programs, all in our v0.6 language subset)

| Bench | Workload | Why this workload |
|---|---|---|
| `lcg_chain_perf` | 100M iterations of an LCG step (`a = a*c1 + c2`) | Sequential data dependency prevents LLVM from vectorising or folding. Pure-arithmetic hot loop. |
| `primes_trial_perf` | Count primes in `[2, 100k)` by trial division up to `sqrt(i)` | Branch-heavy, nested while + `break`. Exercises the alloca/load/store pattern under control flow. |
| `modexp_perf` | 1M iterations of binary exponentiation `b^e mod p` | Div/mod heavy (the floor-correction blocks). Each outer iteration mutates `base` and `exp` so LLVM can't precompute. |

Each bench takes 13–21 ms per run in both implementations, well above the ~1 ms process-startup floor — so the timing reflects compute, not startup.

### Initial baseline (from this slice)

| Bench | py median | rs median | ratio |
|---|---|---|---|
| `lcg_chain_perf` | 13.2 ms | 13.6 ms | **0.97x** |
| `primes_trial_perf` | 13.6 ms | 13.9 ms | **0.98x** |
| `modexp_perf` | 20.5 ms | 21.3 ms | **0.96x** |

`max_ratio` is set to **1.5x** for all three — generous safety margin for noise / scheduling, but tight enough that any meaningful regression in the codegen pipeline (e.g. accidentally disabling an optimization, or generating bloated IR LLVM can't clean up) will trip the assertion.

### Where the wins/losses come from

- We compile to LLVM IR; rustc compiles to LLVM IR; both go through the same optimizer at `-O2`. Steady-state code quality should be essentially identical.
- pyx86's binaries are slightly leaner (no Rust panic infrastructure, no `_Unwind_Resume`, no debug-info-by-default), which helps the startup component slightly. On compute-dominated workloads this is in the noise.
- The floor-div / floor-mod correction blocks in pyx86's IR are emitted regardless of operand range; LLVM-O2 doesn't always eliminate the correction even when it's provably dead. The Rust benchmarks call equivalent helper functions explicitly so the comparison is fair, but if pyx86 ever needs to compete against *idiomatic* Rust (where `%` is plain `srem`), it'll need an analysis pass that drops the correction when both operands are statically non-negative.

### Future work

- Tighten `max_ratio` toward 1.1x once the codegen has enough optimization affordances (constant folding pre-LLVM, dead-code elim for floor corrections on non-negative operands).
- Add larger benchmarks: a fixed-size matrix-multiply, a Mersenne-Twister RNG hot loop, a small sorting kernel — all of which need either container types or fixed-size arrays first.

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
