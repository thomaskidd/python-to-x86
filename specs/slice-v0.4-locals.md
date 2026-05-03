# Spec: slice v0.4 — local variables

> Status: in progress.

## What v0.4 adds

`main()` may bind local variables and reference them in subsequent expressions:

```python
def main(a: int, b: int) -> int:
    x = a + b
    y = x * 2
    return y - 1
```

- Plain assignment: `name = <expr>`
- Annotated assignment: `name: int = <expr>` (annotation must be `int`)
- Reassignment: `x = 1; x = x + 1` — allowed (no SSA pain because v0.4 has no control flow yet)
- Reading a local in a later expression
- Multi-statement function body: a sequence of assignments terminated by exactly one `return`

## What v0.4 does **not** add

- Augmented assignment: `x += 1` (lands when we do operator desugaring; deferred)
- Tuple unpacking: `a, b = (1, 2)` (deferred)
- Multiple targets: `a = b = 1` (deferred — Python allows but it's syntactic sugar)
- Walrus operator `:=` (deferred)
- Branching or loops (v0.5)
- `del` statement (likely never)
- `global` / `nonlocal` (no nested functions yet)

## Architectural changes

### HIR: function body becomes a list of statements

```rust
pub enum Expr {
    ConstI64(i64),
    Var(String),                       // RENAMED from Param: any name in scope
    BinOp { … },
    UnaryOp { … },
}

pub enum Stmt {
    /// `x = <expr>` (annotation, if any, was already validated to be `int`)
    Let { name: String, value: Expr },
    /// `return <expr>` — must be the last statement in the body.
    Return { value: Expr },
}

pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: Vec<Stmt>,               // was: body: Expr
}
```

`Expr::Param` is renamed to `Expr::Var` since locals and parameters share the same scope and resolve through the same lookup. Codegen distinguishes them only by the SSA name they're bound to.

### Check (lower)

Walk the function body:
- Maintain a `HashSet<String>` (or `HashMap<String, Type>` once we have multiple types) of in-scope names, seeded with the parameter names.
- For each `ast::Stmt::Assign(targets=[Name(x)], value=e)`: lower `e`, add `x` to the in-scope set.
- For each `ast::Stmt::AnnAssign(target=Name(x), annotation=Name("int"), value=Some(e))`: same, additionally validate the annotation is `int`.
- For `ast::Stmt::Return(Some(e))`: lower `e`, terminate body. Must be the **last** statement; statements after `return` are an `unsupported_feature` error.
- Anything else in the body produces `unsupported_feature`.

References to a name not currently in scope produce `unsupported_feature: name '{x}' is not bound`.

### Codegen

Locals are SSA values. Codegen maintains a `HashMap<String, String>` mapping HIR variable names to LLVM operands (e.g. `"x" → "%v3"`, `"a" → "%p_a"`).

- For `Stmt::Let { name, value }`: lower the expression, get its operand, insert `name → operand` into the map. **Reassignment overwrites** the existing entry — fine in v0.4 because we have straight-line code; once branches exist, this needs phi nodes.
- For `Stmt::Return { value }`: lower the expression, emit `ret i64 <operand>`.
- For `Expr::Var(name)`: look up the operand in the map; if absent, that's an internal error (check should have caught it).

There are **no `alloca` / `store` / `load` instructions**. Locals are pure SSA values. This relies on our straight-line invariant.

When v0.5 adds control flow, we'll switch locals to `alloca` + `load`/`store` (the simple route LLVM's `mem2reg` pass cleans up at `-O1+`), which avoids hand-rolling phi nodes.

## Test programs

| Test | Purpose |
|---|---|
| `temp_vars` | `x = a + b; y = x * 2; return y - 1` — basic local binding chain |
| `reassignment` | `x = a; x = x + 1; x = x * 2; return x` — overwrite path |
| `local_chain` | 8 locals with each defined in terms of the previous — ensures the SSA map scales |
| `local_with_annotation` | `x: int = a + 1; return x` — annotated assignment |

All tier 1, with `iter_at.tier1 = 5` (5 random inputs each).

## Files changed from v0.3

- `crates/pyx86/src/hir.rs` — `Expr::Param` → `Expr::Var`; new `Stmt` enum; `Function.body` becomes `Vec<Stmt>`.
- `crates/pyx86/src/check.rs` — handle assignment + annotated assignment; track in-scope names; lower `Name` to `Var`; statements after `return` rejected.
- `crates/pyx86/src/codegen.rs` — `Codegen` gains a `vars: HashMap<String, String>`; lower body statement-by-statement; `Var` looks up the operand.
- `crates/pyx86_bench/src/main.rs` — no change needed.
- `tests/correctness/{temp_vars, reassignment, local_chain, local_with_annotation}/` — new programs.
- `specs/check.md`, `specs/codegen-llvm.md`, `specs/hir.md` (new) — updated.
