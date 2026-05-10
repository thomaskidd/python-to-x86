use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::hir::{
    BinOp, BoolOp, CmpOp, Expr, Function, Program, Stmt, Type, TypedExpr, UnaryOp,
};

thread_local! {
    /// Per-emit_ll() collection of string literals encountered during
    /// codegen. Each gets a unique global symbol `@.str.<idx>`. Cleared
    /// at the start of every emit_ll call.
    static STR_LITS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn emit_ll(prog: &Program, source_basename: &str) -> String {
    let basename = sanitize_module_id(source_basename);
    STR_LITS.with(|s| s.borrow_mut().clear());

    let mut function_defs = String::new();
    for func in &prog.functions {
        let mut cg = Codegen::new();
        cg.lower_function(func);
        let _ = writeln!(
            function_defs,
            "define {ret} @py_{name}({sig}) {{\n{body}}}\n",
            ret = llvm_ty(func.return_ty),
            name = func.name,
            sig = format_signature(func),
            body = cg.body,
        );
    }

    // Emit string-literal globals. Each literal is `[N x i8] c"<bytes>\00"`.
    // The struct view used at runtime drops the trailing NUL from len.
    let mut str_globals = String::new();
    STR_LITS.with(|s| {
        for (i, lit) in s.borrow().iter().enumerate() {
            // Encode bytes with hex escape for non-printable, leave printable
            // as-is. We've already restricted literals to ASCII-without-quotes-
            // or-backslashes, so no escape needed in the body.
            let n = lit.len() + 1; // include trailing NUL
            let _ = writeln!(
                str_globals,
                "@.str.{} = private unnamed_addr constant [{n} x i8] c\"{body}\\00\"",
                i,
                n = n,
                body = lit
            );
        }
    });

    let main_fn = prog.main();
    let py_main_call_args = main_fn
        .params
        .iter()
        .map(|p| format!("{} %p_{}", llvm_ty(p.ty), p.name))
        .collect::<Vec<_>>()
        .join(", ");
    let parse_args_block = format_argv_parsing(main_fn);
    let print_block = match main_fn.return_ty {
        Type::I64 => "  %fmt = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_i64, i64 0, i64 0\n  call i32 (i8*, ...) @printf(i8* %fmt, i64 %r)".to_string(),
        Type::F64 => "  call void @pyx86_print_f64(double %r)".to_string(),
        Type::Str => "  %r_len = extractvalue { i64, i8* } %r, 0\n  %r_data = extractvalue { i64, i8* } %r, 1\n  %r_len32 = trunc i64 %r_len to i32\n  %fmt_repr = getelementptr inbounds [8 x i8], [8 x i8]* @.fmt_str_repr, i64 0, i64 0\n  call i32 (i8*, ...) @printf(i8* %fmt_repr, i32 %r_len32, i8* %r_data)".to_string(),
        Type::I8 | Type::I16 | Type::I32 | Type::Bool | Type::Tuple(_) | Type::List(_) | Type::Dict(_) | Type::Class(_) => {
            unreachable!("check rejects this main return type")
        }
    };

    format!(
        "; ModuleID = 'pyx86_{name}'
target triple = \"x86_64-unknown-linux-gnu\"

declare i32 @printf(i8*, ...)
declare i64 @atoll(i8*)
declare double @atof(i8*)
declare double @llvm.pow.f64(double, double)
declare i64 @strlen(i8*)
declare i32 @sprintf(i8*, i8*, ...)
declare i8* @malloc(i64)
declare i8* @realloc(i8*, i64)
declare i32 @memcmp(i8*, i8*, i64)
declare void @llvm.memcpy.p0i8.p0i8.i64(i8*, i8*, i64, i1)
declare void @llvm.memset.p0i8.i64(i8*, i8, i64, i1)
declare double @llvm.sqrt.f64(double)
declare double @llvm.sin.f64(double)
declare double @llvm.cos.f64(double)
declare double @llvm.exp.f64(double)
declare double @llvm.log.f64(double)
declare double @llvm.floor.f64(double)
declare double @llvm.ceil.f64(double)
declare double @llvm.fabs.f64(double)
declare double @tan(double)

@.fmt_i64 = private unnamed_addr constant [5 x i8] c\"%ld\\0A\\00\"
@.fmt_f64_g = private unnamed_addr constant [6 x i8] c\"%.17g\\00\"
@.fmt_str_nl = private unnamed_addr constant [4 x i8] c\"%s\\0A\\00\"
@.fmt_i64_buf = private unnamed_addr constant [4 x i8] c\"%ld\\00\"
@.s_true = private unnamed_addr constant [5 x i8] c\"True\\00\"
@.s_false = private unnamed_addr constant [6 x i8] c\"False\\00\"
@.fmt_str_repr = private unnamed_addr constant [8 x i8] c\"'%.*s'\\0A\\00\"

{str_globals}
{runtime}{defs}define i32 @main(i32 %argc, i8** %argv) {{
entry:
{parse}  %r = call {ret_ty} @py_main({call_args})
{print_block}
  ret i32 0
}}
",
        name = basename,
        ret_ty = llvm_ty(main_fn.return_ty),
        runtime = RUNTIME_HELPERS,
        defs = function_defs,
        parse = parse_args_block,
        call_args = py_main_call_args,
        print_block = print_block,
        str_globals = str_globals,
    )
}

const RUNTIME_HELPERS: &str = "\
; pyx86_print_f64 — best-effort Python-style repr of a double, plus newline.
;   - Prints %.17g into a 64-byte buffer.
;   - Scans for '.' / 'e' / 'E' / 'n' (nan) / 'i' (inf) / 'N' / 'I'.
;   - If none present (i.e. integer-valued shortest form), append '.0'.
;   - Print buffer with trailing newline via printf.
; Matches CPython's repr exactly for the common cases (integer-valued
; floats, sums/products that produce 17-digit shortest forms).
; Diverges for values whose shortest round-trip uses fewer than 17
; digits (e.g. 0.1 prints as 0.10000000000000001 vs Python's 0.1).
; Test programs target the matching cases.
define internal void @pyx86_print_f64(double %x) {
entry:
  %buf = alloca [64 x i8]
  %buf_p = getelementptr inbounds [64 x i8], [64 x i8]* %buf, i64 0, i64 0
  %fmt_p = getelementptr inbounds [6 x i8], [6 x i8]* @.fmt_f64_g, i64 0, i64 0
  %_w = call i32 (i8*, i8*, ...) @sprintf(i8* %buf_p, i8* %fmt_p, double %x)
  ; Scan for any of '.', 'e', 'E', 'n', 'i', 'N', 'I' in buf.
  %has_special = call i1 @pyx86_has_decimal_or_special(i8* %buf_p)
  br i1 %has_special, label %print, label %append_dot_zero
append_dot_zero:
  %len = call i64 @strlen(i8* %buf_p)
  %end0 = getelementptr i8, i8* %buf_p, i64 %len
  store i8 46, i8* %end0          ; '.'
  %end1 = getelementptr i8, i8* %end0, i64 1
  store i8 48, i8* %end1          ; '0'
  %end2 = getelementptr i8, i8* %end1, i64 1
  store i8 0, i8* %end2           ; '\\0'
  br label %print
print:
  %out_fmt = getelementptr inbounds [4 x i8], [4 x i8]* @.fmt_str_nl, i64 0, i64 0
  %_p = call i32 (i8*, ...) @printf(i8* %out_fmt, i8* %buf_p)
  ret void
}

define internal i1 @pyx86_has_decimal_or_special(i8* %s) {
entry:
  %i.addr = alloca i64
  store i64 0, i64* %i.addr
  br label %loop_header
loop_header:
  %i = load i64, i64* %i.addr
  %p = getelementptr i8, i8* %s, i64 %i
  %c = load i8, i8* %p
  %is_zero = icmp eq i8 %c, 0
  br i1 %is_zero, label %not_found, label %check
check:
  %is_dot = icmp eq i8 %c, 46
  %is_e   = icmp eq i8 %c, 101
  %is_E   = icmp eq i8 %c, 69
  %is_n   = icmp eq i8 %c, 110
  %is_i   = icmp eq i8 %c, 105
  %is_N   = icmp eq i8 %c, 78
  %is_I   = icmp eq i8 %c, 73
  %t1 = or i1 %is_dot, %is_e
  %t2 = or i1 %t1, %is_E
  %t3 = or i1 %t2, %is_n
  %t4 = or i1 %t3, %is_i
  %t5 = or i1 %t4, %is_N
  %found = or i1 %t5, %is_I
  br i1 %found, label %yes, label %step
step:
  %i_new = add i64 %i, 1
  store i64 %i_new, i64* %i.addr
  br label %loop_header
yes:
  ret i1 true
not_found:
  ret i1 false
}

; ---------------------------------------------------------------------
; pyx86_dict_i64_lookup / pyx86_dict_i64_insert / pyx86_dict_i64_has
; Hash-table runtime for dict[i64, i64]. Open addressing with linear
; probing; cap is always a power of two; slot layout
;   { i64 key, i64 value, i64 occupied (0 or 1) }
; (24 bytes per slot — wasteful but simple). Hash of i64 key is
; `key & (cap - 1)`.
; ---------------------------------------------------------------------

define internal i64 @pyx86_dict_i64_lookup(i8* %table_raw, i64 %key) {
entry:
  %tp = bitcast i8* %table_raw to { i64, i64, i8* }*
  %cap_p = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 1
  %cap = load i64, i64* %cap_p
  %slots_pp = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 2
  %slots_raw = load i8*, i8** %slots_pp
  %slots = bitcast i8* %slots_raw to { i64, i64, i64 }*
  %mask = sub i64 %cap, 1
  %h = and i64 %key, %mask
  %i.addr = alloca i64
  store i64 %h, i64* %i.addr
  br label %loop
loop:
  %i = load i64, i64* %i.addr
  %slot = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slots, i64 %i
  %occ_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 2
  %occ = load i64, i64* %occ_p
  %is_empty = icmp eq i64 %occ, 0
  br i1 %is_empty, label %not_found, label %check
check:
  %k_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 0
  %k = load i64, i64* %k_p
  %match = icmp eq i64 %k, %key
  br i1 %match, label %found, label %next
next:
  %i_next = add i64 %i, 1
  %i_wrap = and i64 %i_next, %mask
  store i64 %i_wrap, i64* %i.addr
  br label %loop
found:
  %v_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 1
  %v = load i64, i64* %v_p
  ret i64 %v
not_found:
  ret i64 0
}

; Grow the dict to 2x its current capacity. Called by insert when load
; factor reaches 75%. The outer-struct pointer stays valid; only the
; slot array is reallocated. Old slot memory is leaked (v1 ref-counting
; not yet wired up; v0.28 ships consistent with v0.27 in this respect).
define internal void @pyx86_dict_i64_grow(i8* %table_raw) {
entry:
  %g.i.addr = alloca i64
  %tp = bitcast i8* %table_raw to { i64, i64, i8* }*
  %size_p = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 0
  %cap_p = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 1
  %slots_pp = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 2
  %old_cap = load i64, i64* %cap_p
  %old_slots_raw = load i8*, i8** %slots_pp
  %old_slots = bitcast i8* %old_slots_raw to { i64, i64, i64 }*
  %new_cap = mul i64 %old_cap, 2
  %new_bytes = mul i64 %new_cap, 24
  %new_slots_raw = call i8* @malloc(i64 %new_bytes)
  call void @llvm.memset.p0i8.i64(i8* %new_slots_raw, i8 0, i64 %new_bytes, i1 false)
  store i8* %new_slots_raw, i8** %slots_pp
  store i64 %new_cap, i64* %cap_p
  store i64 0, i64* %size_p
  store i64 0, i64* %g.i.addr
  br label %g_loop
g_loop:
  %g_i = load i64, i64* %g.i.addr
  %g_done = icmp uge i64 %g_i, %old_cap
  br i1 %g_done, label %g_ret, label %g_body
g_body:
  %g_slot = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %old_slots, i64 %g_i
  %g_occ_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %g_slot, i32 0, i32 2
  %g_occ = load i64, i64* %g_occ_p
  %g_is_occ = icmp ne i64 %g_occ, 0
  br i1 %g_is_occ, label %g_reinsert, label %g_skip
g_reinsert:
  %g_k_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %g_slot, i32 0, i32 0
  %g_k = load i64, i64* %g_k_p
  %g_v_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %g_slot, i32 0, i32 1
  %g_v = load i64, i64* %g_v_p
  call void @pyx86_dict_i64_insert(i8* %table_raw, i64 %g_k, i64 %g_v)
  br label %g_skip
g_skip:
  %g_i_next = add i64 %g_i, 1
  store i64 %g_i_next, i64* %g.i.addr
  br label %g_loop
g_ret:
  ret void
}

define internal void @pyx86_dict_i64_insert(i8* %table_raw, i64 %key, i64 %value) {
entry:
  %i.addr = alloca i64
  %tp = bitcast i8* %table_raw to { i64, i64, i8* }*
  %size_p = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 0
  %cap_p = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 1
  %slots_pp = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 2
  %sz0 = load i64, i64* %size_p
  %cap0 = load i64, i64* %cap_p
  %sz4 = mul i64 %sz0, 4
  %cap3 = mul i64 %cap0, 3
  %need_grow = icmp uge i64 %sz4, %cap3
  br i1 %need_grow, label %do_grow, label %probe_init
do_grow:
  call void @pyx86_dict_i64_grow(i8* %table_raw)
  br label %probe_init
probe_init:
  %cap = load i64, i64* %cap_p
  %slots_raw = load i8*, i8** %slots_pp
  %slots = bitcast i8* %slots_raw to { i64, i64, i64 }*
  %mask = sub i64 %cap, 1
  %h = and i64 %key, %mask
  store i64 %h, i64* %i.addr
  br label %loop
loop:
  %i = load i64, i64* %i.addr
  %slot = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slots, i64 %i
  %occ_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 2
  %occ = load i64, i64* %occ_p
  %is_empty = icmp eq i64 %occ, 0
  br i1 %is_empty, label %do_insert, label %check
check:
  %k_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 0
  %k = load i64, i64* %k_p
  %match = icmp eq i64 %k, %key
  br i1 %match, label %do_overwrite, label %next
next:
  %i_next = add i64 %i, 1
  %i_wrap = and i64 %i_next, %mask
  store i64 %i_wrap, i64* %i.addr
  br label %loop
do_insert:
  %k_p2 = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 0
  store i64 %key, i64* %k_p2
  %v_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 1
  store i64 %value, i64* %v_p
  store i64 1, i64* %occ_p
  %old_size = load i64, i64* %size_p
  %new_size = add i64 %old_size, 1
  store i64 %new_size, i64* %size_p
  ret void
do_overwrite:
  %v_p2 = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 1
  store i64 %value, i64* %v_p2
  ret void
}

define internal i1 @pyx86_dict_i64_has(i8* %table_raw, i64 %key) {
entry:
  %tp = bitcast i8* %table_raw to { i64, i64, i8* }*
  %cap_p = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 1
  %cap = load i64, i64* %cap_p
  %slots_pp = getelementptr { i64, i64, i8* }, { i64, i64, i8* }* %tp, i32 0, i32 2
  %slots_raw = load i8*, i8** %slots_pp
  %slots = bitcast i8* %slots_raw to { i64, i64, i64 }*
  %mask = sub i64 %cap, 1
  %h = and i64 %key, %mask
  %i.addr = alloca i64
  store i64 %h, i64* %i.addr
  br label %loop
loop:
  %i = load i64, i64* %i.addr
  %slot = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slots, i64 %i
  %occ_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 2
  %occ = load i64, i64* %occ_p
  %is_empty = icmp eq i64 %occ, 0
  br i1 %is_empty, label %not_found, label %check
check:
  %k_p = getelementptr { i64, i64, i64 }, { i64, i64, i64 }* %slot, i32 0, i32 0
  %k = load i64, i64* %k_p
  %match = icmp eq i64 %k, %key
  br i1 %match, label %found, label %next
next:
  %i_next = add i64 %i, 1
  %i_wrap = and i64 %i_next, %mask
  store i64 %i_wrap, i64* %i.addr
  br label %loop
found:
  ret i1 true
not_found:
  ret i1 false
}

; pyx86_pow_i64(base, exp) — int ** by binary exponentiation. For
; exp < 0 returns 0 (Python returns float; we have no float printer
; on main return, but local float values do exist — float**float
; uses llvm.pow.f64 via the generic pow path).
define internal i64 @pyx86_pow_i64(i64 %base, i64 %exp) {
entry:
  %neg = icmp slt i64 %exp, 0
  br i1 %neg, label %neg_exit, label %init
neg_exit:
  ret i64 0
init:
  %r.addr = alloca i64
  %b.addr = alloca i64
  %e.addr = alloca i64
  store i64 1, i64* %r.addr
  store i64 %base, i64* %b.addr
  store i64 %exp, i64* %e.addr
  br label %loop_header
loop_header:
  %e0 = load i64, i64* %e.addr
  %done = icmp eq i64 %e0, 0
  br i1 %done, label %loop_exit, label %loop_body
loop_body:
  %e1 = load i64, i64* %e.addr
  %odd_bit = and i64 %e1, 1
  %is_odd = icmp ne i64 %odd_bit, 0
  br i1 %is_odd, label %mul_r, label %skip_mul
mul_r:
  %r0 = load i64, i64* %r.addr
  %b0 = load i64, i64* %b.addr
  %r1 = mul i64 %r0, %b0
  store i64 %r1, i64* %r.addr
  br label %skip_mul
skip_mul:
  %b1 = load i64, i64* %b.addr
  %b2 = mul i64 %b1, %b1
  store i64 %b2, i64* %b.addr
  %e2 = load i64, i64* %e.addr
  %e3 = ashr i64 %e2, 1
  store i64 %e3, i64* %e.addr
  br label %loop_header
loop_exit:
  %r_final = load i64, i64* %r.addr
  ret i64 %r_final
}

; pyx86_i64_to_str — format an i64 in base 10, return a fresh str struct.
; Buffer is 24 bytes (enough for -9223372036854775808 + NUL).
define internal { i64, i8* } @pyx86_i64_to_str(i64 %x) {
entry:
  %buf = call i8* @malloc(i64 24)
  %fmt = getelementptr inbounds [4 x i8], [4 x i8]* @.fmt_i64_buf, i64 0, i64 0
  %n = call i32 (i8*, i8*, ...) @sprintf(i8* %buf, i8* %fmt, i64 %x)
  %len = sext i32 %n to i64
  %s0 = insertvalue { i64, i8* } undef, i64 %len, 0
  %s1 = insertvalue { i64, i8* } %s0, i8* %buf, 1
  ret { i64, i8* } %s1
}

";

fn llvm_ty(ty: Type) -> String {
    match ty {
        Type::I8 => "i8".to_string(),
        Type::I16 => "i16".to_string(),
        Type::I32 => "i32".to_string(),
        Type::I64 => "i64".to_string(),
        Type::F64 => "double".to_string(),
        Type::Bool => "i1".to_string(),
        Type::Tuple(id) => {
            let inner = id.with_elems(|elems| {
                elems
                    .iter()
                    .map(|t| llvm_ty(*t))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            format!("{{ {} }}", inner)
        }
        // Lists are heap-allocated structs; the value passed around is
        // a pointer. The struct holds {len, cap, untyped data ptr}; data
        // is bitcast to the element-typed pointer at index/append time.
        Type::List(_) => "{ i64, i64, i8* }*".to_string(),
        Type::Str => "{ i64, i8* }".to_string(),
        // Same shape as a list — the slot array layout differs but the
        // outer struct has the same {len, cap, ptr} fields.
        Type::Dict(_) => "{ i64, i64, i8* }*".to_string(),
        Type::Class(id) => {
            let inner = id
                .fields()
                .iter()
                .map(|(_, t)| llvm_ty(*t))
                .collect::<Vec<_>>()
                .join(", ");
            // Empty class → use a single-byte placeholder so malloc(0) is fine.
            if inner.is_empty() {
                "{ i8 }*".to_string()
            } else {
                format!("{{ {} }}*", inner)
            }
        }
    }
}

/// Element size in bytes for malloc sizing.
fn type_byte_size(ty: Type) -> u64 {
    match ty {
        Type::I8 | Type::Bool => 1,
        Type::I16 => 2,
        Type::I32 => 4,
        Type::I64 | Type::F64 => 8,
        Type::Tuple(id) => id.with_elems(|elems| elems.iter().map(|t| type_byte_size(*t)).sum()),
        Type::List(_) => 8,
        Type::Str => 16,
        Type::Dict(_) => 8,
        Type::Class(_) => 8, // pointer
    }
}

/// Total bytes of all fields of a class (heap-alloc size).
fn class_byte_size(id: crate::hir::ClassId) -> u64 {
    id.fields().iter().map(|(_, t)| type_byte_size(*t)).sum::<u64>().max(1)
}

fn format_signature(func: &Function) -> String {
    func.params
        .iter()
        .map(|p| format!("{} %p_{}", llvm_ty(p.ty), p.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_argv_parsing(func: &Function) -> String {
    let mut s = String::new();
    for (i, p) in func.params.iter().enumerate() {
        let argv_index = (i + 1) as i64;
        let _ = writeln!(
            s,
            "  %slot{i} = getelementptr inbounds i8*, i8** %argv, i64 {idx}",
            i = i,
            idx = argv_index
        );
        let _ = writeln!(s, "  %str{i} = load i8*, i8** %slot{i}", i = i);
        // Dispatch on parameter type. Check rejects anything other than I64/F64.
        match p.ty {
            Type::I64 => {
                let _ = writeln!(
                    s,
                    "  %p_{name} = call i64 @atoll(i8* %str{i})",
                    name = p.name,
                    i = i
                );
            }
            Type::F64 => {
                let _ = writeln!(
                    s,
                    "  %p_{name} = call double @atof(i8* %str{i})",
                    name = p.name,
                    i = i
                );
            }
            Type::I8 | Type::I16 | Type::I32 | Type::Bool | Type::Tuple(_) | Type::List(_) | Type::Str | Type::Dict(_) | Type::Class(_) => {
                unreachable!("check rejects non-(I64|F64) main params")
            }
        }
    }
    s
}

struct Codegen {
    body: String,
    next_id: usize,
    next_block_id: usize,
    block_terminated: bool,
    /// continue_target / break_target stack for nested loops.
    loop_targets: Vec<(String, String)>,
    /// Variable scope: name → type (set when entering function).
    locals: HashMap<String, Type>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            body: String::new(),
            next_id: 0,
            next_block_id: 0,
            block_terminated: false,
            loop_targets: Vec::new(),
            locals: HashMap::new(),
        }
    }

    fn fresh(&mut self) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("%v{}", n)
    }

    fn emit(&mut self, line: &str) {
        if self.block_terminated {
            return;
        }
        let _ = writeln!(self.body, "  {}", line);
    }

    fn open_block(&mut self, label: &str) {
        let _ = writeln!(self.body, "{}:", label);
        self.block_terminated = false;
    }

    fn lower_function(&mut self, func: &Function) {
        self.open_block("entry");

        // Seed locals scope with params + collected let-introduced names.
        let local_decls = collect_locals(&func.body);
        let mut emitted: HashSet<String> = HashSet::new();
        for p in &func.params {
            self.locals.insert(p.name.clone(), p.ty);
            self.emit(&format!("%{}.addr = alloca {}", p.name, llvm_ty(p.ty)));
            self.emit(&format!(
                "store {ty} %p_{name}, {ty}* %{name}.addr",
                ty = llvm_ty(p.ty),
                name = p.name
            ));
            emitted.insert(p.name.clone());
        }
        for (name, ty) in &local_decls {
            // If name is also a param, the param's slot already exists.
            // The local would shadow but we share the slot — only legal
            // if the types match.
            if let Some(existing_ty) = self.locals.get(name) {
                if *existing_ty != *ty {
                    panic!(
                        "internal: local `{}` re-binds with different type ({}→{}) — should be rejected by check",
                        name, existing_ty.name(), ty.name()
                    );
                }
                continue;
            }
            self.locals.insert(name.clone(), *ty);
            if emitted.insert(name.clone()) {
                self.emit(&format!("%{}.addr = alloca {}", name, llvm_ty(*ty)));
            }
        }

        self.lower_block(&func.body);

        if !self.block_terminated {
            self.emit("unreachable");
            self.block_terminated = true;
        }
    }

    fn lower_block(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if self.block_terminated {
                break;
            }
            match stmt {
                Stmt::Let { name, value } => {
                    let op = self.lower(value);
                    self.emit(&format!(
                        "store {ty} {op}, {ty}* %{name}.addr",
                        ty = llvm_ty(value.ty),
                        op = op,
                        name = name
                    ));
                }
                Stmt::Return { value } => {
                    let op = self.lower(value);
                    self.emit(&format!("ret {} {}", llvm_ty(value.ty), op));
                    self.block_terminated = true;
                }
                Stmt::If { cond, then_body, else_body } => {
                    debug_assert_eq!(cond.ty, Type::Bool);
                    let cond_i1 = self.lower(cond);
                    let id = self.next_block_id;
                    self.next_block_id += 1;
                    let then_lbl = format!("then.{}", id);
                    let else_lbl = format!("else.{}", id);
                    let merge_lbl = format!("merge.{}", id);
                    self.emit(&format!(
                        "br i1 {}, label %{}, label %{}",
                        cond_i1, then_lbl, else_lbl
                    ));
                    self.block_terminated = true;
                    self.open_block(&then_lbl);
                    self.lower_block(then_body);
                    if !self.block_terminated {
                        self.emit(&format!("br label %{}", merge_lbl));
                        self.block_terminated = true;
                    }
                    self.open_block(&else_lbl);
                    self.lower_block(else_body);
                    if !self.block_terminated {
                        self.emit(&format!("br label %{}", merge_lbl));
                        self.block_terminated = true;
                    }
                    self.open_block(&merge_lbl);
                }
                Stmt::While { cond, body } => {
                    debug_assert_eq!(cond.ty, Type::Bool);
                    let id = self.next_block_id;
                    self.next_block_id += 1;
                    let header_lbl = format!("loop_header.{}", id);
                    let body_lbl = format!("loop_body.{}", id);
                    let exit_lbl = format!("loop_exit.{}", id);
                    self.emit(&format!("br label %{}", header_lbl));
                    self.block_terminated = true;
                    self.open_block(&header_lbl);
                    let cond_i1 = self.lower(cond);
                    self.emit(&format!(
                        "br i1 {}, label %{}, label %{}",
                        cond_i1, body_lbl, exit_lbl
                    ));
                    self.block_terminated = true;
                    self.open_block(&body_lbl);
                    self.loop_targets
                        .push((header_lbl.clone(), exit_lbl.clone()));
                    self.lower_block(body);
                    self.loop_targets.pop();
                    if !self.block_terminated {
                        self.emit(&format!("br label %{}", header_lbl));
                        self.block_terminated = true;
                    }
                    self.open_block(&exit_lbl);
                }
                Stmt::Break => {
                    let (_, brk) = self
                        .loop_targets
                        .last()
                        .expect("internal: Break with empty loop stack");
                    let target = brk.clone();
                    self.emit(&format!("br label %{}", target));
                    self.block_terminated = true;
                }
                Stmt::ListAppend { list, value } => {
                    self.lower_list_append(list, value);
                }
                Stmt::SetField { obj, field_index, value } => {
                    self.lower_set_field(obj, *field_index, value);
                }
                Stmt::SetSubscript { container, key, value } => {
                    self.lower_set_subscript(container, key, value);
                }
                Stmt::ExprStmt(e) => {
                    let _ = self.lower(e);
                }
                Stmt::Continue => {
                    let (cnt, _) = self
                        .loop_targets
                        .last()
                        .expect("internal: Continue with empty loop stack");
                    let target = cnt.clone();
                    self.emit(&format!("br label %{}", target));
                    self.block_terminated = true;
                }
            }
        }
    }

    /// Lower a TypedExpr to an LLVM operand of the corresponding LLVM type.
    fn lower(&mut self, te: &TypedExpr) -> String {
        match &te.expr {
            Expr::ConstI64(v) => v.to_string(),
            Expr::ConstF64(v) => format_f64_literal(*v),
            Expr::ConstBool(b) => if *b { "1".into() } else { "0".into() },
            Expr::Var(name) => {
                let ty = *self
                    .locals
                    .get(name)
                    .unwrap_or_else(|| panic!("internal: var `{}` not in codegen scope", name));
                let dst = self.fresh();
                self.emit(&format!(
                    "{} = load {ty}, {ty}* %{name}.addr",
                    dst,
                    ty = llvm_ty(ty),
                    name = name
                ));
                dst
            }
            Expr::Coerce { inner } => self.lower_coerce(inner, te.ty),
            Expr::UnaryOp { op, operand } => self.lower_unary(*op, operand, te.ty),
            Expr::BinOp { op, lhs, rhs } => self.lower_binop(*op, lhs, rhs, te.ty),
            Expr::Cmp { op, lhs, rhs } => self.lower_cmp(*op, lhs, rhs),
            Expr::CmpChain { first, rest } => self.lower_cmp_chain(first, rest),
            Expr::Not(inner) => self.lower_not(inner),
            Expr::BoolOp { op, lhs, rhs } => self.lower_bool_op(*op, lhs, rhs, te.ty),
            Expr::Call { callee, args } => self.lower_call(callee, args, te.ty),
            Expr::TupleLit { elements } => self.lower_tuple_lit(elements, te.ty),
            Expr::TupleIndex { tuple, index } => self.lower_tuple_index(tuple, *index, te.ty),
            Expr::ListLit { elements } => self.lower_list_lit(elements, te.ty),
            Expr::ListIndex { list, index } => self.lower_list_index(list, index, te.ty),
            Expr::ListLen { list } => self.lower_list_len(list),
            Expr::ListConcat { lhs, rhs } => self.lower_list_concat(lhs, rhs, te.ty),
            Expr::DoBlock { stmts, result } => {
                self.lower_block(stmts);
                self.lower(result)
            }
            Expr::StrLit(s) => self.lower_str_lit(s),
            Expr::StrConcat { lhs, rhs } => self.lower_str_concat(lhs, rhs),
            Expr::StrLen { s } => self.lower_str_len(s),
            Expr::StrEq { lhs, rhs, negated } => self.lower_str_eq(lhs, rhs, *negated),
            Expr::FormatToStr { inner } => self.lower_format_to_str(inner),
            Expr::MathCall { intrinsic, arg } => self.lower_math_call(intrinsic, arg),
            Expr::DictLit { entries } => self.lower_dict_lit(entries, te.ty),
            Expr::DictGet { dict, key } => self.lower_dict_get(dict, key),
            Expr::DictHas { dict, key } => self.lower_dict_has(dict, key),
            Expr::DictLen { dict } => self.lower_dict_len(dict),
            Expr::FieldGet { obj, field_index } => self.lower_field_get(obj, *field_index, te.ty),
            Expr::ClassNew { class, args } => self.lower_class_new(*class, args),
        }
    }

    fn lower_coerce(&mut self, inner: &TypedExpr, target: Type) -> String {
        let inner_op = self.lower(inner);
        if inner.ty == target {
            return inner_op;
        }
        // Bool ↔ Int / Bool ↔ Float
        match (inner.ty, target) {
            (Type::Bool, t) if t.is_int() => {
                let dst = self.fresh();
                self.emit(&format!("{} = zext i1 {} to {}", dst, inner_op, llvm_ty(t)));
                dst
            }
            (a, Type::Bool) if a.is_int() => {
                let dst = self.fresh();
                self.emit(&format!("{} = icmp ne {} {}, 0", dst, llvm_ty(a), inner_op));
                dst
            }
            (Type::Bool, Type::F64) => {
                let intermediate = self.fresh();
                self.emit(&format!("{} = zext i1 {} to i64", intermediate, inner_op));
                let dst = self.fresh();
                self.emit(&format!("{} = sitofp i64 {} to double", dst, intermediate));
                dst
            }
            (Type::F64, Type::Bool) => {
                let dst = self.fresh();
                self.emit(&format!("{} = fcmp one double {}, 0.0", dst, inner_op));
                dst
            }
            (a, Type::F64) if a.is_int() => {
                let dst = self.fresh();
                self.emit(&format!("{} = sitofp {} {} to double", dst, llvm_ty(a), inner_op));
                dst
            }
            (Type::F64, b) if b.is_int() => {
                // Generated by `int(x)` builtin. Truncates toward zero,
                // matching CPython for finite values. NaN / Inf produce
                // poison in LLVM (undefined behaviour) — Python raises
                // ValueError / OverflowError for those; documented divergence.
                let dst = self.fresh();
                self.emit(&format!("{} = fptosi double {} to {}", dst, inner_op, llvm_ty(b)));
                dst
            }
            // Int width changes
            (a, b) if a.is_int() && b.is_int() => {
                let aw = a.int_width().unwrap();
                let bw = b.int_width().unwrap();
                let dst = self.fresh();
                if bw > aw {
                    // sign-extend
                    self.emit(&format!("{} = sext {} {} to {}", dst, llvm_ty(a), inner_op, llvm_ty(b)));
                } else {
                    // narrow
                    self.emit(&format!("{} = trunc {} {} to {}", dst, llvm_ty(a), inner_op, llvm_ty(b)));
                }
                dst
            }
            (a, b) => panic!(
                "internal: codegen Coerce {} → {} not implemented",
                a.name(),
                b.name()
            ),
        }
    }

    fn lower_unary(&mut self, op: UnaryOp, operand: &TypedExpr, _result_ty: Type) -> String {
        let v = self.lower(operand);
        match op {
            UnaryOp::Pos => v,
            UnaryOp::Neg if operand.ty.is_int() => {
                let dst = self.fresh();
                self.emit(&format!("{} = sub {} 0, {}", dst, llvm_ty(operand.ty), v));
                dst
            }
            UnaryOp::Neg if operand.ty == Type::F64 => {
                let dst = self.fresh();
                self.emit(&format!("{} = fneg double {}", dst, v));
                dst
            }
            UnaryOp::BitNot if operand.ty.is_int() => {
                let dst = self.fresh();
                self.emit(&format!("{} = xor {} {}, -1", dst, llvm_ty(operand.ty), v));
                dst
            }
            _ => panic!(
                "internal: unary op {:?} on type {} should have been coerced",
                op,
                operand.ty.name()
            ),
        }
    }

    fn lower_binop(&mut self, op: BinOp, lhs: &TypedExpr, rhs: &TypedExpr, result_ty: Type) -> String {
        let l = self.lower(lhs);
        let r = self.lower(rhs);
        debug_assert_eq!(lhs.ty, rhs.ty, "binop operands must have matching types");
        let ty = lhs.ty;
        match (op, ty) {
            (BinOp::Add, t) if t.is_int() => self.simple_iop("add", &l, &r, t),
            (BinOp::Sub, t) if t.is_int() => self.simple_iop("sub", &l, &r, t),
            (BinOp::Mul, t) if t.is_int() => self.simple_iop("mul", &l, &r, t),
            (BinOp::FloorDiv, t) if t.is_int() => self.floor_div_int(&l, &r, t),
            (BinOp::Mod, t) if t.is_int() => self.floor_mod_int(&l, &r, t),
            (BinOp::BitAnd, t) if t.is_int() => self.simple_iop("and", &l, &r, t),
            (BinOp::BitOr, t) if t.is_int() => self.simple_iop("or", &l, &r, t),
            (BinOp::BitXor, t) if t.is_int() => self.simple_iop("xor", &l, &r, t),
            (BinOp::Shl, t) if t.is_int() => self.simple_iop("shl", &l, &r, t),
            (BinOp::Shr, t) if t.is_int() => self.simple_iop("ashr", &l, &r, t),
            (BinOp::Add, Type::F64) => self.simple_fop("fadd", &l, &r),
            (BinOp::Sub, Type::F64) => self.simple_fop("fsub", &l, &r),
            (BinOp::Mul, Type::F64) => self.simple_fop("fmul", &l, &r),
            (BinOp::TrueDiv, Type::F64) => {
                debug_assert_eq!(result_ty, Type::F64);
                self.simple_fop("fdiv", &l, &r)
            }
            (BinOp::Pow, Type::I64) => {
                let dst = self.fresh();
                self.emit(&format!("{} = call i64 @pyx86_pow_i64(i64 {}, i64 {})", dst, l, r));
                dst
            }
            (BinOp::Pow, Type::F64) => {
                let dst = self.fresh();
                self.emit(&format!(
                    "{} = call double @llvm.pow.f64(double {}, double {})",
                    dst, l, r
                ));
                dst
            }
            (op, ty) => panic!(
                "internal: binop {:?} on type {} not supported (should be rejected by check)",
                op,
                ty.name()
            ),
        }
    }

    fn simple_iop(&mut self, op: &str, l: &str, r: &str, ty: Type) -> String {
        let dst = self.fresh();
        self.emit(&format!("{} = {} {} {}, {}", dst, op, llvm_ty(ty), l, r));
        dst
    }

    fn simple_fop(&mut self, op: &str, l: &str, r: &str) -> String {
        let dst = self.fresh();
        self.emit(&format!("{} = {} double {}, {}", dst, op, l, r));
        dst
    }

    fn floor_div_int(&mut self, l: &str, r: &str, ty: Type) -> String {
        let t = llvm_ty(ty);
        let q = self.fresh();
        let rem = self.fresh();
        let rem_nz = self.fresh();
        let xor_sign = self.fresh();
        let signs_differ = self.fresh();
        let needs = self.fresh();
        let adj = self.fresh();
        let dst = self.fresh();
        self.emit(&format!("{} = sdiv {} {}, {}", q, t, l, r));
        self.emit(&format!("{} = srem {} {}, {}", rem, t, l, r));
        self.emit(&format!("{} = icmp ne {} {}, 0", rem_nz, t, rem));
        self.emit(&format!("{} = xor {} {}, {}", xor_sign, t, l, r));
        self.emit(&format!("{} = icmp slt {} {}, 0", signs_differ, t, xor_sign));
        self.emit(&format!("{} = and i1 {}, {}", needs, rem_nz, signs_differ));
        self.emit(&format!("{} = sext i1 {} to {}", adj, needs, t));
        self.emit(&format!("{} = add {} {}, {}", dst, t, q, adj));
        dst
    }

    fn floor_mod_int(&mut self, l: &str, r: &str, ty: Type) -> String {
        let t = llvm_ty(ty);
        let rem = self.fresh();
        let rem_nz = self.fresh();
        let xor_sign = self.fresh();
        let signs_differ = self.fresh();
        let needs = self.fresh();
        let adj = self.fresh();
        let dst = self.fresh();
        self.emit(&format!("{} = srem {} {}, {}", rem, t, l, r));
        self.emit(&format!("{} = icmp ne {} {}, 0", rem_nz, t, rem));
        self.emit(&format!("{} = xor {} {}, {}", xor_sign, t, rem, r));
        self.emit(&format!("{} = icmp slt {} {}, 0", signs_differ, t, xor_sign));
        self.emit(&format!("{} = and i1 {}, {}", needs, rem_nz, signs_differ));
        self.emit(&format!("{} = select i1 {}, {} {}, {} 0", adj, needs, t, r, t));
        self.emit(&format!("{} = add {} {}, {}", dst, t, rem, adj));
        dst
    }

    fn lower_cmp(&mut self, op: CmpOp, lhs: &TypedExpr, rhs: &TypedExpr) -> String {
        debug_assert_eq!(lhs.ty, rhs.ty);
        let l = self.lower(lhs);
        let r = self.lower(rhs);
        let dst = self.fresh();
        if lhs.ty == Type::F64 {
            self.emit(&format!(
                "{} = fcmp {} double {}, {}",
                dst,
                llvm_fcmp_op(op),
                l,
                r
            ));
        } else {
            // Int or Bool — both LLVM integer types, use icmp.
            self.emit(&format!(
                "{} = icmp {} {} {}, {}",
                dst,
                llvm_icmp_op(op),
                llvm_ty(lhs.ty),
                l,
                r
            ));
        }
        dst
    }

    fn lower_cmp_chain(&mut self, first: &TypedExpr, rest: &[(CmpOp, TypedExpr)]) -> String {
        let mut prev_ty = first.ty;
        let mut prev_op = self.lower(first);
        let mut acc: Option<String> = None;
        for (op, e) in rest {
            let next_op = self.lower(e);
            debug_assert_eq!(prev_ty, e.ty);
            let cmp = self.fresh();
            if prev_ty == Type::F64 {
                self.emit(&format!(
                    "{} = fcmp {} double {}, {}",
                    cmp,
                    llvm_fcmp_op(*op),
                    prev_op,
                    next_op
                ));
            } else {
                self.emit(&format!(
                    "{} = icmp {} {} {}, {}",
                    cmp,
                    llvm_icmp_op(*op),
                    llvm_ty(prev_ty),
                    prev_op,
                    next_op
                ));
            }
            acc = match acc {
                None => Some(cmp),
                Some(a) => {
                    let combined = self.fresh();
                    self.emit(&format!("{} = and i1 {}, {}", combined, a, cmp));
                    Some(combined)
                }
            };
            prev_op = next_op;
            prev_ty = e.ty;
        }
        acc.expect("CmpChain.rest must be non-empty")
    }

    fn lower_not(&mut self, inner: &TypedExpr) -> String {
        debug_assert_eq!(inner.ty, Type::Bool, "Not operand must already be Bool");
        let v = self.lower(inner);
        let dst = self.fresh();
        self.emit(&format!("{} = xor i1 {}, true", dst, v));
        dst
    }

    fn lower_bool_op(
        &mut self,
        op: BoolOp,
        lhs: &TypedExpr,
        rhs: &TypedExpr,
        result_ty: Type,
    ) -> String {
        // Both operands have the same type after unification.
        debug_assert_eq!(lhs.ty, rhs.ty);
        debug_assert_eq!(result_ty, lhs.ty);
        let ty_str = llvm_ty(result_ty);

        let slot_id = self.next_id;
        self.next_id += 1;
        let slot = format!("%bool.{}.addr", slot_id);
        self.emit(&format!("{} = alloca {}", slot, ty_str));

        let lhs_op = self.lower(lhs);
        self.emit(&format!("store {ty} {op}, {ty}* {slot}", ty = ty_str, op = lhs_op, slot = slot));

        // Truthiness check on lhs.
        let cond = self.fresh();
        if lhs.ty == Type::F64 {
            self.emit(&format!("{} = fcmp one double {}, 0.0", cond, lhs_op));
        } else {
            // Int or Bool — both LLVM integer types.
            self.emit(&format!(
                "{} = icmp ne {} {}, 0",
                cond,
                llvm_ty(lhs.ty),
                lhs_op
            ));
        }

        let id = self.next_block_id;
        self.next_block_id += 1;
        let eval_rhs_lbl = format!("bool.eval_rhs.{}", id);
        let merge_lbl = format!("bool.merge.{}", id);
        let (truthy_lbl, falsy_lbl) = match op {
            BoolOp::And => (eval_rhs_lbl.as_str(), merge_lbl.as_str()),
            BoolOp::Or => (merge_lbl.as_str(), eval_rhs_lbl.as_str()),
        };
        self.emit(&format!("br i1 {}, label %{}, label %{}", cond, truthy_lbl, falsy_lbl));
        self.block_terminated = true;

        self.open_block(&eval_rhs_lbl);
        let rhs_op = self.lower(rhs);
        self.emit(&format!("store {ty} {op}, {ty}* {slot}", ty = ty_str, op = rhs_op, slot = slot));
        self.emit(&format!("br label %{}", merge_lbl));
        self.block_terminated = true;

        self.open_block(&merge_lbl);
        let dst = self.fresh();
        self.emit(&format!("{} = load {ty}, {ty}* {}", dst, slot, ty = ty_str));
        dst
    }

    /// Build a tuple value via repeated `insertvalue`.
    /// `{ undef, ... }` → insertvalue at each index.
    fn lower_tuple_lit(&mut self, elements: &[TypedExpr], tuple_ty: Type) -> String {
        let tuple_llvm_ty = llvm_ty(tuple_ty);
        let mut acc = format!("undef");
        for (i, elem) in elements.iter().enumerate() {
            let elem_op = self.lower(elem);
            let elem_ty = llvm_ty(elem.ty);
            let dst = self.fresh();
            self.emit(&format!(
                "{} = insertvalue {} {}, {} {}, {}",
                dst, tuple_llvm_ty, acc, elem_ty, elem_op, i
            ));
            acc = dst;
        }
        acc
    }

    fn lower_tuple_index(&mut self, tuple: &TypedExpr, index: usize, _result_ty: Type) -> String {
        let tuple_op = self.lower(tuple);
        let tuple_llvm_ty = llvm_ty(tuple.ty);
        let dst = self.fresh();
        self.emit(&format!(
            "{} = extractvalue {} {}, {}",
            dst, tuple_llvm_ty, tuple_op, index
        ));
        dst
    }

    /// Ref-semantics: lists are pointers to `{i64 len, i64 cap, i8* data}`
    /// on the heap. Mutations via .append() see through aliases.
    fn lower_list_lit(&mut self, elements: &[TypedExpr], list_ty: Type) -> String {
        let id = match list_ty {
            Type::List(id) => id,
            _ => panic!("internal: list_lit with non-list type"),
        };
        let elem_ty = id.elem();
        let elem_llvm = llvm_ty(elem_ty);
        let elem_size = type_byte_size(elem_ty);
        let n = elements.len() as i64;

        let elem_ops: Vec<String> = elements.iter().map(|e| self.lower(e)).collect();

        let data_bytes = (n as u64).max(1) * elem_size;
        let data_raw = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 {})", data_raw, data_bytes));
        let data_typed = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {}*",
            data_typed, data_raw, elem_llvm
        ));
        for (i, op) in elem_ops.iter().enumerate() {
            let p = self.fresh();
            self.emit(&format!(
                "{} = getelementptr {ety}, {ety}* {}, i64 {}",
                p, data_typed, i, ety = elem_llvm
            ));
            self.emit(&format!("store {ety} {}, {ety}* {}", op, p, ety = elem_llvm));
        }

        let struct_raw = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 24)", struct_raw));
        let struct_p = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {{ i64, i64, i8* }}*",
            struct_p, struct_raw
        ));
        let len_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 0",
            len_p, struct_p
        ));
        self.emit(&format!("store i64 {}, i64* {}", n, len_p));
        let cap_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 1",
            cap_p, struct_p
        ));
        self.emit(&format!("store i64 {}, i64* {}", n, cap_p));
        let data_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
            data_p, struct_p
        ));
        self.emit(&format!("store i8* {}, i8** {}", data_raw, data_p));
        struct_p
    }

    fn lower_list_index(&mut self, list: &TypedExpr, index: &TypedExpr, _result_ty: Type) -> String {
        let list_op = self.lower(list);
        let idx_op = self.lower(index);
        let elem_ty = match list.ty {
            Type::List(id) => id.elem(),
            _ => panic!("internal: list_index on non-list"),
        };
        let elem_llvm = llvm_ty(elem_ty);
        let data_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
            data_p, list_op
        ));
        let data_raw = self.fresh();
        self.emit(&format!("{} = load i8*, i8** {}", data_raw, data_p));
        let data_typed = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {}*",
            data_typed, data_raw, elem_llvm
        ));
        let p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {ety}, {ety}* {}, i64 {}",
            p, data_typed, idx_op, ety = elem_llvm
        ));
        let dst = self.fresh();
        self.emit(&format!("{} = load {ety}, {ety}* {}", dst, p, ety = elem_llvm));
        dst
    }

    /// New ref-semantics: deref both lhs/rhs structs to get len/data,
    /// memcpy bytes into a fresh malloc'd buffer, allocate a new
    /// struct, return its pointer.
    fn lower_list_concat(&mut self, lhs: &TypedExpr, rhs: &TypedExpr, list_ty: Type) -> String {
        let id = match list_ty {
            Type::List(id) => id,
            _ => panic!("internal: list_concat with non-list result type"),
        };
        let elem_size = type_byte_size(id.elem());

        let lhs_op = self.lower(lhs);
        let rhs_op = self.lower(rhs);

        let lhs_len = self.list_load_len(&lhs_op);
        let lhs_data = self.list_load_data_raw(&lhs_op);
        let rhs_len = self.list_load_len(&rhs_op);
        let rhs_data = self.list_load_data_raw(&rhs_op);

        let total_len = self.fresh();
        self.emit(&format!("{} = add i64 {}, {}", total_len, lhs_len, rhs_len));
        let lhs_bytes = self.fresh();
        self.emit(&format!("{} = mul i64 {}, {}", lhs_bytes, lhs_len, elem_size));
        let rhs_bytes = self.fresh();
        self.emit(&format!("{} = mul i64 {}, {}", rhs_bytes, rhs_len, elem_size));
        let total_bytes = self.fresh();
        self.emit(&format!(
            "{} = add i64 {}, {}",
            total_bytes, lhs_bytes, rhs_bytes
        ));
        // Avoid 0-byte malloc.
        let bytes_or_one = self.fresh();
        let nz = self.fresh();
        self.emit(&format!("{} = icmp ne i64 {}, 0", nz, total_bytes));
        self.emit(&format!(
            "{} = select i1 {}, i64 {}, i64 1",
            bytes_or_one, nz, total_bytes
        ));

        let new_data = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 {})", new_data, bytes_or_one));
        // memcpy lhs bytes
        self.emit(&format!(
            "call void @llvm.memcpy.p0i8.p0i8.i64(i8* {}, i8* {}, i64 {}, i1 false)",
            new_data, lhs_data, lhs_bytes
        ));
        // memcpy rhs bytes at offset lhs_bytes
        let mid = self.fresh();
        self.emit(&format!(
            "{} = getelementptr i8, i8* {}, i64 {}",
            mid, new_data, lhs_bytes
        ));
        self.emit(&format!(
            "call void @llvm.memcpy.p0i8.p0i8.i64(i8* {}, i8* {}, i64 {}, i1 false)",
            mid, rhs_data, rhs_bytes
        ));

        // Build the struct.
        self.list_build_struct(&total_len, &total_len, &new_data)
    }

    /// `lst.append(value)` — mutates the heap struct in place. Grows
    /// the data buffer (doubling, min 4) when len == cap.
    fn lower_list_append(&mut self, list: &TypedExpr, value: &TypedExpr) {
        let elem_ty = match list.ty {
            Type::List(id) => id.elem(),
            _ => panic!("internal: list_append on non-list"),
        };
        let elem_llvm = llvm_ty(elem_ty);
        let elem_size = type_byte_size(elem_ty);

        let list_op = self.lower(list);
        let value_op = self.lower(value);

        // Load len, cap.
        let len_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 0",
            len_p, list_op
        ));
        let len = self.fresh();
        self.emit(&format!("{} = load i64, i64* {}", len, len_p));
        let cap_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 1",
            cap_p, list_op
        ));
        let cap = self.fresh();
        self.emit(&format!("{} = load i64, i64* {}", cap, cap_p));
        let data_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
            data_p, list_op
        ));

        // Grow if len >= cap.
        let need_grow = self.fresh();
        self.emit(&format!("{} = icmp uge i64 {}, {}", need_grow, len, cap));
        let id = self.next_block_id;
        self.next_block_id += 1;
        let grow_lbl = format!("append.grow.{}", id);
        let after_lbl = format!("append.after.{}", id);
        self.emit(&format!(
            "br i1 {}, label %{}, label %{}",
            need_grow, grow_lbl, after_lbl
        ));
        self.block_terminated = true;

        self.open_block(&grow_lbl);
        // new_cap = max(cap*2, 4)
        let cap2 = self.fresh();
        self.emit(&format!("{} = mul i64 {}, 2", cap2, cap));
        let cmp4 = self.fresh();
        self.emit(&format!("{} = icmp ult i64 {}, 4", cmp4, cap2));
        let new_cap = self.fresh();
        self.emit(&format!(
            "{} = select i1 {}, i64 4, i64 {}",
            new_cap, cmp4, cap2
        ));
        let new_bytes = self.fresh();
        self.emit(&format!("{} = mul i64 {}, {}", new_bytes, new_cap, elem_size));
        let old_data = self.fresh();
        self.emit(&format!("{} = load i8*, i8** {}", old_data, data_p));
        let new_data = self.fresh();
        self.emit(&format!(
            "{} = call i8* @realloc(i8* {}, i64 {})",
            new_data, old_data, new_bytes
        ));
        self.emit(&format!("store i8* {}, i8** {}", new_data, data_p));
        self.emit(&format!("store i64 {}, i64* {}", new_cap, cap_p));
        self.emit(&format!("br label %{}", after_lbl));
        self.block_terminated = true;

        self.open_block(&after_lbl);
        // Store value at data[len], increment len.
        let data_raw = self.fresh();
        self.emit(&format!("{} = load i8*, i8** {}", data_raw, data_p));
        let data_typed = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {}*",
            data_typed, data_raw, elem_llvm
        ));
        let slot = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {ety}, {ety}* {}, i64 {}",
            slot, data_typed, len, ety = elem_llvm
        ));
        self.emit(&format!(
            "store {ety} {}, {ety}* {}",
            value_op, slot, ety = elem_llvm
        ));
        let new_len = self.fresh();
        self.emit(&format!("{} = add i64 {}, 1", new_len, len));
        self.emit(&format!("store i64 {}, i64* {}", new_len, len_p));
    }

    /// Helper: emit GEP + load of `struct.len` from a list pointer operand.
    fn list_load_len(&mut self, list_op: &str) -> String {
        let p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 0",
            p, list_op
        ));
        let dst = self.fresh();
        self.emit(&format!("{} = load i64, i64* {}", dst, p));
        dst
    }

    /// Helper: emit GEP + load of `struct.data` (untyped i8*) from a list pointer.
    fn list_load_data_raw(&mut self, list_op: &str) -> String {
        let p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
            p, list_op
        ));
        let dst = self.fresh();
        self.emit(&format!("{} = load i8*, i8** {}", dst, p));
        dst
    }

    /// Helper: malloc + initialize a `{ i64 len, i64 cap, i8* data }`
    /// struct on the heap and return its pointer.
    fn list_build_struct(&mut self, len: &str, cap: &str, data: &str) -> String {
        let raw = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 24)", raw));
        let p = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {{ i64, i64, i8* }}*",
            p, raw
        ));
        let lp = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 0",
            lp, p
        ));
        self.emit(&format!("store i64 {}, i64* {}", len, lp));
        let cp = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 1",
            cp, p
        ));
        self.emit(&format!("store i64 {}, i64* {}", cap, cp));
        let dp = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
            dp, p
        ));
        self.emit(&format!("store i8* {}, i8** {}", data, dp));
        p
    }

    /// Build a dict[i64, i64] literal: allocate the outer struct +
    /// the slot array (cap = next pow2 >= 2*N, min 4), then call
    /// pyx86_dict_i64_insert for each (k, v) pair.
    fn lower_dict_lit(&mut self, entries: &[(TypedExpr, TypedExpr)], _dict_ty: Type) -> String {
        // Compute cap: next pow2 >= max(2 * entries.len(), 4).
        let n = entries.len();
        let want = (n * 2).max(4);
        let mut cap: u64 = 4;
        while (cap as usize) < want {
            cap *= 2;
        }
        // Slot size = 24 bytes ({i64, i64, i64}).
        let slot_bytes = cap * 24;

        // Allocate the slot array (zeroed via calloc-style: malloc + memset 0).
        let slots_raw = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 {})", slots_raw, slot_bytes));
        // Zero it. Use memcpy from a zeroed buffer? Easier: another helper.
        // Use llvm.memset:
        self.emit(&format!(
            "call void @llvm.memset.p0i8.i64(i8* {}, i8 0, i64 {}, i1 false)",
            slots_raw, slot_bytes
        ));

        // Allocate outer struct.
        let table_raw = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 24)", table_raw));
        let tp = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {{ i64, i64, i8* }}*",
            tp, table_raw
        ));
        // size = 0 (insert increments)
        let size_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 0",
            size_p, tp
        ));
        self.emit(&format!("store i64 0, i64* {}", size_p));
        let cap_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 1",
            cap_p, tp
        ));
        self.emit(&format!("store i64 {}, i64* {}", cap, cap_p));
        let slots_p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
            slots_p, tp
        ));
        self.emit(&format!("store i8* {}, i8** {}", slots_raw, slots_p));

        // Insert each entry via the runtime helper.
        for (k, v) in entries {
            let k_op = self.lower(k);
            let v_op = self.lower(v);
            self.emit(&format!(
                "call void @pyx86_dict_i64_insert(i8* {}, i64 {}, i64 {})",
                table_raw, k_op, v_op
            ));
        }
        tp
    }

    fn lower_dict_get(&mut self, dict: &TypedExpr, key: &TypedExpr) -> String {
        let dict_op = self.lower(dict);
        let key_op = self.lower(key);
        let table_raw = self.fresh();
        self.emit(&format!(
            "{} = bitcast {{ i64, i64, i8* }}* {} to i8*",
            table_raw, dict_op
        ));
        let dst = self.fresh();
        self.emit(&format!(
            "{} = call i64 @pyx86_dict_i64_lookup(i8* {}, i64 {})",
            dst, table_raw, key_op
        ));
        dst
    }

    fn lower_dict_has(&mut self, dict: &TypedExpr, key: &TypedExpr) -> String {
        let dict_op = self.lower(dict);
        let key_op = self.lower(key);
        let table_raw = self.fresh();
        self.emit(&format!(
            "{} = bitcast {{ i64, i64, i8* }}* {} to i8*",
            table_raw, dict_op
        ));
        let dst = self.fresh();
        self.emit(&format!(
            "{} = call i1 @pyx86_dict_i64_has(i8* {}, i64 {})",
            dst, table_raw, key_op
        ));
        dst
    }

    fn lower_dict_len(&mut self, dict: &TypedExpr) -> String {
        let dict_op = self.lower(dict);
        let p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 0",
            p, dict_op
        ));
        let dst = self.fresh();
        self.emit(&format!("{} = load i64, i64* {}", dst, p));
        dst
    }

    /// Read a field from a class instance. Just GEP + load.
    fn lower_field_get(&mut self, obj: &TypedExpr, field_index: usize, result_ty: Type) -> String {
        let obj_op = self.lower(obj);
        let class_id = match obj.ty {
            Type::Class(id) => id,
            _ => panic!("internal: field_get on non-class"),
        };
        let class_llvm = llvm_ty(obj.ty);
        // class_llvm is "{ ... }*"; strip trailing '*' to get the struct type.
        let struct_ty = class_llvm.trim_end_matches('*').trim_end().to_string();
        let _ = class_id;
        let p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {sty}, {sty}* {}, i32 0, i32 {}",
            p, obj_op, field_index,
            sty = struct_ty
        ));
        let dst = self.fresh();
        self.emit(&format!(
            "{} = load {ft}, {ft}* {}",
            dst, p,
            ft = llvm_ty(result_ty)
        ));
        dst
    }

    /// `Foo(args...)` — allocate the struct on the heap and call
    /// `Foo.__init__(self_ptr, args...)`. The init function returns
    /// the same self_ptr (we synthesize that in check.rs).
    fn lower_class_new(&mut self, class_id: crate::hir::ClassId, args: &[TypedExpr]) -> String {
        let class_llvm = llvm_ty(Type::Class(class_id));
        let struct_ty = class_llvm.trim_end_matches('*').trim_end().to_string();
        let bytes = class_byte_size(class_id);
        let raw = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 {})", raw, bytes));
        let self_p = self.fresh();
        self.emit(&format!(
            "{} = bitcast i8* {} to {sty}*",
            self_p, raw,
            sty = struct_ty
        ));
        // Call __init__(self_ptr, args...). The init signature has
        // self as the first param.
        let arg_ops: Vec<String> = args.iter().map(|a| self.lower(a)).collect();
        let mut call_args: Vec<String> =
            vec![format!("{sty}* {}", self_p, sty = struct_ty)];
        for (op, ty) in arg_ops.iter().zip(args.iter().map(|a| a.ty)) {
            call_args.push(format!("{} {}", llvm_ty(ty), op));
        }
        let dst = self.fresh();
        self.emit(&format!(
            "{} = call {sty}* @py_{}.__init__({})",
            dst,
            class_id.name(),
            call_args.join(", "),
            sty = struct_ty
        ));
        dst
    }

    /// `obj.field = value` — GEP + store.
    fn lower_set_field(&mut self, obj: &TypedExpr, field_index: usize, value: &TypedExpr) {
        let obj_op = self.lower(obj);
        let value_op = self.lower(value);
        let class_llvm = llvm_ty(obj.ty);
        let struct_ty = class_llvm.trim_end_matches('*').trim_end().to_string();
        let p = self.fresh();
        self.emit(&format!(
            "{} = getelementptr {sty}, {sty}* {}, i32 0, i32 {}",
            p, obj_op, field_index,
            sty = struct_ty
        ));
        self.emit(&format!(
            "store {ft} {}, {ft}* {}",
            value_op, p,
            ft = llvm_ty(value.ty)
        ));
    }

    /// `container[key] = value` for either `Type::Dict` or `Type::List`.
    fn lower_set_subscript(&mut self, container: &TypedExpr, key: &TypedExpr, value: &TypedExpr) {
        match container.ty {
            Type::Dict(_) => {
                let dict_op = self.lower(container);
                let key_op = self.lower(key);
                let value_op = self.lower(value);
                let table_raw = self.fresh();
                self.emit(&format!(
                    "{} = bitcast {{ i64, i64, i8* }}* {} to i8*",
                    table_raw, dict_op
                ));
                self.emit(&format!(
                    "call void @pyx86_dict_i64_insert(i8* {}, i64 {}, i64 {})",
                    table_raw, key_op, value_op
                ));
            }
            Type::List(id) => {
                let elem_llvm = llvm_ty(id.elem());
                let list_op = self.lower(container);
                let idx_op = self.lower(key);
                let value_op = self.lower(value);
                let data_p = self.fresh();
                self.emit(&format!(
                    "{} = getelementptr {{ i64, i64, i8* }}, {{ i64, i64, i8* }}* {}, i32 0, i32 2",
                    data_p, list_op
                ));
                let data_raw = self.fresh();
                self.emit(&format!("{} = load i8*, i8** {}", data_raw, data_p));
                let data_typed = self.fresh();
                self.emit(&format!(
                    "{} = bitcast i8* {} to {}*",
                    data_typed, data_raw, elem_llvm
                ));
                let slot = self.fresh();
                self.emit(&format!(
                    "{} = getelementptr {ety}, {ety}* {}, i64 {}",
                    slot, data_typed, idx_op, ety = elem_llvm
                ));
                self.emit(&format!(
                    "store {ety} {}, {ety}* {}",
                    value_op, slot, ety = elem_llvm
                ));
            }
            other => panic!("internal: SetSubscript on unsupported type {:?}", other),
        }
    }

    fn lower_math_call(&mut self, intrinsic: &str, arg: &TypedExpr) -> String {
        let v = self.lower(arg);
        let dst = self.fresh();
        // The needed declaration is added once by the wrapper module (we
        // emit declarations for all math intrinsics regardless of use;
        // LLVM strips unused decls).
        self.emit(&format!(
            "{} = call double @{}(double {})",
            dst, intrinsic, v
        ));
        dst
    }

    fn lower_str_lit(&mut self, s: &str) -> String {
        let idx = STR_LITS.with(|sl| {
            let mut sl = sl.borrow_mut();
            // Dedupe: if literal already exists, reuse its id.
            for (i, existing) in sl.iter().enumerate() {
                if existing == s {
                    return i;
                }
            }
            let id = sl.len();
            sl.push(s.to_string());
            id
        });
        let len = s.len() as i64;
        let n = s.len() + 1;
        let data = self.fresh();
        // Get a `i8*` to the global's first byte.
        self.emit(&format!(
            "{} = getelementptr inbounds [{n} x i8], [{n} x i8]* @.str.{}, i64 0, i64 0",
            data,
            idx,
            n = n
        ));
        // Build {len, data} struct.
        let s0 = self.fresh();
        self.emit(&format!(
            "{} = insertvalue {{ i64, i8* }} undef, i64 {}, 0",
            s0, len
        ));
        let s1 = self.fresh();
        self.emit(&format!(
            "{} = insertvalue {{ i64, i8* }} {}, i8* {}, 1",
            s1, s0, data
        ));
        s1
    }

    fn lower_str_concat(&mut self, lhs: &TypedExpr, rhs: &TypedExpr) -> String {
        let l = self.lower(lhs);
        let r = self.lower(rhs);
        let lhs_len = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 0", lhs_len, l));
        let lhs_data = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 1", lhs_data, l));
        let rhs_len = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 0", rhs_len, r));
        let rhs_data = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 1", rhs_data, r));
        let total = self.fresh();
        self.emit(&format!("{} = add i64 {}, {}", total, lhs_len, rhs_len));
        let new_data = self.fresh();
        self.emit(&format!("{} = call i8* @malloc(i64 {})", new_data, total));
        // memcpy(new_data, lhs_data, lhs_len)
        self.emit(&format!(
            "call void @llvm.memcpy.p0i8.p0i8.i64(i8* {}, i8* {}, i64 {}, i1 false)",
            new_data, lhs_data, lhs_len
        ));
        let mid = self.fresh();
        self.emit(&format!(
            "{} = getelementptr i8, i8* {}, i64 {}",
            mid, new_data, lhs_len
        ));
        // memcpy(new_data + lhs_len, rhs_data, rhs_len)
        self.emit(&format!(
            "call void @llvm.memcpy.p0i8.p0i8.i64(i8* {}, i8* {}, i64 {}, i1 false)",
            mid, rhs_data, rhs_len
        ));
        let s0 = self.fresh();
        self.emit(&format!(
            "{} = insertvalue {{ i64, i8* }} undef, i64 {}, 0",
            s0, total
        ));
        let s1 = self.fresh();
        self.emit(&format!(
            "{} = insertvalue {{ i64, i8* }} {}, i8* {}, 1",
            s1, s0, new_data
        ));
        s1
    }

    /// Format an integer/bool to a `{ i64, i8* }` str struct.
    /// Dispatches on `inner.ty`. Str-typed inputs are not passed here
    /// (check.rs unwraps them).
    fn lower_format_to_str(&mut self, inner: &TypedExpr) -> String {
        match inner.ty {
            Type::I64 => {
                let v = self.lower(inner);
                let dst = self.fresh();
                self.emit(&format!(
                    "{} = call {{ i64, i8* }} @pyx86_i64_to_str(i64 {})",
                    dst, v
                ));
                dst
            }
            Type::I8 | Type::I16 | Type::I32 => {
                let v = self.lower(inner);
                let widened = self.fresh();
                self.emit(&format!(
                    "{} = sext {ity} {} to i64",
                    widened, v, ity = llvm_ty(inner.ty)
                ));
                let dst = self.fresh();
                self.emit(&format!(
                    "{} = call {{ i64, i8* }} @pyx86_i64_to_str(i64 {})",
                    dst, widened
                ));
                dst
            }
            Type::Bool => {
                let v = self.lower(inner);
                // Build both candidate str structs, select.
                let true_data = self.fresh();
                self.emit(&format!(
                    "{} = getelementptr inbounds [5 x i8], [5 x i8]* @.s_true, i64 0, i64 0",
                    true_data
                ));
                let false_data = self.fresh();
                self.emit(&format!(
                    "{} = getelementptr inbounds [6 x i8], [6 x i8]* @.s_false, i64 0, i64 0",
                    false_data
                ));
                let chosen_len = self.fresh();
                self.emit(&format!(
                    "{} = select i1 {}, i64 4, i64 5",
                    chosen_len, v
                ));
                let chosen_data = self.fresh();
                self.emit(&format!(
                    "{} = select i1 {}, i8* {}, i8* {}",
                    chosen_data, v, true_data, false_data
                ));
                let s0 = self.fresh();
                self.emit(&format!(
                    "{} = insertvalue {{ i64, i8* }} undef, i64 {}, 0",
                    s0, chosen_len
                ));
                let s1 = self.fresh();
                self.emit(&format!(
                    "{} = insertvalue {{ i64, i8* }} {}, i8* {}, 1",
                    s1, s0, chosen_data
                ));
                s1
            }
            other => panic!("internal: lower_format_to_str on unsupported type {:?}", other),
        }
    }

    fn lower_str_len(&mut self, s: &TypedExpr) -> String {
        let v = self.lower(s);
        let dst = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 0", dst, v));
        dst
    }

    fn lower_str_eq(&mut self, lhs: &TypedExpr, rhs: &TypedExpr, negated: bool) -> String {
        let l = self.lower(lhs);
        let r = self.lower(rhs);
        let lhs_len = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 0", lhs_len, l));
        let rhs_len = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 0", rhs_len, r));

        let result_addr = self.fresh();
        self.emit(&format!("{} = alloca i1", result_addr));
        // Default: not equal (i1 0)
        self.emit(&format!("store i1 0, i1* {}", result_addr));

        let len_eq = self.fresh();
        self.emit(&format!("{} = icmp eq i64 {}, {}", len_eq, lhs_len, rhs_len));

        let id = self.next_block_id;
        self.next_block_id += 1;
        let memcmp_lbl = format!("streq.memcmp.{}", id);
        let merge_lbl = format!("streq.merge.{}", id);
        self.emit(&format!("br i1 {}, label %{}, label %{}", len_eq, memcmp_lbl, merge_lbl));
        self.block_terminated = true;

        self.open_block(&memcmp_lbl);
        let lhs_data = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 1", lhs_data, l));
        let rhs_data = self.fresh();
        self.emit(&format!("{} = extractvalue {{ i64, i8* }} {}, 1", rhs_data, r));
        let cmp_res = self.fresh();
        self.emit(&format!(
            "{} = call i32 @memcmp(i8* {}, i8* {}, i64 {})",
            cmp_res, lhs_data, rhs_data, lhs_len
        ));
        let eq = self.fresh();
        self.emit(&format!("{} = icmp eq i32 {}, 0", eq, cmp_res));
        self.emit(&format!("store i1 {}, i1* {}", eq, result_addr));
        self.emit(&format!("br label %{}", merge_lbl));
        self.block_terminated = true;

        self.open_block(&merge_lbl);
        let result = self.fresh();
        self.emit(&format!("{} = load i1, i1* {}", result, result_addr));
        if negated {
            let neg = self.fresh();
            self.emit(&format!("{} = xor i1 {}, true", neg, result));
            neg
        } else {
            result
        }
    }

    fn lower_list_len(&mut self, list: &TypedExpr) -> String {
        let list_op = self.lower(list);
        self.list_load_len(&list_op)
    }

    fn lower_call(&mut self, callee: &str, args: &[TypedExpr], result_ty: Type) -> String {
        let arg_ops: Vec<(String, Type)> = args
            .iter()
            .map(|a| (self.lower(a), a.ty))
            .collect();
        let dst = self.fresh();
        let call_args = arg_ops
            .iter()
            .map(|(op, ty)| format!("{} {}", llvm_ty(*ty), op))
            .collect::<Vec<_>>()
            .join(", ");
        self.emit(&format!(
            "{} = call {} @py_{}({})",
            dst,
            llvm_ty(result_ty),
            callee,
            call_args
        ));
        dst
    }
}

