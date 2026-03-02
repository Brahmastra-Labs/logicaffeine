# FUTURISTIC.md — The Optimizer Mountain Climb

> LogicAffeine has something almost no other compiled language has: total semantic
> knowledge at every stage. No pointer aliasing, no undefined behavior, known types,
> known mutability, known effects. LLVM works on a pancake — flat IR where a
> `getelementptr` could be anything. We work on the cake itself — we know it's a sort,
> a reduction, a tree traversal. This document specifies what lives in that gap.

---

## The Mountain

```
                          SUMMIT
                 Full Supercompilation (Turchin, 1986)
                    Driving + Folding + Generalization
                    /                \
              Camp 8                  Camp 9
         Auto-Parallelization    Partial Evaluation
         (Bernstein, 1966)       (Futamura, 1971)
                  |                      |
              Camp 7                Camp 6
        Polyhedral Tiling       Deforestation
        (Feautrier, 1991)       (Wadler, 1988)
                   \                  /
                     \              /
                      Camp 5
                 Closed-Form + Strength Reduction
                 + Reassociation + Division Magic
                 (Cocke & Allen, 1971; Granlund, 1994)
                        |
                      Camp 4
                LICM + DSE + Loop Unswitching + Peeling
                (Allen, 1969; Kildall, 1973)
                        |
                      Camp 3
                Abstract Interpretation
                (Cousot & Cousot, 1977)
                        |
                    CAMP 2
               Return-Value-As-Output-Parameter
              + For-In Reference Iteration
                        |
                    CAMP 1
                  Effect System
                  (Gifford & Lucassen, 1986)
                        |
                 ===============
                 CAMP 0 (QUICK WINS)
                 ===============
                 0a: FxHashMap for integer keys
                 0b: Generalized bounds check elision
                 0c: Float comparison folding
                 0d: Bitwise operation folding
                 0e: Propagation in all expression contexts
                 0f: Fix compile_to_rust_checked() optimization
                 0g: Boolean algebra laws
                 0h: Self-comparison identities
                 0i: Local CSE / Value numbering
                 0j: Compile-time function evaluation (CTFE)
                 0k: Trait-dispatch elimination for Vec indexing
                 0l: 1-based → 0-based index lowering in for-range
                 ===============
                 GROUND (today)
                 ===============
```

### What We Have Today (Ground Level)

| Optimization | Location | What It Does |
|---|---|---|
| Constant Folding | `optimize/fold.rs` | `2 + 3` -> `5`, algebraic identities |
| Dead Code Elimination | `optimize/dce.rs` | Remove code after `Return`, eliminate `If false` |
| Constant Propagation | `optimize/propagate.rs` | Substitute immutable bindings with literal values |
| For-Range Conversion | `codegen/peephole.rs` | `Let i=0. While i<n: ...i+1` -> `for i in 0..n` |
| Vec Fill | `codegen/peephole.rs` | `new Seq + loop Push constant` -> `vec![val; n]` |
| Vec With Capacity | `codegen/peephole.rs` | Pre-allocate when push count is provably known |
| Swap Detection | `codegen/peephole.rs` | `Let temp=a[i]; a[i]=a[j]; a[j]=temp` -> `.swap()` |
| Seq Copy / Slice | `codegen/peephole.rs` | Loop-copy -> `.to_vec()`, slice-push -> `.extend_from_slice()` |
| Buffer Reuse | `codegen/peephole.rs` | Hoist allocation out of while-loop, `.clear()` + reuse |
| Rotate Left | `codegen/peephole.rs` | Push-all-but-first + push-first -> `.rotate_left(1)` |
| Readonly Params | `analysis/readonly.rs` | `Seq<T>` params never mutated -> `&[T]` borrow |
| Mutable Borrow | `analysis/readonly.rs` | `Seq<T>` params only element-mutated -> `&mut [T]` |
| Tail-Call Elimination | `codegen/tce.rs` | Recursive tail calls -> loops |
| String Concat Flatten | `codegen/expr.rs` | `a + b + c` -> single `format!("{}{}{}", a, b, c)` |
| Self-Append | `codegen/stmt.rs` | `s = s + x` -> `write!(s, "{}", x)` |
| Bounds Check Hint | `codegen/peephole.rs` | `assert_unchecked(limit <= arr.len())` for one pattern |

### What We're Building (The Mountain)

Each camp builds on the camps below it. Dependencies flow downward. No camp can be
started until its dependencies are complete and green.

---

## KNOWN PITFALLS (Read Before Climbing)

These are structural hazards discovered during deep audit of the codebase against
this spec. Every climber must know these before starting any camp.

### Pitfall 1 — The Spec Covers 12 of 54 Stmt Variants

The Base Camp originally listed `Let`, `Set`, `Push`, `Show`, `If`, `While`,
`FunctionDef`, and a few others. The actual `Stmt` enum in `ast/stmt.rs` has
**54 variants**. The following 30+ production-critical variants were unaddressed:

- **Ownership:** `Give` (move semantics — not Read, not Write, it's a *consumption*)
- **CRDT:** `MergeCrdt`, `IncreaseCrdt`, `DecreaseCrdt`, `AppendToSequence`, `ResolveConflict` (5 variants, all commutative writes)
- **Concurrency:** `LaunchTask`, `LaunchTaskWithHandle`, `CreatePipe`, `SendPipe`, `ReceivePipe`, `TrySendPipe`, `TryReceivePipe`, `StopTask`, `Select` (9 variants, all IO+Diverge)
- **Network:** `Listen`, `ConnectTo`, `LetPeerAgent`, `Sleep`, `Sync`, `Mount` (6 variants, all IO+Diverge)
- **Collections:** `Pop`, `Add`, `Remove`, `SetIndex`, `SetField` (5 variants, all Write)
- **Control:** `Inspect` (pattern matching — branch-join semantics like If), `Break`, `Return`
- **Security:** `Check` (MUST be marked non-eliminable — see Pitfall 9)
- **Memory:** `Zone` (Alloc + scoped body effects)
- **Parallel:** `Concurrent` (tokio::join!), `Parallel` (rayon::join)
- **IO:** `ReadFrom`, `WriteFile`, `Spawn`, `SendMessage`, `AwaitMessage`

**Resolution:** Camp 1 (Effect System) now includes a COMPLETE classification table
mapping ALL 54 Stmt variants and ALL 28 Expr variants. No `_ => Unknown` escape
hatch — every variant has explicit classification.

### Pitfall 2 — Pipeline Order Conflict

The spec originally said: "run effect analysis after type checking, before optimization."

Reality at `compile.rs:274`:
```
Parse -> Optimize (fold/propagate/dce) -> Escape analysis -> Type checking -> Codegen
```

Optimization runs BEFORE type checking. The effect system needs types (to distinguish
`Push` on `Seq` vs string append).

**Resolution:** The pipeline must be reordered to:
```
Parse -> Type checking (moved up) -> Effect analysis (new) -> Optimize (now type+effect-aware) -> Escape analysis -> Codegen
```

This is a non-trivial change. Type checking currently runs on the post-optimization
AST. Moving it earlier means it runs on the pre-optimization AST, which has more
nodes (dead code not yet eliminated). Must verify the type checker handles
`If false: <type-inconsistent-code>` gracefully.

### Pitfall 3 — Existing Infrastructure to Reuse

The spec originally implied building everything from scratch. These already exist
and MUST be reused:

| Existing | Location | How to Reuse |
|----------|----------|--------------|
| Call graph with SCC | `analysis/callgraph.rs` | Effect system fixed-point iteration |
| Fixed-point propagation | `analysis/readonly.rs` | Already does Read/Write tracking for Seq params |
| Async function detection | `codegen/detection.rs:collect_async_functions()` | Already does 2-pass effect propagation |
| Liveness analysis | `analysis/liveness.rs` | LICM and DSE need this |
| Mutable var collection | `codegen/detection.rs:collect_mutable_vars()` | Subset of Write effect |
| Bidirectional type checker | `analysis/check.rs` | Unification infrastructure for effect inference |
| Ownership analysis | `analysis/ownership.rs` | Overlaps with Give/Move effect tracking |

The effect system should be built ON TOP of `CallGraph` (reuse SCC iteration),
informed BY `TypeEnv`, and designed to REPLACE the ad-hoc parameters in
`codegen_stmt`'s 14-parameter signature (`mutable_vars`, `async_functions`,
`synced_vars`, `pipe_vars`).

### Pitfall 4 — Constant Propagation Is Deliberately Crippled

`propagate.rs` ONLY substitutes inside `Let`/`Set` value expressions. It does NOT
substitute into:
- `If` conditions
- `While` conditions
- `Return` values
- `Show` arguments
- `Push` values
- `Repeat` iterables

This means: `Let x be 5. If x is greater than 100: Show "impossible".` will NOT be
eliminated by the fold+propagate+dce pipeline, because `x` is never substituted into
the `If` condition. The condition remains `x > 100` (not `5 > 100`), so DCE never
sees a literal boolean.

**Resolution:** Camp 0e strengthens propagation to substitute in ALL expression
contexts. Abstract interpretation (Camp 3) will no longer need to re-derive values
that propagation already knows.

### Pitfall 5 — `compile_to_rust_checked()` Skips Optimization

`compile.rs:~476`: The `_checked` path (escape + ownership analysis) never calls
`optimize_program()`. Checked builds produce unoptimized code.

**Resolution:** Camp 0f wires `optimize_program()` into the checked path.

### Pitfall 6 — InterpolatedString Is Never Optimized

Both `fold.rs` and `propagate.rs` skip `Expr::InterpolatedString` entirely. The
effect system must classify it (Read for each hole expression). LICM could hoist
invariant interpolated strings out of loops. Abstract interpretation could track
values through string holes.

### Pitfall 7 — Float Comparisons Are Never Folded

`fold.rs:fold_float_op` handles `Add/Sub/Mul/Div` but NOT `Eq/NotEq/Lt/Gt/LtEq/GtEq`.
Integer comparisons are folded in `fold_int_op`, but `3.14 > 0.0` remains a runtime
comparison. Camp 3's abstract interpretation assumes folding handles trivial cases —
it doesn't for floats.

**Resolution:** Camp 0c adds comparison handling to `fold_float_op`.

### Pitfall 8 — Bitwise Operations Are Never Folded

`BitXor`, `Shl`, `Shr` exist as `BinaryOpKind` variants but are not handled by
`fold_int_op` (falls through to `_ => None`). The `nqueens` benchmark uses bitwise
operations heavily.

**Resolution:** Camp 0d adds bitwise folding to `fold_int_op`.

### Pitfall 9 — The `Check` Statement Must Never Be Eliminated

`Stmt::Check` is a security guard. DSE and DCE must explicitly exclude it. Even if a
`Check` appears to be dead or redundant, it MUST execute. The effect system classifies
it as `SecurityCheck` — non-eliminable, non-reorderable.

### Pitfall 10 — For-Range Peephole Ordering With New Passes

The for-range peephole (`try_emit_for_range_pattern`) restructures While loops into
for-loops during codegen. LICM (Camp 4) needs the WHILE form to see the original
loop structure.

**Resolution:** LICM runs as an AST-level pass in `optimize/` on While loops, BEFORE
the codegen peephole chain.

### Pitfall 11 — LICM + Zero-Trip Loops

If a hoisted expression can panic (division by zero, index out of bounds) and the
loop may execute zero times, hoisting changes behavior.

**Resolution:** LICM is strictly AFTER abstract interpretation in the dependency
graph. LICM uses range information to prove loops execute at least once before
hoisting potentially-panicking expressions. For provably-safe expressions (Pure
arithmetic), hoist unconditionally.

### Pitfall 12 — Deforestation Linearity Claim Is Overstated

LOGOS does NOT have linear types. It has MOVE semantics with clone. The codegen
inserts `.clone()` liberally (e.g., for-in loops clone the collection:
`for x in collection.clone()`). The ownership system prevents use-after-move, but
that's weaker than linearity. A variable can be cloned before being consumed.

**Resolution:** Linearity checking for deforestation must verify: (1) no clone of
the intermediate collection, (2) no borrow/reference, (3) single consumer. The
ownership analysis in `analysis/ownership.rs` helps but doesn't provide linearity
proofs.

### Pitfall 13 — Polyhedral Tiling Requires 2D Array Detection

LOGOS represents 2D arrays as 1D with manual indexing: `item (i * n + j + 1) of c`.
The polyhedral model needs to detect this `i*n+j` pattern and reconstruct the 2D
access — a non-trivial pattern match.

### Pitfall 14 — Auto-Parallelization + Existing `Simultaneously:` Conflict

LOGOS already has explicit parallel blocks (`Simultaneously:` emits `rayon::join`).
Camp 8's auto-parallelization would emit `rayon::par_iter`. Nested `par_iter` inside
explicit `Simultaneously:` blocks causes thread pool contention. Detection must check
whether the loop is ALREADY inside a parallel context.

### Pitfall 15 — Closed-Form vs. Modular Arithmetic

Every benchmark that does summation uses `% 1000000007` (modular arithmetic).
`sum(i, 1..n) mod M` does NOT equal `n*(n+1)/2 mod M` unless modular arithmetic
versions of the formulas are used (which exist but are more complex). The formula
table needs modular variants.

### Pitfall 16 — Algorithm Selection Semantic Equivalence

