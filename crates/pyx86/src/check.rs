use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};
use rustpython_parser::ast;

use crate::hir::{BinOp, BoolOp, CmpOp, Expr, Function, Param, Program, Stmt, Type, UnaryOp};
use crate::parser::Module;

const MAX_PARAMS: usize = 16;

/// Function signature, used by the call-resolution pass so calls can
/// be validated before bodies are lowered (which lets us support
/// recursion and forward references).
#[derive(Debug)]
struct FunctionSig {
    params: Vec<Param>,
    /// Defaults aligned with `params`. `None` for required params,
    /// `Some(<literal expr>)` for params with default values. Python
    /// requires all defaulted params to come after all required ones;
    /// we enforce that.
    defaults: Vec<Option<Expr>>,
    return_ty: Type,
}

type SignatureTable = HashMap<String, FunctionSig>;

/// Lower the parsed module into a `hir::Program`. The module body is
/// a sequence of `def <name>(...)` blocks; one must be named `main`.
/// Functions may call each other (including recursively / mutually).
pub fn lower(module: &Module) -> Result<Program> {
    // Pass 1: collect signatures of all top-level functions so calls
    // can be validated before any body is lowered. This is what makes
    // forward references and mutual recursion work.
    let mut signatures: SignatureTable = HashMap::new();
    for stmt in &module.body {
        let func = match stmt {
            ast::Stmt::FunctionDef(f) => f,
            other => bail!(
                "unsupported_feature: only `def` is allowed at the top level, found {}",
                stmt_kind_name(other)
            ),
        };
        let sig = collect_signature(func)?;
        let name = func.name.as_str().to_string();
        if signatures.contains_key(&name) {
            bail!("unsupported_feature: duplicate top-level function `{}`", name);
        }
        signatures.insert(name, sig);
    }

    if !signatures.contains_key("main") {
        bail!("unsupported_feature: no `main` function defined at the top level");
    }

    // Pass 2: lower each function body using the signature table.
    let mut functions = Vec::with_capacity(module.body.len());
    for stmt in &module.body {
        let func = match stmt {
            ast::Stmt::FunctionDef(f) => f,
            _ => unreachable!(),
        };
        let f = lower_function(func, &signatures)?;
        functions.push(f);
    }

    Ok(Program { functions })
}

fn collect_signature(func: &ast::StmtFunctionDef) -> Result<FunctionSig> {
    if !func.args.posonlyargs.is_empty() || !func.args.kwonlyargs.is_empty() {
        bail!(
            "unsupported_feature: positional-only and keyword-only parameters are not supported (in `{}`)",
            func.name
        );
    }
    if func.args.vararg.is_some() || func.args.kwarg.is_some() {
        bail!(
            "unsupported_feature: *args / **kwargs are not supported (in `{}`)",
            func.name
        );
    }
    if !func.decorator_list.is_empty() {
        bail!(
            "unsupported_feature: decorators are not supported (in `{}`)",
            func.name
        );
    }
    if func.args.args.len() > MAX_PARAMS {
        bail!(
            "unsupported_feature: at most {} parameters supported, found {} (in `{}`)",
            MAX_PARAMS,
            func.args.args.len(),
            func.name
        );
    }

    let mut params = Vec::with_capacity(func.args.args.len());
    let mut seen = HashSet::new();
    for arg in &func.args.args {
        let name = arg.def.arg.as_str().to_string();
        if !seen.insert(name.clone()) {
            bail!(
                "unsupported_feature: duplicate parameter name `{}` (in `{}`)",
                name,
                func.name
            );
        }
        let ty = parse_type_annotation(arg.def.annotation.as_deref()).ok_or_else(|| {
            anyhow!(
                "unsupported_feature: parameter `{}` must be annotated `: int` (in `{}`)",
                name,
                func.name
            )
        })?;
        params.push(Param { name, ty });
    }

    let return_ty = match parse_type_annotation(func.returns.as_deref()) {
        Some(ty) => ty,
        None => bail!(
            "unsupported_feature: function `{}` requires a return annotation `-> int`",
            func.name
        ),
    };

    // Defaults: rustpython-ast's Arguments.defaults are listed in the
    // same order as the parameters they belong to, applied to the
    // *trailing* parameters. So for `def f(a, b, c=3, d=4)`, defaults
    // = [3, 4] and they apply to params[2] and params[3].
    let raw_defaults: Vec<&ast::Expr> = func.args.defaults().collect();
    let n = params.len();
    let n_defaulted = raw_defaults.len();
    if n_defaulted > n {
        // Should be impossible for valid Python.
        bail!(
            "internal: more defaults than parameters in `{}`",
            func.name
        );
    }
    let n_required = n - n_defaulted;
    let mut defaults: Vec<Option<Expr>> = vec![None; n];
    for (i, raw) in raw_defaults.iter().enumerate() {
        let param_idx = n_required + i;
        defaults[param_idx] = Some(lower_default(raw).map_err(|e| {
            anyhow!(
                "unsupported_feature: default for parameter `{}` (in `{}`): {}",
                params[param_idx].name,
                func.name,
                e
            )
        })?);
    }

    Ok(FunctionSig { params, defaults, return_ty })
}