fn llvm_icmp_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Lt => "slt",
        CmpOp::Le => "sle",
        CmpOp::Gt => "sgt",
        CmpOp::Ge => "sge",
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
    }
}

fn llvm_fcmp_op(op: CmpOp) -> &'static str {
    // Use *ordered* predicates ("o*") so NaN compares as false in
    // every direction — matches Python's behaviour for nan
    // comparisons (which all return False except != which returns True).
    // For ne we use "one" (ordered, not equal) so nan != nan = false;
    // Python nan != nan == True. Documented divergence.
    match op {
        CmpOp::Lt => "olt",
        CmpOp::Le => "ole",
        CmpOp::Gt => "ogt",
        CmpOp::Ge => "oge",
        CmpOp::Eq => "oeq",
        CmpOp::Ne => "one",
    }
}

/// Format an f64 as an LLVM IR float literal. LLVM accepts decimal
/// notation but the canonical (always-accepted) form is hex: `0xH...`
/// for half, `0x...` for double. Use Rust's debug-friendly format
/// which matches LLVM's expected double syntax.
fn format_f64_literal(v: f64) -> String {
    // LLVM accepts standard decimal float literals like `1.0`, `1.5e2`.
    // For special values:
    if v.is_nan() {
        // Use hex-encoded NaN.
        return "0x7FF8000000000000".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "0x7FF0000000000000".to_string()
        } else {
            "0xFFF0000000000000".to_string()
        };
    }
    // For finite values, use Rust's default Display which gives an
    // exact decimal representation (Rust uses Grisu-like shortest).
    // Make sure it has a decimal point so LLVM parses it as float.
    let s = format!("{:?}", v); // "{:?}" guarantees the trailing .0 for integer-valued floats
    s
}

