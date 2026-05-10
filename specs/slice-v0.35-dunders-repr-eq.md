# Spec: slice v0.35 — `__repr__` and `__eq__`

> Status: in progress.

## What v0.35 adds

Two dunder methods on user classes, recognized syntactically by name:

- **`__repr__(self) -> str`** — when defined on class `Foo`, calling
  `repr(x)` for `x: Foo` invokes it. f-string interpolation
  (`f"x={x}"`) does the same.
- **`__eq__(self, other: Foo) -> bool`** — when defined on class `Foo`,
  the `==` operator on two `Foo` values invokes it. `!=` calls the
  same method and negates the result.

```python
class Point:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x; self.y = y
    def __repr__(self) -> str:
        return f"Point({self.x}, {self.y})"
    def __eq__(self, other: Point) -> bool:
        return self.x == other.x and self.y == other.y

def main(a: int, b: int) -> str:
    p: Point = Point(a, b)
    q: Point = Point(a, b)
    if p == q:
        return repr(p)
    return "neq"
```

Both methods can be inherited from a parent class via the v0.33 chain
walk. Override behavior is identical to ordinary methods (static
dispatch on the receiver's declared type).

## What v0.35 does **not** add

- **`__hash__`** — needs container integration to be useful (set/dict
  keys). Deferred to a slice that lifts the i64-only restriction on
  set/dict keys.
- **Default `repr` / `__eq__`** — if a class doesn't define them,
  we **reject the operation** at compile time with a clear error, rather
  than synthesize a generic repr or identity-based eq. (CPython falls
  back; we'd rather force users to be explicit.)
- **Mixed-type equality** — `a == b` where `a: Foo, b: Bar` rejected.
- **NotImplemented return** — Python's `__eq__` can return `NotImplemented`
  to defer to the other operand. We require `__eq__` to return a `bool`.
- **`__ne__`** — `!=` invokes `__eq__` and negates; no separate `__ne__`.

## HIR additions

No new expression variants. f-string `FormatToStr` (v0.30) gains a new
input type: class instances whose class has `__repr__`. The check pass
rewrites `FormatToStr { inner: <class instance> }` into a Call to
`__repr__`, which already returns `Str`, so the existing concat machinery
applies.

`==` and `!=` between class instances are lowered as calls to
`__eq__`, producing a `Bool` value. The `Expr::Cmp` variant remains the
same; only the lower-time dispatch changes.

## Check (lower)

**`repr(x)` builtin:** new handler in `lower_builtin_call`. If `x: str`,
return `x` unchanged (mirroring Python's behaviour where `repr(str)`
quotes — actually no, Python's `repr("abc")` returns `"'abc'"`). For
v0.35, **scope is narrowed**: `repr` is only defined on class instances
(with `__repr__`). Calling `repr` on a primitive is rejected with a
clear "use f-strings or str()" pointer. This keeps the slice focused.

**f-string class interpolation:** in `lower_joined_str`, when an
interpolation's inner type is `Type::Class(c)`, resolve `__repr__` via
the inheritance chain. Found → rewrite the segment as a call to the
resolved repr method. Not found → reject with a pointer to v0.35.

**`==` and `!=` on class instances:** in `apply_cmp` (or wherever Cmp
gets lowered), if both sides are `Type::Class(_)` of the **same** class
(or one is a subclass of the other via subtyping coerce — for simplicity
v0.35 requires exactly equal types), resolve `__eq__` on the receiver's
type. Found → emit a Call expr. For `!=`, wrap the call in `Expr::Not`.
Not found → reject with a clear error.

Different-class equality (`a == b` where `a.ty != b.ty` and both are
`Type::Class`) → reject.

## Codegen

No changes. The dunders compile as ordinary methods (via the existing
`lower_method` path); the call expressions are ordinary Call exprs.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `dunder_repr_basic` | `Point` with `__repr__` → return `repr(p)` |
| `dunder_repr_in_fstring` | `f"p={p}"` — class instance interpolated via `__repr__` |
| `dunder_eq_basic` | `p == q` for two equal-content Points → True; `p == r` for unequal → False |
| `dunder_ne_basic` | `p != q` is the negation of `==` |
| `dunder_repr_inherited` | Subclass inherits parent's `__repr__` |
| `dunder_eq_override` | Subclass overrides `__eq__` |

## Files changed

- `crates/pyx86/src/check.rs` — `repr` builtin; f-string class
  interpolation via `__repr__`; `==`/`!=` dispatch via `__eq__` for
  class instances.
- `tests/correctness/dunder_*` — six new programs.
- This file.
