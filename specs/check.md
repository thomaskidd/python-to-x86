# Spec: check / lower

## Responsibility

Take the parsed `rustpython-ast::ModModule` and lower it to our `hir::Program`. As part of that lowering, reject anything outside the currently supported subset with an `unsupported_feature` error pointing at the rejected construct.

This module is the bridge between "raw Python AST" and "what the compiler actually understands." It is the one place that has to grow each time we widen the language subset, and the one place that polices the "reject rather than fall back" rule from CLAUDE.md.

In later slices this module evolves into a full type-inference pass and is renamed to `infer.rs`. For now it does no inference because every value is i64.

## Inputs / Outputs

- **Input**: `&parser::Module` (alias for `rustpython_ast::ModModule`)
- **Output**: `Result<hir::Program>` — `Err` on any unsupported construct.

## Current accepted shape

```text
def main(<param>: int, …) -> int:
    return <expr>
```

with up to 16 typed `int` parameters, and `<expr>` built from:
- integer literals (must fit i64)
- parameter references (`Name` nodes that resolve to a declared param)
- binary operators `+ - * // %`
- unary operators `+x`, `-x`
- parentheses (transparent in the AST)

Anything else — different function name, untyped/non-int parameters, default args, decorators, missing/wrong return annotation, local variable assignment, multiple top-level statements, unsupported expression forms, division by `/`, exponentiation, bitwise, calls, comprehensions, etc. — produces `unsupported_feature: <reason>`. References to names that aren't parameters (locals) are explicitly rejected with a "locals not supported until v0.4" hint.

## Why not infer types here

For v0.2 every expression is i64 by construction (the only literals we accept are `int`, the only operators preserve int, the return annotation is `int`). Type inference is meaningful starting in v0.3 (or whenever we add another type), at which point this module gains a unification pass and is renamed.

## Test surface

Unit tests in `check.rs`:
- `lowers_int_literal`, `lowers_unary_minus`, `lowers_binary_arith`, `lowers_floordiv_and_mod` — happy paths for each accepted construct.
- `rejects_true_division`, `rejects_pow`, `rejects_bitwise`, `rejects_variable_reference`, `rejects_missing_return_annotation`, `rejects_non_main_function` — rejection cases that pin the subset boundary.

Each new feature added in a future slice should add **both** an accept test and a clear-rejection test for its boundaries.
