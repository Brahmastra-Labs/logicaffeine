# PROPER_FUTAMURA.md — TDD Agent Spec for Futamura Projections

LogicAffeine's optimizer pipeline is a collection of eleven excellent local passes —
constant folding, propagation, CTFE, CSE, LICM, closed-form recognition, deforestation,
abstract interpretation, dead code elimination, and a memoized supercompiler. Each pass
does its job well. But a partial evaluator is not a collection of local passes — it is a
single unified framework that subsumes them all. Futamura showed in 1971 that
specializing an interpreter with respect to a program produces a compiled version of
that program; specializing the specializer with respect to the interpreter produces a
compiler; specializing the specializer with respect to itself produces a compiler
generator. These three projections are the theoretical capstone of program
transformation. This document specifies the path from where we are (corner-cut local
passes that cannot handle mixed static/dynamic arguments) to where we need to be (a
self-applicable partial evaluator that achieves all three Futamura projections).

```
                    FULL COVERAGE VERIFICATION
                 All Expr/Stmt variants in Core
                          Sprint 20
                              |
              ================================
              CORE SUBSET EXTENSIONS (9-19)
              Float, Iteration, Sets, Structs,
              Enums, Closures, Temporal, IO,
              Security, CRDTs, Concurrency
              Sprints 9-19  (~173 tests)
              ================================
                              |
                        PROJECTION 3
                   pe(pe, pe) = compiler_generator
                        Sprint 8
                           |
                      PROJECTION 2
                 pe(pe, int) = compiler
                      Sprint 7
                           |
                   SELF-APPLICABLE PE
              PE written in LogicAffeine itself
                      Sprint 6
                           |
                      PROJECTION 1
               pe(int, program) = compiled
                      Sprint 5
                      /        \
     LOGOS-IN-LOGOS              TRUE PE
     SELF-INTERPRETER     Upgraded supercompiler
          Sprint 4             Sprint 3
                                    |
                         FUNCTION SPECIALIZATION
                     Create f_specialized for mixed args
                              Sprint 2
                                    |
                       BINDING-TIME ANALYSIS
                    Classify every variable as S or D
                              Sprint 1
                                    |
                        ===============
                        GROUND (today)
                        ===============
                        11 local passes
                        No BTA, no PE
                        All-or-nothing CTFE
                        ===============
```

---

## TDD Format Specification

This document is a strict TDD spec. An agent follows it step-by-step, top to bottom.
Every sprint is a numbered sequence of **STEPS**. Every step is one of:

| Step Type | What Happens | Expected Outcome |
|-----------|-------------|-----------------|
| **RED** | Write test(s). Run them. They MUST fail. | FAIL (compile error or assertion failure) |
| **GREEN** | Implement the minimum to make RED tests pass. Run them. | PASS |
| **VERIFY** | Run `cargo test -- --skip e2e` on broader test suites. | Zero failures |
| **GATE** | Run full `cargo test -- --skip e2e`. Hard stop if any failure. | Zero failures. Proceed only if green. |

**Rules:**
1. Every RED step specifies exact test names, the file they go in, what source code to use, and what to assert.
2. Every GREEN step specifies exact file paths to create/modify and what to implement.
   **Refactoring invariant:** If a GREEN step touches shared infrastructure (e.g.,
   `embeds()`, `generalize()`, `BtaCache`), verify that ALL previously passing tests
   still pass before proceeding. Shared infrastructure changes require a VERIFY step.
3. Steps have explicit dependencies: "Depends on: Steps 1-3" means those must exist first.
4. Test harness helpers (e.g., `run_interpreter_program()`) are TDD steps too — the FIRST steps in sprints that need them are RED→GREEN cycles for the helpers.
5. Never modify a RED test to make it pass. Fix the implementation.
6. Every sprint ends with a GATE. No proceeding until the gate is green.

**Test helpers available** (in `crates/logicaffeine_tests/tests/common/mod.rs`):
- `compile_to_rust(source) -> Result<String, ParseError>` — compile LOGOS source to Rust code string
- `assert_exact_output(source, expected)` — compile, run, assert stdout matches exactly
- `assert_c_output(source, expected)` — compile to C, run, assert stdout matches exactly
- `run_logos(source) -> E2EResult` — compile and run, return result with stdout/stderr
- `assert_compile_fails(source, expected_error)` — assert compilation fails with error message

**Test count:** 413 total (394 test functions + 19 verification gates).

---

## KNOWN PITFALLS (Read Before Starting)

These are structural hazards discovered during deep audit of the optimizer codebase.
Every pitfall cites exact function/location. Every sprint must account for these.

### Pitfall 1 — Text Values Not Propagated

**Location:** `drive_expr()` match on `Expr::Identifier`, the `Value::Text(_) => {}` guard in `supercompile.rs`

The supercompiler's `Value` enum includes `Text(Symbol)`, and text literals are stored
into the `SuperEnv::store`. But the identifier-substitution path deliberately skips them:

```rust
Expr::Identifier(sym) => {
    if let Some(val) = env.store.get(sym) {
        match val {
            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Nothing => {
                if let Some(lit) = value_to_literal(val) {
                    return expr_arena.alloc(Expr::Literal(lit));
                }
            }
            Value::Text(_) => {}   // <-- deliberately skipped
        }
    }
    expr
}
```

The rationale was Rust move semantics — propagating a string literal creates
independent allocations. But the optimizer operates on the AST, not on Rust code.
Codegen already inserts `.clone()` where needed. The optimizer should propagate all
known values; codegen handles ownership.

**Resolution:** Sprint 3a removes this guard. Text values propagate like any other.

### Pitfall 2 — Index/Slice Never Driven

**Location:** `drive_expr()` passthrough for `Expr::Index | Expr::Slice` in `supercompile.rs`

```rust
Expr::Index { .. } | Expr::Slice { .. } => expr,
```

These expressions are passed through unchanged. This means `Let x be item 1 of items.`
where `items` is known at compile time cannot be evaluated.

**Resolution:** Sprint 3b makes peephole patterns match on AST structure rather than
variable names, so the supercompiler can drive through Index/Slice.

### Pitfall 3 — CTFE All-or-Nothing

**Location:** `try_ctfe_expr()`, the `_ => return None` in argument matching in `ctfe.rs`

```rust
for arg in args {
    match arg {
        Expr::Literal(Literal::Number(n)) => arg_values.push(Value::Int(*n)),
        Expr::Literal(Literal::Float(f)) => arg_values.push(Value::Float(*f)),
        Expr::Literal(Literal::Boolean(b)) => arg_values.push(Value::Bool(*b)),
        Expr::Literal(Literal::Nothing) => arg_values.push(Value::Nothing),
        _ => return None,   // <-- ANY non-literal arg → abandon entirely
    }
}
```

A call like `multiply(3, x)` with one static and one dynamic arg is left untouched.
CTFE cannot create a specialized `multiply_3(x) { return 3 * x; }`.

**Resolution:** Sprint 2 handles mixed args via function specialization with seamless
CTFE integration. When BTA classifies a call as all-static, CTFE evaluates it directly
(no specialization overhead). When BTA classifies it as mixed, the specializer creates
a residual function with static args substituted and dynamic args preserved. The
transition is seamless: the same `analyze_function()` call determines which path to
take. CTFE is not "abandoned" — it remains the fast path for the all-static case.

### Pitfall 4 — CTFE Has No Text Type

**Location:** `Value` enum definition in `ctfe.rs`

```rust
enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Nothing,
}
```

No `Text` variant. Functions that concatenate strings cannot be evaluated at compile time.

**Resolution:** Sprint 3a adds `Text(Symbol)` to the CTFE `Value` enum.

### Pitfall 5 — Crude Loop Generalization

**Location:** `drive_stmt()` match on `Stmt::While`, `collect_modified_vars_block()` call in `supercompile.rs`

```rust
Stmt::While { cond, body, decreasing } => {
    let modified = collect_modified_vars_block(body);
    for sym in &modified {
        env.store.remove(sym);       // <-- forgets EVERYTHING
    }
    // ... drives body ...
    for sym in &modified {
        env.store.remove(sym);       // <-- forgets EVERYTHING again
    }
}
```

This removes ALL modified variables from the store. A true supercompiler uses
homeomorphic embedding and MSG to forget only what's necessary for termination.

**Resolution:** Sprint 3d replaces crude removal with homeomorphic embedding and MSG
(Most Specific Generalization). When embedding detects a growing configuration, MSG
computes the invariant part shared between the current and predecessor stores, and
forgets only the varying part — the minimum necessary for termination. Variables whose
values are identical across iterations remain in the store as concrete values.

### Pitfall 6 — No BTA Anywhere

No pass classifies variables as Static or Dynamic. The supercompiler tracks concrete
values via `SuperEnv::store`, but absence from the store means "unknown" — there is no
formal distinction between "dynamic because it's a function parameter" and "unknown
because we haven't reached its definition yet." The optimizer cannot generate residual
code for mixed static/dynamic expressions.

**Resolution:** Sprint 1 adds formal Binding-Time Analysis.

### Pitfall 7 — Propagation Skips Text

**Location:** `is_propagatable_literal()` function in `propagate.rs`

```rust
fn is_propagatable_literal(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(
        Literal::Number(_) | Literal::Float(_) | Literal::Boolean(_) | Literal::Nothing
    ))
}
```

`Literal::Text` is excluded from propagation.

**Resolution:** Sprint 3a fixes both `propagate.rs` and `supercompile.rs`.

### Pitfall 8 — Supercompiler `_interner` Unused

**Location:** `supercompile_stmts()` signature, `_interner` parameter in `supercompile.rs`

```rust
pub fn supercompile_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    _interner: &mut Interner,        // <-- accepted but unused
) -> Vec<Stmt<'a>> {
```

Function specialization (Sprint 2) requires interning names like `multiply_s0_3`.

**Resolution:** Sprint 2 removes the underscore prefix and uses the interner.

### Pitfall 9 — Camp 9 Never Built

`optimize/partial_eval.rs` does not exist. The supercompiler was a stand-in that handles
the all-static case but cannot handle the mixed static/dynamic case.

**Resolution:** Sprints 1-2 build BTA + specialization. Sprint 3 upgrades the supercompiler.

### Pitfall 10 — No Fixpoint in Pipeline

**Location:** `optimize_program()` function in `optimize/mod.rs`

The optimizer pipeline is a linear 11-pass sequence. Each pass runs exactly once. If PE
creates new constant expressions, fold does not run again to simplify them.

**Resolution:** After inserting PE (Sprint 2), add a fixpoint loop:
`fold → propagate → PE → fold` iterated until no changes occur (max 8 cycles).

---

## Infrastructure to Reuse

These existing modules provide infrastructure that the PE sprints build on. Do not
reimplement what already exists.

| Existing Module | Location | How PE Reuses It |
|-----------------|----------|------------------|
| Effect analysis | `optimize/effects.rs` | `EffectEnv::function_is_pure()` gates PE: never specialize impure functions |
| Call graph with SCC | `analysis/callgraph.rs` | SCC ordering for fixed-point BTA on mutually recursive functions |
| Constant folding | `optimize/fold.rs` | Rerun on specialized function bodies to simplify after substitution |
| Dead code elimination | `optimize/dce.rs` | Rerun on specialized bodies to remove dead branches |
| SuperEnv value tracking | `optimize/supercompile.rs` | Pattern for `store: HashMap<Symbol, Value>` — BTA Division uses same shape |
| CTFE step-limited eval | `optimize/ctfe.rs` | All-static calls delegated to CTFE unchanged; PE handles mixed-arg calls |
| Symbol interner | `compile.rs` | PE interns names for specialized function variants |
| Readonly analysis | `analysis/readonly.rs` | Identifies immutable parameters (always S in BTA) |

### Pipeline Order

**Current Pipeline:**
```
fold → propagate → ctfe → fold → cse → licm → closed_form → deforest → abstract_interp → dce → supercompile
```

**After Sprint 2 (PE Inserted):**
```
fold → propagate → partial_eval → ctfe → fold → cse → licm → closed_form → deforest → abstract_interp → dce → supercompile
```

**After Sprint 2 (Fixpoint Variant):**
```
LOOP {
    fold → propagate → partial_eval
} UNTIL stable (max 8 cycles)
→ ctfe → fold → cse → licm → closed_form → deforest → abstract_interp → dce → supercompile
```

Termination guarantee: each iteration either reduces the AST or is identity. Code size
limit (2x original) in PE prevents unbounded growth. Hard limit of 8 cycles.

---

## Sprint 1 — Binding-Time Analysis (BTA)

### Overview

At each function call site, classify every variable and expression as **Static** (S) —
its value is known at compile time — or **Dynamic** (D) — its value depends on runtime
input. This classification is called a *division*. The division drives all subsequent
specialization: S expressions are evaluated at compile time, D expressions become
residual code.

Without BTA, the optimizer faces a binary choice: either ALL arguments are known (CTFE)
or ANY argument is unknown (nothing happens). BTA enables a third path: SOME arguments
are known, and the function can be *specialized* for those known values.

**Citations:**
- Nielson & Nielson, "Two-Level Functional Languages" (1992). Formalizes BTA.
- Jones, Gomard & Sestoft, "Partial Evaluation and Automatic Program Generation" (1993),
  Chapter 4: Binding-Time Analysis.

### Data Structures

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingTime {
    Static(Literal),
    Dynamic,
}

pub type Division = HashMap<Symbol, BindingTime>;

pub struct BtaResult {
    pub division: Division,
    pub return_bt: BindingTime,
}

pub type BtaCache = HashMap<(Symbol, Vec<BindingTime>), BtaResult>;
```

> **Polyvariant BTA.** The `BtaCache` is keyed on `(function_symbol, arg_binding_times)`,
> not just `function_symbol`. This means the same function analyzed at two different call
> sites with different argument divisions produces two distinct `BtaResult`s. For example,
> `multiply(S(3), D)` and `multiply(D, S(5))` yield different divisions — the first has
> `a=S(3), b=D`, the second has `a=D, b=S(5)`. Monovariant BTA (one result per function)
> would conservatively join these to `a=D, b=D`, losing all specialization opportunity.
> Polyvariant BTA preserves per-call-site precision.

### Algorithm

1. **Entry point:** `analyze_function(func, arg_bts, cache: &mut BtaCache) -> BtaResult`
2. **Cache check:** Compute key `(func.name, arg_bts.clone())`. If `cache[key]` exists,
   return the cached result immediately. This makes BTA polyvariant — the same function
   with different call-site divisions produces different results, and repeated queries
   with identical divisions are O(1).
3. **Initialize division:** For each parameter `p_i`, set `division[p_i] = arg_bts[i]`.
4. **Forward dataflow** — process statements in order:
   - `Let x = expr` → `division[x] = bt(expr)`
   - `Set x = expr` → `division[x] = bt(expr)`. S→D transition is permanent.
   - `If cond` → S(true): only then. S(false): only else. D: both, join divisions.
     Join: `S(v) ⊔ S(v) = S(v)`, `S(v1) ⊔ S(v2) = D`, `_ ⊔ D = D`.
   - `While cond: body` → fixed-point: re-analyze until no division changes. Terminates
     because `{S(v)} → D` is finite and monotone.
   - `Return expr` → `return_bt = bt(expr)`.
5. **Expression binding time** `bt(expr)`:
   - `Literal(_)` → `S(literal_value)`
   - `Identifier(sym)` → `division[sym]`
   - `BinaryOp(left, right)` → S(v1) op S(v2) = S(result). Otherwise D.
   - `Call(f, args)` → look up f's BTA result for given arg binding times. Recursive
     via SCC from `analysis/callgraph.rs`.
   - `Not(expr)` → S(Bool(b)) → S(Bool(!b)). Otherwise D.
   - `Length(_)` → D (collection lengths are runtime-dependent)
   - `Index(_, _)` → D (array contents are runtime-dependent)
   - All other → D (conservative)

### New File

`crates/logicaffeine_compile/src/optimize/bta.rs`

### Modified Files

- `optimize/mod.rs` — add `pub mod bta;`

### TDD Steps

#### STEP 1: RED — Expression type classification

**File:** `crates/logicaffeine_tests/tests/phase_bta.rs`

Write the following tests. All must fail (module `bta` does not exist yet).

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** FAIL (compile error — `bta` module not found)

**Tests (12):**

1. `bta_literal_int_static`
   Source: `Let x be 42. Show x.`
   Verify: BTA classifies `42` as Static(42). After pipeline, value is inlined.
   Assert: Generated Rust contains inlined `42` — no dynamic variable lookup.

2. `bta_literal_float_static`
   Source: `Let x be 3.14. Show x.`
   Verify: BTA classifies `3.14` as Static(3.14).
   Assert: Generated Rust contains inlined `3.14`.

3. `bta_literal_bool_static`
   Source: `Let x be true. Show x.`
   Verify: BTA classifies `true` as Static(true).

4. `bta_literal_text_static`
   Source: `Let x be "hello". Show x.`
   Verify: BTA classifies `"hello"` as Static(Text). (Depends on Sprint 3a for full text propagation, but BTA classification itself should work.)

5. `bta_literal_nothing_static`
   Source: `Let x be nothing. Show x.`
   Verify: BTA classifies `nothing` as Static(Nothing).

6. `bta_identifier_tracks_division`
   Source: Function with param `x: Int` called with S(10).
   Verify: `bt(Identifier(x))` returns `division[x]` = S(10).
   Assert: BTA result has `x → S(10)`.

7. `bta_binop_static_static`
   Source: `Let x be 2 + 3.`
   Verify: S(2) + S(3) → S(5). Both operands static → result static.

8. `bta_binop_static_dynamic`
   Source: Function `f(a: Int, b: Int)` with `Return a + b.` Called with S(3), D.
   Verify: S(3) + D → D. Any dynamic operand makes result dynamic.

9. `bta_binop_dynamic_dynamic`
   Source: Function with two dynamic params, `Return a * b.`
   Verify: D * D → D.

10. `bta_not_static`
    Source: `Let x be not true.`
    Verify: not S(true) → S(false).

11. `bta_length_is_dynamic`
    Source: `Let n be length of items.` where items is a Seq param.
    Verify: Length of collection → always D (conservative).

12. `bta_index_is_dynamic`
    Source: `Let x be item 1 of items.` where items is a Seq param.
    Verify: Index into collection → always D (conservative).

#### STEP 2: GREEN — Implement core BTA for expressions

**File:** Create `crates/logicaffeine_compile/src/optimize/bta.rs`
**Also:** Add `pub mod bta;` to `optimize/mod.rs`

Implement:
- `BindingTime` enum, `Division` type, `BtaResult` struct
- `analyze_expr(expr, division) -> BindingTime` handling Literal, Identifier, BinaryOp, Not, Length, Index
- Stub `analyze_function()` that initializes division from arg_bts and processes a flat sequence of Let statements

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** PASS (all 12 expression tests)

#### STEP 3: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures. BTA is additive — no existing pass modified.

#### STEP 4: RED — Control flow analysis

**File:** `crates/logicaffeine_tests/tests/phase_bta.rs` (append)

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** FAIL (control flow not yet handled in BTA)

**Tests (6):**

13. `bta_if_static_true_only_then`
    Source: Function `select(flag: Bool, x: Int, y: Int)` called with S(true), D, S(0).
    Verify: BTA with S(true) condition analyzes only then-branch. Else-branch is dead.
    Assert: Return BT comes from then-branch only.

14. `bta_if_static_false_only_else`
    Source: Same function called with S(false), S(1), D.
    Verify: BTA analyzes only else-branch. Then-branch is dead.

15. `bta_if_dynamic_both_branches`
    Source: Function `abs(x: Int)` with `If x > 0: Return x. Otherwise: Return 0 - x.`
    Called with D.
    Verify: Dynamic condition → both branches analyzed, divisions joined.

16. `bta_if_join_same_value`
    Source: `If flag: Let y be 5. Otherwise: Let y be 5.` with flag=D.
    Verify: Both branches assign S(5) to y. Join: S(5) ⊔ S(5) = S(5). y is still static.

17. `bta_while_fixpoint_converges`
    Source: `While i <= n: Set sum to sum + i. Set i to i + 1.` with n=D.
    Verify: Fixpoint iteration. i starts S, becomes D (written in D-bounded loop).
    sum starts S(0), becomes D. Fixpoint reached in ≤2 iterations.

18. `bta_nested_if`
    Source: Nested `If` — outer condition D, inner condition S(true).
    Verify: Inner S(true) branch eliminates the inner else even though outer is D.

#### STEP 5: GREEN — Implement control flow BTA

**File:** `crates/logicaffeine_compile/src/optimize/bta.rs`

Extend `analyze_function()` to handle:
- `Stmt::If` — check condition BT, branch accordingly, implement join
- `Stmt::While` — fixed-point loop over body until division stabilizes
- Nested control flow via recursive block analysis

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** PASS (all 18 tests)

#### STEP 6: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures.

#### STEP 7: RED — Function call patterns

**File:** `crates/logicaffeine_tests/tests/phase_bta.rs` (append)

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** FAIL (function call BTA not implemented)

**Tests (6):**

19. `bta_all_static_args`
    Source: `add5(x: Int) -> Int: Return x + 5.` Called with S(10).
    Verify: x=S(10), x+5=S(15), return=S(15). Entire call is static.

20. `bta_all_dynamic_args`
    Source: `double(x: Int) -> Int: Return x * 2.` Called with D.
    Verify: x=D, return=D.

21. `bta_mixed_args`
    Source: `multiply(a: Int, b: Int) -> Int: Return a * b.` Called with S(3), D.
    Verify: a=S(3), b=D, a*b=D, return=D. This is the PE target case.

22. `bta_recursive_static`
    Source: `factorial(n: Int)` called with S(5).
    Verify: BTA traces through recursion via SCC from `analysis/callgraph.rs`.
    Return=S(120). Recursive BTA with all-static args fully evaluates.

23. `bta_mutual_recursion_scc`
    Source: `f(x)` calls `g(x-1)`, `g(x)` calls `f(x-1)`. Called with D.
    Verify: SCC ordering processes {f, g} together. Both return D.
    Assert: No infinite loop — cycle detection via SCC terminates.

24. `bta_nested_call_chain`
    Source: `f(g(3), h(x))` where g is pure, h is identity.
    Verify: g(3) → S(result), h(x) → D. f gets S, D as args.
    Assert: BTA analyzes g and h first to determine f's arg binding times.

#### STEP 8: GREEN — Implement function call BTA

**File:** `crates/logicaffeine_compile/src/optimize/bta.rs`

Implement:
- `analyze_call(function, arg_bts)` — look up function def, recursively analyze
- SCC integration from `analysis/callgraph.rs` for mutual recursion
- Memoization: cache `(function, arg_bts) → BtaResult` to prevent re-analysis
- Cycle detection: if analyzing a function already on the stack, assume return=D

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** PASS (all 24 tests)

#### STEP 9: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures.

#### STEP 10: RED — Edge cases

**File:** `crates/logicaffeine_tests/tests/phase_bta.rs` (append)

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** FAIL (edge cases not handled)

**Tests (6):**

25. `bta_mutable_s_to_d_transition`
    Source: `Let mutable x be 5. Set x to y.` where y=D.
    Verify: x starts S(5), transitions to D after Set. Once D, never becomes S.
    Assert: Final division has x=D.

26. `bta_collection_params_always_d`
    Source: Function `f(items: Seq of Int)` — items is a collection parameter.
    Verify: Collection params are always D regardless of call-site binding time.
    Assert: division[items] = D even if called with a literal collection.

27. `bta_set_makes_dynamic`
    Source: `Let mutable y be 5. Set y to x.` where x=D. `Return y + 1.`
    Verify: y=S(5) initially, then D after Set. y+1=D. return=D.

28. `bta_branch_dynamic_condition`
    Source: `abs(x: Int)` with x=D. `If x > 0: Return x. Otherwise: Return 0 - x.`
    Verify: Condition D → both branches live, return=D.
    E2E: assert_exact_output with input "5" → "5", input "-3" → "3".

29. `bta_loop_static_bound`
    Source: `sumTo(n: Int)` with n=S(10). Loop `Repeat for i from 1 to n`.
    Verify: n=S(10), loop bound S → unrollable. i=S on each iteration. sum=S(55).
    Assert: return=S(55).

30. `bta_loop_dynamic_bound`
    Source: Same `sumTo(n)` with n=D.
    Verify: n=D, loop bound D → preserve structure. i=D, sum=D, return=D.

31. `bta_polyvariant_different_sites`
    Source: `multiply(a: Int, b: Int) -> Int: Return a * b.`
    Called at site 1 with S(3), D. Called at site 2 with D, S(5).
    Verify: BtaCache contains two distinct entries for `multiply`:
    key (multiply, [S(3), D]) → {a=S(3), b=D, return=D}
    key (multiply, [D, S(5)]) → {a=D, b=S(5), return=D}
    Assert: Polyvariant analysis produces different divisions per call site.

32. `bta_polyvariant_cache_hit`
    Source: `add(a: Int, b: Int) -> Int: Return a + b.`
    Called at sites 1, 2, and 3 — all with S(10), D.
    Verify: BtaCache is queried 3 times with key (add, [S(10), D]).
    First call computes the result. Second and third return the cached result.
    Assert: Only one `analyze_function` computation occurs (cache hit on calls 2, 3).

33. `bta_polyvariant_recursive_distinct`
    Source: `power(base: Int, exp: Int) -> Int: If exp <= 0: Return 1. Return base * power(base, exp - 1).`
    Called with S(2), S(5) at site 1 and S(3), S(4) at site 2.
    Verify: BtaCache contains two entries with different static values.
    Assert: Both entries exist and have return=S (fully static).

#### STEP 11: GREEN — Implement edge case handling

**File:** `crates/logicaffeine_compile/src/optimize/bta.rs`

Handle:
- Mutable variable S→D transition (Set with D value pessimizes permanently)
- Collection parameters always classified as D
- Static loop bounds enabling unrolling (up to 256 iterations)

**Run:** `cargo test --test phase_bta -- --skip e2e`
**Expected:** PASS (all 33 tests)

#### STEP 12: VERIFY — No regressions

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures across all test suites.

### VERIFICATION GATE — Sprint 1

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 33 BTA tests pass (including 3 polyvariant tests). No regressions in existing tests.
**Hard stop:** Do NOT proceed to Sprint 2 until this gate is green.

---

## Sprint 2 — Function Specialization

### Overview

When a function is called with some arguments known at compile time and others unknown,
create a *specialized* version with the known arguments baked in. The specialized version
has fewer parameters (only the dynamic ones) and a simplified body (static arguments
substituted, dead branches eliminated, static computations evaluated).

This is the central operation of partial evaluation. CTFE (all-static) and normal
compilation (all-dynamic) are the two degenerate cases. Specialization handles everything
in between.

**Example:**
```
## To multiply (a: Int, b: Int) -> Int:
    Return a * b.

Let y be multiply(3, x).
```

BTA classifies: `a = S(3)`, `b = D`. Specialization creates:
```
## To multiply_s0_3 (b: Int) -> Int:
    Return 3 * b.

Let y be multiply_s0_3(x).
```

**Citations:**
- Futamura, Y. "Partial Evaluation of Computation Process" (1971).
- Jones, Gomard & Sestoft (1993), Chapter 5: Function Specialization.

### Data Structures

```rust
type SpecKey = (Symbol, Vec<Option<Literal>>);

pub struct SpecRegistry {
    pub cache: HashMap<SpecKey, Symbol>,
    pub new_funcs: Vec<Stmt>,
    pub variant_count: HashMap<Symbol, usize>,
    pub history: Vec<SpecKey>,
}
```

> **Termination via embedding, not depth counting.** `history` records the sequence of
> specialization keys encountered during recursive specialization. Before each new
> specialization, the current key is checked against the history using the same
> `embeds()` function from Sprint 3d. If the current key is homeomorphically embedded
> in a predecessor, MSG generalizes the static arguments and specialization emits a
> residual call instead of recursing. This replaces the arbitrary `depth < 16` counter
> with a mathematically principled termination guarantee. Variant limit (≤ 8 per
> function) remains as a code-size safety net.

### Algorithm

1. **Identify candidates:** For each `Expr::Call`, run BTA on args. Skip if all-D (no
   benefit) or all-S (delegate to CTFE). Proceed if mixed.
2. **Check safety:** `EffectEnv::function_is_pure()` from `optimize/effects.rs`. NEVER
   specialize impure functions. Check `variant_count[f] < 8`. Check embedding:
   if current `SpecKey` is homeomorphically embedded in any entry in `history`,
   generalize via MSG and emit residual instead of specializing further.
3. **Compute memo key:** `(function_sym, [Some(lit) if S else None per arg])`.
   If key exists in cache: reuse. Replace call with `Call { cache[key], [D args only] }`.
4. **Create specialized variant:** Fresh symbol via interner. Clone body. Substitute S
   params with literals. Run fold + DCE on specialized body. **Simplicity check:** if
   `specialized_stmt_count > original * 0.8`, discard (not enough simplification).
5. **Replace call site:** `Call { specialized_sym, [D args only] }`.
6. **Pipeline position:** After propagation, before CTFE.

### New File

`crates/logicaffeine_compile/src/optimize/partial_eval.rs`

### Modified Files

- `optimize/mod.rs` — add `pub mod partial_eval;`, insert into pipeline after propagate
- `optimize/supercompile.rs` — remove underscore from `_interner` parameter

### TDD Steps

#### STEP 1: RED — Specialization mechanics

**File:** `crates/logicaffeine_tests/tests/phase_partial_eval.rs`

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** FAIL (module `partial_eval` does not exist)

**Tests (8):**

1. `pe_creates_specialized_function`
   Source: `multiply(a, b)` called with S(3), D.
   Verify: A new function definition appears in the generated code.
   Assert: Generated Rust contains a function named like `multiply_s0_3`.

2. `pe_static_params_removed`
   Source: Same multiply with S(3), D.
   Verify: Specialized function has ONE parameter (b), not two.
   Assert: Function signature in generated code has one param.

3. `pe_body_substituted`
   Source: Same multiply.
   Verify: Body contains `3 * b` — the static param `a` is replaced with its value.
   Assert: Generated code contains literal `3` in the multiplication.

4. `pe_specialized_name_format`
   Source: `scale(factor, x)` called with S(2), D.
   Verify: Specialized name follows pattern `funcname_s<indices>_<hash>`.
   Assert: Name contains "scale" and encodes the static arg position.

5. `pe_call_site_rewritten`
   Source: `multiply(3, n)` where n is dynamic.
   Verify: Call site is rewritten to pass only the dynamic arg.
   Assert: Generated call has ONE argument, not two.

6. `pe_fold_runs_on_specialized_body`
   Source: `f(a, b): Let c be a + 1. Return c * b.` Called with S(4), D.
   Verify: After substitution a=4, fold simplifies c=4+1=5. Body becomes `5 * b`.
   Assert: Generated code contains `5 * b` or `5 *` (fold ran on specialized body).

7. `pe_dce_runs_on_specialized_body`
   Source: `select(flag, a, b): If flag: Return a. Otherwise: Return b.` Called with S(true), D, D.
   Verify: After substitution flag=true, DCE removes else-branch.
   Assert: Specialized body has NO if/else — only `Return a`.

8. `pe_simplicity_check`
   Source: Function with 4 if-branches, specialized with one static flag that eliminates 3 branches.
   Verify: Simplicity check PASSES — specialized is much smaller (1 branch vs 4).
   Assert: Specialization is accepted (not rejected by size check).

#### STEP 2: GREEN — Implement core specialization

**File:** Create `crates/logicaffeine_compile/src/optimize/partial_eval.rs`
**Also:** Update `optimize/mod.rs`, fix `_interner` in `supercompile.rs`

Implement:
- `SpecRegistry` struct with cache, new_funcs, variant_count, depth
- `specialize_call(call, division, registry, interner)` — the core specialization logic
- `specialize_stmts(stmts, ...)` — walk statements, find Call expressions, attempt specialization
- Integrate with BTA from Sprint 1 to classify arguments
- Run fold + DCE on specialized bodies
- Simplicity check (0.8x threshold)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** PASS (all 8 tests)

#### STEP 3: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures.

#### STEP 4: RED — Memoization

**File:** `crates/logicaffeine_tests/tests/phase_partial_eval.rs` (append)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** FAIL

**Tests (5):**

9. `pe_same_key_reuses`
   Source: `f(3, x)` called TWICE with same static value 3.
   Verify: Second call reuses the already-specialized variant. Only ONE specialized function emitted.
   Assert: Generated code has exactly one `f_s0_3` definition, called twice.

10. `pe_different_key_creates_new`
    Source: `f(3, x)` and `f(5, x)` — same function, different static values.
    Verify: Two different specialized variants created (different keys).
    Assert: Generated code has both `f_s0_3` and `f_s0_5`.

11. `pe_variant_limit_8`
    Source: `scale(factor, x)` called with 10 different static factors (1..10).
    Verify: At most 8 specialized variants. Calls 9 and 10 use unspecialized `scale`.
    Assert: Exactly 8 specialized function defs. Two calls use original `scale`.

12. `pe_embedding_terminates`
    Source: `deep(n, x): If n <= 0: Return x. Return deep(n-1, x+1).` Called with S(20), D.
    Verify: Homeomorphic embedding on SpecKey history detects the growing pattern
    (n=20 → n=19 → n=18...) and generalizes via MSG instead of unfolding all 20 levels.
    Assert: Compilation terminates. Residual contains a recursive call or loop for the
    generalized computation. Output is still correct (deep(20, 5) = 25).

13. `pe_interner_used_for_names`
    Source: Any specialization scenario.
    Verify: Specialized function names are properly interned via the Interner.
    Assert: The interner contains the new symbol after specialization.

#### STEP 5: GREEN — Implement memoization and limits

**File:** `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

