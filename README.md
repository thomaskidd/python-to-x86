# python-to-x86

An ahead-of-time compiler from Python to native x86-64 ELF binaries. Performance target: competitive with `clang -O2` and `rustc --release` — not "faster than CPython," but on par with hand-written native code.

> **Status:** early. Repo charter is in place; no compiler code yet.

## Approach in one paragraph

Written in Rust, emits LLVM IR via Inkwell, and accepts a statically-inferable subset of Python (no annotations required, but the compiler must derive every expression's type — `Any` and dynamic fallback are not in scope). Default `int` is `int64` with C-width opt-in via a `pyx86.types` stub. Classes, single inheritance, and `abc.ABC` (with monomorphization or vtable dispatch) are supported; metaclasses, generators, `async`, `eval`, monkey-patching, and arbitrary stdlib/3rd-party imports are out of scope. Tested via a tiered bench that differential-tests against CPython for correctness and against equivalent Rust for performance.

## Full spec

See [`CLAUDE.md`](./CLAUDE.md) for the project charter — architectural commitments, the in/out-of-scope feature enumeration, the testing strategy (four tiers, 3s/30s/5min/1hr budgets, promotion rule), and the working norms (spec-first, frequent commits, reject-rather-than-fall-back).

Component-level specs will live under `specs/` as the corresponding components are built.