/// Defaults are evaluated at function-definition time in Python. We
/// only allow constants (int / bool literals, optionally negated) so
/// there's no need for a "definition-time scope" — the expression is
/// fully reduced at compile time and inlined at the call site.
fn lower_default(e: &ast::Expr) -> Result<Expr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big.try_into().map_err(|_| {
                    anyhow!("integer literal does not fit in i64")
                })?;
                Ok(Expr::ConstI64(v))
            }
            ast::Constant::Bool(b) => Ok(Expr::ConstI64(if *b { 1 } else { 0 })),
            _ => bail!("only integer literals are allowed as defaults"),
        },
        ast::Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub | ast::UnaryOp::UAdd) => {
            let inner = lower_default(&u.operand)?;
            match (u.op, inner) {
                (ast::UnaryOp::USub, Expr::ConstI64(v)) => Ok(Expr::ConstI64(-v)),
                (ast::UnaryOp::UAdd, Expr::ConstI64(v)) => Ok(Expr::ConstI64(v)),
                _ => bail!("only integer literals are allowed as defaults"),
            }
        }
        _ => bail!("only integer literals are allowed as defaults"),
    }
}

fn lower_function(func: &ast::StmtFunctionDef, signatures: &SignatureTable) -> Result<Function> {
    let name = func.name.as_str().to_string();
    let sig = signatures.get(&name).expect("signature must have been collected in pass 1");

    // Seed the local scope with the parameter names.
    let mut scope: HashSet<String> = sig.params.iter().map(|p| p.name.clone()).collect();

    if func.body.is_empty() {
        bail!(
            "unsupported_feature: function `{}` body is empty",
            name
        );
    }

    let body = lower_block(&func.body, &mut scope, 0, signatures)?;

    if !block_always_returns(&body) {
        bail!(
            "unsupported_feature: not all paths return a value in `{}` (the function body, or both branches of every trailing `if`, must end with `return`)",
            name
        );
    }

    Ok(Function {
        name,
        params: sig.params.clone(),
        return_ty: sig.return_ty,
        body,
    })
}

/// Conservative path-coverage check. Returns true iff:
/// - the block ends with a `Return`, OR
/// - the block ends with an `If` whose then_body and else_body both
///   recursively cover, AND the else_body is non-empty.
///
/// `While`, `Break`, `Continue`, `Let` are **not** covering — a while
/// may execute zero iterations and break/continue jump rather than
/// returning.
///
/// This rejects valid programs where coverage requires reasoning
/// about expressions (e.g. `if True: return 1`) but never accepts
/// invalid ones, which is the side we want to err on.
fn block_always_returns(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Return { .. }) => true,
        Some(Stmt::If { then_body, else_body, .. }) => {
            !else_body.is_empty()
                && block_always_returns(then_body)
                && block_always_returns(else_body)
        }
        _ => false,
    }
}