`sort_unstable()` is NOT stable — equal elements may be reordered. If the user wrote
a bubble sort and their program depends on stability, replacing with
`sort_unstable()` is wrong. Must use `sort()` (stable) as the replacement.

### Pitfall 17 — Priority Inversions Fixed In This Revision

Three priority inversions from the original spec have been corrected:

1. **Bounds check elimination** was buried behind 2 sprints. It's the #1 performance
   bottleneck (1.3x-2.2x slowdown vs C across 12+ benchmarks). Now in Camp 0b
   (no dependencies, ships first).

2. **HashMap performance** was not in the spec at all. `collect` (5.34x) and
   `two_sum` (5.33x) are bottlenecked by SipHash. Now in Camp 0a (one-line fix).

3. **Closed-form recognition** had zero real benchmark impact (all use modular
   arithmetic). Demoted from Camp 3 to Camp 5, combined with strength reduction.

### Pitfall 18 — Trait-Dispatch Indexing Is the Worst Offender

The histogram benchmark is **10.62x slower than C** — the single worst benchmark in
the entire suite. The root cause: codegen emits `LogosIndex::logos_get(&counts, (v + 1))`
(trait dispatch through `LogosIndex`) where C uses `counts[v]` (direct array access).
This virtual dispatch adds indirect call overhead on EVERY array access.

The same pattern affects counting_sort (2.84x), knapsack (2.60x), and every benchmark
that does indexed access through `item N of collection`.

**Resolution:** Camp 0k eliminates trait dispatch when the collection type is
statically known to be `Vec<T>`. Direct indexing replaces `LogosIndex::logos_get()`.

### Pitfall 19 — 1-Based Indexing Costs a Subtraction on Every Access

LOGOS uses 1-based indexing. Every `item i of arr` compiles to `arr[(i - 1) as usize]`.
Inside a tight loop iterating from 1 to n, this extra subtraction executes on every
iteration. On benchmarks where the loop body is a single array access (sieve,
prefix_sum, array operations), this subtraction is 5-10% of the loop body cost.

**Resolution:** Camp 0l detects for-range loops starting at 1 where the counter is
only used for indexing, and rewrites `1..=n` to `0..n` with direct zero-based access.

---

## CAMP 0 — Quick Wins (No Dependencies)

Each of these can be implemented independently in 1-2 days. Zero dependencies on
each other or on any new infrastructure. Ship them all before starting Camp 1.

### 0a: FxHashMap for Integer Keys

**Files:** `codegen/program.rs` (use imports) + `codegen/types.rs` (type emission)

When `LogosType::Map(Box<Int>, _)` is detected, emit `FxHashMap` instead of `HashMap`.
Add `rustc-hash = "2"` to generated `Cargo.toml` dependencies when integer-keyed maps
are used.

**Why:** `collect` (5.34x slower than C) and `two_sum` (5.33x) are bottlenecked by
Rust's `HashMap` with SipHash. The C versions use hand-rolled open-addressing. The
generated code already uses `FxHashMap` for memoization, but `Map of Int and Int`
in user code generates `std::collections::HashMap`.

**Estimated impact:** 20-40% improvement on collect/two_sum.

**TDD tests:**
```
fxhash_int_key_map — assert generated code contains "FxHashMap"
fxhash_string_key_stays_hashmap — assert generated code contains "HashMap" (not Fx)
fxhash_e2e_collect — run collect benchmark, verify identical output
```

### 0b: Generalized Bounds Check Elision

**File:** `codegen/peephole.rs` — extend OPT-4 in `try_emit_for_range_pattern`

Current OPT-4 only handles one specific sub-pattern. Generalize to: within any
for-range body, if `item <counter_expr> of <collection>` appears where `counter_expr`
is provably in `[1, length of collection]`, emit `get_unchecked`.

**Why:** Bounds-checked indexed access is the #1 performance bottleneck:

| Benchmark | Slowdown vs C | Root Cause |
|-----------|---------------|------------|
| bubble_sort | 4.15x | Inner loop indexed swap |
| counting_sort | 2.84x | Array-as-histogram indexing |
| knapsack | 2.60x | 2D DP table indexed access |
| prefix_sum | 2.52x | Sequential indexed RMW |
| string_search | 2.35x | Character indexed access |
| coins | 2.30x | 1D DP indexing |
| mergesort | 2.28x | Allocation + indexed merge |
| array_reverse | 1.83x | Two-pointer indexed swap |
| graph_bfs | 1.77x | Heavy indexed queue access |
| matrix_mult | 1.63x | Triple-nested indexed access |

**Provably-safe patterns:**
- Counter `i` in `1..=length of arr` indexing `item i of arr`
- Counter `i` in `0..length of arr` indexing `item (i + 1) of arr` (1-based offset)
- Counter `i` in `1..=n` indexing `item i of arr` when `n <= length of arr` at loop entry

**Estimated impact:** 5-15% geometric mean across 12+ benchmarks.

**TDD tests:**
```
bounds_elim_simple_range — for i from 1 to length of arr: item i of arr
bounds_elim_offset — for i from 0 to n: item (i+1) of arr (where n = length of arr)
bounds_elim_no_elide_unknown — item n of arr where n is dynamic
bounds_elim_e2e_bubble_sort — bubble sort produces correct output with elided checks
bounds_elim_e2e_counting_sort — counting sort correct with elided checks
```

### 0c: Float Comparison Folding

**File:** `optimize/fold.rs` — add comparison cases to `fold_float_op`

Add `Eq`, `NotEq`, `Lt`, `Gt`, `LtEq`, `GtEq` handling for two float literals.
Result is `Literal::Boolean`.

Currently `fold_float_op` only handles `Add/Sub/Mul/Div`. Integer comparisons are
already folded in `fold_int_op` — this brings floats to parity.

**TDD tests:**
```
fold_float_gt — 3.14 > 0.0 -> true
fold_float_eq — 1.0 == 1.0 -> true
fold_float_lt — -1.5 < 2.5 -> true
fold_float_neq — 1.0 != 2.0 -> true
fold_float_lteq — 1.0 <= 1.0 -> true
fold_float_gteq — 0.0 >= 1.0 -> false
```

### 0d: Bitwise Operation Folding

**File:** `optimize/fold.rs` — add `BitXor`, `Shl`, `Shr` to `fold_int_op`

All three exist as `BinaryOpKind` variants but fall through to `_ => None` in
`fold_int_op`. The `nqueens` benchmark uses bitwise operations heavily.

**TDD tests:**
```
fold_bitxor — 0xFF xor 0x0F -> 0xF0
fold_shl — 1 shifted left by 10 -> 1024
fold_shr — 1024 shifted right by 5 -> 32
```

### 0e: Propagation in All Expression Contexts

**File:** `optimize/propagate.rs` — substitute in `If.cond`, `While.cond`,
`Return.value`, `Show.object`, `Push.value`, `Repeat` bounds, and all other
expression positions.

