# Spec: HIR

## Responsibility

The high-level intermediate representation. Sits between the validated Python AST (`rustpython-ast::ModModule`) and the LLVM IR text emitted by codegen.

The HIR is the source of truth for "what the compiler can compile." Every accepted Python program is lowered into a `hir::Program` by `check.rs`, and codegen consumes that. If a construct exists in the HIR, codegen must handle it; if it doesn't, check must reject the corresponding Python.

This module is intentionally tiny and grows slice by slice. It is not a "permanent" IR design — it accretes shape as features land.

## Current shape (v0.5)

```rust
pub enum Type { I64 }                  // only one user-facing type so far

pub struct Param { pub name: String, pub ty: Type }

pub struct Function {
    pub name: String,                  // always "main" in v0.5
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: Vec<Stmt>,               // every-path-return enforced by check
}

pub struct Program { pub main: Function }

pub enum Stmt {
    Let { name: String, value: Expr },
    Return { value: Expr },
    If { cond: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
}

pub enum Expr {
    ConstI64(i64),
    Var(String),
    BinOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
    Cmp { op: CmpOp, lhs: Box<Expr>, rhs: Box<Expr> },
    CmpChain { first: Box<Expr>, rest: Vec<(CmpOp, Expr)> },
    Not(Box<Expr>),
}

pub enum BinOp { Add, Sub, Mul, FloorDiv, Mod }
pub enum UnaryOp { Neg, Pos }
pub enum CmpOp { Lt, Le, Gt, Ge, Eq, Ne }
```

## Invariants

- `Function.body` provably returns on every path. The check module enforces this conservatively (last stmt is `Return` or an `If` whose two branches both recursively cover).
- All `Type` values are `I64` in v0.5. `Cmp`/`CmpChain`/`Not` produce a logical Bool internally; in value context they're zext'd to i64 (0 or 1).
- `Expr::Var(name)` references a name that is either a parameter or has been bound by a preceding `Stmt::Let` in the same surrounding scope. `check.rs` enforces this; codegen panics if it sees an unbound name (treated as internal compiler bug).
- `BinOp::FloorDiv` / `BinOp::Mod` follow Python semantics (floor toward -∞). Codegen emits the correction blocks documented in `specs/codegen-llvm.md`.
- `If.else_body` is `Vec::new()` when there is no `else` clause.
- `CmpChain.rest` is non-empty (single comparisons use `Cmp` instead).
- Operands are pure (no side effects). Codegen exploits this to avoid name-tracking for chained comparisons.

## What the HIR deliberately does **not** model yet

These are listed so a future contributor adding the corresponding feature knows the HIR has to grow:

- Function calls (no `Call` Expr — calls are not supported)
- Loops (no `While` / `For` / labels — v0.6+)
- `and`, `or` boolean operators (deferred — short-circuit value semantics need careful design)
- Multiple types beyond i64 (no user-facing `bool`, `f64`, `str`, container types)
- Multiple functions (`Program` holds exactly one `Function`)
- Mutability beyond reassignment (no `&mut`-style, no observable side effects on Vars)
- Closures, nested function defs
- Type coercions (everything is i64 in value context; cmp/not internally i1)

When any of those land, both the HIR and `specs/hir.md` get a new section.

## Where the HIR is consumed

- `check.rs` constructs `hir::Program` and is the one place that writes to it.
- `codegen.rs` reads `hir::Program` and emits LLVM IR text.

No other module touches HIR. If a future stage (e.g. an SSA mid-IR or an optimizer pass) is added between check and codegen, it must take HIR in and produce something else, not mutate HIR in place.
