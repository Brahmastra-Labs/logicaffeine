# The LogicAffeine Optimizer

## Vision

Every compiler optimizer is a rewriting system. Most compilers encode their rewrites as ad-hoc pattern matching — a grab bag of special cases. LogicAffeine is different. We already have:

- **A polynomial ring normalizer** (`kernel/ring.rs`) that canonicalizes arithmetic expressions
- **Congruence closure with E-graphs** (`kernel/cc.rs`) that discovers equalities between terms
- **A simplification engine** (`kernel/simp.rs`) that applies hypothesis-driven rewriting with fuel limits
- **Tactic combinators** (`kernel/reduction.rs`) that compose strategies: `then`, `orelse`, `try`, `repeat`, `first`
- **A tree-walking interpreter** (`compile/interpreter.rs`) that evaluates programs at runtime

The insight: **an optimizer IS a tactic system operating on programs instead of proofs.** Each optimization pass is a "tactic" that transforms an AST while preserving semantics. Tactic combinators compose these passes into a pipeline. The proof engine's rewrite machinery becomes the optimizer's transformation engine.

No other compiler has this foundation. We're not bolting optimizations onto a compiler — we're building an optimizer from the same formal reasoning infrastructure that powers our proof kernel.

---

## Architecture: Optimization as Tactic Application

### Kernel Tactic Combinators (already exist in `reduction.rs`)

| Combinator | Kernel | Optimizer Equivalent |
|---|---|---|
| `tact_then(t1, t2)` | Apply t1, then t2 to result | `fold` then `dce` — fold creates literals, DCE eliminates dead branches |
| `tact_orelse(t1, t2)` | Try t1, fallback to t2 | Try polynomial normalization, fallback to algebraic identity |
| `tact_try(t)` | Apply t, ignore failure | Attempt compile-time eval, skip if impure/timeout |
| `tact_repeat(t)` | Apply until fixpoint (max 100) | Iterate propagate→fold→dce until no changes |
| `tact_first([t1,..,tn])` | Try each until one succeeds | Try each simplification strategy on a BinaryOp |
| `tact_solve(t)` | Apply t, require success | — |

### Optimizer Tactics (what we build)

| Tactic | Inspired By | Operates On | What It Does |
|---|---|---|---|
| `fold` | `simp` arithmetic | `Expr` | Evaluate known operations to literals |
| `algebraic` | `ring` identities | `BinaryOp` | Simplify using identity/absorbing elements |
| `normalize` | `ring` canonicalization | `Expr` (integer arithmetic) | Canonicalize to polynomial form, reconstruct minimal |
| `propagate` | `simp` substitution | `Stmt[]` | Replace immutable variables with their known values |
| `compile_eval` | `auto` (try everything) | `Expr::Call` | Evaluate pure functions at compile time via interpreter |
| `dce` | `simp` on `Literal(Boolean)` | `Stmt[]` | Eliminate dead branches and unreachable code |
| `peephole` | Pattern rewriting | `Expr` | Double negation, strength reduction |

### Composed Pipeline (using combinator logic)

```
optimize = repeat(
    then(propagate,
    then(try(compile_eval),
    then(fold_and_normalize,
         dce)))
)
```

In Rust, this becomes the `optimize_program` function that runs the passes in sequence, with a second iteration to catch cascading opportunities.

---

## What We Have Today

**Three files in `crates/logicaffeine_compile/src/optimize/`:**

### `fold.rs` (264 lines) — Constant Folding

**Statement-level recursion:** `fold_stmt` walks into all block-bearing and expression-bearing Stmt variants — Let, Set, If, While, Repeat, FunctionDef, Show, Return, RuntimeAssert, Push, SetField, SetIndex, Call, Give, Inspect, Pop, Add, Remove, Zone, Concurrent, Parallel, WriteFile, SendMessage, IncreaseCrdt, DecreaseCrdt, Sleep, MergeCrdt.

**Expression-level folding:** `fold_expr` handles `BinaryOp` where **both** operands are `Literal` — Int arithmetic (+,-,*,/,%), Int comparisons, Float arithmetic, Bool &&/||/==/!=, Text concat. Uses arena allocation with pointer-equality optimization (`ptr::eq`).

