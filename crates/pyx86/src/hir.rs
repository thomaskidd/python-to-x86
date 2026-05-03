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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl Type {
    pub fn name(self) -> &'static str {
        match self {
            Type::I8 => "i8",
            Type::I16 => "i16",
            Type::I32 => "i32",
            Type::I64 => "int",
            Type::F64 => "float",
            Type::Bool => "bool",
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
