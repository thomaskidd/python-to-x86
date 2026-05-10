# Spec: slice v0.33 — single concrete inheritance

> Status: in progress.

## What v0.33 adds

Allows one concrete class to inherit from another:

```python
class Animal:
    name: str
    def __init__(self, name: str):
        self.name = name
    def species(self) -> str:
        return "generic"

class Dog(Animal):
    breed: str
    def __init__(self, name: str, breed: str):
        super().__init__(name)
        self.breed = breed
    def species(self) -> str:
        return "dog"
```

Specifically:

- **One base class** (`class B(A):`). Multiple bases are rejected.
- **Field layout**: subclass fields are appended to the parent's. Parent fields
  are at the same offsets in both, so a `B*` is interchangeable with an `A*`
  for field access.
- **Method inheritance**: if a subclass doesn't define a method, the parent's
  is used. If it does, the override is used (no chaining unless the user
  explicitly calls `super().method(...)`).
- **`super()`** for `super().__init__(...)` and `super().<method>(...)` —
  resolves at compile time to a direct call to the parent's mangled method.
- **Subtyping in assignments / parameters**: a `Type::Class(B)` value can flow
  into a slot annotated `Type::Class(A)` when B inherits from A (transitively).
  The LLVM type for both is a struct pointer; this is a documented bitcast.

## What v0.33 does **not** add — explicit divergences from CPython

- **No dynamic dispatch.** A method call on a `Type::Class(A)`-typed value
  *always* dispatches to A's method, even if the value's actual concrete
  type is B and B overrides the method. This matches CLAUDE.md's commitment:
  polymorphic dispatch is reserved for ABCs + vtables (a later slice).
  Documented as a deliberate divergence. Tests are crafted to keep variable
  annotations matching the concrete type.
- **No `isinstance(x, A)`** — deferred, would need runtime type tags.
- **No multiple inheritance** of concrete classes — rejected.
- **No `__init_subclass__`, `__set_name__`, etc.** — none of the meta-OOP
  hooks are supported.
- **No diamond resolution / MRO** — out of scope; we only have linear chains.
- **`super()` only inside a method** of a class with a parent. The bare form
  `super()` without a method call is rejected (we don't synthesize a proxy).

## HIR additions

```rust
pub struct ClassDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub parent: Option<ClassId>,        // NEW
}
```

The `fields` vector now includes the parent's fields **prepended** in order,
so codegen sees one flat layout per class.

Methods continue to live as top-level functions with mangled names
(`<ClassName>.<method>`). Inheritance is resolved at the call site by
walking the chain.

## Check (lower)

**Pass 0a** (class pre-registration): unchanged.

**Pass 0b** (class body processing):

- Allow `c.bases.len() == 1` and require the single base be an
  `ast::Expr::Name` whose id is a previously-registered class. Reject
  multiple bases, attribute bases, or generic-bracket bases.
- The base ClassId is stored on the subclass's `ClassDef.parent`.
- The subclass's `fields` vector is built as `parent_fields ++ own_fields`.
  Duplicate field names (across parent + child) are rejected to keep the
  layout unambiguous.
- The subclass's methods are stored under `<Subclass>.<method>` as before.
  If the subclass doesn't define `__init__` but the parent does, no
  alias is created — the constructor lookup walks the chain instead.

**Method resolution helper:**

```rust
fn resolve_method(class_id: ClassId, method: &str) -> Option<String> {
    let mangled = format!("{}.{}", class_id.name(), method);
    if signatures.contains(&mangled) { return Some(mangled); }
    if let Some(parent) = class_id.parent() {
        return resolve_method(parent, method);
    }
    None
}
```

Used by:
- Method calls (`obj.foo(...)` → resolve against `obj.ty`'s class).
- Constructor calls (`ClassName(...)` → resolve `__init__`).
- `super().<method>(...)` → resolve against the current method's class's
  parent.

**`coerce` extension (subtyping):**

```rust
// B → A is allowed when B inherits transitively from A.
if let (Type::Class(b), Type::Class(a)) = (e.ty, target) {
    if is_subclass_of(b, a) {
        // No-op at runtime: both lower to struct pointers; we re-tag.
        return Ok(TypedExpr::new(target, Expr::Coerce { inner: Box::new(e) }));
    }
}
```

`Expr::Coerce` already exists for numeric width conversions; for class
subtyping the codegen lowering will emit a `bitcast` between struct pointer
types — that's a no-op at the machine level but keeps LLVM IR well-typed.

**`super()` handling:**

`super()` is recognized in `lower_expr` as a Call. By itself it returns
nothing useful — we only support the form `super().<method>(...)` which is
a Call whose `func` is an Attribute whose value is a Call to `super` with
zero args. Detected at the call lowering level and resolved to a direct
call to `<ParentClassName>.<method>(self_arg, args...)`.

To know "the current method's class", `lower_method` threads a
`current_class: Option<ClassId>` through the lowering pass (added to
`Scope` or carried as an explicit parameter). The detection then reads
`current_class.parent()` for the resolution target.

## Codegen

The LLVM type for `Type::Class(_)` is unchanged: a heap-allocated struct
pointer. Because the subclass's fields include the parent's prepended, the
existing field-index lookup just works for inherited fields.

**Coerce on classes** lowers to a `bitcast`:

```llvm
%casted = bitcast {b_layout}* %from to {a_layout}*
```

In v0.33 the struct types for distinct classes are LLVM-level different,
but the underlying memory layout is prefix-compatible by construction.
The bitcast is type-system glue only.

**Class instantiation** (`B(args...)`): unchanged; calls the resolved
`<ResolvedClass>.__init__` (which may be the parent's if the subclass
doesn't override).

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `inherit_field_extension` | `class B(A)` adds a field; access both A's and B's fields via a `B` instance |
| `inherit_method_inherited` | B inherits A's method without overriding; `B().method()` returns A's behaviour |
| `inherit_method_override` | B overrides A's method; `B().method()` returns B's behaviour |
| `inherit_super_init` | B's `__init__` calls `super().__init__(args)` to initialize parent fields |
| `inherit_super_method` | B's method calls `super().<method>()` and adds to it |
| `inherit_subtyping_param` | A function annotated to take `A` receives a `B` instance and calls A's methods (documents static-dispatch divergence) |
| `inherit_three_level` | C inherits from B inherits from A; field access and method resolution walk the chain |

## Files changed

- `crates/pyx86/src/hir.rs` — `ClassDef.parent`, getter helper.
- `crates/pyx86/src/check.rs` — Pass 0b base handling; method resolution;
  `super().<method>` detection; `coerce` subtyping; `current_class` threading.
- `crates/pyx86/src/codegen.rs` — `Coerce` lowering for `Type::Class` →
  `Type::Class` (bitcast).
- `tests/correctness/inherit_*/` — seven new programs.
- This file.
