use anyhow::{anyhow, bail, Result};
use rustpython_parser::ast;

use crate::hir::{BinOp, Expr, Program, UnaryOp};
use crate::parser::Module;

/// Validate that the module is in the v0.2 supported subset and lower
/// it to HIR. v0.2 supports:
///
/// ```text
/// def main() -> int:
///     return <int-expr>
/// ```
///
/// where `<int-expr>` is built from:
/// - `int` literals
/// - binary `+ - * // %`
/// - unary `+x`, `-x`
/// - parenthesisation
///
/// Anything else produces an `unsupported_feature` error so future
/// slices grow this match arm by arm — never silently falling back
/// to dynamic semantics.
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
            "unsupported_feature: expected `def main()` at the top level, found {}",
            stmt_kind_name(other)
        ),
    };

    if func.name.as_str() != "main" {
        bail!(
            "unsupported_feature: top-level function must be `main`, found `{}`",
            func.name
        );
    }
    if !func.args.args.is_empty()
        || !func.args.posonlyargs.is_empty()
        || !func.args.kwonlyargs.is_empty()
        || func.args.vararg.is_some()
        || func.args.kwarg.is_some()
    {
        bail!("unsupported_feature: v0.2 `main()` must take no arguments");
    }
    if !func.decorator_list.is_empty() {
        bail!("unsupported_feature: decorators are not supported");
    }
    match func.returns.as_deref() {
        Some(ast::Expr::Name(n)) if n.id.as_str() == "int" => {}
        Some(_) => bail!("unsupported_feature: v0.2 only supports `-> int` return annotation"),
        None => bail!("unsupported_feature: v0.2 requires a return annotation `-> int`"),
    }

    if func.body.len() != 1 {
        bail!(
            "unsupported_feature: v0.2 `main()` body must be a single `return` statement, found {} stmts",
            func.body.len()
        );
    }
    let ret = match &func.body[0] {
        ast::Stmt::Return(r) => r,
        other => bail!(
            "unsupported_feature: v0.2 `main()` body must be `return <expr>`, found {}",
            stmt_kind_name(other)
        ),
    };

    let value_expr = ret
        .value
        .as_deref()
        .ok_or_else(|| anyhow!("unsupported_feature: v0.2 main() must return a value"))?;
    let lowered = lower_expr(value_expr)?;
    Ok(Program { main_return: lowered })
}

fn lower_expr(e: &ast::Expr) -> Result<Expr> {
    match e {
        ast::Expr::Constant(c) => match &c.value {
            ast::Constant::Int(big) => {
                let v: i64 = big.try_into().map_err(|_| {
                    anyhow!("unsupported_feature: integer literal does not fit in i64")
                })?;
                Ok(Expr::ConstI64(v))
            }
            _ => bail!("unsupported_feature: only integer literals are supported in v0.2"),
        },
        ast::Expr::BinOp(b) => {
            let op = match b.op {
                ast::Operator::Add => BinOp::Add,
                ast::Operator::Sub => BinOp::Sub,
                ast::Operator::Mult => BinOp::Mul,
                ast::Operator::FloorDiv => BinOp::FloorDiv,
                ast::Operator::Mod => BinOp::Mod,
                ast::Operator::Div => bail!("unsupported_feature: `/` (true division) is not in v0.2 — float support is deferred"),
                ast::Operator::Pow => bail!("unsupported_feature: `**` (exponentiation) is not in v0.2"),
                ast::Operator::MatMult => bail!("unsupported_feature: `@` (matmul) is not in v0.2"),
                ast::Operator::LShift | ast::Operator::RShift => {
                    bail!("unsupported_feature: bit-shift operators are not in v0.2")
                }
                ast::Operator::BitOr | ast::Operator::BitXor | ast::Operator::BitAnd => {
                    bail!("unsupported_feature: bitwise operators are not in v0.2")
                }
            };
            Ok(Expr::BinOp {
                op,
                lhs: Box::new(lower_expr(&b.left)?),
                rhs: Box::new(lower_expr(&b.right)?),
            })
        }
        ast::Expr::UnaryOp(u) => {
            let op = match u.op {
                ast::UnaryOp::USub => UnaryOp::Neg,
                ast::UnaryOp::UAdd => UnaryOp::Pos,
                ast::UnaryOp::Not => bail!("unsupported_feature: boolean `not` is not in v0.2"),
                ast::UnaryOp::Invert => {
                    bail!("unsupported_feature: bitwise `~` is not in v0.2")
                }
            };
            Ok(Expr::UnaryOp {
                op,
                operand: Box::new(lower_expr(&u.operand)?),
            })
        }
        other => bail!(
            "unsupported_feature: expression form `{}` is not supported in v0.2",
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

    fn lower_main(src: &str) -> Expr {
        lower(&parse(src)).unwrap().main_return
    }

    #[test]
    fn lowers_int_literal() {
        assert!(matches!(lower_main("def main() -> int:\n    return 42\n"), Expr::ConstI64(42)));
    }

    #[test]
    fn lowers_unary_minus() {
        let e = lower_main("def main() -> int:\n    return -42\n");
        match e {
            Expr::UnaryOp { op: UnaryOp::Neg, operand } => {
                assert!(matches!(*operand, Expr::ConstI64(42)));
            }
            _ => panic!("expected UnaryOp::Neg, got {:?}", e),
        }
    }

    #[test]
    fn lowers_binary_arith() {
        let e = lower_main("def main() -> int:\n    return 1 + 2 * 3\n");
        // 1 + (2 * 3) — Python's BinOp tree groups by precedence
        match e {
            Expr::BinOp { op: BinOp::Add, lhs, rhs } => {
                assert!(matches!(*lhs, Expr::ConstI64(1)));
                assert!(matches!(*rhs, Expr::BinOp { op: BinOp::Mul, .. }));
            }
            _ => panic!("expected Add at top, got {:?}", e),
        }
    }

    #[test]
    fn lowers_floordiv_and_mod() {
        let e = lower_main("def main() -> int:\n    return 100 // 7\n");
        assert!(matches!(e, Expr::BinOp { op: BinOp::FloorDiv, .. }));
        let e = lower_main("def main() -> int:\n    return 100 % 7\n");
        assert!(matches!(e, Expr::BinOp { op: BinOp::Mod, .. }));
    }

    #[test]
    fn rejects_true_division() {
        let m = parse("def main() -> int:\n    return 7 / 2\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("true division"));
    }

    #[test]
    fn rejects_pow() {
        let m = parse("def main() -> int:\n    return 2 ** 8\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("exponentiation"));
    }

    #[test]
    fn rejects_bitwise() {
        let m = parse("def main() -> int:\n    return 1 | 2\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("bitwise"));
    }

    #[test]
    fn rejects_missing_return_annotation() {
        let m = parse("def main():\n    return 0\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("return annotation"));
    }

    #[test]
    fn rejects_non_main_function() {
        let m = parse("def foo() -> int:\n    return 1\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("`main`"));
    }

    #[test]
    fn rejects_variable_reference() {
        let m = parse("def main() -> int:\n    return x\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("Name"));
    }
}