fn collect_locals(body: &[Stmt]) -> Vec<(String, Type)> {
    let mut out: Vec<(String, Type)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    walk_stmts(body, &mut out, &mut seen);
    out
}

fn walk_stmts(stmts: &[Stmt], out: &mut Vec<(String, Type)>, seen: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Let { name, value } => {
                if seen.insert(name.clone()) {
                    out.push((name.clone(), value.ty));
                }
                walk_expr(value, out, seen);
            }
            Stmt::Return { value } => walk_expr(value, out, seen),
            Stmt::Break | Stmt::Continue => {}
            Stmt::ListAppend { list, value } => {
                walk_expr(list, out, seen);
                walk_expr(value, out, seen);
            }
            Stmt::SetField { obj, value, .. } => {
                walk_expr(obj, out, seen);
                walk_expr(value, out, seen);
            }
            Stmt::SetSubscript { container, key, value } => {
                walk_expr(container, out, seen);
                walk_expr(key, out, seen);
                walk_expr(value, out, seen);
            }
            Stmt::ExprStmt(e) => walk_expr(e, out, seen),
            Stmt::If { cond, then_body, else_body } => {
                walk_expr(cond, out, seen);
                walk_stmts(then_body, out, seen);
                walk_stmts(else_body, out, seen);
            }
            Stmt::While { cond, body } => {
                walk_expr(cond, out, seen);
                walk_stmts(body, out, seen);
            }
        }
    }
}

