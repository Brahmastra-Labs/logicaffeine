# Codegen Ownership Optimizations

## Interview Q&A

**Q: How deep should readonly call-graph analysis go?**
Full call-graph with fixed-point iteration. World-class handling at native boundaries. Trust native's declared signature — it acts as a forcing function for FFI correctness.

**Q: Handling closure captures in readonly inference?**
Inspect the closure body too. If the closure only reads the captured variable, still optimize.

**Q: Cycles in the call graph?**
Fixed-point iteration over the call graph (classic dataflow convergence).

**Q: Native function boundaries?**
Trust the native's declared signature. Acts as a forcing function for getting FFI declarations right.

**Q: Last-use move failure mode?**
Proper backward dataflow over CFG. World-class liveness. Wrapped in tests. Three levels of testing.

**Q: TypeEnv threading?**
Thread TypeEnv properly — update all codegen signatures. Replace legacy `HashMap<Symbol, String>` everywhere.

**Q: Bounds checking for direct indexing?**
Maximum performance. Rust's default panic is fine. Speed is most important.

**Q: Liveness shared between optimizer and codegen?**
Yes — build one `analysis/liveness.rs`, used by both OPTIMIZER_PLAN dead-Let elimination and codegen move-not-clone.

**Q: Compile-eval (OPTIMIZER_PLAN Phase 4) and memoization?**
Complementary. `compile_eval` handles `fib(25)` with literal args. Memoization handles `fib(n)` from argv. Both needed.

**Q: Pass structure?**
Three separate passes in order: (1) call-graph + readonly, (2) liveness + last-use move, (3) TypeEnv indexing.

**Q: Liveness model?**
Proper backward dataflow over the CFG.

**Q: Compile-time cost?**
Precision over speed. Compile time doesn't matter.

**Q: Module location?**
New submodules in `analysis/`: `callgraph.rs`, `readonly.rs`, `liveness.rs`.

**Q: Test levels?**
All three — analysis unit tests, codegen output tests, E2E behavior tests.

**Q: Spec detail level?**
Define exact types and function signatures in the spec.

**Q: Spec location?**
`project-specs/codegen-ownership-optimizations.md` at repo root.

---

## Context

LOGOS compiles to Rust (via rustc/LLVM) and achieves 0.575x C speed (geometric mean, 32 benchmarks). Hand-written Rust achieves 0.932x C. The 37-point gap is entirely codegen quality. This spec defines three structural compiler optimizations that address the root causes without heuristics.

Three benchmarks reveal the core problems:

- **heapsort (8.48x slower)**: `arr = siftDown(arr.clone(), ...)` — O(n) clone called O(n log n) times, plus LogosIndex trait dispatch in the hot siftDown loop
- **nqueens (12.22x slower)**: `isSafe(queens.clone(), ...)` — O(n) clone per safety check, called n times per recursion level
- **sieve (1.39x slower)**: `LogosIndex::logos_get(&flags, (i+1))` in the innermost marking loop

The root causes are three missing compiler analyses:

1. No call-graph-based readonly parameter inference → unnecessary clones at call sites
2. No liveness analysis → can't detect when a clone is the last use (could be a move)
3. TypeEnv not consulted during codegen → trait dispatch used even when type is statically known

---

## Architecture

Three new analysis modules run after type inference (TypeEnv construction) and before codegen:

```
LOGOS AST
  └─→ Type Inference (existing: analysis/check.rs → TypeEnv)
        └─→ CallGraph Analysis (NEW: analysis/callgraph.rs)
              └─→ Readonly Inference (NEW: analysis/readonly.rs)
                    └─→ Liveness Analysis (NEW: analysis/liveness.rs)
                          └─→ Codegen (updated: codegen/*.rs)
                                └─→ Rust source
```

The analyses are computed once and stored in a `CompileContext` struct passed to all codegen functions alongside `TypeEnv`.

---

## Analysis 1: Call Graph (`analysis/callgraph.rs`)

### Purpose

Build a whole-program call graph to support readonly inference and purity analysis (also used by OPTIMIZER_PLAN Phase 4).

### Types

