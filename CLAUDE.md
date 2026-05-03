# python-to-x86

An ahead-of-time compiler that takes Python source and produces a native x86-64 ELF binary. **Performance target:** within a small constant factor of equivalent C or Rust (i.e. competitive with `clang -O2` / `rustc --release`). Not "faster than CPython" — competitive with hand-written native code.

This document is the project-level spec. Component-level specs live under `specs/`.

## Architectural commitments

| Topic | Decision |
|---|---|
| Compiler implementation language | **Rust** |
| Backend | **Emit LLVM IR** via Inkwell (LLVM Rust bindings) |
| Python subset | **Inferred-typing.** No annotations required; the compiler infers types. If any expression's type cannot be derived statically, compilation fails with an error pointing at the un-inferable site. **No `Any`. No dynamic fallback.** |
| Integer semantics | Default `int` = `int64`, wraps on overflow (C-style). User opts into explicit C-width types via the `pyx86.types` stub: `i8 i16 i32 i64 u8 u16 u32 u64 f32`. Annotations like `x: i32 = 5` override the default. |
| 3rd-party libraries | **Out of scope.** The compiler targets the language, not the ecosystem. |

## Compiler architecture

```
.py source
  └─> AST                 (rustpython-parser)
        └─> typed HIR     (after inference + monomorphization)
              └─> SSA mid-IR
                    └─> LLVM IR  (via Inkwell)
                          └─> object  ──>  linked ELF binary
```

The runtime is a small Rust crate, statically linked: allocator, panic, string ops, bounds-check failures. Memory management is **reference counting in v1** (predictable, simple). Tracing GC and escape-analysis arenas are deferred.

## Type system

- Hindley-Milner-flavoured bidirectional inference. Whole-program in v1 (no per-module separate compilation).
- Polymorphic functions (e.g. `def f(x): return x + 1`) are monomorphized per call-site type combination, like Rust generics.
- Annotations are *allowed* and constrain inference; they are not required.
- Integers default to `int64` and wrap on overflow. Opt into other widths via `from pyx86.types import i32`.
- Floats default to IEEE 754 `f64`; `f32` available via the same stub.

## Python feature support

### In scope for v1

**Primitives & operators**
- `int` (default int64, wraps), `float` (f64), `bool`, `None`, `str`, `bytes`
- C-width types via `pyx86.types`: `i8 i16 i32 i64 u8 u16 u32 u64 f32`
- Arithmetic: `+ - * / // % ** -x`
- Comparison: `< <= > >= == !=` (chained comparisons supported)
- Boolean (short-circuit): `and or not`
- Bitwise: `& | ^ ~ << >>`
- Augmented assignment: `+= -= *= ...`
- Membership: `x in container`
- Identity `is`, `is not` — only against `None` in v1

**Control flow**
- `if / elif / else`, `while`, `for ... in ...`, `break`, `continue`, `return`, `pass`
- Tuple unpacking, for-loop unpacking, starred LHS unpacking

**Functions**
- `def` at module and nested scope
- Positional and keyword arguments; literal/constant defaults
- `*args` / `**kwargs` only when all call sites have statically inferable shapes
- Recursion (direct and mutual), lambdas, closures over locals
- Annotations allowed but not required

**Data types**
- `list[T]`, `tuple[T1, T2, ...]`, `dict[K, V]`, `set[T]` — **monomorphic** in element types; mixed-type containers are a compile error
- Indexing and slicing on lists/tuples/strings/bytes
- List, dict, set comprehensions
- Generator expressions only when fusable into the consuming loop (no general generator support)
- f-strings (with statically known interpolated types)

**Classes**
- Class bodies with annotated fields (struct layout)
- `__init__`, `__repr__`, `__eq__`, `__hash__`
- Methods, `@staticmethod`, `@classmethod`, `@property`
- Single inheritance for concrete classes (no MRO)
- Operator overloading via `__add__`, `__mul__`, etc. — statically dispatched
- **`abc.ABC` and `@abstractmethod`** are supported, recognized syntactically (not via the general metaclass machinery). A class inheriting from `ABC` is abstract and cannot be instantiated. Concrete subclasses must implement every `@abstractmethod`, enforced at compile time. A class may inherit from one concrete class and additionally implement multiple ABCs (interface-style). Polymorphic use of an ABC-typed value compiles via monomorphization where the concrete type is statically determinable, and via vtable dispatch otherwise — matching Rust's split between generics and `dyn Trait`. Vtable dispatch is the **one** place we knowingly accept an indirect call; this is consistent with the perf target because Rust does the same thing for the same use case.

**Imports**
- Other `.py` files in the same project
- `pyx86.types` stub
- Stdlib allowlist (initially `math`, `sys.argv`)

### Out of scope for v1 — compile-time error

- `eval`, `exec`, `compile`
- Monkey-patching (assigning to module/class attributes at runtime after definition)
- `__getattr__`, `__setattr__`, `__getattribute__`, `__slots__`
- General metaclasses, `ABCMeta` as a user-accessible metaclass (`abc.ABC` itself is supported — see above)
- Multiple concrete inheritance, MRO-dependent code (multiple ABC inheritance is fine)
- Decorators in general — only `@staticmethod`, `@classmethod`, `@property`, `@abstractmethod` are recognized
- Generators with `yield`, generator coroutines
- `async` / `await`
- CPython C-API, `ctypes`, `cffi`, C extensions
- Pickling, `marshal`, `weakref`
- `threading`, `multiprocessing`, `asyncio`
- Reflection: `type(x)` for dynamic dispatch, dynamic `isinstance`
- Mixed-type containers (`list[int | str]`)
- Arbitrary stdlib modules (anything not on the allowlist)
- 3rd-party libraries
- `typing.Any`, `typing.Protocol` runtime checks

