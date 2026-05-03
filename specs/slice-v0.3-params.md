# Spec: slice v0.3 — function parameters

> Status: in progress.

## What v0.3 adds

`main()` may take typed `int` parameters and reference them in its return expression. Subset:

```python
def main(a: int, b: int) -> int:
    return a * b - 1
```

- Each parameter must have an `int` annotation (no inference yet — that's v0.4 onward).
- Parameter names appear in `Expr::Param` HIR nodes; the body is still a single `return <expr>`.
- All parameters are i64.
- 0 to 16 parameters supported. (Arbitrary cap; raise later if anything needs it.)

## What v0.3 does **not** add

- Local variable assignment (no `x = ...`)
- Multiple statements in body (still single `return`)
- Default arguments
- Keyword-only / positional-only arguments
- Type inference (annotations are required and must be `int`)
- Other types — float, bool, str, list etc.

## Argv encoding

The compiled binary takes parameter values as positional argv strings:

```
$ ./add_two_ints 3 4
7
```

The wrapper (`@main`) calls `atoll(argv[i+1])` for each parameter, then invokes `py_main` with those values. Parameter parsing failures (non-numeric input, missing args) are not handled in v0.3 — argv is trusted to come from the bench, which always passes well-formed integers.

## Architectural changes

### HIR additions

```rust
pub enum Type { I64 }                  // only one type so far

pub struct Param {
    pub name: String,
    pub ty: Type,
}

pub struct Function {
    pub name: String,                  // always "main" in v0.3
    pub params: Vec<Param>,
    pub return_ty: Type,
    pub body: Expr,                    // single return expression
}

pub enum Expr {
    ConstI64(i64),
    Param(String),                     // NEW: reference a parameter by name
    BinOp { … },
    UnaryOp { … },
}

pub struct Program {
    pub main: Function,
}
```

### Check

`lower(module) -> hir::Program`:
- Validate the function has at most 16 typed `int` params.
- Build `Vec<Param>` from `func.args.args`.
- During expression lowering, when seeing `ast::Expr::Name(n)`, look up `n.id` in the param set; if found, emit `Expr::Param(name)`; if not, error with `unsupported_feature: name '{n}' is not a parameter (locals not supported until v0.4)`.

### Codegen

`py_main` becomes `define i64 @py_main(i64 %a, i64 %b, …)`. Inside the body, `Expr::Param(name)` lowers to the operand `%<name>`.

Wrapper `@main` becomes:

```llvm
declare i64 @atoll(i8*)

define i32 @main(i32 %argc, i8** %argv) {
entry:
  ; for each param i (0-indexed), parse argv[i+1]
  %p0_slot = getelementptr inbounds i8*, i8** %argv, i64 1
  %p0_str  = load i8*, i8** %p0_slot
  %a = call i64 @atoll(i8* %p0_str)
  ; …repeat for each param…
  %r = call i64 @py_main(i64 %a, …)
  %fmt = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0
  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)
  ret i32 0
}
```

When there are no parameters (the v0.1/v0.2 case) the wrapper falls back to the existing 0-arg form.

## Bench changes

The bench now reads `strategy.toml` and generates inputs.

`strategy.toml`:
```toml
[[arg]]
type = "i64"
range = [-1_000_000, 1_000_000]
```

Bench changes:
- Parse `strategy.toml` into `Vec<ArgStrategy>`.
- For each iteration (count from `tier.toml.iter_at` or the per-tier default), generate one random value per arg via `rand`.
- For CPython: format args as Python int literals into the `print(repr(main(<a>, <b>)))` snippet.
- For the compiled ELF: pass args as positional argv strings.
- Compare stdout + exit code.

No proptest / shrinking yet — for v0.3 the bench keeps the failing input as-is. Shrinking lands when the false-positive rate justifies it.

The "unsupported" stub for strategy.toml is removed.

## Test programs added

| Test | Purpose |
|---|---|
| `add_two_ints` | `def main(a: int, b: int) -> int: return a + b` — basic 2-param happy path |
| `single_param_identity` | `return a` — checks parameter passthrough on its own |
| `params_arith_mixed` | combines params with literals and floor-div, exercises codegen interaction |

All are tier 1, with `iter_at.tier1 = 5` so each runs 5 random inputs. Tier 2 tests can grow this to 100+ when promotion needs more confidence.

## Files changed from v0.2

- `crates/pyx86/src/hir.rs` — add `Type`, `Param`, `Function`; `Program` now holds a `Function`; `Expr::Param` variant added.
- `crates/pyx86/src/check.rs` — handle params, lower `Name` to `Expr::Param`.
- `crates/pyx86/src/codegen.rs` — emit `py_main` with params, emit argv-parsing wrapper.
- `crates/pyx86_bench/src/main.rs` — parse `strategy.toml`, generate inputs, format args for both runners.
- `crates/pyx86_bench/Cargo.toml` — add `rand`.
- `tests/correctness/{add_two_ints, single_param_identity, params_arith_mixed}/` — new programs + strategies.
- `specs/check.md`, `specs/codegen-llvm.md`, `specs/test-bench.md` — updated to match the new code.