Implement:
- SpecKey computation: `(function_sym, [Some(lit) if S else None per arg])`
- Cache lookup before creating new variants
- variant_count per function, hard limit 8
- depth tracking, hard limit 16
- Interner integration for generating fresh names

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** PASS (all 13 tests)

#### STEP 6: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures.

#### STEP 7: RED — Multiple call sites and cascading

**File:** `crates/logicaffeine_tests/tests/phase_partial_eval.rs` (append)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** FAIL

**Tests (5):**

14. `pe_same_static_reuses`
    Source: `f(3, a)` and `f(3, b)` at two different call sites with same static value.
    Verify: Same specialized variant used at both sites.
    Assert: Only one `f_s0_3` definition in generated code.

15. `pe_different_static_creates`
    Source: `f(3, a)` and `f(7, b)` — different static values.
    Verify: Two different specialized variants.

16. `pe_cascading_specialization`
    Source: `f(a, b): Return g(a, b).` `g(x, y): Return x * y.`
    Called `f(3, n)` — f is specialized, and inside f's specialized body, g(3, n) is
    also a specialization candidate.
    Verify: Both f and g get specialized. Cascading specialization works.

17. `pe_nested_specialization`
    Source: `outer(a, b): Return inner(a, b) + inner(a, b+1).`
    `inner(x, y): Return x * y.` Called `outer(5, n)`.
    Verify: outer specialized for a=5. Inside outer_spec, inner(5, n) and inner(5, n+1)
    both specialize — but both reuse same inner_s0_5 variant (same key).

18. `pe_multiple_static_params`
    Source: `f(a, b, c): Return a * b + c.` Called with S(2), D, S(7).
    Verify: Both a=2 and c=7 are substituted. Specialized function has ONE param (b).
    Body becomes `2 * b + 7`.
    Assert: Generated specialized function has one parameter and substituted body.

#### STEP 8: GREEN — Handle multiple call sites and cascading

**File:** `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

Implement:
- Walk all call sites in the program (not just the first)
- Cascading: after creating a specialized function body, scan it for further
  specialization candidates (recursive specialization walk)
- Multiple static params: substitute all S params, keep only D params

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** PASS (all 18 tests)

#### STEP 9: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures.

#### STEP 10: RED — Safety guards

**File:** `crates/logicaffeine_tests/tests/phase_partial_eval.rs` (append)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** FAIL

**Tests (5):**

19. `pe_impure_skipped`
    Source: `sideEffect(n: Int): Show n.` Called with S(5).
    Verify: Function has IO effect → NOT specialized at compile time.
    Assert: Generated code still contains `sideEffect(5)` or equivalent Show call.

20. `pe_io_preserved`
    Source: `f(a, b): Show a. Return b.` Called with S(3), D.
    Verify: Even though a=S(3), function has IO → not specialized.
    Assert: Show call preserved in output.

21. `pe_collections_as_dynamic`
    Source: `f(items: Seq of Int, x: Int)` called with a literal collection and S(5).
    Verify: Collection param always treated as D. Only x=S(5) is specialized.
    Assert: Specialized function still has items param.

22. `pe_self_referential_reuse`
    Source: `f(a, b): If b <= 0: Return a. Return f(a, b-1).` Called with S(3), D.
    Verify: Inside f_s0_3's body, the recursive call f(3, b-1) has same key → reuses
    f_s0_3. No infinite specialization.
    Assert: Only ONE specialized variant. Recursive call targets same variant.

23. `pe_no_specialize_all_dynamic`
    Source: `add(a, b): Return a + b.` Called with D, D.
    Verify: Both args dynamic → no specialization benefit. Call left unchanged.
    Assert: Generated code calls `add(x, z)` with two args, no specialized function.

#### STEP 11: GREEN — Implement safety guards

**File:** `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

Implement:
- Effect check via `EffectEnv::function_is_pure()` — skip impure functions. Note: `function_is_pure()` takes `&str`, not `Symbol`. Resolve via `interner.resolve(sym)` before calling.
- Collection params always classified as D in BTA
- Self-referential call detection via memo key matching during body specialization
- Skip when all args are D (no benefit)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** PASS (all 23 tests)

#### STEP 12: VERIFY — No regressions

**Run:** `cargo test --test phase_optimize -- --skip e2e`
**Expected:** Zero failures.

#### STEP 13: RED — Pipeline integration and E2E correctness

**File:** `crates/logicaffeine_tests/tests/phase_partial_eval.rs` (append)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** FAIL

**Tests (12):**

24. `pe_after_propagate`
    Source: `Let a be 3. Let y be multiply(a, n).` (n dynamic)
    Verify: Propagation resolves a=3 before PE runs. PE sees multiply(S(3), D).
    Assert: Specialization fires even though the static value came from propagation.

25. `pe_before_ctfe`
    Source: `add5(x): Return x + 5.` Called with S(10).
    Verify: PE could specialize, but all args S → CTFE handles it instead.
    Assert: Result is fully evaluated to 15 (CTFE path).

26. `pe_fixpoint_terminates`
    Source: `f(a, b): Let c be a + 1. Return g(c, b).` `g(x, y): Return x * y.`
    Called with S(2), D. PE creates f_spec with c=3, calls g(3, b). Fold/propagate/PE
    loop iterates.
    Verify: Fixpoint loop terminates (max 8 cycles).
    Assert: Compilation completes in finite time.

27. `pe_constant_arg_fully_evaluated`
    Source: `add5(10)` — all static.
    Verify: PE detects all-S → delegates to CTFE → result is 15.
    Assert: Generated code contains `let y = 15` or equivalent.

28. `pe_pipeline_fold_interaction`
    Source: `f(a, b): Return (a + 1) * b.` Called with S(4), D.
    Verify: After substitution a=4, fold simplifies `(4+1)*b` → `5*b`.
    Assert: Generated code contains `5 * b`, not `(4 + 1) * b`.

29. `pe_factorial_output`
    Source: `factorial(10)` — fully static recursive call.
    Verify: CTFE or recursive specialization fully evaluates.
    E2E: assert_exact_output → "3628800"

30. `pe_branch_elimination_output`
    Source: `select(true, n, 0)` where n = parseInt(args).
    Verify: Branch eliminated, result is just n.
    E2E: assert_exact_output with input "7" → "7"

31. `pe_partial_specialization_output`
    Source: `multiply(3, n)` where n = parseInt(args).
    Verify: Specialized to `3 * n`.
    E2E: assert_exact_output with input "7" → "21"

32. `pe_recursive_memoization`
    Source: `factorial(10)` via recursive specialization.
    Verify: Memoization prevents exponential blowup.
    E2E: assert_exact_output → "3628800"

33. `pe_code_bloat_limit`
    Source: `scale(factor, x)` called with 10 different static factors.
    Verify: At most 8 variants. Program still produces correct output for all.
    E2E: Sum of scale results is correct for any input.

34. `pe_depth_limit_preserves_correctness`
    Source: `deep(20, n)` — recursive with depth > limit.
    Verify: Depth limit fires, residual has recursive call. Output still correct.
    E2E: assert_exact_output with input "5" → "25" (n + 20)

35. `pe_simplicity_check_passes`
    Source: `complex(1, n)` — 4 branches, one static flag eliminates 3.
    Verify: Simplicity check passes (1 branch is much smaller than 4).
    E2E: assert_exact_output with input "10" → "11" (n + 1)

#### STEP 14: GREEN — Pipeline integration

**File:** `crates/logicaffeine_compile/src/optimize/partial_eval.rs` and `optimize/mod.rs`

Implement:
- Insert `partial_eval::specialize_stmts` into pipeline after propagate, before CTFE
- Fixpoint loop: `fold → propagate → PE` iterated until stable (max 8 cycles)
- Change counter: each pass increments when it modifies the AST
- All-S detection → delegate to CTFE (don't specialize when CTFE handles it)

**Run:** `cargo test --test phase_partial_eval -- --skip e2e`
**Expected:** PASS (all 35 tests)

#### STEP 15: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures across all test suites.

### VERIFICATION GATE — Sprint 2

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 35 PE tests pass. All 33 BTA tests still pass.
No regressions in phase_optimize or any other test suite.
**Hard stop:** Do NOT proceed to Sprint 3 until this gate is green.

---

## Sprint 3a — Text Propagation and CTFE Text Support

### Overview

Remove the text-value guards in both the supercompiler and the propagation pass. Add
`Text(Symbol)` to the CTFE `Value` enum. After this sub-sprint, all three optimization
passes (propagate, CTFE, supercompile) can handle string values at compile time.

### Algorithm

1. **supercompile.rs** — Remove the `Value::Text(_) => {}` guard in `drive_expr()`.
   Replace with `Value::Text(s) => return expr_arena.alloc(Expr::Literal(Literal::Text(*s)));`

2. **propagate.rs** — Extend `is_propagatable_literal()` to include `Literal::Text(_)`.

3. **ctfe.rs** — Add `Text(Symbol)` to Value enum. Add `Literal::Text → Value::Text`
   in argument processing and `Value::Text → Literal::Text` in result conversion.

### Modified Files

- `optimize/supercompile.rs` — remove text guard in `drive_expr()`
- `optimize/propagate.rs` — extend `is_propagatable_literal()`
- `optimize/ctfe.rs` — add `Text(Symbol)` to `Value`, add conversion paths

### TDD Steps

#### STEP 1: RED — Text propagation tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL (text values not propagated — these tests assert behavior the guards prevent)

**Tests (12):**

1. `text_supercompile_propagates`
   Source: `Let name be "Alice". Show name.`
   Verify: Supercompiler substitutes `name` with literal `"Alice"`.
   Assert: Generated code contains direct `"Alice"` in the Show, not a variable lookup.

2. `text_propagate_constant_prop`
   Source: `Let x be "world". Show x.`
   Verify: Constant propagation pass substitutes x → `"world"`.
   Assert: Generated code contains `println!("world")` or equivalent.

3. `text_ctfe_pure_function`
   Source: `greet(name: Text) -> Text: Return "Hello, " + name + "!".`
   Called with `greet("Bob")`.
   Verify: CTFE evaluates `greet("Bob")` to `"Hello, Bob!"` at compile time.
   Assert: Generated code contains the literal `"Hello, Bob!"`.

4. `text_ctfe_concat`
   Source: `Let x be "Hello, " + "World" + "!". Show x.`
   Verify: CTFE folds the concatenation at compile time.
   Assert: Generated code contains `"Hello, World!"` as a single literal.

5. `text_ctfe_compare`
   Source: `Let x be "a" equals "a". Show x.`
   Verify: CTFE evaluates string comparison at compile time.
   Assert: Generated code contains `true` (folded).

6. `text_symbol_identity_preserved`
   Source: `Let a be "hello". Let b be a. Show b.`
   Verify: After propagation, both a and b reference the same interned Symbol.
   Assert: No duplicate string allocation in generated code.

7. `text_codegen_ownership`
   Source: `Let x be "test". Show x. Show x.`
   Verify: After text propagation, codegen correctly handles ownership (`.clone()`).
   Assert: Compiles and runs without ownership errors. No panics.

8. `text_cross_function_propagation`
   Source: `f() -> Text: Return "static".` `Let x be f(). Show x.`
   Verify: CTFE evaluates f() → "static". Propagation substitutes x.
   Assert: Generated code inlines `"static"`.

9. `text_ctfe_mixed_not_evaluated`
   Source: `greet(name: Text) -> Text: Return "Hello, " + name + "!".`
   Called with `greet(item 1 of args())` — dynamic arg.
   Verify: CTFE does NOT evaluate (arg is dynamic). Call preserved.
   Assert: Generated code contains the function call, not a literal.

10. `text_e2e_hello_alice`
    Source: `Let name be "Alice". Let msg be "Hello, " + name. Show msg.`
    E2E: assert_exact_output → "Hello, Alice"

11. `text_e2e_greeting_function`
    Source: `greet("Bob")` function that concatenates.
    E2E: assert_exact_output → "Hello, Bob!"

12. `text_propagate_multiple_uses`
    Source: `Let x be "hi". Show x + " " + x.`
    Verify: Text propagated into both uses of x.
    E2E: assert_exact_output → "hi hi"

#### STEP 2: GREEN — Remove text guards, add CTFE text support

**Files:**
- `optimize/supercompile.rs` — change `Value::Text(_) => {}` to emit `Expr::Literal(Literal::Text(s))`
- `optimize/propagate.rs` — add `| Literal::Text(_)` to `is_propagatable_literal()`
- `optimize/ctfe.rs` — add `Text(Symbol)` to Value enum, add `Literal::Text(s) => Value::Text(s)` in arg matching, add `Value::Text(s) => Some(Literal::Text(s))` in result conversion

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 12 text tests)

#### STEP 3: VERIFY — No regressions

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Text propagation is a bug fix, not a new feature.

---

## Sprint 3b — Index/Slice Driving

### Overview

Allow the supercompiler to drive through `Expr::Index` and `Expr::Slice` instead of
passing them through unchanged. Make peephole patterns robust to optimized AST shapes.

### Algorithm

1. **supercompile.rs** — Replace `Expr::Index { .. } | Expr::Slice { .. } => expr` with:
   Drive both operands. If collection is a known list literal AND index is a known
   integer, evaluate at compile time (1-based → 0-based). Otherwise, construct new
   Index node with driven operands.

2. **codegen/peephole.rs** — Audit swap/vec-fill/index-lowering patterns. Verify they
   match on AST structure (statement shapes), not variable names. Add case for
   `BinaryOp(Subtract, Literal(n), Literal(1))` → fold already handles this.

### Modified Files

- `optimize/supercompile.rs` — replace Index/Slice passthrough with driving
- `codegen/peephole.rs` — harden pattern matching (if needed after audit)

### TDD Steps

#### STEP 1: RED — Index driving tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL (Index/Slice currently passed through unchanged)

**Tests (10):**

1. `index_driven_first_element`
   Source: Build Seq [10, 20, 30]. `Let x be item 1 of items. Show x.`
   Verify: Supercompiler resolves `item 1 of items` to 10 at compile time.
   E2E: assert_exact_output → "10"

2. `index_driven_last_element`
   Source: Build Seq [10, 20, 30]. `Let x be item 3 of items.`
   Verify: Resolves to 30.
   E2E: assert_exact_output → "30"

3. `index_out_of_bounds_preserved`
   Source: Build Seq [10, 20, 30]. `Let x be item 5 of items.`
   Verify: Index 5 > length 3. Do NOT evaluate at compile time (would panic).
   Assert: Runtime indexing preserved so bounds check error occurs at runtime.

4. `index_after_push_tracks`
   Source: `Push 10 to items. Push 20 to items. Let x be item 2 of items.`
   Verify: Supercompiler tracks pushes and resolves index after mutations.
   E2E: assert_exact_output → "20"

5. `index_dynamic_collection_preserved`
   Source: Collection built from dynamic input. `Let x be item 1 of items.`
   Verify: Collection is dynamic → index NOT resolved at compile time.
   Assert: Generated code preserves runtime indexing.

6. `index_dynamic_index_preserved`
   Source: Static collection [10, 20, 30]. `Let n be parseInt(args()). Let x be item n of items.`
   Verify: Index n is dynamic → NOT resolved even though collection is known.
   Assert: Runtime indexing preserved.

7. `slice_driving_basic`
   Source: Known sequence, slice with known bounds.
   Verify: Slice of known sequence with known start/end resolves at compile time.

8. `swap_pattern_still_detected`
   Source: Classic swap pattern — `temp = items[1]; items[1] = items[3]; items[3] = temp.`
   Verify: After supercompiler drives through Index, the swap peephole STILL fires.
   Assert: Generated code contains `.swap(` or equivalent.
   E2E: assert_exact_output → "3\n2\n1"

9. `index_lowering_after_fold`
   Source: `Let x be item (2 + 1) of items.` where items = [10, 20, 30].
   Verify: Fold simplifies 2+1=3. Then index driving resolves item 3 → 30.
   E2E: assert_exact_output → "30"

10. `index_e2e_output`
    Source: Build Seq [100, 200, 300]. Show item 2.
    E2E: assert_exact_output → "200"

#### STEP 2: GREEN — Implement Index/Slice driving

**File:** `optimize/supercompile.rs`

Replace `Expr::Index { .. } | Expr::Slice { .. } => expr` with driving logic:
- Drive collection and index subexpressions
- If both are known (list literal + integer literal): evaluate at compile time
  with bounds check (out-of-bounds → preserve runtime indexing)
- Otherwise: construct new Index/Slice node with driven operands

Audit `codegen/peephole.rs` — verify patterns still fire on optimized AST.

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 10 index tests)

#### STEP 3: VERIFY — No regressions (especially peephole patterns)

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Peephole patterns (swap, vec-fill) still work.

---

## Sprint 3c — Residual Code Generation

### Overview

When the supercompiler encounters a mixed static/dynamic expression, it must construct
*new* AST nodes representing the simplified computation. Currently, it either evaluates
completely (all-static) or returns the original AST unchanged (any-dynamic). The missing
middle is residual code: a new AST tree with static parts folded in and dynamic parts
preserved.

### Algorithm

1. **Driving BinaryOp with residual emission:**
   Drive both operands. Both literals → fold. One literal + one dynamic → new BinaryOp
   with literal and driven dynamic. Both dynamic → new BinaryOp with both driven.

2. **Driving Call with BTA:**
   All literal args → CTFE. Mixed → Sprint 2 specialization. All dynamic → drive body,
   emit residual.

3. **Driving If/While with known conditions:**
   Literal-true → drive only then. Literal-false → drive only else / eliminate loop.
   Dynamic → drive condition and both branches, emit new If/While.

### Modified Files

- `optimize/supercompile.rs` — extend `drive_expr` and `drive_stmt` to construct new
  AST nodes instead of returning originals

### TDD Steps

#### STEP 1: RED — Residual expression tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL

**Tests (6):**

1. `residual_static_left`
   Source: `f(a, b): Return a * b.` Called with S(3), D.
   Verify: Residual for body is `3 * b` — new BinaryOp with literal left, dynamic right.
   Assert: Generated code contains `3 * n` or equivalent.

2. `residual_static_right`
   Source: `f(a, b): Return a * b.` Called with D, S(5).
   Verify: Residual is `a * 5` — new BinaryOp with dynamic left, literal right.
   Assert: Generated code contains `n * 5` or equivalent.

3. `residual_both_static`
   Source: `f(a, b): Return a * b.` Called with S(3), S(5).
   Verify: Both static → fold to S(15). No residual BinaryOp.
   Assert: Generated code contains `15`.

4. `residual_nested_binary`
   Source: `f(a, b): Return a * b + a.` Called with S(3), D.
   Verify: Residual is `3 * b + 3` — nested BinaryOp with substituted literals.
   Assert: Generated code contains `3 * n + 3` or equivalent.

5. `residual_if_true_eliminated`
   Source: `g(flag, x): If flag: Return x + 1. Otherwise: Return x - 1.`
   Called with S(true), D.
   Verify: flag=true → else-branch eliminated. Residual is just `Return x + 1`.
   Assert: Generated code has NO if/else, only `x + 1`.

6. `residual_if_false_eliminated`
   Source: Same function, called with S(false), D.
   Verify: flag=false → then-branch eliminated. Residual is `Return x - 1`.

#### STEP 2: GREEN — Implement residual construction

**File:** `optimize/supercompile.rs`

Extend `drive_expr()`:
- BinaryOp: construct new node when one operand is literal and other is dynamic
- If/While: prune dead branches when condition is literal

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 6 residual tests)

#### STEP 3: RED — Residual control flow and call tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL

**Tests (6):**

7. `residual_if_dynamic_preserved`
   Source: `h(x): If x > 0: Return x. Otherwise: Return 0 - x.` Called with D.
   Verify: All dynamic → If/Otherwise structure preserved in residual.
   E2E: input "5" → "5", input "-3" → "3"

8. `residual_while_false_eliminated`
   Source: `While false: Show "never".` (condition is literal false)
   Verify: Loop eliminated entirely. No While in residual.
   Assert: Generated code has no loop.

9. `residual_while_dynamic_preserved`
   Source: `While x > 0: Set x to x - 1.` where x=D.
   Verify: Dynamic condition → While preserved in residual. Body driven.

10. `residual_call_all_dynamic`
    Source: `f(a, b): Return a + b.` Called with D, D.
    Verify: All dynamic → call preserved. Body driven but both operands dynamic.
    Assert: Generated code contains `f(a, b)` or equivalent unspecialized call.

11. `residual_call_mixed`
    Source: `f(a, b): Return a + b.` Called with S(3), D.
    Verify: Mixed → specialization creates f_s0_3(b) with body `3 + b`.
    Assert: Generated code contains specialized call with one arg.

12. `residual_e2e_mixed_binary_op`
    Source: `f(3, n)` where f returns `a * b + a`. n from input.
    E2E: input "7" → "24" (3 * 7 + 3 = 24)

#### STEP 4: GREEN — Implement remaining residual patterns

**File:** `optimize/supercompile.rs`

Extend:
- While with literal-false condition → eliminate
- Call integration with Sprint 2 specialization for mixed args
- Call with all-dynamic args → preserve call, drive body

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 12 residual tests)

#### STEP 5: VERIFY — No regressions

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

---

## Sprint 3e — Identity / Perfect Residuals

### Overview

A partial evaluator satisfies the *identity property* when specializing an interpreter
with respect to a program where ALL inputs are dynamic produces a residual that is
semantically identical to the original program. Formally:
`pe(int, P_all_dynamic) ≡ P`. If this fails, the PE is introducing overhead (extra
let-bindings, redundant inspects, identity assignments) or losing information (dropping
branches, mangling variable names).

This sub-sprint adds a `cleanup_identities` post-processing pass and 4 tests that verify
the identity property across different program shapes.

**Citations:**
- Jones, Gomard & Sestoft (1993), Section 4.6: "The identity property".
- Glück & Jørgensen, "Efficient Multi-Level Generating Extensions" (1995).

### Algorithm

`cleanup_identities(stmts) -> stmts`:
1. Remove identity let-bindings: `Let x = x.` → delete
2. Collapse single-arm inspects: `Inspect v: When VInt(n): <body>` with no other arms
   and v is known to be VInt → inline body
3. Remove no-op env lookups: `Let x = item "x" of env.` where x is already bound → delete
4. Collapse `Let x = <literal>. Return x.` → `Return <literal>.`
5. Iterate until fixpoint (max 4 passes)

### New Function

Add `cleanup_identities()` to `optimize/partial_eval.rs` (or a dedicated helper module).

### TDD Steps

#### STEP 1: RED — Identity tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile identity_ -- --skip e2e`
**Expected:** FAIL

**Tests (4):**

1. `identity_trivial_program`
   Source: `CProgram([], [CShow(CInt(42))])` — trivial show, all dynamic.
   Harness: residual = pe(int, P_all_dynamic). Run cleanup_identities.
   Verify: Residual is structurally equivalent to `Show 42.` — no extra bindings, no
   inspect dispatch, no env lookups.
   E2E: assert_exact_output → "42"

2. `identity_arithmetic_program`
   Source: `CProgram([], [CLet("x", CBinOp("+", CVar("a"), CVar("b"))), CShow(CVar("x"))])` — all dynamic.
   Harness: residual = pe(int, P_all_dynamic). Run cleanup_identities.
   Verify: Residual is `Let x = a + b. Show x.` — arithmetic preserved, no overhead.
   E2E: With a=3, b=4 → "7"

3. `identity_control_flow_program`
   Source: `CProgram([], [CIf(CBinOp(">", CVar("x"), CInt(0)), [CShow(CText("pos"))], [CShow(CText("neg"))])])` — all dynamic.
   Harness: residual = pe(int, P_all_dynamic). Run cleanup_identities.
   Verify: If/else structure preserved. No dead branch elimination (condition is dynamic).
   E2E: With x=5 → "pos", with x=-1 → "neg"

4. `identity_function_call_program`
   Source: `CProgram([CFunc("double", ["n"], [CReturn(CBinOp("*", CVar("n"), CInt(2)))])], [CShow(CCall("double", [CVar("x")]))])` — all dynamic.
   Harness: residual = pe(int, P_all_dynamic). Run cleanup_identities.
   Verify: Function call preserved. No inlining (dynamic arg). Residual calls `double(x)`.
   E2E: With x=21 → "42"

#### STEP 2: GREEN — Implement cleanup_identities

**File:** `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

Implement `cleanup_identities()` as described in the Algorithm section. Integrate as
a post-processing step after PE residual generation.

**Run:** `cargo test --test phase_supercompile identity_ -- --skip e2e`
**Expected:** PASS (all 4 identity tests)

#### STEP 3: VERIFY — No regressions

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 3e

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 4 identity tests pass.
**Hard stop:** Identity property verified before proceeding.

---

## Sprint 3d — Homeomorphic Embedding and Generalization

### Overview

Replace the crude "remove all modified variables from the store" (Pitfall 5) with a
proper termination criterion based on Kruskal's tree theorem. The supercompiler maintains
a *configuration history*. Before each driving step, it checks whether the current
configuration is *homeomorphically embedded* in any previous one. If so, the state space
is growing — generalization is applied.

**Unified Termination Mechanism:** This embedding + MSG framework is shared between
Sprint 3d (supercompilation) and Sprint 2 (function specialization). Sprint 2's
`SpecRegistry` uses `history: Vec<SpecKey>` instead of an ad-hoc depth counter —
before each specialization step, the current spec key is checked against the history
via `embeds()`. If the current configuration embeds in a predecessor, MSG generalizes
and specialization emits a residual instead of recursing further. Both passes share
the same `embeds()` and `generalize()` functions. The only difference is the
configuration type: Sprint 2 operates on `SpecKey` (function + static args), while
Sprint 3d operates on `Configuration` (program point + store). Kruskal's tree theorem
guarantees termination for both.

**Citations:**
- Kruskal, J.B. "Well-quasi-ordering, the Tree Theorem" (1960).
- Turchin, V.F. "The Concept of a Supercompiler" (1986). ACM TOPLAS 8(3):292-325.
- Sørensen & Glück, "An Algorithm of Generalization in Positive Supercompilation" (1995).

### Data Structures

```rust
struct Configuration<'a> {
    program_point: &'a Stmt<'a>,
    store: HashMap<Symbol, Value>,
}

struct History<'a> {
    stack: Vec<Configuration<'a>>,
}
```

### Algorithm

**Homeomorphic embedding** `e1 ◁ e2`:
- `x ◁ x` (variables embed in themselves)
- `e1 ◁ f(t1,...,tn)` if `e1 ◁ ti` for some i (diving)
- `f(s1,...,sn) ◁ f(t1,...,tn)` if `si ◁ ti` for all i (coupling)

**Most Specific Generalization (MSG):** Given `e1` and `e2`, find most specific `g` such
that `e1 = g[s/x]` and `e2 = g[t/x]`. The MSG replaces differing subexpressions with
fresh variables.

**Procedure:**
1. Before each driving step: push configuration onto history.
2. For each previous config: if same program point AND previous store embeds in current
   store → generalize.
3. Generalization: compute MSG of current and predecessor store. Replace differing
   concrete values with unknowns.
4. Safety net: hard depth limit of 64. Emit residual if reached.

```rust
fn embeds(e1: &Expr, e2: &Expr) -> bool {
    match (e1, e2) {
        // Coupling: same constructor, all children embed
        (Expr::BinaryOp { op: op1, left: l1, right: r1 },
         Expr::BinaryOp { op: op2, left: l2, right: r2 }) if op1 == op2 =>
            embeds(l1, l2) && embeds(r1, r2),
        (Expr::Call { function: f1, args: a1 },
         Expr::Call { function: f2, args: a2 }) if f1 == f2 && a1.len() == a2.len() =>
            a1.iter().zip(a2.iter()).all(|(x, y)| embeds(x, y)),
        (Expr::Not { expr: e1 }, Expr::Not { expr: e2 }) =>
            embeds(e1, e2),
        // Base: literals and identifiers embed in themselves
        (Expr::Literal(l1), Expr::Literal(l2)) => l1 == l2,
        (Expr::Identifier(s1), Expr::Identifier(s2)) => s1 == s2,
        // Diving: e1 embeds in a subterm of e2
        (_, Expr::BinaryOp { left, right, .. }) =>
            embeds(e1, left) || embeds(e1, right),
        (_, Expr::Call { args, .. }) =>
            args.iter().any(|a| embeds(e1, a)),
        (_, Expr::Not { expr }) =>
            embeds(e1, expr),
        _ => false,
    }
}
```

### Modified Files

- `optimize/supercompile.rs` — add Configuration, History, `embeds()`, `generalize()`,
  integrate into `drive_stmt` loop

### TDD Steps

#### STEP 1: RED — Embedding detection tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL

**Tests (5):**

1. `embedding_self_check`
   Verify: `embeds(x, x)` returns true — a variable embeds in itself.
   Assert: Unit test on `embeds()` function directly.

2. `embedding_diving_check`
   Verify: `embeds(x, f(x, y))` returns true — x embeds in a subterm of f(x,y).
   Assert: Diving rule works for BinaryOp, Call, Not.

3. `embedding_coupling_check`
   Verify: `embeds(f(a, b), f(a+1, b+1))` returns true if a ◁ a+1 and b ◁ b+1.
   Assert: Coupling rule works for same-constructor matching.

4. `embedding_rejects_non_embedded`
   Verify: `embeds(f(a, b), g(c))` returns false — different constructors, no diving match.
   Assert: Non-embedded expressions correctly rejected.

5. `embedding_growing_detected`
   Source: `grow(n): Return grow(n + 1).` Called with D.
   Verify: Supercompiler detects growing configuration (n → n+1 → n+1+1...).
   Embedding triggers before depth 64.
   Assert: Compilation terminates (no infinite loop/stack overflow).

#### STEP 2: GREEN — Implement embeds() and configuration history

**File:** `optimize/supercompile.rs`

Implement:
- `embeds(e1, e2) -> bool` — the homeomorphic embedding check
- `Configuration` struct and `History` stack
- Push configuration before each driving step
- Check embedding against all previous configurations

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 5 embedding tests)

#### STEP 3: RED — MSG and generalization tests

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL

**Tests (4):**

6. `msg_computation`
   Verify: MSG of `a + b` and `a + c` is `a + ?1` where ?1 is fresh.
   Assert: MSG preserves common structure (the `+` and `a`), replaces differing parts.

7. `msg_precision`
   Verify: MSG of `f(3, x)` and `f(3, y)` is `f(3, ?1)` — keeps the common `3`.
   Assert: MSG is maximally precise (doesn't generalize more than needed).

8. `msg_replacement`
   Verify: After MSG, the generalized values become unknown (removed from store).
   Assert: Store no longer contains concrete values for generalized variables.

9. `embedding_depth_limit_64`
   Source: Deeply recursive function that would drive to depth > 64.
   Verify: Hard depth limit fires, residual emitted at current state.
   Assert: Compilation terminates within bounded time.

#### STEP 4: GREEN — Implement MSG and generalization

**File:** `optimize/supercompile.rs`

Implement:
- `generalize(store1, store2) -> HashMap<Symbol, Value>` — MSG computation
- When embedding detected: compute MSG, replace store with generalized version
- Depth limit 64 safety net
- Replace crude `collect_modified_vars_block` + `remove` with embedding-based approach

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 9 embedding/MSG tests)

#### STEP 5: RED — E2E correctness with embedding

**File:** `crates/logicaffeine_tests/tests/phase_supercompile.rs` (append)

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** FAIL (or PASS if implementation is already correct)

**Tests (3):**

10. `embedding_tail_recursive_e2e`
    Source: `f(n, acc): If n <= 0: Return acc. Return f(n-1, acc+n).`
    `Let y be f(n, 0).` where n from input.
    Verify: Driving encounters growing config (acc increases each step). Generalization
    introduces loop or preserves tail recursion. Output correct.
    E2E: input "10" → "55"

11. `embedding_while_loop_e2e`
    Source: `sumLoop(n): Let sum=0, i=1. While i <= n: sum += i; i += 1. Return sum.`
    n from input.
    Verify: While loop preserved (dynamic bound). Body driven. Output correct.
    E2E: input "100" → "5050"

12. `generalization_preserves_correctness`
    Source: `f(n, acc)` with complex body that triggers generalization.
    Verify: After generalization, program still produces correct output.
    E2E: Multiple inputs all produce correct results.

#### STEP 6: GREEN — Fix any E2E failures

**File:** `optimize/supercompile.rs`

Verify generalized code produces correct residual. Fix any issues.

**Run:** `cargo test --test phase_supercompile -- --skip e2e`
**Expected:** PASS (all 12 embedding/generalization tests)

#### STEP 7: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 3 (all sub-sprints)

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All Sprint 3 tests pass (12 text + 10 index + 12 residual + 12 embedding + 4 identity = 50 tests). All Sprint 1-2 tests still pass. No regressions.
**Hard stop:** Do NOT proceed to Sprint 4 until this gate is green.

---

## Sprint 4 — LOGOS-in-LOGOS Self-Interpreter

### Overview

For the Futamura projections to work literally, we need an interpreter written in
LogicAffeine that takes a program representation (as data) and executes it. This
interpreter is the `int` in `pe(int, program) = compiled_program`.

The self-interpreter must:
1. Be written entirely in LogicAffeine (so the PE can process it)
2. Accept programs encoded as LogicAffeine data structures (programs-as-data)
3. Handle the full semantic core: arithmetic, control flow, functions, sequences, maps, and error propagation (division by zero, modulo by zero, index out of bounds)

**Citations:**
- Futamura, Y. (1971). Defines `int` such that `int(program, input) = output`.
- Jones, N.D. (1996). Section 3: the self-interpreter requirement for Projections 2-3.

### Data Types (LogicAffeine Recursive Enums)

```
## A CExpr is one of:
    A CInt with value Int.
    A CBool with value Bool.
    A CText with value Text.
    A CVar with name Text.
    A CBinOp with op Text and left CExpr and right CExpr.
    A CNot with inner CExpr.
    A CCall with name Text and args Seq of CExpr.
    A CIndex with collection CExpr and index CExpr.
    A CLen with collection CExpr.
    A CMapGet with map CExpr and key CExpr.

