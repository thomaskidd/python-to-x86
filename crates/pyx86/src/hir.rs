//! High-level IR. Grows slice by slice; v0.5 adds control flow + comparisons.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    I64,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    /// Always Type::I64 in v0.5.
    #[allow(dead_code)]
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Function {
    /// Always "main" in v0.5.
    #[allow(dead_code)]
    pub name: String,
    pub params: Vec<Param>,
    /// Always Type::I64 in v0.5.
    #[allow(dead_code)]
    pub return_ty: Type,
    pub body: Vec<Stmt>,
}

#[derive(Debug)]
pub struct Program {
    pub main: Function,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        value: Expr,
    },
    Return {
        value: Expr,
    },
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        /// Empty Vec when there is no `else` clause.
        else_body: Vec<Stmt>,
    },
}

#[derive(Debug, Clone)]
pub enum Expr {
    ConstI64(i64),
    /// Reference to a parameter or previously assigned local.
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
    /// A single comparison: `a <op> b` → produces i64 0 or 1 (zext of the i1).
    Cmp {
        op: CmpOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Python-style chained comparison `a < b < c < d`. The vector
    /// holds the operators and the right-hand operand of each pair;
    /// `first` is the leftmost operand. All sub-expressions are
    /// pure (no calls, no side effects in v0.5), so codegen lowers
    /// each operand once per appearance and AND's the i1 results.
    CmpChain {
        first: Box<Expr>,
        rest: Vec<(CmpOp, Expr)>,
    },
    /// Logical `not`. Codegen treats inner as truthy-if-nonzero (for
    /// i64 operands) or as a direct i1 for nested Cmp/Not. Always
    /// produces i64 0 or 1.
    Not(Box<Expr>),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}
