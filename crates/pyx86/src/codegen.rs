use std::fmt::Write as _;

use crate::hir::{BinOp, Expr, Program, UnaryOp};

/// Emit LLVM IR text for a v0.2 HIR program. The user's `main()` is
/// renamed to `py_main`; a C `main` wrapper calls it, prints the
/// return value via `printf("%ld\n", ...)`, and returns 0.
///
/// We use typed pointers (`i8*`) so the output works on LLVM 10+;
/// opaque pointers (`ptr`) are LLVM 14+.
pub fn emit_ll(prog: &Program, source_basename: &str) -> String {
    let mut cg = Codegen::new();
    let result = cg.lower(&prog.main_return);
    let basename = sanitize_module_id(source_basename);
    format!(
        "; ModuleID = 'pyx86_{name}'
target triple = \"x86_64-unknown-linux-gnu\"

declare i32 @printf(i8*, ...)

@.fmt_i64 = private unnamed_addr constant [5 x i8] c\"%ld\\0A\\00\"

define i64 @py_main() {{
entry:
{body}  ret i64 {result}
}}

define i32 @main() {{
entry:
  %r = call i64 @py_main()
  %fmt = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0
  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)
  ret i32 0
}}
",
        name = basename,
        body = cg.body,
        result = result,
    )
}

/// Builds a single `py_main` body. Each `lower(...)` call returns the
/// LLVM operand (either a literal like `42` or an SSA name like `%v3`)
/// holding the expression's value, after appending any necessary
/// instructions to `self.body`.
struct Codegen {
    body: String,
    next_id: usize,
}

impl Codegen {
    fn new() -> Self {
        Self { body: String::new(), next_id: 0 }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("%v{}", n)
    }

    fn emit(&mut self, line: &str) {
        // All instruction lines are 2-space indented.
        let _ = writeln!(self.body, "  {}", line);
    }

    fn lower(&mut self, e: &Expr) -> String {
        match e {
            Expr::ConstI64(v) => v.to_string(),
            Expr::UnaryOp { op, operand } => {
                let inner = self.lower(operand);
                match op {
                    UnaryOp::Pos => inner,
                    UnaryOp::Neg => {
                        let dst = self.fresh();
                        self.emit(&format!("{} = sub i64 0, {}", dst, inner));
                        dst
                    }
                }
            }
            Expr::BinOp { op, lhs, rhs } => {
                let l = self.lower(lhs);
                let r = self.lower(rhs);
                match op {
                    BinOp::Add => self.simple_binop("add", &l, &r),
                    BinOp::Sub => self.simple_binop("sub", &l, &r),
                    BinOp::Mul => self.simple_binop("mul", &l, &r),
                    BinOp::FloorDiv => self.floor_div(&l, &r),
                    BinOp::Mod => self.floor_mod(&l, &r),
                }
            }
        }
    }

    fn simple_binop(&mut self, op: &str, l: &str, r: &str) -> String {
        let dst = self.fresh();
        self.emit(&format!("{} = {} i64 {}, {}", dst, op, l, r));
        dst
    }

    /// Python `a // b` differs from LLVM `sdiv` for negative operands:
    /// Python rounds toward -infinity, LLVM truncates toward 0.
    /// Correction: `result = sdiv(a, b) - ((srem(a, b) != 0) & (signs(a, b) differ) ? 1 : 0)`.
    /// We compute the adjustment as `sext i1 → i64` (true sign-extends to -1)
    /// and then `sdiv + adj` instead of `sdiv - adj` to save an instruction.
    fn floor_div(&mut self, l: &str, r: &str) -> String {
        let q = self.fresh();
        let rem = self.fresh();
        let rem_nz = self.fresh();
        let xor_sign = self.fresh();
        let signs_differ = self.fresh();
        let needs = self.fresh();
        let adj = self.fresh();
        let dst = self.fresh();
        self.emit(&format!("{} = sdiv i64 {}, {}", q, l, r));
        self.emit(&format!("{} = srem i64 {}, {}", rem, l, r));
        self.emit(&format!("{} = icmp ne i64 {}, 0", rem_nz, rem));
        self.emit(&format!("{} = xor i64 {}, {}", xor_sign, l, r));
        self.emit(&format!("{} = icmp slt i64 {}, 0", signs_differ, xor_sign));
        self.emit(&format!("{} = and i1 {}, {}", needs, rem_nz, signs_differ));
        // sext i1 → i64: true → -1, false → 0; then q + adj == q - 1 (or q).
        self.emit(&format!("{} = sext i1 {} to i64", adj, needs));
        self.emit(&format!("{} = add i64 {}, {}", dst, q, adj));
        dst
    }

