//! Typed high-level IR.
//!
//! Each expression carries its result type (`TypedExpr.ty`). The
//! check pass infers the type bottom-up and inserts `Coerce` nodes
//! when an expression used in a context expecting type T has type
//! U ≠ T (e.g. an i64 used where a Bool is required for `if`, or a
//! Bool used in arithmetic).
//!
//! Codegen dispatches on `ty` to choose between integer and float
//! LLVM ops, choose alloca element type, etc.

use std::cell::RefCell;

/// Compile-time identifier for an interned tuple type. Two tuples
/// with the same element types share the same `TupleId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TupleId(u32);

/// Identifier for an interned list element type. `list[int]` and
/// `list[int]` share the same `ListId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListId(u32);

/// Identifier for an interned (key, value) pair for a dict type.
/// Two `dict[int, float]` types share the same `DictId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DictId(u32);

/// Identifier for an interned element type for a set. Two `set[int]`
/// types share the same `SetId`. Layout-identical to a `dict[int, _]`
/// at the runtime level — see v0.32 spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetId(u32);

/// Identifier for a class definition. Each `class Foo:` gets a unique
/// id; same-named class redefinitions are rejected by check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClassId(u32);

#[derive(Debug, Clone)]
pub struct ClassDef {
    pub name: String,
    /// (field_name, field_type) in declaration order. For a subclass,
    /// the parent's fields are **prepended** before the subclass's own,
    /// so the underlying struct prefix matches the parent's layout.
    pub fields: Vec<(String, Type)>,
    /// Parent class for single concrete inheritance (v0.33). None for
    /// root classes. ABC bases (v0.34) are not represented here — they
    /// only contribute abstract-method names.
    pub parent: Option<ClassId>,
    /// v0.34: names of methods that are declared `@abstractmethod`
    /// (in this class or inherited from any parent/ABC) and have not
    /// yet been overridden with a concrete implementation. A class is
    /// abstract iff this list is non-empty.
    pub abstract_methods: Vec<String>,
}

thread_local! {
    static TUPLE_ARENA: RefCell<Vec<Vec<Type>>> = const { RefCell::new(Vec::new()) };
    static LIST_ARENA: RefCell<Vec<Type>> = const { RefCell::new(Vec::new()) };
    static DICT_ARENA: RefCell<Vec<(Type, Type)>> = const { RefCell::new(Vec::new()) };
    static SET_ARENA: RefCell<Vec<Type>> = const { RefCell::new(Vec::new()) };
    static CLASS_ARENA: RefCell<Vec<ClassDef>> = const { RefCell::new(Vec::new()) };
}

impl ClassId {
    pub fn intern(def: ClassDef) -> ClassId {
        CLASS_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            let id = a.len() as u32;
            a.push(def);
            ClassId(id)
        })
    }
    pub fn name(self) -> String {
        CLASS_ARENA.with(|a| a.borrow()[self.0 as usize].name.clone())
    }
    pub fn fields(self) -> Vec<(String, Type)> {
        CLASS_ARENA.with(|a| a.borrow()[self.0 as usize].fields.clone())
    }
    pub fn field_index(self, name: &str) -> Option<usize> {
        CLASS_ARENA.with(|a| {
            a.borrow()[self.0 as usize]
                .fields
                .iter()
                .position(|(n, _)| n == name)
        })
    }
    pub fn field_ty(self, name: &str) -> Option<Type> {
        CLASS_ARENA.with(|a| {
            a.borrow()[self.0 as usize]
                .fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| *t)
        })
    }
    /// Set the fields after pre-registration. Used by check to allow
    /// class names to be referenced (e.g. as method param types) before
    /// the field list is fully built.
    pub fn set_fields(self, fields: Vec<(String, Type)>) {
        CLASS_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            a[self.0 as usize].fields = fields;
        })
    }
    pub fn parent(self) -> Option<ClassId> {
        CLASS_ARENA.with(|a| a.borrow()[self.0 as usize].parent)
    }
    pub fn set_parent(self, parent: Option<ClassId>) {
        CLASS_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            a[self.0 as usize].parent = parent;
        })
    }
    pub fn abstract_methods(self) -> Vec<String> {
        CLASS_ARENA.with(|a| a.borrow()[self.0 as usize].abstract_methods.clone())
    }
    pub fn set_abstract_methods(self, methods: Vec<String>) {
        CLASS_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            a[self.0 as usize].abstract_methods = methods;
        })
    }
    pub fn is_abstract(self) -> bool {
        CLASS_ARENA.with(|a| !a.borrow()[self.0 as usize].abstract_methods.is_empty())
    }
}