fn lower_block(
    stmts: &[ast::Stmt],
    scope: &mut HashSet<String>,
    loop_depth: usize,
    signatures: &SignatureTable,
) -> Result<Vec<Stmt>> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        match stmt {
            ast::Stmt::Assign(a) => {
                let name = parse_assign_target(&a.targets)?;
                let value = lower_expr(&a.value, scope, signatures)?;
                scope.insert(name.clone());
                out.push(Stmt::Let { name, value });
            }
            ast::Stmt::AnnAssign(a) => {
                let name = match a.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: only simple-name targets are supported in assignments"
                    ),
                };
                if !a.simple {
                    bail!(
                        "unsupported_feature: parenthesised annotation targets are not supported"
                    );
                }
                if parse_type_annotation(Some(&a.annotation)).is_none() {
                    bail!(
                        "unsupported_feature: only `: int` annotations are supported on locals, on `{}`",
                        name
                    );
                }
                let value_expr = a.value.as_deref().ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: bare annotation `{}: int` (no value) is not supported",
                        name
                    )
                })?;
                let value = lower_expr(value_expr, scope, signatures)?;
                scope.insert(name.clone());
                out.push(Stmt::Let { name, value });
            }
            ast::Stmt::AugAssign(a) => {
                // `x op= e` desugars to `x = x op e`. Requires x to
                // already be in scope (CPython gives UnboundLocalError
                // otherwise — we reject at compile time).
                let name = match a.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: augmented-assignment target must be a simple name"
                    ),
                };
                if !scope.contains(&name) {
                    bail!(
                        "unsupported_feature: augmented assignment to unbound name `{}` (must already be a parameter or assigned local)",
                        name
                    );
                }
                let op = match a.op {
                    ast::Operator::Add => BinOp::Add,
                    ast::Operator::Sub => BinOp::Sub,
                    ast::Operator::Mult => BinOp::Mul,
                    ast::Operator::FloorDiv => BinOp::FloorDiv,
                    ast::Operator::Mod => BinOp::Mod,
                    ast::Operator::LShift => BinOp::Shl,
                    ast::Operator::RShift => BinOp::Shr,
                    ast::Operator::BitAnd => BinOp::BitAnd,
                    ast::Operator::BitOr => BinOp::BitOr,
                    ast::Operator::BitXor => BinOp::BitXor,
                    ast::Operator::Div => bail!(
                        "unsupported_feature: `/=` (true division) is not yet supported"
                    ),
                    ast::Operator::Pow => BinOp::Pow,
                    ast::Operator::MatMult => bail!(
                        "unsupported_feature: `@=` (matmul) is not supported"
                    ),
                };
                let rhs = lower_expr(&a.value, scope, signatures)?;
                let value = Expr::BinOp {
                    op,
                    lhs: Box::new(Expr::Var(name.clone())),
                    rhs: Box::new(rhs),
                };
                out.push(Stmt::Let { name, value });
            }
            ast::Stmt::Return(r) => {
                let value_expr = r
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow!("unsupported_feature: `return` must have a value"))?;
                let value = lower_expr(value_expr, scope, signatures)?;
                out.push(Stmt::Return { value });
            }
            ast::Stmt::If(if_stmt) => {
                let cond = lower_expr(&if_stmt.test, scope, signatures)?;
                // Branch bodies see and may extend the same scope as
                // the surrounding block. (Python doesn't have block
                // scope; locals introduced in a branch are accessible
                // after the branch — though using them when the
                // branch wasn't taken is a runtime UnboundLocalError
                // in CPython. We use alloca slots that hold the
                // last-stored value or undef; we accept this as a
                // pragmatic deviation.)
                let then_body = lower_block(&if_stmt.body, scope, loop_depth, signatures)?;
                let else_body = lower_block(&if_stmt.orelse, scope, loop_depth, signatures)?;
                out.push(Stmt::If { cond, then_body, else_body });
            }
            ast::Stmt::While(w) => {
                if !w.orelse.is_empty() {
                    bail!(
                        "unsupported_feature: `else` clause on `while` is not supported"
                    );
                }
                let cond = lower_expr(&w.test, scope, signatures)?;
                let body = lower_block(&w.body, scope, loop_depth + 1, signatures)?;
                out.push(Stmt::While { cond, body });
            }
            ast::Stmt::Break(_) => {
                if loop_depth == 0 {
                    bail!("unsupported_feature: `break` outside of a loop");
                }
                out.push(Stmt::Break);
            }
            ast::Stmt::Continue(_) => {
                if loop_depth == 0 {
                    bail!("unsupported_feature: `continue` outside of a loop");
                }
                out.push(Stmt::Continue);
            }
            ast::Stmt::Pass(_) => {
                // pass is a no-op; lower it as nothing.
            }
            other => bail!(
                "unsupported_feature: statement `{}` is not supported in v0.6",
                stmt_kind_name(other)
            ),
        }
    }
    Ok(out)
}

