# Spec: slice v0.38 — lambdas + closures

> Status: in progress.

## What v0.38 adds

- **`Callable[[T1, T2, ...], R]`** — a first-class function type
  (imported from `typing`, recognized syntactically).
- **`lambda x, y: <expr>`** — anonymous function value. Type inferred
  from the context's expected `Callable` type.
- **Closures** — lambdas may reference enclosing-function locals; the
  captures are packed by value into a heap-allocated env struct.
- **Indirect call** through a callable value: `f(args)` where `f` has
  type `Callable`.

```python
from typing import Callable

def make_adder(n: int) -> Callable[[int], int]:
    return lambda x: x + n

def apply(f: Callable[[int], int], x: int) -> int:
    return f(x)

def main(a: int, b: int) -> int:
    add = make_adder(a)
    return apply(add, b)        # → a + b
```

## What v0.38 does **not** add

- **Annotated lambdas** — Python doesn't allow them; we follow suit.
  All lambda parameter types come from the slot's expected `Callable`
  type. If we can't determine it (e.g., bare `x = lambda y: y`), we
  reject.
- **`def`-nested closures** — only `lambda` lifts to a closure for
  v0.38. Nested `def`s are deferred.
- **Mutation of captured vars** from inside the lambda — captures are
  by value; assignment within the lambda creates a new local.
- **Recursive lambdas** — Y-combinator-style. Deferred.
- **`Callable[..., R]`** with ellipsis — must specify the param list
  explicitly.

## HIR additions

```rust
pub struct CallableId(u32);
Type::Callable(CallableId)              // LLVM: { i8* fn, i8* env }
// CallableId interns (params: Vec<Type>, ret: Type).

Expr::LambdaValue {
    fn_name: String,                    // mangled top-level name `__lambda.<idx>`
    env_fields: Vec<(String, Type)>,    // captured-var names + types
    env_init: Vec<TypedExpr>,           // values to write into env at creation
    callable_ty: Type,                  // Type::Callable(...)
}
Expr::IndirectCall {
    callee: Box<TypedExpr>,             // type Callable
    args: Vec<TypedExpr>,               // user args (no env)
    return_ty: Type,
}
```

Each lambda also produces a top-level `Function` in `Program.functions`
named `__lambda.<idx>`. Its first parameter is the env pointer; the
rest are the lambda's declared params.

## Check (lower)

**Annotation:** `Callable[[T1, T2, ...], R]` parsed via
`parse_type_annotation`. Recognize `Callable` as the head of a
subscript whose slice is a tuple `(<param_list>, <return_type>)`. The
param list is itself a `List` or a single type.

**Lambda lowering** is **context-driven**: a `lambda` expression can
only be lowered when an expected `Callable` type is known. The lower
flow:

1. When lowering an argument / let-rhs / return, we know the target
   slot's type.
2. Detect `ast::Expr::Lambda` in that lowering. Walk the AST to find
   the slot's expected type:
   - In a function call site, the slot type = param's declared type.
   - In an annotated assignment, the slot type = declared type.
   - In a return, the slot type = function's declared return.
3. If expected type is not `Type::Callable`, reject.
4. Bind lambda's positional args to the callable's param types.
5. Lower the body in a temporary scope that includes the lambda's
   params **plus the enclosing scope**.
6. Walk the lowered body to find free variables: any `Expr::Var(name)`
   where `name` is not a lambda param. Each such name's type comes
   from the enclosing scope.
7. Build an env struct (`Vec<(name, type)>`) of unique free-var names.
8. Emit a top-level function `__lambda.<idx>` with params
   `(env: env_struct_ptr, ...lambda_params...)`. The body references
   captures as `env->name`.
9. Emit `Expr::LambdaValue` carrying the env contents.

**Calling a callable** is straightforward: `f(a, b)` where `f.ty ==
Type::Callable(...)`. Verify arg count + types, emit
`Expr::IndirectCall`.

## Codegen

`Type::Callable(_)` lowers to `{ i8*, i8* }` (value-typed fat pointer:
function pointer + env pointer). 16 bytes.

`Expr::LambdaValue`:
- Allocate env struct on heap (`malloc(size_of(env))`).
- Store each captured value into its field.
- Pack `{fn_ptr_as_i8*, env_ptr_as_i8*}` into a `{i8*, i8*}` value.

`Expr::IndirectCall`:
- Extract fn + env from the callable value.
- Bitcast fn to `<return> (i8*, <arg_types>...)*`.
- Indirect-call with env prepended.

Lifted lambda function:
- Defined as a normal `Function` with first param of type
  `Type::I64` masquerading as `i8*`. Actually we use a special
  internal type: we synthesize the function with a leading `env: i8*`
  param. Inside, we bitcast it to the env struct type and access
  captures via GEP.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `lambda_no_capture` | `apply(lambda x: x * 2, 5)` |
| `lambda_capture_one_int` | `make_adder(a)` returns `lambda x: x + a`; call it |
| `lambda_capture_multi` | captures 2+ locals of mixed types |
| `lambda_in_let` | `f: Callable[[int], int] = lambda x: x + 1; f(7)` |
| `lambda_returns_callable` | function returns a callable; caller invokes it |
| `lambda_passed_to_higher_order` | `def foreach(f, xs): for x in xs: f(x)` — generic pattern |

## Files changed

- `crates/pyx86/src/hir.rs` — `CallableId`, `Type::Callable`,
  `Expr::LambdaValue` + `IndirectCall`.
- `crates/pyx86/src/check.rs` — `Callable[...]` annotation parse;
  lambda detection driven by expected type; free-var analysis;
  env-struct synthesis; lifted-function emission.
- `crates/pyx86/src/codegen.rs` — Callable LLVM type; LambdaValue +
  IndirectCall lowering.
- `tests/correctness/lambda_*` — six new programs.
- This file.
