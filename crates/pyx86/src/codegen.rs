use std::collections::HashMap;
use std::fmt::Write as _;

use crate::hir::{BinOp, Expr, Function, Program, Stmt, UnaryOp};

/// Emit LLVM IR text for a v0.4 HIR program.
///
/// Function body is now a sequence of statements. Locals are pure
/// SSA values (no `alloca`/`load`/`store`) — fine because v0.4 has
/// no control flow. Reassignment overwrites the entry in the
/// variable map, producing a fresh SSA name each time.
///
/// When v0.5 adds branching, locals will switch to `alloca`+
/// `load`/`store` and let LLVM's `mem2reg` collapse them back to
/// SSA at -O1+.
pub fn emit_ll(prog: &Program, source_basename: &str) -> String {
    let basename = sanitize_module_id(source_basename);
    let func = &prog.main;

    let mut cg = Codegen::new();
    // Seed the variable map with parameters so `Var(name)` lowering
    // can find them.
    for p in &func.params {
        cg.vars.insert(p.name.clone(), format!("%p_{}", p.name));
    }
    cg.lower_body(&func.body);
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
{body}}}

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

struct Codegen {
    body: String,
    next_id: usize,
    /// Maps HIR variable name → LLVM operand currently holding its value.
    /// Reassignment overwrites the entry; the old SSA name becomes
    /// dead code (LLVM DCE removes it).
    vars: HashMap<String, String>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            body: String::new(),
            next_id: 0,
            vars: HashMap::new(),
        }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("%v{}", n)
    }

    fn emit(&mut self, line: &str) {
        let _ = writeln!(self.body, "  {}", line);
    }

    fn lower_body(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { name, value } => {
                    let operand = self.lower(value);
                    self.vars.insert(name.clone(), operand);
                }
                Stmt::Return { value } => {
                    let operand = self.lower(value);
                    self.emit(&format!("ret i64 {}", operand));
                }
            }
        }
    }

    fn lower(&mut self, e: &Expr) -> String {
        match e {
            Expr::ConstI64(v) => v.to_string(),
            Expr::Var(name) => self
                .vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("internal: var `{}` not in codegen scope (check should have caught this)", name)),
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
    use crate::hir::{BinOp, Expr, Function, Param, Program, Stmt, Type};

    fn make_program(params: Vec<&str>, body: Vec<Stmt>) -> Program {
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

    #[test]
    fn no_locals_emits_a_simple_return() {
        let ll = emit_ll(
            &make_program(vec![], vec![Stmt::Return { value: Expr::ConstI64(42) }]),
            "test",
        );
        assert!(ll.contains("ret i64 42"));
    }

    #[test]
    fn local_binds_then_returns() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![
                    Stmt::Let {
                        name: "x".into(),
                        value: Expr::BinOp {
                            op: BinOp::Add,
                            lhs: Box::new(Expr::Var("a".into())),
                            rhs: Box::new(Expr::ConstI64(1)),
                        },
                    },
                    Stmt::Return { value: Expr::Var("x".into()) },
                ],
            ),
            "test",
        );
        // The Let statement should produce an `add i64 %p_a, 1` and
        // the Return should `ret i64` of that SSA value (%v0).
        assert!(ll.contains("%v0 = add i64 %p_a, 1"));
        assert!(ll.contains("ret i64 %v0"));
    }

    #[test]
    fn reassignment_overwrites_var_map() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![
                    Stmt::Let { name: "x".into(), value: Expr::Var("a".into()) },
                    Stmt::Let {
                        name: "x".into(),
                        value: Expr::BinOp {
                            op: BinOp::Add,
                            lhs: Box::new(Expr::Var("x".into())),
                            rhs: Box::new(Expr::ConstI64(1)),
                        },
                    },
                    Stmt::Return { value: Expr::Var("x".into()) },
                ],
            ),
            "test",
        );
        // First Let: x = a, no instruction emitted (Var lookup just maps x → %p_a).
        // Second Let: x = x + 1 → emits `%v0 = add i64 %p_a, 1`, then x → %v0.
        // Return: ret i64 %v0.
        assert!(ll.contains("%v0 = add i64 %p_a, 1"));
        assert!(ll.contains("ret i64 %v0"));
    }

    #[test]
    fn chain_of_locals_uses_each_predecessor() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![
                    Stmt::Let {
                        name: "x".into(),
                        value: Expr::BinOp {
                            op: BinOp::Add,
                            lhs: Box::new(Expr::Var("a".into())),
                            rhs: Box::new(Expr::ConstI64(1)),
                        },
                    },
                    Stmt::Let {
                        name: "y".into(),
                        value: Expr::BinOp {
                            op: BinOp::Mul,
                            lhs: Box::new(Expr::Var("x".into())),
                            rhs: Box::new(Expr::ConstI64(2)),
                        },
                    },
                    Stmt::Return { value: Expr::Var("y".into()) },
                ],
            ),
            "test",
        );
        assert!(ll.contains("%v0 = add i64 %p_a, 1"));
        assert!(ll.contains("%v1 = mul i64 %v0, 2"));
        assert!(ll.contains("ret i64 %v1"));
    }
}
