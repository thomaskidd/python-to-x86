# Spec: slice v0.46 — generator-expression fusion into `for` loops

> Status: in progress.

## What v0.46 adds

A `for` loop may iterate over a **generator expression**, which is fused
into the consuming loop (no generator object is materialised). This is
the second consumer of genexps, after `sum()` in v0.43.

```python
def main(n: int) -> int:
    total: int = 0
    for sq in (i * i for i in range(n) if i % 2 == 0):
        if sq > 50:
            continue
        total = total + sq
    return total
```

- The genexp's iterable may be `range(...)` or a `list[T]`.
- The genexp's `if` clauses filter which elements reach the loop body.
- `break` / `continue` in the loop body act on the single fused loop
  (correct because the loop advance lives in the `while` latch — v0.45).

## Desugaring

```
for x in (<elt> for y in <it> if <c>):   →   for y in <it>:
    <body>                                       if <c>:
                                                     x = <elt>
                                                     <body>
```

reusing `lower_comp_generator` + `build_comp_loop`. `build_comp_loop`'s
body parameter was generalised from a single `Stmt` to a `Vec<Stmt>` so
the fused per-iteration sequence (`x = <elt>` followed by the user body)
can be passed directly.

## What v0.46 does **not** add

- **Free-standing generator expressions** — still a compile error; only
  `sum(genexp)` (v0.43) and `for … in genexp` (this slice) consume them.
- **Tuple-target fusion** (`for a, b in (… for …)`) — for-loop target
  must be a simple name; tuple unpacking arrives with `enumerate`/`zip`
  (v0.47–v0.48).
- **Nested generators** in the fused genexp — rejected.

## Codegen

No changes. Reuses the comprehension loop scaffold (now with the v0.45
latch).

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `for_genexp_basic` | `for v in (i*i for i in range(n))` |
| `for_genexp_filter` | genexp `if` clause filters elements |
| `for_genexp_over_list` | genexp source is a `list[T]` |
| `for_genexp_continue` | `continue` in the fused body |
| `for_genexp_break` | `break` in the fused body |

## Files changed

- `crates/pyx86/src/check.rs` — genexp branch in the `For` handler;
  `build_comp_loop` body generalised to `Vec<Stmt>` (callers updated).
- `tests/correctness/for_genexp_*`.
- This file.