impl DictId {
    pub fn intern(key: Type, value: Type) -> DictId {
        DICT_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            for (i, (k, v)) in a.iter().enumerate() {
                if *k == key && *v == value {
                    return DictId(i as u32);
                }
            }
            let id = a.len() as u32;
            a.push((key, value));
            DictId(id)
        })
    }
    pub fn key(self) -> Type {
        DICT_ARENA.with(|a| a.borrow()[self.0 as usize].0)
    }
    pub fn val(self) -> Type {
        DICT_ARENA.with(|a| a.borrow()[self.0 as usize].1)
    }
}

impl ListId {
    pub fn intern(elem: Type) -> ListId {
        LIST_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            for (i, e) in a.iter().enumerate() {
                if *e == elem {
                    return ListId(i as u32);
                }
            }
            let id = a.len() as u32;
            a.push(elem);
            ListId(id)
        })
    }
    pub fn elem(self) -> Type {
        LIST_ARENA.with(|a| a.borrow()[self.0 as usize])
    }
}

impl SetId {
    pub fn intern(elem: Type) -> SetId {
        SET_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            for (i, e) in a.iter().enumerate() {
                if *e == elem {
                    return SetId(i as u32);
                }
            }
            let id = a.len() as u32;
            a.push(elem);
            SetId(id)
        })
    }
    pub fn elem(self) -> Type {
        SET_ARENA.with(|a| a.borrow()[self.0 as usize])
    }
}