fn parse_assign_target(targets: &[ast::Expr]) -> Result<String> {
    if targets.len() != 1 {
        bail!(
            "unsupported_feature: chained assignment `a = b = ...` is not supported (use separate statements)"
        );
    }
    match &targets[0] {
        ast::Expr::Name(n) => Ok(n.id.as_str().to_string()),
        ast::Expr::Tuple(_) | ast::Expr::List(_) => {
            bail!("unsupported_feature: tuple/list unpacking on assignment is not supported")
        }
        ast::Expr::Subscript(_) | ast::Expr::Attribute(_) => {
            bail!("unsupported_feature: subscript / attribute assignment is not supported")
        }
        other => bail!(
            "unsupported_feature: assignment target `{}` is not supported",
            expr_kind_name(other)
        ),
    }
}

fn parse_type_annotation(ann: Option<&ast::Expr>) -> Option<Type> {
    match ann? {
        ast::Expr::Name(n) if n.id.as_str() == "int" => Some(Type::I64),
        _ => None,
    }
}

fn lower_expr(e: &ast::Expr, scope: &HashSet<String>, signatures: &SignatureTable) -> Result<Expr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big.try_into().map_err(|_| {
                    anyhow!("unsupported_feature: integer literal does not fit in i64")
                })?;
                Ok(Expr::ConstI64(v))
            }
            ast::Constant::Bool(b) => Ok(Expr::ConstI64(if *b { 1 } else { 0 })),
            _ => bail!("unsupported_feature: only integer (and bool) literals are supported"),
        },
        ast::Expr::Name(n) => {
            let name = n.id.as_str();
            if scope.contains(name) {
                Ok(Expr::Var(name.to_string()))
            } else {
                bail!(
                    "unsupported_feature: name `{}` is not in scope (must be a parameter or previously assigned local)",
                    name
                )
            }
        }
        ast::Expr::BinOp(b) => {
            let op = match b.op {
                ast::Operator::Add => BinOp::Add,
                ast::Operator::Sub => BinOp::Sub,
                ast::Operator::Mult => BinOp::Mul,
                ast::Operator::FloorDiv => BinOp::FloorDiv,
                ast::Operator::Mod => BinOp::Mod,
                ast::Operator::Div => bail!(
                    "unsupported_feature: `/` (true division) requires float support, not yet in scope"
                ),
                ast::Operator::Pow => BinOp::Pow,
                ast::Operator::MatMult => bail!("unsupported_feature: `@` (matmul) is not supported"),
                ast::Operator::LShift => BinOp::Shl,
                ast::Operator::RShift => BinOp::Shr,
                ast::Operator::BitAnd => BinOp::BitAnd,
                ast::Operator::BitOr => BinOp::BitOr,
                ast::Operator::BitXor => BinOp::BitXor,
            };
            Ok(Expr::BinOp {
                op,
                lhs: Box::new(lower_expr(&b.left, scope, signatures)?),
                rhs: Box::new(lower_expr(&b.right, scope, signatures)?),
            })
        }
        ast::Expr::UnaryOp(u) => {
            let op = match u.op {
                ast::UnaryOp::USub => UnaryOp::Neg,
                ast::UnaryOp::UAdd => UnaryOp::Pos,
                ast::UnaryOp::Not => {
                    return Ok(Expr::Not(Box::new(lower_expr(&u.operand, scope, signatures)?)));
                }
                ast::UnaryOp::Invert => UnaryOp::BitNot,
            };
            Ok(Expr::UnaryOp {
                op,
                operand: Box::new(lower_expr(&u.operand, scope, signatures)?),
            })
        }
        ast::Expr::Compare(c) => {
            // Python AST: left + ops[] + comparators[]. ops.len() == comparators.len().
            let first = lower_expr(&c.left, scope, signatures)?;
            let rest: Result<Vec<(CmpOp, Expr)>> = c
                .ops
                .iter()
                .zip(c.comparators.iter())
                .map(|(op, e)| Ok((convert_cmp_op(op)?, lower_expr(e, scope, signatures)?)))
                .collect();
            let rest = rest?;
            if rest.len() == 1 {
                let (op, rhs) = rest.into_iter().next().unwrap();
                Ok(Expr::Cmp {
                    op,
                    lhs: Box::new(first),
                    rhs: Box::new(rhs),
                })
            } else {
                Ok(Expr::CmpChain {
                    first: Box::new(first),
                    rest,
                })
            }
        }
        ast::Expr::BoolOp(b) => {
            // Python's BoolOp is N-ary: `a and b and c` parses with
            // values=[a, b, c]. Lower as left-associative pairs so the
            // codegen only ever has to deal with binary BoolOp.
            let op = match b.op {
                ast::BoolOp::And => BoolOp::And,
                ast::BoolOp::Or => BoolOp::Or,
            };
            if b.values.is_empty() {
                bail!("unsupported_feature: empty BoolOp (should not be possible from valid Python)");
            }
            let mut iter = b.values.iter();
            let first = lower_expr(iter.next().unwrap(), scope, signatures)?;
            let mut acc = first;
            for next in iter {
                let next = lower_expr(next, scope, signatures)?;
                acc = Expr::BoolOp { op, lhs: Box::new(acc), rhs: Box::new(next) };
            }
            Ok(acc)
        }
        ast::Expr::Call(c) => {
            // Resolve callee. Only plain name calls (`foo(a, b)`)
            // supported — no method calls, no `obj.method()`, no
            // higher-order calls.
            let callee = match c.func.as_ref() {
                ast::Expr::Name(n) => n.id.as_str().to_string(),
                _ => bail!(
                    "unsupported_feature: only direct calls to top-level functions are supported (no method / attribute / higher-order calls yet)"
                ),
            };
            let sig = signatures.get(&callee).ok_or_else(|| {
                anyhow!(
                    "unsupported_feature: call to undefined function `{}`",
                    callee
                )
            })?;
            let args = resolve_call_args(&callee, sig, &c.args, &c.keywords, scope, signatures)?;
            Ok(Expr::Call { callee, args })
        }
        other => bail!(
            "unsupported_feature: expression form `{}` is not supported",
            expr_kind_name(other)
        ),
    }
}