### Deferred to later phases

- **Exceptions**: `try / except / raise / finally` — phase 2
- **Sum types**: `int | str` annotations compiled to tagged unions — phase 2
- **Stdlib expansion**: `os.path`, `random`, `json`, basic file I/O — incremental
- **Memory management upgrade**: tracing GC or escape-analysis arenas

## Testing

Two pillars: **correctness** (does the compiled binary produce the same output as CPython?) and **performance** (is it as fast as equivalent Rust?).

**Never run the test suite by hand-invoking pytest, cargo test, or rustc directly.** Always launch the test bench binary, which handles parallelism, tiering, and reporting.

### Correctness pillar

Corpus at `tests/correctness/<program-name>/`:
- `program.py` — the program under test
- `strategy.py` — Hypothesis-style strategy describing input shape (typed: ints in range, lists of ints, etc.)
- `tier.toml` — declares tier and per-tier iteration count

Procedure for each program × tier:
1. Compile `program.py` with python-to-x86 → ELF binary.
2. Draw N inputs from the strategy.
3. Run CPython on `program.py` with each input → record stdout + exit code.
4. Run the compiled ELF with the same input → record stdout + exit code.
5. Assert equality.

On failure, shrink the input to a minimal repro and emit a repro file.

The corpus starts at the trivial end (`return 42`, `return a + b`) and grows feature by feature. Every supported feature gets at least one program in tier 1 and broader fuzzing in tier 3.

### Performance pillar

Corpus at `tests/performance/<benchmark-name>/`:
- `program.py` — Python implementation
- `program.rs` — equivalent Rust implementation (idiomatic, not micro-optimized)
- `bench.toml` — iteration counts, warmup counts, tier

Procedure:
1. Compile Python with python-to-x86 (release mode, full LLVM opt).
2. Compile Rust with `rustc -O` / `cargo build --release`.
3. Run both with the same inputs; record CPU time (median of K runs after W warmups, plus stddev).
4. Compute ratio `python_time / rust_time`.
5. Assert the ratio is below a configured threshold. Initially loose (≤ 2.0x) and tightened toward ≤ 1.1x as the compiler matures.

Bench programs start small (fib, mandelbrot, sieve) and grow (sorting, BFS, raytracer). The "as fast as Rust" target is judged on the median ratio across all bench programs at the highest tier.

### Tiers and budgets

| Tier | Wall-clock | Contents | When to run |
|---|---|---|---|
| **1 — Unit** | 3 s | Rust unit tests of compiler internals (parser, type inferer, IR passes) | After every meaningful edit |
| **2 — Smoke** | 30 s | Tens of small E2E programs, ~10 fuzz inputs each, no perf | After feature completion / before commit |
| **3 — Medium** | 5 min | Hundreds of programs, ~1000 fuzz inputs each, perf benches at small/medium sizes | Before push |
| **4 — Large** | 1 hr | Full corpus, millions of fuzz inputs, full perf benches with statistical rigor | **Sparingly**: only after several major features have landed, or before a release. Not nightly, not per-PR. |

**Promotion rule.** Run tier 1 first; only if it passes do you run tier 2; only then tier 3. Tier 4 is held back further and only run at milestone boundaries — running it on every change wastes hours of compute. Don't burn the 1-hour bench on a change that fails 3-second unit tests.

### Test bench design

Single Rust binary in `tests/bench/`:
```
cargo run --release --bin bench -- --tier=2 --jobs=8 [--filter=glob]
```

- Discovers test programs from `tests/correctness/` and `tests/performance/` based on tier metadata.
- Parallelizes across `--jobs` cores (each program is independent: separate compile, separate run).
- Outputs structured JSON results plus a human-readable summary.
- Designed for fire-and-forget: launch with `Bash run_in_background=true` and poll for completion.
- Hypothesis-style strategies and shrinking implemented in Rust via the `proptest` crate.

## Working norms for future Claude sessions

- **Spec-first.** Before building any non-trivial component, write a spec for it as a markdown file under `specs/` (e.g. `specs/parser.md`, `specs/type-inference.md`, `specs/codegen-llvm.md`, `specs/runtime.md`, `specs/test-bench.md`). The spec should be high-level but accurate: state the component's responsibility, its inputs and outputs, the interfaces it exposes, the invariants it maintains, and any non-obvious design choices with their *why*. **Treat the spec as the source of truth.** If you change the code, update the spec in the same commit. If the spec is wrong, fix the spec before touching the code. CLAUDE.md is the project-level spec; `specs/*.md` are component-level specs.
- **Commit frequently.** Small focused commits, one logical change per commit. Don't accumulate a multi-day diff. After every passing tier-2 run on a meaningful change, commit. Commit messages should name what changed and link the relevant spec file when applicable.
- **Reject rather than fall back.** When unsure whether a Python feature is in-scope, default to rejecting it with a clear error. Never silently fall back to dynamic dispatch or boxing. Subset creep is what kills the perf target.
- **The bench is the source of truth for performance**, not intuition. Add a benchmark before claiming a feature is fast.
- **Run tiers in promotion order.** Don't skip tier 1. Tier 4 is held back for milestone boundaries.
- **Build the first vertical slice end-to-end** (parser → infer → LLVM → binary) on `def main() -> int: return 42` before broadening. Don't speculatively stub files.