impl TupleId {
    /// Intern a tuple-element list and get back its id. Idempotent
    /// for repeated calls with the same elements.
    pub fn intern(elems: Vec<Type>) -> TupleId {
        TUPLE_ARENA.with(|a| {
            let mut a = a.borrow_mut();
            for (i, existing) in a.iter().enumerate() {
                if existing == &elems {
                    return TupleId(i as u32);
                }
            }
            let id = a.len() as u32;
            a.push(elems);
            TupleId(id)
        })
    }
    /// Borrow the element types of this tuple. Panics if the id is invalid.
    /// `f` is called with a slice of the elements; the borrow is released
    /// once `f` returns.
    pub fn with_elems<R>(self, f: impl FnOnce(&[Type]) -> R) -> R {
        TUPLE_ARENA.with(|a| {
            let a = a.borrow();
            f(&a[self.0 as usize])
        })
    }
    /// Convenience: clone the elements out of the arena.
    pub fn elems(self) -> Vec<Type> {
        self.with_elems(|e| e.to_vec())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    I8,
    I16,
    I32,
    /// 64-bit signed integer. The default int type and what
    /// `: int` annotations mean. Wraps on overflow.
    I64,
    /// IEEE-754 double-precision float. What `: float` means.
    F64,
    /// Internal type produced by comparisons, `not`, boolean literals.
    /// Lowered as LLVM `i1`.
    Bool,
    /// Fixed-arity heterogeneous tuple. Stored as an LLVM struct,
    /// passed by value (no heap). Indexable only by constant integer
    /// at compile time. Element types live in a thread-local arena
    /// keyed by `TupleId` so `Type` remains `Copy`.
    Tuple(TupleId),
    /// Homogeneous heap-allocated list `list[T]`. Stored as a value-
    /// type `{ i64 len, T* data }` carrying the length and a pointer
    /// to a malloc'd buffer. Element type is interned via `ListId`.
    /// v0.19: literal construction, runtime indexing, len(), iteration.
    /// Append / mutation is deferred.
    List(ListId),
    /// Immutable byte-string. Stored as `{ i64 len, i8* data }`. For
    /// literals the data pointer aliases a compile-time constant; for
    /// runtime concatenations it's a fresh heap allocation. v0.22:
    /// literal, len, concat, ==/!=. Subscripting, slicing, methods
    /// deferred.
    Str,
    /// Heap-allocated dict[K, V]. Stored as a pointer to
    /// `{ i64 size, i64 cap, i8* slots }`. v0.26: K must be I64,
    /// V must be I64 (extension to other types is mostly mechanical
    /// — needs hash function per K type and stride per V type).
    /// Read-only operations only: literal, lookup `d[k]`, len, `k in d`.
    /// Mutation (`d[k] = v`) deferred to a follow-up.
    Dict(DictId),
    /// `set[T]`. v0.32: T must be I64. Layout-identical to a
    /// `dict[i64, i64]` — same `{ size, cap, slots }` heap struct,
    /// same runtime helpers. The per-slot value field is stored as 0
    /// and ignored on read. Kept as a distinct `Type` so the user gets
    /// proper type errors (set ≠ dict).
    Set(SetId),
    /// User-defined class instance. Stored as a heap-allocated struct
    /// pointer; ref-semantics like Python objects. Fields and methods
    /// resolved against the ClassDef in the arena.
    Class(ClassId),
}

impl Type {
    pub fn name(self) -> String {
        match self {
            Type::I8 => "i8".to_string(),
            Type::I16 => "i16".to_string(),
            Type::I32 => "i32".to_string(),
            Type::I64 => "int".to_string(),
            Type::F64 => "float".to_string(),
            Type::Bool => "bool".to_string(),
            Type::Tuple(id) => {
                let inner = id.with_elems(|elems| {
                    elems
                        .iter()
                        .map(|t| t.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                });
                format!("tuple[{}]", inner)
            }
            Type::List(id) => format!("list[{}]", id.elem().name()),
            Type::Str => "str".to_string(),
            Type::Dict(id) => format!("dict[{}, {}]", id.key().name(), id.val().name()),
            Type::Set(id) => format!("set[{}]", id.elem().name()),
            Type::Class(id) => id.name(),
        }
    }
    /// Width of the integer type in bits, or None for non-int types.
    pub fn int_width(self) -> Option<u8> {
        match self {
            Type::I8 => Some(8),
            Type::I16 => Some(16),
            Type::I32 => Some(32),
            Type::I64 => Some(64),
            _ => None,
        }
    }
    pub fn is_int(self) -> bool {
        self.int_width().is_some()
    }
    pub fn is_tuple(self) -> bool {
        matches!(self, Type::Tuple(_))
    }
    /// True iff `self` is a `Type::Class(c)` whose class is abstract
    /// (has unimplemented abstract methods). Used by check to reject
    /// abstract types in value-flow positions until vtables land.
    pub fn is_abstract_class(self) -> bool {
        matches!(self, Type::Class(c) if c.is_abstract())
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct Program {
    /// All user-defined functions in declaration order. Exactly one
    /// is named "main" — that's the entry point invoked by the C
    /// `main(argc, argv)` wrapper.
    pub functions: Vec<Function>,
}

impl Program {
    pub fn main(&self) -> &Function {
        self.functions
            .iter()
            .find(|f| f.name == "main")
            .expect("Program invariant: must contain a `main` function")
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { name: String, value: TypedExpr },
    Return { value: TypedExpr },
    If {
        cond: TypedExpr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While { cond: TypedExpr, body: Vec<Stmt> },
    Break,
    Continue,
    /// `<list>.append(<value>)`. List must be a Var (so codegen knows
    /// which slot to mutate); same shared heap struct as Python.
    ListAppend { list: TypedExpr, value: TypedExpr },
    /// `<set>.add(<value>)`. Set must be a Var so the runtime mutation
    /// is visible through all aliases.
    SetAdd { set: TypedExpr, value: TypedExpr },
    /// `<obj>.<field> = <value>`. The obj expression evaluates to a
    /// class instance pointer; the field index is resolved at check.
    SetField {
        obj: TypedExpr,
        field_index: usize,
        value: TypedExpr,
    },
    /// `<container>[<key>] = <value>`. In v0.28, `container.ty` must be
    /// `Type::Dict`. Codegen lowers to `pyx86_dict_i64_insert`.
    SetSubscript {
        container: TypedExpr,
        key: TypedExpr,
        value: TypedExpr,
    },
    /// Expression evaluated for its side effect; result discarded.
    /// Currently only used for Call expressions in stmt position.
    ExprStmt(TypedExpr),
}

/// An expression annotated with its result type. Operands inside `expr`
/// are themselves `TypedExpr`s — types propagate through the tree.
#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub ty: Type,
    pub expr: Expr,
}

impl TypedExpr {
    pub fn new(ty: Type, expr: Expr) -> Self {
        Self { ty, expr }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    ConstI64(i64),
    ConstF64(f64),
    ConstBool(bool),
    /// Reference to a parameter or previously assigned local.
    Var(String),
    BinOp { op: BinOp, lhs: Box<TypedExpr>, rhs: Box<TypedExpr> },
    UnaryOp { op: UnaryOp, operand: Box<TypedExpr> },
    Cmp { op: CmpOp, lhs: Box<TypedExpr>, rhs: Box<TypedExpr> },
    /// Python-style chained comparison `a < b < c < d`. All sub-expressions
    /// are pure in the current language subset; codegen evaluates each
    /// operand once per appearance and AND's the i1 results.
    CmpChain { first: Box<TypedExpr>, rest: Vec<(CmpOp, TypedExpr)> },
    /// Logical `not`. Always produces Bool.
    Not(Box<TypedExpr>),
    /// `and` / `or` with short-circuit value semantics. Result type is
    /// the unified type of the two branches (currently always I64;
    /// once floats land it can be F64 too).
    BoolOp { op: BoolOp, lhs: Box<TypedExpr>, rhs: Box<TypedExpr> },
    Call { callee: String, args: Vec<TypedExpr> },
    /// Insert a type conversion. The inner.ty is the source type;
    /// the surrounding TypedExpr.ty is the target. Codegen emits the
    /// appropriate LLVM coercion (zext, sext, sitofp, fptosi, icmp-ne-0).
    Coerce { inner: Box<TypedExpr> },
    /// Construct a tuple from N values. Element types must match the
    /// surrounding TypedExpr.ty (which must be a Tuple).
    TupleLit { elements: Vec<TypedExpr> },
    /// Index into a tuple at a compile-time constant position.
    /// Result type is the element type at that index.
    TupleIndex { tuple: Box<TypedExpr>, index: usize },
    /// Construct a list from N values. The surrounding TypedExpr.ty
    /// must be a List; all elements are coerced to the list's element type.
    ListLit { elements: Vec<TypedExpr> },
    /// Runtime list indexing. The list and index are evaluated; the
    /// load happens at runtime. Bounds are NOT currently checked
    /// (matching CPython's IndexError behaviour is deferred).
    ListIndex { list: Box<TypedExpr>, index: Box<TypedExpr> },
    /// `len(list)` — extract the length field. Made an Expr (not a
    /// builtin call) because it's a single-instruction GEP/extractvalue
    /// the codegen handles directly.
    ListLen { list: Box<TypedExpr> },
    /// Concatenate two lists into a new heap-allocated list.
    /// Both operands must have the same List type; the result has that
    /// same type. Element data is copied from both sources.
    ListConcat { lhs: Box<TypedExpr>, rhs: Box<TypedExpr> },
    /// Block expression: execute `stmts` for their effects, then
    /// evaluate `result`. Used by list comprehensions to inline a
    /// while-loop accumulator pattern at expression position.
    /// Locals introduced by the inner stmts are collected by codegen
    /// and allocated up-front in the function entry block.
    DoBlock { stmts: Vec<Stmt>, result: Box<TypedExpr> },
    /// String literal. The codegen emits a private constant `[N x i8]`
    /// global and returns a `{ i64 N-without-NUL, i8* &.str.x[0] }` value.
    StrLit(String),
    /// String concatenation. Allocates a new buffer of total length
    /// and memcpy's both sources.
    StrConcat { lhs: Box<TypedExpr>, rhs: Box<TypedExpr> },
    /// String length (the i64 stored in the str struct's first field).
    StrLen { s: Box<TypedExpr> },
    /// String equality / inequality. Result is Bool.
    StrEq { lhs: Box<TypedExpr>, rhs: Box<TypedExpr>, negated: bool },
    /// Format an integer/bool value as a Str. Used in f-string lowering.
    /// `inner.ty` is one of I8/I16/I32/I64/Bool. Heap-allocates the buffer.
    /// (Str-typed interpolations are passed through directly; this variant
    /// is never emitted for Str inputs.)
    FormatToStr { inner: Box<TypedExpr> },
    /// `s[i]` — single-char substring. Result is a fresh 1-byte heap-allocated str.
    /// No bounds check; index must be non-negative (check rejects literal
    /// negatives at compile time).
    StrIndex { s: Box<TypedExpr>, index: Box<TypedExpr> },
    /// `s[start:stop]` — substring. Bounds are clamped to [0, len(s)] at
    /// runtime. Result is a fresh heap-allocated copy. `start` and `stop`
    /// are both I64; defaults (0 and StrLen) are substituted by check.
    StrSlice { s: Box<TypedExpr>, start: Box<TypedExpr>, stop: Box<TypedExpr> },
    /// Call to a math runtime / LLVM intrinsic. The `name` is the
    /// LLVM symbol to invoke (e.g. `llvm.sqrt.f64`, `tan`). Single-arg
    /// f64 → f64 only in v0.24.
    MathCall { intrinsic: &'static str, arg: Box<TypedExpr> },
    /// Construct a dict from N (key, value) pairs.
    DictLit { entries: Vec<(TypedExpr, TypedExpr)> },
    /// `d[k]` — runtime hash lookup. Returns the value or, on miss,
    /// returns the value-type's zero (Python raises KeyError; we
    /// don't have exceptions yet — documented divergence).
    DictGet { dict: Box<TypedExpr>, key: Box<TypedExpr> },
    /// `k in d` — returns Bool.
    DictHas { dict: Box<TypedExpr>, key: Box<TypedExpr> },
    /// `len(d)` — returns I64.
    DictLen { dict: Box<TypedExpr> },
    /// Construct a set from N keys. Reuses the dict[i64, i64] runtime;
    /// values stored as 0.
    SetLit { elements: Vec<TypedExpr> },
    /// `k in s` — Bool. Reuses pyx86_dict_i64_has.
    SetHas { set: Box<TypedExpr>, key: Box<TypedExpr> },
    /// `len(s)` — I64. Reads the same size field as DictLen.
    SetLen { set: Box<TypedExpr> },
    /// Read a field of a class instance: `obj.field`. The field index
    /// is resolved by check at lower time.
    FieldGet { obj: Box<TypedExpr>, field_index: usize },
    /// Construct a class instance: `Foo(args...)`. Allocates the
    /// struct on the heap and calls `__init__(self, args...)` which
    /// is the regular top-level function with mangled name
    /// `Foo.__init__`.
    /// `class` is the outer instantiated class (determines allocation
    /// size + result type). `init_class` is the class that owns the
    /// `__init__` being called (may be `class` or a parent, walking the
    /// chain). `None` means no `__init__` exists in the chain — codegen
    /// allocates and returns without calling any init function (Python's
    /// implicit `object.__init__`; only valid with zero args). With
    /// single inheritance the layout is prefix-compatible so codegen
    /// bitcasts the `self_ptr` to `init_class` for the call.
    ClassNew { class: ClassId, init_class: Option<ClassId>, args: Vec<TypedExpr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// Integer floor-division `//`. Operates on I64 only.
    FloorDiv,
    /// Integer floor-mod `%`. Operates on I64 only (for now; Python
    /// also defines float `%` but we don't yet need it).
    Mod,
    /// True division `/`. Always produces F64 even on int operands.
    TrueDiv,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    /// `a ** b`. Int**Int via runtime helper; float**float via libm pow.
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    /// Bitwise not (`~x`). I64 only; LLVM `xor i64 %x, -1`.
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    And,
    Or,
}
