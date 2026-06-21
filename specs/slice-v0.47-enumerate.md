# Spec: slice v0.47 — `enumerate()` with tuple-target unpacking

> Status: in progress.

## What v0.47 adds

`for i, x in enumerate(it[, start]):` — iterate an iterable while tracking
a running index. First use of a **tuple for-loop target**.

```python
def main(n: int) -> int:
    xs: list[int] = [10, 20, 30, 40]
    total: int = 0
    for i, x in enumerate(xs):
        total = total + i * x
    for j, v in enumerate(range(n), 5):   # optional start
        if v % 2 == 0:
            continue
        total = total + j
    return total
```

- Iterable may be `range(...)` or `list[T]`.
- Optional `start` (default `0`), coerced to `i64`.
- The target must be exactly two simple names `(index, value)`.
- `break` / `continue` behave correctly (index lives in the latch).

## Desugaring

```
for i, x in enumerate(it, start):   →   i = start
    <body>                                  for x in it:    # build_comp_loop
                                                <body>
                                            # latch: <advance>; i = i + 1
```

The index variable is initialised **before** the loop and bumped in the
`while` **latch** (via `build_comp_loop`'s new `extra_latch` parameter),
so it advances by exactly 1 per iteration regardless of the range step and
survives `continue`.

## Refactor

- `build_comp_loop` gains an `extra_latch: Vec<Stmt>` parameter, appended
  to the loop latch after the built-in advance. All existing callers pass
  `Vec::new()`.
- New `parse_pair_target(target, who)` helper validates a two-name tuple
  target (shared with `zip` in v0.48).

## What v0.47 does **not** add

- **General tuple unpacking** `for a, b in list_of_tuples` — only the
  `enumerate`/`zip` (v0.48) producers introduce pair targets; iterating a
  `list[tuple[...]]` with unpacking is separate.
- **Nested unpacking** `for i, (a, b) in …` — rejected.
- **`enumerate` of a genexp** — the argument must be `range`/`list`.

## Codegen

No changes. Reuses the comprehension loop scaffold + latch.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `enumerate_list` | `for i, x in enumerate(xs)` |
| `enumerate_range` | `for i, x in enumerate(range(n))` |
| `enumerate_start` | non-zero `start` argument |
| `enumerate_continue` | `continue` keeps the index correct |

## Files changed

- `crates/pyx86/src/check.rs` — `parse_pair_target`; `enumerate` branch
  in the `For` handler; `build_comp_loop` gains `extra_latch` (callers
  updated).
- `tests/correctness/enumerate_*`.
- This file.
