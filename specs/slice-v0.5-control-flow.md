# Spec: slice v0.5 — control flow (if/else, comparisons, early return)

> Status: in progress.

## What v0.5 adds

- **Comparisons**: `< <= > >= == !=` between i64 expressions, including chained (`a < b < c`).
- **`if` / `elif` / `else`** statements.
- **Early `return`** from any branch.
- **`not`** unary operator (only one of `and`/`or`/`not` for now — `and`/`or` come in v0.6 with their proper short-circuit value semantics).
- **Truthy conditions**: `if <int-expr>:` is allowed; the compiler inserts an implicit `!= 0` check, matching CPython.

```python
def main(a: int, b: int) -> int:
    if a < b:
        return a
    elif a > b:
        return b
    else:
        return 0
```

## What v0.5 does **not** add

- `while` loops (v0.6)
- `for` loops (v0.7+)
- `and` / `or` (v0.6 — short-circuit semantics with value-not-bool result is non-trivial)
- `break` / `continue` (need loops first)
- Boolean as a real type — internally i1 used only for branch conditions; values stored / returned remain i64
- Comparison chaining produces *one* combined Bool per Python semantics, not per pair (the current scope just needs to handle it)

## Architectural change: locals as `alloca` slots

Until v0.4, locals were pure SSA values that survived just because the body was straight-line. With branches, a variable assigned inside an `if`-branch and read after the branch can no longer be a single SSA name — it needs `phi` nodes at the merge point.

Hand-rolling `phi` is annoying. The well-trodden alternative used by clang itself: **emit every local as an `alloca i64` at function entry, with `store` on assignment and `load` on reference**. LLVM's `mem2reg` pass (enabled at `-O1` and above) collapses these back into SSA + phis automatically. We always run `-O2` for the bench, so the generated code is no slower than hand-rolled phi.

Parameters get the same treatment: a `%<name>.addr = alloca i64` plus `store i64 %p_<name>, i64* %<name>.addr` at the entry block. Subsequent `Var(<name>)` becomes `load i64, i64* %<name>.addr`.

This is the standard pattern from the LLVM "Kaleidoscope" tutorial; mem2reg and the rest of `-O2` produce essentially the same code as hand-rolled phi.

### HIR additions

```rust
pub enum Type { I64, Bool }            // Bool is internal — used only by Cmp / If condition / not

pub enum CmpOp { Lt, Le, Gt, Ge, Eq, Ne }

pub enum Expr {
    ConstI64(i64),
    Var(String),
    BinOp { … },
    UnaryOp { … },                     // Neg, Pos, NotI64 (eager `int` → bool-ish)
    Cmp {
        op: CmpOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },                                 // produces Bool
    /// Chained comparison: `a < b < c` lowers to:
    ///   And([Cmp(<, a, b), Cmp(<, b, c)])
    /// — implemented as nested Cmp at codegen time without
    /// duplicating the middle expression. v0.5 emits this as
    /// a sequence of compare + branch, not via a true `and`.
    CmpChain {
        first: Box<Expr>,
        rest: Vec<(CmpOp, Expr)>,
    },
    /// Logical-not on a Bool, or a truthy-coerce on i64.
    Not(Box<Expr>),
}

pub enum Stmt {
    Let { name: String, value: Expr },
    Return { value: Expr },
    /// `if <cond>: <then> [elif …]* [else <else>]`
    If {
        cond: Expr,                    // either Bool or i64 (auto-coerced)
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,          // empty Vec for no else
    },
}
```

### Check (lower)

- A new helper `lower_block(stmts, scope)` recursively lowers a `Vec<ast::Stmt>` into `Vec<hir::Stmt>`. The function body and each branch body call it.
- `if`/`elif`/`else` collapses into nested `Stmt::If`. Python's `elif` is sugar for `else: if`.
- `Return` may now appear anywhere in a block, not only as the last statement at top level. The "function ends with return" invariant is replaced with **"every path returns"** — too expensive to fully verify in v0.5, so we just require the function body itself ends with either `Return` or an `If` whose branches both end with `Return`. Anything else is rejected with `unsupported_feature: not all paths return a value`.
  - Simpler v0.5 enforcement: require the **last** statement to be either `Return` or a covering `If` (both branches end with `Return`), recursively. Conservative; rejects valid programs but never accepts invalid ones.
