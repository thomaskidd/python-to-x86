# Spec: slice v0.39 — list / tuple slicing

> Status: in progress.

## What v0.39 adds

Slicing on lists and tuples, mirroring v0.31's string slicing.

- **`lst[i:j]`** — substring-style slice of a `list[T]`. Bounds
  clamped at runtime. Returns a fresh heap-allocated `list[T]`.
- **`lst[:j]`**, **`lst[i:]`**, **`lst[:]`** — open bounds; defaults
  are `0` and `len(lst)`.
- **`tup[i:j]`** on a tuple **requires compile-time integer literal
  bounds**. Tuples are fixed-arity heterogeneous in our model, so the
  result type is only inferable when both bounds are known statically.
  Returns a new tuple value containing the selected elements.

## What v0.39 does **not** add

- **Negative indices / bounds** — rejected (literal form). Consistent
  with v0.31 strings.
- **Step** (`lst[::2]`) — rejected.
- **Runtime bounds on tuple slicing** — rejected (would need a
  variable-arity tuple type, which we don't have).
- **`bytes` slicing** — `bytes` is not yet a v1 type.
- **Slice-LHS assignment** (`lst[i:j] = ...`) — deferred.

## HIR additions

```rust
Expr::ListSlice {
    list: Box<TypedExpr>,
    start: Box<TypedExpr>,
    stop: Box<TypedExpr>,
}
```

Tuple slicing rewrites at lower-time into a `TupleLit` of the
selected elements; no new variant.

## Check (lower)

In `lower_expr`'s `Subscript` arm, when the slice is `ast::Expr::Slice`:

- **List receiver**: reject step + literal negatives; lower `lower`
  (default `ConstI64(0)`) and `upper` (default `ListLen(<clone>)`)
  coerced to I64; emit `Expr::ListSlice`.
- **Tuple receiver**: both bounds must be compile-time integer
  literals (or omitted). Compute `start` and `stop` as `i64`,
  clamp to `[0, arity]`, build a `TupleLit` from
  `tuple[start..stop]` element types via `TupleIndex` accesses.
  Reject runtime-valued bounds.
- **String receiver**: unchanged from v0.31.

## Codegen

`ListSlice` is element-size-aware: data is at offset `start *
elem_bytes`, copied to a fresh malloc'd buffer of `out_len *
elem_bytes`. Same clamp logic as string slice. The resulting list
struct: `{ out_len, out_len, new_data }*` (matching the existing
list layout's `(len, cap, data)`).

Tuple slicing has no codegen — it lowers to `TupleLit`, which already
has a lowering.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `list_slice_basic` | `xs[i:j]` of an int list |
| `list_slice_open_left` | `xs[:j]` |
| `list_slice_open_right` | `xs[i:]` |
| `list_slice_full_copy` | `xs[:]` — aliasing check via mutation |
| `list_slice_clamped` | bounds past `len(xs)` |
| `tuple_slice_literal` | `t[0:2]` of a fixed-arity tuple |
| `tuple_slice_omitted_bounds` | `t[:]` of a tuple |

## Files changed

- `crates/pyx86/src/hir.rs` — `Expr::ListSlice`.
- `crates/pyx86/src/check.rs` — Subscript arm: list/tuple slice paths.
- `crates/pyx86/src/codegen.rs` — `lower_list_slice` (and walker arm).
- `tests/correctness/list_slice_*` + `tuple_slice_*`.
- This file.
