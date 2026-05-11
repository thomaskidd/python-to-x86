# Spec: slice v0.37 — vtable dispatch for ABC chains

> Status: in progress.

## What v0.37 adds

The one architecturally novel piece left from CLAUDE.md's v1 scope:
**vtable-based dynamic dispatch for abstract-typed values**. v0.34
deferred this and rejected abstract-typed parameters / locals; v0.37
lifts that restriction.

```python
from abc import ABC, abstractmethod

class Shape(ABC):
    @abstractmethod
    def area(self) -> int: ...

class Square(Shape):
    side: int
    def __init__(self, side: int): self.side = side
    def area(self) -> int: return self.side * self.side

class Circle(Shape):
    r: int
    def __init__(self, r: int): self.r = r
    def area(self) -> int: return self.r * self.r * 3

def total_area(shapes: list[Shape]) -> int:   # <-- now allowed
    total: int = 0
    for s in shapes:
        total = total + s.area()              # <-- vtable dispatch
    return total
```

## Design

- A class is in an **ABC chain** iff itself or any ancestor is
  abstract. (`needs_vtable(c)` = transitive parent-walk.)
- Per ABC chain, a canonical **vtable type** is established at the
  topmost abstract ancestor (the "vtable root"). Its slots are the
  union of all method names that are `@abstractmethod` anywhere in the
  chain, sorted alphabetically for determinism.
- Every class in an ABC chain has the same LLVM vtable type as its
  root; only the global filled-in instance varies. Concrete classes
  emit a `@vtable_<Class>` constant whose entries point to the
  resolved concrete implementations.
- **Instance layout** for ABC-chain classes prepends an `i8*`
  (the vtable pointer). All subsequent fields shift down by 1. The
  prefix-compatibility invariant (v0.33) still holds because every
  class in the chain shares the same vtable slot at offset 0.
- `ClassNew` for an ABC-chain concrete class stores
  `@vtable_<Class>` into the instance's slot 0 before calling
  `__init__`.
- **Method dispatch** on a receiver in an ABC chain calling a method
  that's in the chain's vtable slots: load vtable, GEP to slot, load
  fn pointer, indirect-call. Receivers outside any ABC chain (or
  calling non-virtual methods) still dispatch statically.

## What v0.37 does **not** add

- **Multi-base ABC inheritance** (`class Dog(Animal, Drawable):`) —
  still deferred. Single-base chains only.
- **Vtable for non-ABC-rooted polymorphism** — concrete classes that
  override parent methods still dispatch statically (no "virtual by
  default"). Only methods that are `@abstractmethod` anywhere in the
  chain go through the vtable.
- **Devirtualization of monomorphizable call sites** — CLAUDE.md
  mentions both monomorphization and vtable. v0.37 always uses vtable
  for ABC chains, never inlines / specializes. A later optimization
  slice can devirtualize known-concrete receivers.
- **`isinstance(x, A)`** — still deferred.
- **`super().<abstract_method>()`** — calling an abstract method via
  super is a Python anti-pattern and isn't supported.

## HIR additions

```rust
pub struct ClassDef {
    // existing fields ...
    /// v0.37: the topmost abstract ancestor (inclusive of self if abstract).
    /// `None` if the class is not in an ABC chain.
    pub vtable_root: Option<ClassId>,
}
```

Helper on `ClassId`:
- `vtable_root() -> Option<ClassId>`
- `needs_vtable() -> bool` (alias for vtable_root.is_some())
- `vtable_slots() -> Vec<String>` — sorted abstract-method names of the
  root's chain (cached on the root).

No new `Expr` variants. Method dispatch through a vtable is a new
codegen pattern but uses the existing `Expr::Call` shape with a special
callee form, or alternatively a new `Expr::VirtualCall`:

```rust
Expr::VirtualCall {
    receiver: Box<TypedExpr>,        // type is the static class
    vtable_slot: usize,              // index into the vtable
    method_name: String,             // for IR readability + sig lookup
    sig_params: Vec<Type>,           // includes self
    sig_return: Type,
    args: Vec<TypedExpr>,            // includes self as args[0]
}
```

This keeps the indirect-call clean in codegen.

## Check (lower)

**Pass 0b** (class processing) is extended to compute `vtable_root`:

```rust
let needs_vtable = class.is_abstract() ||
    parent.map(|p| p.vtable_root().is_some()).unwrap_or(false);
let vtable_root = if needs_vtable {
    if class.is_abstract() && parent.map(|p| p.vtable_root()).flatten().is_none() {
        Some(class)  // this class is the root
    } else {
        parent.unwrap().vtable_root()  // inherit
    }
} else {
    None
};
```

Pass also computes per-root the canonical method slot list.

**`reject_abstract_type` removed** — abstract-typed values now flow.

**Method dispatch in `lower_expr`**:

```rust
if let Type::Class(c) = obj.ty {
    if let Some(root) = c.vtable_root() {
        if let Some(slot) = root.vtable_slots().iter().position(|n| n == method) {
            // Vtable dispatch.
            // Need the signature — get it from any concrete impl. The
            // root's abstract method declaration carries the signature
            // (we store the @abstractmethod's signature too).
            ...
            return Ok(VirtualCall {...});
        }
    }
    // Fall through to static dispatch.
    ...
}
```

For this to work, we need signatures for abstract methods. v0.34
**didn't** store these (abstract methods skip the signature table to
avoid linking issues). v0.37 stores them under the mangled name —
they're never **called as functions**, only used for signature
metadata. Codegen filters out abstract-method functions when emitting
`define` blocks.

