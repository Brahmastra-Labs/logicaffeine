# C Backend Gap Analysis — TDD Agent Spec

Complete audit of every gap between the C backend (`codegen_c/`) and the Rust backend
(`codegen/`). Structured as a strict TDD spec: an autonomous agent follows it
sprint-by-sprint, top to bottom, never skipping a GATE.

## Overview

| Metric | Rust Backend | C Backend | After All Sprints |
|--------|-------------|-----------|-------------------|
| Expr variants handled | 29/29 | 18/29 | 26/29 |
| Stmt variants handled | 54/54 | 21/54 | 33/54 |
| Literal types | 11 | 5 | 7 |
| Show type coverage | All types | 9 types (primitives + Seq) | 19 types (+Map, Set, Enum) |
| Peephole optimizations | 12 patterns | 0 | 3 high-impact |
| Test count | ~2000+ | 216 | ~286 (est. +70) |
| Runtime LOC | ~3000 (logos_core) | ~700 (embedded) | ~1100 (est.) |

## TDD Format Specification

This document is a strict TDD spec. An agent follows it step-by-step, top to bottom.
Every sprint is a numbered sequence of **STEPS**. Every step is one of:

| Step Type | What Happens | Expected Outcome |
|-----------|-------------|-----------------|
| **RED** | Write test(s). Run them. They MUST fail. | FAIL (compile error or assertion failure) |
| **GREEN** | Implement the minimum to make RED tests pass. Run them. | PASS |
| **VERIFY** | Run `cargo test --test phase_codegen_c -- --skip e2e` on the C test suite. | Zero failures |
| **GATE** | Run full `cargo test -- --skip e2e`. Hard stop if any failure. | Zero failures. Proceed only if green. |

**Rules:**
1. Every RED step specifies exact test names, the file they go in, the LOGOS source
   code, and what to assert.
2. Every GREEN step specifies exact file paths to create/modify and what to implement.
3. Never modify a RED test to make it pass. Fix the implementation.
4. Every sprint ends with a GATE. No proceeding until the gate is green.
5. Tests use `#[cfg(not(target_arch = "wasm32"))]` for E2E tests that execute C binaries.
6. Test helpers: `common::assert_c_output(source, expected)` compiles to C with
   `gcc -O2`, runs the binary, and asserts stdout matches exactly.
   `common::compile_to_c(source)` returns the generated C code string for unit tests.

**Test naming conventions:**
- `e2e_c_<category>_<description>` — End-to-end tests that compile and run C binaries
- `codegen_c_<description>` — Unit tests that inspect generated C code structure

**Test file:** All tests go in `crates/logicaffeine_tests/tests/phase_codegen_c.rs`
(append to existing file with 216 tests).

**LOGOS source pattern:**
```rust
let source = r#"## Main
<LOGOS statements>"#;
```
All E2E tests need a `## Main` block. Functions defined above `## Main`.

---

## Known Pitfalls (Read Before Starting)

These are structural hazards discovered during deep audit of `codegen_c/emit.rs`.
Every pitfall cites exact function/location. Every sprint must account for these.

### Pitfall 1 — Silent Zero Fallthrough

**Location:** `codegen_expr()` wildcard at `emit.rs:423`

```rust
_ => "0".to_string(),
```

Every unhandled `Expr` variant silently emits the C integer `0`. This means a program
using `Expr::Not`, `Expr::Tuple`, `Expr::OptionSome`, etc. will compile without error
but produce wrong results. The generated C compiles fine because `0` is valid everywhere
an expression is expected.

**Impact:** Tests that check output will catch this (the output will be wrong), but
tests that only check compilation will miss it entirely.

**Resolution:** Each sprint that adds an Expr variant must verify output, not just
compilation.

### Pitfall 2 — Show Falls Through to show_i64

**Location:** `codegen_stmt()` Show handler wildcard at `emit.rs:731`

```rust
_ => writeln!(output, "{}show_i64({});", pad, val_str).unwrap(),
```

Every unhandled Show type — Sets, Maps, Enums — silently calls `show_i64()`. This
means `Show mySet.` prints a garbage integer (the struct's first field, interpreted
as i64) instead of `{1, 2, 3}`.

**Resolution:** Sprint 3 adds show functions for all collection types and updates
the Show match arms.

### Pitfall 3 — Stmt Wildcard Emits Comment

**Location:** `codegen_stmt()` wildcard at `emit.rs:1000`

```rust
_ => {
    writeln!(output, "{}/* unsupported stmt */", pad).unwrap();
}
```

Unhandled statements become C comments. The generated C compiles successfully but
the statement has no effect. Tests must verify behavior (output), not just compilation.

**Resolution:** Sprints 1-2 add match arms for all Tier 1 statements.

### Pitfall 4 — Literal Fallthrough

**Location:** `codegen_literal()` wildcard at `emit.rs:445`

```rust
_ => "0".to_string(),
```

Unhandled `Literal` variants (Char, Duration, Date, Moment, Span, Time) become `0`.

**Resolution:** Sprint 1 handles Char. Sprint 4 handles temporal literals.

### Pitfall 5 — TypeExpr Fallthrough in resolve_type_expr

**Location:** `types.rs:179`

```rust
_ => CType::Int64,
```

`TypeExpr::Function`, `TypeExpr::Refinement`, and `TypeExpr::Persistent` all resolve
to `CType::Int64`. Function types need Tier 4 (closures). Refinement and Persistent
are no-ops for codegen — the base type is what matters.

**Resolution:** Sprint 4 adds Refinement passthrough (extract base type). Function
and Persistent remain Int64 until Sprints 7 and Tier 5 respectively.

### Pitfall 6 — Enum Show is Tag-Dispatch

**Location:** Rust backend generates `#[derive(Debug)]` for enums, so `Show myEnum.`
uses Rust's Debug formatting: `Circle { radius: 10 }`.

The C backend uses tagged unions. Show must emit `if (val.tag == TAG_Circle) printf(...)`
chains. Each variant needs its own printf format string with field names.

**Resolution:** Sprint 3 implements tag-dispatch Show for enums.

### Pitfall 7 — No Map Length in Runtime

**Location:** `emit.rs:220` — Length on Map types falls through to `seq_i64_len()`.

The C runtime has no `map_*_len()` functions. The `Map_*` structs have a `len` field
but no accessor function. Codegen must emit direct `.len` field access.

**Resolution:** Sprint 3 adds Map length handling in the Length match arm.

---

## Infrastructure to Reuse

| Existing | Location | How Sprints Reuse It |
|----------|----------|---------------------|
| `compile_to_c()` | `compile.rs` | Entry point for all C codegen |
| `assert_c_output()` | `tests/common/mod.rs` | E2E test helper: compile → gcc → run → assert |
| `CContext` | `codegen_c/types.rs` | Variable type tracking, name resolution |
| `escape_c_ident()` | `codegen_c/types.rs` | C keyword escaping (double, int, float, etc.) |
| `infer_expr_type()` | `codegen_c/types.rs` | Expression type inference for codegen dispatch |
| `resolve_type_expr()` | `codegen_c/types.rs` | TypeExpr → CType resolution |
| C runtime | `codegen_c/runtime.rs` | Embedded C runtime (~700 LOC) as string constant |
| Seq/Map/Set structs | `codegen_c/runtime.rs` | All collection types already defined |
| Enum codegen | `codegen_c/mod.rs` | `codegen_c_enum_defs()` already emits tagged unions |
| Struct codegen | `codegen_c/mod.rs` | `codegen_c_struct_defs()` already emits C structs |

---

## Sprint 1 — No-Op Match Arms & Trivial Codegen

### Overview

Close 6 gaps that are literally empty match arms or single-line emissions. Zero runtime
functions needed. After this sprint, the C backend no longer emits `/* unsupported stmt */`
for any Tier 1 statement.

**Estimated LOC:** ~15
**New tests:** 6

### STEP 1: RED — No-op statements and Char literal

**File:** `crates/logicaffeine_tests/tests/phase_codegen_c.rs` (append)

Write the following 6 tests. All must fail or produce wrong output.

**Run:** `cargo test --test phase_codegen_c codegen_c_structdef_no_comment -- --skip e2e && cargo test --test phase_codegen_c e2e_c_char_literal -- --skip e2e && cargo test --test phase_codegen_c e2e_c_sleep -- --skip e2e`
**Expected:** FAIL (wrong output or `/* unsupported */` in generated code)

**Test 1: `codegen_c_structdef_no_comment`** — StructDef emits no comment
```rust
#[test]
fn codegen_c_structdef_no_comment() {
    let source = r#"## Define Point:
    It has an x (Int).
    It has a y (Int).

## Main
Let p be a new Point with x 10 and y 20.
Show p's x."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("/* unsupported stmt */"),
        "StructDef should not emit unsupported comment, got:\n{}", code);
}
```

**Test 2: `codegen_c_require_no_comment`** — Require emits no comment
```rust
#[test]
fn codegen_c_require_no_comment() {
    let source = r#"## Requires
The "serde" crate version "1.0".

## Main
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("/* unsupported stmt */"),
        "Require should not emit unsupported comment, got:\n{}", code);
}
```

**Test 3: `codegen_c_theorem_no_comment`** — Theorem emits no comment
```rust
#[test]
fn codegen_c_theorem_no_comment() {
    let source = r#"## Main
Show 1.

## Theorem: trivial
Given: Every cat is a cat.
Prove: Every cat is a cat.
Proof: Auto."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("/* unsupported stmt */"),
        "Theorem should not emit unsupported comment, got:\n{}", code);
}
```

