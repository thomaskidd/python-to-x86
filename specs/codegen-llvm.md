# Spec: LLVM IR codegen

## Responsibility

Take the validated program (eventually: typed HIR / SSA mid-IR) and emit LLVM IR as text. Drive the final assemble + link step by shelling out to `clang`.

## Current scope (v0.6)

`main(<a>: int, ...)` (up to 16 typed-int params) has a body of statements (assignments, `if`/`elif`/`else`, `while`, `break`, `continue`, `pass`, `return`) that provably returns on every path. Expressions are unchanged from v0.5.

No `and`/`or`, no `for`, no calls yet.

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

## v0.3 IR — function parameters and argv-driven wrapper

When `py_main` has parameters, each gets an i64 SSA argument named `%p_<param-name>`. The wrapper (`@main`) receives `(argc, argv)` and parses `argv[i+1]` for each parameter via `atoll`.

```llvm
declare i64 @atoll(i8*)

define i64 @py_main(i64 %p_a, i64 %p_b) {
entry:
  %v0 = add i64 %p_a, %p_b
  ret i64 %v0
}

define i32 @main(i32 %argc, i8** %argv) {
entry:
  %slot0 = getelementptr inbounds i8*, i8** %argv, i64 1
  %str0  = load i8*, i8** %slot0
  %p_a   = call i64 @atoll(i8* %str0)
  %slot1 = getelementptr inbounds i8*, i8** %argv, i64 2
  %str1  = load i8*, i8** %slot1
  %p_b   = call i64 @atoll(i8* %str1)
  %r     = call i64 @py_main(i64 %p_a, i64 %p_b)
  %fmt   = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0
  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)
  ret i32 0
}
```

Naming convention: `%p_<name>` for parameters, `%v<n>` for internal SSA values, `%slot<i>` / `%str<i>` for the argv-parsing scaffold. The `p_` prefix avoids collisions when a user names a parameter like `v0`.

`atoll` returns 0 on parse failure rather than reporting an error. v0.3 trusts the caller (the bench) to pass well-formed integers; argv-validation lands when we add error handling.

## v0.4 IR — locals as pure SSA

The function body is now a sequence of statements. Codegen maintains a `HashMap<String, String>` mapping HIR variable names to LLVM operands. The map is **seeded with parameters** (`"a" → "%p_a"`) so `Var(name)` lookups uniformly resolve both params and locals.

For each `Stmt::Let { name, value }`:
- Lower `value`, get an operand (literal or SSA name).
- Insert (or **overwrite**) `name → operand` in the map.

For `Stmt::Return { value }`:
- Lower `value`, emit `ret i64 <operand>`.

There are **no `alloca` / `store` / `load` instructions** in v0.4. Locals are pure SSA. Reassignment is a map overwrite, which produces a fresh SSA name on the next emitted instruction; the previous SSA name becomes dead and LLVM's DCE removes it.

```python
def main(a: int) -> int:
    x = a + 1
    y = x * 2
    return y
```

emits

```llvm
define i64 @py_main(i64 %p_a) {
entry:
  %v0 = add i64 %p_a, 1     ; x
  %v1 = mul i64 %v0, 2      ; y
  ret i64 %v1
}
```

Aliasing assignments like `x = a` emit no instruction at all: the map gets `"x" → "%p_a"` and subsequent `Var("x")` lookups return `"%p_a"` directly.

When v0.5 adds branching, this scheme stops working — locals modified in a then-branch can't be SSA-renamed without `phi` nodes. The plan is to switch locals to `alloca`+`load`/`store` in v0.5 and let LLVM's `mem2reg` collapse them back to SSA at `-O1+`. Hand-rolled phis are not on the menu.

## v0.5 IR — alloca-based locals + branching

### Why alloca

Once `if`/`else` is in the picture, a local assigned inside one branch and used after the merge can't be a single SSA name. Hand-rolling `phi` nodes is annoying and error-prone. We follow the LLVM "Kaleidoscope" pattern: **every local (and every parameter, for uniformity) gets an `alloca i64` slot at function entry**. Reads emit `load`, writes emit `store`, and LLVM's `mem2reg` pass (active at `-O1+`, and we always run `-O2`) collapses the slot back into SSA + phi.

### Function prologue

For every parameter and every local name (collected by walking the HIR body):

```llvm
entry:
  %a.addr = alloca i64
  store i64 %p_a, i64* %a.addr
  %x.addr = alloca i64
  …
```

