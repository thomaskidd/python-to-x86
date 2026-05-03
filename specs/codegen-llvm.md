# Spec: LLVM IR codegen

## Responsibility

Take the validated program (eventually: typed HIR / SSA mid-IR) and emit LLVM IR as text. Drive the final assemble + link step by shelling out to `clang`.

## Current scope (v0.2)

`main()` returns an arbitrary integer expression composed of:
- `int` literals (i64-range)
- Binary `+ - * // %`
- Unary `+x`, `-x`
- Parentheses

No parameters, no variables, no control flow yet — those are v0.3+.

## v0.1 IR template (literal return)

For the simplest case, `def main() -> int: return <int literal>`, codegen emits:

```llvm
; ModuleID = 'pyx86_<source basename>'
target triple = "x86_64-unknown-linux-gnu"

declare i32 @printf(i8*, ...)

@.fmt_i64 = private unnamed_addr constant [5 x i8] c"%ld\0A\00"

define i64 @py_main() {
entry:
  ret i64 <CONST>
}

define i32 @main() {
entry:
  %r = call i64 @py_main()
  %fmt = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0
  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)
  ret i32 0
}
```

Key points:
- The user's `main` is renamed to `py_main` to avoid colliding with the C `main` symbol that libc startup expects.
- The C `main` is the wrapper: it calls `py_main`, prints the return value via `printf("%ld\n", r)`, and returns 0.
- `printf` is from libc; `clang` links libc by default. No custom runtime crate is needed for this slice.
- We use **typed pointers (`i8*`)** rather than opaque pointers (`ptr`) so the IR is accepted by LLVM 10 through 18+. Opaque pointers are LLVM 14+ only; if/when we drop support for older LLVMs, the IR can be simplified.

## v0.2 IR — expression lowering

`Expr` is lowered post-order; each `lower(...)` call returns the LLVM operand (a literal like `42` or an SSA name like `%v3`) holding the expression's value, and appends any necessary instructions.

### Trivially translated ops

| HIR op | LLVM |
|---|---|
| `Add` / `Sub` / `Mul` | `add` / `sub` / `mul` `i64` |
| `UnaryOp::Neg` | `sub i64 0, <operand>` |
| `UnaryOp::Pos` | passthrough (no instruction) |

### Floor-div correction (`a // b`)

LLVM `sdiv` truncates toward zero; Python `//` floors toward -∞. They diverge for mixed-sign operands (`-7 // 2 == -4` in Python; `sdiv(-7, 2) == -3`).

Correction: `result = sdiv(a, b) + adj`, where `adj = -1` iff the remainder is non-zero AND the operand signs differ:

```llvm
%q     = sdiv i64 %a, %b
%rem   = srem i64 %a, %b
%nz    = icmp ne i64 %rem, 0
%xor   = xor i64 %a, %b
%diff  = icmp slt i64 %xor, 0      ; signs differ ⇔ xor < 0
%need  = and i1 %nz, %diff
%adj   = sext i1 %need to i64       ; true → -1, false → 0
%out   = add i64 %q, %adj
```

### Floor-mod correction (`a % b`)

LLVM `srem` returns a value with the dividend's sign; Python `%` returns one with the divisor's sign. Correction: take `srem`, and add `b` iff the remainder is non-zero AND its sign differs from `b`'s.

```llvm
%rem   = srem i64 %a, %b
%nz    = icmp ne i64 %rem, 0
%xor   = xor i64 %rem, %b
%diff  = icmp slt i64 %xor, 0
%need  = and i1 %nz, %diff
%adj   = select i1 %need, i64 %b, i64 0
%out   = add i64 %rem, %adj
```

These are pinned by the `floordiv_negatives` correctness test; CPython is the source of truth.

### Division-by-zero

We emit raw `sdiv` / `srem`; LLVM's behaviour for `a / 0` is to raise SIGFPE on x86-64. CPython raises `ZeroDivisionError`. v0.2 has no exception support — programs that divide by zero are diagnosed by the differential test. Test programs avoid div-by-zero until exceptions land in phase 2.

## Pipeline driven by codegen

```
HIR (or v0.1 simplified shape)
   │ codegen::emit_ll       ──▶  String containing LLVM IR
   │
   │ write to <tmp>/program.ll
   │
   │ shell out:
   │    clang -O<n> -o <output.elf> <tmp>/program.ll
   ▼
ELF binary
```

`--emit=ll` stops after writing the .ll file (useful for debugging and for golden tests).
`--emit=asm` runs `clang -S -O<n> ... -o <output>.s`.
`--emit=elf` (default) runs `clang -O<n> ... -o <output>` to produce the executable.

## External dependencies at runtime

- `clang` on `$PATH`. If absent, the compiler exits with:

  ```
  pyx86 error: internal: required tool `clang` not found on PATH
    = note: install LLVM (e.g. `sudo apt install clang`) and retry
  ```

## Module structure (Rust)

```
crates/pyx86/src/
    main.rs         CLI driver
    parser.rs       (per parser.md)
    check.rs        v0.1 validator: confirm the program is in the supported v0.1 subset
                    and extract the constant value to be returned.
    codegen.rs      String-builder for LLVM IR.
    link.rs         Shells out to clang.
```

`check.rs` will evolve into `infer.rs` (full type inference) as later slices land. For v0.1 it does pattern matching on the AST.

## Codegen API (v0.1)

```rust
pub struct Program {
    pub return_value: i64,   // the literal returned by main()
}

pub fn emit_ll(prog: &Program, source_basename: &str) -> String;
```

This is intentionally narrow. Each slice widens the `Program` type and the `emit_ll` function. The HIR / mid-IR layers come online when there is something nontrivial to lower.

## Testing

Unit tests in `codegen.rs`:
- `emits_expected_ir_for_v0_1` — golden test: assert the emitted IR exactly matches an expected string for `Program { return_value: 42 }`.
- (later) golden tests per IR fragment as features land.

Integration is covered by the test bench: a `.py` program → `pyx86 program.py -o program.elf` → run → diff stdout against CPython.

## Optimization

`--opt-level` is forwarded to clang as `-O<n>`. v1 default is `-O2`. We do not run any optimization passes ourselves — LLVM/clang handle that. The "as fast as Rust" target is judged on `-O2` output.

## Out of scope for v1

- Cross-compilation (we always emit for the host triple).
- LTO, PGO, BOLT.
- Inline assembly emission (no `__asm__` Python equivalent).
- Linking against shared libraries beyond libc.
- Static linking of musl / no-libc builds.
