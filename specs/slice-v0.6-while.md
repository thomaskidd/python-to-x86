# Spec: slice v0.6 — while loops + break/continue

> Status: in progress.

## What v0.6 adds

- **`while <cond>:`** loops with the same condition rules as `if` (i64 truthy via implicit `!= 0`, or a bool-producing expression).
- **`break`** to exit the innermost enclosing loop.
- **`continue`** to jump to the next iteration of the innermost enclosing loop.
- (No `else` clause on `while` — Python supports it but it's rarely useful and trivially backed out.)

```python
def main(n: int) -> int:
    i: int = 0
    total: int = 0
    while i < n:
        total = total + i
        i = i + 1
    return total
```

## What v0.6 does **not** add

- `for` loops (no iterators yet — needs container types)
- `else` on `while`
- `and` / `or` boolean operators (deferred to v0.7)
- Any infinite-loop detection / bound on iteration count

## HIR additions

```rust
pub enum Stmt {
    Let { name: String, value: Expr },
    Return { value: Expr },
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    While { cond: Expr, body: Vec<Stmt> },           // NEW
    Break,                                           // NEW
    Continue,                                        // NEW
}
```

`Expr` and the rest of the HIR are unchanged.

## Check (lower)

- New AST handler for `ast::Stmt::While`. Reject `while … : … else: …` (the `orelse` field non-empty) with `unsupported_feature`.
- New AST handlers for `ast::Stmt::Break` and `ast::Stmt::Continue`. Both record their position; the check pass also tracks "are we inside a loop" via a `loop_depth` counter (or stack) to reject break/continue outside any loop.
- The `block_always_returns` check is updated:
  - `While { ... }` is **not a covering construct** — a `while` may execute zero iterations. So `while …: return …` followed by nothing produces "not all paths return."
  - `Break` and `Continue` are also not covering. Functions that need to provably return after a loop must end with `return` (or a covering `if`).

## Codegen

For `Stmt::While { cond, body }` with stmt id `N`:

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

For nested loops, codegen pushes the `(continue_target = loop_header.N, break_target = loop_exit.N)` pair onto a stack. `Stmt::Break` emits `br label %<break_target>` from the top of the stack and marks the current block terminated; `Stmt::Continue` emits `br label %<continue_target>` similarly. After lowering the body, codegen pops the stack.

If the body terminates (via return, break, continue) without falling through to the back-edge, the back-edge `br` is skipped — same pattern as the existing `If` handling.

The alloca scheme established in v0.5 needs no change; locals defined inside a loop body are allocated up front by `collect_locals`, which already walks all nested bodies (extending it to walk `While.body` is a one-line change).

`block_terminated` resets to false when each new labeled block opens (`loop_header.N`, `loop_body.N`, `loop_exit.N`).

### Codegen notes

- Loop labels share the same `next_block_id` as `if` labels: `loop_header.5` / `then.5` etc. (The id is unique per stmt, not per kind.) This was a deliberate choice in v0.5 and continues to read well.
- LLVM's loop optimizations (`-O2` runs `loop-rotate`, `licm`, `indvars`, etc.) all work fine on the `loop_header` → `loop_body` → back-edge shape generated here.

## Test programs (tier 1, 5 inputs each)

| Test | Purpose |
|---|---|
| `loop_sum` | `while i < n: total += i; i += 1; return total` — canonical accumulating loop. (We have `=` not `+=`, so use `total = total + i`.) |
| `factorial` | `while n > 1: r = r * n; n = n - 1; return r` |
| `gcd` | Euclidean algorithm with `while b != 0` and reassignment of both vars |
| `loop_with_break` | `while True: if i >= n: break; ...; return ...` |
| `loop_with_continue` | accumulate only odd values via `if i % 2 == 0: i = i+1; continue` |

Strategy ranges chosen to keep iterations bounded (e.g. `gcd` uses small positive ints; `factorial` keeps `n ≤ 12` so the result fits in i64).

## Files changed from v0.5

- `crates/pyx86/src/hir.rs` — add `Stmt::While`, `Stmt::Break`, `Stmt::Continue`.
- `crates/pyx86/src/check.rs` — handle `While`/`Break`/`Continue`; reject `else` on `while`; track loop depth; extend `block_always_returns` to treat `While` as not covering.
- `crates/pyx86/src/codegen.rs` — `Codegen` gains a `Vec<(String, String)>` stack of `(continue_target, break_target)`; new emission for `While`/`Break`/`Continue`; `collect_locals` extended to walk while bodies.
- `tests/correctness/{loop_sum, factorial, gcd, loop_with_break, loop_with_continue}/` — new programs.
- `specs/check.md`, `specs/codegen-llvm.md`, `specs/hir.md` — updated.