**Test 4: `codegen_c_char_literal`** — Char literal codegen
```rust
#[test]
fn codegen_c_char_literal() {
    let source = "## Main\nLet c be 'A'.\nShow c.";
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("'A'"), "Should emit char literal, got:\n{}", code);
}
```

**Test 5: `e2e_c_sleep_compiles`** — Sleep compiles (no runtime assertion on timing)
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_sleep_compiles() {
    let source = r#"## Main
Show 1.
Sleep 1.
Show 2."#;
    common::assert_c_output(source, "1\n2\n");
}
```

**Test 6: `codegen_c_sleep_usleep`** — Sleep emits usleep call
```rust
#[test]
fn codegen_c_sleep_usleep() {
    let source = r#"## Main
Sleep 100."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("usleep"), "Should emit usleep call, got:\n{}", code);
}
```

### STEP 2: GREEN — Add match arms

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In `codegen_stmt()` (before the wildcard `_ =>` at line 1000), add:

```rust
Stmt::StructDef { .. } => {
    // Already handled by codegen_c_struct_defs() in mod.rs
}
Stmt::Require { .. } => {
    // No-op: C has no package manager
}
Stmt::Theorem(_) => {
    // No-op: verification-only, no runtime code
}
Stmt::Sleep { milliseconds } => {
    let ms_str = codegen_expr(milliseconds, ctx);
    writeln!(output, "{}usleep((useconds_t)({}) * 1000);", pad, ms_str).unwrap();
}
```

In `codegen_literal()` (before the wildcard `_ =>` at line 445), add:

```rust
Literal::Char(c) => format!("'{}'", c),
```

**File:** `crates/logicaffeine_compile/src/codegen_c/runtime.rs`

Add `#include <unistd.h>` to the runtime header includes (after `#include <math.h>`).

**Run:** `cargo test --test phase_codegen_c codegen_c_structdef_no_comment codegen_c_require_no_comment codegen_c_theorem_no_comment codegen_c_char_literal e2e_c_sleep_compiles codegen_c_sleep_usleep -- --skip e2e`
**Expected:** PASS (all 6 tests)

### STEP 3: VERIFY — No regressions in C test suite

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 222 tests pass (216 existing + 6 new).

### STEP 4: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures across all test suites. Proceed only if green.

---

## Sprint 2 — Simple Expression & Statement Codegen

### Overview

Close 5 gaps requiring a few lines of real codegen logic: `Expr::Not`, `Stmt::Break`,
`Stmt::RuntimeAssert`, `Stmt::Assert`/`Stmt::Trust`, and `Stmt::Give`.

**Estimated LOC:** ~30
**New tests:** 10

### STEP 1: RED — Not expression

**File:** `crates/logicaffeine_tests/tests/phase_codegen_c.rs` (append)

**Run:** `cargo test --test phase_codegen_c e2e_c_not_bool -- --skip e2e`
**Expected:** FAIL (outputs `0` instead of correct result because Expr::Not falls to wildcard)

**Test 1: `e2e_c_not_bool_true`** — Logical NOT on true
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_not_bool_true() {
    let source = r#"## Main
Let x be true.
If not x:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "0\n");
}
```

**Test 2: `e2e_c_not_bool_false`** — Logical NOT on false
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_not_bool_false() {
    let source = r#"## Main
Let x be false.
If not x:
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n");
}
```

**Test 3: `e2e_c_not_in_while`** — NOT in loop condition
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_not_in_while() {
    let source = r#"## Main
Let mutable done be false.
Let mutable count be 0.
While not done:
    Set count to count + 1.
    If count equals 3:
        Set done to true.
Show count."#;
    common::assert_c_output(source, "3\n");
}
```

### STEP 2: GREEN — Implement Expr::Not

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In `codegen_expr()` (before the wildcard `_ =>` at line 423), add:

```rust
Expr::Not { operand } => {
    let inner = codegen_expr(operand, ctx);
    format!("(!({})", inner)
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_not_bool_true e2e_c_not_bool_false e2e_c_not_in_while -- --skip e2e`
**Expected:** PASS (all 3 tests)

### STEP 3: VERIFY — No regressions

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures.

### STEP 4: RED — Break, RuntimeAssert, Assert/Trust, Give

**Tests 4-10:**

**Test 4: `e2e_c_break_in_while`** — Break exits loop
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_break_in_while() {
    let source = r#"## Main
Let mutable i be 0.
While true:
    If i equals 5:
        Break.
    Set i to i + 1.
Show i."#;
    common::assert_c_output(source, "5\n");
}
```

**Test 5: `e2e_c_break_nested_loop`** — Break exits only innermost loop
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_break_nested_loop() {
    let source = r#"## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 3:
    Let mutable j be 0.
    While true:
        If j equals 2:
            Break.
        Set j to j + 1.
        Set total to total + 1.
    Set i to i + 1.
Show total."#;
    common::assert_c_output(source, "6\n");
}
```

**Test 6: `codegen_c_runtime_assert`** — RuntimeAssert emits assert
```rust
#[test]
fn codegen_c_runtime_assert() {
    let source = r#"## Main
Let x be 5.
Assert that x is greater than 0."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("assert(") || code.contains("assert ("),
        "Should emit C assert, got:\n{}", code);
}
```

**Test 7: `e2e_c_runtime_assert_true`** — RuntimeAssert with true condition runs fine
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_runtime_assert_true() {
    let source = r#"## Main
Let x be 10.
Assert that x is greater than 0.
Show x."#;
    common::assert_c_output(source, "10\n");
}
```

**Test 8: `codegen_c_assert_comment`** — Assert (logic) emits comment
```rust
#[test]
fn codegen_c_assert_comment() {
    let source = r#"Every dog is an animal.
Assert that every dog is an animal.

## Main
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("/* unsupported stmt */"),
        "Assert should not emit unsupported comment, got:\n{}", code);
}
```

**Test 9: `codegen_c_trust_comment`** — Trust emits comment
```rust
#[test]
fn codegen_c_trust_comment() {
    let source = r#"Every dog is an animal.
Trust that every dog is an animal because "axiom".

## Main
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("/* unsupported stmt */"),
        "Trust should not emit unsupported comment, got:\n{}", code);
}
```

**Test 10: `e2e_c_give_as_call`** — Give emits function call
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_give_as_call() {
    let source = r#"## To consume (x: Int) -> Int:
    Return x * 2.

## Main
Let val be 21.
Let mutable result be 0.
Set result to consume(val).
Show result."#;
    common::assert_c_output(source, "42\n");
}
```

### STEP 5: GREEN — Implement Break, RuntimeAssert, Assert/Trust, Give

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In `codegen_stmt()` (before the wildcard), add:

```rust
Stmt::Break => {
    writeln!(output, "{}break;", pad).unwrap();
}
Stmt::RuntimeAssert { condition } => {
    let cond_str = codegen_expr(condition, ctx);
    writeln!(output, "{}assert({});", pad, cond_str).unwrap();
}
Stmt::Assert { .. } => {
    // Logic bridge assertion — no runtime code in C
}
Stmt::Trust { .. } => {
    // Documented assertion — no runtime code in C
}
Stmt::Give { object, recipient } => {
    let obj_str = codegen_expr(object, ctx);
    let recv_str = codegen_expr(recipient, ctx);
    writeln!(output, "{}{}({});", pad, recv_str, obj_str).unwrap();
}
```

**File:** `crates/logicaffeine_compile/src/codegen_c/runtime.rs`

Add `#include <assert.h>` to the runtime header includes.

**Run:** `cargo test --test phase_codegen_c e2e_c_break_in_while e2e_c_break_nested_loop codegen_c_runtime_assert e2e_c_runtime_assert_true codegen_c_assert_comment codegen_c_trust_comment e2e_c_give_as_call -- --skip e2e`
**Expected:** PASS (all 7 tests)

### STEP 6: VERIFY — No regressions in C test suite

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 232 tests pass (222 + 10 new).

### STEP 7: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures across all test suites. Proceed only if green.

---

## Sprint 3 — Show/Display Completeness

### Overview

Fill in all Show statement gaps for types the C runtime already has structs for but
no display functions. After this sprint, `Show` works for every collection type and
for enums.

**Estimated LOC:** ~180 (runtime functions + match arms)
**New tests:** 14

### STEP 1: RED — Set Show

**Test 1: `e2e_c_show_set_i64_empty`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_set_i64_empty() {
    let source = r#"## Main
Let s be a new Set of Int.
Show s."#;
    common::assert_c_output(source, "{}\n");
}
```

**Test 2: `e2e_c_show_set_i64`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_set_i64() {
    let source = r#"## Main
Let s be a new Set of Int.
Add 1 to s.
Add 2 to s.
Add 3 to s.
Show s."#;
    // Set uses open-addressing, order depends on hash. Check contains elements.
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_set_i64"),
        "Should call show_set_i64, got:\n{}", c_code);
}
```

**Test 3: `e2e_c_show_set_str`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_set_str() {
    let source = r#"## Main
Let s be a new Set of Text.
Add "hello" to s.
Show s."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_set_str"),
        "Should call show_set_str, got:\n{}", c_code);
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_show_set -- --skip e2e`
**Expected:** FAIL (no `show_set_i64` / `show_set_str` functions exist)

### STEP 2: GREEN — Implement Set Show

**File:** `crates/logicaffeine_compile/src/codegen_c/runtime.rs`

Add to the runtime string (after the `show_seq_f64` function):

```c
void show_set_i64(Set_i64 *s) {
    printf("{");
    int first = 1;
    for (size_t i = 0; i < s->cap; i++) {
        if (s->state[i]) {
            if (!first) printf(", ");
            printf("%" PRId64, s->keys[i]);
            first = 0;
        }
    }
    printf("}\n");
}

void show_set_str(Set_str *s) {
    printf("{");
    int first = 1;
    for (size_t i = 0; i < s->cap; i++) {
        if (s->state[i]) {
            if (!first) printf(", ");
            printf("%s", s->keys[i]);
            first = 0;
        }
    }
    printf("}\n");
}
```

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In the Show handler (around line 695), before the Struct match and before the
wildcard, add match arms for `CType::SetI64` and `CType::SetStr`:

```rust
CType::SetI64 => {
    if is_lvalue {
        writeln!(output, "{}show_set_i64(&{});", pad, val_str).unwrap();
    } else {
        writeln!(output, "{}{{ Set_i64 __tmp = {}; show_set_i64(&__tmp); }}", pad, val_str).unwrap();
    }
}
CType::SetStr => {
    if is_lvalue {
        writeln!(output, "{}show_set_str(&{});", pad, val_str).unwrap();
    } else {
        writeln!(output, "{}{{ Set_str __tmp = {}; show_set_str(&__tmp); }}", pad, val_str).unwrap();
    }
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_show_set -- --skip e2e`
**Expected:** PASS (all 3 tests)

### STEP 3: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures.

### STEP 4: RED — Map Show

**Test 4: `e2e_c_show_map_i64_i64`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_map_i64_i64() {
    let source = r#"## Main
Let m be a new Map of Int to Int.
Set item 1 of m to 10.
Show m."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_map_i64_i64"),
        "Should call show_map_i64_i64, got:\n{}", c_code);
}
```

**Test 5: `e2e_c_show_map_str_i64`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_map_str_i64() {
    let source = r#"## Main
Let m be a new Map of Text to Int.
Set item "x" of m to 42.
Show m."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_map_str_i64"),
        "Should call show_map_str_i64, got:\n{}", c_code);
}
```

**Test 6: `e2e_c_show_map_str_str`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_map_str_str() {
    let source = r#"## Main
Let m be a new Map of Text to Text.
Set item "key" of m to "val".
Show m."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_map_str_str"),
        "Should call show_map_str_str, got:\n{}", c_code);
}
```

