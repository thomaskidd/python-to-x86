//! High-level IR. Tiny: int values + parameter references + arithmetic.
//! Grows as later slices add types, locals, control flow.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    I64,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    /// Always Type::I64 in v0.3. Carried so codegen can dispatch on
    /// it once other types land.
    #[allow(dead_code)]
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Function {
    /// Always "main" in v0.3. Carried so error messages and later
    /// slices that add user-defined functions don't need to refactor.
    #[allow(dead_code)]
    pub name: String,
    pub params: Vec<Param>,
    /// Always Type::I64 in v0.3. Will gate codegen choices when
    /// other types land.
    #[allow(dead_code)]
    pub return_ty: Type,
    pub body: Expr,
}

#[derive(Debug)]
pub struct Program {
    pub main: Function,
}

#[derive(Debug, Clone)]
pub enum Expr {
    ConstI64(i64),
    /// Reference to a parameter by name.
    Param(String),
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