**Gap:** `fold_expr` has `_ => expr` catch-all. Expressions inside `Call` args, `List` elements, `Index`, `OptionSome`, `Copy`, `Length`, `Contains`, `Range`, `Union`, `Intersection`, etc. are **never visited**.

**Gap:** No algebraic simplification — `x + 0`, `x * 1`, `x * 0`, `true && x`, `false || x` are not simplified.

### `dce.rs` (93 lines) — Dead Code Elimination

**Recursive block processing:** `dce_block` helper converts blocks to Vec, applies DCE, re-allocates via `stmt_arena`. Recurses into: If (with literal-condition inlining), While (with false-elimination), Repeat, FunctionDef, Zone, Concurrent, Parallel, Inspect.

**Gap:** No post-Return unreachable code elimination — code after `Return` in a block survives.

### `mod.rs` (17 lines) — Pipeline

Simple sequential: `fold → dce`. Passes `stmt_arena` to DCE.

### Tests

~40 tests in `phase_optimize.rs` covering: integer/float/boolean folding, nested arithmetic, division-by-zero safety, block recursion (if/while/repeat bodies), DCE for true/false/folded conditions, While-false elimination, DCE inside function bodies, float comparison safety, mixed-type safety.

---

## Phase 1: Deep Expression Recursion + Algebraic Simplification

**Tactic model:** Extend `fold` to be an `everywhere` combinator — apply to every sub-expression, not just top-level BinaryOp. Add `algebraic` as a `first()` fallback when literal folding doesn't apply.

### What Changes

**`optimize/fold.rs`** — Two modifications:

**1a. Deep expression recursion.** Replace the `_ => expr` catch-all in `fold_expr` with explicit arms for every Expr variant. Each arm recursively calls `fold_expr` on sub-expressions.

Variants to handle:
- `Call { function, args }` — fold each arg
- `List(elements)` — fold each element
- `Tuple(elements)` — fold each element
- `Index { collection, index }` — fold index
- `Slice { collection, start, end }` — fold start and end
- `Length(inner)` — fold inner
- `Contains { collection, element }` — fold both
- `OptionSome(inner)` — fold inner
- `Copy(inner)` — fold inner
- `Give(inner)` — fold inner
- `Range { start, end }` — fold both
- `Union { left, right }` / `Intersection { left, right }` — fold both
- `FieldAccess { object, .. }` — fold object
- `New { fields, .. }` — fold field values
- `NewVariant { fields, .. }` — fold field values

Use `ptr::eq` pattern from existing BinaryOp arm — only allocate new node if children changed.

**1b. Algebraic simplification.** Add `try_simplify_algebraic(op, left, right) -> Option<&'a Expr<'a>>` called when `try_fold_binary` returns None (not both literal). Pattern match on identity/absorbing elements:

```
x + 0 → x        0 + x → x       (additive identity)
x - 0 → x                         (subtractive identity)
x * 1 → x        1 * x → x       (multiplicative identity)
x * 0 → 0        0 * x → 0       (multiplicative absorbing)
x / 1 → x                         (division identity)
true && x → x    x && true → x   (boolean AND identity)
false && x → false                 (boolean AND absorbing)
false || x → x   x || false → x  (boolean OR identity)
true || x → true                   (boolean OR absorbing)
"" ++ x → x      x ++ "" → x     (concat identity)
```

Flow in `fold_expr` BinaryOp arm:
```
fold children → try_fold_binary (both literal)
             → try_simplify_algebraic (one literal identity/absorbing)
             → rebuild if changed
```

### RED Tests (~15)

```rust
// Deep expression recursion
fold_inside_function_call_args      // double(2 + 3) → double(5)
fold_inside_list_literal            // [1+1, 2+2, 3+3] → [2, 4, 6]
fold_inside_index_expression        // items[1+1] → items[2]
fold_inside_return_value            // Return 2 + 3 → Return 5

// Algebraic simplification — arithmetic
simplify_add_zero_right             // a + 0 → a
simplify_add_zero_left              // 0 + a → a
simplify_multiply_one               // a * 1 → a
simplify_multiply_one_left          // 1 * a → a
simplify_multiply_zero              // a * 0 → 0
simplify_subtract_zero              // a - 0 → a
simplify_divide_by_one              // a / 1 → a

// Algebraic simplification — boolean
simplify_and_true_identity          // true && a → a
simplify_and_false_absorbing        // false && a → false (DCE eliminates branch)
simplify_or_false_identity          // false || a → a
simplify_or_true_absorbing          // true || a → true (DCE eliminates branch)
```

