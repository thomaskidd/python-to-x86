use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};
use rustpython_parser::ast;

use crate::hir::{
    BinOp, BoolOp, CmpOp, Expr, Function, Param, Program, Stmt, TupleId, Type, TypedExpr, UnaryOp,
};
use crate::parser::Module;

const MAX_PARAMS: usize = 16;

/// Per-function type scope: name → type.
type Scope = HashMap<String, Type>;

#[derive(Debug)]
struct FunctionSig {
    params: Vec<Param>,
    defaults: Vec<Option<TypedExpr>>,
    return_ty: Type,
}

type SignatureTable = HashMap<String, FunctionSig>;

pub fn lower(module: &Module) -> Result<Program> {
    // Pass 1: collect signatures.
    let mut signatures: SignatureTable = HashMap::new();
    for stmt in &module.body {
        match stmt {
            ast::Stmt::FunctionDef(f) => {
                let sig = collect_signature(f)?;
                let name = f.name.as_str().to_string();
                if signatures.contains_key(&name) {
                    bail!("unsupported_feature: duplicate top-level function `{}`", name);
                }
                signatures.insert(name, sig);
            }
            ast::Stmt::ImportFrom(im) => {
                // Allowed: `from pyx86.types import …` (type-name documentation)
                //          `from __future__ import annotations` (lets test
                //              programs use PEP 585 syntax under Python 3.8)
                let module_name = im
                    .module
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("");
                match module_name {
                    "pyx86.types" | "__future__" => {
                        // Accepted; we don't act on the imports — type names
                        // are resolved directly and we ignore Python's lazy-
                        // annotation semantics.
                    }
                    _ => bail!(
                        "unsupported_feature: only `from pyx86.types import …` and `from __future__ import …` imports are supported at module level (found `from {} import …`)",
                        module_name
                    ),
                }
            }
            other => bail!(
                "unsupported_feature: only `def` and `from pyx86.types import …` are allowed at the top level, found {}",
                stmt_kind_name(other)
            ),
        }
    }
    let main_sig = signatures.get("main").ok_or_else(|| {
        anyhow!("unsupported_feature: no `main` function defined at the top level")
    })?;
    // main return must be int or float — bool isn't a useful CLI return
    // (and we don't have a printer for it).
    if !matches!(main_sig.return_ty, Type::I64 | Type::F64) {
        bail!(
            "unsupported_feature: `main` must return `int` or `float`, found {}",
            main_sig.return_ty.name()
        );
    }
    for p in &main_sig.params {
        if !matches!(p.ty, Type::I64 | Type::F64) {
            bail!(
                "unsupported_feature: `main` parameter `{}` must be `int` or `float`, found {}",
                p.name,
                p.ty.name()
            );
        }
    }

    // Pass 2: lower each function body. Skip non-FunctionDef stmts
    // (already validated in pass 1).
    let mut functions = Vec::with_capacity(module.body.len());
    for stmt in &module.body {
        if let ast::Stmt::FunctionDef(func) = stmt {
            functions.push(lower_function(func, &signatures)?);
        }
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

    let raw_defaults: Vec<&ast::Expr> = func.args.defaults().collect();
    let n = params.len();
    let n_defaulted = raw_defaults.len();
    let n_required = n - n_defaulted;
    let mut defaults: Vec<Option<TypedExpr>> = vec![None; n];
    for (i, raw) in raw_defaults.iter().enumerate() {
        let param_idx = n_required + i;
        let lowered = lower_default(raw).map_err(|e| {
            anyhow!(
                "unsupported_feature: default for parameter `{}` (in `{}`): {}",
                params[param_idx].name,
                func.name,
                e
            )
        })?;
        // Coerce default to declared param type.
        let coerced = coerce(lowered, params[param_idx].ty)?;
        defaults[param_idx] = Some(coerced);
    }

    Ok(FunctionSig { params, defaults, return_ty })
}

fn lower_default(e: &ast::Expr) -> Result<TypedExpr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big
                    .try_into()
                    .map_err(|_| anyhow!("integer literal does not fit in i64"))?;
                Ok(TypedExpr::new(Type::I64, Expr::ConstI64(v)))
            }
            ast::Constant::Bool(b) => Ok(TypedExpr::new(Type::Bool, Expr::ConstBool(*b))),
            _ => bail!("only integer or bool literals are allowed as defaults"),
        },
        ast::Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub | ast::UnaryOp::UAdd) => {
            let inner = lower_default(&u.operand)?;
            match (u.op, &inner.expr) {
                (ast::UnaryOp::USub, Expr::ConstI64(v)) => {
                    Ok(TypedExpr::new(Type::I64, Expr::ConstI64(-v)))
                }
                (ast::UnaryOp::UAdd, _) => Ok(inner),
                _ => bail!("only integer literals are allowed as defaults"),
            }
        }
        _ => bail!("only integer literals are allowed as defaults"),
    }
}