```rust
/// Whole-program call graph.
pub struct CallGraph {
    /// fn_sym → set of functions directly called in its body (LOGOS-defined only).
    pub edges: HashMap<Symbol, HashSet<Symbol>>,
    /// Native functions (opaque bodies — trust their declared signature).
    pub native_fns: HashSet<Symbol>,
    /// Strongly connected components (for cycle detection).
    pub sccs: Vec<Vec<Symbol>>,
}

impl CallGraph {
    /// Build from a program's top-level statements.
    pub fn build(stmts: &[Stmt<'_>], interner: &Interner) -> Self;

    /// All functions reachable from `fn_sym` (transitive closure).
    pub fn reachable_from(&self, fn_sym: Symbol) -> HashSet<Symbol>;

    /// Is `fn_sym` recursive (directly or mutually)?
    pub fn is_recursive(&self, fn_sym: Symbol) -> bool;
}
```

### Algorithm

1. Walk all `FunctionDef` statements in the program.
2. For each function body, collect all `Call` targets → add edges.
3. Walk closure bodies too — closures can call LOGOS functions.
4. Compute SCCs (Tarjan's or Kosaraju's) for cycle detection.
5. Mark any function declared `## To native` as a native function.

### Native Boundary Rule

Native functions are trusted by their declared signature. If a native is declared as taking `Seq<T>` (not `Seq<T> mutable`), the analysis treats it as a read-only consumer. This is a **forcing function**: it requires FFI declarations to accurately reflect mutability, which is the right design constraint.

---

## Analysis 2: Readonly Parameter Inference (`analysis/readonly.rs`)

### Purpose

Determine which `Seq<T>` parameters are never mutated — directly or transitively through callees. These parameters can be emitted as `&[T]` instead of `Vec<T>`, eliminating clones at all call sites.

### Types

```rust
/// Per-function: which parameter symbols are never mutated.
pub struct ReadonlyParams {
    /// fn_sym → set of param symbols that are readonly.
    pub readonly: HashMap<Symbol, HashSet<Symbol>>,
}

impl ReadonlyParams {
    /// Compute from the program, call graph, and type env.
    pub fn analyze(
        stmts: &[Stmt<'_>],
        callgraph: &CallGraph,
        type_env: &TypeEnv,
    ) -> Self;

    /// Is the given parameter readonly in the given function?
    pub fn is_readonly(&self, fn_sym: Symbol, param_sym: Symbol) -> bool;
}
```

### Algorithm: Fixed-Point Iteration

```
Initialize:
    For each FunctionDef with Seq<T> params:
        readonly[fn] = { all Seq<T> param symbols }  // optimistic start

Iterate until stable:
    For each FunctionDef fn:
        For each param p in readonly[fn].clone():
            If is_mutated_in_body(fn.body, p, callgraph, readonly):
                readonly[fn].remove(p)
                changed = true

is_mutated_in_body(stmts, param, callgraph, readonly) → bool:
    For each stmt:
        Push { collection: Identifier(p), .. } where p == param → MUTATED
        Pop { collection: Identifier(p) } where p == param → MUTATED
        Add/Remove { collection: Identifier(p) } → MUTATED
        SetIndex { collection: Identifier(p), .. } → MUTATED
        SetField { target: Identifier(p), .. } → MUTATED
        Set { target: Identifier(p), .. } → MUTATED (reassignment)

        Call { function: callee, args } →
            For each arg i that is Identifier(p):
                callee_param_sym = callee's i-th parameter
                If callee is native: check declared mutability from FnSig
                Else if NOT readonly[callee].contains(callee_param_sym):
                    → MUTATED (transitively)

        Closure { body, captures } →
            If p is captured AND is_mutated_in_body(body, p, ...):
                → MUTATED

        If/While/Repeat: recurse into all branches/bodies
```

Cycles in the call graph are handled naturally: fixed-point iteration continues until no change. For mutually recursive functions, if any function in the SCC mutates a parameter, that propagates outward.

### Codegen Effect

**Function definitions** (`codegen/program.rs`):
```rust
// Before: fn isSafe(queens: Vec<i64>, row: i64, col: i64) -> i64
// After:  fn isSafe(queens: &[i64], row: i64, col: i64) -> i64
```

