# Spec: pyx86 compiler — overview

This is the top-level compiler spec. Each pipeline stage gets its own spec file as it lands; this document only describes the stages, their interfaces, and the bootstrap path.

## Responsibility

The `pyx86` binary takes a Python source file and produces a native x86-64 ELF executable. CLI:

```
pyx86 <input.py> -o <output.elf> [--emit=ll|asm|elf] [--opt-level=0..3] [--keep-tmp]
```

Default `--emit=elf`. `--emit=ll` stops after IR generation; `--emit=asm` stops after `llc`. `--opt-level` defaults to 2.

Exit code 0 on success, non-zero with a structured error on stderr otherwise.

## Pipeline

```
.py  ──parse──▶  AST
                  │
              resolve+infer
                  │
                  ▼
              typed HIR
                  │
              monomorphize
                  │
                  ▼
            mid-level SSA IR
                  │
              codegen (LLVM IR text)
                  │
                  ▼
              .ll file
                  │
              clang -O<n>  ──▶  .elf
```

Each stage is a Rust module under `crates/pyx86/src/`:

| Stage | Module | Spec |
|---|---|---|
| Parse | `parser.rs` | `specs/parser.md` |
| Type inference | `infer.rs` | `specs/type-inference.md` |
| Typed HIR + monomorphization | `hir.rs` | `specs/hir.md` |
| Mid-IR (SSA) | `mir.rs` | `specs/mir.md` |
| LLVM IR codegen | `codegen.rs` | `specs/codegen-llvm.md` |
| Driver / CLI | `main.rs` | covered here |

Specs are written **immediately before** the corresponding module is implemented. Code without a spec is forbidden by repo policy (see CLAUDE.md "Working norms").

## Backend strategy: emit LLVM IR text, shell out

The repo charter commits to LLVM. v1 emits **LLVM IR text** (`.ll`) and shells out to `clang` for the final assemble+link step, instead of linking against `libLLVM` via Inkwell.

Rationale:
- Avoids the `llvm-sys` build-time dependency (large, version-coupled, non-trivial to set up).
- IR is human-readable and trivially inspectable — debugging is much easier than poking at C++ objects through FFI.
- LLVM IR text format is stable across LLVM 14–18; we are not pinned to a specific libLLVM version.
- Process-spawn overhead (~10 ms per compile) is irrelevant for this project.

Inkwell may be revisited if/when compile time becomes the bottleneck. Until then, text IR is the source of truth.

External dependencies the compiler invokes at runtime:
- `clang` (used as the LLVM frontend driver: linking, optimization passes, libc startup glue). Located via `$PATH`. Required for `--emit=elf`.

## Input/output convention used by the test bench

The test bench compiles each `program.py` and runs the resulting binary, comparing stdout against `python3 -c "from program import main; print(repr(main()))"`.

This means the compiler must generate a `_start`/`main` wrapper that:
1. Calls the user's `main()` function.
2. Formats the return value the same way Python's `repr()` would.
3. Writes the formatted string (plus a single `\n`) to stdout.
4. Exits 0.

For v1's first slice (return type `int`), this is a `printf("%ld\n", result)` plus `exit(0)`. Other types come online as they are added (each gets a printer in the runtime crate).

## Bootstrap path (the order of operations)

The repo charter forbids speculative stub files. The compiler is built one vertical slice at a time, end-to-end, and each slice is gated by tier-1 + tier-2 of the test bench passing:

| Slice | Adds | Test programs introduced |
|---|---|---|
| `v0.1` — return literal | parser (literal `int` return only); codegen for `i64` return + printf wrapper | `return_constant` |
| `v0.2` — arithmetic on i64 | binary `+ - * // %`, parameter-less function returning expression | `arith_constants`, `precedence` |
| `v0.3` — function args | `def main(a: int, b: int) -> int`; argv parsing in the wrapper; `strategy.toml` support in the bench | `add_two_ints` |
| `v0.4` — local variables | name binding, assignment, name resolution | `temp_vars` |
| `v0.5` — control flow | `if/else`, `while`, `return` from anywhere | `abs_value`, `loop_sum` |
| later slices… | type inference, classes, ABCs, ref counting, … | … |

Each slice gets its own spec file (or a section in an existing one) and is merged via PR before the next slice begins.

## Errors

Compiler errors are structured. Format (printed to stderr):

```
pyx86 error: <category>: <one-line summary>
 --> <file>:<line>:<col>
  |
  | <source line>
  |     ^^^^ <pointer + caret>
  |
  = note: <hint or expected/got pair>
```

Categories: `parse`, `unsupported_feature`, `type`, `name`, `internal`. The rule is **"reject rather than fall back"** (CLAUDE.md): unsupported features always emit `unsupported_feature` rather than silently being ignored.

## Out of scope for the overview spec

- Per-stage data structures and algorithms — see the per-stage specs.
- Optimization passes beyond what `clang -O2` does — none in v1.
- Incremental / cached compilation — none in v1.
- Multi-module compilation — v1 compiles a single `.py` file.