/// Match positional + keyword call args to the callee's parameters,
/// filling in defaults for any unmatched. Errors on duplicates,
/// unknown keywords, missing required args, or `**kwargs` unpacking.
fn resolve_call_args(
    callee: &str,
    sig: &FunctionSig,
    pos_args: &[ast::Expr],
    kw_args: &[ast::Keyword],
    scope: &HashSet<String>,
    signatures: &SignatureTable,
) -> Result<Vec<Expr>> {
    let n = sig.params.len();
    if pos_args.len() > n {
        bail!(
            "unsupported_feature: function `{}` takes {} positional arguments but {} were supplied",
            callee,
            n,
            pos_args.len()
        );
    }
    let mut filled: Vec<Option<Expr>> = vec![None; n];
    for (i, a) in pos_args.iter().enumerate() {
        filled[i] = Some(lower_expr(a, scope, signatures)?);
    }
    for kw in kw_args {
        let name = kw.arg.as_ref().ok_or_else(|| {
            anyhow!(
                "unsupported_feature: `**kwargs` unpacking at call site is not supported (in call to `{}`)",
                callee
            )
        })?;
        let idx = sig
            .params
            .iter()
            .position(|p| p.name == name.as_str())
            .ok_or_else(|| {
                anyhow!(
                    "unsupported_feature: function `{}` has no parameter named `{}`",
                    callee,
                    name
                )
            })?;
        if filled[idx].is_some() {
            bail!(
                "unsupported_feature: multiple values for argument `{}` in call to `{}`",
                name,
                callee
            );
        }
        filled[idx] = Some(lower_expr(&kw.value, scope, signatures)?);
    }
    let mut out = Vec::with_capacity(n);
    for (i, slot) in filled.into_iter().enumerate() {
        match slot {
            Some(e) => out.push(e),
            None => match &sig.defaults[i] {
                Some(default) => out.push(default.clone()),
                None => bail!(
                    "unsupported_feature: missing required argument `{}` in call to `{}`",
                    sig.params[i].name,
                    callee
                ),
            },
        }
    }
    Ok(out)
}