fn walk_expr(te: &TypedExpr, out: &mut Vec<(String, Type)>, seen: &mut HashSet<String>) {
    match &te.expr {
        Expr::ConstI64(_) | Expr::ConstF64(_) | Expr::ConstBool(_) | Expr::Var(_) => {}
        Expr::BinOp { lhs, rhs, .. }
        | Expr::Cmp { lhs, rhs, .. }
        | Expr::BoolOp { lhs, rhs, .. }
        | Expr::ListConcat { lhs, rhs } => {
            walk_expr(lhs, out, seen);
            walk_expr(rhs, out, seen);
        }
        Expr::UnaryOp { operand, .. } | Expr::Not(operand) | Expr::Coerce { inner: operand } => {
            walk_expr(operand, out, seen);
        }
        Expr::CmpChain { first, rest } => {
            walk_expr(first, out, seen);
            for (_, e) in rest {
                walk_expr(e, out, seen);
            }
        }
        Expr::Call { args, .. } => {
            for a in args {
                walk_expr(a, out, seen);
            }
        }
        Expr::TupleLit { elements } | Expr::ListLit { elements } => {
            for e in elements {
                walk_expr(e, out, seen);
            }
        }
        Expr::TupleIndex { tuple, .. } => walk_expr(tuple, out, seen),
        Expr::ListIndex { list, index } => {
            walk_expr(list, out, seen);
            walk_expr(index, out, seen);
        }
        Expr::ListLen { list } => walk_expr(list, out, seen),
        Expr::DoBlock { stmts, result } => {
            walk_stmts(stmts, out, seen);
            walk_expr(result, out, seen);
        }
        Expr::StrLit(_) => {}
        Expr::StrConcat { lhs, rhs } | Expr::StrEq { lhs, rhs, .. } => {
            walk_expr(lhs, out, seen);
            walk_expr(rhs, out, seen);
        }
        Expr::StrLen { s } => walk_expr(s, out, seen),
        Expr::FormatToStr { inner } => walk_expr(inner, out, seen),
        Expr::MathCall { arg, .. } => walk_expr(arg, out, seen),
        Expr::DictLit { entries } => {
            for (k, v) in entries {
                walk_expr(k, out, seen);
                walk_expr(v, out, seen);
            }
        }
        Expr::DictGet { dict, key } | Expr::DictHas { dict, key } => {
            walk_expr(dict, out, seen);
            walk_expr(key, out, seen);
        }
        Expr::DictLen { dict } => walk_expr(dict, out, seen),
        Expr::FieldGet { obj, .. } => walk_expr(obj, out, seen),
        Expr::ClassNew { args, .. } => {
            for a in args {
                walk_expr(a, out, seen);
            }
        }
    }
}