### Files

- **Modify:** `crates/logicaffeine_compile/src/optimize/fold.rs`
- **Modify:** `crates/logicaffeine_tests/tests/phase_optimize.rs`

---

## Phase 2: Unreachable Code Elimination

**Tactic model:** Complete DCE with post-Return elimination. Everything after a `Return` statement in a block is unreachable.

### What Changes

**`optimize/dce.rs`** — Add post-Return truncation.

When iterating statements in `eliminate_dead_code`, after pushing a `Stmt::Return`, `break` — everything after is unreachable. Same logic in `dce_block`.

### RED Tests (~4)

```rust
dce_unreachable_after_return        // Return 42. Show "dead". → dead code eliminated
dce_unreachable_keeps_first_return  // Return 42. Return 99. → only first kept
dce_unreachable_inside_function     // Function with code after return → eliminated
dce_never_removes_check             // If true: Check... → Check preserved after DCE
```

### Files

- **Modify:** `crates/logicaffeine_compile/src/optimize/dce.rs`
- **Modify:** `crates/logicaffeine_tests/tests/phase_optimize.rs`

---

## Phase 3: Constant Propagation

**Tactic model:** This is the `simp` tactic's hypothesis substitution applied to imperative code. `simp` extracts `x = t` from hypotheses and substitutes; we extract `Let x be 5.` and substitute `x → 5` in subsequent expressions. Runs before fold so that propagated constants create new folding opportunities.

### What Changes

**New file: `optimize/propagate.rs`**

**3a. `propagate_constants(stmts, expr_arena, stmt_arena, interner) -> Vec<Stmt<'a>>`**

