# Spec: slice v0.28 — mutable dict (`d[k] = v`)

> Status: in progress.

## What v0.28 adds

Completes the dict feature begun in v0.26 (which was read-only). Adds:

- **Subscript-assignment on `dict[K, V]`**: `d[k] = v` as a statement.
- **Empty dict literal `{}`**: lowered via the same re-tag trick used for
  empty lists since v0.20. Required for `d: dict[K, V] = {}; d[k] = v` to
  be expressible at all.
- **Growth**: `pyx86_dict_i64_insert` rehashes to 2×cap when load exceeds 75%.
  This makes building a dict from an empty literal (`d: dict[int,int] = {}`)
  safe to do for arbitrary numbers of inserts.

```python
def main(n: int) -> int:
    d: dict[int, int] = {}
    i: int = 0
    while i < n:
        d[i] = i * i
        i = i + 1
    return d[n // 2]
```

## What v0.28 does **not** add

- **Mutable list subscript** (`lst[i] = v`) — deferred to v0.29. Rejected at
  check-time with a clear `unsupported_feature` error.
- **`del d[k]`** — Python supports it; we don't yet. Open-addressed deletion
  needs tombstones, which complicates the load-factor accounting. Deferred.
- **Augmented subscript** (`d[k] += 1`) — desugars to `d[k] = d[k] + 1` in
  Python; we don't support this rewrite yet. Deferred.
- **String/tuple/class keys** — keys are still `i64` only (matching v0.26).
- **`.get(k, default)`, `.keys()`, `.values()`, `.items()`, iteration** — none
  of these are added in this slice.

## HIR additions

One new `Stmt` variant:

```rust
pub enum Stmt {
    // ...
    /// `container[key] = value`. In v0.28, `container.ty` must be `Type::Dict`.
    /// Codegen lowers to `pyx86_dict_i64_insert(table_raw, key, value)`.
    SetSubscript { container: TypedExpr, key: TypedExpr, value: TypedExpr },
}
```

The existing `Stmt::SetField` (from v0.27) is the analogue for attribute assignment.

## Check (lower)

The `ast::Stmt::Assign` handler in `lower_stmts` already dispatches on the
target's kind (`Name`, `Attribute`, …). Add a `Subscript` arm:

1. Lower the container expression.
2. Match on `container.ty`:
   - `Type::Dict(id)` → lower key (coerced to `id.key()`), lower value
     (coerced to `id.val()`), emit `Stmt::SetSubscript`.
   - `Type::List(_)` → `bail!("unsupported_feature: list subscript-assignment `lst[i] = v` is not supported in v0.28 (deferred to v0.29)")`.
   - any other type → `bail!("unsupported_feature: subscript-assignment on {ty} is not supported")`.
3. The check pass already rejects chained `a = b = ...` and unpacking
   targets. No new rejections needed for those.

The dead `parse_assign_target` helper (currently never called) is updated or
removed — its stale error message wrongly mentions "subscript / attribute
assignment is not supported", which is no longer accurate.

## Codegen

### Runtime growth

`pyx86_dict_i64_insert` gains a load-factor check at entry. New shape:

```
entry:
  ; check 4*size >= 3*cap → grow first
  if 4*size >= 3*cap: call @pyx86_dict_i64_grow(table_raw)
  ; ... existing probe/insert/overwrite logic ...
```

New helper `pyx86_dict_i64_grow(table_raw)`:

1. Read `old_slots`, `old_cap` from the outer struct.
2. Compute `new_cap = old_cap * 2`.
3. `new_slots = malloc(new_cap * 24); memset(new_slots, 0, ...)`.
4. Store the new pointer + cap into the outer struct; zero `size`.
5. For `i in 0..old_cap`: if `old_slots[i].occupied`, recursively call
   `pyx86_dict_i64_insert(table_raw, old_slots[i].key, old_slots[i].value)`.
   The recursion terminates because after doubling, load factor is well
   below the growth threshold.

Old slot memory is leaked (consistent with v0.27 — refcounting is on the v1
roadmap but not yet wired up).

### SetSubscript lowering

For `Stmt::SetSubscript { container, key, value }` (container is `Type::Dict`):

```llvm
; container_op is %{i64,i64,i8*}* (already)
%raw = bitcast {i64,i64,i8*}* %container_op to i8*
call void @pyx86_dict_i64_insert(i8* %raw, i64 %key, i64 %value)
```

Identical to the per-entry insert that `lower_dict_lit` already emits, just
driven from a statement instead of the literal initializer.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `dict_assign_basic` | `d = {1: 0, 2: 0}; d[1] = a; d[2] = b; return d[1] * 10 + d[2]` |
| `dict_overwrite` | repeatedly assign different values to the same key; verify last write wins |
| `dict_grow_past_cap` | start from `{}`, insert `n` distinct keys in a loop, read one back — verifies growth |
| `dict_build_empty` | `d: dict[int, int] = {}; d[k] = v; return d[k]` — empty literal + single write |

Existing v0.26 dict tests (`dict_basic`, `dict_lookup`, `dict_in_op`) must
continue to pass — growth must not break read-only behaviour.

## Files changed

- `crates/pyx86/src/hir.rs` — add `Stmt::SetSubscript`.
- `crates/pyx86/src/check.rs` — extend `Assign` lowering for `Subscript`
  targets; reject lists; remove or fix the dead `parse_assign_target`.
- `crates/pyx86/src/codegen.rs` — add growth to `pyx86_dict_i64_insert`;
  emit `pyx86_dict_i64_grow`; lower `Stmt::SetSubscript`.
- `tests/correctness/dict_assign_basic/`, `dict_overwrite/`,
  `dict_grow_past_cap/`, `dict_build_empty/` — new programs.
- This file (`specs/slice-v0.28-dict-mut.md`).
