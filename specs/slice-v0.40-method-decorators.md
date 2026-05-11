# Spec: slice v0.40 — `@property` + `@staticmethod`

> Status: in progress.

## What v0.40 adds

Two method decorators on classes, recognized syntactically by name:

- **`@property`** — defines a getter. Accessed as `obj.name` (no
  parens), lowers to a call to the underlying method.
- **`@staticmethod`** — a method that doesn't take `self`. Called via
  `Cls.method(args)` or `instance.method(args)`; both forms dispatch
  the same.

```python
class Point:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x; self.y = y

    @property
    def magnitude_sq(self) -> int:
        return self.x * self.x + self.y * self.y

    @staticmethod
    def origin() -> int:
        return 0

def main(a: int, b: int) -> int:
    p: Point = Point(a, b)
    return p.magnitude_sq + Point.origin()
```

## What v0.40 does **not** add

- **`@classmethod`** — `cls` as a value requires runtime type
  representations we don't have. Deferred.
- **Property setters** (`@x.setter`) — `obj.x = val` for a `@property`
  is rejected with a clear pointer.
- **`@staticmethod` for class instantiation pattern** (factory
  methods returning the class) — works as long as the return type is
  annotated as the class.
- **Multiple decorators stacked** on a method — rejected.

## HIR additions

Two new lists on `ClassDef`:

```rust
pub struct ClassDef {
    // existing ...
    pub property_methods: Vec<String>,    // method names declared @property
    pub static_methods: Vec<String>,      // method names declared @staticmethod
}
```

No new `Expr` variants. `@property` access lowers to `Expr::Call` for the
getter. `@staticmethod` calls lower to `Expr::Call` without a `self` arg.

## Check (lower)

**Decorator collection (Pass 0b)**: extend the method-def handler to
also accept `@property` and `@staticmethod` decorators in addition to
`@abstractmethod`. Record the method name in the appropriate per-class
list.

**Method signature** for staticmethods: `self` is not a parameter. The
mangled name is still `<Class>.<method>`; codegen emits a normal
function.

**Attribute access** (`obj.name` in expression context): if `name` is
a property of `obj`'s class (walking the inheritance chain), lower to
a method call. Existing field-access path handles ordinary fields.

**Attribute call** (`obj.method(args)` or `Cls.method(args)`):
- If the receiver `Cls` is a class name AND `method` is a staticmethod:
  lower to `Expr::Call { callee: "<Cls>.<method>", args: [user_args] }`.
- If `obj.method` where `obj` is an instance and method is a
  staticmethod: same — drop the self.
- Otherwise: existing instance method path (with self prepended).

**Attribute assignment** (`obj.name = val`): if `name` is a property,
reject with a clear "property setters not supported" message. Existing
field-assignment path otherwise.

## Codegen

No changes — properties and staticmethods compile to ordinary functions
and ordinary calls. The difference is purely how check lowers their
call sites.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `property_basic` | `Point.magnitude_sq` as `@property`; access without parens |
| `property_inherited` | subclass inherits parent's `@property` |
| `property_uses_method_call` | property body calls another method on self |
| `staticmethod_basic` | called via `Cls.method(args)` |
| `staticmethod_via_instance` | same method called via `obj.method(args)` |
| `staticmethod_returns_class` | factory pattern returning a class instance |

## Files changed

- `crates/pyx86/src/hir.rs` — `ClassDef.property_methods` +
  `ClassDef.static_methods`; getter helpers.
- `crates/pyx86/src/check.rs` — decorator recognition; sig collection
  for staticmethods (no `self`); attribute-access property lowering;
  attribute-call staticmethod dispatch; property assignment rejection.
- `tests/correctness/property_*`, `staticmethod_*`.
- This file.
