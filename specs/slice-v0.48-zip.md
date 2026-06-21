# Spec: slice v0.48 — `zip()` with tuple-target unpacking

> Status: in progress.

## What v0.48 adds

`for a, b in zip(xs, ys):` — iterate two lists pairwise, stopping at the
shorter. Plus a related correctness fix for multi-`if` comprehension
filters (same i64→i1 bug class).

```python
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b, 7, 9]
    ys: list[int] = [1, 2, 3]
    total: int = 0
    for x, y in zip(xs, ys):       # stops after 3 pairs
        if x == 0:
            continue
        total = total + x * y
    return total
```

- Exactly **two** operands in v0.48, each a `list[T]`.
- The target must be two simple names `(a, b)`.
- Stops at the shorter list; `break` / `continue` behave correctly.

## Desugaring

`zip` over two lists is a single index walk bounded by the shorter list:

```
zl0 = xs; zl1 = ys; i = 0
while i < len(zl0) and i < len(zl1):
    a = zl0[i]; b = zl1[i]
    <body>
    # latch: i = i + 1
```

The bound `i < len(zl0) and i < len(zl1)` is exactly `i < min(len0, len1)`.
Both comparisons are `Bool`, so the `and` is a clean `i1`. The index bump
lives in the `while` latch so `continue` advances it.

## Related fix: multi-`if` comprehension filters

`lower_comp_filter` combined multiple `if` clauses with
`BoolOp::And` over operands **coerced to i64** (short-circuit value
semantics), then tagged the result `Bool`. That emitted an `i64` value
into an `i1` slot — a hard codegen error — so any comprehension/genexp
with two or more `if` clauses (e.g. `[i for i in range(n) if i%2==0 if
i%3==0]`) failed to compile. There was no multi-`if` test, so it was
latent. Fixed by `and`-ing the `Bool` operands directly (no i64
widening), matching how `zip`'s bound is built.

## What v0.48 does **not** add

- **`zip` of more than two iterables** — rejected (deferred).
- **`zip` with a `range` operand** — rejected with a clear message;
  range needs length/index arithmetic, deferred.
- **`zip` producing a materialised list** (`list(zip(...))`) — out of
  scope; only the fused for-loop form is supported.

## Codegen

No changes. Reuses `ListLen`, `ListIndex`, `BoolOp`, and the `while`
latch.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `zip_equal_len` | two equal-length lists |
| `zip_unequal_len` | stops at the shorter list |
| `zip_continue` | `continue` in the body keeps pairing correct |
| `compr_two_filters` | regression: comprehension with two `if` clauses |

## Files changed

- `crates/pyx86/src/check.rs` — `zip` branch in the `For` handler;
  `lower_comp_filter` multi-`if` fix.
- `tests/correctness/zip_*`, `tests/correctness/compr_two_filters`.
- This file.