Currently `propagate.rs` only substitutes into `Stmt::Let` and `Stmt::Set` value
expressions (lines 68-81). `If` conditions, `While` conditions, `Return` values,
`Show` arguments, and `Push` values are all skipped (lines 83-97 recurse into nested
blocks but don't substitute in non-value positions).

**Why this matters:** Without this fix, `Let x be 5. If x > 100: Show "impossible".`
is NOT eliminated by the fold+propagate+dce pipeline. The `If` condition remains
`x > 100` instead of becoming `5 > 100 -> false`.

**Critical safety rule:** Continue to avoid substituting inside `Expr::Index` and
`Expr::Slice` targets (lines 221-224) — this preserves AST shape for swap and
vec-fill pattern detection in the peephole.

**TDD tests:**
```
propagate_if_condition — Let x = 5. If x > 100: ... -> If false: ... -> eliminated
propagate_while_condition — Let x = false. While x: ... -> While false: ... -> eliminated
propagate_return_value — Let x = 42. Return x. -> Return 42.
propagate_show_arg — Let x = "hello". Show x. -> Show "hello".
propagate_push_value — Let x = 5. Push x to items. -> Push 5 to items.
propagate_no_substitute_index_target — preserves Index AST for peephole
```

### 0f: Fix compile_to_rust_checked() Optimization

**File:** `compile.rs:~476` — add `optimize_program()` call to the checked path.

The `compile_to_rust()` path runs `optimize_program()` at line 273. The
`compile_to_rust_checked()` path (escape + ownership analysis) skips it entirely.
Checked builds produce unoptimized code.

**TDD tests:**
```
checked_path_folds_constants — compile_to_rust_checked("Let x be 2 + 3.") folds to 5
checked_path_eliminates_dead_code — compile_to_rust_checked with dead branch eliminates it
```

### 0g: Boolean Algebra Laws

**File:** `optimize/fold.rs`

**The literature:** George Boole, *The Laws of Thought* (1854). The fold pass handles
arithmetic identities (`x + 0 → x`, `x * 1 → x`) but completely ignores Boolean
algebra. These are O(1) pattern matches with zero risk.

**Missing laws:**
```
x || true    → true        (Annihilation)
x || false   → x           (Identity)
x && true    → x           (Identity)
x && false   → false       (Annihilation)
x || x       → x           (Idempotence)
x && x       → x           (Idempotence)
!!x          → x           (Double negation / Involution)
!(x && y)    → !x || !y    (De Morgan I)
!(x || y)    → !x && !y    (De Morgan II)
```

**Why it matters:** `If x and true:` appears in generated code from macro expansions
and conditional compilation patterns. Without these, DCE can't eliminate trivially-true
guards.

**TDD tests:**
```
fold_bool_or_true — x || true -> true
fold_bool_or_false — x || false -> x
fold_bool_and_true — x && true -> x
fold_bool_and_false — x && false -> false
fold_bool_or_idempotent — x || x -> x
fold_bool_and_idempotent — x && x -> x
fold_bool_double_negation — !!x -> x
fold_bool_demorgan_and — !(x && y) -> !x || !y
fold_bool_demorgan_or — !(x || y) -> !x && !y
```

### 0h: Self-Comparison Identities

**File:** `optimize/fold.rs`

When fold sees `BinaryOp(op, left, right)` where `left` and `right` are the SAME
identifier symbol:

```
x == x  → true     (Reflexivity of equality)
x != x  → false    (Irreflexivity)
x <= x  → true     (Reflexivity of ≤)
x >= x  → true     (Reflexivity of ≥)
x < x   → false    (Irreflexivity of strict order)
x > x   → false    (Irreflexivity of strict order)
x - x   → 0        (Additive inverse)
x / x   → 1        (Multiplicative inverse, guard x ≠ 0)
x % x   → 0        (Modular identity, guard x ≠ 0)
x xor x → 0        (Self-XOR, Knuth TAOCP 7.1.3)
```

**Safety:** Only when both operands are the SAME `Symbol`. Cannot apply to arbitrary
expressions (side effects).

**TDD tests:**
```
fold_self_eq — x == x -> true
fold_self_neq — x != x -> false
fold_self_leq — x <= x -> true
fold_self_geq — x >= x -> true
fold_self_lt — x < x -> false
fold_self_gt — x > x -> false
fold_self_sub — x - x -> 0
fold_self_div — x / x -> 1 (when x provably ≠ 0)
fold_self_mod — x % x -> 0 (when x provably ≠ 0)
fold_self_xor — x xor x -> 0
fold_self_div_no_fold_unknown — x / x unchanged when x could be 0
```

### 0i: Local CSE / Value Numbering

**File:** New `optimize/gvn.rs` or integrated into `optimize/propagate.rs`

**The literature:** John Cocke, "Global Common Subexpression Elimination" (1970).
Assign each expression a "value number." If two expressions have the same value
number, they compute the same value. Eliminate the second computation.

**Data structure:** Hash-cons table mapping `(operator, vn(left), vn(right))` to a
value number. Two expressions get the same value number iff they have the same
operator and their operands have the same value numbers.

```
a = x + y      // vn(a) = vn(+, vn(x), vn(y)) = V1
b = x + y      // vn(b) = vn(+, vn(x), vn(y)) = V1 — same as a!
c = a + b      // → c = a + a (substitute b with a)
```

**Scope:** Local CSE only — within a single basic block (no control flow). No effect
system needed for intra-block CSE, just a hash table scoped to the block. Kills the
table entry when any operand is written.

**Why it matters:** In the spectral norm benchmark, `mulAv` computes
`1.0 / ((i+j)*(i+j+1)/2 + i + 1)` in the inner loop. The subexpression `i+j` is
computed twice. CSE would compute it once. Across all benchmarks, CSE typically
recovers 5-15% of redundant computation.

**TDD tests:**
```
cse_same_expression — Let a = x + y. Let b = x + y. -> b reuses a
cse_invalidated_by_write — Let a = x + y. Set x to 10. Let b = x + y. -> NOT reused
cse_nested_subexpression — Let a = (x + y) * (x + y). -> (x + y) computed once
cse_different_operators — Let a = x + y. Let b = x * y. -> NOT reused (different op)
cse_across_blocks_not_applied — CSE does not cross If/While boundaries
```

### 0j: Compile-Time Function Evaluation (CTFE)

**File:** New `optimize/compile_eval.rs`

**The literature:** Futamura, "Partial Evaluation of Computation Process" (1971).
First Futamura projection: specializing an interpreter with a program produces
a compiled version of that program.

**The idea:** Pure functions called with all-literal arguments can be evaluated at
compile time using the existing sync interpreter. This is the simplest form of
supercompilation — the compiler IS an interpreter that also emits code.

**Algorithm:**
1. Identify pure functions via `collect_pure_functions()` (check: no IO, no Write
   to non-local variables, no Escape blocks).
2. At each call site, check if all arguments are literals.
3. If pure + all-literal: run the sync interpreter with a step limit (10,000 steps)
   on the function body with the literal arguments.
4. If evaluation completes within the limit: convert the `RuntimeValue` result back
   to `Literal` and replace the call with the literal.
5. If evaluation exceeds the step limit (infinite recursion, huge computation): fall
   back to normal compilation.

**Prerequisites:**
- Step-limited interpreter (add `max_steps` parameter to interpreter entry point)
- `RuntimeValue → Literal` conversion path (new function)
- Purity check via effect system or ad-hoc `collect_pure_functions`

**TDD tests:**
```
ctfe_pure_constant_call — ## To add5(x: Int) -> Int: Return x + 5. Let y = add5(10). -> Let y = 15.
ctfe_recursive_pure — ## To fib(n: Int) -> Int: ... Let y = fib(10). -> Let y = 55.
ctfe_step_limit_exceeded — ## To infinite(): infinite(). infinite(). -> preserved (not evaluated)
ctfe_impure_not_evaluated — ## To greet(x: Text): Show x. greet("hi"). -> preserved (IO)
ctfe_partial_args_not_evaluated — add5(x). -> preserved (x is not literal)
```

### 0k: Trait-Dispatch Elimination for Vec Indexing

**File:** `codegen/expr.rs` and/or `codegen/stmt.rs`

**Why this is Camp 0:** The histogram benchmark is 10.62x slower than C — the single
worst benchmark. The root cause: codegen emits `LogosIndex::logos_get(&counts, (v + 1))`
(trait dispatch through `LogosIndex`) where C uses `counts[v]` (direct array access).
When the collection type is statically known to be `Vec<T>`, emit direct indexing
instead of trait dispatch.

**Detection:** When the type system resolves the collection in `item N of collection`
as `Vec<i64>`, `Vec<f64>`, `Vec<bool>`, or `Vec<String>`, emit:
```rust
collection[(index - 1) as usize]          // direct index (read)
collection[(index - 1) as usize] = value  // direct index (write)
```
instead of:
```rust
LogosIndex::logos_get(&collection, index)           // trait dispatch (read)
LogosIndex::logos_set(&mut collection, index, value) // trait dispatch (write)
```

**Safety:** Only when the type is statically known. For generic or unknown types,
preserve trait dispatch.

**Estimated impact:** 5-10x improvement on histogram. 2-3x on counting_sort. Significant
improvement on any benchmark that does heavy indexed access.

**TDD tests:**
```
trait_elim_vec_i64_read — item i of vec_of_int -> direct index, no LogosIndex
trait_elim_vec_i64_write — Set item i of vec to val -> direct index assignment
trait_elim_map_preserved — item k of map_of_int_int -> LogosIndex preserved (not Vec)
trait_elim_e2e_histogram — histogram produces correct output with direct indexing
trait_elim_e2e_counting_sort — counting sort correct with direct indexing
```

### 0l: 1-Based → 0-Based Index Lowering in For-Range

**File:** `codegen/peephole.rs`

**The problem:** Every array access in LOGOS generates `(i - 1) as usize` because
LOGOS uses 1-based indexing. Inside a for-range that iterates `1..=n` and only uses
the counter for indexing (`item i of arr` → `arr[(i-1) as usize]`), this subtraction
is pure overhead.

**The fix:** Rewrite the range to `0..n` and index as `arr[i as usize]`. Eliminates
one subtraction per array access per loop iteration.

**Detection:** In `try_emit_for_range_pattern`, if:
1. The for-range starts at 1 (`start == Literal(1)`)
2. The counter variable `i` is ONLY used in `item i of collection` expressions
   (no other uses of `i` in the loop body)
3. All indexing uses the bare counter (not `i + offset` or `i * stride`)

Then: emit `for i in 0..n` and replace `arr[(i - 1) as usize]` with `arr[i as usize]`.

**The peephole already detects for-range patterns — this extends it to perform the
index base transformation.**

**Estimated impact:** Saves one subtraction per array access. For tight loops (sieve,
prefix_sum, array operations), this is 5-10% of the loop body cost.

**TDD tests:**
```
index_lower_simple — for i from 1 to n: item i of arr -> for i in 0..n: arr[i]
index_lower_non_unit_start — for i from 5 to n: item i of arr -> NOT lowered (start ≠ 1)
index_lower_counter_used_elsewhere — for i from 1 to n: Show i. item i of arr -> NOT lowered
index_lower_e2e_correctness — verify identical output with lowered indices
```

---

## CAMP 1 — Effect System

### Sprint 1 (Foundation)

**The Idea:** Classify every expression and statement by its computational effects.
Everything above this camp depends on knowing what's pure, what reads, what writes,
what does IO. This is the load-bearing wall of the entire mountain.

**The Math:** Effect lattices. Effects form a partial order:
```
Pure < Read(v) < Write(v) < Consume(v) < Alloc < IO < SecurityCheck < Diverge < Unknown
```
Join combines effects: `Pure join Read(x) = Read(x)`.
A function's effect is the join of its body's effects.
Recursive functions use fixed-point iteration.

**Data Structures:**

```rust
pub enum Effect {
    /// No observable effects. Safe to reorder, eliminate, memoize, parallelize.
    Pure,
    /// Reads from variables but does not write. Safe to CSE if writes don't intervene.
    Read(HashSet<Symbol>),
    /// Writes to variables. Creates ordering dependencies.
    Write(HashSet<Symbol>),
    /// Ownership transfer (Give). Source becomes dead, target gains value.
    Consume(HashSet<Symbol>),
    /// Allocates heap memory. Pure otherwise (e.g., Vec::new()).
    Alloc,
    /// Performs console/file IO. Must preserve ordering.
    IO,
    /// Security guard (Check). Non-eliminable, non-reorderable.
    SecurityCheck,
    /// May not terminate (unbounded recursion, infinite loops without proof).
    Diverge,
    /// Escape blocks, FFI, unknown. Conservative: assume all effects.
    Unknown,
}

pub struct EffectSet {
    pub reads: HashSet<Symbol>,
    pub writes: HashSet<Symbol>,
    pub consumes: HashSet<Symbol>,
    pub commutative_writes: HashSet<Symbol>,  // CRDT targets — enables auto-parallelization
    pub allocates: bool,
    pub io: bool,
    pub security_check: bool,
    pub diverges: bool,
    pub unknown: bool,
}

pub struct EffectEnv {
    pub functions: HashMap<Symbol, EffectSet>,
}
```

**New file:** `crates/logicaffeine_compile/src/optimize/effects.rs`

**Modified files:**
- `optimize/mod.rs` — add `pub mod effects;`
- `compile.rs` — run effect analysis after type checking, before optimization

**Reuse existing infrastructure:**
- Build on `CallGraph` from `analysis/callgraph.rs` (SCC iteration order)
- Informed by `TypeEnv` from `analysis/types.rs` (type-dependent effects)
- Designed to REPLACE ad-hoc detection functions in `codegen/detection.rs`:
  `collect_mutable_vars()`, `collect_async_functions()`, `collect_pipe_vars()`

**Algorithm:**

1. **Expression effects:** Bottom-up traversal.
   - `Literal(_)` -> `Pure`
   - `Identifier(sym)` -> `Read({sym})`
   - `BinaryOp { left, right }` -> `join(effects(left), effects(right))`
   - `Call { function, args }` -> `join(lookup(function), effects(args))`
   - `Length { collection }` -> `Read({collection})`
   - `Index { collection, index }` -> `Read({collection}) join effects(index)`
   - `New { .. }` -> `Alloc`
   - `Escape { .. }` -> `Unknown`
   - `InterpolatedString { parts }` -> `join_all(effects(each hole expression))`
   - `Contains { collection, value }` -> `Read({collection}) join effects(value)`
   - `Union/Intersection { left, right }` -> `Read({left, right})`
   - `Slice { collection, .. }` -> `Read({collection})`
   - `Copy { source }` -> `Read({source}) join Alloc`
   - `Give { source }` -> `Consume({source})`
   - `FieldAccess { object, .. }` -> `Read({object})`
   - `List { elements }` -> `Alloc join join_all(effects(elements))`
   - `Tuple { elements }` -> `join_all(effects(elements))`
   - `Range { .. }` -> `Pure`
   - `Not { expr }` -> `effects(expr)`
   - `OptionSome { value }` -> `effects(value)`
   - `OptionNone` -> `Pure`
   - `WithCapacity { .. }` -> `Alloc`
   - `Closure { body }` -> `Pure` (definition, not invocation; body stored in EffectEnv)
   - `CallExpr { function, args }` -> `Unknown join effects(args)` (dynamic dispatch)
   - `ManifestOf { .. }` -> `Read(source) join Alloc`
   - `ChunkAt { .. }` -> `Read(source)`
   - `NewVariant { .. }` -> `Alloc`

2. **Statement effects — ALL 54 variants classified:**

| # | Variant | Effect | Notes |
|---|---------|--------|-------|
| 1 | `Let` | `Write({var}) join effects(value)` | |
| 2 | `Set` | `Write({target}) join effects(value) join Read(deps)` | |
| 3 | `Call` | `lookup(function) join effects(args)` | |
| 4 | `If` | `effects(cond) join effects(then) join effects(else)` | Conservative join of branches |
| 5 | `While` | `effects(cond) join effects(body) join Diverge` | May not terminate |
| 6 | `Repeat` | `Read({iterable}) join effects(body)` | |
| 7 | `Return` | `effects(value)` | |
| 8 | `Break` | `Pure` | Control flow, no data effect |
| 9 | `Assert` | `Pure` | Compile-time (logic kernel) |
| 10 | `Trust` | `Pure` | Compile-time |
| 11 | `RuntimeAssert` | `effects(condition)` | May panic but preservable |
| 12 | `Give` | `Consume({object}) join Write({recipient})` | Ownership transfer |
| 13 | `Show` | `IO join effects(value)` | |
| 14 | `SetField` | `Write({target}) join effects(value)` | |
| 15 | `StructDef` | `Pure` | Compile-time |
| 16 | `FunctionDef` | `Pure` | Definition, not invocation; body stored in EffectEnv |
| 17 | `Inspect` | `Read({target}) join join_all(arm_effects)` | Pattern matching, branch-join like If |
| 18 | `Push` | `Write({collection}) join effects(value)` | |
| 19 | `Pop` | `Write({collection})` | May also write binding |
| 20 | `Add` | `Write({set}) join effects(value)` | |
| 21 | `Remove` | `Write({set}) join effects(value)` | |
| 22 | `SetIndex` | `Write({collection}) join effects(index) join effects(value)` | |
| 23 | `Zone` | `Alloc join effects(body)` | Scoped allocation |
| 24 | `Concurrent` | `IO join Diverge join effects(tasks)` | tokio::join! |
| 25 | `Parallel` | `effects(tasks)` | rayon::join — CPU parallel, preserves purity |
| 26 | `ReadFrom` | `IO join Write({binding})` | |
| 27 | `WriteFile` | `IO join effects(content)` | |
| 28 | `Spawn` | `IO` | Agent spawn |
| 29 | `SendMessage` | `IO join effects(message)` | |
| 30 | `AwaitMessage` | `IO join Diverge join Write({binding})` | |
| 31 | `MergeCrdt` | `Write({target}) join Read({source})` | Commutative |
| 32 | `IncreaseCrdt` | `Write({target}) join effects(amount)` | Commutative |
| 33 | `DecreaseCrdt` | `Write({target}) join effects(amount)` | Commutative |
| 34 | `AppendToSequence` | `Write({target}) join effects(value)` | |
| 35 | `ResolveConflict` | `Write({target}) join effects(value)` | |
| 36 | `Check` | `SecurityCheck join effects(predicate)` | NEVER eliminate |
| 37 | `Listen` | `IO join Diverge` | |
| 38 | `ConnectTo` | `IO join Diverge` | |
| 39 | `LetPeerAgent` | `IO join Write({binding})` | |
| 40 | `Sleep` | `IO` | |
| 41 | `Sync` | `IO join Diverge` | |
| 42 | `Mount` | `IO join Write({binding})` | |
| 43 | `LaunchTask` | `IO join effects(call)` | |
| 44 | `LaunchTaskWithHandle` | `IO join Write({binding}) join effects(call)` | |
| 45 | `CreatePipe` | `Alloc join Write({binding})` | |
| 46 | `SendPipe` | `IO join Diverge join effects(value)` | May block |
| 47 | `ReceivePipe` | `IO join Diverge join Write({binding})` | May block |
| 48 | `TrySendPipe` | `IO join effects(value)` | Non-blocking |
| 49 | `TryReceivePipe` | `IO join Write({binding})` | Non-blocking |
| 50 | `StopTask` | `IO` | |
| 51 | `Select` | `IO join Diverge join join_all(branch_effects)` | |
| 52 | `Theorem` | `Pure` | Compile-time |
| 53 | `Escape` | `Unknown` | No analysis inside foreign code |
| 54 | `Require` | `Pure` | Compile-time |

3. **Fixed-point for recursion:** Initialize all functions as `Pure`. Iterate using
   SCC ordering from `analysis/callgraph.rs` until no function's effect set changes.
   Guaranteed to terminate because the lattice is finite and monotone.

4. **Commutative flag for auto-parallelization:** CRDT operations (variants 31-35)
   are commutative by definition. Tag their write targets in
   `EffectSet::commutative_writes`. This enables Camp 8 to parallelize loops
   containing CRDT operations.

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_effects.rs`

**RED tests to write first:**

```
effect_pure_literal
  Input:  Let x be 42.
  Assert: effect of `42` is Pure

effect_pure_arithmetic
  Input:  Let x be 2 + 3 * 4.
  Assert: effect of `2 + 3 * 4` is Pure

effect_read_variable
  Input:  Let y be x + 1.
  Assert: effect of `x + 1` is Read({x})

effect_write_let
  Input:  Let x be 5.
  Assert: effect of Let statement is Write({x})

effect_write_set
  Input:  Set x to x + 1.
  Assert: effect is Read({x}) join Write({x})

effect_write_push
  Input:  Push 5 to items.
  Assert: effect is Write({items})

effect_io_show
  Input:  Show x.
  Assert: effect is IO join Read({x})

effect_alloc_new
  Input:  Let items be a new Seq of Int.
  Assert: effect includes Alloc

effect_unknown_escape
  Input:  Escape rust { println!("hi"); }
  Assert: effect is Unknown

effect_pure_function
  Input:  ## To double (x: Int) -> Int:\n    Return x * 2.
  Assert: double's effect is Pure

effect_io_function
  Input:  ## To greet (name: Text):\n    Show name.
  Assert: greet's effect is IO

effect_write_function
  Input:  ## To fill (items: Seq of Int):\n    Push 1 to items.
  Assert: fill's effect is Write({items})

effect_transitive_pure
  Input:  ## To f (x: Int) -> Int:\n    Return double(x) + 1.
  Assert: f's effect is Pure (because double is Pure)

effect_transitive_io
  Input:  ## To f (x: Int):\n    greet("hello").
  Assert: f's effect is IO (transitive through greet)

effect_recursive_pure
  Input:  ## To factorial (n: Int) -> Int:\n    If n is at most 1: Return 1.\n    Return n * factorial(n - 1).
  Assert: factorial's effect is Pure (recursive but pure)

effect_recursive_diverge
  Input:  ## To loop ():\n    loop().
  Assert: loop's effect includes Diverge

effect_length_is_read
  Input:  Let n be length of items.
  Assert: effect is Read({items})

effect_index_is_read
  Input:  Let x be item 5 of items.
  Assert: effect is Read({items})

effect_contains_is_read
  Input:  Let b be 5 in items.
  Assert: effect is Read({items})

effect_consume_give
  Input:  Give items to recipient.
  Assert: effect is Consume({items}) join Write({recipient})

effect_security_check
  Input:  Check that user has "admin" permission.
  Assert: effect includes SecurityCheck, is non-eliminable

effect_crdt_commutative
  Input:  Increase counter by 1.
  Assert: effect is Write({counter}), commutative_writes includes {counter}

effect_interpolated_string
  Input:  Let msg be "{name} has {count} items".
  Assert: effect is Read({name, count})

effect_inspect_branches
  Input:  Inspect shape: When Circle(r): Show r. When Square(s): Show s.
  Assert: effect is Read({shape}) join IO

effect_pipe_send_diverge
  Input:  Send 42 to pipe.
  Assert: effect is IO join Diverge (may block)

effect_try_send_no_diverge
  Input:  Try to send 42 to pipe.
  Assert: effect is IO (non-blocking, no Diverge)
```

**Negative tests (edge cases):**

```
effect_while_may_diverge
  Input:  While true: Show 1.
  Assert: effect includes Diverge AND IO

effect_if_conservative
  Input:  If condition: Push 1 to items. Otherwise: Show "no".
  Assert: effect is Write({items}) join IO join Read({condition})
  Reason: Both branches contribute effects (conservative)

effect_closure_captures
  Input:  Let f be () => Show x.
  Assert: closure's effect is IO join Read({x})

effect_set_index_is_write
  Input:  Set item 5 of items to 10.
  Assert: effect is Write({items})

effect_no_false_pure
  Verify: No function with IO/Write in its body is classified as Pure
  Method: Property test over all benchmark programs

effect_check_never_eliminated
  Input:  If false: Check that x. (dead branch containing Check)
  Assert: Check is preserved even in dead code context
  Method: Assert SecurityCheck flag prevents DCE removal
```

**Verification:** `cargo test --test phase_effects`

### Edge Cases to Watch

1. **Mutual recursion:** `f` calls `g`, `g` calls `f`. Fixed-point must handle cycles.
   Use SCC ordering from `analysis/callgraph.rs`.
2. **Higher-order functions:** `Repeat for x in items: f(x)` — effect depends on `f`.
   Conservative: if `f` is passed as argument and unknown, assume Unknown.
3. **Escape blocks:** Always `Unknown`. No analysis inside foreign code.
4. **Native functions:** Effect must be annotated or conservatively assumed IO.
   For now: all native functions -> IO (safe overapproximation).
5. **Collection iteration reads:** `Repeat for x in items` reads `items` on every
   iteration. If body writes `items` (Push), that's a read-write conflict.

### What This Clears the Path For

Every camp above uses the effect system:
- **Camp 3 (Abstract Interp):** Needs to know which variables are written in a loop body
- **Camp 4 (LICM):** Hoist expressions whose reads don't conflict with loop writes
- **Camp 6 (Deforestation):** Prove intermediate collections are consumed linearly
- **Camp 8 (Auto-Parallel):** Bernstein's conditions require read/write sets

---

## CAMP 2 — Return-Value-As-Output-Parameter + For-In Reference Iteration

### Sprint 2

Two codegen improvements that leverage the effect system and readonly analysis.

### 2a: Return-Value-As-Output-Parameter

**The Idea:** The pattern `Set arr to f(arr, ...)` where `f` returns a modified
collection appears in `mergesort`, `heap_sort`, and `quicksort`. Currently each call:
1. Moves `arr` into `f`
2. `f` creates a modified version
3. `f` returns it
4. Caller rebinds `arr`

This is semantically identical to `f(&mut arr, ...)`.

**Detection:** If a function (a) takes a `Seq<T>` parameter as its last param,
(b) returns `Seq<T>`, (c) every return path returns that parameter (possibly mutated),
and (d) every call site immediately rebinds via `Set x to f(x, ...)` — then convert
to `f(x: &mut [T])` with no return.

**The readonly analysis (`analysis/readonly.rs`) already handles `Seq<T>` -> `&[T]`
and `&mut [T]`, but NOT the "take ownership, return modified" pattern.

**Impact:** Would close the 1.6-1.8x gaps in mergesort and heap_sort by eliminating
recursive allocation.

**TDD tests:**
```
output_param_detected
  Source: ## To fill (items: Seq of Int) -> Seq of Int: Push 1 to items. Return items.
          Let items be a new Seq of Int. Set items to fill(items).
  Assert: generated code uses &mut [i64] parameter, no return

output_param_not_detected_different_return
  Source: ## To transform (items: Seq of Int) -> Seq of Int: Let new_items be ... Return new_items.
  Assert: NOT converted (returns different variable)

output_param_e2e_mergesort
  E2E: mergesort produces correct sorted output with optimization enabled
```

### 2b: For-In Reference Iteration

**The Idea:** The codegen currently emits `for x in collection.clone()` for every
`Repeat for x in items:`. When the loop body doesn't mutate `items`, this clone is
unnecessary. Emitting `for x in &items` (reference iteration) eliminates the clone.

The readonly analysis already knows which collections are mutated. Thread this info
to the for-in codegen.

**Impact:** Eliminates an O(n) clone for every non-mutating for-in loop.

**TDD tests:**
```
ref_iter_no_mutation — body reads only -> for x in &items (no clone)
ref_iter_with_mutation — body pushes to items -> for x in items.clone() (clone preserved)
ref_iter_e2e_correctness — verify identical output with ref iteration
```

---

## CAMP 3 — Abstract Interpretation (Value Range Analysis)

### Sprint 3

**The Idea:** Track the possible range of every integer variable at every program point.
Use this to eliminate bounds checks, dead branches, and prove loop termination.

**The Math:** Interval abstract domain. Each variable maps to an interval `[lo, hi]`
where `lo, hi` are in `Z union {-inf, +inf}`. Operations:

```
[a,b] + [c,d] = [a+c, b+d]
[a,b] - [c,d] = [a-d, b-c]
[a,b] * [c,d] = [min(ac,ad,bc,bd), max(ac,ad,bc,bd)]
[a,b] / [c,d] = [a,b] * [1/d, 1/c]  (if 0 not in [c,d])

Join: [a,b] join [c,d] = [min(a,c), max(b,d)]
Meet: [a,b] meet [c,d] = [max(a,c), min(b,d)]  (empty if max>min)
```

**Widening** (ensures termination at loop heads):
```
[a,b] widen [c,d] = [if c<a then -inf else a, if d>b then +inf else b]
```

**Narrowing** (recovers precision after widening):
```
[a,b] narrow [c,d] = [if a=-inf then c else a, if b=+inf then d else b]
```

**Data Structures:**

```rust
pub struct Interval {
    pub lo: Bound,
    pub hi: Bound,
}

pub enum Bound {
    NegInf,
    Finite(i64),
    PosInf,
}

pub type AbstractState = HashMap<Symbol, Interval>;

pub struct RangeEnv {
    pub states: Vec<AbstractState>,
    pub proven_bounds: Vec<ProvenBound>,
}

pub struct ProvenBound {
    pub collection: Symbol,
    pub index_interval: Interval,
    pub collection_length: Interval,
    pub is_safe: bool,  // true if index_interval is within [1, collection_length]
}
```

**New file:** `crates/logicaffeine_compile/src/optimize/abstract_interp.rs`

**Modified files:**
- `optimize/mod.rs` — add `pub mod abstract_interp;`
- `compile.rs` — run after effects, before codegen
- `codegen/peephole.rs` — use `RangeEnv` for general bounds check elimination
  (replaces the hand-coded single-pattern `assert_unchecked`)

**Algorithm:**

1. **Initialize:** All variables -> bottom (unreachable). Entry point variables -> from
   type/annotation (e.g., function param `n: Int` -> `[-inf, +inf]`).

2. **Forward analysis:** Process statements in order:
   - `Let x = Literal(5)` -> `x in [5, 5]`
   - `Let x = a + b` -> `x in range(a) + range(b)`
   - `Set x to expr` -> update `x` to `range(expr)`
   - `Push val to items` -> `items.length += [1, 1]`

3. **Conditionals:** Branch narrowing.
   - `If x > 0:` -> in then-branch: `x in range(x) meet [1, +inf]`
   - `If x > 0: ... Otherwise:` -> in else-branch: `x in range(x) meet [-inf, 0]`

4. **Loops:** Fixed-point with widening.
   - First iteration: propagate normally
   - If loop head state changes: apply widening
   - Iterate until stable
   - Apply narrowing for one pass to recover precision

5. **Index safety:** At each `Index { collection, index }`:
   - Look up `range(index)` and `range(collection.length)`
   - If `range(index)` is within `[1, range(collection.length)]` -> proven safe ->
     emit `unsafe { assert_unchecked!(...) }` or eliminate bounds check entirely

### Widening Strategy: Bourdoncle (1993)

**The literature:** François Bourdoncle, "Efficient Chaotic Iteration Strategies with
Widenings" (1993).

Instead of widening at every loop head on every iteration, use Bourdoncle's hierarchical
iteration strategy: compute strongly connected components in the control flow graph,
widen only at the headers of each SCC. This converges faster and produces tighter bounds.

The existing `analysis/callgraph.rs` already computes SCCs for function calls. Extend
this to compute SCCs over basic blocks within a function for the widening schedule.

### Stretch Goal: Octagon Domain (Miné, 2006)

**The literature:** Antoine Miné, "The Octagon Abstract Domain" (2006).

The interval domain cannot track relationships between variables. For the pattern
`i*n + j` (2D array indexing), the interval domain can't prove `i*n + j < n*n`
when `i < n` and `j < n`, because it doesn't know how `i`, `n`, and `j` relate.

The octagon domain tracks constraints of the form `±x ± y ≤ c` — enough to prove
these relational properties. This is the key enabler for 2D array bounds check
elimination in the polyhedral tiling camp (Camp 7).

**Not required for initial implementation.** The interval domain handles all 1D
bounds checking. The octagon domain becomes relevant when Camp 7 (polyhedral tiling)
needs to prove safety of tiled 2D accesses.

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_abstract_interp.rs`

**RED tests:**

```
range_literal
  Input:  Let x be 42.
  Assert: x in [42, 42]

range_arithmetic_add
  Input:  Let x be 3. Let y be x + 7.
  Assert: y in [10, 10]

range_arithmetic_multiply
  Input:  Let x be 5. Let y be x * 3.
  Assert: y in [15, 15]

range_variable_unknown
  Input:  ## To f (n: Int) -> Int:
  Assert: n in [-inf, +inf]

range_conditional_narrowing_then
  Input:  If n is greater than 0: Let x be n.
  Assert: in then-branch, n in [1, +inf], x in [1, +inf]

range_conditional_narrowing_else
  Input:  If n is greater than 0: ... Otherwise: Let x be n.
  Assert: in else-branch, n in [-inf, 0]

range_for_loop_counter
  Input:  Repeat for i from 1 to 100: ...
  Assert: inside loop body, i in [1, 100]

range_for_loop_counter_variable_bound
  Input:  Repeat for i from 1 to n: ...
  Assert: inside loop body, i in [1, n] (symbolic)

range_while_loop_widening
  Input:  Let i be 0. While i < 1000: Set i to i + 1.
  Assert: at loop head after widening, i in [0, +inf]
  Assert: after narrowing, i in [0, 999] or [0, 1000]

range_collection_length_after_fill
  Input:  Let items be a new Seq of Int.
          Repeat for i from 1 to n:
              Push i to items.
  Assert: after loop, length(items) in [n, n] (if n >= 0)

range_bounds_check_proven_safe
  Input:  Let items be a new Seq of Int.
          Repeat for i from 1 to 100:
              Push 0 to items.
          Let x be item 50 of items.
  Assert: index 50 in [1, 100] is within [1, len(items)] -> safe
  Assert: generated code has NO bounds check (or has assert_unchecked)

range_bounds_check_unsafe
  Input:  Let x be item n of items.
  Assert: n in [-inf, +inf], cannot prove safe -> keep bounds check

range_dead_branch_elimination
  Input:  Let x be 5.
          If x is greater than 100:
              Show "impossible".
  Assert: x in [5, 5], 5 > 100 is false -> branch eliminated

range_set_updates_range
  Input:  Let x be 5. Set x to x + 10.
  Assert: after Set, x in [15, 15]

range_push_increments_length
  Input:  Let items be a new Seq of Int.
          Push 1 to items.
          Push 2 to items.
          Push 3 to items.
  Assert: length(items) in [3, 3]
```

**Negative / edge-case tests:**

```
range_overflow_saturates
  Input:  Let x be 9223372036854775807. Let y be x + 1.
  Assert: y in [-inf, +inf] (overflow -> widen to full range, conservative)

range_division_by_zero_excluded
  Input:  Let y be x / n.
  Assert: if n in [0, 0], result is bottom (undefined) or runtime error preserved

range_conditional_join
  Input:  If condition: Set x to 5. Otherwise: Set x to 10.
  Assert: after If, x in [5, 10] (join of both branches)

range_nested_loop
  Input:  Repeat for i from 1 to n:
              Repeat for j from 1 to i:
                  ...
  Assert: in inner body, j in [1, i], i in [1, n]

range_loop_never_executes
  Input:  Let i be 0. While i < 0: Set i to i + 1.
  Assert: after loop, i in [0, 0] (loop body never ran)

range_function_call_unknown_return
  Input:  Let x be computeSomething(n).
  Assert: x in [-inf, +inf] unless computeSomething is analyzed
```

### Edge Cases to Watch

1. **Widening too early:** Widen only after the second iteration at a loop head, not
   the first. First iteration establishes the base; second detects growth direction.
2. **Negative ranges:** `Repeat for i from n to 1` (counting down) — range is `[1, n]`
   but iteration direction matters.
3. **Symbolic bounds:** `Repeat for i from 1 to length of items` — the length is
   symbolic. Track it as a symbolic interval `[1, len(items)]` where `len(items)` is
   a separate tracked quantity.
4. **Collection length changes INSIDE loops:** `Push` inside a for-loop changes the
   length of the target collection on each iteration. Must update length range
   incrementally.
5. **Overflow:** i64 arithmetic can overflow. When arithmetic approaches i64::MAX or
   i64::MIN, widen the interval to `[-inf, +inf]` (safe overapproximation).

---

## CAMP 4 — LICM + DSE + Loop Unswitching + Peeling

### Sprint 4

**The Idea:** Four loop transforms that leverage the effect system and range analysis.

**LICM (Loop-Invariant Code Motion):** If an expression inside a loop body is `Pure`
or `Read(vars)` where none of `vars` are written in the loop body, hoist it above the
loop.

**Dead Store Elimination:** If a variable is written twice with no intervening read,
the first write is dead. Remove it.

**Loop Unswitching (Allen, 1969):** If a loop contains a branch whose condition is
loop-invariant, hoist the branch outside the loop and duplicate the loop body into
each branch. Eliminates a branch prediction miss on every iteration.

```
// Before:
while i < n:
    if flag:          // flag doesn't change in loop
        a[i] = x
    else:
        a[i] = y
    i += 1

// After:
if flag:
    while i < n:
        a[i] = x
        i += 1
else:
    while i < n:
        a[i] = y
        i += 1
```

Doubles code size but removes a branch from the hot loop. Only apply when the branch
condition is provably loop-invariant (via the effect system: condition reads no variables
written in the loop body).

**Loop Peeling (Muchnick, 1997):** Peel the first and/or last iteration of a loop to
simplify the loop body:

```
// Before (with boundary check):
for i in 0..n:
    if i == 0: handle_first()
    else if i == n-1: handle_last()
    else: handle_middle()

// After peeling:
handle_first()
for i in 1..n-1:
    handle_middle()          // No branches!
handle_last()
```

The peeled loop body has no branches, enabling better vectorization and pipelining.

**Partial Redundancy Elimination (Morel & Renvoise, 1979):** PRE subsumes LICM and
CSE as special cases. An expression is "partially redundant" if it's available on some
paths to a point but not all. PRE inserts computations on the missing paths to make it
"fully redundant," then eliminates. PRE = LICM + CSE + code hoisting in one unified
framework. Noted here as the theoretical unification — the practical implementation
uses LICM + local CSE (Camp 0i) separately.

**IMPORTANT:** LICM runs as an AST-level pass in `optimize/` on While loops, BEFORE
the codegen peephole chain that converts While to for-range. This gives LICM access
to the original loop structure.

**Modified files:**
- New `optimize/licm.rs` — LICM pass, loop unswitching, loop peeling
- `optimize/dce.rs` — extend with dead store elimination

**LICM Algorithm:**

```
For each While/Repeat loop:
  loop_writes = collect_all_writes_in_body(body)  // from effect system
  For each expression E in body:
    if effects(E).reads intersection loop_writes = empty AND effects(E) is Pure or Read-only:
      // Safety check for zero-trip loops (Pitfall 11):
      if E may panic AND loop may execute zero times (from abstract interpretation):
        skip hoisting (would change behavior)
      else:
        hoist E above the loop
        replace E in body with the hoisted variable
```

**Dead Store Algorithm:**

```
For each block:
  last_write: HashMap<Symbol, StmtIndex> = {}
  reads_since_write: HashMap<Symbol, bool> = {}
  For each statement S:
    // INVARIANT 11: Never eliminate SecurityCheck
    if S is Stmt::Check: skip DSE for this statement
    For each var read by S:
      reads_since_write[var] = true
    For each var written by S:
      if last_write[var] exists AND !reads_since_write[var]:
        mark last_write[var] as dead
      last_write[var] = S.index
      reads_since_write[var] = false
  Remove all dead writes
```

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_licm.rs`

**RED tests — LICM:**

```
licm_hoist_length
  Input:  While i < length of items:
              Let x be item i of items.
              Set i to i + 1.
  Assert: `length of items` computed once, before loop
  Verify: generated code has `let _len = items.len();` before loop

licm_no_hoist_when_written
  Input:  While i < length of items:
              Push i to items.
              Set i to i + 1.
  Assert: `length of items` NOT hoisted (items is written by Push)

licm_hoist_pure_call
  Input:  ## To square (x: Int) -> Int: Return x * x.
          While i < n:
              Let s be square(5).
              Set i to i + 1.
  Assert: `square(5)` hoisted above loop (pure function, constant arg)

licm_no_hoist_io
  Input:  While i < n:
              Show i.
              Set i to i + 1.
  Assert: `Show i` NOT hoisted (IO effect)

licm_hoist_nested
  Input:  While i < n:
              While j < m:
                  Let x be n * m.
                  ...
  Assert: `n * m` hoisted to OUTER loop level (invariant in both)

licm_hoist_complex_expression
  Input:  While i < n:
              Let x be (a + b) * (c + d).
              ...
  Assert: entire expression hoisted if a, b, c, d not written in loop

licm_partial_hoist
  Input:  While i < n:
              Let x be a + i.
  Assert: `a` is loop-invariant but `i` is not -> expression NOT hoisted
          (subexpression hoisting is a later optimization)

licm_no_hoist_panicking_zero_trip
  Input:  While i < 0:
              Let x be 1 / y.
  Assert: `1 / y` NOT hoisted (loop never executes, hoisting would panic if y=0)
```

**RED tests — Dead Store Elimination:**

```
dse_consecutive_writes
  Input:  Let mutable x be 5. Set x to 10. Show x.
  Assert: `Let x = 5` eliminated (immediately overwritten, never read)

dse_no_eliminate_when_read
  Input:  Let mutable x be 5. Show x. Set x to 10. Show x.
  Assert: Both writes preserved (x is read between them)

dse_loop_write_not_dead
  Input:  Let mutable x be 0.
          While x < 10:
              Set x to x + 1.
  Assert: Initial write preserved (read in condition)

dse_conditional_conservative
  Input:  Let mutable x be 5.
          If condition:
              Set x to 10.
          Show x.
  Assert: Initial write NOT eliminated (condition may be false)

dse_check_never_eliminated
  Input:  Check that user has "admin" permission.
          Check that user has "admin" permission.
  Assert: Both Check statements preserved (SecurityCheck is non-eliminable)

dse_push_not_dead
  Input:  Push 1 to items. Push 2 to items.
  Assert: Both pushes preserved (additive, not overwriting)
```

**RED tests — Loop Unswitching:**

```
unswitch_invariant_branch
  Input:  While i < n:
              If flag:
                  Set item i of result to x.
              Otherwise:
                  Set item i of result to y.
              Set i to i + 1.
  Assert: `if flag` hoisted outside loop, two loop versions generated
  Verify: generated code has `if flag { for ... } else { for ... }` structure

unswitch_no_fire_variant_condition
  Input:  While i < n:
              If i > 5:
                  Set item i of result to x.
              Set i to i + 1.
  Assert: NOT unswitched (condition depends on loop counter i)

unswitch_no_fire_written_condition
  Input:  While i < n:
              If flag:
                  Set flag to false.
              Set i to i + 1.
  Assert: NOT unswitched (flag is written inside loop)
```

**RED tests — Loop Peeling:**

```
peel_first_iteration
  Input:  Repeat for i from 1 to n:
              If i equals 1: handle_first(). Otherwise: handle_rest().
  Assert: first iteration peeled, inner loop body has no i==1 branch

peel_last_iteration
  Input:  Repeat for i from 1 to n:
              If i equals n: handle_last(). Otherwise: handle_rest().
  Assert: last iteration peeled, inner loop body has no i==n branch

peel_no_fire_non_boundary
  Input:  Repeat for i from 1 to n:
              If i equals 5: special(). Otherwise: normal().
  Assert: NOT peeled (branch condition is not on loop boundary)
```

**Verification:** All loop transforms run `cargo test -- --skip e2e` with zero regressions.
Also run benchmarks to measure improvement (expect 5-15% on loop-heavy benchmarks).

### Edge Cases to Watch

1. **LICM + for-range interaction:** LICM runs BEFORE the peephole (on While form).
2. **LICM safety with exceptions:** Only hoist if either (a) the expression can't panic,
   or (b) the loop provably executes at least once (from abstract interpretation).
3. **DSE across function boundaries:** A write to a variable that's passed to a function
   might be "read" inside that function. Use effect system to check.
4. **DSE with collections:** `Push 1 to items. Push 2 to items.` — these are NOT dead
   stores (each Push is additive). Only `Set` to the SAME target is dead.

---

## CAMP 5 — Closed-Form Recognition + Strength Reduction

### Sprint 5

### 5a: Closed-Form Recognition

**The Idea:** Recognize that a counted loop computes a known mathematical function and
replace it with the closed-form formula. O(n) -> O(1).

**The Math:** The loop body defines a recurrence relation on accumulator variables. If
the recurrence is linear and the step function is a polynomial, geometric, or otherwise
in our table of known forms, replace it.

**Caveat (Pitfall 15):** Every benchmark that does summation uses `% 1000000007`
(modular arithmetic). The formula table includes modular variants where applicable.
`sum(i, 1..n) mod M` requires computing `n*(n+1)/2 mod M` using modular arithmetic
(which is safe for multiplication/addition but not division — use modular inverse).

**Modified file:** `codegen/peephole.rs` — new function `try_emit_closed_form_pattern`

**Detection algorithm:**

1. Identify a for-range loop (already detected by existing peephole).
2. Find the accumulator: a variable `acc` initialized before the loop and updated via
   `Set acc to acc OP f(counter)` inside the loop body.
3. Verify:
   - The loop body has no other side effects (Pure except for acc write — use effects).
   - `f(counter)` is a known function of the counter variable only.
   - The accumulator operation OP is `+` (summation) or `*` (product).
4. Match `f(counter)` against the table:

| `f(counter)` | OP | Closed Form | Guard |
|---|---|---|---|
| `1` (constant) | `+` | `n - start + 1` | n >= start |
| `counter` | `+` | `(n - start + 1) * (start + n) / 2` | n >= start |
| `counter * counter` | `+` | `n*(n+1)*(2*n+1)/6 - (start-1)*start*(2*start-1)/6` | n >= start |
| `counter * counter * counter` | `+` | `(n*(n+1)/2)^2 - ((start-1)*start/2)^2` | n >= start |

5. Emit the closed form with a guard: `if n >= start { formula } else { init_value }`.

### 5b: Strength Reduction

**The literature:** Frances Allen and John Cocke, "A Catalogue of Optimizing
Transformations," IBM Research Report RC 3548 (1971). The definitive enumeration
of what a compiler can do to code.

**The Idea:** The pattern `item (i * n + j + 1) of flat_matrix` appears in
matrix_mult and other 2D-as-1D benchmarks. The `i * n` computation is loop-invariant
in the inner `j` loop. Classic strength reduction converts `i*n + j` to an
incrementing base pointer.

LICM (Camp 4) can hoist `i * n`, but strength reduction goes further: replace the
multiplication with an addition in the inner loop (`base += 1` instead of recomputing
`i*n+j`).

**Detection:** In a nested loop where the outer counter `i` and inner counter `j` are
used in an expression `i * n + j`, introduce `base = i * n` before the inner loop and
replace `i * n + j` with `base + j` (or increment `base` each iteration).

**Induction variable analysis (Cocke & Kennedy, 1977):** If `f(i) = a*i + b` is an
affine function of loop counter `i`, then `f(i+1) - f(i) = a` is constant. Replace
`f(i)` with a running accumulator initialized to `f(start)` and incremented by `a`
each iteration. This converts O(1) multiplies to O(1) additions — same asymptotic
but 3-4x faster on modern CPUs where multiply latency is 3-4 cycles vs. add at 1 cycle.

```
arr[i * stride]     → arr[base]; base += stride    (Multiply → add)
arr[i * n + j]      → arr[base + j]; base += n     (Hoist invariant multiply)
i * i               → sq; sq += 2*i + 1            (Square → add, Nicomachus's theorem)
```

### 5c: Bit-Twiddling Strength Reduction

**The literature:** Henry Warren, *Hacker's Delight* (2002). Knuth, TAOCP Vol. 4A.

**File:** `optimize/fold.rs` for constant cases, `codegen/peephole.rs` for loop patterns

```
x * 2       → x << 1       (Shift left by 1)
x * 4       → x << 2       (Shift left by 2)
x * 2^k     → x << k       (General power-of-two multiply)
x / 2       → x >> 1       (Shift right by 1, for positive x)
x / 2^k     → x >> k       (General power-of-two divide)
x % 2       → x & 1        (Parity check, for positive x)
x % 2^k     → x & (2^k-1)  (Modular power-of-two, for positive x)
x * 3       → (x << 1) + x (Reduce multiply latency)
x * 5       → (x << 2) + x
x * 7       → (x << 3) - x
x * 9       → (x << 3) + x
x * 15      → (x << 4) - x
```

**Why it matters for LOGOS specifically:** The `nqueens` benchmark uses `1 << col`,
`board | (1 << col)`, and `board ^ mask` heavily. Folding these enables further
constant propagation through bitwise expressions. The `sieve` benchmark uses `i * i`
as the starting point for marking composites — strength-reducing `i * i` to
`prev_square + 2*i - 1` (difference of consecutive squares) saves a multiply per
outer iteration.

### 5d: Algebraic Reassociation

**The literature:** Knuth, TAOCP Vol. 2, §4.6.4 (1997).

Reorder operations to minimize critical path length in the CPU pipeline:

```
// Before: sequential dependency chain (3 adds, latency = 3)
a + b + c + d
→ ((a + b) + c) + d

// After: balanced tree (2 adds in parallel, latency = 2)
→ (a + b) + (c + d)
```

For `n` terms, the sequential chain has latency `n-1`. The balanced tree has latency
`ceil(log2(n))`. For 8 terms: 7 cycles → 3 cycles.

**Caveat:** Only for associative operations (integer `+`, `*`, `|`, `&`, `^`). NOT
for floating-point (non-associative due to rounding). NOT for subtraction.

The existing string concat flatten (`codegen/expr.rs` — `a + b + c` →
`format!("{}{}{}", a, b, c)`) is a form of reassociation. This generalizes to
integer arithmetic.

### 5e: Division Strength Reduction (Granlund & Montgomery, 1994)

**The literature:** Torbjörn Granlund and Peter L. Montgomery, "Division by Invariant
Integers using Multiplication" (1994).

Division by a compile-time constant `d` can be replaced with multiplication by a
"magic number" `m` followed by a shift:

```
x / 7  →  (x * 0x24924925) >> 33  (for 32-bit x)
x % 7  →  x - ((x * 0x24924925) >> 33) * 7
```

**The math:** For unsigned division by `d`, find `m` and `s` such that
`floor(x/d) = floor(x*m / 2^(N+s))` for all `x` in the range. The algorithm finds
the smallest such `m` using ceiling division on `2^(N+s)` by `d`.

**Why this matters:** LLVM already does this for Rust code, and gcc -O2 does it for
C. But the LOGOS fold pass could do it BEFORE codegen for constant denominators,
enabling further constant propagation. More importantly, if we ever target
less-optimizing backends (WASM, Cranelift debug mode), having this in our pass saves
20-40 cycles per division.

**Note:** Lower priority than 5a-5d. Implement only after the primary strength
reductions are stable.

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_closed_form.rs`

**RED tests:**

```
closed_form_sum_1_to_n
  Source: Let sum be 0. Repeat for i from 1 to n: Set sum to sum + i.
  Assert: generated code contains `n * (n + 1) / 2` (or equivalent)
  E2E:    n=100 -> 5050, n=0 -> 0, n=1 -> 1, n=1000000 -> 500000500000

closed_form_sum_squares
  Source: Let sum be 0. Repeat for i from 1 to n: Set sum to sum + i * i.
  Assert: generated code contains the sum-of-squares formula
  E2E:    n=10 -> 385, n=100 -> 338350

closed_form_count
  Source: Let count be 0. Repeat for i from 1 to n: Set count to count + 1.
  Assert: generated code is just `let count = n;` (or n - start + 1)
  E2E:    n=100 -> 100

closed_form_non_zero_start
  Source: Let sum be 0. Repeat for i from 5 to n: Set sum to sum + i.
  Assert: formula adjusted for start=5
  E2E:    n=10 -> 5+6+7+8+9+10 = 45

closed_form_non_zero_init
  Source: Let sum be 100. Repeat for i from 1 to n: Set sum to sum + i.
  Assert: 100 + n*(n+1)/2
  E2E:    n=10 -> 155

closed_form_product_not_applied
  Source: Let prod be 1. Repeat for i from 1 to n: Set prod to prod * i.
  Assert: NOT replaced (factorial grows too fast, only safe for small n)

strength_reduction_2d_index
  Source: (nested loop with item (i * n + j + 1) of matrix access)
  Assert: i * n hoisted, inner loop uses base + j

strength_reduction_iva_square
  Source: (loop computing i*i at each iteration)
  Assert: generated code uses accumulator with 2*i+1 increment, no multiply

bit_strength_power_of_two_mul
  Source: Let y be x * 8.
  Assert: generated code contains `x << 3` (not `x * 8`)

bit_strength_power_of_two_mod
  Source: Let y be x % 16.
  Assert: generated code contains `x & 15` (not `x % 16`)

bit_strength_mul_by_3
  Source: Let y be x * 3.
  Assert: generated code contains `(x << 1) + x` (not `x * 3`)

reassociation_balanced_tree
  Source: Let y be a + b + c + d.
  Assert: generated code uses balanced evaluation: `(a + b) + (c + d)`

reassociation_no_float
  Source: Let y be a + b + c + d. (where a,b,c,d are Float)
  Assert: NOT reassociated (floating-point is non-associative)
```

**Negative tests:**

```
closed_form_no_match_impure_body
  Source: Let sum be 0. Repeat for i from 1 to n: Set sum to sum + i. Show i.
  Assert: NOT replaced (body has IO side effect beyond accumulator update)

closed_form_no_match_conditional
  Source: Let sum be 0. Repeat for i from 1 to n:
              If i % 2 equals 0: Set sum to sum + i.
  Assert: NOT replaced (conditional accumulation)

closed_form_no_match_data_dependency
  Source: Let sum be 0. Repeat for i from 1 to n:
              Set sum to sum + item i of data.
  Assert: NOT replaced (f(i) depends on runtime data)
```

### Edge Cases to Watch

1. **Integer overflow:** `n*(n+1)/2` overflows i64 for n > ~4.3 billion. Guard with
   checked arithmetic or fall back to loop.
2. **Division ordering:** `n*(n+1)/2` — one of n or n+1 is even, so division is exact.
   But `n*(n+1)*(2*n+1)/6` requires careful ordering to avoid intermediate overflow.
3. **Non-1 start:** All formulas need adjustment.
   `sum(i, start..n) = sum(i, 1..n) - sum(i, 1..start-1)`.
4. **Empty range:** When `n < start`, return the initial accumulator value.

---

## CAMP 6 — Deforestation (Stream Fusion)

### Sprint 6

**The Idea:** When a collection is produced by one loop and immediately consumed by
another loop, fuse them into a single loop. Eliminate the intermediate collection.

**The Math:** Wadler's deforestation (1988). If a value is produced and consumed
exactly once (linearity), the intermediate representation can be eliminated.

**The key theorem (Wadler 1988, Theorem 1):** If `f` is a *treeless* function
(no intermediate data structure in any argument position) and `g` is treeless,
then `f . g` can be deforested into a single treeless function.

**Shortcut fusion (Gill, Launchbury & Peyton Jones, 1993):** A practical variant.
Producer functions that build data using `build(g)` and consumer functions that
consume using `foldr(k, z)` fuse via the rule:
```
foldr(k, z, build(g)) = g(k, z)
```
This is how GHC's `map/filter/fold` pipelines fuse.

**For LOGOS:** The pattern `Push x to temp` (producer, equivalent to `build`) and
`Repeat for x in temp: acc += f(x)` (consumer, equivalent to `foldr`) can fuse
via shortcut fusion without full treelessness analysis. Detect the `build`/`foldr`
pattern directly at the AST level.

**Linearity verification (Pitfall 12):** LOGOS has move semantics with clone, NOT
linear types. Linearity for deforestation must verify:
1. No `.clone()` of the intermediate collection
2. No borrow/reference
3. Single consumer
4. Ownership analysis from `analysis/ownership.rs` helps but doesn't provide
   linearity proofs — verify these conditions explicitly.

**Modified file:** `codegen/peephole.rs` — new function `try_emit_fused_loops_pattern`

**Detection algorithm:**

1. Find a "producer": `Let temp = new Seq. While/Repeat: Push expr to temp.`
2. Find the "consumer": the next statement (or nearby) that iterates over `temp`:
   `Repeat for x in temp: ...`
3. Verify linearity: `temp` is NEVER used after the consumer loop. Check:
   - Not read after consumer loop
   - Not passed to any function
   - Not returned
   - Not cloned
   - No other reference to `temp` anywhere
4. Verify safety: producer body effects and consumer body effects are compatible
   (no write-write conflict on shared variables).
5. Fuse: inline the consumer body into the producer body, replacing `x` with `expr`.
   Eliminate `temp` entirely.

**Multi-stage fusion:**

```
Producer: Push f(i) to temp1.
Transform: Push g(x) to temp2 for x in temp1.
Consumer: acc += h(x) for x in temp2.

-> Fused: acc += h(g(f(i)))   (single loop, zero allocations)
```

Implement by repeated pairwise fusion.

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_deforestation.rs`

**RED tests:**

```
deforest_map_reduce
  Source: Let doubled be a new Seq of Int.
          Repeat for x in items: Push x * 2 to doubled.
          Let sum be 0.
          Repeat for x in doubled: Set sum to sum + x.
  Assert: generated code has ONE loop, not two
  Assert: NO `Vec::new()` or `push` for `doubled` in generated code
  E2E:    items=[1,2,3] -> sum=12

deforest_filter_collect
  Source: Let temp be a new Seq of Int.
          Repeat for x in items:
              If x is greater than 0: Push x to temp.
          Let result be a new Seq of Int.
          Repeat for x in temp: Push x to result.
  Assert: fused into single loop (filter-copy elimination)
  E2E:    items=[-1,2,-3,4] -> result=[2,4]

deforest_map_filter_reduce
  Source: Let mapped be a new Seq of Int.
          Repeat for x in items: Push x * 2 to mapped.
          Let filtered be a new Seq of Int.
          Repeat for x in mapped:
              If x is greater than 5: Push x to filtered.
          Let sum be 0.
          Repeat for x in filtered: Set sum to sum + x.
  Assert: ALL THREE loops fused into one. Zero intermediate collections.
  E2E:    items=[1,2,3,4,5] -> sum=24

deforest_preserves_order
  Source: (same as map_reduce but with ordering-sensitive operations)
  Assert: output order is identical to unfused version

deforest_empty_input
  Source: (map_reduce with empty items)
  Assert: sum=0, no crash
```

**Negative tests:**

```
deforest_no_fuse_when_used_later
  Source: Let temp be a new Seq. Loop: Push x to temp.
          Loop: acc += x for x in temp.
          Show length of temp.
  Assert: NOT fused (temp is used after consumer -> not linear)

deforest_no_fuse_when_passed_to_function
  Source: Let temp be a new Seq. Loop: Push to temp.
          processData(temp).
          Loop: acc += x for x in temp.
  Assert: NOT fused (temp is passed to external function)

deforest_no_fuse_conditional_producer
  Source: Let temp be a new Seq.
          If condition: Loop: Push x to temp.
          Otherwise: Loop: Push y to temp.
          Loop: acc += x for x in temp.
  Assert: NOT fused (producer is conditional)

deforest_no_fuse_write_conflict
  Source: Let temp be a new Seq. Let mutable z be 0.
          Loop: Push z to temp. Set z to z + 1.
          Loop for x in temp: Set z to z + x.
  Assert: Fuse carefully -- z is both written in producer and consumer.
```

### Edge Cases to Watch

1. **Break in consumer:** If the consumer loop has `Break`, it doesn't consume all
   elements. Can still fuse, but must add the break condition to the producer loop.
2. **Multiple pushes in producer:** `If cond: Push a. Otherwise: Push b.` — the
   consumer processes whichever was pushed. Fusion inlines the entire conditional.
3. **Producer and consumer on different collections:** Only fuse when the consumer
   iterates over the EXACT variable the producer builds.
4. **Nested loops in producer/consumer:** Don't fuse if the producer or consumer
   contains nested loops that would create quadratic behavior when fused.
5. **Accumulator initial value:** The consumer's accumulator init must be preserved.

---

## CAMP 7 — Polyhedral Tiling

### Sprint 7

**The Idea:** For dense nested loops with affine array accesses (e.g., matrix multiply),
tile the iteration space for cache locality. This is the single highest-impact
optimization for numerical code.

**The Math:** The Polyhedral Model (Feautrier, 1991). Each loop iteration
`(i, j, k)` is a point in Z^3. The iteration domain is a convex polyhedron defined by
the loop bounds. Array accesses define affine dependence functions. Tiling partitions
the iteration space into blocks that fit in L1 cache.

**2D Array Detection (Pitfall 13):** LOGOS represents 2D arrays as 1D with manual
indexing: `item (i * n + j + 1) of c`. The polyhedral model must detect this `i*n+j`
pattern and reconstruct the 2D access. Detection strategy:
- Match `BinaryOp(Add, BinaryOp(Mul, i, n), j)` where `i` and `j` are loop counters
- Reconstruct as 2D access `[i][j]` with stride `n`
- Validate `n` matches collection allocation (if known from abstract interpretation)

**Modified file:** `codegen/peephole.rs` — new function `try_emit_tiled_loop_pattern`

**Detection algorithm:**

1. Identify a triple-nested for-range loop (the for-range peephole must fire first).
2. Verify affine access patterns: all array indices are affine functions of loop
   variables (e.g., `item (i*n + j)` or `item j of (item i of matrix)`).
3. Check that dependencies allow tiling (no loop-carried dependencies that cross tile
   boundaries, OR all dependencies are lexicographically positive).
4. Choose tile size: 32 for i64 (fits 32*32*8 = 8KB per tile in 32KB L1).
5. Emit tiled loop nest with remainder handling for non-divisible bounds.

**Tiling transformation for matrix multiply:**

```
// Original:
for i in 0..n {
    for j in 0..n {
        for k in 0..n {
            result[i][j] += a[i][k] * b[k][j];
        }
    }
}

// Tiled (T=32):
for ii in (0..n).step_by(T) {
    for jj in (0..n).step_by(T) {
        for kk in (0..n).step_by(T) {
            let i_end = min(ii + T, n);
            let j_end = min(jj + T, n);
            let k_end = min(kk + T, n);
            for i in ii..i_end {
                for j in jj..j_end {
                    for k in kk..k_end {
                        result[i][j] += a[i][k] * b[k][j];
                    }
                }
            }
        }
    }
}
```

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_polyhedral.rs`

**RED tests:**

```
tile_matrix_mult_detected
  Source: (matrix multiply in LOGOS)
  Assert: generated code contains `step_by` or tiling pattern

tile_matrix_mult_correct
  Source: (matrix multiply in LOGOS, n=4 identity x [[1,2],[3,4],..])
  E2E:   verify output matches naive computation

tile_no_match_single_loop
  Source: (single for-range loop)
  Assert: NOT tiled (need at least 2-deep nesting for benefit)

tile_no_match_non_affine
  Source: (nested loop with non-affine access like item (i*i) of arr)
  Assert: NOT tiled (access not affine)

tile_remainder_handling
  Source: (matrix multiply with n=5, tile_size=4)
  E2E:   verify corner elements are correct (5 % 4 = 1 remainder)

tile_small_input_no_tile
  Source: (matrix multiply with n=2, tile_size=32)
  Assert: NOT tiled (n < tile_size -> tiling overhead worse than benefit)

tile_1d_flat_array_detected
  Source: (matrix multiply using item (i * n + j + 1) of flat_matrix)
  Assert: 2D access pattern detected from 1D array + i*n+j index
```

### Edge Cases to Watch

1. **Non-square bounds:** When i, j, k have different ranges, tile sizes may differ.
2. **Remainder handling:** When n % tile_size != 0, the last tile is partial.
3. **Dependency direction:** If `result[i][j]` depends on `result[i-1][j]` (not the
   matrix multiply case), tiling requires skewing first.
4. **LOGOS 1-based indexing:** All the math assumes 0-based. Convert carefully.
5. **Nested Seq access:** `item j of (item i of matrix)` is a double-index.

---

## CAMP 8 — Automatic Parallelization

### Sprint 8

**The Idea:** Detect loops with no loop-carried dependencies and emit parallel iteration.
Detect reduction patterns and emit parallel reductions.

**The Math:** Bernstein's conditions (1966). Two iterations S_i, S_j can execute in
parallel iff: `Write(S_i) intersection Read(S_j) = empty AND Read(S_i) intersection
Write(S_j) = empty AND Write(S_i) intersection Write(S_j) = empty`. The effect
system provides Read/Write sets. The no-aliasing guarantee means `items[i]` and
`items[j]` for `i != j` don't alias.

**Conflict with Simultaneously: blocks (Pitfall 14):** LOGOS already has explicit
parallel blocks (`Simultaneously:` emits `rayon::join`). Auto-parallelization
(`rayon::par_iter`) must check whether the loop is ALREADY inside a parallel context
to avoid thread pool contention. Detection must walk up the AST to verify no
enclosing `Parallel` or `Concurrent` statement.

**CRDT commutative writes:** Loops containing CRDT operations (tagged as
`commutative_writes` in the effect system) can be parallelized even though they
have write effects — commutativity guarantees correctness.

**Modified files:**
- `codegen/stmt.rs` — emit `rayon::par_iter` for parallelizable loops
- `codegen/program.rs` — add rayon dependency detection
- `codegen/peephole.rs` — reduction pattern detection

**Classification:**

| Pattern | Parallelization | Output |
|---|---|---|
| `for i in 0..n: result[i] = f(data[i])` | Embarrassingly parallel | `par_iter().for_each()` |
| `for x in items: acc += f(x)` | Reduction (commutative +) | `par_iter().map().sum()` |
| `for x in items: acc = max(acc, f(x))` | Reduction (commutative max) | `par_iter().map().max()` |
| `for i in 0..n: result[i] = f(result[i-1])` | Loop-carried dependency | NOT parallelizable |
| `for x in items: Show x` | IO ordering | NOT parallelizable |
| `for x in items: Increase crdt by f(x)` | Commutative CRDT write | `par_iter()` (safe) |

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_autoparallel.rs`

```
par_independent_map
  Source: Repeat for i from 1 to length of items:
              Set item i of result to (item i of items) * 2.
  Assert: generated code contains `par_iter` or `par_chunks`

par_reduction_sum
  Source: Let sum be 0.
          Repeat for x in items: Set sum to sum + x.
  Assert: generated code contains parallel reduction pattern

par_no_parallel_dependency
  Source: Repeat for i from 2 to length of items:
              Set item i of items to (item i of items) + (item (i - 1) of items).
  Assert: NOT parallelized (items[i] depends on items[i-1])

par_no_parallel_io
  Source: Repeat for x in items: Show x.
  Assert: NOT parallelized (IO must preserve order)

par_threshold
  Source: (same as par_independent_map but with 3-element array)
  Assert: NOT parallelized (below threshold, overhead > benefit)

par_no_nested_parallel
  Source: Simultaneously:
              Repeat for x in items: ...
  Assert: NOT parallelized (already inside explicit parallel context)

par_crdt_commutative
  Source: Repeat for x in items: Increase counter by x.
  Assert: parallelized (CRDT writes are commutative)

par_correctness_sum
  E2E: items = [1..100000], verify sum matches sequential

par_correctness_map
  E2E: items = [1..100000], verify all elements doubled correctly
```

### Edge Cases to Watch

1. **Floating-point reduction:** `sum += f` is NOT associative for floats.
   Only parallelize integer reductions, or accept float non-determinism with a flag.
2. **Threshold tuning:** Below ~10000 elements, sequential is faster. Use abstract
   interpretation range to estimate iteration count.
3. **rayon dependency:** Detect when parallel code is emitted and add `rayon` to
   the generated Cargo.toml.
4. **Nested parallelism:** Don't nest `par_iter` inside `par_iter`.

---

## CAMP 9 — Partial Evaluation

### Sprint 9

**The Idea:** When a function is called with some arguments known at compile time,
specialize the function for those arguments. Inline constants, eliminate dead branches,
unroll loops with known bounds.

**The Math:** Binding-time analysis (BTA). Classify each variable as *static* (S) or
*dynamic* (D). Propagate: `S op S -> S`, `S op D -> D`, `D op anything -> D`. Generate
a residual program that only contains dynamic computations.

**The Futamura Projections (1971):** The theoretical foundation for why partial
evaluation is so powerful:

1. **First projection:** Specializing an interpreter `int` with program `p` produces
   a compiled version of `p`: `pe(int, p) = target`. This is EXACTLY what CTFE
   (Camp 0j) does — it runs the LOGOS interpreter on known arguments to produce a
   literal.

2. **Second projection:** Specializing the partial evaluator `pe` with the interpreter
   `int` produces a compiler: `pe(pe, int) = compiler`. This is the theoretical
   foundation for this camp — specializing the optimizer itself for specific program
   patterns.

3. **Third projection:** Specializing `pe` with `pe` produces a compiler generator:
   `pe(pe, pe) = compiler_generator`. This is the summit — a system that generates
   optimizers.

Camp 0j (CTFE) implements the first projection in its simplest form. This camp
generalizes to full binding-time analysis and mixed static/dynamic specialization.

**New file:** `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

**Modified files:**
- `optimize/mod.rs`
- `compile.rs` — run after constant propagation, before codegen

**Algorithm:**

1. For each call site `f(arg1, arg2, ...)`:
   - Classify args: literals and propagated constants -> S. Everything else -> D.
   - If ALL args are D: skip (no specialization possible).
   - If ANY arg is S: create a specialized version `f_specialized`.

2. Specialization:
   - Clone the function body.
   - Substitute S args with their values.
   - Run constant folding + DCE on the specialized body.
   - If the specialized body is significantly simpler (fewer statements), use it.

3. Depth limiting:
   - Max specialization depth: 16 (prevent infinite unfolding of recursion).
   - Max specialized variants per function: 8 (prevent code bloat).

4. Effect gating: NEVER partially evaluate a function with IO or Write effects.
   The effect system (Camp 1) gates this.

### TDD Test Specification

**Test file:** `crates/logicaffeine_tests/tests/phase_partial_eval.rs`

```
pe_constant_arg_eliminated
  Source: ## To add5 (x: Int) -> Int: Return x + 5.
          Let y be add5(10).
  Assert: generated code contains `let y = 15;` (fully evaluated)

pe_partial_specialization
  Source: ## To multiply (a: Int, b: Int) -> Int: Return a * b.
          Let y be multiply(3, x).
  Assert: generated code contains `3 * x` or `x * 3` (a=3 substituted)

pe_branch_elimination
  Source: ## To select (flag: Bool, a: Int, b: Int) -> Int:
              If flag: Return a. Otherwise: Return b.
          Let y be select(true, x, z).
  Assert: generated code contains just `let y = x;`

pe_recursive_depth_limit
  Source: ## To fib (n: Int) -> Int:
              If n is at most 1: Return n.
              Return fib(n - 1) + fib(n - 2).
          Let y be fib(10).
  Assert: fully evaluated to 55 (depth 10 < limit 16)

pe_no_specialize_all_dynamic
  Source: ## To add (a: Int, b: Int) -> Int: Return a + b.
          Let y be add(x, z).
  Assert: NOT specialized (both args dynamic)

pe_no_specialize_impure
  Source: ## To sideEffect (n: Int): Show n.
          sideEffect(5).
  Assert: NOT evaluated at compile time (IO effect)

pe_code_bloat_limit
  Source: (function called with 20 different constant args)
  Assert: at most 8 specialized variants generated
```

### Edge Cases to Watch

1. **Recursive specialization:** `fib(10)` unfolds exponentially. Use memoization
   during PE to avoid blowup.
2. **Side effects:** NEVER partially evaluate IO or Write functions. Effect system gates this.
3. **Code size:** Track total code size increase, stop if it exceeds 2x original.
4. **Interaction with TCO:** Specialized version must preserve tail-call elimination.

---

## THE SUMMIT — Supercompilation

### Sprint 10 (Research Phase)

**The Idea:** The unification of all previous optimizations into a single framework.
Instead of running 10 separate passes, supercompilation drives the program forward
symbolically, applies all transformations as it encounters opportunities, and produces
an optimally specialized program.

**The vision (FRIEND_PLANS.md §3):** "Your compiler IS an interpreter that also emits
code." **§20:** "Don't distinguish between compile time and run time — the compiler is
an evaluator that stops when it runs out of information."

The supercompilation lineage runs through the entire mountain:
- **Camp 0j (CTFE):** The simplest form — evaluate pure functions with known arguments
  at compile time. First Futamura projection.
- **Camp 9 (Partial Evaluation):** Generalize to mixed static/dynamic specialization.
  Binding-time analysis classifies what can be evaluated statically.
- **Summit:** Full driving, folding, and generalization. The compiler evaluates
  everything it can, emits residual code for what it can't, and ties back loops by
  detecting repeated configurations.

**The Math:** Turchin's supercompilation (1986).

Three operations:
1. **Driving:** Evaluate one symbolic step. Unfold function calls, split on branches.
2. **Folding:** If the current state matches a previous state (up to renaming), create
   a loop. This is the "tying the knot" step.
3. **Generalization:** If no fold applies and the state space is growing, generalize
   (widen variables) to ensure termination.

**Why this is the summit:** Supercompilation subsumes:
- Constant folding (driving evaluates constants)
- Constant propagation (driving substitutes known values)
- Dead code elimination (unreachable branches are never driven)
- Partial evaluation (specialization via driving with known args)
- Deforestation (intermediate structures consumed during driving are never materialized)
- LICM (loop-invariant expressions are factored out by generalization)

**This sprint is research-only.** Build a prototype supercompiler on a SUBSET of the
AST (pure integer expressions + simple loops). Measure the optimization quality vs.
the existing pass pipeline. If it's competitive, design a production version.

### TDD Test Specification

```
super_subsumes_fold
  Source: Let x be 2 + 3.
  Assert: supercompiled result is `let x = 5;`

super_subsumes_propagation
  Source: Let a be 5. Let b be a + 1.
  Assert: supercompiled result is `let b = 6;`

super_subsumes_dce
  Source: If false: Show "dead".
  Assert: dead branch eliminated

super_deforestation
  Source: (produce-consume pipeline from Camp 6)
  Assert: fused into single loop without intermediate collection

super_specialization
  Source: (factorial(10) from Camp 9)
  Assert: fully evaluated to 3628800

super_terminates_on_infinite
  Source: ## To f(n: Int) -> Int: Return f(n+1).
  Assert: supercompiler terminates (via generalization/depth limit)
  Assert: emitted code is a loop or preserved recursive call
```

---

## Sprint Calendar

| Sprint | Camp | Duration | Depends On | Key Deliverable |
|---|---|---|---|---|
| 0 | Camp 0: Quick Wins (0a-0l) | 1 week | Ground | FxHashMap, bounds elision, folds, boolean algebra, CTFE, trait elimination, index lowering |
| 1 | Camp 1: Effect System | 2 weeks | Camp 0 | `EffectEnv` for all 54 Stmt + 28 Expr variants |
| 2 | Camp 2: Ref Iteration + Output Params | 1 week | Camp 1 | `&items`, `f(&mut x)` |
| 3 | Camp 3: Abstract Interpretation | 2 weeks | Camp 1 | `RangeEnv`, Bourdoncle widening |
| 4 | Camp 4: LICM + DSE + Unswitching + Peeling | 1.5 weeks | Camp 1, 3 | Loop transforms |
| 5 | Camp 5: Closed-Form + Strength Reduction + IVA + Reassociation | 1.5 weeks | Camp 3 | Arithmetic optimization suite |
| 6 | Camp 6: Deforestation | 2 weeks | Camp 1 | Shortcut fusion |
| 7 | Camp 7: Polyhedral Tiling | 2 weeks | Camp 3 | Cache-tiled matrix multiply |
| 8 | Camp 8: Auto-Parallelization | 2 weeks | Camp 1 | `rayon::par_iter` |
| 9 | Camp 9: Partial Evaluation | 2 weeks | Camp 1, 3 | Function specialization, Futamura I |
| 10 | Summit: Supercompilation | Research | All | Driving, Folding, Generalization |

**Total: ~17.5 weeks to the summit (expanded scope justifies the extra 1.5 weeks).**

---

## The Benchmark Wall (Real Data from `benchmarks/results/latest.json`)

Current geometric mean: **2.17x** slower than C across 32 benchmarks.

| Benchmark | Ratio vs C | Root Cause | Fix Camp |
|-----------|-----------|------------|----------|
| histogram | 10.62x | Trait-dispatch indexing (`LogosIndex::logos_get`) | 0k |
| fib | 9.94x | Function call overhead at recursive leaves | 0j (CTFE) |
| collect | 5.34x | HashMap with SipHash | 0a |
| two_sum | 5.33x | HashMap with SipHash | 0a |
| primes | 4.45x | Nested loop + early exit | 0b, Camp 3 |
| bubble_sort | 4.15x | Bounds-checked swap | 0b |
| binary_trees | 3.68x | Recursive allocation | Camp 2 |
| nbody | 3.33x | Struct array + sqrt | Camp 4 (LICM) |
| nqueens | 3.08x | Bitwise + recursion | 0d |
| counting_sort | 2.84x | Array histogram indexing | 0k |
| fannkuch | 2.74x | Array permutation + swap | 0b |
| knapsack | 2.60x | 2D DP table indexing | 0b, Camp 3 |
| prefix_sum | 2.52x | Sequential indexed RMW | 0b |
| string_search | 2.35x | Character indexing | 0b |
| coins | 2.30x | 1D DP indexing | 0b |
| mergesort | 2.28x | Allocation + indexed merge | Camp 2 |
| pi_leibniz | 1.90x | Loop accumulation | Camp 4 (LICM) |
| array_reverse | 1.83x | Two-pointer swap | 0b |
| graph_bfs | 1.77x | Queue random access | 0b |
| strings | 1.74x | Concatenation | --- |
| matrix_mult | 1.63x | Triple-nested indexed access | Camp 7 |
| quicksort | 1.58x | In-place partition | Camp 2 |
| array_fill | 1.48x | Push + sum | --- |
| mandelbrot | 1.46x | FP branch | Camp 4 |
| gcd | 1.27x | Pure recursion | --- |
| ackermann | 1.20x | Deep recursion | --- |
| spectral_norm | 1.11x | FP dot product | Camp 4 (CSE) |
| sieve | 1.09x | Boolean array | --- |
| fib_iterative | 0.93x | **FASTER** | --- |
| loop_sum | 0.92x | **FASTER** | --- |
| heap_sort | 0.70x | **FASTER** | --- |
| collatz | 0.62x | **FASTER** | --- |

**Key insight:** The top 6 slowest benchmarks (histogram through primes) account for
the vast majority of the geometric mean gap. Fixing just these 6 would bring the
overall ratio from 2.17x to approximately 1.3-1.4x.

### Expected Improvement Per Camp

| Camp | Benchmarks Improved | Expected Improvement |
|------|---------------------|----------------------|
| 0a (FxHashMap) | collect, two_sum | 20-40% on those 2 |
| 0b (Bounds elision) | 12+ sorting/DP/BFS benchmarks | 5-15% geometric mean |
| 0g-0h (Boolean/comparison folding) | all benchmarks with trivial guards | Enables further DCE |
| 0i (Local CSE) | spectral_norm, matrix_mult | 5-15% on subexpression-heavy |
| 0j (CTFE) | fib | 9.94x → ~1.0x (fully evaluated) |
| 0k (Trait elimination) | histogram, counting_sort | 5-10x on histogram |
| 0l (Index lowering) | sieve, prefix_sum, array ops | 5-10% on tight loops |
| 2 (Output params) | mergesort, binary_trees, quicksort | 20-40% on those 3 |
| 3 (Abstract interp) | all indexed access benchmarks | 10-20% geometric mean |
| 4 (LICM + Unswitching) | nbody, pi_leibniz, mandelbrot | 10-30% on loop-heavy |
| 5 (Strength reduction) | matrix_mult, nqueens, sieve | 10-20% on arithmetic-heavy |
| 7 (Polyhedral) | matrix_mult | 30-50% on matrix_mult |
| 8 (Auto-parallel) | mandelbrot, spectral_norm | 2-4x on parallel-amenable |

Every optimization PR must include before/after benchmark numbers for the specific
benchmarks it targets. No "trust me it's faster."

---

## Testing Philosophy

Every optimization follows the same TDD discipline:

### Layer 1: Unit Tests (Optimization Fires)
```rust
let rust = compile_to_rust(source).unwrap();
assert!(rust.contains("expected_pattern"), "Should emit X.\nGot:\n{}", rust);
```
Verifies the optimization DETECTED the pattern and EMITTED the right code.

### Layer 2: E2E Tests (Optimization is Correct)
```rust
common::assert_exact_output(source, "expected_output");
```
Verifies the optimized code produces IDENTICAL output to the unoptimized version.

### Layer 3: Negative Tests (Optimization Doesn't Misfire)
```rust
assert!(!rust.contains("pattern"), "Should NOT optimize this case");
```
Verifies the optimization does NOT fire when preconditions aren't met.

### Layer 4: Regression Suite
```bash
cargo test -- --skip e2e    # All unit/integration tests
cargo test                  # Full suite including e2e
```
Zero regressions after each sprint. Non-negotiable.

### Layer 5: Benchmark Verification
```bash
./benchmarks/run.sh         # Full benchmark suite
```
Performance improvement measured. No benchmark regression allowed.

---

## Pipeline Order (Sacred)

The pipeline order is fixed. No pass may run before its dependencies are available.

```
Parse
  -> Type Check (moved up from post-optimization)
  -> Effect Analysis (new — Camp 1)
  -> Optimize (fold -> propagate -> dce, now type+effect-aware)
  -> Abstract Interpretation (Camp 3)
  -> LICM (Camp 4, on While form)
  -> Escape Analysis
  -> Codegen (peephole chain runs here: for-range, tiling, closed-form, etc.)
```

This is a non-trivial reorder from the current pipeline:
```
Parse -> Optimize -> Escape -> Type Check -> Codegen
```

Type checking moves before optimization. This means the type checker runs on the
pre-optimization AST (more nodes, dead code not yet eliminated). Verify the type
checker handles `If false: <type-inconsistent-code>` gracefully — unreachable
branches should not cause type errors.

---

## Invariants (Carry These Up the Mountain)

1. **Correctness over performance.** An optimization that changes output is a bug, not
   an optimization. Test output identity exhaustively.

2. **Conservative over aggressive.** When in doubt, don't optimize. A missed optimization
   costs performance. A wrong optimization costs correctness.

3. **Composable passes.** Each optimization must work correctly regardless of whether
   other optimizations ran before it. No implicit ordering dependencies beyond the
   explicit dependency graph.

4. **No peephole duplication.** If two peepholes overlap, unify them. Each pattern
   should be detected in exactly one place.

5. **Opt-out annotations.** Every optimization respects `## No Optimize` and
   `## No Peephole`. Users can always disable.

6. **Effect system is the ground truth.** If the effect system says something is Pure,
   it IS pure. If it says IO, it IS IO. Every optimization above Camp 1 trusts the
   effect system unconditionally.

7. **All tests pass at every commit.** No "temporary" regressions. No "will fix later".
   The mountain has no shortcuts.

8. **Every Stmt variant is classified.** If a new Stmt variant is added to the AST,
   the effect system must be updated in the same PR. Enforce with match exhaustiveness
   (no `_ =>` wildcard in effect analysis).

9. **The pipeline order is sacred.** Parse -> Type Check -> Effect Analysis -> Optimize
   -> Escape -> Codegen. No pass may run before its dependencies are available.

10. **Quick wins first.** Any optimization that can be implemented without the effect
    system (pattern-matching, codegen changes) should be implemented BEFORE the effect
    system exists. Don't let a 2-week dependency block a 2-day win.

11. **SecurityCheck is non-negotiable.** No optimization pass, no matter how confident,
    may eliminate or reorder a `Check` statement. This is a compliance requirement.

12. **Benchmark before and after.** Every optimization PR must include before/after
    benchmark numbers for the specific benchmarks it targets.

13. **No trait dispatch for known types.** When the static type of a collection is
    known (e.g., `Vec<i64>`), codegen MUST emit direct indexing, not
    `LogosIndex::logos_get()`. Trait dispatch is for generic contexts only.

14. **Cite the literature.** Each optimization pass in this spec names its originating
    paper or textbook. Implementors must read the cited work before implementing.
    This prevents reinventing wheels badly.

15. **The interpreter is the reference semantics.** Any optimization that changes
    observable behavior must be detected by comparing interpreter output against
    compiled output on the benchmark suite. The interpreter is always correct; the
    optimizer must match it.
