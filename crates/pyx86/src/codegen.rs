use std::fmt::Write as _;

use crate::hir::{BinOp, Expr, Function, Program, UnaryOp};

/// Emit LLVM IR text for a v0.3 HIR program.
///
/// `py_main` carries the user's function signature (i64 params,
/// i64 return). A C `main(argc, argv)` wrapper parses each argv
/// string via `atoll`, calls `py_main`, prints the return value
/// via `printf("%ld\n", ...)`, and returns 0.
///
/// Typed pointers (`i8*`) so this works on LLVM 10+. Opaque
/// pointers (`ptr`) are LLVM 14+; we don't require them.
pub fn emit_ll(prog: &Program, source_basename: &str) -> String {
    let basename = sanitize_module_id(source_basename);
    let func = &prog.main;

    let mut cg = Codegen::new();
    let result = cg.lower(&func.body);
    let body = cg.body;

    let signature = format_signature(func);
    let py_main_call_args = func
        .params
        .iter()
        .map(|p| format!("i64 %p_{}", p.name))
        .collect::<Vec<_>>()
        .join(", ");
    let parse_args_block = format_argv_parsing(func);

    format!(
        "; ModuleID = 'pyx86_{name}'
target triple = \"x86_64-unknown-linux-gnu\"

declare i32 @printf(i8*, ...)
declare i64 @atoll(i8*)

@.fmt_i64 = private unnamed_addr constant [5 x i8] c\"%ld\\0A\\00\"

define i64 @py_main({sig}) {{
entry:
{body}  ret i64 {result}
}}

define i32 @main(i32 %argc, i8** %argv) {{
entry:
{parse}  %r = call i64 @py_main({call_args})
  %fmt = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0
  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)
  ret i32 0
}}
",
        name = basename,
        sig = signature,
        body = body,
        result = result,
        parse = parse_args_block,
        call_args = py_main_call_args,
    )
}

fn format_signature(func: &Function) -> String {
    func.params
        .iter()
        .map(|p| format!("i64 %p_{}", p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate the wrapper instructions that pull each parameter out of
/// argv (1-indexed: argv[0] is the program name) and atoll it.
fn format_argv_parsing(func: &Function) -> String {
    let mut s = String::new();
    for (i, p) in func.params.iter().enumerate() {
        let argv_index = (i + 1) as i64;
        let _ = writeln!(
            s,
            "  %slot{i} = getelementptr inbounds i8*, i8** %argv, i64 {idx}",
            i = i,
            idx = argv_index
        );
        let _ = writeln!(s, "  %str{i} = load i8*, i8** %slot{i}", i = i);
        let _ = writeln!(
            s,
            "  %p_{name} = call i64 @atoll(i8* %str{i})",
            name = p.name,
            i = i
        );
    }
    s
}

/// Builds a single `py_main` body. Each `lower(...)` call returns the
/// LLVM operand (literal like `42` or SSA name like `%v3`) holding
/// the expression's value, after appending instructions to `self.body`.
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
        let _ = writeln!(self.body, "  {}", line);
    }

    fn lower(&mut self, e: &Expr) -> String {
        match e {
            Expr::ConstI64(v) => v.to_string(),
            Expr::Param(name) => format!("%p_{}", name),
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

    /// See specs/codegen-llvm.md "Floor-div correction".
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
        self.emit(&format!("{} = sext i1 {} to i64", adj, needs));
        self.emit(&format!("{} = add i64 {}, {}", dst, q, adj));
        dst
    }

    /// See specs/codegen-llvm.md "Floor-mod correction".
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
    use crate::hir::{BinOp, Expr, Function, Param, Program, Type, UnaryOp};

    fn make_program(params: Vec<&str>, body: Expr) -> Program {
        Program {
            main: Function {
                name: "main".into(),
                params: params
                    .into_iter()
                    .map(|n| Param { name: n.into(), ty: Type::I64 })
                    .collect(),
                return_ty: Type::I64,
                body,
            },
        }
    }

    fn ll_for(prog: Program) -> String {
        emit_ll(&prog, "test")
    }

    #[test]
    fn no_param_const_returns_literal() {
        let ll = ll_for(make_program(vec![], Expr::ConstI64(42)));
        assert!(ll.contains("define i64 @py_main()"));
        assert!(ll.contains("ret i64 42"));
        assert!(ll.contains("call i64 @py_main()"));
    }

    #[test]
    fn one_param_identity_passes_argv0_through() {
        let ll = ll_for(make_program(vec!["x"], Expr::Param("x".into())));
        assert!(ll.contains("define i64 @py_main(i64 %p_x)"));
        // Wrapper parses argv[1] into %p_x via atoll.
        assert!(ll.contains("getelementptr inbounds i8*, i8** %argv, i64 1"));
        assert!(ll.contains("%p_x = call i64 @atoll"));
        assert!(ll.contains("call i64 @py_main(i64 %p_x)"));
        // Body returns the param directly (no extra instructions).
        assert!(ll.contains("ret i64 %p_x"));
    }

    #[test]
    fn two_param_add_uses_correct_argv_indices() {
        let ll = ll_for(make_program(
            vec!["a", "b"],
            Expr::BinOp {
                op: BinOp::Add,
                lhs: Box::new(Expr::Param("a".into())),
                rhs: Box::new(Expr::Param("b".into())),
            },
        ));
        assert!(ll.contains("define i64 @py_main(i64 %p_a, i64 %p_b)"));
        assert!(ll.contains("getelementptr inbounds i8*, i8** %argv, i64 1"));
        assert!(ll.contains("getelementptr inbounds i8*, i8** %argv, i64 2"));
        assert!(ll.contains("call i64 @py_main(i64 %p_a, i64 %p_b)"));
        assert!(ll.contains("add i64 %p_a, %p_b"));
    }

    #[test]
    fn unary_neg_emits_zero_minus() {
        let ll = ll_for(make_program(
            vec![],
            Expr::UnaryOp { op: UnaryOp::Neg, operand: Box::new(Expr::ConstI64(5)) },
        ));
        assert!(ll.contains("sub i64 0, 5"));
    }

    #[test]
    fn floor_div_emits_correction_block() {
        let ll = ll_for(make_program(
            vec![],
            Expr::BinOp {
                op: BinOp::FloorDiv,
                lhs: Box::new(Expr::ConstI64(-7)),
                rhs: Box::new(Expr::ConstI64(2)),
            },
        ));
        assert!(ll.contains("sdiv i64"));
        assert!(ll.contains("srem i64"));
        assert!(ll.contains("xor i64"));
        assert!(ll.contains("sext i1"));
    }

    #[test]
    fn module_id_is_sanitized() {
        let ll = emit_ll(
            &make_program(vec![], Expr::ConstI64(0)),
            "weird-name.with.dots",
        );
        assert!(ll.contains("ModuleID = 'pyx86_weird_name_with_dots'"));
    }
}