**Constructor:** for ABC-chain concrete classes, the constructor
synthesizes vtable-pointer initialization before calling `__init__`.

## Codegen

**LLVM struct layout** for ABC-chain classes:

```llvm
%pyx86.Square = type { i8*,        ; vtable pointer
                       i64 }       ; side
```

Non-ABC classes remain unchanged.

**Field offsets** in codegen GEPs shift by 1 for ABC-chain classes.
Easiest implementation: store a `field_offset_base` derived from
`class.needs_vtable()`, and add it to every field index lookup.

**Vtable globals**:

```llvm
%VTable_Shape = type { i8* }       ; one slot per abstract method
                                    ; declared in Shape's chain

@vtable_Square = private unnamed_addr constant %VTable_Shape {
    i8* bitcast (i64 (%pyx86.Square*)* @py_Square.area to i8*)
}
@vtable_Circle = private unnamed_addr constant %VTable_Shape {
    i8* bitcast (i64 (%pyx86.Circle*)* @py_Circle.area to i8*)
}
```

We use `i8*` for slot types and bitcast at the call site to recover the
actual function type. This avoids the per-method-signature struct
explosion.

**ClassNew** for ABC-chain concrete classes: store `@vtable_<Class>`
into slot 0 after malloc. The `Stmt::SetField`-style logic is inlined
in `lower_class_new`.

**Method dispatch via VirtualCall**:

```llvm
; %obj is %pyx86.SomeChainClass*
%vt_p = getelementptr <obj_layout>, <obj_layout>* %obj, i32 0, i32 0
%vt = load i8*, i8** %vt_p
%vt_typed = bitcast i8* %vt to %VTable_<root>*
%slot_p = getelementptr %VTable_<root>, %VTable_<root>* %vt_typed, i32 0, i32 <slot>
%fn_raw = load i8*, i8** %slot_p
%fn = bitcast i8* %fn_raw to <method_sig>*
%result = call <method_sig> %fn(<args>...)
```

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `vtable_abstract_param` | function takes `Shape` (abstract), receives `Square`, calls `.area()` |
| `vtable_polymorphic_list` | `list[Shape]` with mixed `Square` / `Circle`, sums areas |
| `vtable_abstract_local_var` | `s: Shape = Square(...)` then `s.area()` |
| `vtable_inherits_through_chain` | `A(ABC) → B(A) → C(B)` where C implements; calls through `A`-typed value |
| `vtable_concrete_methods_static` | ABC chain class with both abstract + concrete methods; concrete methods still static-dispatch (regression — must not pay vtable cost for them) |

## Files changed

- `crates/pyx86/src/hir.rs` — `ClassDef.vtable_root`, `Expr::VirtualCall`,
  helper methods.
- `crates/pyx86/src/check.rs` — compute `vtable_root` + canonical slot
  list; remove `reject_abstract_type`; method-dispatch site emits
  `VirtualCall` when applicable; abstract methods now get registered
  signatures (under their mangled name) so dispatch can read them.
- `crates/pyx86/src/codegen.rs` — vtable type + global emission; field
  offset shift for ABC-chain classes; `lower_class_new` vtable-pointer
  init; `Expr::VirtualCall` lowering.
- `tests/correctness/vtable_*` — five new programs.
- This file.
