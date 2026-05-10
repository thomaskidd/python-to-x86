# Spec: slice v0.32 — `set[T]` core

> Status: in progress.

## What v0.32 adds

The last missing core container type from CLAUDE.md's v1 in-scope list:
`set[T]`. Operations:

- **Literal** `{1, 2, 3}` (no colons distinguishes set from dict).
- **Empty set** `set()` (Python reserves `{}` for the empty dict).
- **`len(s)`**.
- **`x in s`** / **`x not in s`**.
- **`s.add(x)`** (statement; mutation).

Element type is `i64` only in v0.32 (mirrors v0.26 dict). Promotion to
other key types is mechanical (hash function + slot stride) and will be
its own slice when needed.

## What v0.32 does **not** add

- **`.remove(x)`, `.discard(x)`** — needs tombstones for open-addressed
  deletion. Deferred.
- **Set operations**: `s1 | s2`, `s1 & s2`, `s1 - s2`, `s1 ^ s2`, etc.
  Deferred — they need a fresh allocation + iteration over the smaller
  operand.
- **Set comprehensions** `{x for x in ...}`. Deferred to a comprehension
  slice.
- **Iteration** `for x in s`. Deferred — needs the same slot-walking
  infrastructure that dict-iteration would.
- **Non-i64 element types** (`set[str]`, `set[tuple[...]]`). Deferred —
  needs a per-key-type hash function and runtime stride.

## Implementation idea

`set[i64]` shares **exactly** the dict runtime. A set is layout-identical
to `dict[i64, i64]`: the same `{ i64 size, i64 cap, i8* slots }` heap
struct, with the per-slot `value` field stored as 0 and ignored on read.

Codegen calls the same `pyx86_dict_i64_insert` / `pyx86_dict_i64_has` /
size-field-load helpers, never duplicating the runtime.

The reason we keep `Type::Set` distinct from `Type::Dict` is purely so
the user gets correct type errors (`set[i64]` ≠ `dict[i64, i64]`).

## HIR additions

```rust
pub struct SetId(u32);            // interns the element type
Type::Set(SetId)                  // LLVM type identical to Dict: { i64, i64, i8* }*

Expr::SetLit { elements: Vec<TypedExpr> }      // ty == Set
Expr::SetHas { set: Box<TypedExpr>, key: Box<TypedExpr> }    // ty == Bool
Expr::SetLen { set: Box<TypedExpr> }           // ty == I64

Stmt::SetAdd { set: TypedExpr, value: TypedExpr }   // method call as a statement
```

## Check (lower)

- **`parse_type_annotation`** — recognize `set[i64]` as `Type::Set(SetId::intern(I64))`.
- **`lower_expr` `ast::Expr::Set`** — list of element expressions, each
  coerced to I64; emit `SetLit`. Empty set literal `{}` is **not** valid
  in Python source (that's a dict) — only `set()` is.
- **`lower_expr` Call to `set`** — `set()` with zero args → empty `SetLit`.
  Any other arg shape rejected.
- **`Compare` `in`/`not in`** — add a `Type::Set` arm; coerce key to
  element type; emit `SetHas`.
- **`len(s)`** — add a `Type::Set` arm; emit `SetLen`.
- **`s.add(x)` expression-statement** — extend the existing dispatcher
  that already handles `list.append(...)`; require single positional arg,
  emit `Stmt::SetAdd`.
- **`coerce`** — add empty-set re-tag (`SetLit { elements: [] }` of any
  Set type can re-tag to any other Set type), matching the empty-list /
  empty-dict trick.

## Codegen

`Type::Set(_)` maps to `{ i64, i64, i8* }*` (same as Dict).

Lowering:

- **SetLit** — same shape as `lower_dict_lit`: allocate outer struct,
  malloc + memset slots, then for each element call
  `pyx86_dict_i64_insert(table_raw, elt, 0)`. The 0 value is ignored on
  reads (set has no values). Same `cap = next_pow2(max(2 * N, 4))`.
- **SetHas** — bitcast + `pyx86_dict_i64_has`.
- **SetLen** — load size field (same layout as dict).
- **SetAdd** — bitcast + `pyx86_dict_i64_insert(table_raw, key, 0)`.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `set_literal_in` | `{1, 2, 3}` then membership test |
| `set_literal_len` | `len({1, 2, 3})` |
| `set_add_grow` | start from `set()`, add `n` distinct keys, check len |
| `set_empty_constructor` | `set()` literal; `len == 0`; `x in s == False` |
| `set_overwrite` | adding an existing key doesn't change len |
| `set_in_fstring` | `f"size={len(s)}"` — combine with v0.30 |

## Files changed

- `crates/pyx86/src/hir.rs` — `SetId`, `Type::Set`, three Expr variants,
  one Stmt variant.
- `crates/pyx86/src/check.rs` — annotation parsing; literal lowering;
  `set()` call; `in`/`not in`; `len`; `.add(x)`; empty-set re-tag.
- `crates/pyx86/src/codegen.rs` — Set type byte size & LLVM type; new
  lower methods (`lower_set_lit`, `lower_set_has`, `lower_set_len`);
  walker arms.
- `tests/correctness/set_*` — six new programs.
- This file.