fn sanitize_module_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{Expr, Function, Param, Program, Stmt, Type, TypedExpr};

    fn const_i64(v: i64) -> TypedExpr {
        TypedExpr::new(Type::I64, Expr::ConstI64(v))
    }

    fn make_program(params: Vec<&str>, body: Vec<Stmt>) -> Program {
        Program {
            functions: vec![Function {
                name: "main".into(),
                params: params
                    .into_iter()
                    .map(|n| Param { name: n.into(), ty: Type::I64 })
                    .collect(),
                return_ty: Type::I64,
                body,
            }],
        }
    }

    #[test]
    fn no_locals_emits_a_simple_return() {
        let ll = emit_ll(
            &make_program(vec![], vec![Stmt::Return { value: const_i64(42) }]),
            "test",
        );
        assert!(ll.contains("ret i64 42"));
    }

    #[test]
    fn local_binds_then_returns() {
        let ll = emit_ll(
            &make_program(
                vec!["a"],
                vec![
                    Stmt::Let {
                        name: "x".into(),
                        value: TypedExpr::new(
                            Type::I64,
                            Expr::BinOp {
                                op: BinOp::Add,
                                lhs: Box::new(TypedExpr::new(Type::I64, Expr::Var("a".into()))),
                                rhs: Box::new(const_i64(1)),
                            },
                        ),
                    },
                    Stmt::Return {
                        value: TypedExpr::new(Type::I64, Expr::Var("x".into())),
                    },
                ],
            ),
            "test",
        );
        assert!(ll.contains("alloca i64"));
        assert!(ll.contains("store i64"));
        assert!(ll.contains("load i64, i64* %x.addr"));
    }
}
