# Spec: slice v0.43 — `sum()` + generator-expression fusion

> Status: in progress.

## What v0.43 adds

The `sum()` builtin, and generator expressions **fused** into it. No
general generator support: a generator expression is only legal as the
sole argument of a consuming `sum(...)` — a free-standing generator
expression remains a compile error.

```python
def main(n: int) -> int:
    a: int = sum(i * i for i in range(n))          # fused genexp
    b: int = sum(i for i in range(n) if i % 3 == 0) # with filter
    xs: list[int] = [1, 2, 3, n]
    c: int = sum(xs)                                # plain list
    d: int = sum(range(n))                          # plain range
    return a + b + c + d
```

- The single argument may be a **generator expression**, a `list[T]`, or
  a `range(...)`.
- No start-value argument (`sum(xs, 10)` is rejected).
- Integer elements (any width / bool) sum to `i64`; `f64` elements sum
  to `f64`. Non-numeric element types are rejected by `coerce`.
- Multiple `if` clauses in the genexp are AND-combined.

## Desugaring

`sum(...)` lowers to a scalar-accumulator `DoBlock`, reusing the shared
comprehension loop scaffold:

```
acc = 0                     # ConstI64 / ConstF64
for <target> in <iter>:     # range or list scaffold (build_comp_loop)
    if <cond>:              # optional filter (genexp only)
        acc = acc + <elt>
acc                         # DoBlock result
```

For a plain iterable `sum(xs)`, `<elt>` is the loop variable itself; for
a generator expression, `<elt>` is the genexp's element expression.

## Refactor

- Extracted `lower_iterable(iter, scope, sigs) -> (Type, CompIter)` from
  `lower_comp_generator`; both comprehensions and `sum()` use it to lower
  a `range(...)`-or-`list[T]` source.
- Added `build_sum_reduction(...)` building the scalar-accumulator
  `DoBlock`.

## Codegen

No changes. Reuses `BinOp::Add`, `DoBlock`, `While`, range/list loop
primitives, and `Coerce`.

## What v0.43 does **not** add

- **General generators / `yield`** — out of scope (project charter).
- **Free-standing generator expressions** — only `sum(genexp)` is legal.
- **`sum(iterable, start)`** — the start-value form is rejected.
- **Genexp fusion into other consumers** (`any`/`all`/`min`/`max`/`for`)
  — deferred; only `sum` consumes a genexp in v0.43.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `sum_genexp_range` | `sum(i*i for i in range(n))` |
| `sum_genexp_filter` | `sum(i for i in range(n) if i % 3 == 0)` |
| `sum_over_list` | `sum(xs)` for a `list[int]` |
| `sum_over_range` | `sum(range(n))` |
| `sum_genexp_float` | float accumulator path, `int(...)`-cast result |

## Files changed

- `crates/pyx86/src/check.rs` — `lower_iterable`, `build_sum_reduction`,
  the `sum` builtin arm; `lower_comp_generator` rewritten on
  `lower_iterable`.
- `tests/correctness/sum_*`.
- This file.
