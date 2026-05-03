use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use rustpython_parser::ast;

use crate::hir::{BinOp, CmpOp, Expr, Function, Param, Program, Stmt, Type, UnaryOp};
use crate::parser::Module;

const MAX_PARAMS: usize = 16;

/// Lower the parsed module into a `hir::Program`. v0.5 supports:
///
/// - `def main(<param>: int, …) -> int:` (up to 16 typed-int params)
/// - body composed of:
///   - `<name> [: int] = <expr>`     (assignment, plain or annotated)
///   - `if <cond>: <body> [elif …]* [else: <body>]`
///   - `return <expr>`
/// - expressions:
///   - int literals
///   - variable references (param or local)
///   - binary `+ - * // %`
///   - unary `+x`, `-x`, `not x`
///   - comparison `< <= > >= == !=` (chained allowed)
///
/// Anything else produces an `unsupported_feature` error.
pub fn lower(module: &Module) -> Result<Program> {
    if module.body.len() != 1 {
        bail!(
            "unsupported_feature: expected exactly one top-level definition, found {}",
            module.body.len()
        );
    }
    let func = match &module.body[0] {
        ast::Stmt::FunctionDef(f) => f,
        other => bail!(
            "unsupported_feature: expected `def main(...)` at the top level, found {}",
            stmt_kind_name(other)
        ),
    };

    if func.name.as_str() != "main" {
        bail!(
            "unsupported_feature: top-level function must be `main`, found `{}`",
            func.name
        );
    }
    if !func.args.posonlyargs.is_empty() || !func.args.kwonlyargs.is_empty() {
        bail!("unsupported_feature: positional-only and keyword-only parameters are not supported");
    }
    if func.args.vararg.is_some() || func.args.kwarg.is_some() {
        bail!("unsupported_feature: *args / **kwargs are not supported");
    }
    if func.args.defaults().next().is_some() {
        bail!("unsupported_feature: default arguments are not supported");
    }
    if !func.decorator_list.is_empty() {
        bail!("unsupported_feature: decorators are not supported");
    }
    if func.args.args.len() > MAX_PARAMS {
        bail!(
            "unsupported_feature: at most {} parameters supported, found {}",
            MAX_PARAMS,
            func.args.args.len()
        );
    }

    let mut params = Vec::with_capacity(func.args.args.len());
    let mut scope: HashSet<String> = HashSet::new();
    for arg in &func.args.args {
        let name = arg.def.arg.as_str().to_string();
        if !scope.insert(name.clone()) {
            bail!("unsupported_feature: duplicate parameter name `{}`", name);
        }
        let ty = parse_type_annotation(arg.def.annotation.as_deref())
            .ok_or_else(|| anyhow!("unsupported_feature: parameter `{}` must be annotated `: int`", name))?;
        params.push(Param { name, ty });
    }

    let return_ty = match parse_type_annotation(func.returns.as_deref()) {
        Some(ty) => ty,
        None => bail!("unsupported_feature: requires a return annotation `-> int`"),
    };

    if func.body.is_empty() {
        bail!("unsupported_feature: function body is empty");
    }

    let body = lower_block(&func.body, &mut scope)?;

    if !block_always_returns(&body) {
        bail!(
            "unsupported_feature: not all paths return a value (the function body, or both branches of every trailing `if`, must end with `return`)"
        );
    }

    Ok(Program {
        main: Function {
            name: "main".to_string(),
            params,
            return_ty,
            body,
        },
    })
}

/// Conservative path-coverage check. Returns true iff:
/// - the block ends with a `Return`, OR
/// - the block ends with an `If` whose then_body and else_body both
///   recursively cover, AND the else_body is non-empty.
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

