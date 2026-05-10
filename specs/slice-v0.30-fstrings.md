# Spec: slice v0.30 — f-strings

> Status: in progress.

## What v0.30 adds

f-strings of the form `f"...{expr}...{expr}..."` where each interpolated
expression has a statically-known type that we know how to format.

```python
def greet(name: str, n: int) -> str:
    return f"hello {name}, n={n}, ok={n > 0}"
```

Supported interpolated types in v0.30:

| Type | Format |
|---|---|
| `int` (any width — `i8`/`i16`/`i32`/`i64` and `bool` widened to i64) | base-10, signed, no padding (`snprintf("%lld", ...)`) |
| `str` | identity (the string is spliced in verbatim) |
| `bool` | `"True"` / `"False"` (matching CPython) |

The result of an f-string expression has type `str`.

## What v0.30 does **not** add

- **`f64` interpolation** — the existing main-return float printer
  prints to stdout, not into a buffer. Refactoring it for f-string use
  is its own slice. v0.30 rejects f64 in f-strings with a clear
  `unsupported_feature` error.
- **Format specs** (`{x:5d}`, `{x:.2f}`, etc.) — anything after a `:`
  in an interpolation. Rejected.
- **Conversions** (`{x!r}`, `{x!s}`, `{x!a}`) — rejected.
- **Multi-line / nested f-strings** — not encountered in practice in
  the typed Python subset we accept; rejected for now.
- **Containers in f-strings** (`f"{lst}"`, `f"{d}"`, `f"{point}"`) —
  would need Python's `repr` machinery, which v0.27 deferred.
  Rejected with a clear pointer.

## HIR additions

One new `Expr` variant:

```rust
Expr::FormatToStr { inner: Box<TypedExpr> }    // ty == Type::Str
```

f-strings themselves are not first-class in the HIR — they are lowered
in check to a tree of `StrConcat` over `StrLit` (the literal segments)
and `FormatToStr` (the interpolated expressions).

## Check (lower)

New handler for `ast::Expr::JoinedStr`:

1. If `values` is empty: return an empty `StrLit("")`.
2. For each value:
   - `Constant(Str(s))` → `StrLit(s)`
   - `FormattedValue` with no conversion and no format spec:
     - Lower the value.
     - Check that its type is i8/i16/i32/i64/Bool/Str. Reject otherwise
       with the type name and a pointer to the spec.
     - Wrap in `FormatToStr` (if type is `Str` already, skip the wrap —
       just use the lowered value).
   - `FormattedValue` with `conversion != -1` or `format_spec != None`:
     reject with `unsupported_feature`.
3. Fold the segments left-to-right with `StrConcat`. If there's exactly
   one segment, return it directly (no wrap).

## Codegen

Add a runtime helper `pyx86_i64_to_str`:

```llvm
define internal { i64, i8* } @pyx86_i64_to_str(i64 %x) {
entry:
  %buf = call i8* @malloc(i64 24)           ; max i64 fits in 21 chars
  %n = call i32 (i8*, i8*, ...) @sprintf(i8* %buf, i8* @.fmt_lld, i64 %x)
  %len = sext i32 %n to i64
  %s0 = insertvalue { i64, i8* } undef, i64 %len, 0
  %s1 = insertvalue { i64, i8* } %s0, i8* %buf, 1
  ret { i64, i8* } %s1
}
```

Add two compile-time constants: `@.str.true = "True"`, `@.str.false = "False"`,
each as a 4 / 5-byte global plus an inline str-struct constructor when
formatting bool.

`lower_format_to_str(&inner)` dispatch:
- `Type::I64` → call `pyx86_i64_to_str(inner)`.
- `Type::I8` / `I16` / `I32` → sext to i64 first, then call.
- `Type::Bool` → emit a `select` on inner that picks the `True` or
  `False` str-struct constants.
- `Type::Str` → identity (the lower stage already unwrapped it; this
  branch should not be hit but defended for safety).
- Anything else → panic (check should have rejected).

Wired-in declaration: `declare i32 @sprintf(i8*, i8*, ...)` — already
present (used by the float printer).

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `fstring_basic` | `f"x={x}, y={y}"` with two ints — sanity. |
| `fstring_no_interp` | `f"hello"` — pure literal segment, just to verify the empty/single-segment path. |
| `fstring_bool` | `f"flag={flag}"` (after `flag: bool = n > 0`) — covers both `True` and `False` outputs. |
| `fstring_str_passthrough` | `f"hello {name}!"` with `name: str` — verifies the no-wrap str path. |
| `fstring_mixed` | `f"{name} #{i}: {flag}"` — all three supported types in one literal. |
| `fstring_int_widths` | one i32 + one i64 in the same f-string — verifies width widening. |
| `fstring_negative_int` | `f"x={x}"` with negative x — verifies the minus sign. |

## Out-of-scope rejection tests

(check-only — no fuzzing needed, just verify the error message is clear.
Captured in compiler unit tests, not the correctness corpus.)

- `f"{x:5d}"` → unsupported format spec
- `f"{x!r}"` → unsupported conversion
- `f"{f}"` where `f: float` → unsupported f64 interpolation

## Files changed

- `crates/pyx86/src/hir.rs` — `Expr::FormatToStr`.
- `crates/pyx86/src/check.rs` — `JoinedStr` handler; reject specs/conversions.
- `crates/pyx86/src/codegen.rs` — `pyx86_i64_to_str` runtime helper;
  `True`/`False` constants; `lower_format_to_str` dispatch; wire into
  the existing expression-lowering switch.
- `tests/correctness/fstring_*/` — seven new programs.
- This file.