fn lower_function(func: &ast::StmtFunctionDef, signatures: &SignatureTable) -> Result<Function> {
    let name = func.name.as_str().to_string();
    let sig = signatures.get(&name).expect("signature collected in pass 1");

    let mut scope: Scope =
        sig.params.iter().map(|p| (p.name.clone(), p.ty)).collect();

    if func.body.is_empty() {
        bail!("unsupported_feature: function `{}` body is empty", name);
    }

    let body = lower_block(&func.body, &mut scope, 0, signatures, sig.return_ty)?;

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
    scope: &mut Scope,
    loop_depth: usize,
    signatures: &SignatureTable,
    return_ty: Type,
) -> Result<Vec<Stmt>> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        match stmt {
            ast::Stmt::Assign(a) => {
                let name = parse_assign_target(&a.targets)?;
                let value = lower_expr(&a.value, scope, signatures)?;
                scope.insert(name.clone(), value.ty);
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
                let declared_ty = parse_type_annotation(Some(&a.annotation)).ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: only `: int` / `: float` / `: bool` annotations are supported on locals, on `{}`",
                        name
                    )
                })?;
                let value_expr = a.value.as_deref().ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: bare annotation `{}: <type>` (no value) is not supported",
                        name
                    )
                })?;
                let value = lower_expr(value_expr, scope, signatures)?;
                let value = coerce(value, declared_ty)?;
                scope.insert(name.clone(), declared_ty);
                out.push(Stmt::Let { name, value });
            }
            ast::Stmt::AugAssign(a) => {
                let name = match a.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: augmented-assignment target must be a simple name"
                    ),
                };
                let lhs_ty = *scope.get(&name).ok_or_else(|| {
                    anyhow!(
                        "unsupported_feature: augmented assignment to unbound name `{}` (must already be a parameter or assigned local)",
                        name
                    )
                })?;
                let op = match a.op {
                    ast::Operator::Add => BinOp::Add,
                    ast::Operator::Sub => BinOp::Sub,
                    ast::Operator::Mult => BinOp::Mul,
                    ast::Operator::FloorDiv => BinOp::FloorDiv,
                    ast::Operator::Mod => BinOp::Mod,
                    ast::Operator::Div => BinOp::TrueDiv,
                    ast::Operator::Pow => BinOp::Pow,
                    ast::Operator::LShift => BinOp::Shl,
                    ast::Operator::RShift => BinOp::Shr,
                    ast::Operator::BitAnd => BinOp::BitAnd,
                    ast::Operator::BitOr => BinOp::BitOr,
                    ast::Operator::BitXor => BinOp::BitXor,
                    ast::Operator::MatMult => bail!(
                        "unsupported_feature: `@=` (matmul) is not supported"
                    ),
                };
                let lhs = TypedExpr::new(lhs_ty, Expr::Var(name.clone()));
                let rhs = lower_expr(&a.value, scope, signatures)?;
                let combined = apply_binop(op, lhs, rhs)?;
                let combined = coerce(combined, lhs_ty)?;
                out.push(Stmt::Let { name, value: combined });
            }
            ast::Stmt::Return(r) => {
                let value_expr = r
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow!("unsupported_feature: `return` must have a value"))?;
                let value = lower_expr(value_expr, scope, signatures)?;
                let value = coerce(value, return_ty)?;
                out.push(Stmt::Return { value });
            }
            ast::Stmt::If(if_stmt) => {
                let cond = lower_expr(&if_stmt.test, scope, signatures)?;
                let cond = coerce(cond, Type::Bool)?;
                let then_body = lower_block(&if_stmt.body, scope, loop_depth, signatures, return_ty)?;
                let else_body =
                    lower_block(&if_stmt.orelse, scope, loop_depth, signatures, return_ty)?;
                out.push(Stmt::If { cond, then_body, else_body });
            }
            ast::Stmt::For(f) => {
                if !f.orelse.is_empty() {
                    bail!("unsupported_feature: `else` clause on `for` is not supported");
                }
                let loop_var = match f.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!(
                        "unsupported_feature: for-loop target must be a simple name (no tuple unpacking yet)"
                    ),
                };
                let (start, stop, step) =
                    parse_and_lower_range(&f.iter, scope, signatures)?;
                let step_value = match &step.expr {
                    Expr::ConstI64(v) => *v,
                    _ => bail!(
                        "unsupported_feature: range() step must be a constant integer literal in v0.14"
                    ),
                };
                if step_value == 0 {
                    bail!("unsupported_feature: range() step must be non-zero");
                }
                if step_value < 0 {
                    bail!(
                        "unsupported_feature: negative range() step is not yet supported (use a `while` loop)"
                    );
                }

                // Bind loop_var BEFORE lowering the body so the body can
                // reference it.
                scope.insert(loop_var.clone(), Type::I64);
                let body_inner =
                    lower_block(&f.body, scope, loop_depth + 1, signatures, return_ty)?;

                // Desugar to:
                //   loop_var = start
                //   while loop_var < stop:
                //     <body>
                //     loop_var = loop_var + step
                out.push(Stmt::Let { name: loop_var.clone(), value: start });

                let cond = TypedExpr::new(
                    Type::Bool,
                    Expr::Cmp {
                        op: CmpOp::Lt,
                        lhs: Box::new(TypedExpr::new(
                            Type::I64,
                            Expr::Var(loop_var.clone()),
                        )),
                        rhs: Box::new(stop),
                    },
                );

                let mut while_body = body_inner;
                let incr = TypedExpr::new(
                    Type::I64,
                    Expr::BinOp {
                        op: BinOp::Add,
                        lhs: Box::new(TypedExpr::new(
                            Type::I64,
                            Expr::Var(loop_var.clone()),
                        )),
                        rhs: Box::new(step),
                    },
                );
                while_body.push(Stmt::Let { name: loop_var.clone(), value: incr });
                out.push(Stmt::While { cond, body: while_body });
            }
            ast::Stmt::While(w) => {
                if !w.orelse.is_empty() {
                    bail!(
                        "unsupported_feature: `else` clause on `while` is not supported"
                    );
                }
                let cond = lower_expr(&w.test, scope, signatures)?;
                let cond = coerce(cond, Type::Bool)?;
                let body = lower_block(&w.body, scope, loop_depth + 1, signatures, return_ty)?;
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
            ast::Stmt::Pass(_) => {}
            other => bail!(
                "unsupported_feature: statement `{}` is not supported",
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
        ast::Expr::Name(n) => match n.id.as_str() {
            "int" => Some(Type::I64),
            "float" => Some(Type::F64),
            "bool" => Some(Type::Bool),
            "i8" => Some(Type::I8),
            "i16" => Some(Type::I16),
            "i32" => Some(Type::I32),
            "i64" => Some(Type::I64),
            _ => None,
        },
        // `tuple[int, float, int]` — Subscript with the tuple of element
        // types as the slice. CPython parses this as Subscript(Name("tuple"), Tuple([...])).
        ast::Expr::Subscript(s) => {
            match s.value.as_ref() {
                ast::Expr::Name(n) if n.id.as_str() == "tuple" => {}
                _ => return None,
            }
            let elem_exprs: Vec<&ast::Expr> = match s.slice.as_ref() {
                ast::Expr::Tuple(t) => t.elts.iter().collect(),
                single => vec![single],
            };
            let mut elems = Vec::with_capacity(elem_exprs.len());
            for e in elem_exprs {
                elems.push(parse_type_annotation(Some(e))?);
            }
            Some(Type::Tuple(TupleId::intern(elems)))
        }
        _ => None,
    }
}

