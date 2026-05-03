use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use rustpython_parser::ast;

use crate::hir::{BinOp, Expr, Function, Param, Program, Stmt, Type, UnaryOp};
use crate::parser::Module;

const MAX_PARAMS: usize = 16;

/// Validate that the module is in the supported subset and lower it
/// to HIR. v0.4 supports:
///
/// ```text
/// def main(<param>: int, …) -> int:
///     <name> [: int] = <expr>      # zero or more local bindings
///     …
///     return <expr>                # required, must be last stmt
/// ```
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
        bail!("unsupported_feature: `main()` body must end with a `return` statement");
    }

    let mut body: Vec<Stmt> = Vec::with_capacity(func.body.len());
    let last_idx = func.body.len() - 1;
    for (i, stmt) in func.body.iter().enumerate() {
        let is_last = i == last_idx;
        match stmt {
            ast::Stmt::Assign(a) => {
                if is_last {
                    bail!(
                        "unsupported_feature: function body must end with `return`, found assignment as last statement"
                    );
                }
                let name = parse_assign_target(&a.targets)?;
                let value = lower_expr(&a.value, &scope)?;
                scope.insert(name.clone());
                body.push(Stmt::Let { name, value });
            }
            ast::Stmt::AnnAssign(a) => {
                if is_last {
                    bail!(
                        "unsupported_feature: function body must end with `return`, found assignment as last statement"
                    );
                }
                let name = match a.target.as_ref() {
                    ast::Expr::Name(n) => n.id.as_str().to_string(),
                    _ => bail!("unsupported_feature: only simple-name targets are supported in assignments"),
                };
                if !a.simple {
                    // `(x): int = ...` — Python allows this, we don't.
                    bail!("unsupported_feature: parenthesised annotation targets are not supported");
                }
                if parse_type_annotation(Some(&a.annotation)).is_none() {
                    bail!(
                        "unsupported_feature: only `: int` annotations are supported on locals, on `{}`",
                        name
                    );
                }
                let value_expr = a
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow!("unsupported_feature: bare annotation `{}: int` (no value) is not supported", name))?;
                let value = lower_expr(value_expr, &scope)?;
                scope.insert(name.clone());
                body.push(Stmt::Let { name, value });
            }
            ast::Stmt::Return(r) => {
                if !is_last {
                    bail!(
                        "unsupported_feature: statements after `return` are not allowed (early return needs control flow, lands in v0.5)"
                    );
                }
                let value_expr = r
                    .value
                    .as_deref()
                    .ok_or_else(|| anyhow!("unsupported_feature: `return` must have a value"))?;
                let value = lower_expr(value_expr, &scope)?;
                body.push(Stmt::Return { value });
            }
            other => bail!(
                "unsupported_feature: statement `{}` is not supported in v0.4",
                stmt_kind_name(other)
            ),
        }
    }

    if !matches!(body.last(), Some(Stmt::Return { .. })) {
        bail!(
            "unsupported_feature: `main()` body must end with a `return` statement"
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
            _ => bail!("unsupported_feature: only integer literals are supported"),
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
                ast::UnaryOp::Not => bail!("unsupported_feature: boolean `not` is not yet supported"),
                ast::UnaryOp::Invert => bail!("unsupported_feature: bitwise `~` is not yet supported"),
            };
            Ok(Expr::UnaryOp {
                op,
                operand: Box::new(lower_expr(&u.operand, scope)?),
            })
        }
        other => bail!(
            "unsupported_feature: expression form `{}` is not supported",
            expr_kind_name(other)
        ),
    }
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
    fn lowers_no_locals() {
        let p = lower(&parse("def main() -> int:\n    return 42\n")).unwrap();
        assert_eq!(p.main.body.len(), 1);
        assert!(matches!(p.main.body[0], Stmt::Return { .. }));
    }

    #[test]
    fn lowers_single_local() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    x = a + 1\n    return x\n",
        ))
        .unwrap();
        assert_eq!(p.main.body.len(), 2);
        match &p.main.body[0] {
            Stmt::Let { name, .. } => assert_eq!(name, "x"),
            _ => panic!("expected Let"),
        }
        match &p.main.body[1] {
            Stmt::Return { value } => assert!(matches!(value, Expr::Var(n) if n == "x")),
            _ => panic!("expected Return"),
        }
    }

    #[test]
    fn lowers_annotated_assignment() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    x: int = a + 1\n    return x\n",
        ))
        .unwrap();
        assert!(matches!(p.main.body[0], Stmt::Let { ref name, .. } if name == "x"));
    }

    #[test]
    fn allows_reassignment() {
        let p = lower(&parse(
            "def main(a: int) -> int:\n    x = a\n    x = x + 1\n    return x\n",
        ))
        .unwrap();
        // Two Let statements, both binding `x`.
        assert_eq!(p.main.body.len(), 3);
        assert!(matches!(p.main.body[0], Stmt::Let { ref name, .. } if name == "x"));
        assert!(matches!(p.main.body[1], Stmt::Let { ref name, .. } if name == "x"));
        assert!(matches!(p.main.body[2], Stmt::Return { .. }));
    }

    #[test]
    fn rejects_non_int_local_annotation() {
        let m = parse("def main(a: int) -> int:\n    x: str = a\n    return a\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`: int`"));
    }

    #[test]
    fn rejects_unbound_name() {
        let m = parse("def main(a: int) -> int:\n    return a + b\n");
        let err = lower(&m).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("`b`"));
        assert!(msg.contains("not in scope"));
    }

    #[test]
    fn rejects_use_before_assignment() {
        let m = parse("def main(a: int) -> int:\n    y = x + 1\n    x = a\n    return y\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`x`"));
    }

    #[test]
    fn rejects_return_not_last() {
        let m = parse("def main(a: int) -> int:\n    return a\n    x = 1\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("after `return`"));
    }

    #[test]
    fn rejects_missing_return() {
        let m = parse("def main(a: int) -> int:\n    x = a\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`return`"));
    }

    #[test]
    fn rejects_chained_assignment() {
        let m = parse("def main(a: int) -> int:\n    x = y = a\n    return x\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("chained"));
    }

    #[test]
    fn rejects_aug_assign() {
        let m = parse("def main(a: int) -> int:\n    a += 1\n    return a\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("AugAssign"));
    }

    #[test]
    fn rejects_tuple_unpacking() {
        let m = parse("def main() -> int:\n    a, b = 1, 2\n    return a + b\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("unpacking"));
    }
}
