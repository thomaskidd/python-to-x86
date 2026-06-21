# Spec: slice v0.42 — set comprehensions

> Status: in progress.

## What v0.42 adds

Set comprehensions:

```python
{<elt> for <target> in <iter> (if <cond>)*}
```

Example:

```python
def main(n: int) -> int:
    remainders: set[int] = {i % 5 for i in range(n)}
    return len(remainders)
```

Duplicate elements collapse, matching Python set semantics. The iterable
may be `range(...)` or a `list[T]`, exactly as for list/dict
comprehensions. Multiple `if` clauses are AND-combined. Elements are
`i64` only, matching set literals (v0.32).

## What v0.42 does **not** add

- **Nested generators** — rejected.
- **Non-`i64` elements** — same limitation as set literals.
- **Targets other than a simple name** — rejected.

## Desugaring

Reuses the shared comprehension helpers from v0.41 (`lower_comp_generator`,
`lower_comp_filter`, `build_comp_loop`, `wrap_filter`, `CompIter`). The
only set-specific parts are the accumulator and the per-element body:

```
acc = set()                # empty SetLit
for <target> in <iter>:    # range or list scaffold (shared)
    if <cond>:             # optional filter
        acc.add(<elt>)     # Stmt::SetAdd
acc                        # DoBlock result
```

## Codegen

No changes. Reuses `SetLit`, `SetAdd`, `DoBlock`, `While`, and the
range/list loop primitives.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `set_compr_range` | `{i % 5 for i in range(n)}`; dedup via `len` |
| `set_compr_over_list` | `{x for x in some_list}` |
| `set_compr_with_filter` | `{i for i in range(n) if i % 2 == 0}` |
| `set_compr_membership` | membership tests against the built set |

## Files changed

- `crates/pyx86/src/check.rs` — new `SetComp` arm.
- `tests/correctness/set_compr_*`.
- This file.
