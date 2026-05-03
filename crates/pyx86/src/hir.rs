//! High-level IR. Grows slice by slice; v0.4 adds locals + multi-stmt bodies.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    I64,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    /// Always Type::I64 in v0.4. Carried so codegen can dispatch on
    /// it once other types land.
    #[allow(dead_code)]
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Function {
    /// Always "main" in v0.4. Carried so error messages and later
    /// slices that add user-defined functions don't need to refactor.
    #[allow(dead_code)]
    pub name: String,
    pub params: Vec<Param>,
    /// Always Type::I64 in v0.4.
    #[allow(dead_code)]
    pub return_ty: Type,
    /// Sequence of statements ending with exactly one `Return`.
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct Program {
    pub main: Function,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `name = <expr>` (annotation, if present, was already validated to be `int`)
    Let { name: String, value: Expr },
    /// `return <expr>` — must be the last statement in the body.
    Return { value: Expr },
}

#[derive(Debug, Clone)]
pub enum Expr {
    ConstI64(i64),
    /// Reference to any name in scope — parameter or local.
    Var(String),
    BinOp {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    FloorDiv,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
}
