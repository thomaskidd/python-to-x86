use crate::check::Program;

/// Emit LLVM IR text for a v0.1 program (a `main()` that returns a
/// single int literal). The wrapper `main` symbol calls the user's
/// `py_main`, prints the result via `printf("%ld\n", ...)`, and
/// returns 0.
pub fn emit_ll(prog: &Program, source_basename: &str) -> String {
    let basename = sanitize_module_id(source_basename);
    // Typed pointers (i8*) rather than opaque pointers (ptr) so we
    // work on LLVM 10+ — opaque pointers are LLVM 14+ only.
    format!(
        "; ModuleID = 'pyx86_{name}'
target triple = \"x86_64-unknown-linux-gnu\"

declare i32 @printf(i8*, ...)

@.fmt_i64 = private unnamed_addr constant [5 x i8] c\"%ld\\0A\\00\"

define i64 @py_main() {{
entry:
  ret i64 {value}
}}

define i32 @main() {{
entry:
  %r = call i64 @py_main()
  %fmt = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0
  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)
  ret i32 0
}}
",
        name = basename,
        value = prog.return_value,
    )
}

fn sanitize_module_id(s: &str) -> String {
    // LLVM tolerates a wide range of characters here, but keep it
    // boring so .ll files are readable.
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_expected_ir_for_return_42() {
        let ll = emit_ll(&Program { return_value: 42 }, "return_constant");
        assert!(ll.contains("ret i64 42"));
        assert!(ll.contains("define i64 @py_main()"));
        assert!(ll.contains("define i32 @main()"));
        assert!(ll.contains("@.fmt_i64"));
        assert!(ll.contains("call i32 (i8*, ...) @printf"));
    }

    #[test]
    fn module_id_is_sanitized() {
        let ll = emit_ll(&Program { return_value: 0 }, "weird-name.with.dots");
        assert!(ll.contains("ModuleID = 'pyx86_weird_name_with_dots'"));
    }
}