fn lower_block(stmts: &[ast::Stmt], scope: &mut HashSet<String>) -> Result<Vec<Stmt>> {
    let mut out = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        match stmt {
            ast::Stmt::Assign(a) => {
                let name = parse_assign_target(&a.targets)?;
                let value = lower_expr(&a.value, scope)?;
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
                let value = lower_expr(value_expr, scope)?;
                scope.insert(name.clone());
                out.push(Stmt::Let { name, value });
            }
            ast::Stmt::Return(r) => {
                let value_expr = r
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow!("unsupported_feature: `return` must have a value"))?;
                let value = lower_expr(value_expr, scope)?;
                out.push(Stmt::Return { value });
            }
            ast::Stmt::If(if_stmt) => {
                let cond = lower_expr(&if_stmt.test, scope)?;
                // Branch bodies see and may extend the same scope as
                // the surrounding block. (Python doesn't have block
                // scope; locals introduced in a branch are accessible
                // after the branch — though using them when the
                // branch wasn't taken is a runtime UnboundLocalError
                // in CPython. v0.5 uses alloca slots that hold the
                // last-stored value or undef; we accept this as a
                // pragmatic deviation.)
                let then_body = lower_block(&if_stmt.body, scope)?;
                let else_body = lower_block(&if_stmt.orelse, scope)?;
                out.push(Stmt::If { cond, then_body, else_body });
            }
            ast::Stmt::Pass(_) => {
                // pass is a no-op; lower it as nothing.
            }
            other => bail!(
                "unsupported_feature: statement `{}` is not supported in v0.5",
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

fn lower_expr(e: &ast::Expr, scope: &HashSet<String>) -> Result<Expr> {
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
                ast::Operator::Pow => bail!("unsupported_feature: `**` (exponentiation) is not yet supported"),
                ast::Operator::MatMult => bail!("unsupported_feature: `@` (matmul) is not supported"),
                ast::Operator::LShift | ast::Operator::RShift => {
                    bail!("unsupported_feature: bit-shift operators are not yet supported")
                }
                ast::Operator::BitOr | ast::Operator::BitXor | ast::Operator::BitAnd => {
                    bail!("unsupported_feature: bitwise operators are not yet supported")
                }
            };
            Ok(Expr::BinOp {
                op,
                lhs: Box::new(lower_expr(&b.left, scope)?),
                rhs: Box::new(lower_expr(&b.right, scope)?),
            })
        }
        ast::Expr::UnaryOp(u) => {
            let op = match u.op {
                ast::UnaryOp::USub => UnaryOp::Neg,
                ast::UnaryOp::UAdd => UnaryOp::Pos,
                ast::UnaryOp::Not => {
                    return Ok(Expr::Not(Box::new(lower_expr(&u.operand, scope)?)));
                }
                ast::UnaryOp::Invert => bail!("unsupported_feature: bitwise `~` is not yet supported"),
            };
            Ok(Expr::UnaryOp {
                op,
                operand: Box::new(lower_expr(&u.operand, scope)?),
            })
        }
        ast::Expr::Compare(c) => {
            // Python AST: left + ops[] + comparators[]. ops.len() == comparators.len().
            let first = lower_expr(&c.left, scope)?;
            let rest: Result<Vec<(CmpOp, Expr)>> = c
                .ops
                .iter()
                .zip(c.comparators.iter())
                .map(|(op, e)| Ok((convert_cmp_op(op)?, lower_expr(e, scope)?)))
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
        ast::Expr::BoolOp(_) => bail!(
            "unsupported_feature: `and` / `or` are not yet supported in v0.5 (use nested `if` for now; they land in v0.6)"
        ),
        other => bail!(
            "unsupported_feature: expression form `{}` is not supported",
            expr_kind_name(other)
        ),
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
    fn lowers_simple_if_else() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    if a < 0:\n        return -a\n    else:\n        return a\n",
        ))
        .unwrap();
        match &p.main.body[0] {
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
        match &p.main.body[0] {
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
        match &p.main.body[0] {
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
        match &p.main.body[0] {
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
        match &p.main.body[0] {
            Stmt::If { cond: Expr::Not(_), .. } => {}
            _ => panic!("expected Not in If condition"),
        }
    }

    #[test]
    fn rejects_and_or() {
        let m = parse(
            "def main(a: int, b: int) -> int:\n    if a > 0 and b > 0:\n        return 1\n    else:\n        return 0\n",
        );
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`and` / `or`"));
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
}