fn convert_cmp_op(op: &ast::CmpOp) -> Result<CmpOp> {
    Ok(match op {
        ast::CmpOp::Lt => CmpOp::Lt,
        ast::CmpOp::LtE => CmpOp::Le,
        ast::CmpOp::Gt => CmpOp::Gt,
        ast::CmpOp::GtE => CmpOp::Ge,
        ast::CmpOp::Eq => CmpOp::Eq,
        ast::CmpOp::NotEq => CmpOp::Ne,
        ast::CmpOp::Is | ast::CmpOp::IsNot => bail!(
            "unsupported_feature: `is` / `is not` are not supported (only allowed against `None` in later slices)"
        ),
        ast::CmpOp::In | ast::CmpOp::NotIn => bail!(
            "unsupported_feature: `in` / `not in` are not supported (need container types first)"
        ),
    })
}

fn stmt_kind_name(s: &ast::Stmt) -> &'static str {
    match s {
        ast::Stmt::FunctionDef(_) => "FunctionDef",
        ast::Stmt::AsyncFunctionDef(_) => "AsyncFunctionDef",
        ast::Stmt::ClassDef(_) => "ClassDef",
        ast::Stmt::Return(_) => "Return",
        ast::Stmt::Delete(_) => "Delete",
        ast::Stmt::Assign(_) => "Assign",
        ast::Stmt::AugAssign(_) => "AugAssign",
        ast::Stmt::AnnAssign(_) => "AnnAssign",
        ast::Stmt::For(_) => "For",
        ast::Stmt::AsyncFor(_) => "AsyncFor",
        ast::Stmt::While(_) => "While",
        ast::Stmt::If(_) => "If",
        ast::Stmt::With(_) => "With",
        ast::Stmt::AsyncWith(_) => "AsyncWith",
        ast::Stmt::Match(_) => "Match",
        ast::Stmt::Raise(_) => "Raise",
        ast::Stmt::Try(_) => "Try",
        ast::Stmt::TryStar(_) => "TryStar",
        ast::Stmt::Assert(_) => "Assert",
        ast::Stmt::Import(_) => "Import",
        ast::Stmt::ImportFrom(_) => "ImportFrom",
        ast::Stmt::Global(_) => "Global",
        ast::Stmt::Nonlocal(_) => "Nonlocal",
        ast::Stmt::Expr(_) => "Expr",
        ast::Stmt::Pass(_) => "Pass",
        ast::Stmt::Break(_) => "Break",
        ast::Stmt::Continue(_) => "Continue",
        ast::Stmt::TypeAlias(_) => "TypeAlias",
    }
}

