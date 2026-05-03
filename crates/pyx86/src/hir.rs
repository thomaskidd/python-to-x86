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
    /// All user-defined functions in declaration order. Exactly one
    /// is named "main" — that's the entry point invoked by the C
    /// `main(argc, argv)` wrapper. Others may be called from any
    /// function (including recursively / mutually).
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
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// Exit the innermost enclosing `While`. Check ensures it only
    /// appears inside one.
    Break,
    /// Jump to the next iteration of the innermost enclosing `While`.
    Continue,
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
    /// Python `and` / `or` with short-circuit value semantics:
    /// - `a and b` = a if a is falsy else b
    /// - `a or  b` = a if a is truthy else b
    /// Lowered as a branch on `a`'s truthiness, with `b` evaluated
    /// only on the branch where it's used. Each chains naturally:
    /// `a and b and c` parses as nested BoolOp(And, BoolOp(And, a, b), c).
    BoolOp {
        op: BoolOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Call to a user-defined function by name. `check.rs` ensures the
    /// callee exists and the argument count matches its signature.
    /// All callees return i64 in the current language subset.
    Call {
        callee: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    FloorDiv,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    /// Bitwise not (`~x`). LLVM `xor i64 %x, -1`.
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