fn lower_expr(e: &ast::Expr, scope: &Scope, signatures: &SignatureTable) -> Result<TypedExpr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big.try_into().map_err(|_| {
                    anyhow!("unsupported_feature: integer literal does not fit in i64")
                })?;
                Ok(TypedExpr::new(Type::I64, Expr::ConstI64(v)))
            }
            ast::Constant::Float(f) => Ok(TypedExpr::new(Type::F64, Expr::ConstF64(*f))),
            ast::Constant::Bool(b) => Ok(TypedExpr::new(Type::Bool, Expr::ConstBool(*b))),
            _ => bail!("unsupported_feature: only int / float / bool literals are supported"),
        },
        ast::Expr::Name(n) => {
            let name = n.id.as_str();
            let ty = scope.get(name).copied().ok_or_else(|| {
                anyhow!(
                    "unsupported_feature: name `{}` is not in scope (must be a parameter or previously assigned local)",
                    name
                )
            })?;
            Ok(TypedExpr::new(ty, Expr::Var(name.to_string())))
        }
        ast::Expr::BinOp(b) => {
            let op = match b.op {
                ast::Operator::Add => BinOp::Add,
                ast::Operator::Sub => BinOp::Sub,
                ast::Operator::Mult => BinOp::Mul,
                ast::Operator::FloorDiv => BinOp::FloorDiv,
                ast::Operator::Mod => BinOp::Mod,
                ast::Operator::Div => BinOp::TrueDiv,
                ast::Operator::Pow => BinOp::Pow,
                ast::Operator::MatMult => {
                    bail!("unsupported_feature: `@` (matmul) is not supported")
                }
                ast::Operator::LShift => BinOp::Shl,
                ast::Operator::RShift => BinOp::Shr,
                ast::Operator::BitAnd => BinOp::BitAnd,
                ast::Operator::BitOr => BinOp::BitOr,
                ast::Operator::BitXor => BinOp::BitXor,
            };
            let lhs = lower_expr(&b.left, scope, signatures)?;
            let rhs = lower_expr(&b.right, scope, signatures)?;
            apply_binop(op, lhs, rhs)
        }
        ast::Expr::UnaryOp(u) => {
            let operand = lower_expr(&u.operand, scope, signatures)?;
            match u.op {
                ast::UnaryOp::USub => apply_unop(UnaryOp::Neg, operand),
                ast::UnaryOp::UAdd => apply_unop(UnaryOp::Pos, operand),
                ast::UnaryOp::Not => {
                    let coerced = coerce(operand, Type::Bool)?;
                    Ok(TypedExpr::new(Type::Bool, Expr::Not(Box::new(coerced))))
                }
                ast::UnaryOp::Invert => apply_unop(UnaryOp::BitNot, operand),
            }
        }
        ast::Expr::Compare(c) => {
            let first = lower_expr(&c.left, scope, signatures)?;
            let rest_ops: Result<Vec<(CmpOp, TypedExpr)>> = c
                .ops
                .iter()
                .zip(c.comparators.iter())
                .map(|(op, e)| Ok((convert_cmp_op(op)?, lower_expr(e, scope, signatures)?)))
                .collect();
            let rest = rest_ops?;
            if rest.len() == 1 {
                let (op, rhs) = rest.into_iter().next().unwrap();
                let (lhs, rhs) = unify_cmp_operands(first, rhs)?;
                Ok(TypedExpr::new(
                    Type::Bool,
                    Expr::Cmp { op, lhs: Box::new(lhs), rhs: Box::new(rhs) },
                ))
            } else {
                Ok(TypedExpr::new(
                    Type::Bool,
                    Expr::CmpChain { first: Box::new(first), rest },
                ))
            }
        }
        ast::Expr::BoolOp(b) => {
            let op = match b.op {
                ast::BoolOp::And => BoolOp::And,
                ast::BoolOp::Or => BoolOp::Or,
            };
            if b.values.is_empty() {
                bail!("unsupported_feature: empty BoolOp");
            }
            let mut iter = b.values.iter();
            let first = lower_expr(iter.next().unwrap(), scope, signatures)?;
            let mut acc = first;
            for next in iter {
                let next = lower_expr(next, scope, signatures)?;
                let (l, r) = unify_numeric(acc, next)?;
                let ty = l.ty;
                acc = TypedExpr::new(
                    ty,
                    Expr::BoolOp { op, lhs: Box::new(l), rhs: Box::new(r) },
                );
            }
            Ok(acc)
        }
        ast::Expr::Call(c) => {
            let callee = match c.func.as_ref() {
                ast::Expr::Name(n) => n.id.as_str().to_string(),
                _ => bail!(
                    "unsupported_feature: only direct calls to top-level functions are supported"
                ),
            };
            // Special-case the small set of supported Python builtins
            // before the user-defined-function lookup. Builtins shadow
            // user functions if both exist (matches CPython where the
            // local function takes priority — but we don't allow
            // shadowing builtins as a user function name).
            if let Some(builtin) = lower_builtin_call(&callee, &c.args, &c.keywords, scope, signatures)? {
                return Ok(builtin);
            }
            let sig = signatures.get(&callee).ok_or_else(|| {
                anyhow!("unsupported_feature: call to undefined function `{}`", callee)
            })?;
            let args = resolve_call_args(&callee, sig, &c.args, &c.keywords, scope, signatures)?;
            Ok(TypedExpr::new(sig.return_ty, Expr::Call { callee, args }))
        }
        ast::Expr::Tuple(t) => {
            let elements: Result<Vec<TypedExpr>> = t
                .elts
                .iter()
                .map(|e| lower_expr(e, scope, signatures))
                .collect();
            let elements = elements?;
            let elem_types: Vec<Type> = elements.iter().map(|e| e.ty).collect();
            let id = TupleId::intern(elem_types);
            Ok(TypedExpr::new(
                Type::Tuple(id),
                Expr::TupleLit { elements },
            ))
        }
        ast::Expr::Subscript(s) => {
            let value = lower_expr(&s.value, scope, signatures)?;
            let id = match value.ty {
                Type::Tuple(id) => id,
                other => bail!(
                    "unsupported_feature: subscripting non-tuple type {} is not supported",
                    other.name()
                ),
            };
            let index_value = match s.slice.as_ref() {
                ast::Expr::Constant(c) => match &c.value {
                    ast::Constant::Int(big) => {
                        let v: i64 = big
                            .try_into()
                            .map_err(|_| anyhow!("tuple index doesn't fit in i64"))?;
                        v
                    }
                    _ => bail!("unsupported_feature: tuple index must be an integer literal"),
                },
                ast::Expr::UnaryOp(u) if matches!(u.op, ast::UnaryOp::USub) => {
                    match u.operand.as_ref() {
                        ast::Expr::Constant(c) => match &c.value {
                            ast::Constant::Int(big) => {
                                let v: i64 = big
                                    .try_into()
                                    .map_err(|_| anyhow!("tuple index doesn't fit in i64"))?;
                                -v
                            }
                            _ => bail!("unsupported_feature: tuple index must be an integer literal"),
                        },
                        _ => bail!("unsupported_feature: tuple index must be an integer literal"),
                    }
                }
                _ => bail!(
                    "unsupported_feature: tuple index must be a constant integer literal"
                ),
            };
            let n = id.with_elems(|elems| elems.len()) as i64;
            let idx = if index_value < 0 { n + index_value } else { index_value };
            if idx < 0 || idx >= n {
                bail!(
                    "unsupported_feature: tuple index {} out of range for {}",
                    index_value,
                    value.ty.name()
                );
            }
            let elem_ty = id.with_elems(|elems| elems[idx as usize]);
            Ok(TypedExpr::new(
                elem_ty,
                Expr::TupleIndex {
                    tuple: Box::new(value),
                    index: idx as usize,
                },
            ))
        }
        other => bail!(
            "unsupported_feature: expression form `{}` is not supported",
            expr_kind_name(other)
        ),
    }
}