fn expr_kind_name(e: &ast::Expr) -> &'static str {
    match e {
        ast::Expr::BoolOp(_) => "BoolOp",
        ast::Expr::NamedExpr(_) => "NamedExpr",
        ast::Expr::BinOp(_) => "BinOp",
        ast::Expr::UnaryOp(_) => "UnaryOp",
        ast::Expr::Lambda(_) => "Lambda",
        ast::Expr::IfExp(_) => "IfExp",
        ast::Expr::Dict(_) => "Dict",
        ast::Expr::Set(_) => "Set",
        ast::Expr::ListComp(_) => "ListComp",
        ast::Expr::SetComp(_) => "SetComp",
        ast::Expr::DictComp(_) => "DictComp",
        ast::Expr::GeneratorExp(_) => "GeneratorExp",
        ast::Expr::Await(_) => "Await",
        ast::Expr::Yield(_) => "Yield",
        ast::Expr::YieldFrom(_) => "YieldFrom",
        ast::Expr::Compare(_) => "Compare",
        ast::Expr::Call(_) => "Call",
        ast::Expr::FormattedValue(_) => "FormattedValue",
        ast::Expr::JoinedStr(_) => "JoinedStr",
        ast::Expr::Constant(_) => "Constant",
        ast::Expr::Attribute(_) => "Attribute",
        ast::Expr::Subscript(_) => "Subscript",
        ast::Expr::Starred(_) => "Starred",
        ast::Expr::Name(_) => "Name",
        ast::Expr::List(_) => "List",
        ast::Expr::Tuple(_) => "Tuple",
        ast::Expr::Slice(_) => "Slice",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::path::PathBuf;

    fn parse(src: &str) -> Module {
        parser::parse(src, &PathBuf::from("t.py")).unwrap()
    }

    #[test]
    fn lowers_simple_if_else() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if a < 0:\n        return -a\n    else:\n        return a\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond, then_body, else_body } => {
                assert!(matches!(cond, Expr::Cmp { op: CmpOp::Lt, .. }));
                assert!(matches!(then_body[0], Stmt::Return { .. }));
                assert!(matches!(else_body[0], Stmt::Return { .. }));
            }
            other => panic!("expected If, got {:?}", other),
        }
    }

    #[test]
    fn elif_lowers_to_nested_if() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if a < 0:\n        return -1\n    elif a == 0:\n        return 0\n    else:\n        return 1\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { else_body, .. } => {
                // else_body should contain a single nested If.
                assert_eq!(else_body.len(), 1);
                assert!(matches!(else_body[0], Stmt::If { .. }));
            }
            _ => panic!("expected If at top"),
        }
    }

    #[test]
    fn lowers_chained_compare() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if 0 < a < 100:\n        return 1\n    else:\n        return 0\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond: Expr::CmpChain { first, rest }, .. } => {
                assert!(matches!(**first, Expr::ConstI64(0)));
                assert_eq!(rest.len(), 2);
            }
            _ => panic!("expected CmpChain in If condition"),
        }
    }

    #[test]
    fn lowers_truthy_int_condition() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if a:\n        return 1\n    else:\n        return 0\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond, .. } => assert!(matches!(cond, Expr::Var(n) if n == "a")),
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn lowers_not() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if not a:\n        return 1\n    else:\n        return 0\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond: Expr::Not(_), .. } => {}
            _ => panic!("expected Not in If condition"),
        }
    }

    #[test]
    fn lowers_and_or() {
        let p = lower(&parse(
            "def main(a: int, b: int) -> int:\n    if a > 0 and b > 0:\n        return 1\n    else:\n        return 0\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond: Expr::BoolOp { op: BoolOp::And, .. }, .. } => {}
            other => panic!("expected If with BoolOp::And, got {:?}", other),
        }
    }

    #[test]
    fn lowers_chained_or() {
        // `a or b or c` parses with values=[a,b,c]; we lower it as
        // left-associative pairs.
        let p = lower(&parse(
            "def main(a: int, b: int, c: int) -> int:\n    if a or b or c:\n        return 1\n    else:\n        return 0\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond: Expr::BoolOp { op: BoolOp::Or, lhs, .. }, .. } => {
                assert!(matches!(**lhs, Expr::BoolOp { op: BoolOp::Or, .. }));
            }
            other => panic!("expected If with nested Or, got {:?}", other),
        }
    }

    #[test]
    fn rejects_is() {
        let m = parse("def main(a: int) -> int:\n    if a is 0:\n        return 1\n    else:\n        return 0\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`is`"));
    }

    #[test]
    fn rejects_path_without_return() {
        // `if` with no else, no trailing return — not all paths return.
        let m = parse("def main(a: int) -> int:\n    if a > 0:\n        return 1\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("not all paths return"));
    }

    #[test]
    fn accepts_if_followed_by_return() {
        let _ = lower(&parse(
            "def main(a: int) -> int:\n    if a < 0:\n        return -1\n    return a\n",
        ))
        .unwrap();
    }

    #[test]
    fn accepts_pass_in_branch() {
        let _ = lower(&parse(
            "def main(a: int) -> int:\n    if a > 0:\n        pass\n    return a\n",
        ))
        .unwrap();
    }

    #[test]
    fn lowers_while_loop() {
        let p = lower(&parse(
            "def main(n: int) -> int:\n    i = 0\n    while i < n:\n        i = i + 1\n    return i\n",
        ))
        .unwrap();
        // body[0] = Let i, body[1] = While, body[2] = Return.
        assert!(matches!(p.main().body[1], Stmt::While { .. }));
    }

    #[test]
    fn lowers_break_and_continue_in_loop() {
        let p = lower(&parse(
            "def main(n: int) -> int:\n    i = 0\n    while i < n:\n        if i == 5:\n            break\n        if i == 3:\n            i = i + 1\n            continue\n        i = i + 1\n    return i\n",
        ))
        .unwrap();
        match &p.main().body[1] {
            Stmt::While { body, .. } => {
                // First inner If has Break in its then_body.
                match &body[0] {
                    Stmt::If { then_body, .. } => {
                        assert!(matches!(then_body[0], Stmt::Break));
                    }
                    _ => panic!("expected If at start of while body"),
                }
            }
            _ => panic!("expected While"),
        }
    }

    #[test]
    fn rejects_break_outside_loop() {
        let m = parse("def main() -> int:\n    break\n    return 0\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`break` outside"));
    }

    #[test]
    fn rejects_continue_outside_loop() {
        let m = parse("def main() -> int:\n    continue\n    return 0\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`continue` outside"));
    }

    #[test]
    fn lowers_aug_assign() {
        // `a += 1` desugars to `a = a + 1`.
        let p = lower(&parse("def main(a: int) -> int:\n    a += 1\n    return a\n")).unwrap();
        match &p.main().body[0] {
            Stmt::Let { name, value: Expr::BinOp { op: BinOp::Add, lhs, rhs } } => {
                assert_eq!(name, "a");
                assert!(matches!(**lhs, Expr::Var(ref n) if n == "a"));
                assert!(matches!(**rhs, Expr::ConstI64(1)));
            }
            other => panic!("expected Let with BinOp::Add desugar, got {:?}", other),
        }
    }

    #[test]
    fn rejects_aug_assign_to_unbound_name() {
        let m = parse("def main() -> int:\n    x += 1\n    return x\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("unbound name `x`"));
    }

    #[test]
    fn rejects_else_on_while() {
        let m = parse(
            "def main(n: int) -> int:\n    while n > 0:\n        n = n - 1\n    else:\n        n = -1\n    return n\n",
        );
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`else` clause on `while`"));
    }

    #[test]
    fn while_alone_does_not_satisfy_path_coverage() {
        // `while` is not a covering construct — last statement must be a return.
        let m = parse(
            "def main(n: int) -> int:\n    while n > 0:\n        return n\n",
        );
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("not all paths return"));
    }
}
