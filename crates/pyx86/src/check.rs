use anyhow::{bail, Result};
use rustpython_parser::ast;

use crate::parser::Module;

/// The information codegen needs from a v0.1 program: just the
/// integer literal returned by `main`.
#[derive(Debug)]
pub struct Program {
    pub return_value: i64,
}

/// Validate that the module is in the v0.1 supported subset, which is
/// exactly:
///
/// ```text
/// def main() -> int:
///     return <int literal>
/// ```
///
/// Anything else produces an `unsupported_feature` error so future
/// slices can grow this match arm by arm without ever silently
/// falling back to dynamic semantics.
pub fn extract_v0_1(module: &Module) -> Result<Program> {
    if module.body.len() != 1 {
        bail!(
            "unsupported_feature: v0.1 expects exactly one top-level definition, found {}",
            module.body.len()
        );
    }
    let func = match &module.body[0] {
        ast::Stmt::FunctionDef(f) => f,
        other => bail!(
            "unsupported_feature: v0.1 expects `def main()` at the top level, found {:?}",
            kind_name(other)
        ),
    };

    if func.name.as_str() != "main" {
        bail!(
            "unsupported_feature: v0.1 expects the top-level function to be `main`, found `{}`",
            func.name
        );
    }
    if !func.args.args.is_empty()
        || !func.args.posonlyargs.is_empty()
        || !func.args.kwonlyargs.is_empty()
        || func.args.vararg.is_some()
        || func.args.kwarg.is_some()
    {
        bail!("unsupported_feature: v0.1 `main()` must take no arguments");
    }
    if !func.decorator_list.is_empty() {
        bail!("unsupported_feature: decorators are not supported in v0.1");
    }

    match func.returns.as_deref() {
        Some(ast::Expr::Name(n)) if n.id.as_str() == "int" => {}
        Some(_) => bail!("unsupported_feature: v0.1 only supports `-> int` return annotation"),
        None => bail!("unsupported_feature: v0.1 requires a return annotation `-> int`"),
    }

    if func.body.len() != 1 {
        bail!(
            "unsupported_feature: v0.1 `main()` body must be a single `return` statement, found {} stmts",
            func.body.len()
        );
    }
    let ret = match &func.body[0] {
        ast::Stmt::Return(r) => r,
        other => bail!(
            "unsupported_feature: v0.1 `main()` body must be `return <int literal>`, found {:?}",
            kind_name(other)
        ),
    };

    let value_expr = match ret.value.as_deref() {
        Some(e) => e,
        None => bail!("unsupported_feature: v0.1 `main()` must return a value"),
    };
    let int_const = match value_expr {
        ast::Expr::Constant(c) => &c.value,
        _ => bail!("unsupported_feature: v0.1 only supports returning an integer literal"),
    };
    let big = match int_const {
        ast::Constant::Int(i) => i,
        _ => bail!("unsupported_feature: v0.1 only supports returning an integer literal"),
    };

    let value: i64 = big.try_into().map_err(|_| {
        anyhow::anyhow!("unsupported_feature: integer literal does not fit in i64")
    })?;
    Ok(Program { return_value: value })
}

fn kind_name(s: &ast::Stmt) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::path::PathBuf;

    fn parse(src: &str) -> Module {
        parser::parse(src, &PathBuf::from("t.py")).unwrap()
    }

    #[test]
    fn accepts_v0_1_program() {
        let m = parse("def main() -> int:\n    return 42\n");
        let p = extract_v0_1(&m).unwrap();
        assert_eq!(p.return_value, 42);
    }

    #[test]
    fn accepts_negative_literal_via_unary_minus_is_rejected() {
        // -42 parses as UnaryOp(USub, Constant(42)), not Constant(-42).
        // v0.1 only supports a bare int literal; unary minus comes in v0.2.
        let m = parse("def main() -> int:\n    return -42\n");
        let err = extract_v0_1(&m).unwrap_err();
        assert!(format!("{}", err).contains("unsupported_feature"));
    }

    #[test]
    fn rejects_missing_return_annotation() {
        let m = parse("def main():\n    return 0\n");
        let err = extract_v0_1(&m).unwrap_err();
        assert!(format!("{}", err).contains("return annotation"));
    }

    #[test]
    fn rejects_non_main_function() {
        let m = parse("def foo() -> int:\n    return 1\n");
        let err = extract_v0_1(&m).unwrap_err();
        assert!(format!("{}", err).contains("`main`"));
    }
}
