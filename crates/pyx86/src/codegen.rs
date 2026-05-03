use std::collections::HashSet;
use std::fmt::Write as _;

use crate::hir::{BinOp, BoolOp, CmpOp, Expr, Function, Program, Stmt, UnaryOp};

/// Emit LLVM IR text for a v0.5 HIR program (`if`/`elif`/`else`,
/// comparisons, early `return`, `not`, truthy int conditions).
///
/// Locals (and parameters, for uniformity) are allocated as i64
/// stack slots in the function entry block. Reads emit `load`,
/// writes emit `store`. LLVM's `mem2reg` (active at -O1+) collapses
/// these back to SSA + phi.
///
/// Typed pointers (`i8*`, `i64*`) so this works on LLVM 10+.
pub fn emit_ll(prog: &Program, source_basename: &str) -> String {
    let basename = sanitize_module_id(source_basename);
    let func = &prog.main;

    let mut cg = Codegen::new();
    cg.lower_function(func);
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
    next_block_id: usize,
    /// Whether the current basic block has been terminated (by `ret`
    /// or `br`). When true, no further instructions are emitted into
    /// the current block until a new label is opened.
    block_terminated: bool,
    /// Stack of `(continue_target, break_target)` pairs, one per
    /// enclosing while loop. The top of the stack is the innermost
    /// loop; `Break` / `Continue` always jump to the top entry.
    loop_targets: Vec<(String, String)>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            body: String::new(),
            next_id: 0,
            next_block_id: 0,
            block_terminated: false,
            loop_targets: Vec::new(),
        }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("%v{}", n)
    }

    fn emit(&mut self, line: &str) {
        if self.block_terminated {
            return;
        }
        let _ = writeln!(self.body, "  {}", line);
    }

    fn open_block(&mut self, label: &str) {
        let _ = writeln!(self.body, "{}:", label);
        self.block_terminated = false;
    }

    fn lower_function(&mut self, func: &Function) {
        self.open_block("entry");

        // Allocate slots for params + locals. A param and local can
        // share a name (Python scope rules); we dedupe by name so we
        // only emit one alloca per slot.
        let local_names = collect_locals(&func.body);
        let mut emitted: HashSet<String> = HashSet::new();
        for p in &func.params {
            self.emit(&format!("%{}.addr = alloca i64", p.name));
            self.emit(&format!("store i64 %p_{name}, i64* %{name}.addr", name = p.name));
            emitted.insert(p.name.clone());
        }
        for name in &local_names {
            if emitted.insert(name.clone()) {
                self.emit(&format!("%{}.addr = alloca i64", name));
            }
        }

        self.lower_block(&func.body);

        // If the user's body falls through without a terminator
        // (shouldn't happen — the check pass enforces every path
        // returns — but be defensive), emit `unreachable`.
        if !self.block_terminated {
            self.emit("unreachable");
            self.block_terminated = true;
        }
    }

    fn lower_block(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if self.block_terminated {
                // Unreachable code follows a `return` or `br`. Skip.
                break;
            }
            match stmt {
                Stmt::Let { name, value } => {
                    let op = self.lower(value);
                    self.emit(&format!("store i64 {}, i64* %{}.addr", op, name));
                }
                Stmt::Return { value } => {
                    let op = self.lower(value);
                    self.emit(&format!("ret i64 {}", op));
                    self.block_terminated = true;
                }
                Stmt::While { cond, body } => {
                    let id = self.next_block_id;
                    self.next_block_id += 1;
                    let header_lbl = format!("loop_header.{}", id);
                    let body_lbl = format!("loop_body.{}", id);
                    let exit_lbl = format!("loop_exit.{}", id);

                    // Branch into the header from the current block.
                    self.emit(&format!("br label %{}", header_lbl));
                    self.block_terminated = true;

                    // Header: evaluate condition, branch to body or exit.
                    self.open_block(&header_lbl);
                    let cond_i1 = self.lower_cond(cond);
                    self.emit(&format!(
                        "br i1 {}, label %{}, label %{}",
                        cond_i1, body_lbl, exit_lbl
                    ));
                    self.block_terminated = true;

                    // Body: lower with loop targets pushed.
                    self.open_block(&body_lbl);
                    self.loop_targets
                        .push((header_lbl.clone(), exit_lbl.clone()));
                    self.lower_block(body);
                    self.loop_targets.pop();
                    if !self.block_terminated {
                        // Back-edge to header.
                        self.emit(&format!("br label %{}", header_lbl));
                        self.block_terminated = true;
                    }

                    // Exit: continue with statements after the while.
                    self.open_block(&exit_lbl);
                }
                Stmt::Break => {
                    let (_, brk) = self
                        .loop_targets
                        .last()
                        .expect("internal: Break with empty loop stack (check should have caught this)");
                    let target = brk.clone();
                    self.emit(&format!("br label %{}", target));
                    self.block_terminated = true;
                }
                Stmt::Continue => {
                    let (cnt, _) = self
                        .loop_targets
                        .last()
                        .expect("internal: Continue with empty loop stack (check should have caught this)");
                    let target = cnt.clone();
                    self.emit(&format!("br label %{}", target));
                    self.block_terminated = true;
                }
                Stmt::If { cond, then_body, else_body } => {
                    let cond_i1 = self.lower_cond(cond);
                    // Single id per `if` statement so labels read as
                    // then.0/else.0/merge.0, then.1/else.1/merge.1, …
                    let id = self.next_block_id;
                    self.next_block_id += 1;
                    let then_lbl = format!("then.{}", id);
                    let else_lbl = format!("else.{}", id);
                    let merge_lbl = format!("merge.{}", id);

                    self.emit(&format!(
                        "br i1 {}, label %{}, label %{}",
                        cond_i1, then_lbl, else_lbl
                    ));
                    self.block_terminated = true;

                    // then-block
                    self.open_block(&then_lbl);
                    self.lower_block(then_body);
                    if !self.block_terminated {
                        self.emit(&format!("br label %{}", merge_lbl));
                        self.block_terminated = true;
                    }

                    // else-block (empty Vec when no else clause)
                    self.open_block(&else_lbl);
                    self.lower_block(else_body);
                    if !self.block_terminated {
                        self.emit(&format!("br label %{}", merge_lbl));
                        self.block_terminated = true;
                    }

                    // merge-block: where post-if statements continue.
                    // If both branches terminated (e.g. both returned),
                    // the merge block is dead — but LLVM tolerates it,
                    // and any subsequent stmt in the surrounding block
                    // emits cleanly into it. If no further statements
                    // follow, lower_function emits `unreachable` here.
                    self.open_block(&merge_lbl);
                }
            }
        }
    }

    /// Lower an expression in a value (i64) context. Comparison /
    /// not results are zext'd to i64 so they can be stored in i64
    /// slots and returned uniformly.
    fn lower(&mut self, e: &Expr) -> String {
        match e {
            Expr::ConstI64(v) => v.to_string(),
            Expr::Var(name) => {
                let dst = self.fresh();
                self.emit(&format!("{} = load i64, i64* %{}.addr", dst, name));
                dst
            }
            Expr::UnaryOp { op, operand } => {
                let inner = self.lower(operand);
                match op {
                    UnaryOp::Pos => inner,
                    UnaryOp::Neg => {
                        let dst = self.fresh();
                        self.emit(&format!("{} = sub i64 0, {}", dst, inner));
                        dst
                    }
                    UnaryOp::BitNot => {
                        // Python `~x` == `-x - 1` == `xor x, -1`.
                        let dst = self.fresh();
                        self.emit(&format!("{} = xor i64 {}, -1", dst, inner));
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
                    BinOp::BitAnd => self.simple_binop("and", &l, &r),
                    BinOp::BitOr => self.simple_binop("or", &l, &r),
                    BinOp::BitXor => self.simple_binop("xor", &l, &r),
                    // Python's `>>` is arithmetic (sign-extending) for ints; LLVM `ashr`.
                    // Python's `<<` is `shl`. Note that Python raises ValueError on
                    // negative or oversized shift counts; LLVM is undefined-behaviour
                    // for shift count >= bit width. Test programs stay within [0, 63].
                    BinOp::Shl => self.simple_binop("shl", &l, &r),
                    BinOp::Shr => self.simple_binop("ashr", &l, &r),
                }
            }
            Expr::Cmp { .. } | Expr::CmpChain { .. } | Expr::Not(_) => {
                let i1 = self.lower_i1(e);
                let dst = self.fresh();
                self.emit(&format!("{} = zext i1 {} to i64", dst, i1));
                dst
            }
            Expr::BoolOp { op, lhs, rhs } => self.lower_bool_op_value(*op, lhs, rhs),
        }
    }

    /// Lower `a and b` / `a or b` in value context, with proper
    /// short-circuit value semantics (returns the actual operand
    /// value, not just a 0/1 — Python's `5 and 7 == 7`).
    ///
    /// Strategy: evaluate `lhs` into a temp slot. If it short-circuits
    /// (and: lhs falsy; or: lhs truthy) we keep it; otherwise we
    /// overwrite the slot with `rhs`. We use a per-call fresh stack
    /// slot so nested BoolOps don't collide.
    fn lower_bool_op_value(&mut self, op: BoolOp, lhs: &Expr, rhs: &Expr) -> String {
        // Allocate a temp slot. To avoid generating allocas late in
        // the function (which mem2reg handles fine but is unusual),
        // we just emit it inline; LLVM will hoist allocas to entry
        // and mem2reg will collapse them.
        let slot_id = self.next_id;
        self.next_id += 1;
        let slot = format!("%bool.{}.addr", slot_id);
        self.emit(&format!("{} = alloca i64", slot));

        let lhs_op = self.lower(lhs);
        self.emit(&format!("store i64 {}, i64* {}", lhs_op, slot));

        let cond = self.fresh();
        self.emit(&format!("{} = icmp ne i64 {}, 0", cond, lhs_op));

        let id = self.next_block_id;
        self.next_block_id += 1;
        let eval_rhs_lbl = format!("bool.eval_rhs.{}", id);
        let merge_lbl = format!("bool.merge.{}", id);

        // For AND: short-circuit when lhs is FALSY (skip rhs); evaluate rhs only when TRUTHY.
        // For OR:  short-circuit when lhs is TRUTHY (skip rhs); evaluate rhs only when FALSY.
        let (truthy_label, falsy_label) = match op {
            BoolOp::And => (eval_rhs_lbl.as_str(), merge_lbl.as_str()),
            BoolOp::Or => (merge_lbl.as_str(), eval_rhs_lbl.as_str()),
        };
        self.emit(&format!(
            "br i1 {}, label %{}, label %{}",
            cond, truthy_label, falsy_label
        ));
        self.block_terminated = true;

        self.open_block(&eval_rhs_lbl);
        let rhs_op = self.lower(rhs);
        self.emit(&format!("store i64 {}, i64* {}", rhs_op, slot));
        self.emit(&format!("br label %{}", merge_lbl));
        self.block_terminated = true;

        self.open_block(&merge_lbl);
        let dst = self.fresh();
        self.emit(&format!("{} = load i64, i64* {}", dst, slot));
        dst
    }

    /// Lower an expression in a condition (i1) context. Comparisons /
    /// not return i1 directly; everything else gets a `!= 0` coercion.
    /// BoolOps fall through to the value-context path then `!= 0`,
    /// which composes the short-circuit branches with the outer
    /// truthiness check; LLVM cleans up.
    fn lower_cond(&mut self, e: &Expr) -> String {
        match e {
            Expr::Cmp { .. } | Expr::CmpChain { .. } | Expr::Not(_) => self.lower_i1(e),
            _ => {
                let v = self.lower(e);
                let dst = self.fresh();
                self.emit(&format!("{} = icmp ne i64 {}, 0", dst, v));
                dst
            }
        }
    }

    /// Direct i1 lowering for Cmp / CmpChain / Not. Caller must know
    /// the expression yields i1.
    fn lower_i1(&mut self, e: &Expr) -> String {
        match e {
            Expr::Cmp { op, lhs, rhs } => {
                let l = self.lower(lhs);
                let r = self.lower(rhs);
                let dst = self.fresh();
                self.emit(&format!("{} = icmp {} i64 {}, {}", dst, llvm_icmp_op(*op), l, r));
                dst
            }
            Expr::CmpChain { first, rest } => {
                // Lower each operand once per comparison. Side-effect
                // free in v0.5 so duplicate evaluation is harmless;
                // LLVM CSEs identical loads.
                let mut prev = self.lower(first);
                let mut acc: Option<String> = None;
                for (op, e) in rest {
                    let next = self.lower(e);
                    let cmp = self.fresh();
                    self.emit(&format!(
                        "{} = icmp {} i64 {}, {}",
                        cmp,
                        llvm_icmp_op(*op),
                        prev,
                        next
                    ));
                    acc = match acc {
                        None => Some(cmp),
                        Some(a) => {
                            let combined = self.fresh();
                            self.emit(&format!("{} = and i1 {}, {}", combined, a, cmp));
                            Some(combined)
                        }
                    };
                    prev = next;
                }
                acc.expect("CmpChain.rest must be non-empty")
            }
            Expr::Not(inner) => {
                let i1 = self.lower_cond(inner);
                let dst = self.fresh();
                self.emit(&format!("{} = xor i1 {}, true", dst, i1));
                dst
            }
            _ => unreachable!("lower_i1 called on a non-bool-producing expression"),
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

fn llvm_icmp_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Lt => "slt",
        CmpOp::Le => "sle",
        CmpOp::Gt => "sgt",
        CmpOp::Ge => "sge",
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
    }
}

fn collect_locals(body: &[Stmt]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    fn walk(stmts: &[Stmt], out: &mut Vec<String>, seen: &mut HashSet<String>) {
        for s in stmts {
            match s {
                Stmt::Let { name, .. } => {
                    if seen.insert(name.clone()) {
                        out.push(name.clone());
                    }
                }
                Stmt::Return { .. } | Stmt::Break | Stmt::Continue => {}
                Stmt::If { then_body, else_body, .. } => {
                    walk(then_body, out, seen);
                    walk(else_body, out, seen);
                }
                Stmt::While { body, .. } => walk(body, out, seen),
            }
        }
    }
    walk(body, &mut out, &mut seen);
    out
}

fn sanitize_module_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{BinOp, CmpOp, Expr, Function, Param, Program, Stmt, Type};

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
    fn allocates_param_and_local_slots() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![
                    Stmt::Let { name: "x".into(), value: Expr::ConstI64(0) },
                    Stmt::Return { value: Expr::Var("x".into()) },
                ],
            ),
            "test",
        );
        assert!(ll.contains("%a.addr = alloca i64"));
        assert!(ll.contains("store i64 %p_a, i64* %a.addr"));
        assert!(ll.contains("%x.addr = alloca i64"));
        assert!(ll.contains("store i64 0, i64* %x.addr"));
        assert!(ll.contains("load i64, i64* %x.addr"));
    }

    #[test]
    fn if_else_emits_branch_and_two_blocks() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![Stmt::If {
                    cond: Expr::Cmp {
                        op: CmpOp::Lt,
                        lhs: Box::new(Expr::Var("a".into())),
                        rhs: Box::new(Expr::ConstI64(0)),
                    },
                    then_body: vec![Stmt::Return {
                        value: Expr::UnaryOp {
                            op: crate::hir::UnaryOp::Neg,
                            operand: Box::new(Expr::Var("a".into())),
                        },
                    }],
                    else_body: vec![Stmt::Return { value: Expr::Var("a".into()) }],
                }],
            ),
            "test",
        );
        assert!(ll.contains("icmp slt i64"));
        assert!(ll.contains("br i1"));
        assert!(ll.contains("then.0:"));
        assert!(ll.contains("else.0:"));
        assert!(ll.contains("merge.0:"));
    }

    #[test]
    fn truthy_int_condition_inserts_icmp_ne_zero() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![Stmt::If {
                    cond: Expr::Var("a".into()),
                    then_body: vec![Stmt::Return { value: Expr::ConstI64(1) }],
                    else_body: vec![Stmt::Return { value: Expr::ConstI64(0) }],
                }],
            ),
            "test",
        );
        assert!(ll.contains("icmp ne i64"));
    }

    #[test]
    fn cmp_chain_ands_pairwise_results() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![Stmt::If {
                    cond: Expr::CmpChain {
                        first: Box::new(Expr::ConstI64(0)),
                        rest: vec![
                            (CmpOp::Lt, Expr::Var("a".into())),
                            (CmpOp::Lt, Expr::ConstI64(100)),
                        ],
                    },
                    then_body: vec![Stmt::Return { value: Expr::ConstI64(1) }],
                    else_body: vec![Stmt::Return { value: Expr::ConstI64(0) }],
                }],
            ),
            "test",
        );
        // Two icmp slt + one and i1.
        let icmps = ll.matches("icmp slt").count();
        assert!(icmps >= 2, "expected ≥2 icmp slt, got: \n{}", ll);
        assert!(ll.contains("and i1"));
    }

    #[test]
    fn not_int_emits_eq_zero() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![Stmt::If {
                    cond: Expr::Not(Box::new(Expr::Var("a".into()))),
                    then_body: vec![Stmt::Return { value: Expr::ConstI64(1) }],
                    else_body: vec![Stmt::Return { value: Expr::ConstI64(0) }],
                }],
            ),
            "test",
        );
        // `not a` (i64) lowers to icmp eq i64 a, 0 ... wait actually
        // we lower as `cond = icmp ne 0; not(cond) = xor cond, true`.
        // Either form works; the bench is the source of truth.
        assert!(ll.contains("icmp ne i64") || ll.contains("icmp eq i64"));
        assert!(ll.contains("xor i1") || ll.contains("icmp eq i64"));
    }

    #[test]
    fn while_emits_header_body_exit_with_back_edge() {
        let ll = emit_ll(
            &make_program(
                vec!["n"],
                vec![
                    Stmt::Let { name: "i".into(), value: Expr::ConstI64(0) },
                    Stmt::While {
                        cond: Expr::Cmp {
                            op: CmpOp::Lt,
                            lhs: Box::new(Expr::Var("i".into())),
                            rhs: Box::new(Expr::Var("n".into())),
                        },
                        body: vec![Stmt::Let {
                            name: "i".into(),
                            value: Expr::BinOp {
                                op: BinOp::Add,
                                lhs: Box::new(Expr::Var("i".into())),
                                rhs: Box::new(Expr::ConstI64(1)),
                            },
                        }],
                    },
                    Stmt::Return { value: Expr::Var("i".into()) },
                ],
            ),
            "test",
        );
        assert!(ll.contains("loop_header.0:"));
        assert!(ll.contains("loop_body.0:"));
        assert!(ll.contains("loop_exit.0:"));
        // Both edges into header: initial entry + back-edge from body.
        assert!(ll.matches("br label %loop_header.0").count() >= 2);
    }

    #[test]
    fn break_jumps_to_loop_exit() {
        let ll = emit_ll(
            &make_program(
                vec!["n"],
                vec![
                    Stmt::While {
                        cond: Expr::ConstI64(1),
                        body: vec![Stmt::Break],
                    },
                    Stmt::Return { value: Expr::ConstI64(0) },
                ],
            ),
            "test",
        );
        assert!(ll.contains("br label %loop_exit.0"));
    }

    #[test]
    fn continue_jumps_to_loop_header() {
        let ll = emit_ll(
            &make_program(
                vec!["n"],
                vec![
                    Stmt::Let { name: "i".into(), value: Expr::ConstI64(0) },
                    Stmt::While {
                        cond: Expr::Cmp {
                            op: CmpOp::Lt,
                            lhs: Box::new(Expr::Var("i".into())),
                            rhs: Box::new(Expr::Var("n".into())),
                        },
                        body: vec![
                            Stmt::Let {
                                name: "i".into(),
                                value: Expr::BinOp {
                                    op: BinOp::Add,
                                    lhs: Box::new(Expr::Var("i".into())),
                                    rhs: Box::new(Expr::ConstI64(1)),
                                },
                            },
                            Stmt::Continue,
                        ],
                    },
                    Stmt::Return { value: Expr::Var("i".into()) },
                ],
            ),
            "test",
        );
        // 2 explicit `br label %loop_header.0` (entry + continue),
        // plus none from the body (continue terminates the block).
        let count = ll.matches("br label %loop_header.0").count();
        assert!(count >= 2, "expected ≥2 br to header, got {} in:\n{}", count, ll);
    }

    #[test]
    fn early_return_in_then_skips_merge_emission() {
        // If both branches return, the merge block is emitted but
        // contains only the implicit `unreachable` from the function
        // wrapper.
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![Stmt::If {
                    cond: Expr::Cmp {
                        op: CmpOp::Lt,
                        lhs: Box::new(Expr::Var("a".into())),
                        rhs: Box::new(Expr::ConstI64(0)),
                    },
                    then_body: vec![Stmt::Return { value: Expr::ConstI64(-1) }],
                    else_body: vec![Stmt::Return { value: Expr::ConstI64(1) }],
                }],
            ),
            "test",
        );
        assert!(ll.contains("merge.0:"));
        assert!(ll.contains("unreachable"));
    }
}
