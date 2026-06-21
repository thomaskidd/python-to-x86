# Spec: slice v0.41 — dict comprehensions

> Status: in progress.

## What v0.41 adds

Dict comprehensions:

```python
{k: v for <target> in <iter> (if <cond>)*}
```

Examples:

```python
def main(n: int) -> int:
    squares: dict[int, int] = {i: i * i for i in range(n)}
    total: int = 0
    for i in range(n):
        total = total + squares[i]
    return total
```

The iterable may be `range(...)` or a `list[T]`, exactly matching the
list-comprehension support added in v0.21. Multiple `if` clauses are
AND-combined. Like dict literals (v0.26), keys and values are `i64`
(the only dict shape the runtime supports today); a non-`i64` key or
value is coerced or rejected by the existing `coerce` path.

## What v0.41 does **not** add

- **Nested generators** (`{... for a in x for b in y}`) — rejected,
  same as list comprehensions.
- **Non-`i64` dict keys/values** — same limitation as dict literals;
  the dict runtime is `pyx86_dict_i64_insert` only.
- **Set comprehensions** — that is v0.42.
- **Comprehension targets other than a simple name** — rejected.

## Desugaring

A dict comprehension lowers to the same `Expr::DoBlock` shape the list
comprehension uses, differing only in the accumulator and the per-element
body:

```
acc = {}                       # empty DictLit
for <target> in <iter>:        # range or list scaffold (shared)
    if <cond>:                 # optional filter
        acc[<key>] = <value>   # Stmt::SetSubscript
acc                            # DoBlock result
```

## Refactor

The list-comprehension arm (v0.21) contained inline loop-scaffold,
filter-combination, and generator-validation code. v0.41 extracts three
shared helpers in `check.rs`, used by both list- and dict-comprehensions
(and by set-comprehensions in v0.42):

- `enum CompIter { Range { start, stop, step }, List { iter } }`
- `lower_comp_generator(gen, scope, sigs) -> (target_name, target_ty, CompIter, inner_scope)`
  — validates the single, non-async, simple-name generator; determines
  the target type from the iterable; returns an inner scope with the
  target bound.
- `lower_comp_filter(ifs, scope, sigs) -> Option<TypedExpr>` — lowers
  and AND-combines the `if` clauses.
- `build_comp_loop(target_name, target_ty, iter, body_stmt, uniq) -> Vec<Stmt>`
  — emits the `while`-loop desugar (range counter or list index walk)
  around a per-element `body_stmt`.
- `wrap_filter(filter, body) -> Stmt` — wraps `body` in an `if` when a
  filter is present.

The list-comprehension behaviour is unchanged; it is rewritten in terms
of these helpers.

## Codegen

No changes. Dict comprehensions reuse `DictLit`, `SetSubscript`,
`DoBlock`, `While`, and the range/list loop primitives already emitted.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `dict_compr_range` | `{i: i*i for i in range(n)}` |
| `dict_compr_over_list` | `{x: x+1 for x in some_list}` |
| `dict_compr_with_filter` | `{i: i for i in range(n) if i % 2 == 0}` |
| `dict_compr_empty` | `range(0)` yields an empty dict |

## Files changed

- `crates/pyx86/src/check.rs` — three shared comp helpers; list-comp
  rewritten on top; new `DictComp` arm.
- `tests/correctness/dict_compr_*`.
- This file.