A param and a local with the same name share the slot (we don't model Python's UnboundLocalError here — accept this as a pragmatic deviation).

### Statement codegen

| Stmt | Emitted |
|---|---|
| `Let { name, value }` | lower `value` → operand; `store i64 <op>, i64* %<name>.addr` |
| `Return { value }` | lower `value` → operand; `ret i64 <op>`; mark current block terminated |
| `If { cond, then, else }` | lower `cond` → i1; `br i1 <c>, label %then.N, label %else.N`; emit then-block + else-block + merge-block |
| `While { cond, body }` | `br label %loop_header.N`; emit header (cond + br to body or exit), body (with loop targets pushed, back-edge to header), exit (continuation point) |
| `Break` | `br label %<top-of-loop-stack break_target>` |
| `Continue` | `br label %<top-of-loop-stack continue_target>` |

`If` blocks use a single per-statement id `N` so labels read `then.0`, `else.0`, `merge.0`, `then.1`, … . When a branch's body terminates (returns), we skip the trailing `br label %merge.N`. If both branches terminate, the merge block is emitted but is dead — LLVM tolerates this and DCE removes it. If the function falls off the end without a terminator (shouldn't happen — `check.rs` enforces every-path-returns), we emit `unreachable`.

### Conditions and value contexts

Conditions are i1; values are i64. Helpers:

- **`lower(e)` (value context)**: produces an i64 operand. For `Cmp`/`CmpChain`/`Not`, internally lowers to i1 then `zext i1 → i64`.
- **`lower_cond(e)` (condition context)**: produces an i1 operand. For `Cmp`/`CmpChain`/`Not`, lowers directly to i1 (skipping the zext). For everything else, lowers to i64 then emits `icmp ne i64 %v, 0`.

The extra zext/cmp pair on the value-context side is trivially eliminated by LLVM.

### Comparison codegen

| `CmpOp` | LLVM `icmp` predicate |
|---|---|
| `Lt` | `slt` |
| `Le` | `sle` |
| `Gt` | `sgt` |
| `Ge` | `sge` |
| `Eq` | `eq` |
| `Ne` | `ne` |

`Cmp { op, lhs, rhs }`:
```llvm
%v = icmp <pred> i64 %lhs, %rhs
```

`CmpChain { first, rest }` (Python's `a < b < c < d`): emit each pairwise comparison and `and` the i1 results.
```llvm
%c1 = icmp <op0> i64 %first, %r0
%c2 = icmp <op1> i64 %r0, %r1
%c12 = and i1 %c1, %c2
…
```
Operands are pure in v0.5 (no calls, no side effects), so duplicate evaluation across `prev`/`next` boundaries is harmless. LLVM CSEs identical loads.

`Not(inner)`:
- Inner is i1 (`Cmp`, etc.) → `xor i1 %inner, true`.
- Inner is i64 → lower as condition (i.e. `icmp ne 0`) then `xor i1, true`.

Both end up as i1; the value-context wrapper zexts to i64 if needed.

## v0.6 IR — while loops, break, continue

For each `Stmt::While { cond, body }` with stmt id `N`:

```llvm
  br label %loop_header.N
loop_header.N:
  <cond lowered to i1 %c>
  br i1 %c, label %loop_body.N, label %loop_exit.N
loop_body.N:
  <body lowered>
  br label %loop_header.N        ; back-edge (omitted if body terminated)
loop_exit.N:
```

Codegen maintains `Vec<(continue_target, break_target)>` as a stack. Entering a `While` pushes `(loop_header.N, loop_exit.N)`; exiting pops. `Stmt::Break` and `Stmt::Continue` always read the **top** of the stack — they jump to the innermost enclosing loop, matching Python semantics.

`Break` / `Continue` mark the current block as terminated, so the back-edge `br` from the body bottom is suppressed when the body itself jumped out. Same pattern as the `Return`-in-`If` case.

Loop labels share `next_block_id` with `if` labels: a `while` after an `if` may produce `loop_header.5` / `loop_body.5` / `loop_exit.5`. The id is unique per stmt, not per kind. This was a deliberate v0.5 choice and continues to read well.

LLVM's loop optimizations (`loop-rotate`, `licm`, `indvars`, `loop-unroll`, …) all run at `-O2` and operate fine on this header-test-body shape — equivalent to what clang generates for C `while`.

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
