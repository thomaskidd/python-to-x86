# Spec: slice v0.36 — operator overloading

> Status: in progress.

## What v0.36 adds

When both operands of a binary operator are the same class type
(or an unary operator's operand is a class instance), dispatch to the
corresponding Python dunder method. v0.36 covers:

| Operator | Dunder |
|---|---|
| `+` `-` `*` `/` `//` `%` `**` | `__add__`, `__sub__`, `__mul__`, `__truediv__`, `__floordiv__`, `__mod__`, `__pow__` |
| `&` `|` `^` `<<` `>>` | `__and__`, `__or__`, `__xor__`, `__lshift__`, `__rshift__` |
| `<` `<=` `>` `>=` | `__lt__`, `__le__`, `__gt__`, `__ge__` |
| unary `-` | `__neg__` |

`==` / `!=` are already covered by v0.35's `__eq__` dispatch.

```python
class Vec:
    x: int
    y: int
    def __init__(self, x: int, y: int): self.x = x; self.y = y
    def __add__(self, other: Vec) -> Vec: return Vec(self.x + other.x, self.y + other.y)
    def __neg__(self) -> Vec: return Vec(-self.x, -self.y)
    def __lt__(self, other: Vec) -> bool: return self.x + self.y < other.x + other.y
```

Like the other class-instance dispatches, the resolution walks the
inheritance chain (so subclasses inherit operator overloads).

## What v0.36 does **not** add

- **Reflected operators** (`__radd__`, etc.) — needed when `a + b`
  is between two different types. Deferred.
- **In-place operators** (`__iadd__`, etc.) — Python falls back to
  `__add__` when not defined, which we already do for the regular form.
  Real in-place semantics are deferred.
- **Different-class operands** — rejected. Both sides must be the
  same class type. Mixing class + primitive is also rejected.
- **`__matmul__` (`@`)** — we already reject `@`; this slice doesn't
  change that.
- **Operator overloading on unary `+`, `not`, `~`** — unary `-` only.
- **`NotImplemented` return** — same as v0.35, deferred.

## HIR additions

None. Operator overload calls reuse `Expr::Call` with the resolved
mangled method name. No new expression variants.

## Check (lower)

`lower_expr`'s `ast::Expr::BinOp` arm: after lowering both operands,
check whether both are `Type::Class(_)`. If so:

- If the classes differ → reject.
- Otherwise: resolve `binop_dunder(op)` on the class. Not found →
  reject. Found → emit `Expr::Call { callee, args: [lhs, rhs] }`.

Same shape for `ast::Expr::UnaryOp` USub on a class instance:
resolve `__neg__`, emit a single-arg `Expr::Call`. Other unary ops
on classes are rejected (the matching dunders would be `__pos__`,
`__invert__`, etc. — defer until needed).

`Compare` arm: extended in v0.35 for `==`/`!=`; v0.36 also handles
`<` `<=` `>` `>=` via `__lt__` / `__le__` / `__gt__` / `__ge__`.
Reflected dispatch (Python's `__gt__` fallback when `__lt__` is missing)
is not implemented — the user must define the operator they use.

## Codegen

No changes — operator overloads are ordinary method calls.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `op_add` | `Vec + Vec → Vec` via `__add__` |
| `op_sub_mul` | `Money - Money → Money` and `Money * Money → int` (mixed return type) |
| `op_neg` | unary `-Vec → Vec` via `__neg__` |
| `op_compound_lt` | `Norm < Norm → bool` via `__lt__` |
| `op_compound_le_gt_ge` | all three ordering dunders |
| `op_inherited` | subclass inherits parent's `__add__` |
| `op_repr_in_fstring` | combines `__add__`, `__repr__`, and f-strings |

## Files changed

- `crates/pyx86/src/check.rs` — `binop_dunder` / `binop_symbol` /
  `cmp_dunder` / `cmp_symbol` helpers; class+class dispatch in the
  `BinOp` and `Compare` arms; unary `-` dispatch on class instances.
- `tests/correctness/op_*` — seven new programs.
- This file.