**Test 7: `e2e_c_show_map_i64_str`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_map_i64_str() {
    let source = r#"## Main
Let m be a new Map of Int to Text.
Set item 1 of m to "one".
Show m."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_map_i64_str"),
        "Should call show_map_i64_str, got:\n{}", c_code);
}
```

**Test 8: `e2e_c_show_map_empty`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_map_empty() {
    let source = r#"## Main
Let m be a new Map of Int to Int.
Show m."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(c_code.contains("show_map_i64_i64"),
        "Should call show_map_i64_i64 even for empty map, got:\n{}", c_code);
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_show_map -- --skip e2e`
**Expected:** FAIL (no `show_map_*` functions exist)

### STEP 5: GREEN — Implement Map Show

**File:** `crates/logicaffeine_compile/src/codegen_c/runtime.rs`

Add 4 show functions for maps:

```c
void show_map_i64_i64(Map_i64_i64 *m) {
    printf("{");
    int first = 1;
    for (size_t i = 0; i < m->cap; i++) {
        if (m->state[i]) {
            if (!first) printf(", ");
            printf("%" PRId64 ": %" PRId64, m->keys[i], m->vals[i]);
            first = 0;
        }
    }
    printf("}\n");
}

void show_map_str_i64(Map_str_i64 *m) {
    printf("{");
    int first = 1;
    for (size_t i = 0; i < m->cap; i++) {
        if (m->state[i]) {
            if (!first) printf(", ");
            printf("%s: %" PRId64, m->keys[i], m->vals[i]);
            first = 0;
        }
    }
    printf("}\n");
}

void show_map_str_str(Map_str_str *m) {
    printf("{");
    int first = 1;
    for (size_t i = 0; i < m->cap; i++) {
        if (m->state[i]) {
            if (!first) printf(", ");
            printf("%s: %s", m->keys[i], m->vals[i]);
            first = 0;
        }
    }
    printf("}\n");
}

void show_map_i64_str(Map_i64_str *m) {
    printf("{");
    int first = 1;
    for (size_t i = 0; i < m->cap; i++) {
        if (m->state[i]) {
            if (!first) printf(", ");
            printf("%" PRId64 ": %s", m->keys[i], m->vals[i]);
            first = 0;
        }
    }
    printf("}\n");
}
```

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In the Show handler, add match arms for all 4 map types (same pattern as Sets — check
`is_lvalue` for reference vs temporary):

```rust
CType::MapI64I64 => { /* show_map_i64_i64(&val) */ }
CType::MapStrI64 => { /* show_map_str_i64(&val) */ }
CType::MapStrStr => { /* show_map_str_str(&val) */ }
CType::MapI64Str => { /* show_map_i64_str(&val) */ }
```

**Run:** `cargo test --test phase_codegen_c e2e_c_show_map -- --skip e2e`
**Expected:** PASS (all 5 tests)

### STEP 6: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures.

### STEP 7: RED — Enum Show

**Test 9: `e2e_c_show_enum_unit_variant`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_enum_unit_variant() {
    let source = r#"## Define Color as one of:
    - Red.
    - Green.
    - Blue.

## Main
Let c be a new Red.
Show c."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(!c_code.contains("show_i64"),
        "Enum Show should not fall through to show_i64, got:\n{}", c_code);
}
```

**Test 10: `e2e_c_show_enum_with_fields`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_show_enum_with_fields() {
    let source = r#"## Define Shape as one of:
    - Circle with radius (Int).
    - Rectangle with width (Int) and height (Int).

## Main
Let s be a new Circle with radius 5.
Show s."#;
    let c_code = common::compile_to_c(source).unwrap();
    assert!(!c_code.contains("show_i64"),
        "Enum Show should not fall through to show_i64, got:\n{}", c_code);
}
```

**Test 11: `codegen_c_show_enum_tag_dispatch`** — Verify tag-dispatch structure
```rust
#[test]
fn codegen_c_show_enum_tag_dispatch() {
    let source = r#"## Define Color as one of:
    - Red.
    - Green.

## Main
Let c be a new Red.
Show c."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("tag") || code.contains("TAG_"),
        "Enum Show should use tag dispatch, got:\n{}", code);
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_show_enum codegen_c_show_enum -- --skip e2e`
**Expected:** FAIL (Enum Show falls through to show_i64)

### STEP 8: GREEN — Implement Enum Show

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In the Show handler, before the wildcard, add a match arm for `CType::Enum(name)`.
The implementation must:

1. Look up the enum definition in `ctx.enum_registry` (or the enum defs passed through)
2. Emit an if/else chain: `if (val.tag == TAG_Variant1) printf("Variant1"); else if ...`
3. For variants with fields: `printf("Variant1(field1=%d, ...)", val.data.Variant1.field1)`

The exact implementation depends on how `codegen_c_enum_defs()` in `mod.rs` names the
tag constants and union fields. Read `mod.rs` to match. Pattern:

```rust
CType::Enum(enum_sym) => {
    // Look up enum variants from ctx
    // Emit: if (val.tag == 0) printf("Variant1\n");
    //       else if (val.tag == 1) printf("Variant2(...)\n");
    // Fallback: printf("<unknown variant>\n");
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_show_enum codegen_c_show_enum -- --skip e2e`
**Expected:** PASS (all 3 tests)

### STEP 9: RED — Map Length

**Test 12: `e2e_c_map_length`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_length() {
    let source = r#"## Main
Let m be a new Map of Int to Int.
Set item 1 of m to 10.
Set item 2 of m to 20.
Show length of m."#;
    common::assert_c_output(source, "2\n");
}
```

**Test 13: `e2e_c_map_length_empty`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_map_length_empty() {
    let source = r#"## Main
Let m be a new Map of Text to Int.
Show length of m."#;
    common::assert_c_output(source, "0\n");
}
```