    /// Python `a % b` differs from LLVM `srem` for mixed-sign operands:
    /// Python's result has the same sign as the divisor; `srem` has the
    /// sign of the dividend.
    /// Correction: `r = srem(a, b); if r != 0 and signs(r, b) differ: r += b`.
    fn floor_mod(&mut self, l: &str, r: &str) -> String {
        let rem = self.fresh();
        let rem_nz = self.fresh();
        let xor_sign = self.fresh();
        let signs_differ = self.fresh();
        let needs = self.fresh();
        let adj = self.fresh();
        let dst = self.fresh();
        self.emit(&format!("{} = srem i64 {}, {}", rem, l, r));
        self.emit(&format!("{} = icmp ne i64 {}, 0", rem_nz, rem));
        self.emit(&format!("{} = xor i64 {}, {}", xor_sign, rem, r));
        self.emit(&format!("{} = icmp slt i64 {}, 0", signs_differ, xor_sign));
        self.emit(&format!("{} = and i1 {}, {}", needs, rem_nz, signs_differ));
        self.emit(&format!("{} = select i1 {}, i64 {}, i64 0", adj, needs, r));
        self.emit(&format!("{} = add i64 {}, {}", dst, rem, adj));
        dst
    }
}

fn sanitize_module_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{BinOp, Expr, Program, UnaryOp};

    fn ll_for(e: Expr) -> String {
        emit_ll(&Program { main_return: e }, "test")
    }

    #[test]
    fn const_i64_returns_literal() {
        let ll = ll_for(Expr::ConstI64(42));
        assert!(ll.contains("ret i64 42"));
        assert!(ll.contains("define i32 @main()"));
        assert!(ll.contains("call i32 (i8*, ...) @printf"));
    }

    #[test]
    fn unary_neg_emits_zero_minus() {
        let ll = ll_for(Expr::UnaryOp {
            op: UnaryOp::Neg,
            operand: Box::new(Expr::ConstI64(5)),
        });
        assert!(ll.contains("sub i64 0, 5"));
    }

    #[test]
    fn unary_pos_is_a_noop() {
        let ll = ll_for(Expr::UnaryOp {
            op: UnaryOp::Pos,
            operand: Box::new(Expr::ConstI64(3)),
        });
        assert!(ll.contains("ret i64 3"));
        assert!(!ll.contains("sub i64"));
    }

    #[test]
    fn add_emits_simple_binop() {
        let ll = ll_for(Expr::BinOp {
            op: BinOp::Add,
            lhs: Box::new(Expr::ConstI64(1)),
            rhs: Box::new(Expr::ConstI64(2)),
        });
        assert!(ll.contains("add i64 1, 2"));
    }

    #[test]
    fn floor_div_emits_correction_block() {
        let ll = ll_for(Expr::BinOp {
            op: BinOp::FloorDiv,
            lhs: Box::new(Expr::ConstI64(-7)),
            rhs: Box::new(Expr::ConstI64(2)),
        });
        // The correction block uses sdiv + srem + xor + sext.
        assert!(ll.contains("sdiv i64"));
        assert!(ll.contains("srem i64"));
        assert!(ll.contains("xor i64"));
        assert!(ll.contains("sext i1"));
    }

    #[test]
    fn floor_mod_emits_select_correction() {
        let ll = ll_for(Expr::BinOp {
            op: BinOp::Mod,
            lhs: Box::new(Expr::ConstI64(-7)),
            rhs: Box::new(Expr::ConstI64(2)),
        });
        assert!(ll.contains("srem i64"));
        assert!(ll.contains("select i1"));
    }

    #[test]
    fn module_id_is_sanitized() {
        let ll = emit_ll(
            &Program { main_return: Expr::ConstI64(0) },
            "weird-name.with.dots",
        );
        assert!(ll.contains("ModuleID = 'pyx86_weird_name_with_dots'"));
    }
}