**Call sites** (`codegen/expr.rs`):
```rust
// Before: isSafe(queens.clone(), row, col)
// After:  isSafe(&queens, row, col)
```

When the callee's parameter is readonly:
- The parameter type becomes `&[T]`
- The call site emits `&collection` instead of `collection.clone()`
- If the collection is already borrowed (`&[T]`), emit directly

---

## Analysis 3: Liveness Analysis (`analysis/liveness.rs`)

### Purpose

Compute, for each statement in each function, which variables are **live after** that statement. A variable is live if it may be read on some execution path after this point without being reassigned first.

This analysis is **shared** between:
- **Codegen**: last-use move optimization (if a non-Copy variable is not live after the function call, move it instead of cloning)
- **OPTIMIZER_PLAN**: dead variable elimination (if a `Let` binding is not live after, and the value is pure, eliminate it)

### Types

```rust
/// Liveness information for a single function's body.
pub struct FunctionLiveness {
    /// For each statement at index i (in flattened order), the set of live variables AFTER it.
    /// Index 0 = live after the first statement.
    pub live_after: Vec<HashSet<Symbol>>,
}

pub struct LivenessResult {
    /// fn_sym → liveness for that function
    pub functions: HashMap<Symbol, FunctionLiveness>,
}

impl LivenessResult {
    /// Compute for the entire program.
    pub fn analyze(stmts: &[Stmt<'_>]) -> Self;

    /// Is `var` live after statement `stmt_idx` in function `fn_sym`?
    pub fn is_live_after(&self, fn_sym: Symbol, stmt_idx: usize, var: Symbol) -> bool;

    /// Get the full live set after statement `stmt_idx` in function `fn_sym`.
    pub fn live_after(&self, fn_sym: Symbol, stmt_idx: usize) -> &HashSet<Symbol>;
}
```

### Algorithm: Backward Dataflow

For each function, run backward dataflow over the statement list. Statements are numbered top-to-bottom; analysis proceeds bottom-to-top.

```
liveness(stmts) → Vec<HashSet<Symbol>>:
    live = {}  // nothing live after the last statement (unless returned)
    result = vec![{}; stmts.len()]

    for i in (0..stmts.len()).rev():
        result[i] = live.clone()  // live AFTER stmt i
        live = live_before(stmts[i], live)

    return result

live_before(stmt, live_after) → HashSet<Symbol>:
    match stmt:
        Let { var, value, .. } →
            (live_after - {var}) ∪ free_vars(value)

        Set { target: Identifier(x), value } →
            (live_after - {x}) ∪ free_vars(value)

        Set { target: Index { Identifier(x), idx }, value } →
            live_after ∪ {x} ∪ free_vars(idx) ∪ free_vars(value)
            // x is live (element update, not full reassignment)

        Return { value } →
            free_vars(value)  // only the returned vars; nothing flows through Return

        Show/Call/Push/etc →
            live_after ∪ free_vars(all_subexpressions)

        If { cond, then_block, else_block } →
            live_then = liveness(then_block).first().cloned().unwrap_or(live_after)
            live_else = liveness(else_block).first().cloned().unwrap_or(live_after)
            (live_then ∪ live_else) ∪ free_vars(cond)

        While { cond, body } →
            // Fixed-point for loops
            live_loop = live_after
            loop:
                live_body = liveness(body).first().cloned().unwrap_or(live_loop)
                new_live = live_body ∪ free_vars(cond) ∪ live_after
                if new_live == live_loop: break
                live_loop = new_live
            live_loop

        FunctionDef { .. } →
            live_after  // function definitions don't consume outer variables
```

`free_vars(expr)` returns the set of `Identifier` symbols used in the expression.

### Codegen Effect: Last-Use Move

In `codegen/stmt.rs`, when generating a `Set { target: Identifier(x), value: Call { function, args } }`:

For each arg at position `i` that is `Identifier(y)` where `y` has type `Seq<T>` (non-Copy):
1. Check `liveness.is_live_after(fn_sym, stmt_idx, y)`
2. If NOT live → emit `y` (move) instead of `y.clone()`
3. If live → emit `y.clone()` (must preserve)