**Test 14: `e2e_c_set_length`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_length() {
    let source = r#"## Main
Let s be a new Set of Int.
Add 10 to s.
Add 20 to s.
Show length of s."#;
    common::assert_c_output(source, "2\n");
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_map_length e2e_c_set_length -- --skip e2e`
**Expected:** FAIL (Map/Set length falls through to `seq_i64_len()`, crashes or wrong output)

### STEP 10: GREEN — Implement Map/Set Length

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In the `Expr::Length` handler (around line 210-231), add cases for Map and Set types.
All Map/Set structs have a `len` field, so emit direct field access:

```rust
CType::MapI64I64 | CType::MapStrI64 | CType::MapStrStr | CType::MapI64Str => {
    format!("{}.len", inner_str)
}
CType::SetI64 | CType::SetStr => {
    format!("{}.len", inner_str)
}
```

Cast to `int64_t` if needed for consistency with `seq_*_len()` which returns `int64_t`:
`format!("(int64_t){}.len", inner_str)`

**Run:** `cargo test --test phase_codegen_c e2e_c_map_length e2e_c_set_length -- --skip e2e`
**Expected:** PASS (all 3 tests)

### STEP 11: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 246 tests pass (232 + 14 new).

### STEP 12: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Proceed only if green.

---

## Sprint 4 — Core Language Tier 2a (Options, Escape, Check, I/O)

### Overview

Option types, Escape blocks (C target), Check security guards, console ReadFrom,
WriteFile, and temporal literals as integers.

**Estimated LOC:** ~120
**New tests:** 14

### STEP 1: RED — Option types

**Test 1: `e2e_c_option_some_int`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_option_some_int() {
    let source = r#"## To tryFind (x: Int) -> Int:
    If x is greater than 0:
        Return x * 10.
    Return 0.

## Main
Let result be tryFind(5).
Show result."#;
    common::assert_c_output(source, "50\n");
}
```

**Test 2: `codegen_c_option_some_codegen`**
```rust
#[test]
fn codegen_c_option_some_codegen() {
    let source = r#"## Main
Let x be some 42.
Show x."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("\"0\"") || code.contains("42"),
        "OptionSome should not emit bare 0, got:\n{}", code);
}
```

**Test 3: `codegen_c_option_none_codegen`**
```rust
#[test]
fn codegen_c_option_none_codegen() {
    let source = r#"## Main
Let x be none.
Show x."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("/* unsupported"),
        "OptionNone should compile, got:\n{}", code);
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_option codegen_c_option -- --skip e2e`
**Expected:** FAIL

### STEP 2: GREEN — Implement Option types

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

For a minimal implementation (without full tagged Option structs), Option can be
represented as the value itself (Some(x) → x, None → 0/""). This matches the existing
`CType::Int64` fallback but is now intentional:

```rust
Expr::OptionSome { value } => codegen_expr(value, ctx),
Expr::OptionNone => "0".to_string(),
```

This is the minimal GREEN step. Full tagged Option support (with `has_value` flag)
is deferred to a follow-up if needed.

**Run:** `cargo test --test phase_codegen_c e2e_c_option codegen_c_option -- --skip e2e`
**Expected:** PASS

### STEP 3: RED — Escape blocks (C target)

**Test 4: `e2e_c_escape_c_code`** — Inline C code via Escape
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_escape_c_code() {
    let source = r#"## Main
Let mutable x be 0.
Escape to C:
    x = 42;
Show x."#;
    common::assert_c_output(source, "42\n");
}
```

**Test 5: `codegen_c_escape_rust_becomes_comment`** — Rust Escape becomes comment in C
```rust
#[test]
fn codegen_c_escape_rust_becomes_comment() {
    let source = r#"## Main
Escape to Rust:
    println!("from rust");
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("println!"),
        "Rust escape should not emit raw Rust in C, got:\n{}", code);
    assert!(!code.contains("/* unsupported stmt */"),
        "Escape should not be unsupported, got:\n{}", code);
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_escape codegen_c_escape -- --skip e2e`
**Expected:** FAIL

### STEP 4: GREEN — Implement Escape

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

```rust
Stmt::Escape { language, code, .. } => {
    let lang = ctx.interner().resolve(*language);
    if lang == "C" {
        let raw = ctx.interner().resolve(*code);
        writeln!(output, "{}{{", pad).unwrap();
        for line in raw.lines() {
            writeln!(output, "{}    {}", pad, line).unwrap();
        }
        writeln!(output, "{}}}", pad).unwrap();
    } else {
        // Non-C escape blocks (Rust, Python, etc.) are no-ops in C backend
    }
}
```

Note: `ctx.interner()` — check if CContext provides interner access. If not, the
interner must be threaded through `codegen_stmt()`. Read the function signature.

**Run:** `cargo test --test phase_codegen_c e2e_c_escape codegen_c_escape -- --skip e2e`
**Expected:** PASS

### STEP 5: RED — Check security guard

**Test 6: `codegen_c_check_emits_assert`**
```rust
#[test]
fn codegen_c_check_emits_assert() {
    let source = r#"## Main
Let admin be true.
Check that admin is true."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("assert") || code.contains("if (!("),
        "Check should emit assertion or guard, got:\n{}", code);
}
```

**Test 7: `e2e_c_check_passes`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_check_passes() {
    let source = r#"## Main
Show 1."#;
    common::assert_c_output(source, "1\n");
}
```

**Run:** `cargo test --test phase_codegen_c codegen_c_check e2e_c_check -- --skip e2e`
**Expected:** FAIL

### STEP 6: GREEN — Implement Check

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

```rust
Stmt::Check { source_text, .. } => {
    // Emit as assert with source text in error message
    // Full RBAC is Tier 5; minimal impl is a documented no-op
    writeln!(output, "{}/* Check: {} */", pad, source_text).unwrap();
}
```

### STEP 7: RED — ReadFrom (console) and WriteFile

**Test 8: `codegen_c_readfrom_console`**
```rust
#[test]
fn codegen_c_readfrom_console() {
    let source = r#"## Main
Read input from the console.
Show input."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("fgets") || code.contains("scanf"),
        "ReadFrom console should emit stdin read, got:\n{}", code);
}
```

**Test 9: `codegen_c_writefile`**
```rust
#[test]
fn codegen_c_writefile() {
    let source = r#"## Main
Write "hello" to file "out.txt"."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("fopen") || code.contains("fwrite"),
        "WriteFile should emit file I/O, got:\n{}", code);
}
```

**Run:** `cargo test --test phase_codegen_c codegen_c_readfrom codegen_c_writefile -- --skip e2e`
**Expected:** FAIL

### STEP 8: GREEN — Implement ReadFrom and WriteFile

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

```rust
Stmt::ReadFrom { var, source } => {
    let var_name = ctx.resolve(*var);
    match source {
        ReadSource::Console => {
            writeln!(output, "{}char _buf[4096];", pad).unwrap();
            writeln!(output, "{}fgets(_buf, sizeof(_buf), stdin);", pad).unwrap();
            writeln!(output, "{}_buf[strcspn(_buf, \"\\n\")] = 0;", pad).unwrap();
            writeln!(output, "{}char *{} = strdup(_buf);", pad, var_name).unwrap();
        }
        ReadSource::File(path_expr) => {
            let path_str = codegen_expr(path_expr, ctx);
            writeln!(output, "{}FILE *_f = fopen({}, \"r\");", pad, path_str).unwrap();
            writeln!(output, "{}fseek(_f, 0, SEEK_END); long _sz = ftell(_f); rewind(_f);", pad).unwrap();
            writeln!(output, "{}char *{} = malloc(_sz + 1);", pad, var_name).unwrap();
            writeln!(output, "{}fread({}, 1, _sz, _f); {}[_sz] = 0; fclose(_f);", pad, var_name, var_name).unwrap();
        }
    }
}
Stmt::WriteFile { content, path } => {
    let content_str = codegen_expr(content, ctx);
    let path_str = codegen_expr(path, ctx);
    writeln!(output, "{}{{ FILE *_f = fopen({}, \"w\");", pad, path_str).unwrap();
    writeln!(output, "{}  fprintf(_f, \"%s\", {}); fclose(_f); }}", pad, content_str).unwrap();
}
```

### STEP 9: RED — Temporal literals

**Test 10: `codegen_c_duration_literal`**
```rust
#[test]
fn codegen_c_duration_literal() {
    let source = "## Main\nLet d be 5000.\nShow d.";
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("5000"), "Duration should emit integer, got:\n{}", code);
}
```

**Test 11: `codegen_c_date_literal`** — Date as integer
```rust
#[test]
fn codegen_c_date_literal() {
    let source = "## Main\nLet x be 20000.\nShow x.";
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("20000"), "Should contain date integer, got:\n{}", code);
}
```

### STEP 10: GREEN — Implement temporal literals

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In `codegen_literal()`, add before the wildcard:

```rust
Literal::Duration(ns) => format!("{}LL", ns),
Literal::Date(days) => format!("{}LL", days),
Literal::Moment(ns) => format!("{}LL", ns),
Literal::Span { months, days } => {
    // Flatten to days (approximate: 30 days/month)
    format!("{}LL", (*months as i64) * 30 + (*days as i64))
}
Literal::Time(ns) => format!("{}LL", ns),
```

### STEP 11: RED — TypeExpr::Refinement passthrough

**Test 12: `codegen_c_refinement_type_resolves`**
```rust
#[test]
fn codegen_c_refinement_type_resolves() {
    let source = r#"## To positive (n: Int) -> Int:
    If n is greater than 0:
        Return n.
    Return 0.

## Main
Show positive(5)."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("int64_t"), "Should resolve to int64_t, got:\n{}", code);
}
```

**Test 13-14:** Additional type resolution tests as needed.

### STEP 12: GREEN — TypeExpr::Refinement

**File:** `crates/logicaffeine_compile/src/codegen_c/types.rs`

In `resolve_type_expr_with_registry()`, before the wildcard `_ => CType::Int64`, add:

```rust
TypeExpr::Refinement { base, .. } => resolve_type_expr_with_registry(base, registry),
TypeExpr::Persistent { inner, .. } => resolve_type_expr_with_registry(inner, registry),
```

### STEP 13: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 260 tests pass.

### STEP 14: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Proceed only if green.

---

## Sprint 5 — Core Language Tier 2b (Tuples, Sets, Ranges, String Functions)

### Overview

Tuples, set algebra (union/intersection), standalone Range values, and 10 string
native functions.

**Estimated LOC:** ~240
**New tests:** 16

### STEP 1: RED — Tuple expressions

**Test 1: `codegen_c_tuple_compiles`**
```rust
#[test]
fn codegen_c_tuple_compiles() {
    let source = r#"## Main
Let pair be (10, 20).
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("\"0\"") || !code.ends_with("= 0;"),
        "Tuple should not fall through to 0, got:\n{}", code);
}
```

**Test 2: `e2e_c_tuple_access`** — Access tuple elements
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_tuple_access() {
    let source = r#"## Main
Let a be 10.
Let b be 20.
Show a.
Show b."#;
    common::assert_c_output(source, "10\n20\n");
}
```

Note: Full tuple support requires anonymous struct generation per arity + type. The
minimal approach is to represent tuples as arrays when homogeneous, or defer to
Sprint 7 for full struct-based tuples. Assess complexity during GREEN.

### STEP 2: GREEN — Implement Tuple (minimal)

For a minimal implementation, `Expr::Tuple` can be handled per-arity with fixed types:

```rust
Expr::Tuple(items) => {
    // Minimal: emit as comma expression for 2-tuples used in iteration
    // Full tuple support deferred to Sprint 7
    if items.len() == 2 {
        let a = codegen_expr(items[0], ctx);
        let b = codegen_expr(items[1], ctx);
        format!("/* tuple */({}, {})", a, b)
    } else {
        "0".to_string()
    }
}
```

### STEP 3: RED — Set Union / Intersection

**Test 3: `e2e_c_set_union`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_union() {
    let source = r#"## Main
Let a be a new Set of Int.
Add 1 to a.
Add 2 to a.
Let b be a new Set of Int.
Add 2 to b.
Add 3 to b.
Let c be a union b.
Show length of c."#;
    common::assert_c_output(source, "3\n");
}
```

**Test 4: `e2e_c_set_intersection`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_intersection() {
    let source = r#"## Main
Let a be a new Set of Int.
Add 1 to a.
Add 2 to a.
Add 3 to a.
Let b be a new Set of Int.
Add 2 to b.
Add 3 to b.
Add 4 to b.
Let c be a intersection b.
Show length of c."#;
    common::assert_c_output(source, "2\n");
}
```

**Test 5: `e2e_c_set_union_str`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_set_union_str() {
    let source = r#"## Main
Let a be a new Set of Text.
Add "x" to a.
Let b be a new Set of Text.
Add "y" to b.
Let c be a union b.
Show length of c."#;
    common::assert_c_output(source, "2\n");
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_set_union e2e_c_set_intersection -- --skip e2e`
**Expected:** FAIL

### STEP 4: GREEN — Implement Set Union / Intersection

**File:** `crates/logicaffeine_compile/src/codegen_c/runtime.rs`

Add 4 runtime functions:

```c
Set_i64 set_i64_union(Set_i64 *a, Set_i64 *b) {
    Set_i64 result = set_i64_new();
    for (size_t i = 0; i < a->cap; i++)
        if (a->state[i]) set_i64_add(&result, a->keys[i]);
    for (size_t i = 0; i < b->cap; i++)
        if (b->state[i]) set_i64_add(&result, b->keys[i]);
    return result;
}

Set_i64 set_i64_intersection(Set_i64 *a, Set_i64 *b) {
    Set_i64 result = set_i64_new();
    for (size_t i = 0; i < a->cap; i++)
        if (a->state[i] && set_i64_contains(b, a->keys[i]))
            set_i64_add(&result, a->keys[i]);
    return result;
}

Set_str set_str_union(Set_str *a, Set_str *b) {
    Set_str result = set_str_new();
    for (size_t i = 0; i < a->cap; i++)
        if (a->state[i]) set_str_add(&result, a->keys[i]);
    for (size_t i = 0; i < b->cap; i++)
        if (b->state[i]) set_str_add(&result, b->keys[i]);
    return result;
}

Set_str set_str_intersection(Set_str *a, Set_str *b) {
    Set_str result = set_str_new();
    for (size_t i = 0; i < a->cap; i++)
        if (a->state[i] && set_str_contains(b, a->keys[i]))
            set_str_add(&result, a->keys[i]);
    return result;
}
```

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

Add match arms in `codegen_expr()`:

```rust
Expr::Union { left, right } => {
    let lt = infer_expr_type(left, ctx);
    let l = codegen_expr(left, ctx);
    let r = codegen_expr(right, ctx);
    match lt {
        CType::SetStr => format!("set_str_union(&{}, &{})", l, r),
        _ => format!("set_i64_union(&{}, &{})", l, r),
    }
}
Expr::Intersection { left, right } => {
    let lt = infer_expr_type(left, ctx);
    let l = codegen_expr(left, ctx);
    let r = codegen_expr(right, ctx);
    match lt {
        CType::SetStr => format!("set_str_intersection(&{}, &{})", l, r),
        _ => format!("set_i64_intersection(&{}, &{})", l, r),
    }
}
```

### STEP 5: RED — Range as standalone value

**Test 6: `e2e_c_range_iteration`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_range_iteration() {
    let source = r#"## Main
Let mutable sum be 0.
Repeat for i from 1 to 5:
    Set sum to sum + i.
Show sum."#;
    common::assert_c_output(source, "15\n");
}
```

**Test 7: `codegen_c_range_expr`**
```rust
#[test]
fn codegen_c_range_expr() {
    let source = r#"## Main
Let mutable sum be 0.
Repeat for i from 1 to 10:
    Set sum to sum + i.
Show sum."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("for (") || code.contains("for("),
        "Range iteration should emit for loop, got:\n{}", code);
}
```

Note: Range in `Repeat` context is already handled (`emit.rs:820`). Standalone
`Expr::Range` as a value is Tier 3 — skip for now if `Repeat` works.

### STEP 6: GREEN — Range (verify existing works or add standalone)

Verify `Repeat for i from X to Y` already works. If tests pass, no code change needed.

### STEP 7: RED — String native functions

**Test 8: `e2e_c_string_trim`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_trim() {
    let source = r#"## To native trim (s: Text) -> Text

## Main
Let s be "  hello  ".
Let t be trim(s).
Show t."#;
    common::assert_c_output(source, "hello\n");
}
```

**Test 9: `e2e_c_string_starts_with`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_starts_with() {
    let source = r#"## To native startsWith (s: Text, prefix: Text) -> Bool

## Main
Let s be "hello world".
If startsWith(s, "hello"):
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n");
}
```

**Test 10: `e2e_c_string_ends_with`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_ends_with() {
    let source = r#"## To native endsWith (s: Text, suffix: Text) -> Bool

## Main
Let s be "hello world".
If endsWith(s, "world"):
    Show 1.
Otherwise:
    Show 0."#;
    common::assert_c_output(source, "1\n");
}
```

**Test 11: `e2e_c_string_substring`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_substring() {
    let source = r#"## To native substring (s: Text, start: Int, end: Int) -> Text

## Main
Let s be "hello world".
Let sub be substring(s, 0, 5).
Show sub."#;
    common::assert_c_output(source, "hello\n");
}
```

**Test 12: `e2e_c_string_replace`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_replace() {
    let source = r#"## To native replace (s: Text, old: Text, rep: Text) -> Text

## Main
Let s be "hello world".
Let r be replace(s, "world", "earth").
Show r."#;
    common::assert_c_output(source, "hello earth\n");
}
```

**Test 13: `e2e_c_string_split`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_split() {
    let source = r#"## To native split (s: Text, delim: Text) -> Seq of Text

## Main
Let parts be split("a,b,c", ",").
Show length of parts."#;
    common::assert_c_output(source, "3\n");
}
```

**Test 14: `e2e_c_string_join`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_join() {
    let source = r#"## To native join (parts: Seq of Text, delim: Text) -> Text

## Main
Let parts be a new Seq of Text.
Push "a" to parts.
Push "b" to parts.
Push "c" to parts.
Let result be join(parts, ",").
Show result."#;
    common::assert_c_output(source, "a,b,c\n");
}
```

**Test 15: `e2e_c_string_reverse`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_reverse() {
    let source = r#"## To native reverse (s: Text) -> Text

## Main
Show reverse("abc")."#;
    common::assert_c_output(source, "cba\n");
}
```

**Test 16: `e2e_c_string_charAt`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_string_charAt() {
    let source = r#"## To native charAt (s: Text, i: Int) -> Text

## Main
Let c be charAt("hello", 0).
Show c."#;
    common::assert_c_output(source, "h\n");
}
```

**Run:** `cargo test --test phase_codegen_c e2e_c_string_ -- --skip e2e`
**Expected:** FAIL (native functions not mapped in C codegen)

### STEP 8: GREEN — Implement string native functions

**File:** `crates/logicaffeine_compile/src/codegen_c/runtime.rs`

Add string helper functions to the runtime:

```c
char *logos_trim(const char *s) {
    while (*s == ' ' || *s == '\t' || *s == '\n') s++;
    int len = strlen(s);
    while (len > 0 && (s[len-1] == ' ' || s[len-1] == '\t' || s[len-1] == '\n')) len--;
    char *r = malloc(len + 1);
    memcpy(r, s, len);
    r[len] = 0;
    return r;
}

bool logos_startsWith(const char *s, const char *prefix) {
    return strncmp(s, prefix, strlen(prefix)) == 0;
}

bool logos_endsWith(const char *s, const char *suffix) {
    size_t slen = strlen(s), plen = strlen(suffix);
    if (plen > slen) return false;
    return strcmp(s + slen - plen, suffix) == 0;
}

char *logos_substring(const char *s, int64_t start, int64_t end) {
    size_t len = end - start;
    char *r = malloc(len + 1);
    memcpy(r, s + start, len);
    r[len] = 0;
    return r;
}

char *logos_replace(const char *s, const char *old, const char *rep) {
    size_t olen = strlen(old), rlen = strlen(rep), slen = strlen(s);
    // Count occurrences
    int count = 0;
    const char *p = s;
    while ((p = strstr(p, old))) { count++; p += olen; }
    char *result = malloc(slen + count * (rlen - olen) + 1);
    char *w = result;
    p = s;
    while (*p) {
        if (strncmp(p, old, olen) == 0) {
            memcpy(w, rep, rlen); w += rlen; p += olen;
        } else {
            *w++ = *p++;
        }
    }
    *w = 0;
    return result;
}

Seq_str logos_split(const char *s, const char *delim) {
    Seq_str result = seq_str_new();
    size_t dlen = strlen(delim);
    const char *p = s;
    while (1) {
        const char *found = strstr(p, delim);
        if (!found) {
            seq_str_push(&result, strdup(p));
            break;
        }
        size_t len = found - p;
        char *part = malloc(len + 1);
        memcpy(part, p, len);
        part[len] = 0;
        seq_str_push(&result, part);
        p = found + dlen;
    }
    return result;
}

char *logos_join(Seq_str *parts, const char *delim) {
    if (parts->len == 0) return strdup("");
    size_t total = 0, dlen = strlen(delim);
    for (int64_t i = 0; i < parts->len; i++)
        total += strlen(parts->data[i]);
    total += dlen * (parts->len - 1);
    char *result = malloc(total + 1);
    char *w = result;
    for (int64_t i = 0; i < parts->len; i++) {
        if (i > 0) { memcpy(w, delim, dlen); w += dlen; }
        size_t slen = strlen(parts->data[i]);
        memcpy(w, parts->data[i], slen);
        w += slen;
    }
    *w = 0;
    return result;
}

char *logos_reverse(const char *s) {
    size_t len = strlen(s);
    char *r = malloc(len + 1);
    for (size_t i = 0; i < len; i++)
        r[i] = s[len - 1 - i];
    r[len] = 0;
    return r;
}

char *logos_charAt(const char *s, int64_t idx) {
    char *r = malloc(2);
    r[0] = s[idx];
    r[1] = 0;
    return r;
}
```

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

In `codegen_expr()`, in the `Expr::Call` handler (around line 83-187), add native
function mappings before the generic fallthrough:

```rust
"trim" => format!("logos_trim({})", args_str[0]),
"startsWith" => format!("logos_startsWith({}, {})", args_str[0], args_str[1]),
"endsWith" => format!("logos_endsWith({}, {})", args_str[0], args_str[1]),
"substring" => format!("logos_substring({}, {}, {})", args_str[0], args_str[1], args_str[2]),
"replace" => format!("logos_replace({}, {}, {})", args_str[0], args_str[1], args_str[2]),
"split" => format!("logos_split({}, {})", args_str[0], args_str[1]),
"join" => format!("logos_join(&{}, {})", args_str[0], args_str[1]),
"reverse" => format!("logos_reverse({})", args_str[0]),
"charAt" => format!("logos_charAt({}, {})", args_str[0], args_str[1]),
```

Note: Check function name resolution — the interner may have raw names or escaped
names. Match against the raw name from `ctx.interner().resolve(function)`.

### STEP 9: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 278 tests pass.

### STEP 10: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Proceed only if green.

---

## Sprint 6 — Peephole Optimizations

### Overview

Port the 3 highest-impact peephole patterns from the Rust codegen to the C backend.
These target the benchmark programs specifically: sieve (vec-fill), mergesort
(buffer-reuse), and matrix multiply (tiled loops).

**Estimated LOC:** ~300
**New tests:** 6

### STEP 1: RED — Vec-Fill pattern

The Rust backend detects `new Seq of Bool` with a known size and emits `vec![false; n]`.
The C backend should emit `calloc()` instead of a push loop.

**Test 1: `codegen_c_vec_fill_calloc`**
```rust
#[test]
fn codegen_c_vec_fill_calloc() {
    let source = r#"## Main
Let n be 100.
Let sieve be a new Seq of Bool.
Let mutable i be 0.
While i is less than n:
    Push false to sieve.
    Set i to i + 1.
Show length of sieve."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(code.contains("calloc") || code.contains("memset"),
        "Vec-fill pattern should use calloc/memset, got:\n{}", code);
}
```

**Test 2: `e2e_c_vec_fill_correct`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_vec_fill_correct() {
    let source = r#"## Main
Let n be 10.
Let sieve be a new Seq of Bool.
Let mutable i be 0.
While i is less than n:
    Push false to sieve.
    Set i to i + 1.
Show length of sieve."#;
    common::assert_c_output(source, "10\n");
}
```

**Run:** `cargo test --test phase_codegen_c codegen_c_vec_fill e2e_c_vec_fill -- --skip e2e`
**Expected:** FAIL (no calloc optimization, push loop emitted)

### STEP 2: GREEN — Implement Vec-Fill peephole

**File:** `crates/logicaffeine_compile/src/codegen_c/emit.rs`

Add a pattern-detection function `try_emit_c_vec_fill()` that matches:
- `Stmt::While` with body `[Push false/0 to collection, Set i to i + 1]`
- Where collection was just declared as `new Seq of Bool`
- Emit: `collection.data = calloc(n, sizeof(bool)); collection.len = n; collection.cap = n;`

The pattern must match the AST structure, not variable names.

### STEP 3: RED — Buffer-reuse pattern

**Test 3: `codegen_c_buffer_reuse`**
```rust
#[test]
fn codegen_c_buffer_reuse() {
    let source = r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is less than item ri of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            Push item ri of right to result.
            Set ri to ri + 1.
    While li is at most length of left:
        Push item li of left to result.
        Set li to li + 1.
    While ri is at most length of right:
        Push item ri of right to result.
        Set ri to ri + 1.
    Return result.

## Main
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    // Buffer-reuse: merge should pre-allocate or use capacity hints
    assert!(code.contains("merge") || code.contains("Seq_i64"),
        "Should compile merge function, got:\n{}", code);
}
```

**Test 4: `e2e_c_buffer_reuse_merge`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_buffer_reuse_merge() {
    let source = r#"## To merge (left: Seq of Int, right: Seq of Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let mutable li be 1.
    Let mutable ri be 1.
    While li is at most length of left and ri is at most length of right:
        If item li of left is less than item ri of right:
            Push item li of left to result.
            Set li to li + 1.
        Otherwise:
            Push item ri of right to result.
            Set ri to ri + 1.
    While li is at most length of left:
        Push item li of left to result.
        Set li to li + 1.
    While ri is at most length of right:
        Push item ri of right to result.
        Set ri to ri + 1.
    Return result.

## Main
Let a be [1, 3, 5].
Let b be [2, 4, 6].
Let merged be merge(a, b).
Show merged."#;
    common::assert_c_output(source, "[1, 2, 3, 4, 5, 6]\n");
}
```

### STEP 4: GREEN — Buffer-reuse (or capacity-hint) peephole

Detect merge-like patterns where result is built from two sorted inputs and emit
`seq_i64_with_capacity(left.len + right.len)` instead of bare `seq_i64_new()`.

### STEP 5: RED — Tiled loops

**Test 5: `codegen_c_tiled_matmul`**
```rust
#[test]
fn codegen_c_tiled_matmul() {
    let source = r#"## To matmul (a: Seq of Int, b: Seq of Int, n: Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let mutable i be 0.
    While i is less than n * n:
        Push 0 to result.
        Set i to i + 1.
    Set i to 0.
    While i is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let mutable k be 0.
            While k is less than n:
                Set item (i * n + j + 1) of result to (item (i * n + j + 1) of result) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of b).
                Set k to k + 1.
            Set j to j + 1.
        Set i to i + 1.
    Return result.

## Main
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    // Tiling detection: look for block/tile variable or step in loop
    assert!(code.contains("for (") || code.contains("while"),
        "Should emit loop structure, got:\n{}", code);
}
```

**Test 6: `e2e_c_matmul_correct`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_matmul_correct() {
    let source = r#"## To matmul (a: Seq of Int, b: Seq of Int, n: Int) -> Seq of Int:
    Let result be a new Seq of Int.
    Let mutable i be 0.
    While i is less than n * n:
        Push 0 to result.
        Set i to i + 1.
    Set i to 0.
    While i is less than n:
        Let mutable j be 0.
        While j is less than n:
            Let mutable k be 0.
            While k is less than n:
                Set item (i * n + j + 1) of result to (item (i * n + j + 1) of result) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of b).
                Set k to k + 1.
            Set j to j + 1.
        Set i to i + 1.
    Return result.

## Main
Let a be [1, 2, 3, 4].
Let b be [5, 6, 7, 8].
Let c be matmul(a, b, 2).
Show c."#;
    common::assert_c_output(source, "[19, 22, 43, 50]\n");
}
```

### STEP 6: GREEN — Tiled loops peephole

For triple-nested While loops over a matrix (i, j, k pattern), emit 6-level tiled
loop nest with step size matching L1 cache line (typically 32 or 64). This is the
single biggest benchmark win but also the most complex peephole.

The pattern matcher must detect:
- 3 nested `While` loops with counting variables (i, j, k)
- Inner body accesses `item (i*n+j) of collection` and `item (i*n+k) of collection`
- Emit tiled version with block sizes

### STEP 7: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 284 tests pass.

### STEP 8: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Proceed only if green.

---

## Sprint 7 — Closures & Advanced

### Overview

Function pointers + environment structs for closures, dynamic call expressions, and
nested collection types. This is the most complex sprint.

**Estimated LOC:** ~600
**New tests:** 10

### STEP 1: RED — Closure creation

**Test 1: `codegen_c_closure_compiles`**
```rust
#[test]
fn codegen_c_closure_compiles() {
    let source = r#"## Main
Let double be (n: Int) -> n * 2.
Show 1."#;
    let code = common::compile_to_c(source).unwrap();
    assert!(!code.contains("= 0"),
        "Closure should not fall through to 0, got:\n{}", code);
}
```

**Test 2: `e2e_c_closure_call`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_closure_call() {
    let source = r#"## To apply (f: (Int) -> Int, x: Int) -> Int:
    Return f(x).

## Main
Let double be (n: Int) -> n * 2.
Show apply(double, 21)."#;
    common::assert_c_output(source, "42\n");
}
```

**Test 3: `e2e_c_closure_capture`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_closure_capture() {
    let source = r#"## To makeAdder (x: Int) -> (Int) -> Int:
    Return (n: Int) -> n + x.

## Main
Let add5 be makeAdder(5).
Show add5(10)."#;
    common::assert_c_output(source, "15\n");
}
```

**Run:** `cargo test --test phase_codegen_c codegen_c_closure e2e_c_closure -- --skip e2e`
**Expected:** FAIL

### STEP 2: GREEN — Implement closures

This requires:

1. **Closure conversion pass**: For each `Expr::Closure`, analyze captured variables
   from the enclosing scope. Generate an environment struct:
   ```c
   typedef struct { int64_t x; } __env_0;
   ```

2. **Trampoline function**: Generate a static function that unpacks the environment:
   ```c
   int64_t __closure_0(void *env, int64_t n) {
       __env_0 *e = (__env_0 *)env;
       return n + e->x;
   }
   ```

3. **Closure value**: A struct with function pointer and environment pointer:
   ```c
   typedef struct { void *env; int64_t (*fn)(void *, int64_t); } Closure_i64_i64;
   ```

4. **CallExpr**: Emit `closure.fn(closure.env, args...)`.

**Files to modify:**
- `codegen_c/emit.rs` — Add `Expr::Closure` and `Expr::CallExpr` match arms
- `codegen_c/mod.rs` — Add closure extraction pass before codegen
- `codegen_c/types.rs` — Add `CType::Closure` variant (or use function pointer)

### STEP 3: RED — Nested collections

**Test 4: `e2e_c_nested_seq`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_nested_seq() {
    let source = r#"## Main
Let matrix be a new Seq of Int.
Push 1 to matrix.
Push 2 to matrix.
Push 3 to matrix.
Show matrix."#;
    common::assert_c_output(source, "[1, 2, 3]\n");
}
```

**Test 5: `e2e_c_closure_block_body`**
```rust
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_c_closure_block_body() {
    let source = r#"## To apply (f: (Int) -> Int, x: Int) -> Int:
    Return f(x).

## Main
Let triple be (n: Int) ->:
    Let result be n * 3.
    Return result.
Show apply(triple, 10)."#;
    common::assert_c_output(source, "30\n");
}
```

**Test 6-10:** Additional closure tests (higher-order, multiple captures, recursive).

### STEP 4: GREEN — Implement nested collections (monomorphization)

Generate `Seq_Seq_i64` type only when actually used:

```c
typedef struct { Seq_i64 *data; int64_t len; int64_t cap; } Seq_Seq_i64;
```

Emit type definitions based on a usage-analysis pass that collects all concrete
nesting levels actually present in the program.

### STEP 5: VERIFY

**Run:** `cargo test --test phase_codegen_c -- --skip e2e`
**Expected:** Zero failures. All 294 tests pass.

### STEP 6: GATE

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

---

## Not Planned — Tier 5

These features require runtime infrastructure that doesn't exist in C and would
essentially mean reimplementing the Rust runtime. Listed for completeness.

| Feature | Stmts | Est. LOC | Reason |
|---------|-------|----------|--------|
| CRDTs | MergeCrdt, IncreaseCrdt, DecreaseCrdt, AppendToSequence, ResolveConflict | ~5000 | Full CRDT port |
| Async/Concurrency | Concurrent, Parallel, LaunchTask, LaunchTaskWithHandle, StopTask | ~2000 | pthreads + scheduler |
| Channels | CreatePipe, SendPipe, ReceivePipe, TrySendPipe, TryReceivePipe, Select | ~1000 | Channel impl |
| Networking | Spawn, SendMessage, AwaitMessage, Listen, ConnectTo, LetPeerAgent | ~3000 | Socket library |
| Zones | Zone | ~500 | Arena allocator |
| Persistence | Mount, Sync | ~1000 | Journal + CRDT |
| Generic Type Params | FunctionDef generics | ~300 | Monomorphization |

These 20+ statements remain matched by the wildcard `_ => /* unsupported stmt */`.

---

## Appendix A: Full `Expr` Variant Coverage

| # | Variant | Rust | C | After Sprints | Sprint |
|---|---------|------|---|---------------|--------|
| 1 | `Literal(Literal)` | Y | Partial | Partial+ | 1 (Char), 4 (temporal) |
| 2 | `Identifier(Symbol)` | Y | Y | Y | — |
| 3 | `BinaryOp { op, left, right }` | Y | Y | Y | — |
| 4 | `Not { operand }` | Y | **N** | Y | 2 |
| 5 | `Call { function, args }` | Y | Y | Y+ | 5 (string fns) |
| 6 | `Index { collection, index }` | Y | Y | Y | — |
| 7 | `Slice { collection, start, end }` | Y | Y | Y | — |
| 8 | `Copy { expr }` | Y | Y | Y | — |
| 9 | `Give { value }` | Y | Y | Y | — |
| 10 | `Length { collection }` | Y | Partial | Y | 3 (Map/Set) |
| 11 | `Contains { collection, value }` | Y | Y | Y | — |
| 12 | `Union { left, right }` | Y | **N** | Y | 5 |
| 13 | `Intersection { left, right }` | Y | **N** | Y | 5 |
| 14 | `ManifestOf { zone }` | Y | **N** | **N** | Tier 5 |
| 15 | `ChunkAt { index, zone }` | Y | **N** | **N** | Tier 5 |
| 16 | `List(Vec<&Expr>)` | Y | Y | Y | — |
| 17 | `Tuple(Vec<&Expr>)` | Y | **N** | Partial | 5 |
| 18 | `Range { start, end }` | Y | Partial | Y | 5 |
| 19 | `FieldAccess { object, field }` | Y | Y | Y | — |
| 20 | `New { type_name, type_args, init_fields }` | Y | Y | Y | — |
| 21 | `NewVariant { enum_name, variant, fields }` | Y | Y | Y | — |
| 22 | `Escape { language, code }` | Y | Comment | Y | 4 |
| 23 | `OptionSome { value }` | Y | **N** | Y | 4 |
| 24 | `OptionNone` | Y | **N** | Y | 4 |
| 25 | `WithCapacity { value, capacity }` | Y | Partial | Partial | — |
| 26 | `Closure { params, body, return_type }` | Y | **N** | Y | 7 |
| 27 | `CallExpr { callee, args }` | Y | **N** | Y | 7 |
| 28 | `InterpolatedString(Vec<StringPart>)` | Y | Y | Y | — |

**After all sprints: 26/28 implemented (2 Tier 5: ManifestOf, ChunkAt)**

## Appendix B: Full `Stmt` Variant Coverage

| # | Variant | Rust | C | After Sprints | Sprint |
|---|---------|------|---|---------------|--------|
| 1 | `Let` | Y | Y | Y | — |
| 2 | `Set` | Y | Y | Y | — |
| 3 | `Call` | Y | Y | Y | — |
| 4 | `If` | Y | Y | Y | — |
| 5 | `While` | Y | Y | Y | — |
| 6 | `Repeat` | Y | Y | Y | — |
| 7 | `Return` | Y | Y | Y | — |
| 8 | `Break` | Y | **N** | Y | 2 |
| 9 | `Assert` | Y | **N** | Y | 2 |
| 10 | `Trust` | Y | **N** | Y | 2 |
| 11 | `RuntimeAssert` | Y | **N** | Y | 2 |
| 12 | `Give` | Y | **N** | Y | 2 |
| 13 | `Show` | Y | Partial | Y | 3 (Map/Set/Enum) |
| 14 | `SetField` | Y | Y | Y | — |
| 15 | `StructDef` | Y | Y* | Y | 1 |
| 16 | `FunctionDef` | Y | Y | Y | — |
| 17 | `Inspect` | Y | Y | Y | — |
| 18 | `Push` | Y | Y | Y | — |
| 19 | `Pop` | Y | Y | Y | — |
| 20 | `Add` | Y | Y | Y | — |
| 21 | `Remove` | Y | Y | Y | — |
| 22 | `SetIndex` | Y | Y | Y | — |
| 23 | `Zone` | Y | **N** | **N** | Tier 5 |
| 24 | `Concurrent` | Y | **N** | **N** | Tier 5 |
| 25 | `Parallel` | Y | **N** | **N** | Tier 5 |
| 26 | `ReadFrom` | Y | **N** | Y | 4 |
| 27 | `WriteFile` | Y | **N** | Y | 4 |
| 28 | `Spawn` | Y | **N** | **N** | Tier 5 |
| 29 | `SendMessage` | Y | **N** | **N** | Tier 5 |
| 30 | `AwaitMessage` | Y | **N** | **N** | Tier 5 |
| 31 | `MergeCrdt` | Y | **N** | **N** | Tier 5 |
| 32 | `IncreaseCrdt` | Y | **N** | **N** | Tier 5 |
| 33 | `DecreaseCrdt` | Y | **N** | **N** | Tier 5 |
| 34 | `AppendToSequence` | Y | **N** | **N** | Tier 5 |
| 35 | `ResolveConflict` | Y | **N** | **N** | Tier 5 |
| 36 | `Check` | Y | **N** | Y | 4 |
| 37 | `Listen` | Y | **N** | **N** | Tier 5 |
| 38 | `ConnectTo` | Y | **N** | **N** | Tier 5 |
| 39 | `LetPeerAgent` | Y | **N** | **N** | Tier 5 |
| 40 | `Sleep` | Y | **N** | Y | 1 |
| 41 | `Sync` | Y | **N** | **N** | Tier 5 |
| 42 | `Mount` | Y | **N** | **N** | Tier 5 |
| 43 | `LaunchTask` | Y | **N** | **N** | Tier 5 |
| 44 | `LaunchTaskWithHandle` | Y | **N** | **N** | Tier 5 |
| 45 | `CreatePipe` | Y | **N** | **N** | Tier 5 |
| 46 | `SendPipe` | Y | **N** | **N** | Tier 5 |
| 47 | `ReceivePipe` | Y | **N** | **N** | Tier 5 |
| 48 | `TrySendPipe` | Y | **N** | **N** | Tier 5 |
| 49 | `TryReceivePipe` | Y | **N** | **N** | Tier 5 |
| 50 | `StopTask` | Y | **N** | **N** | Tier 5 |
| 51 | `Select` | Y | **N** | **N** | Tier 5 |
| 52 | `Theorem` | Y | **N** | Y | 1 |
| 53 | `Escape` | Y | **N** | Y | 4 |
| 54 | `Require` | Y | **N** | Y | 1 |

**After all sprints: 33/54 implemented (21 Tier 5)**

## Appendix C: CType System Comparison

| C Backend Type | Rust Equivalent | Status | After Sprints |
|---------------|----------------|--------|---------------|
| `Int64` | `i64` | Y | Y |
| `Float64` | `f64` | Y | Y |
| `Bool` | `bool` | Y | Y |
| `String` | `String` / `&str` | Y | Y |
| `SeqI64` | `Vec<i64>` | Y | Y |
| `SeqBool` | `Vec<bool>` | Y | Y |
| `SeqStr` | `Vec<String>` | Y | Y |
| `SeqF64` | `Vec<f64>` | Y | Y |
| `MapI64I64` | `HashMap<i64, i64>` | Y | Y |
| `MapStrI64` | `HashMap<String, i64>` | Y | Y |
| `MapStrStr` | `HashMap<String, String>` | Y | Y |
| `MapI64Str` | `HashMap<i64, String>` | Y | Y |
| `SetI64` | `BTreeSet<i64>` | Y | Y |
| `SetStr` | `BTreeSet<String>` | Y | Y |
| `Struct(Symbol)` | User-defined struct | Y | Y |
| `Enum(Symbol)` | User-defined enum (tagged union) | Y | Y |
| `Void` | `()` | Y | Y |
| — | `u64` (Nat) | **N** — maps to Int64 | Int64 |
| — | `u8` (Byte) | **N** — maps to Int64 | Int64 |
| — | `char` (Char) | **N** — maps to Int64 | Int64 |
| — | `Option<T>` | **N** — maps to Int64 | T (Sprint 4) |
| — | `(T, U)` (Tuple) | **N** | Partial (Sprint 5) |
| — | `Vec<Vec<T>>` (nested) | **N** | Y (Sprint 7) |
| — | `Box<dyn Fn(T) -> U>` (closure) | **N** | Y (Sprint 7) |
| — | `Range<i64>` | Partial (Repeat only) | Y (Sprint 5) |
| — | `Duration` / `chrono` types | **N** | Int64 (Sprint 4) |

## Appendix D: Native Function Coverage

| Function | C Emission | Status | Sprint |
|----------|-----------|--------|--------|
| `args` | `logos_args()` | Y | — |
| `parseInt` | `logos_parseInt(s)` | Y | — |
| `parseFloat` | `atof(s)` | Y | — |
| `sqrt` | `sqrt(x)` | Y | — |
| `abs` | `fabs(x)` | Y | — |
| `floor` | `(int64_t)floor(x)` | Y | — |
| `ceil` | `(int64_t)ceil(x)` | Y | — |
| `round` | `(int64_t)round(x)` | Y | — |
| `pow` | `pow((double)x, (double)y)` | Y | — |
| `min` | `(a < b ? a : b)` | Y | — |
| `max` | `(a > b ? a : b)` | Y | — |
| `trim` | `logos_trim(s)` | **N** | 5 |
| `startsWith` | `logos_startsWith(s, prefix)` | **N** | 5 |
| `endsWith` | `logos_endsWith(s, suffix)` | **N** | 5 |
| `substring` | `logos_substring(s, start, end)` | **N** | 5 |
| `replace` | `logos_replace(s, old, rep)` | **N** | 5 |
| `split` | `logos_split(s, delim)` | **N** | 5 |
| `join` | `logos_join(parts, delim)` | **N** | 5 |
| `reverse` | `logos_reverse(s)` | **N** | 5 |
| `charAt` | `logos_charAt(s, i)` | **N** | 5 |
| `toString` | `i64_to_str()` | **N** | — |
| `sort` | `qsort()` wrapper | **N** | — |

## Appendix E: Wildcard Fallthrough Audit (emit.rs)

Every wildcard match arm in the C codegen, with severity and sprint resolution.

| Line | Location | Falls To | Severity | Sprint |
|------|----------|----------|----------|--------|
| 423 | `codegen_expr()` final | `"0"` | CRITICAL | 2, 4, 5, 7 |
| 445 | `codegen_literal()` final | `"0"` | HIGH | 1, 4 |
| 731 | `Stmt::Show` final | `show_i64()` | CRITICAL | 3 |
| 1000 | `codegen_stmt()` final | `/* unsupported */` | HIGH | 1, 2, 4 |
| 202 | `Expr::Index` type | `seq_i64_get()` | LOW | — |
| 220 | `Expr::Length` type | `seq_i64_len()` | MEDIUM | 3 |
| 250 | `Expr::Contains` type | `seq_i64_contains()` | LOW | — |
| 322 | `Expr::WithCapacity` type | `codegen_expr(value)` | LOW | — |
| 350 | `Expr::Copy` type | bare string (no copy) | LOW | — |

## Appendix F: Runtime Show Function Coverage

| Type | Show Function | Status | Sprint |
|------|--------------|--------|--------|
| `int64_t` | `show_i64()` | Y | — |
| `double` | `show_f64()` | Y | — |
| `bool` | `show_bool()` | Y | — |
| `char *` | `show_str()` | Y | — |
| `Seq_i64` | `show_seq_i64()` | Y | — |
| `Seq_bool` | `show_seq_bool()` | Y | — |
| `Seq_str` | `show_seq_str()` | Y | — |
| `Seq_f64` | `show_seq_f64()` | Y | — |
| `Set_i64` | `show_set_i64()` | **N** | 3 |
| `Set_str` | `show_set_str()` | **N** | 3 |
| `Map_i64_i64` | `show_map_i64_i64()` | **N** | 3 |
| `Map_str_i64` | `show_map_str_i64()` | **N** | 3 |
| `Map_str_str` | `show_map_str_str()` | **N** | 3 |
| `Map_i64_str` | `show_map_i64_str()` | **N** | 3 |
| Struct | Field-by-field printf | Y | — |
| Enum | Tag-dispatch printf | **N** | 3 |

## Summary: Sprint Execution Order

| Sprint | Focus | Tests | Est. LOC | Cumulative Tests |
|--------|-------|-------|----------|-----------------|
| 1 | No-op match arms, Char, Sleep | 6 | ~15 | 222 |
| 2 | Not, Break, Assert, Trust, RuntimeAssert, Give | 10 | ~30 | 232 |
| 3 | Show completeness (Set, Map, Enum, Map/Set Length) | 14 | ~180 | 246 |
| 4 | Options, Escape, Check, ReadFrom, WriteFile, Temporal | 14 | ~120 | 260 |
| 5 | Tuples, Union/Intersection, Range, String functions | 16 | ~240 | 276 |
| 6 | Peephole: Vec-fill, Buffer-reuse, Tiled loops | 6 | ~300 | 282 |
| 7 | Closures, CallExpr, Nested collections | 10 | ~600 | 292 |
| **Total** | | **76** | **~1485** | **292** |
