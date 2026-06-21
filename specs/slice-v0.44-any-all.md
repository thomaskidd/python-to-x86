# Spec: slice v0.44 — `any()` / `all()` reductions

> Status: in progress.

## What v0.44 adds

The `any()` and `all()` builtins, with generator expressions fused into
them (same fusion rule as `sum()` in v0.43) and short-circuit evaluation.

```python
def main(n: int) -> int:
    a: bool = any(i * i > 100 for i in range(n))   # fused genexp
    b: bool = all(i < n for i in range(n))         # True for all n >= 0
    xs: list[int] = [0, 0, n]
    c: bool = any(xs)                              # truthiness of elements
    return int(a) * 100 + int(b) * 10 + int(c)
```

- The single argument may be a **generator expression**, a `list[T]`, or
  a `range(...)`.
- For a generator expression, the predicate is the genexp's element
  (coerced to `bool`); `if` clauses filter which elements are tested.
- For a plain iterable, each element's **truthiness** is tested
  (`any([0, 0, 3])` is `True`), matching Python.
- `any([])` is `False`; `all([])` is `True`.

## Desugaring

Both lower to a short-circuiting `Bool`-accumulator `DoBlock` over the
shared comprehension loop scaffold (`build_comp_loop`), using `break` to
stop at the first decisive element:

```
# any:                          # all:
acc = False                     acc = True
for t in iter:                  for t in iter:
    if <filter>:                    if <filter>:
        if <pred>:                     if <pred>: pass
            acc = True; break          else: acc = False; break
acc                             acc
```

`break` inside the desugared `while` targets the loop exit via the
existing `loop_targets` machinery; the post-body increment is reached
only on the non-breaking path, giving correct loop semantics.

## Codegen

No changes. Reuses `If`, `Break`, `ConstBool`, `DoBlock`, `While`, the
range/list loop primitives, and numeric→`bool` `Coerce`.

## What v0.44 does **not** add

- **`min`/`max` over an iterable** — deferred: `min([])`/`max([])` raise
  `ValueError` (exceptions are phase 2), which would diverge from CPython
  on empty inputs.
- **Genexp fusion into a `for` loop** — deferred.
- **Free-standing generator expressions** — still a compile error.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `any_genexp` | `any(i*i > 100 for i in range(n))` |
| `all_genexp` | `all(i < n for i in range(n))` |
| `any_all_over_list` | truthiness of list elements |
| `any_genexp_filter` | filtered genexp + early exit |
| `all_genexp_empty` | `all(... for i in range(0))` is `True` |

## Files changed

- `crates/pyx86/src/check.rs` — `build_any_all_reduction`; the
  `any`/`all` builtin arm.
- `tests/correctness/any_*`, `all_*`.
- This file.