## A CStmt is one of:
    A CLet with name Text and expr CExpr.
    A CSet with name Text and expr CExpr.
    A CIf with cond CExpr and thenBlock Seq of CStmt and elseBlock Seq of CStmt.
    A CWhile with cond CExpr and body Seq of CStmt.
    A CReturn with expr CExpr.
    A CShow with expr CExpr.
    A CCallS with name Text and args Seq of CExpr.
    A CPush with expr CExpr and target Text.
    A CSetIdx with target Text and index CExpr and value CExpr.
    A CMapSetS with target Text and key CExpr and value CExpr.

## A CFunc is one of:
    A CFunc with name Text and params Seq of Text and body Seq of CStmt.

## A CProgram is one of:
    A CProgram with funcs Seq of CFunc and main Seq of CStmt.

## A CVal is one of:
    A VInt with value Int.
    A VBool with value Bool.
    A VText with value Text.
    A VSeq with items Seq of CVal.
    A VMap with data Map of Text and CVal.
    A VError with msg Text.
    A VNothing.
```

> **VError semantics:** `VError` propagates through all operations — any binary op,
> index, or function call receiving a VError operand returns VError unchanged. This
> gives the self-interpreter the same error behavior as the host language: division by
> zero, modulo by zero, and out-of-bounds indexing produce error values rather than
> crashing the interpreter. `valToText(VError(msg))` produces `"Error: " + msg` for
> display via CShow.

```
```

> **Note:** `CShow` intentionally omits `recipient` — the self-interpreter always prints to stdout via the default `show` function. The `encode_program()` helper discards `Stmt::Show.recipient`.

### The Interpreter (LogicAffeine)

```
## To coreEval (expr: CExpr, env: Map of Text and CVal, funcs: Map of Text and CFunc) -> CVal:
    Inspect expr:
        When CInt(n):
            Return a new VInt with value n.
        When CBool(b):
            Return a new VBool with value b.
        When CText(s):
            Return a new VText with value s.
        When CVar(name):
            Return item name of env.
        When CBinOp(op, left, right):
            Let lv be coreEval(left, env, funcs).
            Let rv be coreEval(right, env, funcs).
            Return applyBinOp(op, lv, rv).
        When CNot(inner):
            Let v be coreEval(inner, env, funcs).
            Inspect v:
                When VBool(b):
                    Return a new VBool with value (not b).
                Otherwise:
                    Return a new VNothing.
        When CCall(name, argExprs):
            Let argVals be a new Seq of CVal.
            Repeat for a in argExprs:
                Push coreEval(a, env, funcs) to argVals.
            Let func be item name of funcs.
            Inspect func:
                When CFunc(fname, params, body):
                    Let callEnv be a new Map of Text and CVal.
                    Let mutable idx be 1.
                    Repeat for p in params:
                        Set item p of callEnv to item idx of argVals.
                        Set idx to idx + 1.
                    Return coreExecBlock(body, callEnv, funcs).
                Otherwise:
                    Return a new VNothing.
        When CIndex(collExpr, idxExpr):
            Let coll be coreEval(collExpr, env, funcs).
            Let idx be coreEval(idxExpr, env, funcs).
            Inspect coll:
                When VError(msg):
                    Return a new VError with msg msg.
                When VSeq(items):
                    Inspect idx:
                        When VError(msg):
                            Return a new VError with msg msg.
                        When VInt(i):
                            If i is less than 1 or i is greater than (length of items):
                                Return a new VError with msg "index out of bounds".
                            Return item i of items.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CLen(collExpr):
            Let coll be coreEval(collExpr, env, funcs).
            Inspect coll:
                When VSeq(items):
                    Return a new VInt with value (length of items).
                Otherwise:
                    Return a new VNothing.
        When CMapGet(mapExpr, keyExpr):
            Let m be coreEval(mapExpr, env, funcs).
            Let k be coreEval(keyExpr, env, funcs).
            Inspect m:
                When VMap(mapData):
                    Inspect k:
                        When VText(key):
                            Return item key of mapData.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.

## To coreExecBlock (stmts: Seq of CStmt, env: Map of Text and CVal, funcs: Map of Text and CFunc) -> CVal:
    Repeat for s in stmts:
        Let result be coreExecStmt(s, env, funcs).
        If not isNothing(result):
            Return result.
    Return a new VNothing.

## To coreExecStmt (stmt: CStmt, env: Map of Text and CVal, funcs: Map of Text and CFunc) -> CVal:
    Inspect stmt:
        When CLet(name, expr):
            Set item name of env to coreEval(expr, env, funcs).
            Return a new VNothing.
        When CSet(name, expr):
            Set item name of env to coreEval(expr, env, funcs).
            Return a new VNothing.
        When CIf(cond, thenBlock, elseBlock):
            Let cv be coreEval(cond, env, funcs).
            Inspect cv:
                When VBool(b):
                    If b:
                        Return coreExecBlock(thenBlock, env, funcs).
                    Otherwise:
                        Return coreExecBlock(elseBlock, env, funcs).
                Otherwise:
                    Return a new VNothing.
        When CWhile(cond, body):
            While true:
                Let cv be coreEval(cond, env, funcs).
                Inspect cv:
                    When VBool(b):
                        If not b:
                            Return a new VNothing.
                    Otherwise:
                        Return a new VNothing.
                Let result be coreExecBlock(body, env, funcs).
                If not isNothing(result):
                    Return result.
            Return a new VNothing.
        When CReturn(expr):
            Return coreEval(expr, env, funcs).
        When CShow(expr):
            Let v be coreEval(expr, env, funcs).
            Show valToText(v).
            Return a new VNothing.
        When CCallS(name, argExprs):
            Let argVals be a new Seq of CVal.
            Repeat for a in argExprs:
                Push coreEval(a, env, funcs) to argVals.
            Let func be item name of funcs.
            Inspect func:
                When CFunc(fname, params, body):
                    Let callEnv be a new Map of Text and CVal.
                    Let mutable idx be 1.
                    Repeat for p in params:
                        Set item p of callEnv to item idx of argVals.
                        Set idx to idx + 1.
                    Let result be coreExecBlock(body, callEnv, funcs).
                    Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
        When CPush(valExpr, collName):
            Let v be coreEval(valExpr, env, funcs).
            Let coll be item collName of env.
            Inspect coll:
                When VSeq(items):
                    Push v to items.
                    Set item collName of env to a new VSeq with items items.
                Otherwise:
                    Return a new VNothing.
            Return a new VNothing.
        When CSetIdx(collName, idxExpr, valExpr):
            Let idx be coreEval(idxExpr, env, funcs).
            Let v be coreEval(valExpr, env, funcs).
            Let coll be item collName of env.
            Inspect coll:
                When VSeq(items):
                    Inspect idx:
                        When VInt(i):
                            Set item i of items to v.
                            Set item collName of env to a new VSeq with items items.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
            Return a new VNothing.
        When CMapSetS(mapName, keyExpr, valExpr):
            Let k be coreEval(keyExpr, env, funcs).
            Let v be coreEval(valExpr, env, funcs).
            Let m be item mapName of env.
            Inspect m:
                When VMap(mapData):
                    Inspect k:
                        When VText(key):
                            Set item key of mapData to v.
                            Set item mapName of env to a new VMap with data mapData.
                        Otherwise:
                            Return a new VNothing.
                Otherwise:
                    Return a new VNothing.
            Return a new VNothing.

## To valToText (v: CVal) -> Text:
    Inspect v:
        When VInt(n):
            Return "{n}".
        When VBool(b):
            If b:
                Return "true".
            Otherwise:
                Return "false".
        When VText(s):
            Return s.
        When VSeq(items):
            Return "[seq]".
        When VMap(m):
            Return "[map]".
        When VError(msg):
            Return "Error: " + msg.
        When VNothing:
            Return "nothing".

## To isNothing (v: CVal) -> Bool:
    Inspect v:
        When VNothing:
            Return true.
        Otherwise:
            Return false.

