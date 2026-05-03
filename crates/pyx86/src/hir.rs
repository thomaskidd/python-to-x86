//! High-level IR. Tiny in v0.2 — just int expressions; grows as
//! later slices add types, variables, control flow.

#[derive(Debug, Clone)]
pub enum Expr {
    ConstI64(i64),
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

#[derive(Debug)]
pub struct Program {
    pub main_return: Expr,
}
