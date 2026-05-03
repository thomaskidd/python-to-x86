# Spec: HIR

## Responsibility

The high-level intermediate representation. Sits between the validated Python AST (`rustpython-ast::ModModule`) and the LLVM IR text emitted by codegen.

The HIR is the source of truth for "what the compiler can compile." Every accepted Python program is lowered into a `hir::Program` by `check.rs`, and codegen consumes that. If a construct exists in the HIR, codegen must handle it; if it doesn't, check must reject the corresponding Python.

This module is intentionally tiny and grows slice by slice. It is not a "permanent" IR design — it accretes shape as features land.

## Current shape (v0.4)

```rust
pub enum Type { I64 }                  // only one type so far

pub struct Param { pub name: String, pub ty: Type }

pub struct Function {
    pub name: String,                  // always "main" in v0.4
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: Vec<Stmt>,               // ends with exactly one Return
}

pub struct Program { pub main: Function }

pub enum Stmt {
    Let { name: String, value: Expr },
    Return { value: Expr },
}

pub enum Expr {
    ConstI64(i64),
    Var(String),                       // parameter or local
    BinOp { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr> },
    UnaryOp { op: UnaryOp, operand: Box<Expr> },
}

pub enum BinOp { Add, Sub, Mul, FloorDiv, Mod }
pub enum UnaryOp { Neg, Pos }
```

## Invariants

- `Function.body` is non-empty and its **last** statement is `Stmt::Return`. There is no early return; that requires control flow (v0.5).
- All `Type` values are `I64` in v0.4. Codegen does not need a type-dispatch yet.
- `Expr::Var(name)` references a name that is either a parameter of the enclosing function or has been bound by a preceding `Stmt::Let` in the same body. `check.rs` enforces this; codegen panics if it sees a name it doesn't recognize (treated as an internal compiler bug).
- `BinOp::FloorDiv` / `BinOp::Mod` follow Python semantics (floor toward -∞), not LLVM `sdiv` / `srem` semantics. Codegen emits the correction blocks documented in `specs/codegen-llvm.md`.

## What the HIR deliberately does **not** model yet

These are listed so a future contributor adding the corresponding feature knows the HIR has to grow:

- Function calls (no `Call` Expr — calls are not supported)
- Control flow (no `If` / `While` / labels)
- Multiple types beyond i64 (no `bool`, `f64`, `str`, container types)
- Multiple functions (`Program` holds exactly one `Function`)
- Mutability beyond reassignment (no `&mut`-style, no observable side effects on Vars)
- Closures, nested function defs
- Type coercions (everything is i64; no widen/narrow casts)

When any of those land, both the HIR and `specs/hir.md` get a new section.

## Where the HIR is consumed

- `check.rs` constructs `hir::Program` and is the one place that writes to it.
- `codegen.rs` reads `hir::Program` and emits LLVM IR text.

No other module touches HIR. If a future stage (e.g. an SSA mid-IR or an optimizer pass) is added between check and codegen, it must take HIR in and produce something else, not mutate HIR in place.
