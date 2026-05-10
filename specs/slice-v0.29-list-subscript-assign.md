# Spec: slice v0.29 — mutable list subscript (`lst[i] = v`)

> Status: in progress.

## What v0.29 adds

The other half of v0.28's container-mutation story: subscript-assignment on
`list[T]`. Promised in the v0.28 PR's "deferred" list with an explicit
`unsupported_feature` error pointing here.

```python
def main(n: int) -> int:
    xs: list[int] = [0, 0, 0]
    xs[0] = n
    xs[1] = n * 2
    xs[2] = xs[0] + xs[1]
    return xs[2]
```

## What v0.29 does **not** add

- **Bounds checks** — `lst[i] = v` with `i >= len(lst)` is UB, same as the
  existing read path (`lst[i]`). Bounds checks (with a panic-on-fail
  runtime call) are deferred. This is consistent with the v0.27 / v0.28
  policy: ship the unchecked happy path, fix safety in a focused later slice.
- **Negative indices** — Python allows `xs[-1]`; we don't yet for read
  or write. Deferred uniformly.
- **Slicing on LHS** (`xs[i:j] = ...`) — deferred to a later slicing slice.
- **Augmented subscript** (`xs[i] += 1`) — deferred.

## HIR additions

No new variants. `Stmt::SetSubscript` (added in v0.28) already carries
the container/key/value. Codegen dispatches on `container.ty`.

## Check (lower)

`lower_stmts`'s `Subscript` arm in `Assign` (added in v0.28) currently
rejects `Type::List(_)`. Change the arm so:

- `Type::List(id)` → lower index (coerce to `I64`), lower value (coerce
  to `id.elem()`), emit `Stmt::SetSubscript`.
- `Type::Dict(id)` — unchanged path from v0.28.
- Other types — unchanged rejection.

The list error message is removed (no longer "deferred to v0.29").

## Codegen

`lower_set_subscript` becomes a dispatch on `container.ty`:

- `Type::Dict(_)` — existing v0.28 path (`pyx86_dict_i64_insert`).
- `Type::List(id)` — load `data` from the list struct (field 2), bitcast
  to `T*`, GEP by index, `store` the new value. Symmetric to
  `lower_list_index`, but with `store` instead of `load`.

No runtime helper needed — the lowering is inline LLVM IR.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `list_assign_basic` | construct `[0,0,0]`, write all three slots, return sum-of-positional-products |
| `list_assign_loop` | `xs = [0]*n` (built via append in a loop), then `xs[i] = i*i` in a second loop |
| `list_assign_aliased` | two names bound to the same list; write through one, read through the other — verifies ref-semantics |
| `list_assign_overwrite` | repeatedly write to `xs[0]`; last write wins |

## Files changed

- `crates/pyx86/src/check.rs` — extend `Subscript` arm to accept `Type::List`.
- `crates/pyx86/src/codegen.rs` — dispatch in `lower_set_subscript`; add
  inline list-store lowering.
- `tests/correctness/list_assign_*/` — four new programs.
- This file.
