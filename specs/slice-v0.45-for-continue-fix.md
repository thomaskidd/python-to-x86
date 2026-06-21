# Spec: slice v0.45 — fix `continue` in `for` loops (while latch)

> Status: in progress.

## The bug

`for` loops desugar to a `while` with the loop advance (the range
counter bump or the list index bump) appended to the **end of the loop
body**. `continue` was lowered to a branch back to the loop **header**,
so it jumped *over* the advance:

```python
for i in range(n):
    if i % 2 == 0:
        continue        # jumps to header without incrementing i
    total += i
```

For an even `i` this is an infinite, side-effect-free loop. LLVM's
forward-progress assumption then lets the optimizer **delete** it, so the
compiled program silently produced a wrong answer (`0`) instead of
hanging — which is why no existing test caught it (there was no
`for`+`continue` test, and a hang would at least have been visible).

## The fix: a loop latch

`Stmt::While` gains an `update: Vec<Stmt>` field — the **loop latch**.
Codegen emits a dedicated latch block:

```
header:  cond ? body : exit
body:    <body>            ; continue -> latch, break -> exit
         -> latch          (fallthrough)
latch:   <update>          ; the loop advance
         -> header
exit:
```

`continue` now targets the **latch**, so `update` always runs before the
next condition test. Plain `while` loops pass an empty `update` (the
latch is an empty `br header`, folded away by LLVM).

For-loop and comprehension/`sum`/`any`/`all` desugars move their counter
/ index bump from the body tail into `update`. This also makes a future
fused `for x in (… for y in …)` (v0.46) honour `continue` correctly,
since it reuses the same loop scaffold.

## Files changed

- `crates/pyx86/src/hir.rs` — `While { cond, body, update }`.
- `crates/pyx86/src/codegen.rs` — latch block; `continue` targets it;
  var-walk recurses into `update`.
- `crates/pyx86/src/check.rs` — for-range, for-list, plain-while, and
  `build_comp_loop` (range + list) updated to put the advance in
  `update`.
- `tests/correctness/for_continue_*`.
- This file.

## Test programs (tier 2)

| Test | Purpose |
|---|---|
| `for_continue_range` | `continue` skipping even `i` in a range loop |
| `for_continue_list` | `continue` skipping a sentinel in a list loop |
| `for_continue_nested` | `continue` in an inner loop, outer unaffected |
| `for_break_continue` | `break` and `continue` interacting in one loop |