## To applyBinOp (op: Text, lv: CVal, rv: CVal) -> CVal:
    Inspect lv:
        When VError(msg):
            Return a new VError with msg msg.
        Otherwise:
            Inspect rv:
                When VError(msg):
                    Return a new VError with msg msg.
                Otherwise:
                    Let pass be true.
    Inspect lv:
        When VInt(a):
            Inspect rv:
                When VInt(b):
                    If op equals "+":
                        Return a new VInt with value (a + b).
                    If op equals "-":
                        Return a new VInt with value (a - b).
                    If op equals "*":
                        Return a new VInt with value (a * b).
                    If op equals "/":
                        If b equals 0:
                            Return a new VError with msg "division by zero".
                        Return a new VInt with value (a / b).
                    If op equals "%":
                        If b equals 0:
                            Return a new VError with msg "modulo by zero".
                        Return a new VInt with value (a % b).
                    If op equals "<":
                        Return a new VBool with value (a is less than b).
                    If op equals ">":
                        Return a new VBool with value (a is greater than b).
                    If op equals "<=":
                        Return a new VBool with value (a is at most b).
                    If op equals ">=":
                        Return a new VBool with value (a is at least b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                Otherwise:
                    Return a new VNothing.
        When VBool(a):
            Inspect rv:
                When VBool(b):
                    If op equals "&&":
                        Return a new VBool with value (a and b).
                    If op equals "||":
                        Return a new VBool with value (a or b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                Otherwise:
                    Return a new VNothing.
        When VText(a):
            Inspect rv:
                When VText(b):
                    If op equals "+":
                        Return a new VText with value (a + b).
                    If op equals "==":
                        Return a new VBool with value (a equals b).
                    If op equals "!=":
                        Return a new VBool with value (a is not b).
                Otherwise:
                    Return a new VNothing.
        Otherwise:
            Return a new VNothing.
```

### The `encode_program()` Rust Helper

A Rust function that traverses a `Vec<Stmt>` AST and produces the equivalent CProgram
as a LogicAffeine expression tree.

**Mapping table:**

| AST Node | CExpr/CStmt Encoding |
|----------|---------------------|
| `Expr::Literal(Number(n))` | `CInt(n)` |
| `Expr::Literal(Boolean(b))` | `CBool(b)` |
| `Expr::Literal(Text(s))` | `CText(s)` |
| `Expr::Identifier(sym)` | `CVar(name_str)` |
| `Expr::BinaryOp { op, left, right }` | `CBinOp(op_str, encode(left), encode(right))` |
| `Expr::Not { operand }` | `CNot(encode(operand))` |
| `Expr::Call { function, args }` | `CCall(name_str, [encode(a) for a in args])` |
| `Stmt::Let { var, ty, value, mutable }` | `CLet(var_str, encode(value))` |
| `Stmt::Set { target, value }` | `CSet(name_str, encode(value))` |
| `Stmt::If { cond, then_block, else_block }` | `CIf(encode(cond), encode_block(then_block), encode_block(else_block))` |
| `Stmt::While { cond, body, decreasing }` | `CWhile(encode(cond), encode_block(body))` — `decreasing` ignored (self-interpreter has no termination proofs) |
| `Stmt::Return { value }` | `CReturn(encode(value))` |
| `Stmt::Show { object, recipient }` | `CShow(encode(object))` |
| `Stmt::Push { value, collection }` | `CPush(encode(value), collection_name_str)` |

### New File

`crates/logicaffeine_tests/tests/phase_futamura.rs`

### Modified Files

- `crates/logicaffeine_compile/src/compile.rs` — add `pub fn encode_program()` helper

### TDD Steps

#### STEP 1: RED — Test harness helper

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs`

Write a test that calls `run_interpreter_program()` — a helper that does not exist yet.
This helper takes a LOGOS source string containing Core types + interpreter + a program,
compiles and runs it, and returns the output.

**Tests (1):**

1. `core_eval_literal_int`
   Source: Core type definitions + interpreter functions + Main that constructs
   `CProgram([], [CShow(CInt(42))])` and runs the interpreter on it.
   Verify: The interpreter evaluates CShow(CInt(42)) and outputs "42".
   E2E: assert_exact_output → "42"

**Run:** `cargo test --test phase_futamura core_eval_literal_int -- --skip e2e`
**Expected:** FAIL (test file doesn't exist / helper doesn't exist)

#### STEP 2: GREEN — Create test file and harness

**File:** Create `crates/logicaffeine_tests/tests/phase_futamura.rs`

The harness helper `run_interpreter_program()`:
- Takes the Core type definitions as a string constant (Define CExpr..., CStmt..., etc.)
- Takes the interpreter source as a string constant (coreEval, coreExecBlock, etc.)
- Takes a Main block that constructs a CProgram and calls the interpreter
- Concatenates them and passes to `assert_exact_output()`

Alternatively, each test is a self-contained LOGOS program with all type definitions
and interpreter functions included. The helper just wraps `assert_exact_output` with
the common preamble.

**Run:** `cargo test --test phase_futamura core_eval_literal_int -- --skip e2e`
**Expected:** PASS

#### STEP 3: RED — Literal and variable evaluation

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** FAIL for new tests (depends on interpreter handling all cases)

**Tests (4):**

2. `core_eval_literal_bool`
   Source: CProgram with `[CShow(CBool(true))]`
   Verify: Interpreter outputs "true".
   E2E: assert_exact_output → "true"

3. `core_eval_literal_text`
   Source: CProgram with `[CShow(CText("hello"))]`
   Verify: Interpreter outputs "hello".
   E2E: assert_exact_output → "hello"

4. `core_eval_literal_nothing`
   Source: CProgram with `[CShow(CVar("_missing_"))]` (variable not in env)
   Verify: Returns VNothing → interpreter handles gracefully.

5. `core_eval_variable`
   Source: CProgram with `[CLet("x", CInt(10)), CShow(CVar("x"))]`
   Verify: CLet binds x=10 in env. CVar("x") looks it up. Output is "10".
   E2E: assert_exact_output → "10"

#### STEP 4: GREEN — Fix interpreter for literal/variable cases

Ensure all CExpr literal variants and CVar work. Fix any issues.

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** PASS (all 5 tests)

#### STEP 5: RED — Arithmetic and boolean operations

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (5):**

6. `core_eval_addition`
   Source: `[CShow(CBinOp("+", CInt(2), CInt(3)))]`
   Verify: applyBinOp("+", VInt(2), VInt(3)) → VInt(5).
   E2E: assert_exact_output → "5"

7. `core_eval_subtraction`
   Source: `[CShow(CBinOp("-", CInt(10), CInt(3)))]`
   E2E: assert_exact_output → "7"

8. `core_eval_multiplication`
   Source: `[CShow(CBinOp("*", CInt(4), CInt(5)))]`
   E2E: assert_exact_output → "20"

9. `core_eval_nested_arithmetic`
   Source: `[CShow(CBinOp("*", CBinOp("+", CInt(2), CInt(3)), CInt(4)))]`
   Verify: (2+3)*4 = 20. Nested expression evaluation.
   E2E: assert_exact_output → "20"

10. `core_eval_comparison_operators`
    Source: Test each comparison: <, >, <=, >=, ==, !=.
    `[CShow(CBinOp("<", CInt(3), CInt(5)))]` → "true"
    Verify: All 6 comparison operators produce correct VBool results.

#### STEP 6: GREEN — Fix arithmetic and comparisons

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** PASS (all 10 tests)

#### STEP 7: RED — Boolean logic and control flow

**Tests (6):**

11. `core_eval_boolean_and`
    Source: `[CShow(CBinOp("&&", CBool(true), CBool(false)))]`
    E2E: assert_exact_output → "false"

12. `core_eval_boolean_or`
    Source: `[CShow(CBinOp("||", CBool(false), CBool(true)))]`
    E2E: assert_exact_output → "true"

13. `core_eval_if_true`
    Source: `[CIf(CBool(true), [CShow(CInt(1))], [CShow(CInt(2))])]`
    Verify: true condition → then-block executes.
    E2E: assert_exact_output → "1"

14. `core_eval_if_false`
    Source: `[CIf(CBool(false), [CShow(CInt(1))], [CShow(CInt(2))])]`
    Verify: false condition → else-block executes.
    E2E: assert_exact_output → "2"

15. `core_eval_nested_if`
    Source: `CIf(CBool(true), [CIf(CBool(false), [CShow(CInt(1))], [CShow(CInt(2))])], [...])`
    Verify: Nested conditions evaluate correctly. Outer true, inner false → inner else.
    E2E: assert_exact_output → "2"

16. `core_eval_while_loop`
    Source: `[CLet("sum", CInt(0)), CLet("i", CInt(1)),
    CWhile(CBinOp("<=", CVar("i"), CInt(5)), [
        CSet("sum", CBinOp("+", CVar("sum"), CVar("i"))),
        CSet("i", CBinOp("+", CVar("i"), CInt(1)))
    ]),
    CShow(CVar("sum"))]`
    Verify: Sum of 1..5 = 15. While loop with mutable env threading.
    E2E: assert_exact_output → "15"

#### STEP 8: GREEN — Fix control flow

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** PASS (all 16 tests)

#### STEP 9: RED — Function calls and recursion

**Tests (5):**

17. `core_eval_function_call`
    Source: funcs = `[CFunc("double", ["x"], [CReturn(CBinOp("*", CVar("x"), CInt(2)))])]`
    main = `[CShow(CCall("double", [CInt(21)]))]`
    Verify: double(21) = 42. Function call with env isolation.
    E2E: assert_exact_output → "42"

18. `core_eval_recursive_factorial`
    Source: funcs = `[CFunc("factorial", ["n"], [
        CIf(CBinOp("<=", CVar("n"), CInt(1)),
            [CReturn(CInt(1))],
            [CReturn(CBinOp("*", CVar("n"),
                CCall("factorial", [CBinOp("-", CVar("n"), CInt(1))])))])
    ])]`
    main = `[CShow(CCall("factorial", [CInt(5)]))]`
    Verify: factorial(5) = 120. Recursive function call.
    E2E: assert_exact_output → "120"

19. `core_eval_recursive_fibonacci`
    Source: fib function with base cases 0,1 and recursive fib(n-1)+fib(n-2).
    main = `[CShow(CCall("fib", [CInt(10)]))]`
    E2E: assert_exact_output → "55"

20. `core_eval_mutual_recursion`
    Source: `isEven(n)` calls `isOdd(n-1)`, `isOdd(n)` calls `isEven(n-1)`.
    Base: isEven(0)=true, isOdd(0)=false.
    main = `[CShow(CCall("isEven", [CInt(4)]))]`
    Verify: Mutual recursion works. isEven(4) = true.
    E2E: assert_exact_output → "true"

21. `core_eval_missing_function`
    Source: main = `[CShow(CCall("nonexistent", [CInt(1)]))]`
    Verify: Unknown function → VNothing (via Otherwise arm). No crash.
    E2E: assert_exact_output → "nothing"

#### STEP 10: GREEN — Fix function calls and recursion

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** PASS (all 21 tests)

#### STEP 11: RED — Collections and edge cases

**Tests (9):**

22. `core_eval_push_and_index`
    Source: `[CLet("items", CCall("newSeq", [])), CPush(CInt(10), "items"),
    CPush(CInt(20), "items"), CShow(CIndex(CVar("items"), CInt(2)))]`
    Verify: Push 10, push 20. Index 2 (1-based) = 20.
    E2E: assert_exact_output → "20"

23. `core_eval_push_multiple`
    Source: Push 10, 20, 30 to a seq. Show all three via index.
    Verify: Multiple pushes tracked correctly.
    E2E: assert_exact_output → "10\n20\n30"

24. `core_eval_set_index`
    Source: Build seq [10, 20]. CSetIdx("items", CInt(1), CInt(99)). Show item 1.
    Verify: Index mutation works. item 1 is now 99.
    E2E: assert_exact_output → "99"

25. `core_eval_sequence_length`
    Source: Build seq [10, 20, 30]. `CShow(CLen(CVar("items")))`.
    Verify: Length = 3.
    E2E: assert_exact_output → "3"

26. `core_eval_map_operations`
    Source: `CLet("m", CCall("newMap", [])), CMapSetS("m", CText("key"), CInt(42)),
    CShow(CMapGet(CVar("m"), CText("key")))`.
    Verify: Map set/get works. get("key") = 42.
    E2E: assert_exact_output → "42"

27. `core_eval_scoping_isolation`
    Source: Caller has variable "x"=100. Callee also uses "x" as parameter.
    Verify: callEnv is fresh — callee's x comes from args, not caller's env.
    E2E: assert_exact_output shows callee's value, not caller's.

28. `core_eval_early_return_in_while`
    Source: While loop with `CReturn(CInt(42))` inside body.
    Verify: Return inside while terminates the loop and returns.
    E2E: assert_exact_output → "42"

29. `core_eval_counter_loop`
    Source: `i=0. While i < 5: i = i + 1. Show i.`
    Verify: Counter increments correctly through while loop.
    E2E: assert_exact_output → "5"

30. `core_eval_string_concat`
    Source: `[CLet("a", CText("Hello")), CLet("b", CText(", World!")),
    CShow(CBinOp("+", CVar("a"), CVar("b")))]`
    Verify: Text concatenation works in interpreter.
    E2E: assert_exact_output → "Hello, World!"

31. `core_eval_div_by_zero`
    Source: `[CShow(CBinOp("/", CInt(10), CInt(0)))]`
    Verify: Division by zero returns VError("division by zero"), not a crash.
    E2E: assert_exact_output → "Error: division by zero"

32. `core_eval_mod_by_zero`
    Source: `[CShow(CBinOp("%", CInt(10), CInt(0)))]`
    Verify: Modulo by zero returns VError("modulo by zero").
    E2E: assert_exact_output → "Error: modulo by zero"

33. `core_eval_index_out_of_bounds`
    Source: Build seq [10, 20, 30]. `CShow(CIndex(CVar("items"), CInt(10)))`.
    Verify: Index 10 out of bounds for 3-element seq → VError("index out of bounds").
    E2E: assert_exact_output → "Error: index out of bounds"

34. `core_eval_error_propagation_binop`
    Source: `[CLet("err", CBinOp("/", CInt(1), CInt(0))),
    CShow(CBinOp("+", CVar("err"), CInt(5)))]`
    Verify: VError propagates through binary operations. VError + 5 → VError.
    E2E: assert_exact_output → "Error: division by zero"

35. `core_eval_error_in_show`
    Source: `[CShow(CBinOp("%", CInt(7), CInt(0)))]`
    Verify: CShow on a VError displays the error message with "Error: " prefix.
    E2E: assert_exact_output → "Error: modulo by zero"

#### STEP 12: GREEN — Fix collections and edge cases

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** PASS (all 35 tests)

#### STEP 13: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 4

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 35 self-interpreter tests pass (including error propagation). All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 5 until this gate is green.

---

## Sprint 5 — Projection 1: `pe(int, program) = compiled_program`

### Overview

The First Futamura Projection: specializing an interpreter with respect to a fixed
program produces a compiled version of that program. The compiled version executes
directly — no dispatch on statement types, no environment lookups by name for
statically known variables.

**Citations:**
- Futamura, Y. (1971). `[[pe]]([[int]], [[program]]) = [[target]]`
- Jones, N.D. (1996). Section 4: "The first projection is the simplest and most
  practically useful."

**What "no interpretive overhead" means structurally:**
1. No `Inspect` on CStmt variants — all statement dispatch is resolved
2. No `Inspect` on CExpr variants — all expression dispatch is resolved
3. No `item <literal_string> of env` for compile-time-known names
4. No reference to CExpr/CStmt/CFunc/CProgram types in the residual
5. Residual contains ONLY: direct computation, control flow, IO, variable bindings

### Algorithm

1. Construct interpreter as LogicAffeine AST (parse Sprint 4's source)
2. Construct source program as static data via `encode_program()`
3. Run PE: `specialize(interpreter_ast, { program: Static(encoded), input: Dynamic })`
4. Post-process: fold → dce on residual
5. Verify: `verify_no_overhead()` on residual
6. Execute: compile and run residual, compare output against direct interpretation

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — add Projection 1 tests
- `crates/logicaffeine_compile/src/compile.rs` — implement `encode_program()`

### TDD Steps

#### STEP 1: RED — Harness helpers for Projection 1

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

Write tests that call `verify_no_overhead()` and `encode_program()` — neither exists yet.

**Tests (2):**

1. `p1_encode_roundtrip`
   Source: A simple program `Show 42.`
   Verify: `encode_program()` converts AST → CProgram expression tree.
   The encoded CProgram, when run through the Core interpreter, produces "42".
   Assert: Encoding is semantics-preserving.

2. `p1_verifier_catches_violations`
   Source: A hand-constructed residual that still contains `Inspect` on CStmt.
   Verify: `verify_no_overhead()` returns Err with descriptive message.
   Assert: Verifier correctly rejects residual with interpretive overhead.

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement encode_program() and verify_no_overhead()

**File:** `crates/logicaffeine_compile/src/compile.rs`

Implement:
- `encode_program(stmts, expr_arena, interner) -> &Expr` — recursive AST → CProgram
- `verify_no_overhead(stmts) -> Result<(), String>` — walk residual, reject Inspect on
  Core types, reject Core constructor calls, reject env lookups on literal strings

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** PASS (2 tests)

#### STEP 3: RED — Overhead verification tests

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (3):**

3. `p1_no_inspect_on_cstmt`
   Source: Trivial program `CShow(CInt(42))`. Run PE(int, program).
   Verify: Residual contains NO Inspect on CStmt variants.
   Assert: `verify_no_overhead()` passes — all statement dispatch resolved.

4. `p1_no_inspect_on_cexpr`
   Source: Arithmetic program `CLet("x", CInt(5)), CShow(CBinOp("+", CVar("x"), CInt(3)))`.
   Run PE(int, program).
   Verify: Residual contains NO Inspect on CExpr variants.

5. `p1_no_core_constructors`
   Source: Function call program. Run PE(int, program).
   Verify: Residual has no CExpr/CStmt/CFunc/CProgram constructor references.
   Assert: All program-as-data structures eliminated.

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** FAIL (PE not yet powerful enough / integration not done)

#### STEP 4: GREEN — Wire up Projection 1 pipeline

Integrate Sprint 2-3's PE with Sprint 4's interpreter:
- Parse interpreter source to AST
- Encode target program as static CProgram
- Run PE with program=Static, input=Dynamic
- Post-process with fold + dce
- Verify with verify_no_overhead

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** PASS (5 tests)

#### STEP 5: RED — Program pattern tests

**Tests (5):**

6. `p1_trivial_show`
   Source: CProgram with `[CShow(CInt(42))]`.
   Verify: PE resolves all Inspect dispatch. Residual is effectively `Show 42.`
   E2E: assert_exact_output → "42"

7. `p1_arithmetic`
   Source: CProgram with `[CLet("x", CInt(5)), CShow(CBinOp("+", CVar("x"), CInt(3)))]`.
   Verify: Residual is direct `Let x be 5. Show x + 3.` or `Show 8.`
   E2E: assert_exact_output → "8"

8. `p1_control_flow`
   Source: CProgram with `CIf(CBinOp(">", CVar("x"), CInt(5)), ...)` where x=10.
   Verify: x=10, 10>5=true, static condition → dead else eliminated.
   E2E: assert_exact_output → "big"

9. `p1_while_loop`
   Source: CProgram with while loop summing 1..5.
   Verify: Residual contains a while loop (dynamic) or unrolled (if bound is static).
   E2E: assert_exact_output → "15"

10. `p1_multiple_functions`
    Source: CProgram with multiple CFunc definitions and calls.
    Verify: All function dispatch resolved. Residual has direct calls.
    E2E: Output matches interpreter execution.

#### STEP 6: GREEN — Fix any failures

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** PASS (10 tests)

#### STEP 7: RED — Equivalence and dynamic input tests

**Tests (7):**

11. `p1_factorial_5`
    Source: factorial(5) as CProgram. PE with all-static input.
    Verify: Fully evaluated to 120.
    E2E: assert_exact_output → "120"

12. `p1_factorial_10`
    Source: factorial(10) as CProgram. PE with all-static input.
    Verify: Fully evaluated to 3628800.
    E2E: assert_exact_output → "3628800"

13. `p1_sum_loop_100`
    Source: sum-loop(100) as CProgram.
    E2E: assert_exact_output → "5050"

14. `p1_fibonacci_0`
    Source: fib(0) as CProgram.
    E2E: assert_exact_output → "0"

15. `p1_fibonacci_1`
    Source: fib(1) as CProgram.
    E2E: assert_exact_output → "1"

16. `p1_dynamic_input_function`
    Source: CProgram with function taking dynamic input. PE with input=Dynamic.
    Verify: Residual preserves the dynamic computation but eliminates dispatch.
    E2E: Correct output for multiple inputs.

17. `p1_fibonacci_dynamic`
    Source: fib(n) as CProgram where n is dynamic input.
    Verify: Residual contains a recursive function or loop. No CExpr dispatch.
    E2E: input "10" → "55", input "0" → "0", input "1" → "1"

#### STEP 8: GREEN — Fix equivalence failures

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** PASS (17 tests)

#### STEP 9: RED — Comprehensive equivalence and correctness

**Tests (5):**

18. `p1_equivalence_25_pairs`
    Harness: For 5 programs × 5 inputs each:
    1. output_interp = run interpreter directly on (program, input)
    2. compiled = pe(interpreter, program)
    3. output_compiled = run(compiled, input)
    4. assert output_interp == output_compiled
    Programs: trivial, arithmetic, factorial, fibonacci, sum_loop.
    Verify: ALL 25 comparisons pass. Semantic equivalence.

19. `p1_compiled_has_direct_computation`
    Source: Any compiled program from P1.
    Verify: Residual uses +, *, If, While directly — no dispatch indirection.

20. `p1_dynamic_control_flow`
    Source: CProgram with CIf where condition depends on dynamic input.
    Verify: Dispatch on CIf resolved. Runtime If/Otherwise preserved.
    E2E: Different inputs take different branches, both correct.

21. `p1_strings_dynamic`
    Source: CProgram with text operations and dynamic input.
    Verify: String operations work correctly through P1 compilation.

22. `p1_no_env_lookup`
    Source: CProgram with multiple variables.
    Verify: Residual has direct variable bindings (Let x = ...), NOT Map lookups.
    Assert: No `get env at` patterns in residual.

23. `p1_identity_test`
    Source: 5 programs (factorial, fibonacci, sum, abs, gcd) with ALL inputs marked Dynamic.
    Harness: For each program P:
    1. residual = pe(int, P_all_dynamic)
    2. Run `cleanup_identities(residual)` to remove identity let-bindings and no-op wrappers
    3. Verify: residual is structurally equivalent to P (same control flow, same operations)
    4. For 5 inputs each: assert run(residual, input) == run(P_directly, input)
    Assert: pe(int, P_all_dynamic) ≡ P — the PE introduces zero overhead when given no
    static information. This is the identity property of partial evaluation.

#### STEP 10: GREEN — Fix remaining failures

**Run:** `cargo test --test phase_futamura p1_ -- --skip e2e`
**Expected:** PASS (all 23 tests)

#### STEP 11: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 5

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 23 Projection 1 tests pass (including identity test). All previous tests pass.
**Hard stop:** Do NOT proceed to Sprint 6 until this gate is green.

---

## Sprint 6 — Self-Applicable Partial Evaluator

### Overview

For Projections 2 and 3, the PE must process its own source code. This requires writing
the PE in LogicAffeine using only the Core subset, so it can be encoded as a CProgram
and fed to itself.

**Citations:**
- Jones, Sestoft & Søndergaard, "MIX: A Self-Applicable Partial Evaluator" (1989).
- Jones, N.D. (1996). Section 6: Self-application.

**Self-applicability constraints:**
1. First-order only — no closures, no higher-order functions
2. Explicit environments — `env: Map`, `funcs: Map` as explicit params
3. Static dispatch — all Inspect arms on variant tags, no dynamic dispatch
4. No dynamic function name computation — names are always literal strings
5. Memoization by explicit key — `makeKey(name, argVals)` is first-order

### The PE in LogicAffeine

The complete PE source is provided in the original spec (helper functions: `isLiteral`,
`exprToVal`, `valToExpr`, `evalBinOp`, `allLiteral`, `dynamicOnly`, `dynamicArgs`,
`makeKey`; core functions: `peExpr`, `peStmt`, `peBlock`, `extractReturn`).

This source is stored as `crates/logicaffeine_compile/src/optimize/pe_source.logos`.

### Quotation Function

```rust
pub fn quote_pe<'a>(
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> &'a Expr<'a> {
    let pe_source = include_str!("pe_source.logos");
    let tokens = lex(pe_source, interner);
    let ast = parse(tokens, interner, expr_arena, stmt_arena);
    encode_program(&ast, expr_arena, interner)
}
```

### New Files

- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — the PE in LogicAffeine

### Modified Files

- `crates/logicaffeine_compile/src/compile.rs` — add `quote_pe()`
- `crates/logicaffeine_tests/tests/phase_futamura.rs` — add Sprint 6 tests

### TDD Steps

#### STEP 1: RED — PE source parsing and constraints

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (3):**

1. `pe_source_parses`
   Verify: The PE source (pe_source.logos) parses without errors.
   Assert: `compile_to_rust(pe_source)` returns Ok.

2. `pe_no_closures`
   Verify: The PE source contains no closure syntax (no lambda, no captured variables).
   Assert: Parsed AST has no `Expr::Closure` or equivalent.

3. `pe_no_dynamic_fn_names`
   Verify: Every function call in the PE source uses a literal string function name,
   not a computed name.
   Assert: All `CCall` and `CCallS` in the PE source have string-literal first args.

**Run:** `cargo test --test phase_futamura pe_ -- --skip e2e`
**Expected:** FAIL (pe_source.logos doesn't exist yet)

#### STEP 2: GREEN — Create pe_source.logos and quote_pe()

**File:** Create `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Write the complete PE in LogicAffeine (the `peExpr`, `peStmt`, `peBlock`, `extractReturn`
functions plus all helpers). Add `quote_pe()` to `compile.rs`.

**Run:** `cargo test --test phase_futamura pe_ -- --skip e2e`
**Expected:** PASS (3 tests)

#### STEP 3: RED — Quotation correctness

**Tests (3):**

4. `pe_quotation_idempotent`
   Verify: `quote_pe()` called twice produces identical CProgram representations.
   Assert: No non-determinism in encoding (HashMap ordering, symbol interning).

5. `pe_quotation_preserves_behavior`
   Source: Encode PE as CProgram. Run it on a trivial program `[CShow(CInt(42))]`.
   Compare against running PE directly.
   Verify: PE-as-data produces same residual as PE-as-code.
   Assert: Both produce `CShow(CInt(42))` (unchanged, since trivial program is all-static).

6. `pe_self_encodes_correctly`
   Source: Encode PE → CProgram. Run through Core interpreter on a test program.
   Also run PE directly on same test program.
   Verify: Both produce identical residual output.

**Run:** `cargo test --test phase_futamura pe_ -- --skip e2e`
**Expected:** FAIL

#### STEP 4: GREEN — Fix quotation issues

Ensure `quote_pe()` produces a correct, deterministic CProgram encoding.

**Run:** `cargo test --test phase_futamura pe_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 5: RED — Self-application tests

**Tests (6):**

7. `pe_self_applicable_arithmetic`
   Source: Encode PE as CProgram. Run PE-as-CProgram on an arithmetic program
   `[CLet("x", CInt(5)), CShow(CBinOp("+", CVar("x"), CInt(3)))]`.
   Verify: PE produces residual that evaluates to 8.
   Assert: Self-application works for arithmetic.

8. `pe_self_applicable_control_flow`
   Source: PE-as-CProgram on a program with `CIf`.
   Verify: PE correctly partially evaluates if/else with known condition.

9. `pe_self_applicable_recursion`
   Source: PE-as-CProgram on factorial(5).
   Verify: PE correctly evaluates recursive program. Result is 120.

10. `pe_memoization_works`
    Source: PE-as-CProgram on a program with multiple calls to same function.
    Verify: Memo table prevents infinite recursion. Result is correct.

11. `pe_self_applicable_smoke`
    Source: Encode trivial program P = `[CLet("x", CInt(5)), CShow(CVar("x"))]`.
    Run PE directly on P → residual_direct.
    Encode PE as CProgram → pe_cprogram.
    Run Core interpreter on pe_cprogram with P as input → residual_meta.
    Verify: residual_direct == residual_meta. Both produce "5".

12. `pe_specializes_interpreter`
    Source: Let P = factorial(5) as CProgram.
    Run PE on (interpreter, program=P) → residual.
    Verify: verify_no_overhead(residual) passes. Output is "120".
    Assert: PE is powerful enough to fully specialize the interpreter.

#### STEP 6: GREEN — Fix self-application issues

Fix any issues with the PE source, quotation, or meta-interpretation.

**Run:** `cargo test --test phase_futamura pe_ -- --skip e2e`
**Expected:** PASS (all 12 tests)

#### STEP 7: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 6

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 12 self-application tests pass.
**Hard stop:** Do NOT proceed to Sprint 7 until this gate is green.

---

## Sprint 7 — Projection 2: `pe(pe, interpreter) = compiler`

### Overview

The Second Futamura Projection: specializing the PE with respect to a fixed interpreter
produces a *compiler* for that interpreter's language. The compiler takes any program
as input and produces compiled code — with no PE overhead and no interpretive overhead.

**Citations:**
- Futamura, Y. (1971). `[[pe]]([[pe]], [[int]]) = [[compiler]]`
- Jones, N.D. (1996). Section 5: "Requires the PE to be self-applicable."

**Structural requirement — the residual `compiler` must contain:**
- NO reference to `peExpr`/`peStmt`/`peBlock` or PE helpers — all PE dispatch resolved
- NO reference to BTA data structures — binding-time analysis pre-computed
- DOES reference CExpr/CStmt — the compiler manipulates program representations

### Algorithm

1. `pe_as_data = quote_pe()` — encode PE as CProgram
2. `int_as_static = encode_program(interpreter_ast)` — interpreter as static input
3. `compiler = interpret(pe_as_data, { program: int_as_static, input: Dynamic })`
4. Verify: compiler is valid CProgram, no PE dispatch, does have program manipulation
5. Use: `compiled_P = interpret(compiler, P_as_cprogram)`, then `interpret(compiled_P, input)`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — add Projection 2 tests

### TDD Steps

#### STEP 1: RED — Compiler structure tests

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (3):**

1. `p2_no_pe_dispatch`
   Harness: compiler = pe(pe, interpreter).
   Verify: Compiler CProgram contains no peExpr/peStmt/peBlock function calls.
   Assert: All PE dispatch logic resolved for this specific interpreter.

2. `p2_no_bta_types`
   Verify: Compiler has no BindingTime/Division data structures.
   Assert: All BTA for this interpreter is pre-computed into the compiler.

3. `p2_has_program_manipulation`
   Verify: Compiler DOES reference CExpr/CStmt (it takes programs as input).
   Assert: The compiler is not trivially empty — it contains program processing logic.

**Run:** `cargo test --test phase_futamura p2_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Wire up Projection 2

Implement the pipeline:
- Encode PE as CProgram
- Encode interpreter as static CProgram
- Run PE on itself with interpreter as static input
- Verify structural properties

**Run:** `cargo test --test phase_futamura p2_ -- --skip e2e`
**Expected:** PASS (3 tests)

#### STEP 3: RED — Compiler correctness tests

**Tests (5):**

4. `p2_factorial_5`
   Harness: compiler = pe(pe, int). compiled_fac = interpret(compiler, factorial_program).
   result = interpret(compiled_fac, [5]).
   E2E: assert result == "120"

5. `p2_fibonacci_10`
   Harness: compiler = pe(pe, int). compiled_fib = interpret(compiler, fib_program).
   result = interpret(compiled_fib, [10]).
   E2E: assert result == "55"

6. `p2_sum_50`
   Harness: compiler = pe(pe, int). compiled_sum = interpret(compiler, sum_program).
   result = interpret(compiled_sum, [50]).
   E2E: assert result == "1275"

7. `p2_gcd`
   Harness: compiler = pe(pe, int). compiled_gcd = interpret(compiler, gcd_program).
   result = interpret(compiled_gcd, [12, 8]).
   E2E: assert result == "4"

8. `p2_strings`
   Harness: compiler = pe(pe, int). compiled_str = interpret(compiler, string_program).
   result = interpret(compiled_str, ["World"]).
   E2E: assert result == "Hello, World!"

#### STEP 4: GREEN — Fix compiler correctness

**Run:** `cargo test --test phase_futamura p2_ -- --skip e2e`
**Expected:** PASS (8 tests)

#### STEP 5: RED — Consistency and reuse tests

**Tests (7):**

9. `p2_matches_p1`
   Harness: For each program P in {factorial, fibonacci, sum_loop}:
   1. p1_result = pe(int, P) — Projection 1
   2. compiler = pe(pe, int) — Projection 2
   3. p2_result = interpret(compiler, P)
   For each input I in {5, 10, 20}:
   assert interpret(p1_result, I) == interpret(p2_result, I)
   Verify: P1 and P2 produce semantically equivalent compiled code.

10. `p2_correct_for_all_inputs`
    Harness: compiler(factorial) tested with inputs 0, 1, 5, 10, 20.
    Verify: All produce correct factorial values.

11. `p2_compiler_reusable`
    Harness: compiler = pe(pe, int) — generated ONCE.
    compiled_fac = interpret(compiler, factorial). compiled_fib = interpret(compiler, fib).
    Verify: Same compiler handles both programs. Not regenerated.
    E2E: fac(10) → "3628800", fib(10) → "55"

12. `p2_depth_limit_sufficient`
    Verify: PE(PE, int) terminates within depth limits.
    Assert: Compilation completes without hitting hard limits or stack overflow.

13. `p2_produces_valid_cprogram`
    Verify: Output of pe(pe, int) is a valid CProgram that can be interpreted.
    Assert: The compiler is well-formed program data.

14. `p2_produces_compiler`
    Verify: The compiler takes a program as input and produces compiled code as output.
    Assert: interpret(compiler, P) produces a CProgram (not just a value).

15. `p2_multiple_programs`
    Harness: compiler = pe(pe, int).
    Test with factorial, fibonacci, sum, gcd, strings.
    Verify: ALL programs compile and produce correct output via the same compiler.

#### STEP 6: GREEN — Fix consistency issues

**Run:** `cargo test --test phase_futamura p2_ -- --skip e2e`
**Expected:** PASS (all 15 tests)

#### STEP 7: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 7

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 15 Projection 2 tests pass.
**Hard stop:** Do NOT proceed to Sprint 8 until this gate is green.

---

## Sprint 8 — Projection 3: `pe(pe, pe) = compiler_generator`

### Overview

The Third Futamura Projection: specializing the PE with respect to itself produces a
*compiler generator*. Feed it any interpreter → it produces a compiler for that
interpreter's language.

**Citations:**
- Futamura, Y. (1971). `[[pe]]([[pe]], [[pe]]) = [[cogen]]`
- Jones, N.D. (1996). Section 7: "The most theoretically ambitious projection."

**What "compiler generator" means:**
1. `cogen` takes an interpreter `int` (as CProgram) as input
2. Produces a compiler `C` (as CProgram) as output
3. `C` takes a source program `P` → produces compiled code
4. Chain: `cogen → compiler → compiled_program → output`

### The RPN Calculator Interpreter (for universality testing)

A second, independent interpreter for a stack-based language. Proves the compiler
generator works on interpreters other than the Core interpreter.

```
## A RToken is one of:
    A RPush with value Int.
    A RAdd.
    A RSub.
    A RMul.
    A RPrint.

## A RProgram is one of:
    A RProgram with tokens Seq of RToken.

## To rpnEval (program: RProgram, input: Int) -> Text:
    Let mutable output be "".
    Let stack be a new Seq of Int.
    Inspect program:
        When RProgram(tokens):
            Repeat for token in tokens:
                Inspect token:
                    When RPush(n):
                        Push n to stack.
                    When RAdd:
                        Let b be item (length of stack) of stack.
                        Pop from stack.
                        Let a be item (length of stack) of stack.
                        Pop from stack.
                        Push a + b to stack.
                    When RSub:
                        Let b be item (length of stack) of stack.
                        Pop from stack.
                        Let a be item (length of stack) of stack.
                        Pop from stack.
                        Push a - b to stack.
                    When RMul:
                        Let b be item (length of stack) of stack.
                        Pop from stack.
                        Let a be item (length of stack) of stack.
                        Pop from stack.
                        Push a * b to stack.
                    When RPrint:
                        Let v be item (length of stack) of stack.
                        Pop from stack.
                        Set output to output + "{v}".
    Return output.
```

### Algorithm

1. `pe_as_data = quote_pe()` — PE as program to specialize
2. `pe_as_static = quote_pe()` — PE as static input (interpreter role)
3. `cogen = interpret(pe_as_data, { program: pe_as_static, input: Dynamic })`
4. Verify: cogen is valid CProgram, no PE self-referential dispatch
5. Use on Core: `compiler = interpret(cogen, int_as_cprogram)` → same as P2
6. Use on RPN: `rpn_compiler = interpret(cogen, rpn_int_as_cprogram)`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — add Projection 3 tests

### TDD Steps

#### STEP 1: RED — Compiler generator structure

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (3):**

1. `p3_no_pe_self_reference`
   Harness: cogen = pe(pe, pe).
   Verify: cogen has no PE dispatch functions (peExpr, peStmt resolved).
   Assert: PE-on-PE dispatch is fully specialized away.

2. `p3_valid_cprogram`
   Verify: cogen is a valid CProgram that can be interpreted.
   Assert: Well-formed program data.

3. `p3_core_compiler_matches_p2`
   Harness: cogen = pe(pe, pe). compiler_from_cogen = interpret(cogen, interpreter).
   compiler_from_p2 = pe(pe, interpreter).
   Verify: Both produce equivalent compilers — same output for same programs.
   Assert: For test programs, outputs match.

**Run:** `cargo test --test phase_futamura p3_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Wire up Projection 3

Implement the pipeline:
- Encode PE as both data and static input
- Run PE on itself with itself as static
- Verify cogen structure

**Run:** `cargo test --test phase_futamura p3_ -- --skip e2e`
**Expected:** PASS (3 tests)

#### STEP 3: RED — Core language through cogen

**Tests (3):**

4. `p3_full_chain_factorial`
   Harness: cogen → compiler → compiled_factorial → run with input 10.
   Chain: pe(pe,pe) → interpret(cogen, int) → interpret(compiler, fac) → interpret(compiled, [10])
   E2E: assert result == "3628800"

5. `p3_full_chain_fibonacci`
   Harness: Same chain for fibonacci(10).
   E2E: assert result == "55"

6. `p3_full_chain_sum`
   Harness: Same chain for sum_loop(100).
   E2E: assert result == "5050"

#### STEP 4: GREEN — Fix core language chain

**Run:** `cargo test --test phase_futamura p3_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 5: RED — RPN universality tests

**Tests (5):**

7. `p3_rpn_push_print`
   Harness: cogen → rpn_compiler → compiled_rpn.
   RPN program: `[RPush(42), RPrint]`
   E2E: assert result == "42"

8. `p3_rpn_add`
   RPN program: `[RPush(3), RPush(4), RAdd, RPrint]`
   E2E: assert result == "7"

9. `p3_rpn_sub`
   RPN program: `[RPush(10), RPush(3), RSub, RPrint]`
   E2E: assert result == "7"

10. `p3_rpn_mul`
    RPN program: `[RPush(2), RPush(3), RMul, RPrint]`
    E2E: assert result == "6"

11. `p3_rpn_complex`
    RPN program: `[RPush(2), RPush(3), RMul, RPush(4), RAdd, RPrint]`
    Verify: (2 * 3) + 4 = 10.
    E2E: assert result == "10"

#### STEP 6: GREEN — Fix RPN universality

Ensure RPN interpreter encoding works with cogen. Handle `Pop` in Core subset.

**Run:** `cargo test --test phase_futamura p3_ -- --skip e2e`
**Expected:** PASS (11 tests)

#### STEP 7: RED — Consistency tests

**Tests (4):**

12. `p3_quotation_idempotent`
    Verify: quote_pe() called twice produces identical results.
    Assert: No non-determinism. Encoding is stable.

13. `p3_consistency_all_projections`
    Harness: For each program P and input I:
    1. direct = pe(int, P) → output_direct
    2. compiler = pe(pe, int) → compiled → output_p2
    3. cogen = pe(pe, pe) → compiler_gen → compiled_gen → output_p3
    Verify: output_direct == output_p2 == output_p3 for ALL pairs.
    Assert: All three projections produce semantically equivalent results.

14. `p3_different_interpreter`
    Harness: cogen = pe(pe, pe). rpn_compiler = interpret(cogen, rpn_interpreter).
    Verify: cogen produces a working compiler for the RPN language (not just Core).
    Assert: Universality — same cogen handles different interpreters.

15. `p3_full_chain_fibonacci_dynamic`
    Harness: Full chain cogen → compiler → compiled_fib with dynamic input.
    E2E: input "10" → "55", input "1" → "1", input "0" → "0"

16. `p3_cross_projection_byte_identical`
    Harness: For 5 programs (factorial, fibonacci, sum, gcd, string_greet):
    1. residual_p1 = pe(int, P)
    2. compiler = pe(pe, int); residual_p2 = interpret(compiler, P)
    3. cogen = pe(pe, pe); compiler_gen = interpret(cogen, int); residual_p3 = interpret(compiler_gen, P)
    Verify: residual_p1, residual_p2, residual_p3 are byte-identical for each P.
    Assert: All 5 programs produce identical residuals across all three projections.

17. `p3_cogen_produces_identical_compiler`
    Harness: compiler_p2 = pe(pe, int). cogen = pe(pe, pe). compiler_p3 = interpret(cogen, int).
    Verify: compiler_p2 and compiler_p3 are byte-identical CPrograms.
    Assert: The compiler generator produces the exact same compiler as direct P2.

18. `p3_triple_equivalence_10_programs`
    Harness: For 10 programs × 5 inputs each (50 total comparisons):
    1. output_p1 = run(pe(int, P), input)
    2. output_p2 = run(interpret(pe(pe, int), P), input)
    3. output_p3 = run(interpret(interpret(pe(pe, pe), int), P), input)
    Verify: output_p1 == output_p2 == output_p3 for ALL 50 pairs.
    Programs: factorial, fibonacci, sum, gcd, string_greet, power, abs, max, min, collatz_steps.

#### STEP 8: GREEN — Fix consistency

**Run:** `cargo test --test phase_futamura p3_ -- --skip e2e`
**Expected:** PASS (all 18 tests)

#### STEP 9: VERIFY — Full regression (FINAL)

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Every test across all sprints passes.

### VERIFICATION GATE — Sprint 8 (FINAL)

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 228 tests from Sprints 1-8 pass (221 test functions + 7 gates).
This is the theoretical capstone. The compiler generator is a universal machine.
Sprints 9-20 extend the Core subset to cover the full language.

---

## Sprint 9 — Float, Extended Operators

### Overview

The Core subset handles integers, booleans, text, sequences, and maps — but not
floating-point numbers. This sprint adds float literals and runtime values, completes
binary operator coverage with bitwise XOR, shift left, and shift right, and extends
`encode_program()` to map `Literal::Float` and the missing `BinaryOp` variants into
the CProgram representation. Every numeric program that uses decimal arithmetic requires
this extension.

**What changes structurally:**
1. New `CFloat` variant in CExpr — float literal
2. New `VFloat` variant in CVal — float runtime value
3. `applyBinOp` extended for float arithmetic, float comparison, int/float promotion
4. `applyBinOp` extended for bitwise operators: `"^"`, `"<<"`, `">>"`
5. `valToText` extended for `VFloat`
6. `encode_program()` maps `Literal::Float` → `CFloat`, `BinaryOp::{BitXor, Shl, Shr}` → `CBinOp`

### New Core Types

```
CExpr additions:
    A CFloat with value Float.

CVal additions:
    A VFloat with value Float.

applyBinOp additions:
    "^"  on VInt(a), VInt(b)     → VInt(a xor b)
    "<<" on VInt(a), VInt(b)     → VInt(a shl b)
    ">>" on VInt(a), VInt(b)     → VInt(a shr b)
    "+", "-", "*", "/" on VFloat → float arithmetic
    comparisons on VFloat        → VBool
    VInt op VFloat               → promote VInt to VFloat, then compute
    VFloat op VInt               → promote VInt to VFloat, then compute
```

### Algorithm

1. Add `CFloat(value: Float)` to CExpr sum type definition
2. Add `VFloat(value: Float)` to CVal sum type definition
3. `coreEval`: `CFloat(v)` → `VFloat(v)`
4. `applyBinOp`: float arithmetic — `VFloat(a) + VFloat(b) → VFloat(a + b)`, etc.
5. `applyBinOp`: int/float promotion — `VInt(a) + VFloat(b) → VFloat(toFloat(a) + b)`
6. `applyBinOp`: float comparison — `VFloat(a) < VFloat(b) → VBool(...)`
7. `applyBinOp`: bitwise — `VInt(a) "^" VInt(b) → VInt(a xor b)`, shifts similarly
8. `applyBinOp`: float division by zero — `VFloat(a) / VFloat(0.0) → VError("division by zero")`
9. `valToText(VFloat(f))` → format float as text string
10. `encode_program`: `Literal::Float(f)` → `CFloat(f)`
11. `encode_program`: `BinaryOp::BitXor` → `CBinOp("^", ...)`, `Shl` → `"<<"`, `Shr` → `">>"`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 15 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Float literals and arithmetic

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

Write tests that use `CFloat` and `VFloat` — neither exists in Core types yet.

**Tests (7):**

1. `core_float_literal`
   Source: CProgram with `[CShow(CFloat(3.14))]`.
   Assert: Output is "3.14"

2. `core_float_addition`
   Source: CProgram with `CLet("x", CFloat(1.5)), CShow(CBinOp("+", CVar("x"), CFloat(2.5)))`.
   Assert: Output is "4"

3. `core_float_multiplication`
   Source: CProgram with `CShow(CBinOp("*", CFloat(2.0), CFloat(3.5)))`.
   Assert: Output is "7"

4. `core_float_division`
   Source: CProgram with `CShow(CBinOp("/", CFloat(10.0), CFloat(4.0)))`.
   Assert: Output is "2.5"

5. `core_float_subtraction`
   Source: CProgram with `CShow(CBinOp("-", CFloat(5.0), CFloat(2.5)))`.
   Assert: Output is "2.5"

6. `core_float_comparison`
   Source: CProgram with `CIf(CBinOp(">", CFloat(3.14), CFloat(2.71)), [CShow(CText("bigger"))], [])`.
   Assert: Output is "bigger"

7. `core_float_int_promotion`
   Source: CProgram with `CShow(CBinOp("+", CInt(2), CFloat(3.5)))`.
   Assert: Output is "5.5"

**Run:** `cargo test --test phase_futamura core_float_ -- --skip e2e`
**Expected:** FAIL (CFloat, VFloat not defined)

#### STEP 2: GREEN — Implement float support

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` — add `CFloat` to CExpr, `VFloat` to CVal
**File:** `crates/logicaffeine_compile/src/optimize/pe_source.logos` — extend `coreEval`, `applyBinOp`, `valToText`

Add `CFloat(v)` → `VFloat(v)` in `coreEval`. Add float arithmetic and comparison
arms in `applyBinOp`. Add int/float promotion: when one operand is VInt and the other
VFloat, convert the VInt to VFloat before computing. Add `VFloat(f)` → `"{f}"` in
`valToText`.

**Run:** `cargo test --test phase_futamura core_float_ -- --skip e2e`
**Expected:** PASS (7 tests)

#### STEP 3: RED — Bitwise operators and encoding

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (8):**

8. `core_float_to_text`
   Source: `Let v be a new VFloat with value 3.14. Show valToText(v).`
   Assert: Output includes "3.14"

9. `core_float_div_by_zero`
   Source: CProgram with `CShow(CBinOp("/", CFloat(1.0), CFloat(0.0)))`.
   Assert: Output contains "Error"

10. `core_bitxor`
    Source: CProgram with `CShow(CBinOp("^", CInt(5), CInt(3)))`.
    Assert: Output is "6"

11. `core_shl`
    Source: CProgram with `CShow(CBinOp("<<", CInt(1), CInt(4)))`.
    Assert: Output is "16"

12. `core_shr`
    Source: CProgram with `CShow(CBinOp(">>", CInt(16), CInt(2)))`.
    Assert: Output is "4"

13. `core_float_comparison_eq`
    Source: CProgram with `CIf(CBinOp("==", CFloat(1.0), CFloat(1.0)), [CShow(CText("eq"))], [CShow(CText("ne"))])`.
    Assert: Output is "eq"

14. `core_float_nested_arithmetic`
    Source: CProgram with `CShow(CBinOp("*", CBinOp("+", CFloat(2.0), CFloat(3.0)), CFloat(4.0)))`.
    Assert: Output is "20"

15. `core_float_encode_roundtrip`
    Source: LogicAffeine program `Show 3.14.` encoded via `encode_program()`.
    Assert: Encoded CProgram contains CFloat. When run through self-interpreter, output is "3.14".

**Run:** `cargo test --test phase_futamura core_float_ core_bitxor core_shl core_shr -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement bitwise operators and encode_program extensions

Extend `applyBinOp` for `"^"`, `"<<"`, `">>"` on VInt operands. Extend
`encode_program()` in `compile.rs` for `Literal::Float` → `CFloat` and
`BinaryOp::BitXor` → `CBinOp("^", ...)`, `BinaryOp::Shl` → `CBinOp("<<", ...)`,
`BinaryOp::Shr` → `CBinOp(">>", ...)`.

**Run:** `cargo test --test phase_futamura core_float_ core_bitxor core_shl core_shr -- --skip e2e`
**Expected:** PASS (15 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 9

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 15 float/operator tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 10 until this gate is green.

---

## Sprint 10 — Iteration: For-Each, For-Range, Break, Pop, Slice, Copy

### Overview

The Core subset handles `CWhile` loops but has no structured iteration. Real programs
iterate over collections with `for-each` and over ranges with `for i in 1..n`. This
sprint adds `CRepeat` (for-each on a sequence), `CRepeatRange` (for over an integer
range), `CBreak` (early loop exit), `CPop` (remove last element), and the expression
forms `CList` (list literal), `CRange` (integer range), `CSlice` (sub-sequence), and
`CCopy` (shallow copy). These are the workhorses of idiomatic LogicAffeine programs.

**Why this matters for PE:**
For-each loops over static sequences can be fully unrolled by the partial evaluator.
Range loops with static bounds can be unrolled or specialized. `CBreak` introduces
non-local control flow within loops, requiring the self-interpreter to propagate a
break signal through `coreExecBlock`.

### New Core Types

```
CExpr additions:
    A CList with items Seq of CExpr.
    A CRange with start CExpr and end CExpr.
    A CSlice with coll CExpr and start CExpr and end CExpr.
    A CCopy with target CExpr.

CStmt additions:
    A CRepeat with var Text and coll CExpr and body Seq of CStmt.
    A CRepeatRange with var Text and start CExpr and end CExpr and body Seq of CStmt.
    A CBreak.
    A CPop with target Text.
```

### Algorithm

1. Add CExpr variants: `CList`, `CRange`, `CSlice`, `CCopy`
2. Add CStmt variants: `CRepeat`, `CRepeatRange`, `CBreak`, `CPop`
3. `coreEval CList(items)` → evaluate each item, collect into `VSeq`
4. `coreEval CRange(start, end)` → `VSeq([start, start+1, ..., end])` (inclusive)
5. `coreEval CSlice(coll, start, end)` → evaluate coll, extract sub-sequence
6. `coreEval CCopy(target)` → evaluate target, return copy of VSeq
7. `coreExecBlock CRepeat(var, coll, body)` → evaluate coll (must be VSeq), iterate
   items, bind `var` in env for each, execute body. Handle CReturn and CBreak signals.
8. `coreExecBlock CRepeatRange(var, start, end, body)` → evaluate start/end (must be
   VInt), iterate from start to end inclusive, bind `var` = VInt(i) for each.
9. `coreExecBlock CBreak` → signal break. The CWhile, CRepeat, and CRepeatRange
   handlers must check for break after each body iteration.
10. `coreExecBlock CPop(target)` → look up `target` in env (must be VSeq), remove last
    element, update env. Error if empty.
11. `encode_program()` maps `Stmt::Repeat` → `CRepeat`, `Stmt::Break` → `CBreak`,
    `Stmt::Pop` → `CPop`, `Expr::List` → `CList`, `Expr::Range` → `CRange`,
    `Expr::Slice` → `CSlice`, `Expr::Copy` → `CCopy`.

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 18 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — List literals and range expressions

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (6):**

1. `core_iter_list_literal`
   Source: CProgram with `CLet("xs", CList([CInt(10), CInt(20), CInt(30)])), CShow(CLen(CVar("xs")))`.
   Assert: Output is "3"

2. `core_iter_range_expr`
   Source: CProgram with `CLet("xs", CRange(CInt(1), CInt(5))), CShow(CLen(CVar("xs")))`.
   Assert: Output is "5" (range 1..5 inclusive = 5 elements)

3. `core_iter_range_empty`
   Source: CProgram with `CLet("xs", CRange(CInt(5), CInt(1))), CShow(CLen(CVar("xs")))`.
   Assert: Output is "0" (start > end = empty)

4. `core_iter_slice`
   Source: CProgram with `CLet("xs", CList([CInt(10), CInt(20), CInt(30), CInt(40)])), CLet("ys", CSlice(CVar("xs"), CInt(2), CInt(3))), CShow(CLen(CVar("ys")))`.
   Assert: Output is "2" (items 2..3)

5. `core_iter_copy`
   Source: CProgram with `CLet("xs", CList([CInt(1), CInt(2)])), CLet("ys", CCopy(CVar("xs"))), CPush(CInt(3), "ys"), CShow(CLen(CVar("xs")))`.
   Assert: Output is "2" (original unchanged after push to copy)

6. `core_iter_list_show_elements`
   Source: CProgram with `CLet("xs", CList([CInt(10), CInt(20)])), CShow(CIndex(CVar("xs"), CInt(1)))`.
   Assert: Output is "10"

**Run:** `cargo test --test phase_futamura core_iter_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement list, range, slice, copy

Extend CExpr type defs. Implement `coreEval` arms for CList, CRange, CSlice, CCopy.

**Run:** `cargo test --test phase_futamura core_iter_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 3: RED — For-each and for-range loops

**Tests (7):**

7. `core_iter_repeat_basic`
   Source: CProgram with `CLet("xs", CList([CInt(1), CInt(2), CInt(3)])), CRepeat("x", CVar("xs"), [CShow(CVar("x"))])`.
   Assert: Output is "1\n2\n3"

8. `core_iter_repeat_accumulate`
   Source: CProgram with `CLet("sum", CInt(0)), CLet("xs", CList([CInt(1), CInt(2), CInt(3)])), CRepeat("x", CVar("xs"), [CSet("sum", CBinOp("+", CVar("sum"), CVar("x")))]), CShow(CVar("sum"))`.
   Assert: Output is "6"

9. `core_iter_repeat_empty`
   Source: CProgram with `CLet("xs", CList([])), CRepeat("x", CVar("xs"), [CShow(CVar("x"))]), CShow(CText("done"))`.
   Assert: Output is "done" (loop body never executes)

10. `core_iter_repeat_range`
    Source: CProgram with `CRepeatRange("i", CInt(1), CInt(5), [CShow(CVar("i"))])`.
    Assert: Output is "1\n2\n3\n4\n5"

11. `core_iter_nested_repeat`
    Source: CProgram with `CLet("xs", CList([CInt(1), CInt(2)])), CRepeat("x", CVar("xs"), [CRepeatRange("i", CInt(1), CInt(2), [CShow(CBinOp("*", CVar("x"), CVar("i")))])])`.
    Assert: Output is "1\n2\n2\n4"

12. `core_iter_repeat_with_return`
    Source: CProgram with function that iterates and returns early.
    Assert: Return inside for-each propagates correctly.

13. `core_iter_repeat_with_push`
    Source: CProgram with `CLet("result", CList([])), CRepeatRange("i", CInt(1), CInt(3), [CPush(CBinOp("*", CVar("i"), CInt(10)), "result")]), CShow(CLen(CVar("result")))`.
    Assert: Output is "3"

**Run:** `cargo test --test phase_futamura core_iter_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement CRepeat, CRepeatRange

Extend CStmt type defs. Implement `coreExecBlock` arms for CRepeat (evaluate collection,
iterate items, bind var) and CRepeatRange (evaluate bounds, iterate integers).

**Run:** `cargo test --test phase_futamura core_iter_ -- --skip e2e`
**Expected:** PASS (13 tests)

#### STEP 5: RED — Break, Pop, and encoding

**Tests (5):**

14. `core_iter_break_basic`
    Source: CProgram with `CRepeatRange("i", CInt(1), CInt(100), [CIf(CBinOp(">", CVar("i"), CInt(3)), [CBreak], []), CShow(CVar("i"))])`.
    Assert: Output is "1\n2\n3"

15. `core_iter_break_in_while`
    Source: CProgram with `CLet("i", CInt(0)), CWhile(CBool(true), [CIf(CBinOp(">=", CVar("i"), CInt(5)), [CBreak], []), CShow(CVar("i")), CSet("i", CBinOp("+", CVar("i"), CInt(1)))])`.
    Assert: Output is "0\n1\n2\n3\n4"

16. `core_iter_pop`
    Source: CProgram with `CLet("xs", CList([CInt(10), CInt(20), CInt(30)])), CPop("xs"), CShow(CLen(CVar("xs")))`.
    Assert: Output is "2"

17. `core_iter_pop_empty_error`
    Source: CProgram with `CLet("xs", CList([])), CPop("xs")`.
    Assert: No crash; empty pop is handled gracefully.

18. `core_iter_encode_repeat`
    Source: LogicAffeine program `Repeat for x in items: Show x.` encoded via `encode_program()`.
    Assert: Encoded CProgram contains CRepeat. Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_iter_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 6: GREEN — Implement Break, Pop, encode_program extensions

CBreak signal propagation: introduce a break sentinel (e.g., a special VError or a flag)
that short-circuits the current loop iteration. CWhile, CRepeat, and CRepeatRange all
check for the break signal after each body execution.

CPop: look up target in env, verify VSeq, remove last element, update env.

`encode_program()`: map Stmt::Repeat → CRepeat, Stmt::Break → CBreak, Stmt::Pop → CPop,
Expr::List → CList, Expr::Range → CRange, Expr::Slice → CSlice, Expr::Copy → CCopy.

**Run:** `cargo test --test phase_futamura core_iter_ -- --skip e2e`
**Expected:** PASS (18 tests)

#### STEP 7: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 10

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 18 iteration tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 11 until this gate is green.

---

## Sprint 11 — Sets, Options, Tuples, Collection Completeness

### Overview

The Core subset handles sequences (VSeq) and maps (VMap) but has no sets, options, or
tuples. This sprint adds `VSet` (unordered unique collection), `VOption` (optional
value), `VTuple` (fixed-size heterogeneous collection), and the operations that
manipulate them: `CContains`, `CUnion`, `CIntersection` for sets; `COptionSome`,
`COptionNone` for options; `CTuple` for tuple construction; and `CAdd`/`CRemove` for
set mutation statements.

**Why this matters for PE:**
Set operations with static contents can be fully evaluated at compile time. Option
wrapping/unwrapping is a common pattern that PE can specialize away. Tuples with
known static elements enable element-wise specialization.

### New Core Types

```
CExpr additions:
    A CContains with coll CExpr and elem CExpr.
    A CUnion with left CExpr and right CExpr.
    A CIntersection with left CExpr and right CExpr.
    A COptionSome with inner CExpr.
    A COptionNone.
    A CTuple with items Seq of CExpr.

CStmt additions:
    A CAdd with elem CExpr and target Text.
    A CRemove with elem CExpr and target Text.

CVal additions:
    A VSet with items Seq of CVal.
    A VOption with inner CVal and present Bool.
    A VTuple with items Seq of CVal.
```

### Algorithm

1. Add CExpr variants: `CContains`, `CUnion`, `CIntersection`, `COptionSome`, `COptionNone`, `CTuple`
2. Add CStmt variants: `CAdd`, `CRemove`
3. Add CVal variants: `VSet`, `VOption`, `VTuple`
4. `coreEval CContains(coll, elem)` → check if elem value exists in VSeq or VSet → VBool
5. `coreEval CUnion(left, right)` → merge two VSet values, deduplicate → VSet
6. `coreEval CIntersection(left, right)` → common elements of two VSets → VSet
7. `coreEval COptionSome(inner)` → `VOption(value, true)`
8. `coreEval COptionNone` → `VOption(VNothing, false)`
9. `coreEval CTuple(items)` → evaluate each, collect into `VTuple`
10. `coreExecBlock CAdd(elem, target)` → look up target (VSet), add elem if not present
11. `coreExecBlock CRemove(elem, target)` → look up target (VSet), remove elem
12. `valToText`: `VSet` → `"[set]"`, `VOption` → `"Some(...)"` or `"None"`, `VTuple` → `"(a, b)"`
13. `encode_program()` maps `Expr::Contains` → `CContains`, `Expr::Union` → `CUnion`,
    `Expr::Intersection` → `CIntersection`, `Expr::OptionSome` → `COptionSome`,
    `Expr::OptionNone` → `COptionNone`, `Expr::Tuple` → `CTuple`,
    `Stmt::Add` → `CAdd`, `Stmt::Remove` → `CRemove`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 15 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Set operations

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (6):**

1. `core_set_add_and_contains`
   Source: CProgram creating empty VSet, CAdd element, CContains check.
   Assert: Contains returns "true"

2. `core_set_remove`
   Source: CProgram with VSet, CAdd, CRemove, CContains.
   Assert: Contains returns "false" after remove

3. `core_set_union`
   Source: CProgram with two VSets, CUnion.
   Assert: Union contains all elements from both sets.

4. `core_set_intersection`
   Source: CProgram with two VSets sharing some elements, CIntersection.
   Assert: Intersection contains only shared elements.

5. `core_set_no_duplicates`
   Source: CProgram adding same element twice.
   Assert: Set still has one copy (length check).

6. `core_set_contains_not_found`
   Source: CProgram with VSet, CContains for missing element.
   Assert: Returns "false"

**Run:** `cargo test --test phase_futamura core_set_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement set types and operations

Add VSet, CAdd, CRemove, CContains, CUnion, CIntersection. Implement in self-interpreter.

**Run:** `cargo test --test phase_futamura core_set_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 3: RED — Options and tuples

**Tests (6):**

7. `core_option_some`
   Source: CProgram with `COptionSome(CInt(42))`, show via valToText.
   Assert: Output indicates Some(42)

8. `core_option_none`
   Source: CProgram with `COptionNone`, show via valToText.
   Assert: Output indicates None

9. `core_option_unwrap`
   Source: CProgram that inspects VOption: when present, show inner value.
   Assert: Output is "42"

10. `core_tuple_create`
    Source: CProgram with `CTuple([CInt(1), CText("hello"), CBool(true)])`.
    Assert: VTuple with 3 items; index access works.

11. `core_tuple_index`
    Source: CProgram with CTuple, CIndex to extract elements.
    Assert: Output is correct element value.

12. `core_tuple_to_text`
    Source: CProgram with CTuple, show via valToText.
    Assert: Output is formatted tuple string.

**Run:** `cargo test --test phase_futamura core_option_ core_tuple_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement option and tuple types

Add VOption, VTuple, COptionSome, COptionNone, CTuple. Implement in self-interpreter.

**Run:** `cargo test --test phase_futamura core_option_ core_tuple_ -- --skip e2e`
**Expected:** PASS (12 tests)

#### STEP 5: RED — Contains on sequences and encoding

**Tests (3):**

13. `core_contains_in_seq`
    Source: CProgram with VSeq, CContains for element in sequence.
    Assert: Returns "true" when element exists.

14. `core_contains_text_in_text`
    Source: CProgram with CContains on VText values (substring check).
    Assert: Returns "true" for substring match.

15. `core_set_encode_roundtrip`
    Source: LogicAffeine program using sets encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_contains_ core_set_encode_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 6: GREEN — Implement sequence contains and encoding

Extend CContains to handle VSeq (linear scan) and VText (substring). Extend
`encode_program()` for all new types.

**Run:** `cargo test --test phase_futamura core_set_ core_option_ core_tuple_ core_contains_ -- --skip e2e`
**Expected:** PASS (15 tests)

#### STEP 7: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 11

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 15 collection tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 12 until this gate is green.

---

## Sprint 12 — Structs, Fields, Constructors

### Overview

LogicAffeine supports user-defined struct types with named fields. The Core subset has
no struct support — only primitive values and sequences/maps. This sprint adds `VStruct`
(struct runtime value with named fields), `CNew` (constructor expression), `CFieldAccess`
(field read), `CSetField` (field mutation), and `CStructDef` (struct type definition
emitted as a no-op in the self-interpreter, since the interpreter is dynamically typed).

**Why this matters for PE:**
Struct field accesses on statically known structs can be resolved at compile time.
Constructor calls with all-static arguments can be fully evaluated. This enables PE
to specialize through data structure boundaries.

### New Core Types

```
CExpr additions:
    A CNew with typeName Text and fields Seq of CExpr.
    A CFieldAccess with target CExpr and field Text.

CStmt additions:
    A CSetField with target Text and field Text and val CExpr.
    A CStructDef with name Text and fieldNames Seq of Text.

CVal additions:
    A VStruct with typeName Text and fields Map of Text to CVal.
```

### Algorithm

1. Add `CNew`, `CFieldAccess` to CExpr; `CSetField`, `CStructDef` to CStmt; `VStruct` to CVal
2. `coreEval CNew(typeName, fieldExprs)` → evaluate each field expr, pair with field
   names from the CStructDef registry, create `VStruct(typeName, fieldMap)`
3. `coreEval CFieldAccess(target, field)` → evaluate target (must be VStruct), look up
   field in its fields map
4. `coreExecBlock CSetField(target, field, val)` → look up target in env (VStruct),
   set field in its fields map, update env
5. `coreExecBlock CStructDef(name, fieldNames)` → register field names in a struct
   registry (stored in env or a separate map) for use by CNew
6. `valToText VStruct(typeName, fields)` → formatted struct string
7. `encode_program()` maps `Expr::New` → `CNew`, `Expr::FieldAccess` → `CFieldAccess`,
   `Stmt::SetField` → `CSetField`, `Stmt::StructDef` → `CStructDef`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 15 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Struct construction and field access

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (8):**

1. `core_struct_new`
   Source: CProgram with CStructDef("Point", ["x", "y"]), CLet("p", CNew("Point", [CInt(3), CInt(4)])), CShow(CFieldAccess(CVar("p"), "x")).
   Assert: Output is "3"

2. `core_struct_field_y`
   Source: Same as above but access field "y".
   Assert: Output is "4"

3. `core_struct_set_field`
   Source: CProgram creating Point, CSetField("p", "x", CInt(10)), show field.
   Assert: Output is "10" (field mutated)

4. `core_struct_to_text`
   Source: CProgram creating Point, show via valToText.
   Assert: Output is formatted struct representation.

5. `core_struct_nested`
   Source: CProgram with struct containing another struct.
   CStructDef("Line", ["start", "end"]), two Points, a Line.
   Assert: Nested field access works.

6. `core_struct_pass_to_function`
   Source: CProgram with CFunc that takes a struct and returns a field.
   Assert: Function correctly accesses struct field.

7. `core_struct_multiple_types`
   Source: CProgram with two different struct types (Point and Color).
   Assert: Both work independently.

8. `core_struct_field_missing`
   Source: CProgram accessing a non-existent field.
   Assert: Returns VNothing or VError gracefully.

**Run:** `cargo test --test phase_futamura core_struct_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement struct types

Add VStruct, CNew, CFieldAccess, CSetField, CStructDef to Core types. Implement in
self-interpreter. CStructDef registers field name ordering. CNew uses that ordering to
pair field expressions with field names.

**Run:** `cargo test --test phase_futamura core_struct_ -- --skip e2e`
**Expected:** PASS (8 tests)

#### STEP 3: RED — Struct computation and encoding

**Tests (7):**

9. `core_struct_arithmetic_fields`
   Source: CProgram with Point, compute distance formula (x*x + y*y), show result.
   Assert: Correct arithmetic on struct fields.

10. `core_struct_in_sequence`
    Source: CProgram creating a sequence of structs, iterating, accessing fields.
    Assert: Correct field access on struct elements.

11. `core_struct_copy_semantics`
    Source: CProgram creating struct, assigning to new variable, mutating original.
    Assert: Copy has value semantics (changes to original don't affect copy).

12. `core_struct_in_map`
    Source: CProgram storing struct as map value, retrieving, accessing field.
    Assert: Struct survives map storage and retrieval.

13. `core_struct_recursive`
    Source: CProgram with struct containing a sequence (e.g., Tree with children).
    Assert: Recursive structure works through the interpreter.

14. `core_struct_with_function`
    Source: CProgram with function returning a new struct.
    Assert: Struct construction inside functions works.

15. `core_struct_encode_roundtrip`
    Source: LogicAffeine program using structs encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_struct_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Fix struct computation and encoding

Extend `encode_program()` for struct-related AST nodes.

**Run:** `cargo test --test phase_futamura core_struct_ -- --skip e2e`
**Expected:** PASS (15 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 12

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 15 struct tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 13 until this gate is green.

---

## Sprint 13 — Enums, Pattern Matching (Inspect), Variants

### Overview

LogicAffeine's sum types (enums) are defined with `A <Type> is one of:` declarations
and deconstructed with `Inspect`/`When` pattern matching. The Core subset currently
has no support for user-defined variants beyond the built-in CExpr/CStmt/CVal types
themselves. This sprint adds `VVariant` (enum runtime value), `CNewVariant` (variant
constructor expression), `CInspect` (pattern matching statement), and `CMatchArm`
(individual match arm with pattern and body).

**Why this matters for PE:**
Inspect on a statically known variant can be resolved at compile time — the partial
evaluator selects the matching arm and discards the rest. This is the sum-type analog
of static branch elimination for `CIf`. Without this, programs using algebraic data
types cannot benefit from PE.

### New Core Types

```
CExpr additions:
    A CNewVariant with typeName Text and variantName Text and fields Seq of CExpr.

CStmt additions:
    A CInspect with target CExpr and arms Seq of CMatchArm.
    A CEnumDef with name Text and variants Seq of Text.

Support type:
## A CMatchArm is one of:
    A CWhen with variantName Text and bindings Seq of Text and body Seq of CStmt.
    A COtherwise with body Seq of CStmt.

CVal additions:
    A VVariant with typeName Text and variantName Text and fields Seq of CVal.
```

### Algorithm

1. Add `CNewVariant` to CExpr, `CInspect`/`CEnumDef` to CStmt, `CMatchArm` as new
   sum type, `VVariant` to CVal
2. `coreEval CNewVariant(typeName, variantName, fieldExprs)` → evaluate fields,
   create `VVariant(typeName, variantName, fieldVals)`
3. `coreExecBlock CInspect(target, arms)` → evaluate target, iterate arms:
   - `CWhen(variantName, bindings, body)` — if target is VVariant with matching
     variantName, bind fields to binding names in env, execute body
   - `COtherwise(body)` — fallback, execute body
4. `coreExecBlock CEnumDef(name, variants)` → register variant names (for validation)
5. `valToText VVariant(typeName, variantName, fields)` → formatted representation
6. `encode_program()` maps `Expr::NewVariant` → `CNewVariant`, `Stmt::Inspect` →
   `CInspect`, match arms → `CWhen`/`COtherwise`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 18 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Variant construction

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (6):**

1. `core_enum_new_variant`
   Source: CProgram with `CNewVariant("Shape", "Circle", [CFloat(5.0)])`, show via valToText.
   Assert: Output represents Circle variant.

2. `core_enum_inspect_match`
   Source: CProgram with `CLet("s", CNewVariant("Shape", "Circle", [CFloat(5.0)])), CInspect(CVar("s"), [CWhen("Circle", ["r"], [CShow(CVar("r"))])])`.
   Assert: Output is "5"

3. `core_enum_inspect_second_arm`
   Source: CProgram with variant "Square", inspect with Circle and Square arms.
   Assert: Square arm executes.

4. `core_enum_inspect_otherwise`
   Source: CProgram with variant not matching any When arm, falls to Otherwise.
   Assert: Otherwise body executes.

5. `core_enum_no_field_variant`
   Source: CProgram with zero-field variant (e.g., "None" of Option type).
   Assert: Construction and matching works.

6. `core_enum_multiple_fields`
   Source: CProgram with multi-field variant (e.g., "Rect" with width and height).
   Assert: All fields bound correctly in match arm.

**Run:** `cargo test --test phase_futamura core_enum_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement variant types and Inspect

Add VVariant, CNewVariant, CInspect, CMatchArm, CEnumDef. Implement variant
construction in coreEval and pattern matching in coreExecBlock.

**Run:** `cargo test --test phase_futamura core_enum_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 3: RED — Complex pattern matching

**Tests (7):**

7. `core_enum_nested_inspect`
   Source: CProgram with variant containing another variant, nested Inspect.
   Assert: Inner variant correctly deconstructed.

8. `core_enum_inspect_return`
   Source: CProgram with function that inspects a variant and returns a value.
   Assert: Return from within Inspect arm works.

9. `core_enum_inspect_with_computation`
   Source: CProgram inspecting Shape, computing area (Circle: pi*r*r, Square: s*s).
   Assert: Correct computed output.

10. `core_enum_in_sequence`
    Source: CProgram with sequence of variants, iterate and inspect each.
    Assert: Each variant matched and processed correctly.

11. `core_enum_variant_equality`
    Source: CProgram comparing two variants (same type, same name, same fields).
    Assert: Comparison works (or shows how equality is expressed).

12. `core_enum_recursive_type`
    Source: CProgram with recursive enum (e.g., Expr tree with Add(Expr, Expr) and Num(Int)).
    Assert: Recursive construction and matching works.

13. `core_enum_inspect_all_arms`
    Source: CProgram with 4+ variants, inspect with one arm per variant.
    Assert: Each variant reaches its corresponding arm.

**Run:** `cargo test --test phase_futamura core_enum_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Fix complex pattern matching

Handle nested Inspect, return propagation from within arms, recursive variant types.

**Run:** `cargo test --test phase_futamura core_enum_ -- --skip e2e`
**Expected:** PASS (13 tests)

#### STEP 5: RED — Encoding and edge cases

**Tests (5):**

14. `core_enum_pass_to_function`
    Source: CProgram with function taking a variant parameter.
    Assert: Variant passed and inspected inside function body.

15. `core_enum_construct_in_function`
    Source: CProgram with function returning a new variant.
    Assert: Variant construction inside function works.

16. `core_enum_map_over_variants`
    Source: CProgram with sequence of variants, map-like pattern (iterate, inspect, push result).
    Assert: Transformed sequence has correct values.

17. `core_enum_inspect_no_match`
    Source: CProgram with Inspect that has no matching arm and no Otherwise.
    Assert: Falls through gracefully (no crash, produces VNothing).

18. `core_enum_encode_roundtrip`
    Source: LogicAffeine program using enums and Inspect encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_enum_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 6: GREEN — Fix encoding and edge cases

Extend `encode_program()` for enum-related AST nodes. Handle Inspect with no match.

**Run:** `cargo test --test phase_futamura core_enum_ -- --skip e2e`
**Expected:** PASS (18 tests)

#### STEP 7: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 13

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 18 enum/Inspect tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 14 until this gate is green.

---

## Sprint 14 — Closures, Higher-Order Functions, String Interpolation

### Overview

LogicAffeine supports closures (anonymous functions), calling closure-holding variables,
and string interpolation with format specifiers. The Core subset has none of these.
This sprint adds `VClosure` (closure runtime value capturing environment), `CClosure`
(closure expression), `CCallExpr` (call a variable holding a closure), and
`CInterpolatedString` with `CStringPart` (string interpolation with embedded
expressions).

**Why this matters for PE:**
Closures passed as arguments create higher-order functions. When the closure is
statically known, PE can inline it at the call site — this is the functional
equivalent of function specialization. String interpolation with static parts can be
partially evaluated to reduce runtime concatenation.

### New Core Types

```
CExpr additions:
    A CClosure with params Seq of Text and body Seq of CStmt and captured Seq of Text.
    A CCallExpr with target CExpr and args Seq of CExpr.
    A CInterpolatedString with parts Seq of CStringPart.

Support type:
## A CStringPart is one of:
    A CLiteralPart with value Text.
    A CExprPart with expr CExpr.

CVal additions:
    A VClosure with params Seq of Text and body Seq of CStmt and capturedEnv Map of Text to CVal.
```

### Algorithm

1. Add `CClosure`, `CCallExpr`, `CInterpolatedString` to CExpr, `CStringPart` as new
   sum type, `VClosure` to CVal
2. `coreEval CClosure(params, body, captured)` → snapshot captured variables from
   current env into a `VClosure(params, body, capturedEnv)`
3. `coreEval CCallExpr(target, args)` → evaluate target (must be VClosure), evaluate
   args, create call env from captured env + param bindings, execute body
4. `coreEval CInterpolatedString(parts)` → iterate parts, evaluate each:
   - `CLiteralPart(s)` → append s to result
   - `CExprPart(expr)` → evaluate expr, convert to text via valToText, append
   Return `VText(result)`
5. `valToText VClosure(...)` → `"<closure>"`
6. `encode_program()` maps `Expr::Closure` → `CClosure`, `Expr::CallExpr` → `CCallExpr`,
   `Expr::InterpolatedString` → `CInterpolatedString`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 15 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Closure creation and invocation

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (7):**

1. `core_closure_basic`
   Source: CProgram with `CLet("f", CClosure(["x"], [CReturn(CBinOp("*", CVar("x"), CInt(2)))], []))`, call f(5), show result.
   Assert: Output is "10"

2. `core_closure_captured_var`
   Source: CProgram with `CLet("factor", CInt(3))`, closure capturing "factor", `CLet("f", CClosure(["x"], [CReturn(CBinOp("*", CVar("x"), CVar("factor")))], ["factor"]))`, call f(4).
   Assert: Output is "12"

3. `core_closure_pass_to_function`
   Source: CProgram with function `apply` taking a closure and a value, calling closure on value.
   Assert: Higher-order function works.

4. `core_closure_return_from_function`
   Source: CProgram with function returning a closure.
   Assert: Returned closure can be called.

5. `core_closure_multiple_params`
   Source: CProgram with closure taking two parameters.
   Assert: Both params bound correctly.

6. `core_closure_no_params`
   Source: CProgram with zero-param closure (thunk).
   Assert: Closure called with no args produces correct result.

7. `core_closure_to_text`
   Source: Show valToText of a VClosure.
   Assert: Output is "<closure>" or similar.

**Run:** `cargo test --test phase_futamura core_closure_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement closure types

Add VClosure, CClosure, CCallExpr. Implement closure creation (capture env snapshot)
and invocation (merge captured env with param bindings, execute body).

**Run:** `cargo test --test phase_futamura core_closure_ -- --skip e2e`
**Expected:** PASS (7 tests)

#### STEP 3: RED — String interpolation and encoding

**Tests (8):**

8. `core_interp_basic`
   Source: CProgram with `CLet("name", CText("World")), CShow(CInterpolatedString([CLiteralPart("Hello, "), CExprPart(CVar("name")), CLiteralPart("!")]))`.
   Assert: Output is "Hello, World!"

9. `core_interp_number`
   Source: CProgram with `CLet("n", CInt(42)), CShow(CInterpolatedString([CLiteralPart("Answer: "), CExprPart(CVar("n"))]))`.
   Assert: Output is "Answer: 42"

10. `core_interp_expression`
    Source: CProgram with interpolation containing arithmetic expression.
    Assert: Expression evaluated and interpolated correctly.

11. `core_interp_multiple_holes`
    Source: CProgram with interpolation containing three expressions.
    Assert: All expressions evaluated and concatenated.

12. `core_interp_empty_string`
    Source: CProgram with interpolation of only literal parts (no expressions).
    Assert: Simple string concatenation.

13. `core_closure_as_map_callback`
    Source: CProgram with closure used in a loop to transform each element.
    Assert: Closure applied to each element produces correct output.

14. `core_closure_nested`
    Source: CProgram with closure that creates and returns another closure.
    Assert: Inner closure captures outer closure's captured variables.

15. `core_closure_encode_roundtrip`
    Source: LogicAffeine program with closures and interpolated strings encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_interp_ core_closure_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement interpolation and encoding

Add CInterpolatedString, CStringPart. Implement string interpolation evaluation.
Extend `encode_program()`.

**Run:** `cargo test --test phase_futamura core_interp_ core_closure_ -- --skip e2e`
**Expected:** PASS (15 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 14

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 15 closure/interpolation tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 15 until this gate is green.

---

## Sprint 15 — Temporal Types: Duration, Date, Moment, Span, Time

### Overview

LogicAffeine provides first-class temporal types for duration arithmetic, date
manipulation, and time-span operations. The Core subset has no temporal awareness.
This sprint adds `VDuration`, `VDate`, `VMoment`, `VSpan`, and `VTime` as CVal
variants, along with constructor expressions and temporal arithmetic in `applyBinOp`.

Temporal types are primarily constructed via named constructors (e.g., `5 seconds`,
`today`) and manipulated via arithmetic operators (date + duration = date, moment
comparison, span containment). The self-interpreter models these as opaque values
with operator support.

**PE treatment:** Temporal constructors with static arguments (e.g., `5 seconds`) can
be evaluated at compile time. Temporal arithmetic on static values can be folded.
Operations that depend on the current time (`now`, `today`) are always Dynamic.

### New Core Types

```
CExpr additions:
    A CDuration with amount CExpr and unit Text.
    A CTimeNow.
    A CDateToday.

CVal additions:
    A VDuration with millis Int.
    A VDate with year Int and month Int and day Int.
    A VMoment with millis Int.
    A VSpan with start VMoment and end VMoment.
    A VTime with hour Int and minute Int and second Int.

applyBinOp additions:
    VDate + VDuration → VDate (date shifted by duration)
    VDate - VDate → VDuration (difference between dates)
    VMoment < VMoment → VBool (temporal comparison)
    VDuration + VDuration → VDuration (additive)
    VDuration * VInt → VDuration (scaling)
```

### Algorithm

1. Add temporal CExpr, CVal variants
2. `coreEval CDuration(amount, unit)` → evaluate amount (VInt), convert to millis
   based on unit ("seconds" → ×1000, "minutes" → ×60000, etc.), return `VDuration(millis)`
3. `coreEval CTimeNow` → `VMoment(current_millis)` — always Dynamic in PE
4. `coreEval CDateToday` → `VDate(year, month, day)` — always Dynamic in PE
5. `applyBinOp` for temporal arithmetic (see table above)
6. `valToText` for temporal values: VDuration → `"5s"`, VDate → `"2026-03-03"`, etc.
7. `encode_program()` maps temporal AST nodes to Core equivalents

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 12 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Duration and temporal construction

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (6):**

1. `core_temporal_duration_seconds`
   Source: CProgram with `CDuration(CInt(5), "seconds")`, show via valToText.
   Assert: Output represents 5-second duration.

2. `core_temporal_duration_minutes`
   Source: CProgram with `CDuration(CInt(3), "minutes")`, show via valToText.
   Assert: Output represents 3-minute duration.

3. `core_temporal_duration_add`
   Source: CProgram with two durations, add via CBinOp("+", d1, d2).
   Assert: Combined duration is correct.

4. `core_temporal_duration_multiply`
   Source: CProgram with duration × integer.
   Assert: Scaled duration is correct.

5. `core_temporal_date_construct`
   Source: CProgram constructing a VDate(2026, 3, 3), show via valToText.
   Assert: Output is formatted date.

6. `core_temporal_date_comparison`
   Source: CProgram comparing two dates.
   Assert: Comparison returns correct VBool.

**Run:** `cargo test --test phase_futamura core_temporal_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement temporal types

Add temporal CVal variants and CDuration expression. Implement arithmetic and valToText.

**Run:** `cargo test --test phase_futamura core_temporal_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 3: RED — Temporal arithmetic and encoding

**Tests (6):**

7. `core_temporal_date_add_duration`
   Source: CProgram with date + duration → new date.
   Assert: Date shifted correctly.

8. `core_temporal_date_difference`
   Source: CProgram with date - date → duration.
   Assert: Duration is correct difference.

9. `core_temporal_moment_comparison`
   Source: CProgram comparing two moments.
   Assert: Earlier moment < later moment → "true"

10. `core_temporal_time_construct`
    Source: CProgram constructing VTime(14, 30, 0), show via valToText.
    Assert: Output is formatted time.

11. `core_temporal_duration_to_text`
    Source: CProgram with various durations, show text representation.
    Assert: Human-readable duration strings.

12. `core_temporal_encode_roundtrip`
    Source: LogicAffeine program using temporal types encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_temporal_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement temporal arithmetic and encoding

Extend applyBinOp for date/duration arithmetic. Extend `encode_program()`.

**Run:** `cargo test --test phase_futamura core_temporal_ -- --skip e2e`
**Expected:** PASS (12 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 15

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 12 temporal tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 16 until this gate is green.

---

## Sprint 16 — IO, Sleep, Assertions, Ownership, Escape

### Overview

The Core subset's only IO operation is `CShow` (print to stdout). Real programs need
console input, file operations, sleep/delay, runtime assertions, ownership transfer,
and escape hatches to foreign code. This sprint adds `CReadConsole`, `CReadFile`,
`CWriteFile`, `CSleep`, `CRuntimeAssert`, `CGive`, and `CEscape` to the Core subset.

**Critical PE constraint:** All IO and side-effecting operations are **always Dynamic**
in PE. The partial evaluator never attempts to specialize them — they are emitted as
residual code unconditionally. The self-interpreter delegates these to the host
language's IO primitives.

### New Core Types

```
CExpr additions:
    A CEscapeExpr with code Text.

CStmt additions:
    A CReadConsole with target Text.
    A CReadFile with path CExpr and target Text.
    A CWriteFile with path CExpr and content CExpr.
    A CSleep with duration CExpr.
    A CRuntimeAssert with cond CExpr and msg CExpr.
    A CGive with expr CExpr and target Text.
    A CEscapeStmt with code Text.
```

### Algorithm

1. Add IO/effect CStmt variants and CEscapeExpr
2. `coreExecBlock CReadConsole(target)` → read a line from stdin, store as VText in env
3. `coreExecBlock CReadFile(path, target)` → read file contents, store as VText
4. `coreExecBlock CWriteFile(path, content)` → evaluate path and content, write to file
5. `coreExecBlock CSleep(duration)` → evaluate duration, sleep for that many milliseconds
6. `coreExecBlock CRuntimeAssert(cond, msg)` → evaluate cond, if false: halt with error message
7. `coreExecBlock CGive(expr, target)` → evaluate expr, store in target (ownership transfer —
   semantically identical to CLet in the interpreter, since the interpreter uses value
   semantics; the distinction matters only in compiled code)
8. `coreEval/coreExecBlock CEscapeExpr/CEscapeStmt` → in the self-interpreter, these
   are no-ops or errors (foreign code cannot be interpreted). The test verifies that
   `encode_program()` emits them and that PE treats them as Dynamic.
9. `encode_program()` maps `Stmt::ReadFrom` → `CReadConsole`/`CReadFile`,
   `Stmt::WriteFile` → `CWriteFile`, `Stmt::Sleep` → `CSleep`,
   `Stmt::RuntimeAssert` → `CRuntimeAssert`, `Stmt::Give` → `CGive`,
   `Stmt::Escape`/`Expr::Escape` → `CEscapeStmt`/`CEscapeExpr`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 12 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — IO operations

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (6):**

1. `core_io_runtime_assert_pass`
   Source: CProgram with `CRuntimeAssert(CBool(true), CText("should not fire"))`, `CShow(CText("ok"))`.
   Assert: Output is "ok" (assertion passed, no error)

2. `core_io_runtime_assert_fail`
   Source: CProgram with `CRuntimeAssert(CBool(false), CText("invariant broken"))`.
   Assert: Output contains error message.

3. `core_io_give`
   Source: CProgram with `CLet("x", CInt(42)), CGive(CVar("x"), "y"), CShow(CVar("y"))`.
   Assert: Output is "42" (ownership transferred)

4. `core_io_escape_stmt`
   Source: CProgram with `CEscapeStmt("foreign_code_here")`.
   Assert: Interpreter handles gracefully (no crash).

5. `core_io_escape_expr`
   Source: CProgram with `CLet("x", CEscapeExpr("foreign_expr")), CShow(CText("after"))`.
   Assert: Interpreter handles gracefully.

6. `core_io_write_and_read`
   Source: CProgram with `CWriteFile(CText("/tmp/core_test.txt"), CText("hello"))`, then `CReadFile(CText("/tmp/core_test.txt"), "contents"), CShow(CVar("contents"))`.
   Assert: Output is "hello" (round-trip through file IO)

**Run:** `cargo test --test phase_futamura core_io_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement IO operations

Add IO CStmt variants and CEscapeExpr. Implement coreExecBlock arms. For CEscapeExpr
and CEscapeStmt, return VNothing (foreign code cannot be interpreted). For file IO,
delegate to host language file operations.

**Run:** `cargo test --test phase_futamura core_io_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 3: RED — Sleep, console, and encoding

**Tests (6):**

7. `core_io_sleep`
   Source: CProgram with `CSleep(CInt(0))`, `CShow(CText("awake"))`.
   Assert: Output is "awake" (sleep completes, execution continues)

8. `core_io_assert_with_expression`
   Source: CProgram with `CRuntimeAssert(CBinOp(">", CInt(5), CInt(3)), CText("math works"))`, show "ok".
   Assert: Output is "ok"

9. `core_io_assert_dynamic_message`
   Source: CProgram with assert that evaluates message expression.
   Assert: Error message is dynamically computed.

10. `core_io_give_in_function`
    Source: CProgram with function that gives ownership of a computed value.
    Assert: Ownership transfer works across function boundaries.

11. `core_io_pe_treats_io_as_dynamic`
    Source: Verify that `encode_program()` marks IO operations. When PE encounters
    them, they remain in residual code unconditionally.
    Assert: IO operations never specialized away.

12. `core_io_encode_roundtrip`
    Source: LogicAffeine program with assertions and give encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_io_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement sleep, console, encoding

Add CSleep handling. Extend `encode_program()` for all IO-related AST nodes.

**Run:** `cargo test --test phase_futamura core_io_ -- --skip e2e`
**Expected:** PASS (12 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 16

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 12 IO tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 17 until this gate is green.

---

## Sprint 17 — Security Policies, Proofs, Require

### Overview

LogicAffeine's security and verification features include `Check` (mandatory security
predicate that can never be optimized away), `Assert` (logic kernel bridge), `Trust`
(documented assertion with justification), and `Require` (dependency declaration).
These are semantically distinct from runtime assertions — `Check` is a security gate,
`Assert`/`Trust` bridge to the logic kernel, and `Require` declares external
dependencies.

**PE treatment:** `CCheck` is **never optimized or specialized** — it must remain in
every residual program, regardless of static analysis. This is a security invariant.
`CAssert` and `CTrust` are logic kernel operations that PE preserves as residual.
`CRequire` is a compile-time declaration, not a runtime operation.

### New Core Types

```
CStmt additions:
    A CCheck with predicate CExpr and msg CExpr.
    A CAssert with proposition CExpr.
    A CTrust with proposition CExpr and justification Text.
    A CRequire with dependency Text.
```

### Algorithm

1. Add `CCheck`, `CAssert`, `CTrust`, `CRequire` to CStmt
2. `coreExecBlock CCheck(predicate, msg)` → evaluate predicate, if false: halt with
   security violation error. Unlike CRuntimeAssert, CCheck carries semantic weight:
   it represents a security policy that the program MUST enforce.
3. `coreExecBlock CAssert(proposition)` → evaluate proposition, if false: halt with
   logic assertion failure. This bridges to the formal verification layer.
4. `coreExecBlock CTrust(proposition, justification)` → evaluate proposition; if false,
   report failure with justification text. Trust is a weaker form of Assert with
   documented reasoning.
5. `coreExecBlock CRequire(dependency)` → no-op at runtime (dependency resolution
   happens at compile time). Included for encode_program completeness.
6. `encode_program()` maps `Stmt::Check` → `CCheck`, `Stmt::Assert` → `CAssert`,
   `Stmt::Trust` → `CTrust`, `Stmt::Require` → `CRequire`

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 8 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Security checks and assertions

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (4):**

1. `core_security_check_pass`
   Source: CProgram with `CCheck(CBool(true), CText("access denied"))`, `CShow(CText("granted"))`.
   Assert: Output is "granted"

2. `core_security_check_fail`
   Source: CProgram with `CCheck(CBool(false), CText("access denied"))`.
   Assert: Output contains "access denied" error.

3. `core_security_assert`
   Source: CProgram with `CAssert(CBinOp("==", CInt(2), CInt(2)))`, `CShow(CText("valid"))`.
   Assert: Output is "valid"

4. `core_security_trust`
   Source: CProgram with `CTrust(CBool(true), "well-known fact")`, `CShow(CText("trusted"))`.
   Assert: Output is "trusted"

**Run:** `cargo test --test phase_futamura core_security_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement security types

Add CCheck, CAssert, CTrust, CRequire. Implement coreExecBlock arms.

**Run:** `cargo test --test phase_futamura core_security_ -- --skip e2e`
**Expected:** PASS (4 tests)

#### STEP 3: RED — PE preservation and encoding

**Tests (4):**

5. `core_security_check_with_expression`
   Source: CProgram with CCheck using dynamic expression.
   Assert: Check evaluates expression correctly.

6. `core_security_require`
   Source: CProgram with CRequire("some_dep"), CShow(CText("loaded")).
   Assert: Output is "loaded" (require is no-op at runtime)

7. `core_security_check_never_eliminated`
   Source: Verify that CCheck is always Dynamic in PE context.
   Assert: Even with static predicate, CCheck remains in residual.

8. `core_security_encode_roundtrip`
   Source: LogicAffeine program with Check and Assert encoded via `encode_program()`.
   Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_security_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement PE preservation and encoding

Extend `encode_program()`. Mark CCheck as always-Dynamic in PE.

**Run:** `cargo test --test phase_futamura core_security_ -- --skip e2e`
**Expected:** PASS (8 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 17

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 8 security tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 18 until this gate is green.

---

## Sprint 18 — CRDTs

### Overview

LogicAffeine provides first-class Conflict-Free Replicated Data Types (CRDTs):
GCounter, PNCounter, ORSet, MVRegister, and RGA (Replicated Growable Array). The Core
subset has no CRDT support. This sprint adds `VCrdt` (CRDT runtime value) and the
mutation operations: `CMerge`, `CIncrease`, `CDecrease`, `CAppendToSequence`,
`CResolve`, `CSync`, and `CMount`.

**PE treatment:** All CRDT operations are side-effecting (they modify distributed
state). They are **always Dynamic** in PE and always appear in residual code.

### New Core Types

```
CStmt additions:
    A CMerge with target Text and other CExpr.
    A CIncrease with target Text and amount CExpr.
    A CDecrease with target Text and amount CExpr.
    A CAppendToSeq with target Text and value CExpr.
    A CResolve with target Text.
    A CSync with target Text and channel CExpr.
    A CMount with target Text and path CExpr.

CVal additions:
    A VCrdt with kind Text and state Map of Text to CVal.
```

### Algorithm

1. Add CRDT CStmt variants and VCrdt to CVal
2. `coreExecBlock CMerge(target, other)` → look up target and other in env (both
   VCrdt), merge their states according to CRDT semantics, update target in env
3. `coreExecBlock CIncrease(target, amount)` → look up target (VCrdt/GCounter),
   increment by amount
4. `coreExecBlock CDecrease(target, amount)` → look up target (VCrdt/PNCounter),
   decrement by amount
5. `coreExecBlock CAppendToSeq(target, value)` → append to RGA
6. `coreExecBlock CResolve(target)` → resolve MVRegister conflicts
7. `coreExecBlock CSync(target, channel)` → no-op in interpreter (networking)
8. `coreExecBlock CMount(target, path)` → no-op in interpreter (persistence)
9. `valToText VCrdt(kind, state)` → `"<crdt:kind>"`
10. `encode_program()` maps CRDT Stmt variants to Core equivalents

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 10 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — CRDT construction and basic operations

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (5):**

1. `core_crdt_gcounter_increase`
   Source: CProgram with VCrdt("GCounter", ...), CIncrease by 5, show state.
   Assert: Counter value is 5.

2. `core_crdt_pncounter`
   Source: CProgram with VCrdt("PNCounter", ...), CIncrease by 10, CDecrease by 3, show.
   Assert: Net value is 7.

3. `core_crdt_merge`
   Source: CProgram with two GCounters, CMerge, show merged state.
   Assert: Merged counter reflects max of each replica.

4. `core_crdt_rga_append`
   Source: CProgram with VCrdt("RGA", ...), CAppendToSeq with values, show.
   Assert: Sequence contains appended values.

5. `core_crdt_resolve`
   Source: CProgram with VCrdt("MVRegister", ...) in conflict, CResolve, show.
   Assert: Conflict resolved to single value.

**Run:** `cargo test --test phase_futamura core_crdt_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement CRDT types and operations

Add VCrdt and CRDT CStmt variants. Implement merge, increase, decrease, append,
resolve in the self-interpreter using map-based state representation.

**Run:** `cargo test --test phase_futamura core_crdt_ -- --skip e2e`
**Expected:** PASS (5 tests)

#### STEP 3: RED — Persistence, sync, and encoding

**Tests (5):**

6. `core_crdt_sync_noop`
   Source: CProgram with CSync (no-op in interpreter), CShow(CText("synced")).
   Assert: Output is "synced"

7. `core_crdt_mount_noop`
   Source: CProgram with CMount (no-op in interpreter), CShow(CText("mounted")).
   Assert: Output is "mounted"

8. `core_crdt_to_text`
   Source: Show valToText of VCrdt values.
   Assert: Readable representation.

9. `core_crdt_multiple_operations`
   Source: CProgram with sequence of increase/decrease/merge operations.
   Assert: Final state is correct.

10. `core_crdt_encode_roundtrip`
    Source: LogicAffeine program using CRDTs encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_crdt_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement sync, mount, encoding

CSync and CMount are no-ops in the interpreter (they require networking/persistence
infrastructure). Extend `encode_program()`.

**Run:** `cargo test --test phase_futamura core_crdt_ -- --skip e2e`
**Expected:** PASS (10 tests)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 18

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 10 CRDT tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 19 until this gate is green.

---

## Sprint 19 — Concurrency, Actors, Networking, Persistence, Zones

### Overview

This sprint covers the remaining ~20 Stmt variants: concurrency primitives
(Concurrent, Parallel, LaunchTask, Select, Sleep as async), pipe/channel operations
(CreatePipe, SendPipe, ReceivePipe, TrySendPipe, TryReceivePipe, StopTask), actor
model (Spawn, SendMessage, AwaitMessage, LetPeerAgent), networking (Listen, ConnectTo),
persistence (Zone, Mount), and dependency management (FunctionDef as runtime
registration).

**PE treatment:** Every operation in this sprint is side-effecting. They are **all
always Dynamic** in PE. The self-interpreter either delegates to host language
primitives (for concurrency/IO) or treats them as no-ops (for networking/actors in
the interpreter context). The key requirement is that `encode_program()` correctly
maps them and PE never attempts to specialize them.

### New Core Types

```
CStmt additions — Concurrency:
    A CConcurrent with branches Seq of Seq of CStmt.
    A CParallel with branches Seq of Seq of CStmt.
    A CLaunchTask with body Seq of CStmt and handle Text.
    A CStopTask with handle CExpr.
    A CSelect with branches Seq of CSelectBranch.

CStmt additions — Pipes:
    A CCreatePipe with name Text and capacity CExpr.
    A CSendPipe with pipe Text and value CExpr.
    A CReceivePipe with pipe Text and target Text.
    A CTrySendPipe with pipe Text and value CExpr.
    A CTryReceivePipe with pipe Text and target Text.

CStmt additions — Actors:
    A CSpawn with agentType Text and target Text.
    A CSendMessage with target CExpr and msg CExpr.
    A CAwaitMessage with target Text.

CStmt additions — Networking/Zones:
    A CListen with addr CExpr and handler Text.
    A CConnectTo with addr CExpr and target Text.
    A CZone with name Text and kind Text and body Seq of CStmt.

Support type:
## A CSelectBranch is one of:
    A CSelectRecv with pipe Text and var Text and body Seq of CStmt.
    A CSelectTimeout with duration CExpr and body Seq of CStmt.
```

### Algorithm

1. Add all remaining CStmt variants
2. **Concurrency** — in the self-interpreter (which is single-threaded):
   - `CConcurrent(branches)` → execute each branch sequentially (simulated)
   - `CParallel(branches)` → execute each branch sequentially (simulated)
   - `CLaunchTask(body, handle)` → execute body immediately, store sentinel in handle
   - `CSelect(branches)` → execute first branch (simplified simulation)
3. **Pipes** — simulated with in-memory queues:
   - `CCreatePipe(name, capacity)` → create a VSeq in env as pipe buffer
   - `CSendPipe(pipe, value)` → push to pipe buffer
   - `CReceivePipe(pipe, target)` → pop from pipe buffer, store in target
4. **Actors/Networking/Zones** — no-ops in interpreter:
   - `CSpawn`, `CSendMessage`, `CAwaitMessage` → no-op or simple simulation
   - `CListen`, `CConnectTo` → no-op
   - `CZone(name, kind, body)` → execute body (zone is transparent in interpreter)
5. `encode_program()` maps all remaining Stmt variants to their Core equivalents

### Modified Files

- `crates/logicaffeine_tests/tests/phase_futamura.rs` — Core type defs + 15 tests
- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — self-interpreter extensions
- `crates/logicaffeine_compile/src/compile.rs` — `encode_program()` extensions

### TDD Steps

#### STEP 1: RED — Concurrency and pipes

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (6):**

1. `core_concurrent_sequential`
   Source: CProgram with CConcurrent([[CShow(CText("a"))], [CShow(CText("b"))]]).
   Assert: Output contains both "a" and "b".

2. `core_parallel_sequential`
   Source: CProgram with CParallel([[CShow(CText("x"))], [CShow(CText("y"))]]).
   Assert: Output contains both "x" and "y".

3. `core_launch_task`
   Source: CProgram with CLaunchTask([CShow(CText("task"))], "h"), CShow(CText("main")).
   Assert: Output contains "task" and "main".

4. `core_pipe_send_receive`
   Source: CProgram with CCreatePipe("ch", CInt(10)), CSendPipe("ch", CInt(42)), CReceivePipe("ch", "val"), CShow(CVar("val")).
   Assert: Output is "42"

5. `core_pipe_multiple`
   Source: CProgram sending multiple values, receiving in order.
   Assert: FIFO ordering preserved.

6. `core_select_basic`
   Source: CProgram with CSelect containing one recv branch.
   Assert: Branch executes when data available.

**Run:** `cargo test --test phase_futamura core_concurrent_ core_parallel_ core_launch_ core_pipe_ core_select_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Implement concurrency and pipe simulation

Implement sequential simulation of concurrent/parallel. Implement pipe buffer as
VSeq in env with push/pop operations.

**Run:** `cargo test --test phase_futamura core_concurrent_ core_parallel_ core_launch_ core_pipe_ core_select_ -- --skip e2e`
**Expected:** PASS (6 tests)

#### STEP 3: RED — Actors, networking, zones, and encoding

**Tests (9):**

7. `core_spawn_noop`
   Source: CProgram with CSpawn, CShow(CText("spawned")).
   Assert: Output is "spawned" (spawn is no-op in interpreter)

8. `core_zone_transparent`
   Source: CProgram with CZone("z", "heap", [CShow(CText("inside"))]).
   Assert: Output is "inside" (zone body executes normally)

9. `core_listen_noop`
   Source: CProgram with CListen, CShow(CText("listening")).
   Assert: Output is "listening"

10. `core_connect_noop`
    Source: CProgram with CConnectTo, CShow(CText("connected")).
    Assert: Output is "connected"

11. `core_stop_task`
    Source: CProgram with CLaunchTask + CStopTask.
    Assert: No crash.

12. `core_try_send_receive`
    Source: CProgram with CTrySendPipe and CTryReceivePipe.
    Assert: Non-blocking operations work.

13. `core_send_message_noop`
    Source: CProgram with CSendMessage/CAwaitMessage.
    Assert: No crash, interpreter handles gracefully.

14. `core_pe_dynamic_all_effects`
    Source: Verify that ALL Sprint 19 operations are marked Dynamic in PE.
    Assert: None of these operations appear specialized in any residual.

15. `core_concurrent_encode_roundtrip`
    Source: LogicAffeine program using concurrency encoded via `encode_program()`.
    Assert: Self-interpreter produces correct output.

**Run:** `cargo test --test phase_futamura core_spawn_ core_zone_ core_listen_ core_connect_ core_stop_ core_try_ core_send_ core_pe_dynamic_ core_concurrent_encode_ -- --skip e2e`
**Expected:** FAIL for new tests

#### STEP 4: GREEN — Implement actors, networking, zones, encoding

Implement no-op handlers for actors and networking. CZone executes body transparently.
Extend `encode_program()` for all remaining Stmt variants.

**Run:** `cargo test --test phase_futamura -- --skip e2e`
**Expected:** PASS (all Sprint 19 tests, 15 total)

#### STEP 5: VERIFY — Full regression

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures.

### VERIFICATION GATE — Sprint 19

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 15 concurrency/actor/networking tests pass. All previous sprint tests still pass.
**Hard stop:** Do NOT proceed to Sprint 20 until this gate is green.

---

## Sprint 20 — Full Coverage Projection Verification

### Overview

Sprints 9-19 extended the Core subset from ~23% to ~100% of language features. Sprint
20 verifies that the extended Core subset correctly supports all three Futamura
projections with programs that exercise every new feature category. This is the
integration test sprint — no new Core types are added, only cross-cutting verification.

**What "full coverage" means:**
1. `encode_program()` handles every Expr and Stmt variant (no panics on unknown nodes)
2. The self-interpreter handles every CExpr, CStmt, and CVal variant
3. PE correctly classifies every operation as Static or Dynamic
4. Projection 1: pe(int, program) works for programs using all feature categories
5. All three projections produce semantically equivalent results for extended programs

### TDD Steps

#### STEP 1: RED — encode_program completeness

**File:** `crates/logicaffeine_tests/tests/phase_futamura.rs` (append)

**Tests (5):**

1. `full_encode_every_expr`
   Source: LogicAffeine program using one of each Expr variant (literal, binop, call,
   index, slice, copy, range, list, tuple, field access, new, new variant, closure,
   call expr, interpolated string, contains, union, intersection, option some/none,
   escape, with capacity, length, not, map get).
   Assert: `encode_program()` does not panic. All nodes encoded.

2. `full_encode_every_stmt`
   Source: LogicAffeine program using one of each Stmt variant.
   Assert: `encode_program()` does not panic. All nodes encoded.

3. `full_interpreter_every_cexpr`
   Source: CProgram exercising every CExpr variant through the self-interpreter.
   Assert: No crashes. All expressions evaluate to valid CVal.

4. `full_interpreter_every_cstmt`
   Source: CProgram exercising every CStmt variant through the self-interpreter.
   Assert: No crashes. All statements execute correctly.

5. `full_interpreter_every_cval`
   Source: CProgram creating every CVal variant, converting via valToText, showing.
   Assert: All CVal types have text representation.

**Run:** `cargo test --test phase_futamura full_ -- --skip e2e`
**Expected:** FAIL

#### STEP 2: GREEN — Fix any encoding or interpreter gaps

Fix any missing arms in `encode_program()`, `coreEval`, `coreExecBlock`, `valToText`,
or `applyBinOp` discovered by completeness tests.

**Run:** `cargo test --test phase_futamura full_ -- --skip e2e`
**Expected:** PASS (5 tests)

#### STEP 3: RED — Extended Projection 1 tests

**Tests (5):**

6. `full_p1_struct_program`
   Source: Program with struct construction, field access, field mutation. Encode
   and run through Projection 1.
   Assert: Residual has no interpretive overhead. E2E output correct.

7. `full_p1_enum_program`
   Source: Program with enum construction and Inspect. Projection 1.
   Assert: Static Inspect arms eliminated. E2E output correct.

8. `full_p1_closure_program`
   Source: Program with closures and higher-order functions. Projection 1.
   Assert: Static closures inlined. E2E output correct.

9. `full_p1_iteration_program`
   Source: Program with for-each, for-range, break. Projection 1.
   Assert: Static ranges unrolled. E2E output correct.

10. `full_p1_mixed_features`
    Source: Program combining structs + enums + closures + iteration + sets.
    Assert: All features work together through Projection 1.

**Run:** `cargo test --test phase_futamura full_p1_ -- --skip e2e`
**Expected:** FAIL

#### STEP 4: GREEN — Fix Projection 1 for extended features

Ensure encode_program, PE, and the self-interpreter all handle the extended feature
set correctly in the Projection 1 pipeline.

**Run:** `cargo test --test phase_futamura full_p1_ -- --skip e2e`
**Expected:** PASS (10 tests)

#### STEP 5: RED — Cross-projection equivalence

**Tests (5):**

11. `full_p1_p2_equivalence`
    Source: For 3 extended-feature programs: compare pe(int, P) output with
    interpret(pe(pe, int), P) output for 3 inputs each.
    Assert: All 9 pairs match.

12. `full_all_projections_struct`
    Source: Struct-heavy program through P1, P2, P3.
    Assert: All three projections produce identical output.

13. `full_all_projections_enum`
    Source: Enum-heavy program through P1, P2, P3.
    Assert: All three projections produce identical output.

14. `full_all_projections_closure`
    Source: Closure-heavy program through P1, P2, P3.
    Assert: All three projections produce identical output.

15. `full_dynamic_operations_preserved`
    Source: Program with IO, CRDT, concurrency operations.
    Assert: All Dynamic operations preserved in residual across all projections.

**Run:** `cargo test --test phase_futamura full_ -- --skip e2e`
**Expected:** FAIL

#### STEP 6: GREEN — Fix cross-projection equivalence

Fix any discrepancies between projections for extended features.

**Run:** `cargo test --test phase_futamura full_ -- --skip e2e`
**Expected:** PASS (15 tests)

#### STEP 7: RED — Comprehensive coverage verification

**Tests (5):**

16. `full_coverage_audit`
    Source: Programmatic check that every Expr variant in the AST has a corresponding
    CExpr variant and encode_program arm.
    Assert: No unmapped variants.

17. `full_coverage_stmt_audit`
    Source: Programmatic check that every Stmt variant has a corresponding CStmt
    variant and encode_program arm.
    Assert: No unmapped variants.

18. `full_identity_extended`
    Source: Extended program with all-Dynamic inputs through pe(int, P).
    Assert: Identity property holds: pe(int, P_all_dynamic) ≡ P.

19. `full_regressions_all_sprints`
    Source: One representative program from each sprint (9-19).
    Assert: All produce correct output through the self-interpreter.

20. `full_triple_equivalence_extended`
    Source: For 5 extended-feature programs × 3 inputs each:
    P1 output == P2 output == P3 output.
    Assert: All 15 triples match.

**Run:** `cargo test --test phase_futamura full_ -- --skip e2e`
**Expected:** FAIL

#### STEP 8: GREEN — Fix coverage and regression issues

**Run:** `cargo test --test phase_futamura full_ -- --skip e2e`
**Expected:** PASS (20 tests)

#### STEP 9: VERIFY — Full regression (FINAL)

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. Every test across all 20 sprints passes.

### VERIFICATION GATE — Sprint 20 (FINAL)

**Run:** `cargo test -- --skip e2e`
**Expected:** Zero failures. All 394 test functions pass (+ 19 verification gates confirmed).
The Core subset now covers 100% of language features. All three Futamura projections
work with programs exercising the full language.

---

## Sprint Calendar

| Sprint | What | Depends On | Tests | Key Deliverable |
|--------|------|------------|-------|-----------------|
| 1 | BTA | — | 33 | `bta.rs`, Division type, BtaCache (polyvariant) |
| 2 | Function Specialization | Sprint 1 | 35 | `partial_eval.rs`, pipeline change |
| 3a | Text Propagation | — | 12 | Fix in supercompile.rs, propagate.rs, ctfe.rs |
| 3b | Index/Slice Driving | — | 10 | Fix in supercompile.rs, peephole audit |
| 3c | Residual Code Generation | Sprint 1 | 12 | New AST construction in drive_expr/drive_stmt |
| 3d | Homeomorphic Embedding | — | 12 | `embeds()`, `generalize()`, configuration history |
| 3e | Identity / Perfect Residuals | Sprint 3c | 4 | `cleanup_identities()`, identity test |
| 4 | LOGOS-in-LOGOS Self-Interpreter | — | 35 | CExpr/CStmt/CVal+VError types, self-interpreter in LOGOS |
| 5 | Projection 1 | Sprints 2, 3, 4 | 23 | pe(int, program) = compiled, identity test |
| 6 | Self-Applicable PE | Sprint 4 | 12 | PE in LOGOS, quotation function |
| 7 | Projection 2 | Sprints 5, 6 | 15 | pe(pe, int) = compiler |
| 8 | Projection 3 | Sprint 7 | 18 | pe(pe, pe) = cogen, RPN interpreter, cross-projection equivalence |
| 9 | Float, Extended Operators | Sprint 4 | 15 | CFloat, VFloat, bitwise ops, encode_program extensions |
| 10 | Iteration | Sprint 9 | 18 | CRepeat, CRepeatRange, CBreak, CPop, CList, CRange, CSlice, CCopy |
| 11 | Sets, Options, Tuples | Sprint 10 | 15 | VSet, VOption, VTuple, CContains, CUnion, CIntersection, CAdd, CRemove |
| 12 | Structs, Fields | Sprint 11 | 15 | VStruct, CNew, CFieldAccess, CSetField, CStructDef |
| 13 | Enums, Pattern Matching | Sprint 12 | 18 | VVariant, CNewVariant, CInspect, CMatchArm, CEnumDef |
| 14 | Closures, HOF, Interpolation | Sprint 13 | 15 | VClosure, CClosure, CCallExpr, CInterpolatedString |
| 15 | Temporal Types | Sprint 9 | 12 | VDuration, VDate, VMoment, VSpan, VTime, temporal arithmetic |
| 16 | IO, Sleep, Assert, Escape | Sprint 10 | 12 | CReadConsole, CReadFile, CWriteFile, CSleep, CRuntimeAssert, CGive, CEscape |
| 17 | Security, Proofs, Require | Sprint 16 | 8 | CCheck, CAssert, CTrust, CRequire |
| 18 | CRDTs | Sprint 11 | 10 | VCrdt, CMerge, CIncrease, CDecrease, CAppendToSeq, CResolve |
| 19 | Concurrency, Actors, Net | Sprint 18 | 15 | CConcurrent, CLaunchTask, CCreatePipe, CSpawn, CZone + remaining Stmts |
| 20 | Full Coverage Verification | Sprint 19 | 20 | Cross-projection tests, coverage audits, identity property |

**Critical path (Sprints 1-8):** Sprint 1 → Sprint 2 → Sprint 5 → Sprint 7 → Sprint 8.
**Critical path (Sprints 9-20):** Sprint 9 → Sprint 10 → Sprint 11 → Sprint 12 → Sprint 13 → Sprint 14 → Sprint 20.
**Parallelizable (1-8):** Sprint 3 (a, b, d) independent of Sprint 1. Sprint 4 independent of Sprints 1-3. Sprint 6 after Sprint 4.
**Parallelizable (9-20):** Sprint 15 parallel with Sprint 10-14 (only needs Sprint 9). Sprint 16 parallel with Sprint 11-14. Sprint 17 after Sprint 16. Sprint 18 parallel with Sprint 12-14. Sprint 19 after Sprint 18.

---

## Appendix A — Complete Test Index

### Sprint 1: BTA (33 tests) — `phase_bta.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `bta_literal_int_static` | Int literal → S(42) |
| 2 | `bta_literal_float_static` | Float literal → S(3.14) |
| 3 | `bta_literal_bool_static` | Bool literal → S(true) |
| 4 | `bta_literal_text_static` | Text literal → S("hello") |
| 5 | `bta_literal_nothing_static` | Nothing → S(nothing) |
| 6 | `bta_identifier_tracks_division` | Identifier → division lookup |
| 7 | `bta_binop_static_static` | S op S → S |
| 8 | `bta_binop_static_dynamic` | S op D → D |
| 9 | `bta_binop_dynamic_dynamic` | D op D → D |
| 10 | `bta_not_static` | not S(true) → S(false) |
| 11 | `bta_length_is_dynamic` | Length → always D |
| 12 | `bta_index_is_dynamic` | Index → always D |
| 13 | `bta_if_static_true_only_then` | S(true) → then-branch only |
| 14 | `bta_if_static_false_only_else` | S(false) → else-branch only |
| 15 | `bta_if_dynamic_both_branches` | D condition → both branches, join |
| 16 | `bta_if_join_same_value` | S(v) ⊔ S(v) = S(v) |
| 17 | `bta_while_fixpoint_converges` | Fixpoint terminates in ≤2 iterations |
| 18 | `bta_nested_if` | Nested S/D conditions |
| 19 | `bta_all_static_args` | f(S,S) → return S |
| 20 | `bta_all_dynamic_args` | f(D,D) → return D |
| 21 | `bta_mixed_args` | f(S,D) → return D |
| 22 | `bta_recursive_static` | factorial(S(5)) → S(120) via SCC |
| 23 | `bta_mutual_recursion_scc` | SCC ordering, cycle detection |
| 24 | `bta_nested_call_chain` | f(g(3), h(x)) → analyze g,h first |
| 25 | `bta_mutable_s_to_d_transition` | Set makes S→D permanently |
| 26 | `bta_collection_params_always_d` | Seq/Map params → D |
| 27 | `bta_set_makes_dynamic` | Let S; Set D → D |
| 28 | `bta_branch_dynamic_condition` | D condition → both live |
| 29 | `bta_loop_static_bound` | S(10) bound → unrollable |
| 30 | `bta_loop_dynamic_bound` | D bound → preserve loop |
| 31 | `bta_polyvariant_different_sites` | Same fn, different call-site divisions → distinct BtaResults |
| 32 | `bta_polyvariant_cache_hit` | Same (fn, arg_bts) at two sites → BtaCache returns cached result |
| 33 | `bta_polyvariant_recursive_distinct` | Recursive fn with S(5) vs S(10) → two cached entries |

### Sprint 2: Function Specialization (35 tests) — `phase_partial_eval.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `pe_creates_specialized_function` | New function def emitted |
| 2 | `pe_static_params_removed` | Only D params in specialized fn |
| 3 | `pe_body_substituted` | S params replaced with values |
| 4 | `pe_specialized_name_format` | Name encodes static arg info |
| 5 | `pe_call_site_rewritten` | Call passes only D args |
| 6 | `pe_fold_runs_on_specialized_body` | Fold simplifies after substitution |
| 7 | `pe_dce_runs_on_specialized_body` | DCE removes dead branches |
| 8 | `pe_simplicity_check` | Reject if not simplified enough |
| 9 | `pe_same_key_reuses` | Same static args → reuse variant |
| 10 | `pe_different_key_creates_new` | Different statics → new variant |
| 11 | `pe_variant_limit_8` | Max 8 variants per function |
| 12 | `pe_embedding_terminates` | Embedding-based termination fires |
| 13 | `pe_interner_used_for_names` | Names properly interned |
| 14 | `pe_same_static_reuses` | Same value at two sites → one variant |
| 15 | `pe_different_static_creates` | Different values → different variants |
| 16 | `pe_cascading_specialization` | f_spec calls g → g also specialized |
| 17 | `pe_nested_specialization` | Nested calls reuse same variant |
| 18 | `pe_multiple_static_params` | f(S, D, S) → two params removed |
| 19 | `pe_impure_skipped` | IO function not specialized |
| 20 | `pe_io_preserved` | Show calls remain |
| 21 | `pe_collections_as_dynamic` | Seq params always D |
| 22 | `pe_self_referential_reuse` | Recursive call reuses own variant |
| 23 | `pe_no_specialize_all_dynamic` | All D → no specialization |
| 24 | `pe_after_propagate` | Propagated constants trigger PE |
| 25 | `pe_before_ctfe` | All-S → CTFE handles instead |
| 26 | `pe_fixpoint_terminates` | fold→propagate→PE loop terminates |
| 27 | `pe_constant_arg_fully_evaluated` | All-S → fully evaluated |
| 28 | `pe_pipeline_fold_interaction` | Fold runs on PE output |
| 29 | `pe_factorial_output` | E2E: "3628800" |
| 30 | `pe_branch_elimination_output` | E2E: correct after branch elimination |
| 31 | `pe_partial_specialization_output` | E2E: multiply(3, n) correct |
| 32 | `pe_recursive_memoization` | No exponential blowup |
| 33 | `pe_code_bloat_limit` | ≤8 variants, correct output |
| 34 | `pe_depth_limit_preserves_correctness` | Depth limit, still correct |
| 35 | `pe_simplicity_check_passes` | Branch elimination passes check |

### Sprint 3a: Text Propagation (12 tests) — `phase_supercompile.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `text_supercompile_propagates` | Text substituted by supercompiler |
| 2 | `text_propagate_constant_prop` | Propagation pass handles text |
| 3 | `text_ctfe_pure_function` | CTFE evaluates text functions |
| 4 | `text_ctfe_concat` | Compile-time string concatenation |
| 5 | `text_ctfe_compare` | Compile-time string comparison |
| 6 | `text_symbol_identity_preserved` | Same symbol, not new intern |
| 7 | `text_codegen_ownership` | No ownership errors after propagation |
| 8 | `text_cross_function_propagation` | Text propagated across functions |
| 9 | `text_ctfe_mixed_not_evaluated` | Dynamic text arg → not evaluated |
| 10 | `text_e2e_hello_alice` | E2E: "Hello, Alice" |
| 11 | `text_e2e_greeting_function` | E2E: "Hello, Bob!" |
| 12 | `text_propagate_multiple_uses` | E2E: "hi hi" |

### Sprint 3b: Index/Slice Driving (10 tests) — `phase_supercompile.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `index_driven_first_element` | item 1 of [10,20,30] → 10 |
| 2 | `index_driven_last_element` | item 3 → 30 |
| 3 | `index_out_of_bounds_preserved` | item 5 → runtime (no compile-time panic) |
| 4 | `index_after_push_tracks` | Push then index correct |
| 5 | `index_dynamic_collection_preserved` | Dynamic collection → runtime index |
| 6 | `index_dynamic_index_preserved` | Static collection, dynamic index → runtime |
| 7 | `slice_driving_basic` | Slice with known bounds resolves |
| 8 | `swap_pattern_still_detected` | Swap peephole still fires |
| 9 | `index_lowering_after_fold` | (2+1) folded then index resolves |
| 10 | `index_e2e_output` | E2E: "200" |

### Sprint 3c: Residual Code Generation (12 tests) — `phase_supercompile.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `residual_static_left` | S * D → new BinaryOp with literal left |
| 2 | `residual_static_right` | D * S → literal right |
| 3 | `residual_both_static` | S * S → fold to literal |
| 4 | `residual_nested_binary` | (S + D) * S → correct nesting |
| 5 | `residual_if_true_eliminated` | S(true) → only then-block |
| 6 | `residual_if_false_eliminated` | S(false) → only else-block |
| 7 | `residual_if_dynamic_preserved` | D → both branches in residual |
| 8 | `residual_while_false_eliminated` | S(false) → loop removed |
| 9 | `residual_while_dynamic_preserved` | D → loop preserved |
| 10 | `residual_call_all_dynamic` | All D → call preserved |
| 11 | `residual_call_mixed` | Mixed → specialized call |
| 12 | `residual_e2e_mixed_binary_op` | E2E: "24" |

### Sprint 3d: Homeomorphic Embedding (12 tests) — `phase_supercompile.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `embedding_self_check` | x ◁ x |
| 2 | `embedding_diving_check` | x ◁ f(x, y) |
| 3 | `embedding_coupling_check` | f(a,b) ◁ f(a+1,b+1) |
| 4 | `embedding_rejects_non_embedded` | f(a,b) ⋪ g(c) |
| 5 | `embedding_growing_detected` | grow(n+1) terminates |
| 6 | `msg_computation` | MSG(a+b, a+c) = a+?1 |
| 7 | `msg_precision` | MSG(f(3,x), f(3,y)) = f(3,?1) |
| 8 | `msg_replacement` | Generalized values removed from store |
| 9 | `embedding_depth_limit_64` | Hard depth limit fires |
| 10 | `embedding_tail_recursive_e2e` | E2E: "55" |
| 11 | `embedding_while_loop_e2e` | E2E: "5050" |
| 12 | `generalization_preserves_correctness` | Correct output after generalization |

### Sprint 3e: Identity / Perfect Residuals (4 tests) — `phase_supercompile.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `identity_trivial_program` | pe(int, Show 42 all-D) ≡ Show 42 |
| 2 | `identity_arithmetic_program` | pe(int, x+y all-D) ≡ x+y |
| 3 | `identity_control_flow_program` | pe(int, if/while all-D) ≡ original structure |
| 4 | `identity_function_call_program` | pe(int, f(x) all-D) ≡ f(x) |

### Sprint 4: LOGOS-in-LOGOS Self-Interpreter (35 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_eval_literal_int` | CInt(42) → "42" |
| 2 | `core_eval_literal_bool` | CBool(true) → "true" |
| 3 | `core_eval_literal_text` | CText("hello") → "hello" |
| 4 | `core_eval_literal_nothing` | Missing var → VNothing |
| 5 | `core_eval_variable` | CLet + CVar lookup |
| 6 | `core_eval_addition` | 2 + 3 → "5" |
| 7 | `core_eval_subtraction` | 10 - 3 → "7" |
| 8 | `core_eval_multiplication` | 4 * 5 → "20" |
| 9 | `core_eval_nested_arithmetic` | (2+3)*4 → "20" |
| 10 | `core_eval_comparison_operators` | All 6 comparisons |
| 11 | `core_eval_boolean_and` | true && false → "false" |
| 12 | `core_eval_boolean_or` | false \|\| true → "true" |
| 13 | `core_eval_if_true` | true → then-block |
| 14 | `core_eval_if_false` | false → else-block |
| 15 | `core_eval_nested_if` | Nested conditions |
| 16 | `core_eval_while_loop` | Sum 1..5 → "15" |
| 17 | `core_eval_function_call` | double(21) → "42" |
| 18 | `core_eval_recursive_factorial` | factorial(5) → "120" |
| 19 | `core_eval_recursive_fibonacci` | fib(10) → "55" |
| 20 | `core_eval_mutual_recursion` | isEven(4) → "true" |
| 21 | `core_eval_missing_function` | Unknown fn → "nothing" |
| 22 | `core_eval_push_and_index` | Push + index → "20" |
| 23 | `core_eval_push_multiple` | Multi-push + show all |
| 24 | `core_eval_set_index` | Set item at index |
| 25 | `core_eval_sequence_length` | Length → "3" |
| 26 | `core_eval_map_operations` | Map set/get → "42" |
| 27 | `core_eval_scoping_isolation` | callEnv is fresh |
| 28 | `core_eval_early_return_in_while` | Return breaks while |
| 29 | `core_eval_counter_loop` | i++ pattern → "5" |
| 30 | `core_eval_string_concat` | "Hello" + ", World!" |
| 31 | `core_eval_div_by_zero` | a / 0 → VError("division by zero") |
| 32 | `core_eval_mod_by_zero` | a % 0 → VError("modulo by zero") |
| 33 | `core_eval_index_out_of_bounds` | item 10 of [1,2,3] → VError |
| 34 | `core_eval_error_propagation_binop` | VError + x → VError (propagates) |
| 35 | `core_eval_error_in_show` | CShow(VError) → displays error message |

### Sprint 5: Projection 1 (23 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `p1_encode_roundtrip` | encode_program semantics-preserving |
| 2 | `p1_verifier_catches_violations` | verify_no_overhead rejects bad residual |
| 3 | `p1_no_inspect_on_cstmt` | No CStmt dispatch in residual |
| 4 | `p1_no_inspect_on_cexpr` | No CExpr dispatch in residual |
| 5 | `p1_no_core_constructors` | No Core type refs in residual |
| 6 | `p1_trivial_show` | E2E: "42" |
| 7 | `p1_arithmetic` | E2E: "8" |
| 8 | `p1_control_flow` | Static branch eliminated |
| 9 | `p1_while_loop` | E2E: "15" |
| 10 | `p1_multiple_functions` | Multiple CFunc resolved |
| 11 | `p1_factorial_5` | E2E: "120" |
| 12 | `p1_factorial_10` | E2E: "3628800" |
| 13 | `p1_sum_loop_100` | E2E: "5050" |
| 14 | `p1_fibonacci_0` | E2E: "0" |
| 15 | `p1_fibonacci_1` | E2E: "1" |
| 16 | `p1_dynamic_input_function` | Dynamic input passes through |
| 17 | `p1_fibonacci_dynamic` | E2E: input-dependent |
| 18 | `p1_equivalence_25_pairs` | 5 programs × 5 inputs match |
| 19 | `p1_compiled_has_direct_computation` | Residual uses +, *, If directly |
| 20 | `p1_dynamic_control_flow` | Dynamic condition preserved |
| 21 | `p1_strings_dynamic` | String ops through P1 |
| 22 | `p1_no_env_lookup` | No Map lookups in residual |
| 23 | `p1_identity_test` | pe(int, P_all_dynamic) ≡ P |

### Sprint 6: Self-Applicable PE (12 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `pe_source_parses` | PE source parses without errors |
| 2 | `pe_no_closures` | No closures in PE source |
| 3 | `pe_no_dynamic_fn_names` | All calls use literal fn names |
| 4 | `pe_quotation_idempotent` | quote_pe() deterministic |
| 5 | `pe_quotation_preserves_behavior` | PE-as-data == PE-as-code |
| 6 | `pe_self_encodes_correctly` | Encoded PE produces same residual |
| 7 | `pe_self_applicable_arithmetic` | Self-app on arithmetic program |
| 8 | `pe_self_applicable_control_flow` | Self-app on if/else |
| 9 | `pe_self_applicable_recursion` | Self-app on factorial |
| 10 | `pe_memoization_works` | Memo prevents infinite recursion |
| 11 | `pe_self_applicable_smoke` | PE(PE, trivial) matches direct |
| 12 | `pe_specializes_interpreter` | PE(int, fac) → no overhead |

### Sprint 7: Projection 2 (15 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `p2_no_pe_dispatch` | No peExpr/peStmt in compiler |
| 2 | `p2_no_bta_types` | No BTA data structures |
| 3 | `p2_has_program_manipulation` | Compiler references CExpr/CStmt |
| 4 | `p2_factorial_5` | E2E: "120" via compiler |
| 5 | `p2_fibonacci_10` | E2E: "55" via compiler |
| 6 | `p2_sum_50` | E2E: "1275" via compiler |
| 7 | `p2_gcd` | E2E: "4" via compiler |
| 8 | `p2_strings` | E2E: "Hello, World!" via compiler |
| 9 | `p2_matches_p1` | P1 and P2 produce same output |
| 10 | `p2_correct_for_all_inputs` | Multiple inputs correct |
| 11 | `p2_compiler_reusable` | Same compiler for multiple programs |
| 12 | `p2_depth_limit_sufficient` | PE(PE) terminates |
| 13 | `p2_produces_valid_cprogram` | Output is valid CProgram |
| 14 | `p2_produces_compiler` | Output processes programs |
| 15 | `p2_multiple_programs` | fac + fib + sum all work |

### Sprint 8: Projection 3 (18 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `p3_no_pe_self_reference` | No PE dispatch in cogen |
| 2 | `p3_valid_cprogram` | cogen is valid CProgram |
| 3 | `p3_core_compiler_matches_p2` | cogen(int) == P2 compiler |
| 4 | `p3_full_chain_factorial` | E2E: "3628800" through 3 levels |
| 5 | `p3_full_chain_fibonacci` | E2E: "55" through 3 levels |
| 6 | `p3_full_chain_sum` | E2E: "5050" through 3 levels |
| 7 | `p3_rpn_push_print` | RPN: "42" |
| 8 | `p3_rpn_add` | RPN: "7" |
| 9 | `p3_rpn_sub` | RPN: "7" |
| 10 | `p3_rpn_mul` | RPN: "6" |
| 11 | `p3_rpn_complex` | RPN: "10" |
| 12 | `p3_quotation_idempotent` | Stable encoding |
| 13 | `p3_consistency_all_projections` | P1 == P2 == P3 for all |
| 14 | `p3_different_interpreter` | Universality (RPN) |
| 15 | `p3_full_chain_fibonacci_dynamic` | Dynamic input through chain |
| 16 | `p3_cross_projection_byte_identical` | P1, P2, P3 residuals byte-identical for 5 programs |
| 17 | `p3_cogen_produces_identical_compiler` | interpret(cogen, int) == pe(pe, int) byte-identical |
| 18 | `p3_triple_equivalence_10_programs` | 10 programs × 5 inputs: P1==P2==P3 output |

### Sprint 9: Float, Extended Operators (15 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_float_literal` | CFloat(3.14) → "3.14" |
| 2 | `core_float_addition` | VFloat(1.5) + VFloat(2.5) → "4" |
| 3 | `core_float_multiplication` | VFloat(2.0) * VFloat(3.5) → "7" |
| 4 | `core_float_division` | VFloat(10.0) / VFloat(4.0) → "2.5" |
| 5 | `core_float_subtraction` | VFloat(5.0) - VFloat(2.5) → "2.5" |
| 6 | `core_float_comparison` | VFloat(3.14) > VFloat(2.71) → "bigger" |
| 7 | `core_float_int_promotion` | VInt(2) + VFloat(3.5) → "5.5" |
| 8 | `core_float_to_text` | valToText(VFloat(3.14)) correct |
| 9 | `core_float_div_by_zero` | VFloat / VFloat(0.0) → VError |
| 10 | `core_bitxor` | 5 ^ 3 → "6" |
| 11 | `core_shl` | 1 << 4 → "16" |
| 12 | `core_shr` | 16 >> 2 → "4" |
| 13 | `core_float_comparison_eq` | VFloat(1.0) == VFloat(1.0) → "eq" |
| 14 | `core_float_nested_arithmetic` | (2.0 + 3.0) * 4.0 → "20" |
| 15 | `core_float_encode_roundtrip` | encode_program Float → CFloat roundtrip |

### Sprint 10: Iteration (18 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_iter_list_literal` | CList([10, 20, 30]) → length 3 |
| 2 | `core_iter_range_expr` | CRange(1, 5) → 5 elements |
| 3 | `core_iter_range_empty` | CRange(5, 1) → empty |
| 4 | `core_iter_slice` | CSlice items 2..3 → 2 elements |
| 5 | `core_iter_copy` | CCopy preserves original |
| 6 | `core_iter_list_show_elements` | CList index access → correct element |
| 7 | `core_iter_repeat_basic` | For x in [1,2,3]: show x → "1\n2\n3" |
| 8 | `core_iter_repeat_accumulate` | For-each sum → "6" |
| 9 | `core_iter_repeat_empty` | For-each empty list → no output |
| 10 | `core_iter_repeat_range` | For i in 1..5: show i → "1\n2\n3\n4\n5" |
| 11 | `core_iter_nested_repeat` | Nested for loops → correct product |
| 12 | `core_iter_repeat_with_return` | Return inside for-each propagates |
| 13 | `core_iter_repeat_with_push` | For + push pattern |
| 14 | `core_iter_break_basic` | Break exits for-each loop |
| 15 | `core_iter_break_in_while` | Break exits while loop |
| 16 | `core_iter_pop` | Pop removes last element |
| 17 | `core_iter_pop_empty_error` | Pop empty → graceful handling |
| 18 | `core_iter_encode_repeat` | encode_program Repeat → CRepeat |

### Sprint 11: Sets, Options, Tuples (15 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_set_add_and_contains` | Add + contains → "true" |
| 2 | `core_set_remove` | Remove + contains → "false" |
| 3 | `core_set_union` | Union of two sets |
| 4 | `core_set_intersection` | Intersection of two sets |
| 5 | `core_set_no_duplicates` | Add same element twice → still one |
| 6 | `core_set_contains_not_found` | Contains missing → "false" |
| 7 | `core_option_some` | COptionSome(42) → "Some(42)" |
| 8 | `core_option_none` | COptionNone → "None" |
| 9 | `core_option_unwrap` | Inspect VOption → inner value |
| 10 | `core_tuple_create` | CTuple([1, "hello", true]) → 3-tuple |
| 11 | `core_tuple_index` | Tuple index access → correct element |
| 12 | `core_tuple_to_text` | valToText(VTuple) → formatted |
| 13 | `core_contains_in_seq` | Contains in VSeq → "true" |
| 14 | `core_contains_text_in_text` | Substring check → "true" |
| 15 | `core_set_encode_roundtrip` | encode_program sets → correct CProgram |

### Sprint 12: Structs, Fields (15 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_struct_new` | CNew("Point", [3, 4]) → field x = 3 |
| 2 | `core_struct_field_y` | Field access y → 4 |
| 3 | `core_struct_set_field` | CSetField mutation → new value |
| 4 | `core_struct_to_text` | valToText(VStruct) → formatted |
| 5 | `core_struct_nested` | Struct containing struct |
| 6 | `core_struct_pass_to_function` | Function takes struct param |
| 7 | `core_struct_multiple_types` | Two different struct types |
| 8 | `core_struct_field_missing` | Non-existent field → VNothing/VError |
| 9 | `core_struct_arithmetic_fields` | Compute from struct fields |
| 10 | `core_struct_in_sequence` | Sequence of structs → iterate + access |
| 11 | `core_struct_copy_semantics` | Value semantics verified |
| 12 | `core_struct_in_map` | Struct as map value |
| 13 | `core_struct_recursive` | Struct with Seq children (tree) |
| 14 | `core_struct_with_function` | Function returning new struct |
| 15 | `core_struct_encode_roundtrip` | encode_program structs → correct CProgram |

### Sprint 13: Enums, Pattern Matching (18 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_enum_new_variant` | CNewVariant("Shape", "Circle", [5.0]) |
| 2 | `core_enum_inspect_match` | Inspect Circle → radius extracted |
| 3 | `core_enum_inspect_second_arm` | Second arm matches |
| 4 | `core_enum_inspect_otherwise` | Otherwise fallback |
| 5 | `core_enum_no_field_variant` | Zero-field variant (e.g., None) |
| 6 | `core_enum_multiple_fields` | Multi-field variant bindings |
| 7 | `core_enum_nested_inspect` | Inspect variant containing variant |
| 8 | `core_enum_inspect_return` | Return from within Inspect arm |
| 9 | `core_enum_inspect_with_computation` | Area computation in match arm |
| 10 | `core_enum_in_sequence` | Seq of variants, iterate + inspect |
| 11 | `core_enum_variant_equality` | Compare two matching variants |
| 12 | `core_enum_recursive_type` | Recursive enum (Expr tree) |
| 13 | `core_enum_inspect_all_arms` | 4+ variants, one arm each |
| 14 | `core_enum_pass_to_function` | Variant as function parameter |
| 15 | `core_enum_construct_in_function` | Function returning variant |
| 16 | `core_enum_map_over_variants` | Transform Seq of variants |
| 17 | `core_enum_inspect_no_match` | No matching arm → graceful |
| 18 | `core_enum_encode_roundtrip` | encode_program enums → correct CProgram |

### Sprint 14: Closures, HOF, Interpolation (15 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_closure_basic` | Closure(["x"], [return x*2], []) → 10 |
| 2 | `core_closure_captured_var` | Closure captures "factor" from env |
| 3 | `core_closure_pass_to_function` | Higher-order: apply(closure, value) |
| 4 | `core_closure_return_from_function` | Function returns closure |
| 5 | `core_closure_multiple_params` | Two-param closure |
| 6 | `core_closure_no_params` | Zero-param thunk |
| 7 | `core_closure_to_text` | valToText(VClosure) → "<closure>" |
| 8 | `core_interp_basic` | "Hello, {name}!" → "Hello, World!" |
| 9 | `core_interp_number` | "Answer: {n}" → "Answer: 42" |
| 10 | `core_interp_expression` | Arithmetic in interpolation hole |
| 11 | `core_interp_multiple_holes` | Three expressions interpolated |
| 12 | `core_interp_empty_string` | Literal-only parts (no expressions) |
| 13 | `core_closure_as_map_callback` | Closure used in loop transform |
| 14 | `core_closure_nested` | Closure returning closure |
| 15 | `core_closure_encode_roundtrip` | encode_program closures → correct CProgram |

### Sprint 15: Temporal Types (12 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_temporal_duration_seconds` | CDuration(5, "seconds") → VDuration |
| 2 | `core_temporal_duration_minutes` | CDuration(3, "minutes") → VDuration |
| 3 | `core_temporal_duration_add` | VDuration + VDuration → combined |
| 4 | `core_temporal_duration_multiply` | VDuration * VInt → scaled |
| 5 | `core_temporal_date_construct` | VDate(2026, 3, 3) → formatted |
| 6 | `core_temporal_date_comparison` | Date < Date → VBool |
| 7 | `core_temporal_date_add_duration` | VDate + VDuration → shifted date |
| 8 | `core_temporal_date_difference` | VDate - VDate → VDuration |
| 9 | `core_temporal_moment_comparison` | VMoment < VMoment → VBool |
| 10 | `core_temporal_time_construct` | VTime(14, 30, 0) → formatted |
| 11 | `core_temporal_duration_to_text` | valToText(VDuration) → human-readable |
| 12 | `core_temporal_encode_roundtrip` | encode_program temporal → correct CProgram |

### Sprint 16: IO, Sleep, Assert, Escape (12 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_io_runtime_assert_pass` | Assert(true) → no error, continues |
| 2 | `core_io_runtime_assert_fail` | Assert(false) → error message |
| 3 | `core_io_give` | CGive ownership transfer |
| 4 | `core_io_escape_stmt` | CEscapeStmt → graceful handling |
| 5 | `core_io_escape_expr` | CEscapeExpr → graceful handling |
| 6 | `core_io_write_and_read` | Write + Read file roundtrip |
| 7 | `core_io_sleep` | CSleep(0) → continues execution |
| 8 | `core_io_assert_with_expression` | Dynamic predicate evaluation |
| 9 | `core_io_assert_dynamic_message` | Dynamic error message |
| 10 | `core_io_give_in_function` | Give across function boundary |
| 11 | `core_io_pe_treats_io_as_dynamic` | IO always Dynamic in PE |
| 12 | `core_io_encode_roundtrip` | encode_program IO → correct CProgram |

### Sprint 17: Security, Proofs, Require (8 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_security_check_pass` | CCheck(true) → no error |
| 2 | `core_security_check_fail` | CCheck(false) → security violation |
| 3 | `core_security_assert` | CAssert(2 == 2) → valid |
| 4 | `core_security_trust` | CTrust(true, "reason") → trusted |
| 5 | `core_security_check_with_expression` | Dynamic predicate in CCheck |
| 6 | `core_security_require` | CRequire → no-op at runtime |
| 7 | `core_security_check_never_eliminated` | CCheck always Dynamic in PE |
| 8 | `core_security_encode_roundtrip` | encode_program security → correct CProgram |

### Sprint 18: CRDTs (10 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_crdt_gcounter_increase` | GCounter +5 → value 5 |
| 2 | `core_crdt_pncounter` | PNCounter +10 -3 → value 7 |
| 3 | `core_crdt_merge` | Two GCounters merged |
| 4 | `core_crdt_rga_append` | RGA sequence append |
| 5 | `core_crdt_resolve` | MVRegister conflict resolution |
| 6 | `core_crdt_sync_noop` | CSync → no-op in interpreter |
| 7 | `core_crdt_mount_noop` | CMount → no-op in interpreter |
| 8 | `core_crdt_to_text` | valToText(VCrdt) → readable |
| 9 | `core_crdt_multiple_operations` | Sequence of increase/decrease/merge |
| 10 | `core_crdt_encode_roundtrip` | encode_program CRDTs → correct CProgram |

### Sprint 19: Concurrency, Actors, Net (15 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `core_concurrent_sequential` | CConcurrent simulated → both outputs |
| 2 | `core_parallel_sequential` | CParallel simulated → both outputs |
| 3 | `core_launch_task` | CLaunchTask → "task" and "main" |
| 4 | `core_pipe_send_receive` | Pipe send 42, receive → "42" |
| 5 | `core_pipe_multiple` | FIFO ordering preserved |
| 6 | `core_select_basic` | CSelect recv branch executes |
| 7 | `core_spawn_noop` | CSpawn → no-op in interpreter |
| 8 | `core_zone_transparent` | CZone body executes normally |
| 9 | `core_listen_noop` | CListen → no-op |
| 10 | `core_connect_noop` | CConnectTo → no-op |
| 11 | `core_stop_task` | CStopTask → no crash |
| 12 | `core_try_send_receive` | Non-blocking pipe ops |
| 13 | `core_send_message_noop` | Actor messages → no crash |
| 14 | `core_pe_dynamic_all_effects` | All Sprint 19 ops always Dynamic |
| 15 | `core_concurrent_encode_roundtrip` | encode_program concurrency → correct CProgram |

### Sprint 20: Full Coverage Verification (20 tests) — `phase_futamura.rs`

| # | Test Name | Verifies |
|---|-----------|----------|
| 1 | `full_encode_every_expr` | encode_program handles all Expr variants |
| 2 | `full_encode_every_stmt` | encode_program handles all Stmt variants |
| 3 | `full_interpreter_every_cexpr` | Self-interpreter handles all CExpr |
| 4 | `full_interpreter_every_cstmt` | Self-interpreter handles all CStmt |
| 5 | `full_interpreter_every_cval` | All CVal → valToText works |
| 6 | `full_p1_struct_program` | P1 with structs → no overhead |
| 7 | `full_p1_enum_program` | P1 with enums → static Inspect eliminated |
| 8 | `full_p1_closure_program` | P1 with closures → static inlined |
| 9 | `full_p1_iteration_program` | P1 with iteration → static unrolled |
| 10 | `full_p1_mixed_features` | P1 with all features combined |
| 11 | `full_p1_p2_equivalence` | P1 == P2 for extended programs |
| 12 | `full_all_projections_struct` | P1 == P2 == P3 for struct program |
| 13 | `full_all_projections_enum` | P1 == P2 == P3 for enum program |
| 14 | `full_all_projections_closure` | P1 == P2 == P3 for closure program |
| 15 | `full_dynamic_operations_preserved` | IO/CRDT/concurrency in residual |
| 16 | `full_coverage_audit` | Every Expr variant has CExpr mapping |
| 17 | `full_coverage_stmt_audit` | Every Stmt variant has CStmt mapping |
| 18 | `full_identity_extended` | pe(int, P_all_dynamic) ≡ P for extended P |
| 19 | `full_regressions_all_sprints` | One program per sprint (9-19) correct |
| 20 | `full_triple_equivalence_extended` | 5 programs × 3 inputs: P1==P2==P3 |

### Summary

| Sprint | Test File | Count |
|--------|-----------|-------|
| 1 (BTA) | `phase_bta.rs` | 33 |
| 2 (PE) | `phase_partial_eval.rs` | 35 |
| 3a (Text) | `phase_supercompile.rs` | 12 |
| 3b (Index) | `phase_supercompile.rs` | 10 |
| 3c (Residual) | `phase_supercompile.rs` | 12 |
| 3d (Embedding) | `phase_supercompile.rs` | 12 |
| 3e (Identity) | `phase_supercompile.rs` | 4 |
| 4 (Self-Interpreter) | `phase_futamura.rs` | 35 |
| 5 (P1) | `phase_futamura.rs` | 23 |
| 6 (Self-App) | `phase_futamura.rs` | 12 |
| 7 (P2) | `phase_futamura.rs` | 15 |
| 8 (P3) | `phase_futamura.rs` | 18 |
| 9 (Float/Ops) | `phase_futamura.rs` | 15 |
| 10 (Iteration) | `phase_futamura.rs` | 18 |
| 11 (Sets/Options/Tuples) | `phase_futamura.rs` | 15 |
| 12 (Structs) | `phase_futamura.rs` | 15 |
| 13 (Enums/Inspect) | `phase_futamura.rs` | 18 |
| 14 (Closures/Interp) | `phase_futamura.rs` | 15 |
| 15 (Temporal) | `phase_futamura.rs` | 12 |
| 16 (IO/Sleep/Assert) | `phase_futamura.rs` | 12 |
| 17 (Security/Proofs) | `phase_futamura.rs` | 8 |
| 18 (CRDTs) | `phase_futamura.rs` | 10 |
| 19 (Concurrency/Net) | `phase_futamura.rs` | 15 |
| 20 (Full Coverage) | `phase_futamura.rs` | 20 |
| **Total** | | **394** |
| + 19 verification gates | | **413** |

---

## Appendix B — File Manifest

### New Files Created

| Sprint | File | Purpose |
|--------|------|---------|
| 1 | `crates/logicaffeine_compile/src/optimize/bta.rs` | Binding-Time Analysis |
| 1 | `crates/logicaffeine_tests/tests/phase_bta.rs` | BTA tests |
| 2 | `crates/logicaffeine_compile/src/optimize/partial_eval.rs` | Function Specialization |
| 2 | `crates/logicaffeine_tests/tests/phase_partial_eval.rs` | PE tests |
| 4 | `crates/logicaffeine_tests/tests/phase_futamura.rs` | Self-interpreter + Projection tests |
| 6 | `crates/logicaffeine_compile/src/optimize/pe_source.logos` | PE written in LogicAffeine |

### Existing Files Modified

| Sprint | File | What Changes |
|--------|------|-------------|
| 1 | `optimize/mod.rs` | Add `pub mod bta;` |
| 2 | `optimize/mod.rs` | Add `pub mod partial_eval;`, insert PE into pipeline, add fixpoint loop |
| 2 | `optimize/supercompile.rs` | Remove `_` from `_interner` parameter |
| 3a | `optimize/supercompile.rs` | Remove `Value::Text(_) => {}` guard in `drive_expr()` |
| 3a | `optimize/propagate.rs` | Add `Literal::Text(_)` to `is_propagatable_literal()` |
| 3a | `optimize/ctfe.rs` | Add `Text(Symbol)` to Value enum, add conversions |
| 3b | `optimize/supercompile.rs` | Replace Index/Slice passthrough with driving |
| 3b | `codegen/peephole.rs` | Harden pattern matching (if needed) |
| 3c | `optimize/supercompile.rs` | Extend `drive_expr`/`drive_stmt` for residual AST construction |
| 3d | `optimize/supercompile.rs` | Add Configuration, History, `embeds()`, `generalize()` |
| 4+ | `compile.rs` | Add `encode_program()`, `verify_no_overhead()`, `quote_pe()` |
| 3a-d | `phase_supercompile.rs` | Append Sprint 3 tests |
| 9 | `phase_futamura.rs` | Core type defs: add CFloat, VFloat. Append 15 tests. |
| 9 | `pe_source.logos` | coreEval CFloat, applyBinOp float + bitwise, valToText VFloat |
| 9 | `compile.rs` | encode_program: Literal::Float → CFloat, BinaryOp::{BitXor,Shl,Shr} |
| 10 | `phase_futamura.rs` | Core type defs: add CList, CRange, CSlice, CCopy, CRepeat, CRepeatRange, CBreak, CPop. Append 18 tests. |
| 10 | `pe_source.logos` | coreEval CList/CRange/CSlice/CCopy, coreExecBlock CRepeat/CRepeatRange/CBreak/CPop |
| 10 | `compile.rs` | encode_program: Repeat, Break, Pop, List, Range, Slice, Copy |
| 11 | `phase_futamura.rs` | Core type defs: add VSet, VOption, VTuple, CContains, CUnion, CIntersection, COptionSome, COptionNone, CTuple, CAdd, CRemove. Append 15 tests. |
| 11 | `pe_source.logos` | coreEval set/option/tuple exprs, coreExecBlock CAdd/CRemove |
| 11 | `compile.rs` | encode_program: Contains, Union, Intersection, OptionSome/None, Tuple, Add, Remove |
| 12 | `phase_futamura.rs` | Core type defs: add VStruct, CNew, CFieldAccess, CSetField, CStructDef. Append 15 tests. |
| 12 | `pe_source.logos` | coreEval CNew/CFieldAccess, coreExecBlock CSetField/CStructDef |
| 12 | `compile.rs` | encode_program: New, FieldAccess, SetField, StructDef |
| 13 | `phase_futamura.rs` | Core type defs: add VVariant, CNewVariant, CInspect, CMatchArm, CEnumDef. Append 18 tests. |
| 13 | `pe_source.logos` | coreEval CNewVariant, coreExecBlock CInspect with CWhen/COtherwise |
| 13 | `compile.rs` | encode_program: NewVariant, Inspect (with match arms) |
| 14 | `phase_futamura.rs` | Core type defs: add VClosure, CClosure, CCallExpr, CInterpolatedString, CStringPart. Append 15 tests. |
| 14 | `pe_source.logos` | coreEval CClosure/CCallExpr/CInterpolatedString, closure capture/invoke |
| 14 | `compile.rs` | encode_program: Closure, CallExpr, InterpolatedString |
| 15 | `phase_futamura.rs` | Core type defs: add VDuration, VDate, VMoment, VSpan, VTime, CDuration, CTimeNow, CDateToday. Append 12 tests. |
| 15 | `pe_source.logos` | coreEval temporal exprs, applyBinOp temporal arithmetic, valToText temporal |
| 15 | `compile.rs` | encode_program: temporal AST nodes |
| 16 | `phase_futamura.rs` | Core type defs: add CReadConsole, CReadFile, CWriteFile, CSleep, CRuntimeAssert, CGive, CEscape. Append 12 tests. |
| 16 | `pe_source.logos` | coreExecBlock IO stmts, coreEval CEscapeExpr |
| 16 | `compile.rs` | encode_program: ReadFrom, WriteFile, Sleep, RuntimeAssert, Give, Escape |
| 17 | `phase_futamura.rs` | Core type defs: add CCheck, CAssert, CTrust, CRequire. Append 8 tests. |
| 17 | `pe_source.logos` | coreExecBlock security stmts |
| 17 | `compile.rs` | encode_program: Check, Assert, Trust, Require |
| 18 | `phase_futamura.rs` | Core type defs: add VCrdt, CMerge, CIncrease, CDecrease, CAppendToSeq, CResolve, CSync, CMount. Append 10 tests. |
| 18 | `pe_source.logos` | coreExecBlock CRDT stmts, valToText VCrdt |
| 18 | `compile.rs` | encode_program: all CRDT Stmt variants |
| 19 | `phase_futamura.rs` | Core type defs: add all concurrency/actor/networking/zone CStmt variants. Append 15 tests. |
| 19 | `pe_source.logos` | coreExecBlock concurrency/pipe/actor/zone stmts (simulated or no-op) |
| 19 | `compile.rs` | encode_program: all remaining Stmt variants |
| 20 | `phase_futamura.rs` | Append 20 full-coverage verification tests. |
| 20 | `compile.rs` | Fix any remaining encode_program gaps discovered by coverage audit |

---

## Invariants (Carry These Through Every Sprint)

1. **No test left behind.** Every sprint's tests must pass before proceeding. No
   temporary failures. No skipped tests. If a test fails, fix the implementation —
   never modify the test (CLAUDE.md Rule 4).

2. **Effect system is the safety gate.** Functions with IO, Write, SecurityCheck, or
   Diverge effects are NEVER specialized by PE. If the effect system says impure, PE
   skips it unconditionally.

3. **Memoization prevents blowup.** Every specialization pass uses memoization keyed on
   (function, static_args). Without memoization, recursive specialization is exponential.

4. **Embedding-based termination prevents divergence.** Both function specialization
   (Sprint 2) and supercompilation (Sprint 3d) share the same termination mechanism:
   homeomorphic embedding on a configuration history. Before each driving/specialization
   step, the current configuration is checked against all previous configurations via
   `embeds()`. If the current state is embedded in a predecessor, MSG generalizes and
   driving emits a residual. This replaces ad-hoc depth counters with a mathematically
   principled termination guarantee (Kruskal's tree theorem). Safety nets remain:
   driving depth ≤ 64, step limit ≤ 10,000 (CTFE), variant limit ≤ 8 per function,
   code size ≤ 2x original.

5. **Correctness over specialization quality.** Correct but unoptimized residual is
   acceptable. Fast but incorrect is a bug.

6. **Self-application is a test, not a requirement for Sprints 1-5.** Sprints 1-5
   deliver value independently. Sprints 6-8 are the theoretical capstone.

7. **The pipeline order is sacred.** `fold → propagate → partial_eval → ctfe → fold →
   cse → licm → closed_form → deforest → abstract_interp → dce → supercompile`.

8. **One approach per problem.** No Option A/B hedging. Each sub-sprint specifies THE
   approach.

9. **Cite the literature.** Every mathematical concept names its originating paper.
   Partial evaluation is a 50-year-old field — do not reinvent it.

10. **Identity property.** `pe(int, P_all_dynamic) ≡ P` — specializing an interpreter
    with respect to a program where ALL inputs are dynamic must produce a residual
    program semantically identical to the original. This is the zero-specialization
    baseline. Sprint 3e's `cleanup_identities` pass ensures perfect residuals by
    removing identity let-bindings (`Let x = x.`), collapsing single-arm inspects,
    and eliminating no-op env lookups. The identity test is the canary: if it fails,
    the PE is introducing overhead or losing information.

11. **IO and side effects are always Dynamic.** CReadConsole, CReadFile, CWriteFile,
    CSleep, CCheck, CMerge, CSync, CMount, CConcurrent, CParallel, CLaunchTask,
    CSendPipe, CReceivePipe, CSpawn, CSendMessage, CListen, CConnectTo — all
    side-effecting CStmt variants are **always Dynamic** in PE. The partial evaluator
    never attempts to specialize them. They appear in residual code unconditionally.
    The self-interpreter delegates them to host language primitives.

12. **Core subset completeness.** After Sprint 20, every `Expr` variant in the AST has
    a corresponding `CExpr` variant and `encode_program()` arm. Every `Stmt` variant
    has a corresponding `CStmt` variant. Every `CVal` variant has a `valToText` arm.
    No `encode_program()` call panics on unknown nodes. This is verified by the
    `full_coverage_audit` and `full_coverage_stmt_audit` tests.

---

## Agent TODO Checklist

Every cycle is exactly three steps: **RED** (write tests, run, confirm fail), **GREEN** (implement, run, confirm pass), **VERIFY GATE** (run broad suite, fix regressions if not green). No cycle proceeds until its VERIFY GATE is green.

---

### Sprint 1 — Binding-Time Analysis

#### 1.1 Expression Classification (Steps 1–3)

- [ ] **RED** — Write 12 tests in `phase_bta.rs` (tests 1–12: `bta_literal_int_static` through `bta_index_is_dynamic`). Run `cargo test --test phase_bta -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Create `optimize/bta.rs`. Add `pub mod bta;` to `optimize/mod.rs`. Implement `BindingTime`, `Division`, `BtaResult`, `analyze_expr()`, stub `analyze_function()`. Run `cargo test --test phase_bta -- --skip e2e`. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 1.2 Control Flow (Steps 4–6)

- [ ] **RED** — Append 6 tests to `phase_bta.rs` (tests 13–18: `bta_if_static_true_only_then` through `bta_nested_if`). Run `cargo test --test phase_bta -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Extend `analyze_function()` for `Stmt::If` (condition BT, branching, join), `Stmt::While` (fixed-point loop), nested control flow. Run `cargo test --test phase_bta -- --skip e2e`. Confirm PASS (18).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 1.3 Function Calls (Steps 7–9)

- [ ] **RED** — Append 6 tests to `phase_bta.rs` (tests 19–24: `bta_all_static_args` through `bta_nested_call_chain`). Run `cargo test --test phase_bta -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement `analyze_call()`, SCC integration from `analysis/callgraph.rs`, memoization via `BtaCache`, cycle detection (on-stack → assume return=D). Run `cargo test --test phase_bta -- --skip e2e`. Confirm PASS (24).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 1.4 Edge Cases + Polyvariant (Steps 10–12)

- [ ] **RED** — Append 9 tests to `phase_bta.rs` (tests 25–33: `bta_mutable_s_to_d_transition` through `bta_polyvariant_recursive_distinct`). Run `cargo test --test phase_bta -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Handle mutable S→D transition, collection params always D, static loop bounds (≤256 iterations), polyvariant cache keyed on `(function, arg_bts)`. Run `cargo test --test phase_bta -- --skip e2e`. Confirm PASS (33).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 1 complete: 33 BTA tests green.**

---

### Sprint 2 — Function Specialization

#### 2.1 Specialization Mechanics (Steps 1–3)

- [ ] **RED** — Create `phase_partial_eval.rs`. Write 8 tests (tests 1–8: `pe_creates_specialized_function` through `pe_simplicity_check`). Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Create `optimize/partial_eval.rs`. Add `pub mod partial_eval;` to `optimize/mod.rs`. Remove underscore from `_interner` in `supercompile.rs`. Implement `SpecRegistry`, `specialize_call()`, `specialize_stmts()`, BTA integration, fold+DCE on specialized bodies, simplicity check (0.8x). Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm PASS (8).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 2.2 Memoization (Steps 4–6)

- [ ] **RED** — Append 5 tests to `phase_partial_eval.rs` (tests 9–13: `pe_same_key_reuses` through `pe_interner_used_for_names`). Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement SpecKey `(function_sym, [Some(lit) if S else None])`, cache lookup, `variant_count` limit 8, embedding-based termination via `history: Vec<SpecKey>`, interner integration for fresh names. Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm PASS (13).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 2.3 Multiple Call Sites + Cascading (Steps 7–9)

- [ ] **RED** — Append 5 tests to `phase_partial_eval.rs` (tests 14–18: `pe_same_static_reuses` through `pe_multiple_static_params`). Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Walk all call sites (not just first). Cascading specialization: scan specialized bodies for further candidates. Multiple static params: substitute all S, keep only D. Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm PASS (18).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 2.4 Safety Guards (Steps 10–12)

- [ ] **RED** — Append 5 tests to `phase_partial_eval.rs` (tests 19–23: `pe_impure_skipped` through `pe_no_specialize_all_dynamic`). Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Effect check via `EffectEnv::function_is_pure()` (resolve Symbol → `&str` first). Collection params always D. Self-referential call detection via memo key match. Skip when all-D. Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm PASS (23).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_optimize -- --skip e2e`. Fix regressions if not green.

#### 2.5 Pipeline Integration + E2E (Steps 13–15)

- [ ] **RED** — Append 12 tests to `phase_partial_eval.rs` (tests 24–35: `pe_after_propagate` through `pe_simplicity_check_passes`). Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Insert `partial_eval::specialize_stmts` into pipeline after propagate, before CTFE. Fixpoint loop: `fold → propagate → PE` until stable (max 8 cycles). Change counter. All-S → delegate to CTFE. Run `cargo test --test phase_partial_eval -- --skip e2e`. Confirm PASS (35).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 2 complete: 35 PE + 33 BTA tests green.**

---

### Sprint 3a — Text Propagation + CTFE Text

#### 3a.1 Text Propagation (Steps 1–3)

- [ ] **RED** — Append 12 tests to `phase_supercompile.rs` (tests 1–12: `text_supercompile_propagates` through `text_propagate_multiple_uses`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — In `supercompile.rs`: change `Value::Text(_) => {}` to emit `Expr::Literal(Literal::Text(s))`. In `propagate.rs`: add `| Literal::Text(_)` to `is_propagatable_literal()`. In `ctfe.rs`: add `Text(Symbol)` to Value, add conversion paths. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green.

---

### Sprint 3b — Index/Slice Driving

#### 3b.1 Index Driving (Steps 1–3)

- [ ] **RED** — Append 10 tests to `phase_supercompile.rs` (tests 1–10: `index_driven_first_element` through `index_e2e_output`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — In `supercompile.rs`: replace `Expr::Index { .. } | Expr::Slice { .. } => expr` with driving logic — drive both operands, evaluate at compile time if both known (with bounds check), otherwise construct new node. Audit `codegen/peephole.rs` — verify swap/vec-fill patterns still fire. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (10).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green.

---

### Sprint 3c — Residual Code Generation

#### 3c.1 Residual Expressions (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_supercompile.rs` (tests 1–6: `residual_static_left` through `residual_if_false_eliminated`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Extend `drive_expr()` in `supercompile.rs`: BinaryOp with one literal → new node. If with literal condition → prune dead branch. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_supercompile -- --skip e2e`. Fix regressions if not green.

#### 3c.2 Residual Control Flow + Calls (Steps 3–5)

- [ ] **RED** — Append 6 tests to `phase_supercompile.rs` (tests 7–12: `residual_if_dynamic_preserved` through `residual_e2e_mixed_binary_op`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — While with literal-false → eliminate. Call with all-D → preserve, drive body. Call with mixed → Sprint 2 specialization. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green.

---

### Sprint 3e — Identity / Perfect Residuals

#### 3e.1 Identity Property (Steps 1–3)

- [ ] **RED** — Append 4 tests to `phase_supercompile.rs` (tests 1–4: `identity_trivial_program` through `identity_function_call_program`). Run `cargo test --test phase_supercompile identity_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement `cleanup_identities()` in `optimize/partial_eval.rs`: remove identity let-bindings, collapse single-arm inspects, remove no-op env lookups, collapse `Let x = lit. Return x.` → `Return lit.`, iterate to fixpoint (max 4). Run `cargo test --test phase_supercompile identity_ -- --skip e2e`. Confirm PASS (4).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green.

---

### Sprint 3d — Homeomorphic Embedding + Generalization

#### 3d.1 Embedding Detection (Steps 1–2)

- [ ] **RED** — Append 5 tests to `phase_supercompile.rs` (tests 1–5: `embedding_self_check` through `embedding_growing_detected`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement `embeds(e1, e2) -> bool` (coupling + diving rules), `Configuration` struct, `History` stack. Push configuration before each driving step, check against predecessors. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (5).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_supercompile -- --skip e2e`. Fix regressions if not green.

#### 3d.2 MSG + Generalization (Steps 3–4)

- [ ] **RED** — Append 4 tests to `phase_supercompile.rs` (tests 6–9: `msg_computation` through `embedding_depth_limit_64`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement `generalize(store1, store2) -> HashMap<Symbol, Value>` (MSG). When embedding detected: compute MSG, replace store with generalized version. Depth limit 64 safety net. Replace crude `collect_modified_vars_block` + `remove` with embedding-based approach. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (9).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_supercompile -- --skip e2e`. Fix regressions if not green.

#### 3d.3 E2E Correctness (Steps 5–7)

- [ ] **RED** — Append 3 tests to `phase_supercompile.rs` (tests 10–12: `embedding_tail_recursive_e2e` through `generalization_preserves_correctness`). Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Verify generalized code produces correct residual. Fix any issues in `supercompile.rs`. Run `cargo test --test phase_supercompile -- --skip e2e`. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 3 complete: 50 tests (12 text + 10 index + 12 residual + 4 identity + 12 embedding) green. All Sprint 1–2 tests still green.**

---

### Sprint 4 — LOGOS-in-LOGOS Self-Interpreter

#### 4.1 Test Harness (Steps 1–2)

- [ ] **RED** — Create `phase_futamura.rs`. Write 1 test (`core_eval_literal_int`): Core type defs + interpreter + Main constructing `CProgram([], [CShow(CInt(42))])`. Run `cargo test --test phase_futamura core_eval_literal_int -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Create test file with harness helper `run_interpreter_program()` (concatenates type defs + interpreter source + Main, passes to `assert_exact_output()`). Run `cargo test --test phase_futamura core_eval_literal_int -- --skip e2e`. Confirm PASS (1).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 4.2 Literals + Variables (Steps 3–4)

- [ ] **RED** — Append 4 tests to `phase_futamura.rs` (tests 2–5: `core_eval_literal_bool` through `core_eval_variable`). Run `cargo test --test phase_futamura -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix interpreter for all CExpr literal variants and CVar env lookup. Run `cargo test --test phase_futamura -- --skip e2e`. Confirm PASS (5).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 4.3 Arithmetic + Comparisons (Steps 5–6)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 6–10: `core_eval_addition` through `core_eval_comparison_operators`). Run `cargo test --test phase_futamura -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix `applyBinOp` for all arithmetic and comparison operators. Run `cargo test --test phase_futamura -- --skip e2e`. Confirm PASS (10).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 4.4 Boolean Logic + Control Flow (Steps 7–8)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 11–16: `core_eval_boolean_and` through `core_eval_while_loop`). Run `cargo test --test phase_futamura -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix `applyBinOp` for `&&`/`||`. Fix `coreExecStmt` for `CIf` (condition eval, branching) and `CWhile` (loop with env threading). Run `cargo test --test phase_futamura -- --skip e2e`. Confirm PASS (16).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 4.5 Function Calls + Recursion (Steps 9–10)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 17–21: `core_eval_function_call` through `core_eval_missing_function`). Run `cargo test --test phase_futamura -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix `coreEval` CCall arm (arg eval, func lookup, callEnv construction, body execution). Fix `coreExecStmt` CCallS arm. Ensure mutual recursion works via shared `funcs` map. Run `cargo test --test phase_futamura -- --skip e2e`. Confirm PASS (21).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 4.6 Collections + Error Propagation + Edge Cases (Steps 11–13)

- [ ] **RED** — Append 14 tests to `phase_futamura.rs` (tests 22–35: `core_eval_push_and_index` through `core_eval_error_in_show`). Run `cargo test --test phase_futamura -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix CPush/CSetIdx/CLen/CIndex/CMapSetS/CMapGet arms. Implement VError propagation in `applyBinOp` and `coreEval` CIndex. Fix `valToText(VError(msg))` → `"Error: " + msg`. Run `cargo test --test phase_futamura -- --skip e2e`. Confirm PASS (35).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 4 complete: 35 self-interpreter tests green. All previous sprints green.**

---

### Sprint 5 — Projection 1: `pe(int, program) = compiled`

#### 5.1 Harness Helpers (Steps 1–2)

- [ ] **RED** — Append 2 tests to `phase_futamura.rs` (tests 1–2: `p1_encode_roundtrip`, `p1_verifier_catches_violations`). Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement `encode_program(stmts, expr_arena, interner) -> &Expr` and `verify_no_overhead(stmts) -> Result<(), String>` in `compile.rs`. Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm PASS (2).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 5.2 Overhead Verification (Steps 3–4)

- [ ] **RED** — Append 3 tests to `phase_futamura.rs` (tests 3–5: `p1_no_inspect_on_cstmt` through `p1_no_core_constructors`). Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Wire up Projection 1 pipeline: parse interpreter → encode target program as static CProgram → run PE with program=Static, input=Dynamic → post-process with fold+dce → verify with `verify_no_overhead`. Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm PASS (5).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 5.3 Program Patterns (Steps 5–6)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 6–10: `p1_trivial_show` through `p1_multiple_functions`). Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix residual generation for trivial programs, arithmetic, control flow with static conditions, while loops, multiple function definitions. Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm PASS (10).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 5.4 Equivalence + Dynamic Input (Steps 7–8)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 11–17: `p1_factorial_5` through `p1_fibonacci_dynamic`). Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix equivalence for fully-static programs (factorial, fibonacci, sum). Fix dynamic-input programs (residual preserves dynamic computation, eliminates dispatch). Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm PASS (17).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 5.5 Comprehensive Equivalence + Identity (Steps 9–11)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 18–23: `p1_equivalence_25_pairs` through `p1_identity_test`). Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix 25-pair equivalence (5 programs × 5 inputs). Fix identity property test: `pe(int, P_all_dynamic) ≡ P` with `cleanup_identities`. Fix direct computation verification, no env lookups. Run `cargo test --test phase_futamura p1_ -- --skip e2e`. Confirm PASS (23).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 5 complete: 23 P1 tests green. All previous sprints green.**

---

### Sprint 6 — Self-Applicable Partial Evaluator

#### 6.1 PE Source + Constraints (Steps 1–2)

- [ ] **RED** — Append 3 tests to `phase_futamura.rs` (tests 1–3: `pe_source_parses`, `pe_no_closures`, `pe_no_dynamic_fn_names`). Run `cargo test --test phase_futamura pe_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Create `optimize/pe_source.logos` with complete PE in LogicAffeine (`peExpr`, `peStmt`, `peBlock`, `extractReturn` + helpers). Add `quote_pe()` to `compile.rs`. Run `cargo test --test phase_futamura pe_ -- --skip e2e`. Confirm PASS (3).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 6.2 Quotation Correctness (Steps 3–4)

- [ ] **RED** — Append 3 tests to `phase_futamura.rs` (tests 4–6: `pe_quotation_idempotent`, `pe_quotation_preserves_behavior`, `pe_self_encodes_correctly`). Run `cargo test --test phase_futamura pe_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix `quote_pe()` for deterministic, correct CProgram encoding. Ensure PE-as-data and PE-as-code produce identical residuals. Run `cargo test --test phase_futamura pe_ -- --skip e2e`. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 6.3 Self-Application (Steps 5–7)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 7–12: `pe_self_applicable_arithmetic` through `pe_specializes_interpreter`). Run `cargo test --test phase_futamura pe_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix PE source, quotation, and meta-interpretation for self-application. PE-as-CProgram must correctly partially evaluate arithmetic, control flow, recursion, and memoization targets. `pe_specializes_interpreter` must pass `verify_no_overhead`. Run `cargo test --test phase_futamura pe_ -- --skip e2e`. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 6 complete: 12 self-application tests green. All previous sprints green.**

---

### Sprint 7 — Projection 2: `pe(pe, int) = compiler`

#### 7.1 Compiler Structure (Steps 1–2)

- [ ] **RED** — Append 3 tests to `phase_futamura.rs` (tests 1–3: `p2_no_pe_dispatch`, `p2_no_bta_types`, `p2_has_program_manipulation`). Run `cargo test --test phase_futamura p2_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Wire up Projection 2 pipeline: encode PE as CProgram → encode interpreter as static CProgram → run PE on itself with interpreter as static input → verify structural properties (no PE dispatch, no BTA types, has CExpr/CStmt refs). Run `cargo test --test phase_futamura p2_ -- --skip e2e`. Confirm PASS (3).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 7.2 Compiler Correctness (Steps 3–4)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 4–8: `p2_factorial_5` through `p2_strings`). Run `cargo test --test phase_futamura p2_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix compiler correctness: `pe(pe, int)` → compiler → `interpret(compiler, P)` → compiled_P → `interpret(compiled_P, input)` → correct output. Run `cargo test --test phase_futamura p2_ -- --skip e2e`. Confirm PASS (8).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 7.3 Consistency + Reuse (Steps 5–7)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 9–15: `p2_matches_p1` through `p2_multiple_programs`). Run `cargo test --test phase_futamura p2_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix P1/P2 equivalence (same outputs for same programs/inputs). Fix compiler reuse (one compiler handles multiple programs). Fix depth limit, valid CProgram output, compiler-as-program-processor. Run `cargo test --test phase_futamura p2_ -- --skip e2e`. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 7 complete: 15 P2 tests green. All previous sprints green.**

---

### Sprint 8 — Projection 3: `pe(pe, pe) = compiler_generator`

#### 8.1 Compiler Generator Structure (Steps 1–2)

- [ ] **RED** — Append 3 tests to `phase_futamura.rs` (tests 1–3: `p3_no_pe_self_reference`, `p3_valid_cprogram`, `p3_core_compiler_matches_p2`). Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Wire up Projection 3 pipeline: `pe_as_data = quote_pe()` → `pe_as_static = quote_pe()` → `cogen = interpret(pe_as_data, { program: pe_as_static, input: Dynamic })` → verify structure (no PE self-reference, valid CProgram, matches P2 compiler). Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm PASS (3).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 8.2 Core Language Chain (Steps 3–4)

- [ ] **RED** — Append 3 tests to `phase_futamura.rs` (tests 4–6: `p3_full_chain_factorial`, `p3_full_chain_fibonacci`, `p3_full_chain_sum`). Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix full chain: `cogen → interpret(cogen, int) → interpret(compiler, P) → interpret(compiled, input)`. factorial(10) → "3628800", fibonacci(10) → "55", sum(100) → "5050". Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 8.3 RPN Universality (Steps 5–6)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 7–11: `p3_rpn_push_print` through `p3_rpn_complex`). Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Encode RPN interpreter as CProgram. `rpn_compiler = interpret(cogen, rpn_int)`. Compile and run RPN programs through the generated compiler. Handle `Pop` in Core subset. Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm PASS (11).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 8.4 Cross-Projection Consistency (Steps 7–9)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 12–18: `p3_quotation_idempotent` through `p3_triple_equivalence_10_programs`). Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Fix stable encoding. Fix cross-projection equivalence: P1 == P2 == P3 outputs for all programs/inputs. Fix byte-identical residuals across projections. Fix `interpret(cogen, int)` == `pe(pe, int)`. Fix 10-program × 5-input triple equivalence (50 comparisons). Run `cargo test --test phase_futamura p3_ -- --skip e2e`. Confirm PASS (18).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 8 complete. All 228 tests green (221 functions + 7 gates). All three Futamura projections achieved.**

---

### Sprint 9 — Float, Extended Operators

#### 9.1 Float Literals and Arithmetic (Steps 1–2)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 1–7: `core_float_literal` through `core_float_int_promotion`). Add CFloat to CExpr, VFloat to CVal in CORE_TYPES. Run `cargo test --test phase_futamura core_float_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Extend `coreEval` for CFloat(v) → VFloat(v). Extend `applyBinOp` for float arithmetic (+, -, *, /) and comparisons. Add int/float promotion: VInt(a) op VFloat(b) → VFloat. Extend `valToText` for VFloat. Run `cargo test --test phase_futamura core_float_ -- --skip e2e`. Confirm PASS (7).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 9.2 Bitwise Operators and Encoding (Steps 3–5)

- [ ] **RED** — Append 8 tests to `phase_futamura.rs` (tests 8–15: `core_float_to_text` through `core_float_encode_roundtrip`). Run `cargo test --test phase_futamura core_float_ core_bitxor core_shl core_shr -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Extend `applyBinOp` for "^" (bitwise XOR), "<<" (shl), ">>" (shr) on VInt. Float div-by-zero → VError. Extend `encode_program()` for Literal::Float → CFloat, BinaryOp::{BitXor, Shl, Shr}. Run tests. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 9 complete: 15 float/operator tests green.**

---

### Sprint 10 — Iteration

#### 10.1 List, Range, Slice, Copy (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 1–6: `core_iter_list_literal` through `core_iter_list_show_elements`). Add CList, CRange, CSlice, CCopy to CExpr in CORE_TYPES. Run `cargo test --test phase_futamura core_iter_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Extend `coreEval` for CList (evaluate items → VSeq), CRange (start..end inclusive → VSeq), CSlice (sub-sequence), CCopy (shallow copy). Run tests. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 10.2 For-Each and For-Range (Steps 3–4)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 7–13: `core_iter_repeat_basic` through `core_iter_repeat_with_push`). Add CRepeat, CRepeatRange to CStmt in CORE_TYPES. Run `cargo test --test phase_futamura core_iter_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Extend `coreExecBlock` for CRepeat (iterate VSeq, bind var, execute body) and CRepeatRange (iterate integers start..end). Handle CReturn propagation from loop body. Run tests. Confirm PASS (13).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 10.3 Break, Pop, and Encoding (Steps 5–7)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 14–18: `core_iter_break_basic` through `core_iter_encode_repeat`). Add CBreak, CPop to CStmt in CORE_TYPES. Run `cargo test --test phase_futamura core_iter_ -- --skip e2e`. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement CBreak signal propagation (short-circuit current loop). CPop: remove last from VSeq. Extend `encode_program()` for Repeat, Break, Pop, List, Range, Slice, Copy. Run tests. Confirm PASS (18).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 10 complete: 18 iteration tests green.**

---

### Sprint 11 — Sets, Options, Tuples

#### 11.1 Set Operations (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 1–6: `core_set_add_and_contains` through `core_set_contains_not_found`). Add VSet to CVal, CContains/CUnion/CIntersection to CExpr, CAdd/CRemove to CStmt. Run `cargo test --test phase_futamura core_set_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement VSet, CAdd, CRemove, CContains, CUnion, CIntersection in self-interpreter. Run tests. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 11.2 Options, Tuples, and Encoding (Steps 3–7)

- [ ] **RED** — Append 9 tests to `phase_futamura.rs` (tests 7–15: `core_option_some` through `core_set_encode_roundtrip`). Add VOption/VTuple to CVal, COptionSome/COptionNone/CTuple to CExpr. Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement VOption (present flag + inner value), VTuple (fixed-size items). CContains on VSeq (linear scan) and VText (substring). Extend `encode_program()`. Run tests. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 11 complete: 15 collection tests green.**

---

### Sprint 12 — Structs, Fields

#### 12.1 Struct Construction and Access (Steps 1–2)

- [ ] **RED** — Append 8 tests to `phase_futamura.rs` (tests 1–8: `core_struct_new` through `core_struct_field_missing`). Add VStruct to CVal, CNew/CFieldAccess to CExpr, CSetField/CStructDef to CStmt. Run `cargo test --test phase_futamura core_struct_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement VStruct (typeName + field map). CStructDef registers field ordering. CNew pairs field exprs with names. CFieldAccess looks up field. CSetField mutates field. Run tests. Confirm PASS (8).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 12.2 Struct Computation and Encoding (Steps 3–5)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 9–15: `core_struct_arithmetic_fields` through `core_struct_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Handle structs in sequences, maps, functions, recursive structures. Extend `encode_program()` for New, FieldAccess, SetField, StructDef. Run tests. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 12 complete: 15 struct tests green.**

---

### Sprint 13 — Enums, Pattern Matching

#### 13.1 Variant Construction and Inspect (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 1–6: `core_enum_new_variant` through `core_enum_multiple_fields`). Add VVariant to CVal, CNewVariant to CExpr, CInspect/CEnumDef to CStmt, CMatchArm type (CWhen/COtherwise). Run `cargo test --test phase_futamura core_enum_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement VVariant, CNewVariant in coreEval, CInspect in coreExecBlock (iterate arms, match variantName, bind fields, execute body). Run tests. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 13.2 Complex Pattern Matching (Steps 3–4)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 7–13: `core_enum_nested_inspect` through `core_enum_inspect_all_arms`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Handle nested Inspect, return propagation from arms, recursive variant types, multi-arm matching. Run tests. Confirm PASS (13).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 13.3 Encoding and Edge Cases (Steps 5–7)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 14–18: `core_enum_pass_to_function` through `core_enum_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Handle Inspect with no matching arm (graceful fallthrough). Extend `encode_program()` for NewVariant, Inspect. Run tests. Confirm PASS (18).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 13 complete: 18 enum/Inspect tests green.**

---

### Sprint 14 — Closures, HOF, Interpolation

#### 14.1 Closure Creation and Invocation (Steps 1–2)

- [ ] **RED** — Append 7 tests to `phase_futamura.rs` (tests 1–7: `core_closure_basic` through `core_closure_to_text`). Add VClosure to CVal, CClosure/CCallExpr to CExpr. Run `cargo test --test phase_futamura core_closure_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement VClosure (params + body + captured env). CClosure → snapshot captured vars. CCallExpr → merge captured env with param bindings, execute body. Run tests. Confirm PASS (7).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 14.2 String Interpolation and Encoding (Steps 3–5)

- [ ] **RED** — Append 8 tests to `phase_futamura.rs` (tests 8–15: `core_interp_basic` through `core_closure_encode_roundtrip`). Add CInterpolatedString/CStringPart to CExpr types. Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement CInterpolatedString (iterate parts, evaluate exprs, concat). Nested closures, closure-as-callback. Extend `encode_program()` for Closure, CallExpr, InterpolatedString. Run tests. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 14 complete: 15 closure/interpolation tests green.**

---

### Sprint 15 — Temporal Types

#### 15.1 Duration and Temporal Construction (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 1–6: `core_temporal_duration_seconds` through `core_temporal_date_comparison`). Add temporal CVal/CExpr variants. Run `cargo test --test phase_futamura core_temporal_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement VDuration, VDate, VMoment, VSpan, VTime. CDuration(amount, unit) → VDuration. Temporal comparisons. Run tests. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 15.2 Temporal Arithmetic and Encoding (Steps 3–5)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 7–12: `core_temporal_date_add_duration` through `core_temporal_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Extend applyBinOp for date+duration, date-date, duration+duration, duration*int. Extend `encode_program()` for temporal AST nodes. Run tests. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 15 complete: 12 temporal tests green.**

---

### Sprint 16 — IO, Sleep, Assert, Escape

#### 16.1 IO Operations (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 1–6: `core_io_runtime_assert_pass` through `core_io_write_and_read`). Add IO CStmt variants and CEscapeExpr. Run `cargo test --test phase_futamura core_io_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement CRuntimeAssert, CGive, CEscapeStmt/Expr (graceful no-op), CWriteFile, CReadFile, CReadConsole. Run tests. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 16.2 Sleep, Console, and Encoding (Steps 3–5)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 7–12: `core_io_sleep` through `core_io_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — Implement CSleep. Extend `encode_program()` for ReadFrom, WriteFile, Sleep, RuntimeAssert, Give, Escape. Verify IO always Dynamic in PE. Run tests. Confirm PASS (12).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 16 complete: 12 IO tests green.**

---

### Sprint 17 — Security, Proofs, Require

#### 17.1 Security Checks (Steps 1–2)

- [ ] **RED** — Append 4 tests to `phase_futamura.rs` (tests 1–4: `core_security_check_pass` through `core_security_trust`). Add CCheck, CAssert, CTrust, CRequire to CStmt. Run `cargo test --test phase_futamura core_security_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement CCheck (security predicate gate), CAssert (logic kernel bridge), CTrust (documented assertion), CRequire (no-op at runtime). Run tests. Confirm PASS (4).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 17.2 PE Preservation and Encoding (Steps 3–5)

- [ ] **RED** — Append 4 tests to `phase_futamura.rs` (tests 5–8: `core_security_check_with_expression` through `core_security_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — CCheck is always Dynamic in PE (security invariant). Extend `encode_program()` for Check, Assert, Trust, Require. Run tests. Confirm PASS (8).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 17 complete: 8 security tests green.**

---

### Sprint 18 — CRDTs

#### 18.1 CRDT Construction and Operations (Steps 1–2)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 1–5: `core_crdt_gcounter_increase` through `core_crdt_resolve`). Add VCrdt to CVal, CRDT CStmt variants. Run `cargo test --test phase_futamura core_crdt_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement VCrdt (kind + state map), CMerge, CIncrease, CDecrease, CAppendToSeq, CResolve. Run tests. Confirm PASS (5).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 18.2 Persistence, Sync, and Encoding (Steps 3–5)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 6–10: `core_crdt_sync_noop` through `core_crdt_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — CSync and CMount as no-ops. Extend `encode_program()` for all CRDT Stmt variants. Run tests. Confirm PASS (10).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 18 complete: 10 CRDT tests green.**

---

### Sprint 19 — Concurrency, Actors, Net

#### 19.1 Concurrency and Pipes (Steps 1–2)

- [ ] **RED** — Append 6 tests to `phase_futamura.rs` (tests 1–6: `core_concurrent_sequential` through `core_select_basic`). Add concurrency/pipe CStmt variants. Run `cargo test --test phase_futamura core_concurrent_ core_parallel_ core_launch_ core_pipe_ core_select_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Implement CConcurrent/CParallel (sequential simulation), CLaunchTask, CCreatePipe (VSeq buffer), CSendPipe/CReceivePipe (push/pop), CSelect (first branch). Run tests. Confirm PASS (6).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 19.2 Actors, Networking, Zones, and Encoding (Steps 3–5)

- [ ] **RED** — Append 9 tests to `phase_futamura.rs` (tests 7–15: `core_spawn_noop` through `core_concurrent_encode_roundtrip`). Run tests. Confirm FAIL for new tests.
- [ ] **GREEN** — CSpawn, CSendMessage, CAwaitMessage as no-ops. CListen, CConnectTo as no-ops. CZone executes body transparently. CStopTask, CTrySend/Recv. Extend `encode_program()` for all remaining Stmt variants. Verify all Sprint 19 ops always Dynamic in PE. Run tests. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 19 complete: 15 concurrency/actor/net tests green.**

---

### Sprint 20 — Full Coverage Verification

#### 20.1 Encoding Completeness (Steps 1–2)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 1–5: `full_encode_every_expr` through `full_interpreter_every_cval`). Run `cargo test --test phase_futamura full_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Fix any remaining gaps in `encode_program()`, `coreEval`, `coreExecBlock`, `valToText`, `applyBinOp`. Run tests. Confirm PASS (5).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 20.2 Extended Projection 1 (Steps 3–4)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 6–10: `full_p1_struct_program` through `full_p1_mixed_features`). Run `cargo test --test phase_futamura full_p1_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Ensure Projection 1 pipeline handles extended features: structs, enums, closures, iteration, sets. Run tests. Confirm PASS (10).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 20.3 Cross-Projection Equivalence (Steps 5–6)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 11–15: `full_p1_p2_equivalence` through `full_dynamic_operations_preserved`). Run `cargo test --test phase_futamura full_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Fix cross-projection discrepancies for extended features. Ensure IO/CRDT/concurrency operations preserved in all residuals. Run tests. Confirm PASS (15).
- [ ] **VERIFY GATE** — Run `cargo test --test phase_futamura -- --skip e2e`. Fix regressions if not green.

#### 20.4 Coverage Audit and Final Verification (Steps 7–9)

- [ ] **RED** — Append 5 tests to `phase_futamura.rs` (tests 16–20: `full_coverage_audit` through `full_triple_equivalence_extended`). Run `cargo test --test phase_futamura full_ -- --skip e2e`. Confirm FAIL.
- [ ] **GREEN** — Fix coverage gaps. Verify identity property for extended programs. Run all sprint regression programs. Triple equivalence for extended features. Run tests. Confirm PASS (20).
- [ ] **VERIFY GATE** — Run `cargo test -- --skip e2e`. Fix regressions if not green. **Sprint 20 complete. All 413 tests green (394 functions + 19 gates). Full language coverage achieved. All three Futamura projections work with the complete language.**
