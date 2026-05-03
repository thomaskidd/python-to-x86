use std::collections::HashSet;

use anyhow::{anyhow, bail, Result};
use rustpython_parser::ast;

use crate::hir::{BinOp, Expr, Function, Param, Program, Type, UnaryOp};
use crate::parser::Module;

const MAX_PARAMS: usize = 16;

/// Validate that the module is in the supported subset and lower it
/// to HIR. v0.3 supports:
///
/// ```text
/// def main(<param>: int, …) -> int:
///     return <expr>
/// ```
///
/// where `<expr>` is built from int literals, parameter references,
/// binary `+ - * // %`, unary `+x/-x`, and parens.
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
    // `defaults()` is a method on Arguments in rustpython-ast 0.4 that
    // returns an iterator over the default expressions. Any default at
    // all is rejected.
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
    let mut seen = HashSet::new();
    for arg in &func.args.args {
        let name = arg.def.arg.as_str().to_string();
        if !seen.insert(name.clone()) {
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

    if func.body.len() != 1 {
        bail!(
            "unsupported_feature: `main()` body must be a single `return` statement, found {} stmts",
            func.body.len()
        );
    }
    let ret = match &func.body[0] {
        ast::Stmt::Return(r) => r,
        other => bail!(
            "unsupported_feature: `main()` body must be `return <expr>`, found {}",
            stmt_kind_name(other)
        ),
    };
    let value_expr = ret
        .value
        .as_deref()
        .ok_or_else(|| anyhow!("unsupported_feature: `main()` must return a value"))?;

    let param_names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();
    let body = lower_expr(value_expr, &param_names)?;

    Ok(Program {
        main: Function {
            name: "main".to_string(),
            params,
            return_ty,
            body,
        },
    })
}

fn parse_type_annotation(ann: Option<&ast::Expr>) -> Option<Type> {
    match ann? {
        ast::Expr::Name(n) if n.id.as_str() == "int" => Some(Type::I64),
        _ => None,
    }
}

fn lower_expr(e: &ast::Expr, params: &HashSet<&str>) -> Result<Expr> {
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
            if params.contains(name) {
                Ok(Expr::Param(name.to_string()))
            } else {
                bail!(
                    "unsupported_feature: name `{}` is not a parameter (locals not supported until v0.4)",
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
                lhs: Box::new(lower_expr(&b.left, params)?),
                rhs: Box::new(lower_expr(&b.right, params)?),
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
                operand: Box::new(lower_expr(&u.operand, params)?),
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
    fn lowers_no_param_main() {
        let m = parse("def main() -> int:\n    return 42\n");
        let p = lower(&m).unwrap();
        assert_eq!(p.main.params.len(), 0);
        assert!(matches!(p.main.body, Expr::ConstI64(42)));
    }

    #[test]
    fn lowers_two_param_main() {
        let m = parse("def main(a: int, b: int) -> int:\n    return a + b\n");
        let p = lower(&m).unwrap();
        assert_eq!(p.main.params.len(), 2);
        assert_eq!(p.main.params[0].name, "a");
        assert_eq!(p.main.params[1].name, "b");
        match &p.main.body {
            Expr::BinOp { op: BinOp::Add, lhs, rhs } => {
                assert!(matches!(**lhs, Expr::Param(ref n) if n == "a"));
                assert!(matches!(**rhs, Expr::Param(ref n) if n == "b"));
            }
            other => panic!("expected Add of params, got {:?}", other),
        }
    }

    #[test]
    fn rejects_unannotated_param() {
        let m = parse("def main(a) -> int:\n    return a\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("annotated"));
    }

    #[test]
    fn rejects_non_int_param_annotation() {
        let m = parse("def main(a: str) -> int:\n    return 0\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("annotated"));
    }

    #[test]
    fn rejects_unknown_name_reference() {
        let m = parse("def main(a: int) -> int:\n    return a + b\n");
        let err = lower(&m).unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("`b`") && msg.contains("not a parameter"));
    }

    #[test]
    fn rejects_default_arguments() {
        let m = parse("def main(a: int = 0) -> int:\n    return a\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("default"));
    }

    #[test]
    fn rejects_too_many_params() {
        let names: Vec<String> = (0..MAX_PARAMS + 1).map(|i| format!("p{}: int", i)).collect();
        let src = format!("def main({}) -> int:\n    return 0\n", names.join(", "));
        let m = parse(&src);
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("at most"));
    }

    #[test]
    fn rejects_missing_return_annotation() {
        let m = parse("def main(a: int):\n    return a\n");
        let err = lower(&m).unwrap_err();
        assert!(format!("{}", err).contains("return annotation"));
    }
}
