# Spec: slice v0.34 — `abc.ABC` + `@abstractmethod`

> Status: in progress.

## What v0.34 adds

Compile-time enforcement of abstract-method protocols. CLAUDE.md
commits to this as a syntactic recognition — no metaclass machinery
runs at compile time, we just match the textual `from abc import ...`
and the `@abstractmethod` decorator.

```python
from abc import ABC, abstractmethod

class Shape(ABC):
    @abstractmethod
    def area(self) -> int:
        ...
    def describe(self) -> str:
        return "I'm a shape"   # concrete method on an abstract class

class Circle(Shape):
    r: int
    def __init__(self, r: int):
        self.r = r
    def area(self) -> int:    # required override
        return self.r * self.r * 3
```

Operational behaviour:

- **Abstract class** = a class with at least one unimplemented abstract
  method (declared or inherited).
- **`<AbstractClass>(...)` is a compile-time error** — you can't
  instantiate.
- **Concrete subclass must implement every inherited abstract method**,
  or it stays abstract.
- v0.34 keeps **single-base inheritance** (the v0.33 rule). The base
  may be concrete or abstract; `class Foo(ABC):` is a valid form where
  `ABC` is treated as a no-op base that signals the ABC protocol.
  Multi-base interface-style inheritance (`class Dog(Animal, Drawable,
  Serializable):`) is **deferred** — it interacts with vtables to be
  useful anyway, and will land alongside the vtable slice.
- Abstract classes can declare fields and concrete methods; their
  subclasses inherit both. The only thing that makes them abstract is
  having an unimplemented `@abstractmethod`.

## What v0.34 does **not** add

- **Polymorphism via vtable dispatch.** An abstract class as a *value
  type* (parameter / variable / return annotation) is rejected with a
  clear "deferred to vtable slice" error. Once vtables land, this
  restriction lifts. v0.34 ships the declaration + correctness side
  only; the dispatch story stays static.
- **`ABCMeta` as a user-accessible metaclass** — out of scope per
  CLAUDE.md.
- **Reflection** (`isinstance`, `issubclass`) — deferred.
- **Abstract properties / classmethods / staticmethods** — deferred.

## HIR additions

```rust
pub struct ClassDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
    pub parent: Option<ClassId>,                  // concrete parent (v0.33)
    pub abstract_methods: Vec<String>,            // NEW — unimplemented method names
}
```

No new `Expr` / `Stmt` variants needed — abstract methods are
syntactically a function-def with `@abstractmethod` and a body we don't
lower.

## Check (lower)

**Imports:** `from abc import ABC, abstractmethod` is recognized
alongside the other documentary imports (`__future__`, `pyx86.types`,
`math`). No file load. `ABC` is pre-registered as a sentinel ClassId
that lives across the lower call (singleton); `abstractmethod` is
recognized as a decorator name.

**Pass 0a / 0b:**

- Class bases are split into at most one **concrete parent** (any
  registered class that is itself concrete or `ABC` of an abstract
  chain still rooted in a concrete shape... actually: the first base
  that isn't an ABC subclass) and any number of **ABC parents**
  (registered ABCs). `ABC` itself counts as an ABC parent and
  contributes no methods. Multiple concrete bases are rejected.
- Abstract method collection per class:
  1. Start with the union of all parents' `abstract_methods` sets
     (concrete + ABC).
  2. For each method declared in the body:
     - If decorated with `@abstractmethod`: add its name to the
       abstract set. The body is **not** lowered (no code emitted).
     - Otherwise: remove the name from the abstract set (concrete
       override).
  3. Store the final set as `class.abstract_methods`.
  4. `is_abstract(c) := !c.abstract_methods.is_empty()`.

**Field rules** for v0.34: ABCs (classes with at least one declared
`@abstractmethod` OR explicit `ABC` in their bases) may not declare
fields. Concrete classes work as in v0.33.

**Restrictions on annotations:** `parse_type_annotation` rejects
`Type::Class(c)` for an abstract `c` with a deferred-to-vtable error.

**Instantiation:** the constructor path (`Foo(...)`) checks
`is_abstract(c)` and rejects.

**Method resolution:** unchanged from v0.33 — `resolve_method` walks
the parent chain. ABC parents are **not** in that chain (only the
single concrete `parent` is). This is fine: ABC abstract methods are
unimplemented; concrete subclasses are required to provide them, so
`resolve_method` always finds the concrete override locally. ABC
concrete methods (if any) are deferred — to inherit them, the class
should use the regular concrete-parent slot.

## Codegen

No changes. Abstract methods produce no functions; they are not in
`signatures` at the codegen stage. `is_abstract` doesn't reach codegen
either — instantiation is rejected upstream.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `abc_concrete_subclass` | `class Shape(ABC); @abstractmethod def area(self)`; `class Square(Shape)` implements `area`; instantiate `Square` |
| `abc_three_methods` | Two abstracts; subclass implements both, returns a sum |
| `abc_inherits_concrete_method` | ABC has one abstract + one concrete method; subclass uses the inherited concrete method |
| `abc_interface_style` | Concrete `Animal`; ABC `Greeter` with abstract `greet`; class `Dog(Animal, Greeter)` implements `greet` and reuses Animal's field/method |
| `abc_chain_of_abstract` | `A(ABC) → B(A) (still abstract) → C(B) (concrete)`; checks transitive abstract-method propagation |

## Compile-time error tests

(Just smoke-tested manually; these don't need fuzzing — the goal is the
error message.)

- Instantiate an abstract class → `unsupported_feature: cannot
  instantiate abstract class <Name>: missing implementations of ...`
- Use abstract class as parameter annotation → `unsupported_feature:
  using abstract class <Name> as a value type is deferred to the
  vtable slice`
- ABC declares a field → `unsupported_feature: abstract classes may
  not declare fields in v0.34`

## Files changed

- `crates/pyx86/src/hir.rs` — `ClassDef.abstract_methods`, getter helpers.
- `crates/pyx86/src/check.rs` — import handling; pre-register `ABC`;
  recognize `@abstractmethod`; multi-base parsing (one concrete + N ABC);
  abstract-method propagation; instantiation + annotation restrictions.
- `tests/correctness/abc_*` — five new programs.
- This file.
