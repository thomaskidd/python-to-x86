# Spec: parser

## Responsibility

Turn a `.py` source string into a Python AST suitable for downstream stages, and reject anything outside our supported subset with a clear `unsupported_feature` error.

We do not write our own Python parser. We use the [`rustpython-parser`](https://crates.io/crates/rustpython-parser) crate, which produces a faithful Python AST in pure Rust. Our parser module is a thin wrapper around it that:

1. Invokes `rustpython_parser::parse_program`.
2. Wraps `rustpython-parser` errors into our own structured error type with file/line/column info.
3. Returns a `Module` value (alias for the relevant `rustpython-ast` type).

## Inputs / Outputs

- **Input**: `(source: &str, source_path: &Path)`
- **Output**: `Result<rustpython_ast::ModModule, ParseError>`

`ParseError` carries:
- A one-line summary
- File path, line, column
- Optional source-line excerpt for pretty-printing

## What this module does **not** do

- Type inference — that's `infer.rs`.
- Subset validation — that's `check.rs` / `infer.rs`. The parser accepts any syntactically valid Python; rejection of unsupported constructs happens later, where the error can describe *why* (e.g. "decorator @memoize is not supported").
- Macro expansion, preprocessing, name resolution.

## Error reporting

Parser errors are formatted as:

```
pyx86 error: parse: <summary>
 --> <file>:<line>:<col>
```

Internal layout: a `ParseError { message: String, location: SourceLocation }`. The driver formats it for display.

## Dependencies

- `rustpython-parser = "0.4"`
- `rustpython-ast = "0.4"` (re-exported types used by the rest of the compiler)

## Test surface

Unit tests in `parser.rs`:
- `parses_trivial_program` — the v0.1 input parses without error.
- `reports_syntax_error_with_location` — a deliberately broken file produces an error with line/column.
- (later, as features land) — additional test cases per AST node we exercise.

There is no integration test for the parser in isolation; integration is covered by the test bench compiling real `.py` programs.
