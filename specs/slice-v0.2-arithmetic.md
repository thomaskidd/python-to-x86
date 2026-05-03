# Spec: slice v0.2 — arithmetic on i64 constants

> Status: **planned**, not implemented. Lands after v0.1 passes tier 2.

## What v0.2 adds

`main()` may return any expression composed of:
- `int` literals (in i64 range)
- Binary ops: `+ - * // %`
- Unary ops: `-x`, `+x`
- Parentheses (handled by the parser; the AST is already a tree)

```python
def main() -> int:
    return (1 + 2) * 3 - 100 // 7
```

Still no parameters, no variables, no control flow. Those come in v0.3+.

## What v0.2 does **not** add

- `/` (true division — produces float, not in scope until floats land)
- `**` (exponentiation — semantics on int are arbitrary-precision in Python; deferred until we figure out i64 wrap behavior on `**`)
- Comparison or boolean operators
- Bitwise operators (deferred to v0.4 or wherever we add `int` operators wholesale)

## Architectural change: introduce a tiny IR

v0.1 had a degenerate IR (just an `i64` value). v0.2 needs an expression tree, so we add a small IR module:

```rust
// crates/pyx86/src/hir.rs (new in v0.2)
pub enum Expr {
    ConstI64(i64),
    BinOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
}

pub enum BinOp { Add, Sub, Mul, FloorDiv, Mod }
pub enum UnaryOp { Neg, Pos }

pub struct Program {
    pub main_return: Expr,
}
```

This replaces `check::Program { return_value: i64 }`. The check stage now lowers the Python AST into this `hir::Program` instead of evaluating it.

Codegen lowers `Expr` to LLVM IR via straightforward post-order traversal, producing SSA values:

```
ret i64 <expr_result>
```

## Semantics: i64 wrap and Python divergence

CPython integers are arbitrary precision; we are i64. The bench's differential test will catch divergence. Two known divergence points:

1. **Overflow**: `2**62 + 2**62` overflows i64 silently (wraps); CPython produces `2**63` correctly. The test corpus must avoid inputs that overflow until the user opts into a bigint type.
2. **Floor div / mod with negative operands**: Python uses *floor* semantics (`-7 // 2 == -4`, `-7 % 2 == 1`); LLVM's `sdiv`/`srem` use *truncation toward zero* (`-7 / 2 == -3`, `-7 % 2 == -1`). We must emit floor-correcting code, not naive `sdiv`/`srem`. Sketch:
   ```
   ; floor_div(a, b) = sdiv(a, b) - (sign-mismatch && remainder != 0 ? 1 : 0)
   ```
   The bench's differential test against CPython is the source of truth — the test program `floordiv_negatives` exercises this and must match exactly.

## Checks v0.2 adds (rejection list, kept honest)

The check pass walks the expression tree. Anything that is not a `Constant(int)`, `BinOp`, or `UnaryOp(USub|UAdd)` produces `unsupported_feature`. In particular:
- Names (variables) → still rejected. v0.3.
- Function calls → still rejected.
- True division `/`, exponentiation `**` → rejected explicitly with a "not in v0.2" note.
- Comparison operators → rejected.

## Test programs added

| Test | Purpose |
|---|---|
| `arith_constants` | `return 1 + 2 * 3` — basic arithmetic + precedence |
| `arith_unary` | `return -5 + +3` |
| `arith_floordiv` | `return 100 // 7` |
| `floordiv_negatives` | `return -7 // 2`, `return -7 % 2` — pins the floor-correction semantics |
| `arith_mixed` | `return (1 + 2) * (3 - 4) // 5` — combines everything |

All are tier 1 (no inputs, single iteration).

## File-by-file delta from v0.1

- `crates/pyx86/src/hir.rs` — new
- `crates/pyx86/src/check.rs` — rewrite: lower AST → `hir::Program`
- `crates/pyx86/src/codegen.rs` — extend: lower `hir::Expr` → LLVM IR with SSA value names
- `crates/pyx86/src/main.rs` — no change (driver shape stays the same)
- `tests/correctness/arith_*` — five new test programs
- `specs/parser.md` — no change (parser already accepts arbitrary Python; check is what gates)
- `specs/codegen-llvm.md` — extended with the v0.2 codegen template
- `specs/check.md` — new (was implicit in compiler-overview.md, now warrants its own spec)