/// Apply a binary op given lowered operands. Handles type promotion
/// and inserts coercions as needed.
fn apply_binop(op: BinOp, lhs: TypedExpr, rhs: TypedExpr) -> Result<TypedExpr> {
    match op {
        BinOp::TrueDiv => {
            // Always F64 result. Promote both to F64.
            let l = coerce(lhs, Type::F64)?;
            let r = coerce(rhs, Type::F64)?;
            Ok(TypedExpr::new(
                Type::F64,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::FloorDiv | BinOp::Mod => {
            // Reject float (Python supports float `%` but we don't yet).
            if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                bail!(
                    "unsupported_feature: `//` / `%` on float operands not yet supported"
                );
            }
            // Unify int widths.
            let (l, r) = unify_int_widths(lhs, rhs)?;
            let ty = l.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
            if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                bail!(
                    "unsupported_feature: bitwise ops on float operands are not allowed"
                );
            }
            let (l, r) = unify_int_widths(lhs, rhs)?;
            let ty = l.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::Shl | BinOp::Shr => {
            if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                bail!("unsupported_feature: shift ops on float operands are not allowed");
            }
            // Result type follows lhs width; rhs is coerced to lhs's width.
            let lhs = coerce_int_keep_width(lhs)?;
            let target = lhs.ty;
            let r = coerce(rhs, target)?;
            Ok(TypedExpr::new(
                target,
                Expr::BinOp { op, lhs: Box::new(lhs), rhs: Box::new(r) },
            ))
        }
        BinOp::Pow => {
            // Int**Int → I64 (using runtime helper, always i64 result for
            // simplicity; sub-i64 ints would be widened). Float**Float → F64. Mixed → F64.
            let result_ty = if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
                Type::F64
            } else {
                Type::I64
            };
            let l = coerce(lhs, result_ty)?;
            let r = coerce(rhs, result_ty)?;
            Ok(TypedExpr::new(
                result_ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
        BinOp::Add | BinOp::Sub | BinOp::Mul => {
            let (l, r) = unify_numeric(lhs, rhs)?;
            let ty = l.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::BinOp { op, lhs: Box::new(l), rhs: Box::new(r) },
            ))
        }
    }
}

/// Unify two int-shaped operands to the wider int width. Bool counts
/// as I64 (Python-ish). Caller must have already verified neither is F64.
fn unify_int_widths(lhs: TypedExpr, rhs: TypedExpr) -> Result<(TypedExpr, TypedExpr)> {
    let lty = if lhs.ty == Type::Bool { Type::I64 } else { lhs.ty };
    let rty = if rhs.ty == Type::Bool { Type::I64 } else { rhs.ty };
    let target = match (lty.int_width(), rty.int_width()) {
        (Some(lw), Some(rw)) => match lw.max(rw) {
            8 => Type::I8,
            16 => Type::I16,
            32 => Type::I32,
            _ => Type::I64,
        },
        _ => bail!(
            "unsupported_feature: cannot unify {} and {} as ints",
            lhs.ty.name(),
            rhs.ty.name()
        ),
    };
    Ok((coerce(lhs, target)?, coerce(rhs, target)?))
}

/// If the expression is a Bool, coerce it to I64; otherwise pass through.
/// Used where we want the int-shaped width of the operand to drive the result.
fn coerce_int_keep_width(e: TypedExpr) -> Result<TypedExpr> {
    if e.ty == Type::Bool {
        coerce(e, Type::I64)
    } else if e.ty.is_int() {
        Ok(e)
    } else {
        bail!("unsupported_feature: expected int-shaped operand, got {}", e.ty.name())
    }
}

fn apply_unop(op: UnaryOp, operand: TypedExpr) -> Result<TypedExpr> {
    match op {
        UnaryOp::Neg | UnaryOp::Pos => {
            // Numeric: keep type; Bool → I64 first.
            let operand = if operand.ty == Type::Bool {
                coerce(operand, Type::I64)?
            } else {
                operand
            };
            let ty = operand.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::UnaryOp { op, operand: Box::new(operand) },
            ))
        }
        UnaryOp::BitNot => {
            // Bitwise not on int operand of any width.
            let operand = coerce_int_keep_width(operand)?;
            let ty = operand.ty;
            Ok(TypedExpr::new(
                ty,
                Expr::UnaryOp { op, operand: Box::new(operand) },
            ))
        }
    }
}