Single-pass forward walk maintaining `HashMap<Symbol, &'a Expr<'a>>` (the "environment" — like `simp`'s `Substitution`).

For each statement:
- `Let { var, value, mutable: false }` — substitute `value` using current env, then check if result is `Literal` or `Identifier` (that maps to a known value). If so, add `var → result` to env.
- `Set { target, .. }` — remove `target` from env (reassigned; safety belt).
- `FunctionDef` — process body with a **fresh** env (function params are unknown). Do NOT propagate from outer scope into function body.
- `If/While/Repeat` — process inner blocks with the current env, but constants defined inside blocks don't escape outward.
- All statement values/conditions — apply `substitute_expr(expr, env, arena)`.

**3b. `substitute_expr(expr, env, arena) -> &'a Expr<'a>`**

Walks the expression tree (same full recursion as Phase 1's `fold_expr`). When hitting `Identifier(sym)`, check env — if present, return the known literal. Otherwise recurse into sub-expressions.

**3c. Pipeline update in `mod.rs`:**
```rust
let stmts = propagate::propagate_constants(stmts, ...);  // substitute known values
let stmts = fold::fold_stmts(stmts, ...);                // fold newly-created literal pairs
let stmts = dce::eliminate_dead_code(stmts, ...);         // eliminate newly-dead branches
```

This mirrors the `simp` tactic's approach: substitute → evaluate → simplify.

### RED Tests (~10)

```rust
propagate_enables_folding           // Let x be 10. Let y be x + 5. → y = 15
propagate_chain                     // Let a be 3. Let b be a. Let c be b+1. → c = 4
propagate_does_not_touch_mutable    // Let mut x be 10. Set x to 20. → NOT propagated
propagate_bool_enables_dce          // Let debug be false. If debug: Show "d". → eliminated
propagate_text_enables_concat_fold  // Let g be "hello". Let m be g ++ " world". → "hello world"
propagate_into_call_args            // Let x be 42. show_val(x). → show_val(42)
propagate_respects_scope            // inner Let doesn't escape to outer scope
propagate_identifier_chain          // Let x be 5. Let y be x. Let z be y+1. → z=6
propagate_stops_at_reassignment     // Let x be 5. Set x to 10. Let y be x+1. → y NOT 6
propagate_multiple_constants        // Let a be 1. Let b be 2. Let c be a+b. → c = 3
```

### Files

- **Create:** `crates/logicaffeine_compile/src/optimize/propagate.rs`
- **Modify:** `crates/logicaffeine_compile/src/optimize/mod.rs`
- **Modify:** `crates/logicaffeine_tests/tests/phase_optimize.rs`

---

## Phase 4: Compile-Time Function Evaluation

**Tactic model:** This is the `auto` tactic — it tries a powerful strategy (interpreter evaluation) and falls back gracefully on failure. Like `tact_try(compile_eval)` — attempt evaluation, skip if impure or timeout.

The interpreter (`compile/interpreter.rs`) already evaluates LOGOS programs. We reuse it as a compile-time oracle for pure functions with known arguments. This is FRIEND_PLANS Stage 4: "Your compiler IS an interpreter that also emits code."

### What Changes

**New file: `optimize/compile_eval.rs`**

**4a. Purity analysis.** Scan all `FunctionDef` bodies to build `HashSet<Symbol>` of pure functions.

A function is **pure** if its body contains only:
- `Let`, `Set`, `Return`, `If`, `While`, `Repeat`, `Inspect`, `RuntimeAssert`
- `Call` to other pure functions (or itself — recursion is fine)

A function is **impure** if it contains ANY of:
- `Show`, `ReadFrom`, `WriteFile`, `Spawn`, `SendMessage`, `Sleep`, `Listen`, `ConnectTo`, `Mount`, `Sync`, `Check`, `Push`, `Pop`, `Add`, `Remove`, `Escape`

Handle mutual recursion with fixed-point iteration: start with all functions as "potentially pure", iterate removing impure ones until stable.

**4b. Step-limited evaluation.** Add to `interpreter.rs`:

```rust
pub fn evaluate_pure_call_sync(
    function_defs: &[Stmt<'_>],
    function_name: Symbol,
    args: Vec<RuntimeValue>,
    interner: &Interner,
    max_steps: u64,
) -> Option<RuntimeValue>
```

Uses a lightweight tokio runtime (`Runtime::new().block_on()`) to run the async interpreter synchronously. Adds a step counter checked on every statement execution — returns `None` if exceeded.

**4c. RuntimeValue → Expr conversion:**
- `Int(n)` → `Literal(Number(n))`
- `Float(f)` → `Literal(Float(f))`
- `Bool(b)` → `Literal(Boolean(b))`
- `Text(s)` → `Literal(Text(interner.intern(&s)))`
- Anything else → `None` (keep original call)

**4d. Integration pass:** For each `Expr::Call { function, args }`:
1. Is `function` in the pure set?
2. Are all `args` now `Literal` (after propagation)?
3. Attempt `evaluate_pure_call_sync` with step limit (10,000)
4. On success → replace Call with resulting Literal
5. On failure → keep original Call unchanged

**4e. Pipeline position:** After propagation (so args are propagated to literals), before fold+dce:
```rust
propagate → compile_eval → fold → dce
```

### RED Tests (~8)

```rust
compile_eval_pure_function          // square(5) → 25
compile_eval_recursive              // factorial(5) → 120
compile_eval_skips_impure           // greet("World") with Show inside → NOT evaluated
compile_eval_with_propagated_args   // Let a be 5. double(a) → double evaluated to 10
compile_eval_mutable_args_not_eval  // double(b) where b is mutable → NOT evaluated
compile_eval_fibonacci              // fib(10) → 55
compile_eval_multi_arg              // add(3, 4) → 7
compile_eval_timeout_protection     // Function with infinite loop → kept as call (not hung)
```

### Files

- **Create:** `crates/logicaffeine_compile/src/optimize/compile_eval.rs`
- **Modify:** `crates/logicaffeine_compile/src/interpreter.rs` (add step-limited sync mode)
- **Modify:** `crates/logicaffeine_compile/src/optimize/mod.rs`
- **Modify:** `crates/logicaffeine_tests/tests/phase_optimize.rs`

---

## Phase 5: Polynomial Expression Normalization

**Tactic model:** This is the `ring` tactic brought to the imperative optimizer. `kernel/ring.rs` already proves polynomial equalities by converting terms to canonical polynomial form and comparing. We adapt this to **simplify** expressions: convert `Expr` to polynomial, reconstruct the minimal Expr from canonical form.

This is where LogicAffeine becomes unique. No other English-to-code compiler normalizes arithmetic to canonical polynomial form. We already proved the algorithm works in the kernel — now we bring it to the optimizer.

### How `kernel/ring.rs` Works (what we're adapting)

```
Input:  Term tree (with add, sub, mul operations)
        ↓
Step 1: Reify — convert Term to Polynomial { terms: BTreeMap<Monomial, i64> }
        ↓
Step 2: Normalize — BTreeMap ensures canonical ordering, zero coefficients removed
        ↓
Step 3: Compare — structural equality IS semantic equality
```

The `Polynomial` and `Monomial` types use `BTreeMap` for deterministic ordering. This means two polynomials are equal if and only if they represent the same mathematical expression. Zero coefficients are automatically cleaned.

### What We Build (adapted for Expr)

**New file: `optimize/normalize.rs`**

**5a. `ExprPolynomial` and `ExprMonomial` types:**

```rust
struct ExprMonomial {
    /// Maps interned Symbol (variable name) → exponent.
    /// Empty map = constant term (1).
    vars: BTreeMap<Symbol, u32>,
}

struct ExprPolynomial {
    /// Maps monomials → integer coefficients.
    /// Zero coefficients removed automatically.
    terms: BTreeMap<ExprMonomial, i64>,
}
```

Directly mirrors `kernel/ring.rs` Polynomial/Monomial but uses `Symbol` instead of `i64` for variable indices. This avoids the need for a name→index mapping.

**5b. `reify_expr(expr) -> Option<ExprPolynomial>`:**
- `Literal(Number(n))` → `ExprPolynomial::constant(n)`
- `Identifier(sym)` → `ExprPolynomial::var(sym)`
- `BinaryOp { Add, l, r }` → `reify(l)?.add(&reify(r)?)`
- `BinaryOp { Subtract, l, r }` → `reify(l)?.sub(&reify(r)?)`
- `BinaryOp { Multiply, l, r }` → `reify(l)?.mul(&reify(r)?)`
- Anything else → `None` (bail — not polynomial. Division, modulo, calls, etc.)

**5c. `unreify(poly, arena, interner) -> &'a Expr<'a>`:**

Convert polynomial back to the **smallest possible** Expr tree:
- Zero polynomial → `Literal(Number(0))`
- Single constant → `Literal(Number(c))`
- Single `sym^1` with coeff 1 → `Identifier(sym)`
- Single `sym^1` with coeff c → `BinaryOp { Multiply, Literal(c), Identifier(sym) }`
- Multiple terms → chain of Add/Subtract (positive coefficients add, negative subtract)

Cost model: the unreified form must have **fewer or equal AST nodes** than the original, or we keep the original. This ensures normalization never makes code worse.

**5d. Integration:** Called from `fold_expr` as an additional strategy. When encountering a `BinaryOp` involving only Add/Subtract/Multiply over Literals and Identifiers:
1. Attempt `reify_expr` on the full expression
2. If successful, `unreify` the canonical polynomial
3. If the result has fewer nodes, use it
4. Otherwise keep the original

**Safety:** Only normalize **integer** arithmetic. Float arithmetic violates ring axioms due to IEEE 754 rounding (`(a+b)-b ≠ a` for floats). The `reify_expr` function returns `None` for `Literal(Float(..))`.

### What This Enables

```
a + b - b          →  a               (cancellation)
2*x + 3*x         →  5*x             (like-term collection)
(a + 1) * (a - 1) →  a*a - 1         (expansion to minimal form)
1 + a + 2 + b + 3 →  a + b + 6       (constant collection)
a - a              →  0               (self-cancellation)
a + a              →  2*a             (doubling)
(x+2)*3 + x       →  4*x + 6         (distribution + collection)
```

These simplifications emerge **automatically** from polynomial normalization. No special-case patterns needed — the ring axioms handle everything.

### RED Tests (~12)

```rust
normalize_cancel_addition           // a + b - b → a
normalize_combine_like_terms        // 2*a + 3*a → 5*a
normalize_distribute                // (a + 1) * (a - 1) → a*a - 1
normalize_collect_constants         // 1 + a + 2 + b + 3 → a + b + 6
normalize_self_subtract             // a - a → 0
normalize_double                    // a + a → 2*a
normalize_complex_cancel            // (a+b)*2 - a - b - (a+b) → 0
normalize_preserves_non_polynomial  // a / b NOT normalized (division not polynomial)
normalize_preserves_minimal         // a + b stays if already minimal
normalize_nested_with_literals      // (x + 2) * 3 + x → 4*x + 6
normalize_only_integer              // float expressions NOT normalized
normalize_wrapping_semantics        // i64 wrapping preserved
```

### Files

- **Create:** `crates/logicaffeine_compile/src/optimize/normalize.rs`
- **Modify:** `crates/logicaffeine_compile/src/optimize/fold.rs` (integrate normalization)
- **Modify:** `crates/logicaffeine_compile/src/optimize/mod.rs`
- **Modify:** `crates/logicaffeine_tests/tests/phase_optimize.rs`

---

## Phase 6: Peephole Rewriting + Strength Reduction

**Tactic model:** This is `tact_first([rule1, rule2, ...])` — a database of small rewrite rules tried in sequence. Each rule is a pattern match on the expression structure. If a rule matches, it fires. If not, try the next one.

These are the fine-grained rewrites that don't fit into algebraic simplification or polynomial normalization. They handle parser-specific encodings and micro-optimizations.

### What Changes

**Modify `optimize/fold.rs`** — add `try_peephole(expr) -> Option<&'a Expr<'a>>` called after algebraic simplification.

**6a. Double negation elimination.** The parser encodes `not x` as `BinaryOp { Eq, x, Literal(Boolean(false)) }`. So `not (not x)` is:
```
BinaryOp { Eq, BinaryOp { Eq, inner, Literal(Boolean(false)) }, Literal(Boolean(false)) }
```
Rewrite to `inner`.

**6b. Negation of literals.** Already mostly handled by `try_fold_binary`, but verify:
- `not true` → `false` (i.e., `true == false` → `false`)
- `not false` → `true` (i.e., `false == false` → `true`)

**6c. Strength reduction.** `x * 2` → `x + x`. Addition is cheaper than multiplication at the hardware level, and this form enables further polynomial normalization.

**6d. Self-identity patterns.** Only for `Identifier` (not complex expressions with potential side effects):
- `Identifier(s) == Identifier(s)` → `Literal(Boolean(true))`
- `Identifier(s) - Identifier(s)` → `Literal(Number(0))`

### RED Tests (~6)

```rust
double_negation_elimination         // not (not flag) → flag
negation_of_literal_true            // not true → false
negation_of_literal_false           // not false → true
strength_reduce_multiply_by_two     // a * 2 → a + a
identity_comparison                 // a == a → true (Identifier only)
subtract_self                       // a - a → 0 (Identifier only)
```

### Files

- **Modify:** `crates/logicaffeine_compile/src/optimize/fold.rs`
- **Modify:** `crates/logicaffeine_tests/tests/phase_optimize.rs`

---

## Final Pipeline

```rust
// optimize/mod.rs

pub fn optimize_program<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    // === First pass ===

    // Tactic: simp-style hypothesis substitution
    let stmts = propagate::propagate_constants(stmts, expr_arena, stmt_arena, interner);

    // Tactic: auto-style try-everything evaluation
    let stmts = compile_eval::evaluate_pure_calls(stmts, expr_arena, stmt_arena, interner);

    // Tactic: ring + algebraic + peephole (composed via tact_first)
    let stmts = fold::fold_stmts(stmts, expr_arena, stmt_arena, interner);

    // Tactic: dead branch + unreachable code elimination
    let stmts = dce::eliminate_dead_code(stmts, stmt_arena);

    // === Second pass (tact_repeat — catches cascading opportunities) ===
    let stmts = fold::fold_stmts(stmts, expr_arena, stmt_arena, interner);
    dce::eliminate_dead_code(stmts, stmt_arena)
}
```

This is `repeat(then(propagate, then(try(compile_eval), then(fold, dce))))` with max iterations = 2.

---

## Implementation Order

```
Phase 1  ─── Deep Expr Recursion + Algebraic ──┐
                                                │ independent
Phase 2  ─── Unreachable After Return ─────────┤
                                                │
Phase 6  ─── Peephole + Strength ──────────────┘

Phase 3  ─── Constant Propagation ──── (needs Phase 1 for cascading folds)
                                                │
Phase 4  ─── Compile-Time Eval ─────── (needs Phase 3 for arg propagation)
                                                │
Phase 5  ─── Polynomial Normalization ─ (needs Phase 1 foundation)
```

Phases 1, 2, 6 can be done in any order. Phase 3 needs Phase 1. Phase 4 needs Phase 3. Phase 5 needs Phase 1.

---

## New Files

| File | Phase | Lines (est.) | Purpose |
|---|---|---|---|
| `optimize/propagate.rs` | 3 | ~120 | Constant propagation (simp-style substitution) |
| `optimize/compile_eval.rs` | 4 | ~200 | Compile-time function evaluation via interpreter |
| `optimize/normalize.rs` | 5 | ~180 | Polynomial normalization (ring-style canonicalization) |

## Modified Files

| File | Phases | Changes |
|---|---|---|
| `optimize/fold.rs` | 1, 5, 6 | Deep Expr recursion, algebraic simplification, normalization integration, peephole |
| `optimize/dce.rs` | 2 | Post-Return unreachable code elimination |
| `optimize/mod.rs` | 3, 4 | Pipeline orchestration with all new passes |
| `interpreter.rs` | 4 | Step-limited synchronous evaluation mode |
| `tests/phase_optimize.rs` | All | ~55 new RED tests |

---

## Verification

After each phase:
1. `cargo test --test phase_optimize` — all new tests GREEN
2. `cargo test -- --skip e2e` — no regressions
3. After all phases: `cargo test` — full suite

### Safety Invariants

- `Check` statements are **NEVER** eliminated (security invariant)
- Float arithmetic is **NOT** algebraically normalized (IEEE 754 rounding)
- Mutable variables are **NOT** propagated
- Division by zero is **NOT** folded
- Impure functions are **NOT** evaluated at compile time
- Step limit prevents infinite loops in compile-time evaluation
- Polynomial normalization uses i64 wrapping semantics (matches runtime)

---

## What Makes This World-Class

**1. Tactic-based architecture.** The optimizer isn't a bag of special cases — it's a composed pipeline modeled on the kernel's tactic combinators. Each pass is a rewriting strategy; the pipeline is `repeat(then(propagate, then(try(compile_eval), then(fold, dce))))`. New optimizations plug in as new tactics.

**2. Polynomial normalization from the proof kernel.** `kernel/ring.rs` already canonicalizes polynomials for theorem proving. We bring the same algorithm to the optimizer. `a + b - b → a` isn't a special case — it's an algebraic identity that emerges automatically from polynomial canonical forms.

**3. Interpreter-as-oracle.** The interpreter and compiler share the same AST in the same crate. Compile-time evaluation of arbitrary pure user-defined recursive functions — `factorial(5) → 120`, `fib(10) → 55` — using the existing interpreter with a step limit.

**4. Hypothesis-driven substitution from `simp`.** Constant propagation mirrors the `simp` tactic: extract known equalities (`Let x be 5` ≡ hypothesis `x = 5`), substitute throughout, re-simplify. The parallel is exact.

**5. Multi-pass cascading.** Like `tact_repeat`, the pipeline runs fold+DCE twice to catch opportunities created by propagation and compile-time evaluation. Each pass creates opportunities for the next.

---

## Future Directions (Beyond This Plan)

These build on the foundation above:

- **E-graph optimization** — Adapt `kernel/cc.rs` Union-Find + congruence closure to maintain equivalence classes over Expr nodes. Choose the cheapest representative from each class. This is equality saturation.

- **Refinement-guided branch elimination** — `TypeExpr::Refinement { predicate }` carries provable properties. If `x: {n: Int | n > 0}`, then `If x > 0:` is always true. Use `kernel/lia.rs` (Fourier-Motzkin) to prove inequality constraints at compile time.

- **Function specialization** — When a function is called with some known and some unknown arguments, generate a specialized version with the known args baked in. This is FRIEND_PLANS Stage 3 and the bridge to partial evaluation.

- **Dead variable elimination** — Liveness analysis to remove `Let x be expr.` when `x` is never read and `expr` is pure. Requires a use-counting pass.

- **Unified evaluator** — The ultimate goal from FRIEND_PLANS Section 20: merge the interpreter and compiler into one evaluator that decides at each AST node whether to evaluate or emit code.
