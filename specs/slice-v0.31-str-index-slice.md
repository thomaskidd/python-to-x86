# Spec: slice v0.31 — string indexing + slicing

> Status: in progress.

## What v0.31 adds

- **`s[i]`** — indexing a string. Returns a `str` of length 1 (CPython
  semantics: there is no `char` type — `s[0]` is a str).
- **`s[start:stop]`** — substring with both bounds. Bounds clamped to
  `[0, len(s)]` (matching CPython's slicing behaviour, which never raises).
- **`s[:stop]`** / **`s[start:]`** / **`s[:]`** — slicing with one or
  both bounds omitted. Defaults are `0` and `len(s)`.

The result of slicing is a fresh heap-allocated copy. The result of
indexing is a fresh 1-byte heap allocation wrapped in a str struct.
Both leak under the v1 no-GC policy, consistent with the rest of the
runtime.

## What v0.31 does **not** add

- **Step** — `s[::2]`, `s[1:10:2]`. Rejected with `unsupported_feature`.
- **Negative indices** for either indexing or slicing. CPython
  treats `s[-1]` as `s[len-1]` and `s[-3:-1]` similarly; we reject
  both with a pointer to a future slice. (Implementing them is mechanical
  — adjust at runtime if `i < 0` — but it widens the scope of the v0.31
  diff and isn't urgent.)
- **Out-of-bounds index** — `s[i]` with `i >= len(s)` is UB, same policy
  as `lst[i]`. A focused safety slice will add bounds checks for both.
- **`bytes`** indexing/slicing — `bytes` is not yet a v1 type.

## HIR additions

```rust
Expr::StrIndex { s: Box<TypedExpr>, index: Box<TypedExpr> }       // ty == Str
Expr::StrSlice { s: Box<TypedExpr>, start: Box<TypedExpr>, stop: Box<TypedExpr> }   // ty == Str
```

The slice's `start` and `stop` are already-defaulted I64 expressions
(check substitutes `ConstI64(0)` for omitted start and `StrLen { s }` for
omitted stop). Codegen treats them uniformly.

## Check (lower)

Extend the existing `ast::Expr::Subscript` arm in `lower_expr`:

- If `value.ty == Type::Str`:
  - If the slice expression is `ast::Expr::Slice`:
    - Reject if `step` is `Some(...)`.
    - Lower `lower` (default `ConstI64(0)`) and `upper` (default
      `StrLen { s: <cloned value> }`); coerce both to `I64`.
    - Emit `StrSlice`.
  - Otherwise treat as an index:
    - Lower and coerce to I64.
    - Emit `StrIndex`.
  - Negative-index detection: reject any `ast::Expr::UnaryOp(USub, ...)` index/bound
    statically. Runtime negative values are not detected (consistent with the
    rest of the codebase's "no runtime safety" stance for now).

## Codegen

### `StrIndex`

```llvm
; %s is { i64, i8* }
%data = extractvalue { i64, i8* } %s, 1
%p    = getelementptr i8, i8* %data, i64 %i
%c    = load i8, i8* %p
%buf  = call i8* @malloc(i64 1)
store i8 %c, i8* %buf
%r0   = insertvalue { i64, i8* } undef, i64 1, 0
%r1   = insertvalue { i64, i8* } %r0, i8* %buf, 1
```

### `StrSlice`

```llvm
; %s is { i64, i8* }
%len  = extractvalue { i64, i8* } %s, 0
%data = extractvalue { i64, i8* } %s, 1
; clamp start
%s0 = select i1 (icmp slt i64 %start, 0), i64 0, i64 %start
%s1 = select i1 (icmp sgt i64 %s0, %len), i64 %len, i64 %s0
; clamp stop
%t0 = select i1 (icmp slt i64 %stop, 0), i64 0, i64 %stop
%t1 = select i1 (icmp sgt i64 %t0, %len), i64 %len, i64 %t0
; out_len = max(0, t1 - s1)
%diff = sub i64 %t1, %s1
%out_len = select i1 (icmp slt i64 %diff, 0), i64 0, i64 %diff
; malloc and memcpy
%alloc_n = select i1 (icmp eq i64 %out_len, 0), i64 1, i64 %out_len   ; avoid 0-byte malloc
%buf = call i8* @malloc(i64 %alloc_n)
%src = getelementptr i8, i8* %data, i64 %s1
call void @llvm.memcpy.p0i8.p0i8.i64(i8* %buf, i8* %src, i64 %out_len, i1 false)
%r0 = insertvalue { i64, i8* } undef, i64 %out_len, 0
%r1 = insertvalue { i64, i8* } %r0, i8* %buf, 1
```

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `str_index_basic` | `s[i]` with parametric `i`; verify length-1 result + char value |
| `str_slice_basic` | `s[i:j]` with parametric bounds; verify content |
| `str_slice_open_left` | `s[:j]` — omitted start |
| `str_slice_open_right` | `s[i:]` — omitted stop |
| `str_slice_full_copy` | `s[:]` — both omitted; should equal `s` |
| `str_slice_clamped` | `s[100:200]` (way past len) — should yield "" |
| `str_index_in_fstring` | `f"first={s[0]}"` — combine with v0.30 |

## Files changed

- `crates/pyx86/src/hir.rs` — `Expr::StrIndex`, `Expr::StrSlice`.
- `crates/pyx86/src/check.rs` — extend Subscript handler for `Type::Str`;
  detect slice-vs-index; substitute defaults; reject step + negative.
- `crates/pyx86/src/codegen.rs` — inline lowering for both.
- `tests/correctness/str_index_*` / `str_slice_*` / `str_index_in_fstring/`.
- This file.