/// Numeric promotion for arithmetic operands.
///
/// Rules (in order):
/// - If either is F64, both become F64.
/// - Otherwise, both become the wider of the two int types (Bool counts
///   as 1-bit; treated as I64 when mixed with anything int-shaped to
///   match Python's `True + 1 == 2` semantics).
fn unify_numeric(lhs: TypedExpr, rhs: TypedExpr) -> Result<(TypedExpr, TypedExpr)> {
    if lhs.ty == Type::F64 || rhs.ty == Type::F64 {
        return Ok((coerce(lhs, Type::F64)?, coerce(rhs, Type::F64)?));
    }
    // Both int-shaped (I*) or Bool. Bool counts as I64 (Python-ish).
    let lty = if lhs.ty == Type::Bool { Type::I64 } else { lhs.ty };
    let rty = if rhs.ty == Type::Bool { Type::I64 } else { rhs.ty };
    let target = match (lty.int_width(), rty.int_width()) {
        (Some(lw), Some(rw)) => match lw.max(rw) {
            8 => Type::I8,
            16 => Type::I16,
            32 => Type::I32,
            _ => Type::I64,
        },
        _ => bail!(
            "unsupported_feature: cannot unify {} and {} for arithmetic",
            lhs.ty.name(),
            rhs.ty.name()
        ),
    };
    Ok((coerce(lhs, target)?, coerce(rhs, target)?))
}