- Comparison ops (`Lt`, `LtE`, `Gt`, `GtE`, `Eq`, `NotEq`) lower to `Expr::Cmp` / `Expr::CmpChain`. Other comparison ops (`Is`, `IsNot`, `In`, `NotIn`) → `unsupported_feature`.
- `ast::UnaryOp::Not` → `Expr::Not(operand)`.
- `ast::BoolOp::And` / `Or` → still rejected (`unsupported_feature: \`and\` / \`or\` not yet supported`).

### Codegen

- **Function entry** allocates a stack slot for every parameter and every assigned local in the function. The list is computed up front by walking the HIR (a small "collect locals" pass).
- **Stmt::Let**: lower the value, emit `store i64 <op>, i64* %<name>.addr`.
- **Expr::Var**: emit `%<fresh> = load i64, i64* %<name>.addr`.
- **Stmt::If**:
  - Lower the condition; if its result type is i64, emit `icmp ne i64 %v, 0` to coerce to i1. If it's already i1 (a Cmp), use directly.
  - Emit `br i1 %cond, label %then.N, label %else.N`.
  - Emit `then.N` block, lower its statements, terminate with `br label %merge.N` *unless* the block already ended with a `ret` (in which case no fall-through is needed).
  - Same for `else.N` (empty else compiles to a direct `br label %merge.N`).
  - Emit `merge.N` block. (LLVM treats unreachable merge blocks fine; if both branches `ret`, the merge is dead and removed by DCE.)
- **Stmt::Return**: emit `%r = load …` (if returning a Var) or just lower the expr; emit `ret i64 %r`. After a `Return` in a block, codegen stops emitting for that block.
- **Expr::Cmp / CmpChain**: emit `icmp <kind> i64 %lhs, %rhs` producing an i1.
  - For `CmpChain`, sequentially `and` the i1 results (or short-circuit via branches — the simpler `and` is fine because we always evaluate both sides anyway in v0.5, since comparison operands have no side effects).
- **Expr::Not(inner)**:
  - If `inner` is Bool (i1) → emit `xor i1 %v, true`.
  - If `inner` is i64 → emit `icmp eq i64 %v, 0` (which is exactly "not truthy").

Block names are uniquified per function via a counter (e.g. `then.0`, `else.0`, `merge.0`, `then.1`, …).

### `clang -O2` cleanup

Without optimization, every Var read is a load and every Let is a store, so naive code is slow and looks like x86 with stack-spilled locals. With `clang -O2` (the default for `pyx86`), **mem2reg** runs early, collapsing the alloca/load/store pattern back to SSA + phi. The bench's perf comparison against Rust thus stays meaningful.

If the bench-time perf comparison ever shows a regression after this slice, run `pyx86 program.py -o /tmp/x.s --emit=asm --opt-level=2` and confirm the alloca/load/store patterns are gone — that's the smoke signal that mem2reg didn't fire.

## Test programs

| Test | Purpose |
|---|---|
| `abs_value` | `if a < 0: return -a; else: return a` — the canonical if/else |
| `max_of_two` | `if a > b: return a; else: return b` |
| `sign_function` | `if/elif/else` returning -1, 0, or 1 |
| `clamp` | `if a < 0: return 0; if a > 100: return 100; return a` — sequential ifs (each may early-return) |
| `truthy_int` | `if a: return 1; else: return 0` — exercises i64-as-condition coercion |
| `cmp_chain` | `if 0 < a < 100: return 1; else: return 0` |

All tier 1, with `iter_at.tier1 = 5`.

## Files changed from v0.4

- `crates/pyx86/src/hir.rs` — add `Type::Bool`, `CmpOp`, `Expr::Cmp`/`CmpChain`/`Not`, `Stmt::If`.
- `crates/pyx86/src/check.rs` — recursive `lower_block`; nested If construction from elif chains; comparisons; the "every path returns" check (conservative version).
- `crates/pyx86/src/codegen.rs` — alloca-based locals; stmt-by-stmt block emission with branch + merge; per-function unique block names.
- `tests/correctness/{abs_value, max_of_two, sign_function, clamp, truthy_int, cmp_chain}/` — new programs.
- `specs/check.md`, `specs/codegen-llvm.md`, `specs/hir.md` — updated.