**Key pattern** — `x = f(x, ...)` where x is the LHS:
- After this statement, the old `x` value is dead (it's been overwritten)
- Therefore `x` is not live after this statement
- So `x` can be moved into `f()` without cloning

```rust
// heapsort — generated now:
arr = siftDown(arr.clone(), start, (n - 1));  // 40KB clone × O(n log n) calls

// heapsort — after optimization:
arr = siftDown(arr, start, (n - 1));          // zero-cost move
```

---

## Codegen Change 1: TypeEnv Threading

### Problem

`codegen_expr` and related functions currently accept `variable_types: &HashMap<Symbol, String>` — the legacy string-based type map. This was a temporary compatibility shim. It prevents proper `LogosType` queries and forces `to_legacy_variable_types()` conversion overhead.

### Change

Replace all codegen function signatures to accept `&TypeEnv` (and the new `&CompileContext`) instead of the legacy `HashMap`. The `CompileContext` bundles all analyses:

```rust
pub struct CompileContext<'a> {
    pub type_env: &'a TypeEnv,
    pub readonly_params: &'a ReadonlyParams,
    pub liveness: &'a LivenessResult,
    pub callgraph: &'a CallGraph,
    pub current_fn: Symbol,           // which function we're generating
    pub current_stmt_idx: usize,      // for liveness queries
    pub string_vars: HashSet<Symbol>, // existing string tracking
    pub async_fns: HashSet<Symbol>,   // existing async tracking
}
```

All `codegen_expr_*` family functions take `&CompileContext` instead of the current scattered `variable_types`, `string_vars`, `async_fns` parameters.

**Files**: `codegen/expr.rs`, `codegen/stmt.rs`, `codegen/program.rs`, `codegen/types.rs`, `codegen/ffi.rs`, `codegen/detection.rs`

---

## Codegen Change 2: TypeEnv-Driven Direct Indexing

### Problem

`LogosIndex::logos_get(&collection, idx)` and `LogosIndexMut::logos_set(&mut collection, idx, val)` are used as fallbacks for ALL collection indexing. The peephole catches simple `+1` patterns but misses all other cases (plain variable indices, complex arithmetic). This puts trait dispatch overhead on every hot array access.

### Change

In `codegen/expr.rs`, the `Index { collection, index }` branch:

```rust
// Current (fallback always used for complex indices):
format!("LogosIndex::logos_get(&{}, {})", coll_code, idx_code)

// New (consult TypeEnv first):
match ctx.type_env.lookup(collection_sym) {
    LogosType::Seq(elem_ty) => {
        let clone_suffix = if !elem_ty.is_copy() { ".clone()" } else { "" };
        format!("{}[(({}) - 1) as usize]{}", coll_code, idx_code, clone_suffix)
    }
    LogosType::Unknown => {
        format!("LogosIndex::logos_get(&{}, {})", coll_code, idx_code)
    }
    // Map/Set handled by existing paths
}
```

In `codegen/stmt.rs`, the `SetIndex` branch:

```rust
// Current:
format!("LogosIndexMut::logos_set(&mut {}, {}, {})", coll_code, idx_code, val_code)

// New:
match ctx.type_env.lookup(collection_sym) {
    LogosType::Seq(_) => {
        format!("{}[(({}) - 1) as usize] = {}", coll_code, idx_code, val_code)
    }
    LogosType::Unknown => {
        format!("LogosIndexMut::logos_set(&mut {}, {}, {})", coll_code, idx_code, val_code)
    }
}
```

`LogosIndex::logos_get` and `LogosIndexMut::logos_set` remain in the runtime as the fallback for dynamically-typed or unknown-type collection access.

**Bounds semantics**: Direct `[(idx-1) as usize]` uses Rust's built-in bounds checking, which panics on out-of-bounds. This is acceptable — Rust's panic message ("index out of bounds: the len is N but the index is M") is informative and the performance benefit of eliminating trait dispatch is significant.

---

## Implementation Order

Three passes run in sequence. Each is independently testable.

### Pass 1: Call-graph + Readonly

1. Implement `analysis/callgraph.rs` with `CallGraph::build()`
2. Implement `analysis/readonly.rs` with `ReadonlyParams::analyze()`
3. Wire into compile pipeline after `TypeEnv::infer_program()`
4. Update `codegen/program.rs` to emit `&[T]` params when readonly
5. Update `codegen/expr.rs` call sites to emit `&collection` when target param is readonly

### Pass 2: Liveness + Last-Use Move

1. Implement `analysis/liveness.rs` with `LivenessResult::analyze()`
2. Add `CompileContext` struct (threading all analyses through codegen)
3. Update `codegen/stmt.rs` to use liveness for move-not-clone at function call sites
4. Thread `CompileContext` through `codegen/expr.rs` signature chain

### Pass 3: TypeEnv Direct Indexing

1. Complete `CompileContext` threading (depends on Pass 2)
2. Update `codegen/expr.rs` Index branch to use `TypeEnv::lookup()`
3. Update `codegen/stmt.rs` SetIndex branch similarly
4. Remove/deprecate the peephole patterns that were patching around this (`logos_get(x, n+1)` → `x[n]`)

---

## Test Specifications

Three test levels, all required. TDD: write RED tests before implementation.

### Level 1: Analysis Unit Tests (`tests/phase_analysis.rs`)

```rust
// CallGraph
callgraph_direct_call            // fn A calls B → A→B edge exists
callgraph_transitive             // fn A calls B calls C → B→C edge exists
callgraph_recursive_detected     // fn A calls A → A in recursive_fns
callgraph_mutual_recursive       // fn A calls B, B calls A → both in same SCC
callgraph_native_marked          // ## To native fn → in native_fns
callgraph_closure_calls_counted  // closure inside fn A calling B → A→B edge

// ReadonlyParams
readonly_pure_reader             // fn f(xs: Seq of Int) that only indexes xs → xs is readonly
readonly_pusher_not_readonly     // fn f(xs: Seq of Int) that pushes → xs NOT readonly
readonly_transitive_mutation     // fn f(xs) calls g(xs) where g mutates → xs NOT readonly
readonly_transitive_pure         // fn f(xs) calls g(xs) where g only reads → xs IS readonly
readonly_fixed_point_convergence // mutual recursion both reading → both params readonly
readonly_closure_read_only       // fn f(xs) has closure that reads xs → xs still readonly
readonly_closure_mutates         // fn f(xs) has closure that pushes to xs → xs NOT readonly
readonly_native_trusted          // fn f(xs) calls native read_fn(xs) declared as read → readonly

// LivenessResult
liveness_simple_sequential       // Let x = 5. Show x. → x live before Show, dead after
liveness_reassignment_kills      // Let x = 5. Set x to 10. → x dead between (old value gone)
liveness_branch_union            // If cond: use(x). Else: use(y). → both x and y live before If
liveness_while_loop_fixed_point  // While c: use(x). Set x = ... → x live at loop entry
liveness_return_terminates       // Return x. unreachable. → only x live at Return
liveness_last_use_detected       // Set arr to f(arr). → arr dead after (overwritten)
liveness_not_last_use            // Set arr to f(arr). Show arr. → arr STILL live (used after)
```

### Level 2: Codegen Output Tests (extend `tests/phase24_codegen.rs`)

```rust
// Readonly parameter tests
codegen_readonly_param_emits_slice    // fn isSafe with readonly queens → fn isSafe(queens: &[i64], ...)
codegen_readonly_call_emits_borrow    // call to isSafe → isSafe(&queens, ...) not isSafe(queens.clone(), ...)

// Last-use move tests
codegen_last_use_emits_move          // arr = siftDown(arr, ...) → no .clone()
codegen_not_last_use_emits_clone     // arr = siftDown(arr, ...) then show(arr) → .clone() kept

// Direct indexing tests
codegen_seq_index_direct             // item i of xs where xs: Seq<Int> → xs[(i-1) as usize]
codegen_seq_setindex_direct          // Set item i of xs to v where xs: Seq<Int> → xs[(i-1) as usize] = v
codegen_unknown_type_uses_trait      // item i of xs where xs: Unknown → LogosIndex::logos_get
codegen_sieve_inner_loop_direct      // sieve benchmark flags indexing → direct, no trait dispatch
```

### Level 3: E2E Behavior Tests (`tests/e2e_codegen_optimization.rs`)

Compile and run all 32 benchmark programs, assert output is byte-identical to `expected_<size>.txt`:

```rust
e2e_all_benchmarks_correct_output  // after optimizations, all 32 benchmarks produce correct results
```

Specific regressions to guard:
- nqueens(10) → "724" (exact count preserved)
- heapsort(5000) → exact first/last/checksum
- sieve(100000) → "9592"
- fib(25) → "75025" (memoized OR compile-eval, same result)

---

## Critical Files

| File | Status | Purpose |
|------|--------|---------|
| `crates/logicaffeine_compile/src/analysis/callgraph.rs` | NEW | CallGraph struct, build(), SCC detection |
| `crates/logicaffeine_compile/src/analysis/readonly.rs` | NEW | ReadonlyParams, fixed-point readonly analysis |
| `crates/logicaffeine_compile/src/analysis/liveness.rs` | NEW | LivenessResult, backward dataflow CFG |
| `crates/logicaffeine_compile/src/analysis/mod.rs` | MODIFY | Export new analysis modules |
| `crates/logicaffeine_compile/src/compile.rs` | MODIFY | Run new analyses in pipeline, build CompileContext |
| `crates/logicaffeine_compile/src/codegen/expr.rs` | MODIFY | TypeEnv direct indexing; call site readonly borrow |
| `crates/logicaffeine_compile/src/codegen/stmt.rs` | MODIFY | SetIndex direct; last-use move at call sites |
| `crates/logicaffeine_compile/src/codegen/program.rs` | MODIFY | &[T] params for readonly; CompileContext threading |
| `crates/logicaffeine_compile/src/codegen/mod.rs` | MODIFY | CompileContext struct definition |
| `crates/logicaffeine_tests/tests/phase_analysis.rs` | NEW | Analysis unit tests |
| `crates/logicaffeine_tests/tests/phase24_codegen.rs` | MODIFY | Codegen output pattern tests |
| `crates/logicaffeine_tests/tests/e2e_codegen_optimization.rs` | MODIFY | E2E correctness regression |

---

## Expected Benchmark Impact

| Benchmark | Current | Expected After | Primary Fix |
|-----------|---------|----------------|-------------|
| heapsort | 8.48x | ~1.5–2x | Last-use move (arr.clone()) + LogosIndex |
| mergesort | 7.18x | ~2.5x | LogosIndex + last-use move |
| nqueens | 12.22x | ~3–5x | Readonly (isSafe clone eliminated) |
| quicksort | 3.96x | ~1.5x | Last-use move (qs(arr.clone())) + LogosIndex |
| sieve | 1.39x | ~1.0x | LogosIndex direct indexing |
| matrix_mult | 2.68x | ~1.3x | LogosIndex direct indexing |
| string_search | 4.83x | ~2.0x | LogosIndex direct indexing |

**Target**: geometric mean moves from **0.575 → 0.75+**, placing LOGOS between Go (0.694) and Nim (0.945).

---

## Relationship to OPTIMIZER_PLAN.md

This spec targets **codegen-level** ownership and type-dispatch issues. OPTIMIZER_PLAN.md targets **AST-level** expression optimization. They are complementary and share infrastructure:

- `analysis/liveness.rs` feeds both OPTIMIZER_PLAN's dead-Let elimination (future) and this spec's last-use move (now)
- `analysis/callgraph.rs` feeds both OPTIMIZER_PLAN Phase 4 purity analysis and this spec's readonly inference
- OPTIMIZER_PLAN Phase 4 (compile-time function evaluation) and memoization are complementary: `compile_eval` handles literal args, memoization handles runtime args

The correct implementation order: complete this spec first (fixes the biggest performance regressions), then implement OPTIMIZER_PLAN phases (which depend on the same analysis infrastructure).