/// For comparisons: same as numeric promotion (Bool → I64, then int+float → float).
fn unify_cmp_operands(lhs: TypedExpr, rhs: TypedExpr) -> Result<(TypedExpr, TypedExpr)> {
    unify_numeric(lhs, rhs)
}

/// Insert a coercion if the expression's type doesn't match the target.
/// Allowed coercions:
/// - between any two int widths (sext or trunc; signed semantics)
/// - Bool ↔ any int (zext / icmp ne 0)
/// - any int → F64 (sitofp)
/// - Bool → F64 (zext to int then sitofp)
/// - F64 → Bool (fcmp one … 0.0)
///
/// F64 → int is rejected (would be lossy; requires explicit cast that
/// we don't have a builtin for yet).
fn coerce(e: TypedExpr, target: Type) -> Result<TypedExpr> {
    if e.ty == target {
        return Ok(e);
    }
    let allowed = match (e.ty, target) {
        // Float → int: lossy, rejected.
        (Type::F64, t) if t.is_int() => {
            bail!(
                "unsupported_feature: implicit float→{} conversion is not allowed (use an explicit cast — coming later)",
                t.name()
            )
        }
        // Int width changes (incl. Bool ↔ int): always allowed via sext/trunc/zext.
        (a, b) if (a.is_int() || a == Type::Bool) && (b.is_int() || b == Type::Bool) => true,
        // Numeric → F64: int / Bool → F64 is allowed.
        (a, Type::F64) if a.is_int() || a == Type::Bool => true,
        // F64 → Bool: allowed (fcmp).
        (Type::F64, Type::Bool) => true,
        _ => false,
    };
    if !allowed {
        bail!(
            "unsupported_feature: cannot coerce {} to {}",
            e.ty.name(),
            target.name()
        );
    }
    Ok(TypedExpr::new(target, Expr::Coerce { inner: Box::new(e) }))
}

