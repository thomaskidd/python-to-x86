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
    <stmt>*                          # any mix of:
                                     #   <name> [: int] = <expr>
                                     #   if <cond>: <body> [elif …]* [else: <body>]
                                     #   while <cond>: <body>
                                     #   break          (only inside a loop)
                                     #   continue       (only inside a loop)
                                     #   pass
                                     #   return <expr>
```

with up to 16 typed `int` parameters and a body that **provably ends with a return on every path** (conservative: the last statement is either `Return` or an `If` whose two branches both recursively end with `Return` — `While` is not a covering construct because it may execute zero iterations).

Expressions: integer literals, bool literals (lowered to `0`/`1`), variable references, `+ - * // %`, unary `+ -`, `not`, comparisons `< <= > >= == !=` (chained allowed: `a < b < c` works as in CPython).

Conditions in `if`/`while` accept any int expression (truthy via implicit `!= 0`) or a comparison/`not` directly.

Anything else — different function name, untyped/non-int parameters, default args, decorators, multiple top-level definitions, augmented assignment (`x += 1`), tuple unpacking, chained assignment (`a = b = 1`), `for` loops, `else` on `while`, `and`/`or`, `is`/`in`, exceptions, division by `/`, exponentiation, bitwise, calls, comprehensions, etc. — produces `unsupported_feature: <reason>`. Use of an unbound name produces a "not in scope" error. `break` / `continue` outside a loop is rejected.

## Why not infer types here

For v0.2 every expression is i64 by construction (the only literals we accept are `int`, the only operators preserve int, the return annotation is `int`). Type inference is meaningful starting in v0.3 (or whenever we add another type), at which point this module gains a unification pass and is renamed.

## Test surface

Unit tests in `check.rs`:
- `lowers_int_literal`, `lowers_unary_minus`, `lowers_binary_arith`, `lowers_floordiv_and_mod` — happy paths for each accepted construct.
- `rejects_true_division`, `rejects_pow`, `rejects_bitwise`, `rejects_variable_reference`, `rejects_missing_return_annotation`, `rejects_non_main_function` — rejection cases that pin the subset boundary.

Each new feature added in a future slice should add **both** an accept test and a clear-rejection test for its boundaries.