/// Recognize and lower a small set of Python builtins. Returns
/// `Ok(Some(_))` if it's a builtin, `Ok(None)` if not, `Err(_)` on
/// invalid use. Builtins handled here:
/// - int(x)    — convert to I64. For F64 inputs uses fptosi (truncate
///                toward zero, matches CPython for finite values).
/// - float(x)  — convert to F64.
/// - bool(x)   — convert to Bool (truthy check).
/// - abs(x)    — absolute value, preserves type (int / float).
/// - min(a, b) — smaller of two same-type values.
/// - max(a, b) — larger of two same-type values.
fn lower_builtin_call(
    name: &str,
    args: &[ast::Expr],
    kwargs: &[ast::Keyword],
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<Option<TypedExpr>> {
    if !kwargs.is_empty() {
        return Ok(None);
    }
    match name {
        "int" => {
            if args.len() != 1 {
                bail!("unsupported_feature: int() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            if inner.ty == Type::F64 {
                // Explicit float→int via fptosi.
                Ok(Some(TypedExpr::new(
                    Type::I64,
                    Expr::Coerce { inner: Box::new(inner) },
                )))
            } else {
                Ok(Some(coerce(inner, Type::I64)?))
            }
        }
        "float" => {
            if args.len() != 1 {
                bail!("unsupported_feature: float() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            Ok(Some(coerce(inner, Type::F64)?))
        }
        "bool" => {
            if args.len() != 1 {
                bail!("unsupported_feature: bool() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            Ok(Some(coerce(inner, Type::Bool)?))
        }
        "abs" => {
            if args.len() != 1 {
                bail!("unsupported_feature: abs() takes exactly 1 argument");
            }
            let inner = lower_expr(&args[0], scope, signatures)?;
            let inner = if inner.ty == Type::Bool { coerce(inner, Type::I64)? } else { inner };
            let ty = inner.ty;
            if ty != Type::F64 && !ty.is_int() {
                bail!("unsupported_feature: abs() argument must be int or float");
            }
            // Build `(x < 0 and -x) or x` using BoolOp's short-circuit
            // value semantics:
            //   x >= 0: (False and ...) = 0, (0 or x) = x
            //   x <  0: (True  and -x ) = -x, (-x or x) = -x  (since -x is truthy)
            //   x == 0: gives 0 ∎
            let neg = TypedExpr::new(
                ty,
                Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    operand: Box::new(inner.clone()),
                },
            );
            let zero = if ty == Type::F64 {
                TypedExpr::new(Type::F64, Expr::ConstF64(0.0))
            } else {
                TypedExpr::new(ty, Expr::ConstI64(0))
            };
            let is_neg = TypedExpr::new(
                Type::Bool,
                Expr::Cmp {
                    op: CmpOp::Lt,
                    lhs: Box::new(inner.clone()),
                    rhs: Box::new(zero),
                },
            );
            let and_branch = TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::And,
                    lhs: Box::new(coerce(is_neg, ty)?),
                    rhs: Box::new(neg),
                },
            );
            Ok(Some(TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::Or,
                    lhs: Box::new(and_branch),
                    rhs: Box::new(inner),
                },
            )))
        }
        "min" | "max" => {
            if args.len() != 2 {
                bail!(
                    "unsupported_feature: {}() with {} arguments — only 2-arg form supported",
                    name,
                    args.len()
                );
            }
            let a = lower_expr(&args[0], scope, signatures)?;
            let b = lower_expr(&args[1], scope, signatures)?;
            let (a, b) = unify_numeric(a, b)?;
            let ty = a.ty;
            let cmp_op = if name == "min" { CmpOp::Le } else { CmpOp::Ge };
            let cmp = TypedExpr::new(
                Type::Bool,
                Expr::Cmp {
                    op: cmp_op,
                    lhs: Box::new(a.clone()),
                    rhs: Box::new(b.clone()),
                },
            );
            let and_branch = TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::And,
                    lhs: Box::new(coerce(cmp, ty)?),
                    rhs: Box::new(a),
                },
            );
            Ok(Some(TypedExpr::new(
                ty,
                Expr::BoolOp {
                    op: BoolOp::Or,
                    lhs: Box::new(and_branch),
                    rhs: Box::new(b),
                },
            )))
        }
        _ => Ok(None),
    }
}

fn resolve_call_args(
    callee: &str,
    sig: &FunctionSig,
    pos_args: &[ast::Expr],
    kw_args: &[ast::Keyword],
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<Vec<TypedExpr>> {
    let n = sig.params.len();
    if pos_args.len() > n {
        bail!(
            "unsupported_feature: function `{}` takes {} positional arguments but {} were supplied",
            callee,
            n,
            pos_args.len()
        );
    }
    let mut filled: Vec<Option<TypedExpr>> = vec![None; n];
    for (i, a) in pos_args.iter().enumerate() {
        let raw = lower_expr(a, scope, signatures)?;
        filled[i] = Some(coerce(raw, sig.params[i].ty)?);
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
        let raw = lower_expr(&kw.value, scope, signatures)?;
        filled[idx] = Some(coerce(raw, sig.params[idx].ty)?);
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

/// Parse and lower a `range(...)` call as the iterable of a for-loop.
/// Returns (start, stop, step) all as I64 TypedExprs. Defaults: start=0, step=1.
fn parse_and_lower_range(
    iter: &ast::Expr,
    scope: &Scope,
    signatures: &SignatureTable,
) -> Result<(TypedExpr, TypedExpr, TypedExpr)> {
    let call = match iter {
        ast::Expr::Call(c) => c,
        _ => bail!(
            "unsupported_feature: for-loop iterables other than range(...) are not supported in v0.14"
        ),
    };
    if !matches!(call.func.as_ref(), ast::Expr::Name(n) if n.id.as_str() == "range") {
        bail!(
            "unsupported_feature: for-loop iterables other than range(...) are not supported in v0.14"
        );
    }
    if !call.keywords.is_empty() {
        bail!("unsupported_feature: range() does not accept keyword arguments");
    }
    let zero = TypedExpr::new(Type::I64, Expr::ConstI64(0));
    let one = TypedExpr::new(Type::I64, Expr::ConstI64(1));
    let lower_to_i64 = |e: &ast::Expr| -> Result<TypedExpr> {
        let raw = lower_expr(e, scope, signatures)?;
        coerce(raw, Type::I64)
    };
    match call.args.as_slice() {
        [stop] => Ok((zero, lower_to_i64(stop)?, one)),
        [start, stop] => Ok((lower_to_i64(start)?, lower_to_i64(stop)?, one)),
        [start, stop, step] => Ok((
            lower_to_i64(start)?,
            lower_to_i64(stop)?,
            lower_to_i64(step)?,
        )),
        _ => bail!("unsupported_feature: range() takes 1, 2, or 3 arguments"),
    }
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
    fn lowers_no_param_main() {
        let p = lower(&parse("def main() -> int:\n    return 42\n")).unwrap();
        assert_eq!(p.main().params.len(), 0);
        assert_eq!(p.main().return_ty, Type::I64);
    }

    #[test]
    fn lowers_two_param_main() {
        let p = lower(&parse(
            "def main(a: int, b: int) -> int:\n    return a + b\n",
        ))
        .unwrap();
        assert_eq!(p.main().params.len(), 2);
        assert_eq!(p.main().params[0].ty, Type::I64);
    }

    #[test]
    fn allows_float_local_inside_int_main() {
        // float values flow through the program but main return is int.
        // The result of true-div is F64; we check it via comparison.
        let _ = lower(&parse(
            "def helper() -> float:\n    return 1.5 + 2.5\n\ndef main() -> int:\n    x: float = helper()\n    if x > 0.0:\n        return 1\n    else:\n        return 0\n",
        ))
        .unwrap();
    }

    #[test]
    fn rejects_implicit_float_to_int_on_return() {
        let m = parse("def main() -> int:\n    return 1.5\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("float→int"));
    }

    #[test]
    fn accepts_float_main_return() {
        let _ = lower(&parse("def main() -> float:\n    return 1.0\n")).unwrap();
    }

    #[test]
    fn rejects_bool_main_return() {
        let m = parse("def main() -> bool:\n    return True\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`main` must return"));
    }

    #[test]
    fn lowers_simple_if_else() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if a < 0:\n        return -a\n    else:\n        return a\n",
        ))
        .unwrap();
        match &p.main().body[0] {
            Stmt::If { cond, .. } => assert_eq!(cond.ty, Type::Bool),
            _ => panic!("expected If"),
        }
    }

    #[test]
    fn rejects_break_outside_loop() {
        let m = parse("def main() -> int:\n    break\n    return 0\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`break` outside"));
    }
}
