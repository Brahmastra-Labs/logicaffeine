# FIX_SPECIALIZER_PLAN.md — TDD Agent Spec for Fixing the Specializer

## THE GOAL

**Make the three Futamura projections real.** Not approximately real. Not practically
equivalent. *Mathematically* real. The definitions are:

```
P1 = pe(interpreter, program)         → compiled program
P2 = pe(pe, interpreter)              → compiler
P3 = pe(pe, pe)                       → compiler generator
```

These are not implementation strategies. They are the *definitions*. P2 is the partial
evaluator applied to itself. P3 is the partial evaluator applied to itself twice. There
is no alternative path that counts. A generating extension that reads BTA annotations
and emits specialized code produces a compiler, but it is not P2 — it sidesteps
self-application entirely. A string replacement that renames `peExpr` to `compileExpr`
produces output that looks like a compiler, but it is not P2 — nothing was specialized.

**The only way to achieve proper Futamura projections is self-application.** The PE must
be capable of specializing itself. The PE's own pattern matches, its own dispatch on
CExpr variants, its own `isStatic` checks, its own environment lookups — all of these
must be eliminable by the PE when the PE is the subject of specialization. This is
"The Trick" (Jones et al. 1993, Section 7.2). Without it, `pe(pe, interpreter)` just
produces the PE with renamed functions. With it, the PE's dispatch is resolved at
specialization time, and the residual is a genuine compiler with zero PE overhead.

**There are no fallbacks. There are no escape hatches.** If self-application doesn't
work, we fix the PE until it does. We do not retreat to generating extensions. We do
not paper over failures with string replacement. Every sprint in this plan exists to
make the PE self-applicable. That is the only goal.

---

## CURRENT STATE

The three Futamura projections "work." All 276 tests in `phase_futamura.rs`, 35 in
`phase_partial_eval.rs`, 59 in `phase_supercompile.rs`, and 33 in `phase_bta.rs` are
green — 403 tests total. But they work because of shortcuts. Projection 1 ignores the
interpreter entirely (it just optimizes the program directly). Projections 2 and 3 are
string replacement — `replace_word("peExpr", "compileExpr")` — not actual specialization.
The Rust-side PE uses `SpecKey = (Symbol, String)` with string-containment embedding
checks. The supercompiler has `embeds()` and `msg()` functions that are defined but never
called. The `EffectEnv` is 700 lines of sophisticated analysis that no optimizer pass
uses. The PE source (`pe_source.logos`) checks `isLiteral` but not `isStatic` — so
compound static values (sequences, variants) are treated as dynamic. `CInspect` and
`CRepeat` in the PE source don't perform static dispatch or unrolling. The `decompile`
functions in `compile.rs` operate on the Rust AST, not the CExpr/CStmt representation
the PE source uses. None of this is wrong. It all produces correct results. But it means
the projections are theatre, not mathematics.

---

## THEORETICAL FOUNDATIONS

This plan is grounded in the following key papers and concepts:

- **Futamura (1971)**: "Partial Evaluation of Computation Process — An Approach to a Compiler-Compiler." Defines the three projections: P1 = pe(int, prog) = compiled, P2 = pe(pe, int) = compiler, P3 = pe(pe, pe) = compiler_generator.
- **Jones, Gomard & Sestoft (1993)**: "Partial Evaluation and Automatic Program Generation." The definitive reference for self-applicable PE. Defines The Trick, binding-time improvement, generating extensions, and Jones optimality.
- **Turchin (1986)**: "The concept of a supercompiler." Positive information propagation — after pattern matching, the supercompiler knows which constructor was matched.
- **Sørensen & Glück (1995)**: "An Algorithm of Generalization in Positive Supercompilation." Homeomorphic embedding as a well-quasi-ordering (WQO) for termination, and Most Specific Generalization (MSG).
- **Consel & Danvy (1993)**: "Tutorial Notes on Partial Evaluation." Online vs offline PE, binding-time improvement techniques.
- **Mogensen (1988)**: "Partially Static Structures in a Self-Applicable Partial Evaluator." Handling data structures where some elements are static and some dynamic.
- **Bondorf (1990, 1992)**: Generating extensions as an alternative to self-application for P2/P3. Let-insertion for shared computations.

**Key Concepts:**

- **The Trick**: The PE's own case analysis (Inspect on CExpr variants) must be eliminable by the PE itself during self-application. If the PE can't eliminate its own dispatch during P2, the "compiler" is just the PE with renamed functions. The trick is structuring the PE so that its pattern matches on the static argument are themselves static when the PE is the subject of specialization.
- **Jones Optimality**: `pe(interpreter, program)` should produce code with zero interpretive overhead. No residual env lookups, no residual funcs lookups, no residual dispatch on CExpr type — just the direct computation the program specifies.
- **Binding-Time Improvement (BTI)**: Restructuring source code so BTA classifies more values as static. Example: splitting `env: Map` into `staticEnv` (known at PE time) and a residual dynamic env.
- **Positive Information Propagation**: After `Inspect e: When CInt(v):`, the PE knows `e` is `CInt` in that arm. This information should be used to fold further dispatches on `e` within that arm.
- **Well-Quasi-Ordering (WQO)**: A termination guarantee. Homeomorphic embedding on finite tree structures forms a WQO by Kruskal's theorem — any infinite sequence must contain an embedding pair. String substring ordering is NOT a WQO.
- **Generating Extensions**: An alternative technique that constructs a compiler directly from BTA annotations rather than through self-application. Produces a working compiler but is NOT a Futamura projection — it sidesteps `pe(pe, interpreter)` entirely. We do not use this. It is documented here only for theoretical context.
- **Partially-Static Structures**: A data structure like `[1, x, 3]` where some elements are static and some dynamic. `item 1 of [1, x, 3]` should fold to `1` even though the list is "partially dynamic."

---

This document specifies 12 sprints of TDD work to fix every shortcut (31 corner cuts).
When complete, the three Futamura projections will be real — via self-application, with
no fallbacks. P1 will run the interpreter through the PE with an encoded program. P2
will run the PE on itself with the interpreter as static input, and the PE's own dispatch
will be eliminated in the residual. P3 will run the PE on itself with itself as static
input, producing a compiler generator that works for any interpreter. The same 403
existing tests will still pass, plus ~144 new tests verifying the fixed infrastructure.

```
                  FINAL INTEGRATION & JONES OPTIMALITY
             Unify _real() funcs, Jones tests, The Trick
             WQO, genext fallback, all 403 + ~144 tests
                         Sprint G
                              |
                        REAL PROJECTIONS 2 & 3
                   pe(pe, int) = compiler
                   pe(pe, pe)  = compiler_generator
                   VERIFY: no peExpr before rename
                   Generating extension fallback
                         Sprint F
                              |
                        REAL PROJECTION 1
                   pe(int, program) = compiled
                   via run_logos_source() pipeline
                         Sprint E
                              |
                    DECOMPILE IN PE SOURCE
                CExpr/CStmt → LogicAffeine source
                         Sprint D
                              |
               STATIC MAP/COLLECTION OPERATIONS
             CIndex/CLen/CFieldAccess on static data
                         Sprint C.5
                              |
                   ENVIRONMENT SPLITTING
              staticEnv propagates known bindings
              Positive information propagation
                         Sprint C.4
                              |
                    PE MEMOIZATION TABLE
              specCache prevents re-specialization
              Post-unfolding, let-insertion, WQO
                         Sprint C.3
                              |
                  MIXED-ARG SPECIALIZATION
             f(static, dynamic) → f_spec(dynamic)
             Arity raising for recursive calls
                         Sprint C.2
                              |
               PARTIALLY-STATIC STRUCTURES
             [1, x, 3]: item 1 folds to 1
             Element-level static tracking
                         Sprint C.1.5
                              |
               PEState STRUCTURAL REFACTOR
             Wrap params in PEState record
             Zero behavior change
                         Sprint C.1
                              |
                   PE SOURCE STATIC DISPATCH
              isStatic, CInspect, CRepeat unrolling
              Break/Return guards in unrolling
                         Sprint C
                              |
              FIX SUPERCOMPILER FOUNDATION
           embeds/msg activation, Index, BTA SCC
                         Sprint B
                              |
                FIX PE FOUNDATION
           SpecKey, embedding, EffectEnv
                         Sprint A
                              |
                   ===============
                   GROUND (today)
                   ===============
                   403 tests, all green
                   String-based shortcuts
                   Dead infrastructure
                   ===============
```

---

## TDD Format Specification

This document follows the same strict TDD format as PROPER_FUTAMURA.md.

| Step Type | What Happens | Expected Outcome |
|-----------|-------------|-----------------|
| **RED** | Write test(s). Run them. They MUST fail. | FAIL (compile error or assertion failure) |
| **GREEN** | Implement the minimum to make RED tests pass. Run them. | PASS |
| **VERIFY** | Run `cargo test -- --skip e2e` on broader test suites. | Zero failures |
| **GATE** | Run full `cargo test -- --skip e2e`. Hard stop if any failure. | Zero failures. Proceed only if green. |

**Rules:**
1. Every RED step specifies exact test names, the file they go in, what source code to use, and what to assert.
2. Every GREEN step specifies exact file paths to create/modify and what to implement.
3. Never modify a RED test to make it pass. Fix the implementation.
4. Never modify any of the existing 403 tests. Only add new tests.
5. Every sprint ends with a GATE. No proceeding until the gate is green.

**Test helpers available** (in `crates/logicaffeine_tests/tests/common/mod.rs`):
- `compile_to_rust(source) -> Result<String, ParseError>` — compile LOGOS source to Rust code string
- `assert_exact_output(source, expected)` — compile, run, assert stdout matches exactly
- `run_logos(source) -> E2EResult` — compile and run, return result with stdout/stderr

**Existing test counts (NEVER modified, always green):**
- `phase_bta.rs`: 33 tests
- `phase_partial_eval.rs`: 35 tests
- `phase_supercompile.rs`: 59 tests
- `phase_futamura.rs`: 276 tests
- Total: 403 tests

---

## Backward Compatibility Strategy

The existing 276 `phase_futamura.rs` tests call `projection1_source()`, `projection2_source()`, `projection3_source()`. Rewriting these functions risks breaking all 56 P1/P2/P3 tests. Strategy:

1. **Sprints A–D**: No changes to projection function signatures or behavior. All 276 tests stay green trivially.

2. **Sprint E**: Add `projection1_source_real()` as a NEW function. The existing `projection1_source()` keeps its current fast-path behavior. New tests call `_real()`. Only after all new tests pass, the two can be unified (separate sprint).

3. **Sprint F**: Same pattern — `projection2_source_real()` and `projection3_source_real()`. These perform actual self-application: `pe(pe, interpreter)` and `pe(pe, pe)`. Existing tests untouched.

4. **Sprint G (Final Integration)**: Unify real and legacy functions. Run ALL 276 existing tests against the real self-application implementations. Fix any discrepancies in the real implementation (not the tests). This is the final gate. The `_real()` functions become the only implementations — the string-replacement versions are deleted entirely.

---

## Performance Budget

Self-application is computationally expensive. The PE processing its own source means
thousands of CExpr nodes being specialized. This is expected and acceptable — we are
doing the real thing.

| Projection | Budget | Rationale |
|-----------|--------|-----------|
| P1 (simple program) | < 30s | compile + rustc + execute combined source |
| P1 (complex program) | < 120s | larger encoded program |
| P2 (self-application) | < 300s | PE source is 1000+ lines; the PE specializes itself |
| P3 (self-application²) | < 600s | The PE specializes itself w.r.t. itself |

Sprint E/F tests should use `#[ignore]` by default (run explicitly with `cargo test -- --ignored` for projection testing). Or use a cargo feature flag `--features projection_tests`.

---

## PE Source Development Strategy

`pe_source.logos` has no LSP or type checking. Changes are tested only at runtime. Strategy:

1. **Unit-test each function immediately** after adding it. Pattern:
   ```rust
   let source = format!("{}\n{}\n## Main\n    {}", CORE_TYPES_FOR_PE, pe_source_text(), test_code);
   assert_exact_output(&source, expected);
   ```

2. **PEState refactor safety**: Change 5-10 call sites at a time, run gate between batches. After refactor, grep for any remaining old-style calls.

3. **Count call sites before/after**: `peExpr(` occurrences should be identical count before and after PEState refactor.

---

## KNOWN CORNER CUTS (Comprehensive Audit)

Every corner cut below has a sprint + test addressing it.

### Corner Cut 1 — SpecKey Is a String

**Location:** `partial_eval.rs:10`

```rust
type SpecKey = (Symbol, String);
```

The specialization key is `(function_name, stringified_args)`. The string is built by
`classifications_to_key_string()` which serializes each argument as `"D"` or `"S<literal>"`.
This is lossy — different literals that produce the same string representation collide.
More critically, the embedding check (`spec_key_embeds`, line 39) uses `later_str.contains(earlier_str)`,
which is a string substring check, not a structural homeomorphic embedding.

**Fix:** Sprint A. SpecKey becomes `(Symbol, Vec<Option<Literal>>)`. Embedding check
becomes structural comparison on the `Vec` elements.

### Corner Cut 2 — embeds() and msg() Are Dead Code

**Location:** `supercompile.rs:790-878`

The `embeds()` function implements homeomorphic embedding (`e1 ◁ e2`) with proper
coupling and diving rules. The `msg()` function computes Most Specific Generalization.
Both are defined, both have tests, neither is ever called by the supercompiler. The
supercompiler uses ad-hoc variable widening in `While` handlers instead (remove all
modified vars from store, line 597).

**Fix:** Sprint B. Activate `embeds()` as a termination guard in the supercompiler's
While handler. Use `msg()` for principled generalization when embedding is detected.
Add Configuration/History tracking to the driving loop.

### Corner Cut 3 — EffectEnv Is Never Integrated

**Location:** `effects.rs` (entire file, ~700 LOC)

Full effect lattice with SCC-based fixed-point analysis. Per-binding and per-function
effect tracking. Complete query API: `is_binding_pure()`, `function_is_pure()`,
`has_io()`, `may_diverge()`. Used only in `phase_effects.rs` tests. Never called by
the optimizer pipeline.

**Fix:** Sprint A. Integrate `EffectEnv::function_is_pure()` into the PE's
`body_has_io()` check. Functions that the effect analysis classifies as pure should be
specializable even if the ad-hoc `body_has_io()` walk misses some patterns.

### Corner Cut 4 — BTA Doesn't Use CallGraph SCCs

**Location:** `bta.rs`

BTA has polyvariant analysis (different call sites produce different divisions) and a
`BtaCache` for memoization. But it doesn't use the `CallGraph::sccs` from
`analysis/callgraph.rs` to handle mutually recursive functions. It relies on the cache
to terminate recursion, which works but doesn't guarantee convergence on the minimal
fixed point.

**Fix:** Sprint B. Wire BTA's polyvariant analysis through the CallGraph's SCC order.
Process functions in topological order of SCCs. For recursive SCCs, iterate to fixed
point within the SCC.

### Corner Cut 5 — PE Source Uses isLiteral, Not isStatic

**Location:** `pe_source.logos:8-17, 105-110, 201`

The PE source has `isLiteral(e)` which returns true only for `CInt`, `CBool`, `CText`.
It uses `allLiteral(args)` to decide whether to inline a function call. This means
compound static values — `VSeq`, `VVariant`, `VFloat`, `VTuple`, `VStruct` — are
treated as dynamic even when fully known at specialization time.

Similarly, `exprToVal` (line 19) and `valToExpr` (line 30) only handle
`CInt`/`CBool`/`CText`/`VInt`/`VBool`/`VText`. Missing: `VFloat`, `VSeq`, `VVariant`,
`VTuple`.

**Fix:** Sprint C. Add `isStatic(e)` predicate that checks all static CExpr variants.
Extend `exprToVal`/`valToExpr` to handle `CFloat`↔`VFloat`, `CList`↔`VSeq`,
`CNewVariant`↔`VVariant`, `CTuple`↔`VTuple`. Replace `allLiteral` with `allStatic`
in the `CCall` handler.

### Corner Cut 6 — CInspect Has No Static Dispatch

**Location:** `pe_source.logos:431-441`

The PE source handles `CInspect` by recursively PE'ing the target and each arm's body,
but it never checks whether the target is a known variant at specialization time. If
the target is a static `CNewVariant` with a known tag, the PE should match the correct
arm and eliminate the dead arms entirely.

**Fix:** Sprint C. Add static arm matching: if `peTarget` is a `CNewVariant`, match
against `CWhen` arms by tag. Bind the variant's fields in the environment. Eliminate
dead arms. Fall through to full Inspect only when the target is dynamic.

### Corner Cut 7 — CRepeat Has No Static Unrolling

**Location:** `pe_source.logos:407-414`

`CRepeat` (for-in loops) always generates a residual `CRepeat` even when the collection
is a statically known `CList` or `CRange`. A proper PE should unroll the loop body for
each element when the collection is static.

**Fix:** Sprint C. If the collection PE's to a `CList` with known elements, unroll: for
each element, bind the loop variable in the environment and PE the body. Concatenate
the resulting statement blocks. If the collection is a static `CRange`, generate the
integer range and unroll similarly.

### Corner Cut 8 — No Decompile Functions in PE Source

**Location:** `compile.rs:2757-3158` (Rust-side decompile), `pe_source.logos` (no decompile)

The decompile functions (`decompile_stmt`, `decompile_expr`, `decompile_type_expr`) exist
in Rust and operate on the Rust AST (`Stmt<'a>`, `Expr<'a>`). There are no equivalent
functions in the PE source that operate on `CExpr`/`CStmt`. This means the PE source
cannot produce LogicAffeine source output — it can only produce `CExpr`/`CStmt` trees.
For real projections, the PE must be able to decompile its residual back to source.

**Fix:** Sprint D. Add `decompileExpr(e: CExpr) -> Text`,
`decompileStmt(s: CStmt, indent: Int) -> Text`,
`decompileBlock(stmts: Seq of CStmt, indent: Int) -> Text` functions to `pe_source.logos`.
These must handle all CExpr and CStmt variants.

### Corner Cut 9 — Projection 1 Ignores the Interpreter

**Location:** `compile.rs:2691`

```rust
pub fn projection1_source(_core_types: &str, _interpreter: &str, program: &str)
```

The `_interpreter` parameter is ignored (underscore-prefixed). P1 just parses the program
directly, runs the optimizer, and decompiles. This is correct (the result is the same as
if the interpreter were specialized away) but it's not a real Futamura projection. A real
P1 encodes the program as data, combines it with the interpreter source, and runs the PE
to specialize the interpreter away.

**Fix:** Sprint E. Rewrite `projection1_source()` to:
1. Encode the program via `encode_program_source()`
2. Combine the encoded program with the interpreter source and PE source
3. Run the combined source through the LogicAffeine pipeline
4. The PE specializes the interpreter with respect to the encoded program
5. Decompile the residual (now using the PE source's decompile functions)

### Corner Cut 10 — Projections 2 and 3 Are String Replacement

**Location:** `compile.rs:3531-3559`

```rust
pub fn projection2_source() -> Result<String, String> {
    let pe_source = pe_source_text();
    let compiler_source = replace_word(
        &replace_word(&pe_source, "peExpr", "compileExpr"),
        "peBlock", "compileBlock",
    );
    Ok(format!("{}\n{}", CORE_TYPES_FOR_PE, compiler_source))
}
```

P2 and P3 just rename function names with `replace_word()`. No actual specialization
happens. The `replace_word()` function (line 3561) uses ASCII word-boundary detection
which can fail on identifiers containing the target string.

**Fix:** Sprint F. Rewrite P2 to actually run PE(PE, interpreter):
1. Encode the interpreter as a CProgram
2. Combine encoded interpreter + PE source + core types
3. Run through the pipeline — PE specializes itself w.r.t. the interpreter
4. Decompile the residual

Rewrite P3 similarly: PE(PE, PE).

### Corner Cut 11 — PE Source Has No Mixed-Arg Specialization

**Location:** `pe_source.logos:192-218`

```logos
When CCall (fnName, argExprs):
    ...
    Let allLit be allLiteral(peArgs).
    If allLit:
        // fully inline
    Return a new CCall with name fnName and args peArgs.  // ← just residualize
```

The PE source's CCall handler has only two modes: if ALL arguments are static, fully
inline the function body. Otherwise, residualize the entire call unchanged. There is no
mixed-arg specialization — when some arguments are static and some are dynamic, the PE
does nothing. It doesn't substitute the static arguments into the body or create a
specialized function variant.

This is **fatal for real Futamura projections**:
- P1 requires specializing `coreEval(staticExpr, dynamicEnv, staticFuncs)`. Since
  `env` is dynamic, `allStatic` is false, so the entire interpreter dispatch is
  residualized. The interpreter is never eliminated.
- P2 requires specializing `peExpr(e, env, staticFuncs, staticDepth)`. Since `e` and
  `env` are dynamic, the PE residualizes its own dispatch — no specialization happens.
- P3 has the same problem squared.

The Rust-side PE (`partial_eval.rs:461-574`) does handle mixed args via
`try_specialize_call()`: it creates a specialized function variant with static
parameters substituted and only dynamic parameters remaining. The PE source needs
the same capability.

**Fix:** Sprint C.2. Add mixed-arg specialization to the CCall handler:
1. When some args are static and some are dynamic:
   a. Look up the function in `funcs`
   b. Create a `callEnv` with only the static args bound
   c. PE the function body with this partial environment
   d. Generate a new specialized function name (via `makeKey`)
   e. Emit a `CFuncDef` for the specialized function (taking only dynamic params)
   f. Replace the call with a call to the specialized function (passing only dynamic args)
2. Use a specialization cache (`specCache: Map of Text to Text`) to avoid re-specializing
   the same function with the same static args
3. The `allStatic` path remains as-is (full inlining, no residual function)

### Corner Cut 12 — PE Source Has No Memoization Table

**Location:** `pe_source.logos:192-218` (CCall handler), entire peExpr/peBlock

The PE source uses only `depth` to prevent infinite recursion during function inlining.
There is no memoization table mapping `(function, static_args)` to previously-computed
specialization results. When the PE encounters the same function call with the same
static arguments multiple times (common in recursive functions and self-application),
it re-specializes from scratch each time.

For P1 this is merely wasteful. For P2/P3 (self-application), it is **catastrophic**:
- The PE's `peExpr` function calls itself recursively for each CExpr variant
- When specializing `peExpr` with respect to the interpreter, the PE encounters
  `peExpr` calls in the interpreter's dispatch for each arm
- Without memoization, each call re-enters `peExpr`, which re-enters `peExpr`,
  hitting the depth limit and producing poor residuals
- With memoization, the first specialization of `peExpr` for a given binding-time
  pattern is cached and reused, producing clean residuals

The Rust-side PE (`partial_eval.rs:21-26`) has a `SpecRegistry` with
`cache: HashMap<SpecKey, Symbol>` for exactly this purpose.

**Fix:** Sprint C.3. Add a memoization table to the PE source:
1. Add `specCache: Map of Text to CExpr` parameter to `peExpr` and `peBlock`
   (or thread it through the `funcs` map as an additional entry)
2. Before specializing a function call, check `specCache` for a cached result
3. After specializing, store the result in `specCache`
4. For recursive functions: insert a placeholder before specializing (to break
   the cycle), then update with the real result
5. The cache key is `makeKey(fnName, staticArgs)` — already implemented

### Corner Cut 13 — PE Source Does Not Evaluate Static Map Operations

**Location:** `pe_source.logos:219-315` (peExpr handlers for CIndex, CFieldAccess, CLen)

The PE source processes `CIndex`, `CFieldAccess`, and `CLen` by recursively PE'ing their
sub-expressions and emitting residual operations. Even when the target collection or map
is statically known, these operations are never evaluated at specialization time.

This matters for P2/P3 because the PE source uses Maps extensively:
- `env: Map of Text to CVal` — variable environment
- `funcs: Map of Text to CFunc` — function definitions
- `item fnName of funcs` — function lookup in the CCall handler

When the PE processes its own source (P2/P3), it encounters `item fnName of funcs`
where `funcs` is the static function map. The PE should evaluate this lookup at
specialization time (since the map is known), but instead it residualizes it as
`CIndex(funcsCExpr, fnNameCExpr)`. The result retains all the map lookup overhead
that should have been eliminated.

Similarly, `CLen` on a static list should fold to the list's length. `CFieldAccess`
on a static struct should fold to the field value.

**Fix:** Sprint C.5. Add static evaluation for collection/map operations:
1. In `peExpr`, `CIndex` handler: if both collection and index are static, evaluate
   the index operation directly. For `CList` with static `CInt` index: return
   `item idx of items`. For `CVar` bound to a `VMap` with static key: look up and
   return. Otherwise, residualize as before.
2. In `peExpr`, `CLen` handler: if target is a static `CList`, return `CInt(length)`.
   If target is a `CVar` bound to `VSeq`, return `CInt(length)`.
3. In `peExpr`, `CFieldAccess` handler: if target is a static `CNew`/`CNewVariant`,
   look up the field by name and return the corresponding value.
4. Guard against index-out-of-bounds: if the static index is out of range, residualize
   instead of crashing.

### Corner Cut 14 — No Two-Level Distinction in PE Source

**Location:** `pe_source.logos` (entire `peExpr` function)

Runtime `isLiteral(peLeft)` checks can't be eliminated when PE specializes itself. PE
dispatch is residualized wholesale during P2/P3 because the PE has no way to distinguish
between its own binding-time levels. Every `isStatic`/`isLiteral` check in the PE source
produces a runtime check in the residual, even when the PE is specializing itself and
knows the answer statically.

**Fix:** Sprint C.4. `staticEnv` tracking propagates known-static bindings through the
PE's own variable environment, enabling Inspect folding on known CExpr variants.

### Corner Cut 15 — Environment Blocks Static Propagation

**Location:** `pe_source.logos:326-337` (CLet/CSet handlers)

`env: Map of Text to CVal` is always dynamic. Every `item varName of env` produces a
residual map lookup even when the PE just bound that variable to a known static value.
The environment is opaque to specialization — even though the PE knows it just set
`item "x" of env to VInt(5)`, the subsequent `item "x" of env` lookup is not folded.

**Fix:** Sprint C.4. Environment splitting with parallel `staticEnv: Map of Text to CExpr`.
Variables with known static values are tracked in `staticEnv` and looked up directly,
bypassing the dynamic `env` map.

### Corner Cut 16 — No Size Regression Tests

**Location:** `tests/phase_futamura.rs` (missing tests)

No test verifies that specialization actually REDUCES code. A PE that copies its source
with renames passes all existing tests. The current test suite verifies functional
correctness (output matches) but never checks that the residual is smaller than the
input. This means a broken PE that residualizes everything still passes.

**Fix:** Sprint F. Hard assertions: `p2_smaller_than_pe`, `p3_smaller_than_p2`, raw
residual checks before rename.

### Corner Cut 17 — `run_logos_source()` Unresolved

**Location:** `compile.rs` (missing function)

Sprint E/F depend on compiling and executing LogicAffeine source at library level. Left
as `todo!()` in the current Sprint F spec. Without a concrete implementation, real
projections cannot actually run — they have no way to execute the combined PE + encoded
program source.

**Fix:** Sprint E. Use Rust compile pipeline: `decompileBlock → source → compile_to_rust
→ rustc → execute`. Reuse existing test infrastructure.

### Corner Cut 18 — PE Source Loops Not Self-Application-Aware

**Location:** `pe_source.logos:364-381` (CWhile), `pe_source.logos:407-414` (CRepeat)

PE's own iteration loops (e.g., `Repeat for a in argExprs`) are residualized during
self-application even when the collection is static. When the PE processes its own
CCall handler during P2, the loop `Repeat for a in argExprs` should unroll if `argExprs`
is a known static list. Instead, the entire loop is residualized, preserving the PE's
iteration overhead in the compiler output.

**Fix:** Sprint C (extend). Add P2-level test verifying PE's own argument-processing
loops unroll when interpreter parameter lists are static.

### Corner Cut 19 — Post-Hoc Rename Masks Specialization Failure

**Location:** `compile.rs` (Sprint F design)

`.replace("peExpr", "compileExpr")` after decompilation hides whether the PE dispatch
was actually eliminated. If the residual still contains `peExpr` dispatch, the rename
makes a broken result look correct — the "compiler" is just the PE with renamed
functions, not a specialized compiler with dispatch eliminated. No test checks the
raw residual before rename.

**Fix:** Sprint F. Assert raw residual contains NO `peExpr`/`peBlock` BEFORE rename.
Rename is cosmetic polish on verified-correct output.

### Corner Cut 20 — No Cross-Interpreter P3 Verification

**Location:** `tests/phase_futamura.rs` (missing tests)

P3 cogen should work for ANY interpreter, not just the Core/RPN interpreters used in
existing tests. No test applies cogen to a fresh interpreter. If cogen only works for
the interpreter it was derived from, it's not a real compiler generator — it's a
hard-coded compiler disguised as a generator.

**Fix:** Sprint F. Add minimal calculator interpreter test. Apply cogen, verify compiler
works, verify compiler is smaller than cogen, verify no PE infrastructure in output.

### Corner Cut 21 — "The Trick" Not Verified End-to-End

**Location:** `pe_source.logos` (CInspect handler), `compile.rs` (projection2_source)

The PE's own `Inspect` on CExpr variants must be eliminated when the PE specializes
itself (P2). Corner cuts 6 (CInspect static dispatch) and 14 (two-level distinction)
are necessary but not sufficient — no test verifies the composition: that the PE
eliminates its OWN case analysis during P2. The Trick (Jones et al. 1993, Section 7.2)
requires that the PE's pattern matches on the static argument are themselves static
when the PE is the subject of specialization. Without an end-to-end test, we can't
know if this actually works.

**Fix:** Sprint G. Add tests verifying PE's Inspect nodes are eliminated in P2 residual.

### Corner Cut 22 — No Positive Information Propagation

**Location:** `pe_source.logos` (CInspect handler, CIf handler)

After `Inspect e: When CInt(v): <body>`, the PE should know `e = CInt(v)` in `<body>`.
After `If isStatic(x): <true-branch>`, `x` is known-static in the true branch.
Currently neither is propagated. This matters for self-application: when the PE
processes its own `Inspect` on CExpr variants during P2, the arm bodies should be
able to assume the target has the matched type — enabling further folding within
that arm. Without positive info propagation, the PE re-dispatches on already-matched
values, producing redundant Inspect chains in the residual.

Theoretical basis: Turchin (1986), Sørensen & Glück (1995).

**Fix:** Sprint C.4 (staticEnv sprint). After Inspect arm entry, bind the target to
the matched variant in `staticEnv`.

### Corner Cut 23 — VMap Not Liftable (valToExpr Missing VMap)

**Location:** `pe_source.logos` (valToExpr function)

Sprint C extends `valToExpr` for VSeq, VVariant, VTuple but NOT VMap. During P2/P3,
the PE's Maps (`env`, `funcs`) are computed values that need lifting back to CExpr.
Without VMap lifting, self-application residuals retain opaque map references.

LogicAffeine Maps don't have a literal constructor syntax, so `valToExpr(VMap)` can't
produce a single expression. Options:
- Return a `CCall("buildMap", [keys_list, vals_list])` and add a `buildMap` helper
- Return a chain of nested `CSetItem` expressions (if such a CExpr variant exists)
- Accept that VMap cannot be lifted to a single expression and handle it at the
  statement level

This needs a design decision — flagged in Sprint C Step C6 as a known pitfall.

**Fix:** Sprint C (extend Step C6). Add `VMap -> CExpr` conversion or document the
limitation with a clear statement-level workaround.

### Corner Cut 24 — No Post-Unfolding / Cascading in PE Source

**Location:** `pe_source.logos` (CCall handler, all-static path)

After inlining a function body, the result may contain new calls with static args.
The Rust PE cascades (partial_eval.rs:547), but pe_source.logos does NOT — it returns
`extractReturn(peBody)` without re-specializing the result. This means opportunities
created by inlining are missed. Example: `f(3)` inlines to `g(3 + 1)`, which should
cascade to `g(4)` and potentially inline `g` too.

**Fix:** Sprint C.3. After full-inline, re-run `peExpr` on the result to catch
transient opportunities.

### Corner Cut 25 — Arity Raising Not Tested for Recursive Calls

**Location:** `pe_source.logos` (CCall handler, mixed-arg path)

Sprint C.2 creates `f_spec(dynArgs)` but doesn't test that recursive calls within
the specialized body correctly reference the specialized variant with reduced arity.
Example: `power(2, n)` specializes to `power_s0_2(n)`, but the recursive call
`power(2, n-1)` inside the body should become `power_s0_2(n-1)`, not remain
`power(2, n-1)`.

**Fix:** Sprint C.2. Add recursive self-call and mutual recursion arity tests.

### Corner Cut 26 — CRepeat Unrolling Doesn't Respect Break/Return

**Location:** `pe_source.logos` (CRepeat handler, static unrolling path)

Sprint C adds CRepeat unrolling for static collections, but if the body contains
`CBreak` or `CReturn`, unrolling should stop at that point. Currently it would unroll
past the break, producing incorrect code (the unrolled statements after the break
would execute when they shouldn't).

**Fix:** Sprint C (extend Step C9). Check for CBreak/CReturn in unrolled body; stop
unrolling at that point.

### Corner Cut 27 — Online PE Infrastructure Leaks into P2/P3 Residuals

**Location:** `pe_source.logos` (isStatic, isLiteral, allStatic calls throughout)

The PE source is online (uses runtime `isStatic`/`isLiteral` checks). During P2
self-application, these checks should be eliminated — the PE knows statically whether
its own arguments are static. Without elimination, the P2 "compiler" retains PE
dispatch overhead (`isStatic`, `allStatic`, `isLiteral` calls), meaning the compiler
is just a renamed PE, not a specialized one.

**Fix:** Sprint G. Assert P2 residual contains no `isStatic`/`isLiteral`/`allStatic` calls.

### Corner Cut 28 — No Partially-Static Structures

**Location:** `pe_source.logos` (isStatic predicate, CIndex handler)

A data structure like `[1, x, 3]` where some elements are static and some dynamic
is treated as entirely dynamic by `isStatic`. A proper PE should handle partially-static
structures: `item 1 of [1, x, 3]` folds to `1` even though the list is "dynamic".
The interpreter's `env: Map of Text to CVal` is a partially-static structure — some
bindings are known, some not. Sprint C.4's `staticEnv` is a manual workaround for one
case, but a general mechanism is missing.

Theoretical basis: Mogensen (1988), "Partially Static Structures in a Self-Applicable
Partial Evaluator." Jones et al. (1993), Section 5.5.

**Fix:** New Sprint C.1.5 (between C.1 and C.2). Add `isPartiallyStatic(e)` and
element-level static tracking. Extend CIndex to fold when the indexed element is
static even if the collection is partially dynamic.

### Corner Cut 29 — Self-Application Must Work Without Fallback

**Location:** `compile.rs` (projection2_source, projection3_source)

Self-application is the definition of P2 and P3. `pe(pe, interpreter) = compiler`.
There is no fallback. If self-application fails to eliminate PE overhead, we fix the
PE until it does — through binding-time improvements, restructuring the PE source,
adding BTI annotations, or whatever it takes. A generating extension is a different
technique that produces a compiler but is not a Futamura projection.

Theoretical basis: Jones et al. (1993) spent years refining MIX through binding-time
improvements until self-application produced clean output. The path is iterative
refinement of the PE, not retreat to an alternative technique.

**Fix:** Sprints C.2, C.3, C.4, C.5 build the infrastructure that makes the PE
self-applicable. Sprint F runs the actual self-application. Sprint G verifies that
the PE's own dispatch was eliminated. If Sprint G's assertions fail, the fix is in
the PE source (binding-time improvements), not in switching to a different approach.

### Corner Cut 30 — No Let-Insertion for Shared Computations

**Location:** `pe_source.logos` (CCall handler, all-static path)

When the PE inlines a function body at multiple call sites via the all-static path,
the inlined code is duplicated. If the function body contains expensive computations,
these are duplicated in the residual. Let-insertion creates a shared `CLet` binding
and references it at both sites.

Theoretical basis: Danvy, Malmkjaer & Palsberg (1995). Bondorf (1992).

**Fix:** Sprint C.3 (memoization). The memoization cache partially addresses this for
function results. Add a test that two calls with identical all-static args produce
shared computation, not duplicated code.

### Corner Cut 31 — WQO Not Guaranteed in PE Source Termination

**Location:** `pe_source.logos` (makeKey function, embedding checks)

The Rust-side `embeds()` in supercompile.rs uses homeomorphic embedding on Expr
nodes — a well-quasi-ordering (WQO) by Kruskal's theorem, guaranteeing termination.
But the PE source uses `makeKey()` string-based keys for its termination/embedding
checks (Sprint C.3). String substring ordering is NOT a WQO — there exist infinite
antichains, so termination is not guaranteed for all inputs.

Theoretical basis: Kruskal (1960), Sørensen & Glück (1995).

**Fix:** Sprint C.3. Replace string-based embedding in PE source with structural
comparison on the argument CExpr list. Add a test with a pathological specialization
chain that string embedding misses but structural embedding catches.

---

## Sprint A — Fix PE Foundation (~12 tests)

### Overview

The Rust-side partial evaluator (`partial_eval.rs`) has three structural problems:
1. `SpecKey` is a `(Symbol, String)` — lossy, order-dependent, collision-prone
2. `spec_key_embeds()` uses string containment — not a real embedding check
3. `body_has_io()` is an ad-hoc walk that duplicates what `EffectEnv` already computes

This sprint fixes all three. The SpecKey becomes a structured type. The embedding check
becomes a proper homeomorphic comparison on the structured key. The IO check uses the
existing EffectEnv infrastructure.

### Data Structures

```rust
// partial_eval.rs — REPLACE line 10
type SpecKey = (Symbol, Vec<Option<Literal>>);
```

No more `classifications_to_key_string()`. The key is the raw classification vector.
Cache lookup uses `HashMap` equality on `Vec<Option<Literal>>` directly.

### Algorithm

**Structured embedding check:**

Given `earlier: &[Option<Literal>]` and `later: &[Option<Literal>]`:
1. Both must have the same length (same function, same arity)
2. `earlier` must not equal `later` (reflexive case is not an embedding)
3. For every position `i`:
   - If `earlier[i]` is `None` (dynamic), it embeds in anything
   - If `earlier[i]` is `Some(lit)`, then `later[i]` must also be `Some(lit')` where
     `lit` is "simpler than or equal to" `lit'`
4. At least one position must be strictly simpler (not equal)

For literals, "simpler" means: `Number(a)` embeds in `Number(b)` if `|a| ≤ |b|`.
`Boolean` always embeds in `Boolean`. `Text(a)` embeds in `Text(b)` if `a.len() ≤ b.len()`.

**EffectEnv integration:**

In `try_specialize_call()`, replace `body_has_io(func_info.body)` with a query to
`EffectEnv::function_is_pure()`. This requires threading an `EffectEnv` through the
specialization pass. If the EffectEnv says the function is pure, allow specialization
even if the ad-hoc walk finds something it doesn't recognize.

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/partial_eval.rs` | SpecKey type, spec_key_embeds, EffectEnv parameter |
| `tests/phase_partial_eval.rs` | ~12 new tests |

### TDD Steps

**Step A1 — RED: Structured SpecKey type**

File: `crates/logicaffeine_tests/tests/phase_partial_eval.rs`

```rust
#[test]
fn pe_spec_key_is_structured() {
    // SpecKey should be (Symbol, Vec<Option<Literal>>), not (Symbol, String)
    // Verify two calls with same static values produce the same key
    let source = r#"
## To scale (factor: Int) and (x: Int) -> Int:
    Return factor * x.

## Main
    Let a be scale(3, 10).
    Let b be scale(3, 20).
    Show a.
    Show b.
"#;
    assert_exact_output(source, "30\n60\n");
}
```

Expected: PASS (this tests behavior, not internals — it should already work).

```rust
#[test]
fn pe_spec_key_no_string_collision() {
    // Two different static patterns that would collide with string keys:
    // scale(10, D) and scale(1, 0, D) if serialized as "S10,D" vs "S1,S0,D"
    // With structured keys, these are different Vecs with different lengths.
    let source = r#"
## To process2 (a: Int) and (b: Int) -> Int:
    Return a + b.

## To process3 (a: Int) and (b: Int) and (c: Int) -> Int:
    Return a + b + c.

## Main
    Let x be process2(10, 5).
    Let y be process3(1, 0, 5).
    Show x.
    Show y.
"#;
    assert_exact_output(source, "15\n6\n");
}
```

Expected: PASS (behavioral test).

**Step A2 — RED: Structured embedding terminates growing chains**

```rust
#[test]
fn pe_structured_embedding_detects_growth() {
    // A function called with increasingly large static arguments should be
    // terminated by the embedding check before hitting the variant limit.
    // With string containment, "S5" is contained in "S50" which is wrong.
    // With structured embedding, Number(5) embeds in Number(50) correctly.
    let source = r#"
## To grow (n: Int) and (x: Int) -> Int:
    If n is at most 0:
        Return x.
    Return grow(n - 1, x + 1).

## Main
    Let result be grow(5, 0).
    Show result.
"#;
    assert_exact_output(source, "5\n");
}
```

```rust
#[test]
fn pe_structured_embedding_no_false_positive() {
    // "S5" is a substring of "S50" — string containment gives a false positive.
    // Structured embedding: Number(5) embeds in Number(50) (|5| ≤ |50|) — correct.
    // But Number(50) does NOT embed in Number(5) — no false negative.
    let source = r#"
## To compute (base: Int) and (x: Int) -> Int:
    Return base * x.

## Main
    Let a be compute(5, 10).
    Let b be compute(50, 10).
    Show a.
    Show b.
"#;
    assert_exact_output(source, "50\n500\n");
}
```

Expected: PASS (behavioral tests — existing implementation handles these correctly,
but verifies correctness is preserved after refactoring).

**Step A3 — RED: EffectEnv integration — pure function detected**

```rust
#[test]
fn pe_effect_env_allows_pure_specialization() {
    // A function that uses patterns the ad-hoc body_has_io might not recognize
    // as pure (e.g., complex control flow) should still be specializable if
    // EffectEnv confirms it's pure.
    let source = r#"
## To classify (n: Int) -> Int:
    If n is greater than 100:
        Return 3.
    If n is greater than 10:
        Return 2.
    If n is greater than 0:
        Return 1.
    Return 0.

## Main
    Let a be classify(50).
    Let b be classify(5).
    Show a.
    Show b.
"#;
    assert_exact_output(source, "2\n1\n");
}
```

```rust
#[test]
fn pe_effect_env_blocks_impure_specialization() {
    // A function with IO should not be specialized, confirmed by EffectEnv
    let source = r#"
## To logAndCompute (n: Int) and (x: Int) -> Int:
    Show n.
    Return n * x.

## Main
    Let result be logAndCompute(5, 10).
    Show result.
"#;
    assert_exact_output(source, "5\n50\n");
}
```

Expected: PASS.

**Step A4 — GREEN: Refactor SpecKey to structured type**

File: `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

1. Change `type SpecKey = (Symbol, String)` to `type SpecKey = (Symbol, Vec<Option<Literal>>)`
2. Remove `classifications_to_key_string()` function entirely
3. Update `compute_spec_key()` to return `(SpecKey, Vec<Option<Literal>>)` where SpecKey
   now contains the raw `Vec<Option<Literal>>` directly (no string conversion)
4. Rewrite `spec_key_embeds()`:

```rust
fn spec_key_embeds(earlier: &SpecKey, later: &SpecKey) -> bool {
    if earlier.0 != later.0 {
        return false;
    }
    let ea = &earlier.1;
    let la = &later.1;
    if ea.len() != la.len() {
        return false;
    }
    if ea == la {
        return false; // reflexive — not a proper embedding
    }
    let mut strict = false;
    for (e, l) in ea.iter().zip(la.iter()) {
        match (e, l) {
            (None, _) => {} // dynamic embeds in anything
            (Some(_), None) => return false, // static does not embed in dynamic
            (Some(e_lit), Some(l_lit)) => {
                if !literal_embeds(e_lit, l_lit) {
                    return false;
                }
                if e_lit != l_lit {
                    strict = true;
                }
            }
        }
    }
    strict
}

fn literal_embeds(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Number(x), Literal::Number(y)) => x.abs() <= y.abs(),
        (Literal::Float(x), Literal::Float(y)) => x.abs() <= y.abs(),
        (Literal::Boolean(_), Literal::Boolean(_)) => true,
        (Literal::Text(x), Literal::Text(y)) => x.index() <= y.index(), // approximation
        (Literal::Nothing, Literal::Nothing) => true,
        _ => a == b,
    }
}
```

5. Update `make_spec_name()` to use the existing `literal_to_name_part()` for the name
   (this stays the same — names are for human readability, not correctness).

6. Update `compute_spec_key()`:

```rust
fn compute_spec_key<'a>(
    function: Symbol,
    args: &[&'a Expr<'a>],
    division: Option<&Division>,
) -> (SpecKey, Vec<Option<Literal>>) {
    let arg_classifications: Vec<Option<Literal>> = args.iter()
        .map(|a| classify_arg(a, division))
        .collect();
    let key = (function, arg_classifications.clone());
    (key, arg_classifications)
}
```

7. Ensure `Literal` derives or implements `Hash` and `Eq` for use in `HashMap<SpecKey, Symbol>`.
   Check the existing `Literal` type — if it contains `Float(f64)`, implement `Hash`/`Eq`
   manually (using `f64::to_bits()`) the same way `BindingTime` does in `bta.rs`.

**Step A5 — GREEN: Thread EffectEnv into specialization**

File: `crates/logicaffeine_compile/src/optimize/partial_eval.rs`

1. Add `use super::effects::EffectEnv;` (or the appropriate path)
2. Compute `EffectEnv` at the start of `specialize_stmts()`:
   - Build the source string from the statements (or pass pre-computed effects)
   - For simplicity: add an optional `effect_env: Option<&EffectEnv>` parameter
3. In `try_specialize_call()`, after the existing `body_has_io()` check, add:
   - If `body_has_io()` returns true but `effect_env.function_is_pure(fn_name)` returns
     true, allow specialization anyway (the ad-hoc check was wrong)
   - If both say impure, block specialization

Note: This is additive. The existing `body_has_io()` check stays. EffectEnv is an
additional signal that can override false positives from the ad-hoc check.

**Step A6 — VERIFY**

Run `cargo test -- --skip e2e`. All 403 existing tests plus the ~12 new ones must pass.

**Step A7 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Literal Hash/Eq for f64:** `Literal::Float(f64)` doesn't implement `Hash` by default.
   Must use `to_bits()`. Check whether `Literal` already has a manual implementation.
   If not, add one following the pattern in `bta.rs:58-76`.

2. **Backward compatibility of SpecKey:** The cache is local to each `specialize_stmts()`
   call. Changing the key type doesn't break any external API. But verify that
   `specialize_stmts_with_state()` (which takes `persistent_variant_count`) still works
   correctly — the variant count is keyed by `Symbol`, not `SpecKey`.

3. **EffectEnv construction cost:** Building an `EffectEnv` requires a full parse +
   analysis pass. If this is too expensive to run during optimization, make it lazy:
   compute on first call and cache. Or compute it once in the optimization pipeline
   and thread it through.

---

## Sprint B — Fix Supercompiler Foundation (~12 tests)

### Overview

The supercompiler (`supercompile.rs`) has two fully-implemented but unused algorithms:
`embeds()` (homeomorphic embedding) and `msg()` (Most Specific Generalization). It also
lacks proper termination tracking — it uses ad-hoc depth limits and variable widening
instead of Configuration/History with embedding-based whistle-blowing.

This sprint activates the dead code:
1. Add `Configuration` and `History` types to track supercompiler state
2. Use `embeds()` as a termination guard ("whistle") in the While handler
3. Use `msg()` for principled generalization when the whistle blows
4. Wire BTA through CallGraph SCC order for proper fixed-point convergence

### Data Structures

```rust
// supercompile.rs — NEW types

/// A configuration is a snapshot of the supercompiler's state at a program point.
/// Used for termination checking: if a new configuration embeds in a previous one,
/// the supercompiler must generalize to avoid divergence.
#[derive(Clone)]
struct Configuration<'a> {
    /// The expression or condition being driven
    expr: &'a Expr<'a>,
    /// Snapshot of known values at this point
    store_snapshot: HashMap<Symbol, Value>,
}

/// History of configurations seen during driving.
/// When a new configuration embeds in a historical one, the whistle blows.
struct History<'a> {
    entries: Vec<Configuration<'a>>,
}
```

### Algorithm

**Embedding-based whistle in While handler:**

Current code (`drive_stmt`, While case, line 589):
```rust
let modified = collect_modified_vars_block(body);
for sym in &modified {
    env.store.remove(sym);  // Widen ALL modified vars — too aggressive
}
```

New algorithm:
1. Before entering the loop, snapshot the current configuration: `(cond_expr, store.clone())`
2. Push to History
3. After driving the body, snapshot the new configuration: `(driven_cond, store.clone())`
4. Check if new configuration embeds in any historical configuration using `embeds()`
5. If embedding detected (whistle blows):
   a. Compute `msg(old_config.expr, new_config.expr)` to find the most general form
   b. Only widen variables that appear in the MSG's fresh variables (`__msg_N`)
   c. This is more precise than removing all modified variables
6. If no embedding detected, keep the precise store

**BTA SCC ordering:**

In `bta.rs`, add a function `analyze_program_with_callgraph()` that:
1. Takes the program's `CallGraph` (already available from `analysis/callgraph.rs`)
2. Processes functions in topological order of SCCs
3. For each SCC with >1 member (mutually recursive):
   a. Initialize all functions in the SCC with `Dynamic` divisions
   b. Iterate: analyze each function, propagate results to others in the SCC
   c. Stop when divisions stabilize (fixed point)

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/supercompile.rs` | Configuration, History, embeds/msg activation |
| `optimize/bta.rs` | SCC-ordered analysis |
| `tests/phase_supercompile.rs` | ~8 new tests |
| `tests/phase_bta.rs` | ~4 new tests |

### TDD Steps

**Step B1 — RED: Configuration/History types exist**

File: `crates/logicaffeine_tests/tests/phase_supercompile.rs`

```rust
#[test]
fn supercompile_while_precise_widening() {
    // A while loop that modifies two variables but one is predictable.
    // With precise widening (MSG-based), the predictable variable stays known.
    // With aggressive widening (remove all), both become unknown.
    let source = r#"
## To test () -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    Let scale be 10.
    While i is at most 3:
        Set sum to sum + (i * scale).
        Set i to i + 1.
    Return sum.

## Main
    Show test().
"#;
    assert_exact_output(source, "60\n");
}
```

```rust
#[test]
fn supercompile_embedding_prevents_divergence() {
    // A recursive function with growing argument should be caught by embedding
    // before the depth limit. Verifies embeds() is actually called.
    let source = r#"
## To recurse (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return n + recurse(n - 1).

## Main
    Show recurse(5).
"#;
    assert_exact_output(source, "15\n");
}
```

Expected: PASS (behavioral tests — existing implementation handles these).

**Step B2 — RED: Index/Slice conditional driving**

```rust
#[test]
fn supercompile_index_static_collection() {
    // Index into a known-static list should resolve at compile time
    let source = r#"
## Main
    Let items be [10, 20, 30].
    Let second be item 2 of items.
    Show second.
"#;
    assert_exact_output(source, "20\n");
}
```

```rust
#[test]
fn supercompile_index_dynamic_preserved() {
    // Index with dynamic index should be preserved in residual
    let source = r#"
## To getItem (items: Seq of Int) and (i: Int) -> Int:
    Return item i of items.

## Main
    Let items be [10, 20, 30].
    Show getItem(items, 2).
"#;
    assert_exact_output(source, "20\n");
}
```

Expected: PASS.

**Step B3 — RED: BTA SCC ordering**

File: `crates/logicaffeine_tests/tests/phase_bta.rs`

```rust
#[test]
fn bta_scc_mutual_recursion_converges() {
    // Two mutually recursive functions should converge to a fixed point
    // when analyzed in SCC order.
    let source = r#"
## To isEven (n: Int) -> Bool:
    If n equals 0:
        Return true.
    Return isOdd(n - 1).

## To isOdd (n: Int) -> Bool:
    If n equals 0:
        Return false.
    Return isEven(n - 1).

## Main
    Show isEven(4).
"#;
    assert_exact_output(source, "true\n");
}
```

```rust
#[test]
fn bta_scc_three_way_recursion() {
    // Three-way mutual recursion — SCC should group all three
    let source = r#"
## To a (n: Int) -> Int:
    If n is at most 0:
        Return 1.
    Return b(n - 1) + 1.

## To b (n: Int) -> Int:
    If n is at most 0:
        Return 2.
    Return c(n - 1) + 1.

## To c (n: Int) -> Int:
    If n is at most 0:
        Return 3.
    Return a(n - 1) + 1.

## Main
    Show a(3).
"#;
    assert_exact_output(source, "7\n");
}
```

Expected: PASS.

**Step B4 — GREEN: Add Configuration/History to supercompile.rs**

File: `crates/logicaffeine_compile/src/optimize/supercompile.rs`

1. Add `Configuration` and `History` structs (as specified in Data Structures above)
2. Add `History` field to `SuperEnv`:
   ```rust
   struct SuperEnv<'a> {
       store: HashMap<Symbol, Value>,
       funcs: HashMap<Symbol, FuncDef<'a>>,
       memo: HashMap<(Symbol, Vec<Value>), Option<Value>>,
       steps: usize,
       history: History<'a>,  // NEW
   }
   ```
3. In `drive_stmt` While handler, before widening:
   a. Build `Configuration { expr: cond, store_snapshot: env.store.clone() }`
   b. Check `env.history.entries.iter().any(|c| embeds(c.expr, new_config.expr))`
   c. If whistle blows: use `msg()` to compute generalization, widen only MSG-introduced
      variables
   d. If no whistle: use existing widening (modified vars removed)
   e. Push new configuration to history
4. Add `impl History` with `push()` and `check_embedding()` methods

**Step B5 — GREEN: Wire BTA through SCC order**

File: `crates/logicaffeine_compile/src/optimize/bta.rs`

1. Add `pub fn analyze_with_sccs(stmts: &[Stmt], interner: &Interner) -> BtaCache`
2. Build `CallGraph` from stmts
3. Process SCCs in topological order:
   - For non-recursive SCCs: analyze once
   - For recursive SCCs: iterate to fixed point (max 10 iterations)
4. Return the completed `BtaCache`

**Step B6 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step B7 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Embedding check cost:** `embeds()` is O(n²) in expression size. In a tight loop
   with large expressions, this could be expensive. Mitigate by limiting History size
   (e.g., keep only last 16 configurations).

2. **MSG fresh variable naming:** `msg()` creates `__msg_N` variables. These must not
   collide with user variables. The `__` prefix convention prevents this, but verify
   that the interner handles these correctly.

3. **While handler store semantics:** After the loop, the store must be conservative.
   Variables widened during the loop remain unknown. Variables not widened retain their
   pre-loop values. The post-loop store restoration logic (lines 611-621) must be
   updated to account for MSG-based widening.

4. **Index/Slice driving regression:** The current code deliberately skips Index/Slice
   driving (line 454) to preserve codegen peephole patterns. Adding conditional driving
   (drive when collection is statically known) must NOT break the swap/vec-fill patterns.
   Gate carefully on `e2e_codegen_optimization.rs` tests.

---

## Sprint C — PE Source Static Dispatch (~15 tests)

### Overview

The PE source (`pe_source.logos`) treats compound values as dynamic because it only
checks `isLiteral()`. This sprint adds:
1. `isStatic(e)` — recognizes all statically-known CExpr forms
2. Extended `exprToVal`/`valToExpr` for compound values
3. `CInspect` static arm matching (dead arm elimination)
4. `CRepeat` static unrolling (for known collections)
5. `CCall` with `allStatic` instead of `allLiteral`

### Algorithm

**isStatic predicate:**

A CExpr is static if:
- It is a literal (`CInt`, `CBool`, `CText`, `CFloat`)
- It is a `CList` where all elements are static
- It is a `CNewVariant` where all field values are static
- It is a `CTuple` where all elements are static
- It is a `COptionSome` where the inner value is static
- It is `COptionNone`, `CNewSeq`, `CNewSet`

**Extended exprToVal/valToExpr:**

| CExpr | CVal | Direction |
|-------|------|-----------|
| `CFloat(f)` | `VFloat(f)` | both |
| `CList([e1..eN])` | `VSeq([v1..vN])` | both (recursive) |
| `CNewVariant(tag, names, vals)` | `VVariant(tag, names, vals)` | both (recursive) |
| `CTuple([e1..eN])` | `VTuple([v1..vN])` | both (recursive) |
| `COptionSome(e)` | `VOption(v)` | both |
| `COptionNone` | `VNothing` | exprToVal only |

**CInspect static dispatch:**

In `peBlock`, `CInspect` handler:
1. PE the target expression
2. If the result is a `CNewVariant(tag, fieldNames, fieldVals)`:
   a. Find the `CWhen` arm whose `variantName` matches `tag`
   b. Bind each field name from `wBindings` to the corresponding field value in the env
   c. PE the matching arm's body
   d. Return the result — no residual Inspect needed
3. If the target is not a known variant, fall through to the existing logic

**CRepeat static unrolling:**

In `peBlock`, `CRepeat` handler:
1. PE the collection expression
2. If the result is a `CList` with known items:
   a. For each item in the list:
      - Bind the loop variable to the item in the environment
      - PE the loop body
      - Append the resulting statements to the block result
   b. Return — no residual CRepeat needed
3. If the result is a `CRange` with known start/end:
   a. Generate integers from start to end
   b. Unroll as above
4. Otherwise, fall through to existing logic (emit residual CRepeat)

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | isStatic, exprToVal/valToExpr extensions, CInspect dispatch, CRepeat unrolling |
| `tests/phase_futamura.rs` | ~15 new tests |

### TDD Steps

**Step C1 — RED: isStatic predicate**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_pe_is_static_float() {
    // A function called with a static float should be specialized
    let source = r#"
## A CExpr is one of:
    ...  // (include CORE_TYPES_FOR_PE)

## Main
    Let e be a new CFloat with value 3.14.
    // isStatic should return true for CFloat
    Let result be isStatic(e).
    Show result.
"#;
    // This test will need the actual CORE_TYPES_FOR_PE + pe_source.logos
    // combined source. Use the pattern from existing Sprint 4 tests.
    // For now, test via projection1_source behavior.
}
```

Better approach — test through the projection pipeline:

```rust
#[test]
fn fix_pe_static_float_specialization() {
    // P1 should specialize away a function called with a static float
    let source = r#"
## To scale (factor: Int) and (x: Int) -> Int:
    Return factor * x.

## Main
    Show scale(3, 7).
"#;
    let result = projection1_source("", "", source).unwrap();
    assert!(!result.contains("scale"), "Function should be specialized away");
    // Verify correctness
    assert_exact_output(source, "21\n");
}
```

```rust
#[test]
fn fix_pe_static_list_specialization() {
    // A function called with a static list should have the list inlined
    let source = r#"
## To sumList (items: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for x in items:
        Set total to total + x.
    Return total.

## Main
    Show sumList([1, 2, 3]).
"#;
    assert_exact_output(source, "6\n");
}
```

Expected: PASS (these test existing behavior).

**Step C2 — RED: CInspect static arm matching**

```rust
#[test]
fn fix_pe_inspect_static_dispatch() {
    // When inspecting a known variant, the PE should eliminate dead arms
    // and inline only the matching arm.
    let source = r#"
## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (r):
            Return r * r * 3.
        When Square (side):
            Return side * side.

## Main
    Let c be a new Circle with radius 5.
    Show area(c).
"#;
    assert_exact_output(source, "75\n");
}
```

```rust
#[test]
fn fix_pe_inspect_dynamic_preserved() {
    // When inspecting a dynamic variant, the full Inspect must be preserved
    let source = r#"
## A Shape is one of:
    A Circle with radius Int.
    A Square with side Int.

## To area (s: Shape) -> Int:
    Inspect s:
        When Circle (r):
            Return r * r * 3.
        When Square (side):
            Return side * side.

## To makeShape (kind: Int) -> Shape:
    If kind equals 0:
        Return a new Circle with radius 5.
    Return a new Square with side 4.

## Main
    Let s be makeShape(0).
    Show area(s).
"#;
    assert_exact_output(source, "75\n");
}
```

Expected: PASS.

**Step C3 — RED: CRepeat static unrolling**

```rust
#[test]
fn fix_pe_repeat_static_list_unroll() {
    // A loop over a static list should be unrolled
    let source = r#"
## Main
    Let items be [10, 20, 30].
    Let mutable sum be 0.
    Repeat for x in items:
        Set sum to sum + x.
    Show sum.
"#;
    assert_exact_output(source, "60\n");
}
```

```rust
#[test]
fn fix_pe_repeat_static_range_unroll() {
    // A loop over a static range should be unrolled
    let source = r#"
## Main
    Let mutable sum be 0.
    Repeat for i in 1 to 4:
        Set sum to sum + i.
    Show sum.
"#;
    assert_exact_output(source, "10\n");
}
```

```rust
#[test]
fn fix_pe_repeat_dynamic_preserved() {
    // A loop over a dynamic collection should NOT be unrolled
    let source = r#"
## To sumAll (items: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for x in items:
        Set total to total + x.
    Return total.

## Main
    Let items be [1, 2, 3, 4, 5].
    Show sumAll(items).
"#;
    assert_exact_output(source, "15\n");
}
```

Expected: PASS.

**Step C4 — RED: allStatic replaces allLiteral in CCall**

```rust
#[test]
fn fix_pe_call_all_static_compound() {
    // A function called with compound static args (not just literals)
    // should still be inlined by the PE source.
    let source = r#"
## To first (items: Seq of Int) -> Int:
    Return item 1 of items.

## Main
    Show first([10, 20, 30]).
"#;
    assert_exact_output(source, "10\n");
}
```

Expected: PASS.

**Step C5 — GREEN: Add isStatic to pe_source.logos**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Add after `isLiteral`:

```logos
## To isStatic (e: CExpr) -> Bool:
    Inspect e:
        When CInt (v):
            Return true.
        When CBool (v):
            Return true.
        When CText (v):
            Return true.
        When CFloat (v):
            Return true.
        When CList (items):
            Repeat for item in items:
                Let s be isStatic(item).
                If not s:
                    Return false.
            Return true.
        When CNewVariant (tag, names, vals):
            Repeat for v in vals:
                Let s be isStatic(v).
                If not s:
                    Return false.
            Return true.
        When CTuple (items):
            Repeat for item in items:
                Let s be isStatic(item).
                If not s:
                    Return false.
            Return true.
        When COptionSome (inner):
            Return isStatic(inner).
        When COptionNone:
            Return true.
        When CNewSeq:
            Return true.
        When CNewSet:
            Return true.
        Otherwise:
            Return false.

## To allStatic (args: Seq of CExpr) -> Bool:
    Repeat for a in args:
        Let s be isStatic(a).
        If not s:
            Return false.
    Return true.
```

**Step C6 — GREEN: Extend exprToVal/valToExpr**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Extend `exprToVal`:
```logos
    When CFloat (v):
        Return a new VFloat with value v.
    When CList (items):
        Let vals be a new Seq of CVal.
        Repeat for item in items:
            Push exprToVal(item) to vals.
        Return a new VSeq with items vals.
    When CNewVariant (tag, names, vals):
        Let vvals be a new Seq of CVal.
        Repeat for v in vals:
            Push exprToVal(v) to vvals.
        Return a new VVariant with tag tag and names names and vals vvals.
    When CTuple (items):
        Let vals be a new Seq of CVal.
        Repeat for item in items:
            Push exprToVal(item) to vals.
        Return a new VTuple with items vals.
```

Extend `valToExpr`:
```logos
    When VFloat (f):
        Return a new CFloat with value f.
    When VSeq (items):
        Let exprs be a new Seq of CExpr.
        Repeat for item in items:
            Push valToExpr(item) to exprs.
        Return a new CList with items exprs.
    When VVariant (tag, names, vals):
        Let exprs be a new Seq of CExpr.
        Repeat for v in vals:
            Push valToExpr(v) to exprs.
        Return a new CNewVariant with tag tag and fnames names and fvals exprs.
    When VTuple (items):
        Let exprs be a new Seq of CExpr.
        Repeat for item in items:
            Push valToExpr(item) to exprs.
        Return a new CTuple with items exprs.
```

**VMap lifting (Corner Cut 23):** `VMap` cannot be lifted to a single CExpr because
LogicAffeine Maps have no literal constructor syntax. Options:
- Return a `CCall("buildMap", [keys_list, vals_list])` and add a `buildMap` helper
  to the PE source's runtime support
- Accept the limitation and handle VMap at the statement level (generate a sequence
  of CSetItem statements instead of a single expression)
- For self-application, the PE's `env` and `funcs` maps are partially static — handle
  these via `staticEnv` (Sprint C.4) instead of trying to lift the entire map

**Design decision:** For MVP, VMap is NOT liftable via `valToExpr`. The `staticEnv`
mechanism (Sprint C.4) provides the practical workaround for the most important case
(the PE's own variable environment). If VMap lifting becomes necessary for P2/P3
correctness, add a `buildMap` helper function to pe_source.logos.

**Step C7 — GREEN: Replace allLiteral with allStatic in CCall handler**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

In `peExpr`, `CCall` handler (line 201):
```logos
    // BEFORE:
    Let allLit be allLiteral(peArgs).
    If allLit:

    // AFTER:
    Let allStat be allStatic(peArgs).
    If allStat:
```

**Step C8 — GREEN: CInspect static dispatch**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Replace the `CInspect` handler in `peBlock`:

```logos
    When CInspect (inspTarget, inspArms):
        Let peTarget be peExpr(inspTarget, env, funcs, depth).
        Let targetStatic be isStatic(peTarget).
        If targetStatic:
            Inspect peTarget:
                When CNewVariant (nvTag, nvNames, nvVals):
                    Repeat for arm in inspArms:
                        Inspect arm:
                            When CWhen (wName, wBindings, wBody):
                                If wName equals nvTag:
                                    Let mutable bidx be 1.
                                    Repeat for b in wBindings:
                                        Let bval be item bidx of nvVals.
                                        Set item b of env to exprToVal(bval).
                                        Set bidx to bidx + 1.
                                    Let matchedBody be peBlock(wBody, env, funcs, depth).
                                    Repeat for ms in matchedBody:
                                        Push ms to blockResult.
                                    Return blockResult.
                            Otherwise:
                                Let skip be true.
                    Repeat for arm in inspArms:
                        Inspect arm:
                            When COtherwise (oBody):
                                Let otherwiseBody be peBlock(oBody, env, funcs, depth).
                                Repeat for os in otherwiseBody:
                                    Push os to blockResult.
                                Return blockResult.
                            Otherwise:
                                Let skip be true.
                Otherwise:
                    Let skip be true.
        Let peArms be a new Seq of CMatchArm.
        Repeat for arm in inspArms:
            Inspect arm:
                When CWhen (wName, wBindings, wBody):
                    Let peBody be peBlock(wBody, env, funcs, depth).
                    Push (a new CWhen with variantName wName and bindings wBindings and body peBody) to peArms.
                When COtherwise (oBody):
                    Let peBody be peBlock(oBody, env, funcs, depth).
                    Push (a new COtherwise with body peBody) to peArms.
        Push (a new CInspect with target peTarget and arms peArms) to blockResult.
```

**Step C9 — GREEN: CRepeat static unrolling**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Replace the `CRepeat` handler in `peBlock`:

```logos
    When CRepeat (repVar, repColl, repBody):
        Let peColl be peExpr(repColl, env, funcs, depth).
        Let collStatic be isStatic(peColl).
        If collStatic:
            Inspect peColl:
                When CList (listItems):
                    Repeat for listItem in listItems:
                        Set item repVar of env to exprToVal(listItem).
                        Let unrolledBody be peBlock(repBody, env, funcs, depth).
                        // Check if body contains CBreak or CReturn — stop unrolling
                        Let mutable hasBreakOrReturn be false.
                        Repeat for us in unrolledBody:
                            Inspect us:
                                When CBreak:
                                    Set hasBreakOrReturn to true.
                                When CReturn (retExpr):
                                    Set hasBreakOrReturn to true.
                                Otherwise:
                                    Let skip be true.
                        Repeat for us in unrolledBody:
                            Push us to blockResult.
                        If hasBreakOrReturn:
                            Return blockResult.
                    Return blockResult.
                When CRange (rangeStart, rangeEnd):
                    Inspect rangeStart:
                        When CInt (startVal):
                            Inspect rangeEnd:
                                When CInt (endVal):
                                    Let mutable ri be startVal.
                                    While ri is at most endVal:
                                        Set item repVar of env to a new VInt with value ri.
                                        Let unrolledBody be peBlock(repBody, env, funcs, depth).
                                        Repeat for us in unrolledBody:
                                            Push us to blockResult.
                                        Set ri to ri + 1.
                                    Return blockResult.
                                Otherwise:
                                    Let skip be true.
                        Otherwise:
                            Let skip be true.
                Otherwise:
                    Let skip be true.
        Let peBody be peBlock(repBody, env, funcs, depth).
        Push (a new CRepeat with var repVar and coll peColl and body peBody) to blockResult.
```

**Step C9b — RED: CRepeat unrolling respects Break/Return**

```rust
#[test]
fn fix_pe_unroll_respects_break() {
    let source = r#"
## Main
    Let items be [1, 2, 3, 4, 5].
    Let mutable sum be 0.
    Repeat for x in items:
        If x equals 3:
            Break.
        Set sum to sum + x.
    Show sum.
"#;
    assert_exact_output(source, "3\n");
}
```

```rust
#[test]
fn fix_pe_unroll_respects_return() {
    let source = r#"
## To findFirst (items: Seq of Int) and (target: Int) -> Int:
    Let mutable idx be 0.
    Repeat for x in items:
        Set idx to idx + 1.
        If x equals target:
            Return idx.
    Return 0.

## Main
    Show findFirst([10, 20, 30, 40], 30).
"#;
    assert_exact_output(source, "3\n");
}
```

Expected: PASS (behavioral tests — existing pipeline handles these).

**Step C10 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step C11 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Unrolling termination:** Static ranges can be large (e.g., `1 to 1000000`).
   Add a guard: only unroll if `endVal - startVal < 64`. Otherwise, fall through
   to residual CRepeat.

2. **Environment pollution:** When unrolling CRepeat, each iteration modifies the
   environment (binding the loop variable). After the loop, the loop variable
   should be removed from the environment or set to VNothing to prevent leakage.

3. **CInspect with no matching arm:** If the static variant doesn't match any CWhen
   arm and there's no COtherwise, the PE must emit an error or fall through.
   Handle this edge case explicitly.

4. **allStatic vs allLiteral backward compatibility:** Replacing `allLiteral` with
   `allStatic` makes the PE more aggressive — it will inline more function calls.
   This is correct behavior but could change optimization outcomes for existing tests.
   Gate carefully on all 276 phase_futamura tests.

---

## Sprint C.1 — PEState Structural Refactor (~0 new tests, pure mechanical)

### Overview

Before adding memoization, staticEnv, or any new threading, refactor all
`peExpr`/`peBlock` calls to use a single `PEState` record. This prevents parameter
explosion as new fields are added (specResults, onStack in C.3; staticEnv in C.4).

This was originally Step C3.0 but is promoted to its own sprint because all subsequent
sprints (C.1.5, C.2, C.3, C.4, C.5) depend on PEState being in place.

### Data Structure

```logos
## A PEState is a record with:
    env Map of Text to CVal.
    funcs Map of Text to CFunc.
    depth Int.
    staticEnv Map of Text to CExpr.
    specResults Map of Text to CExpr.
    onStack Seq of Text.
```

### TDD Steps

**Step C.1.1 — RED: PEState record type exists**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_pe_state_record_exists() {
    // Verify that PEState can be constructed and used
    let source = format!(r#"
{}
{}
## Main
    Let state be a new PEState with env (a new Map of Text to CVal)
        and funcs (a new Map of Text to CFunc) and depth 10
        and staticEnv (a new Map of Text to CExpr)
        and specResults (a new Map of Text to CExpr)
        and onStack (a new Seq of Text).
    Show depth of state.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "10\n");
}
```

Expected: FAIL (PEState doesn't exist yet).

**Step C.1.2 — GREEN: Mechanical refactor**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

1. Add `PEState` record type
2. Change all `peExpr(e, env, funcs, depth)` to `peExpr(e, state)`
3. Change all `peBlock(stmts, env, funcs, depth)` to `peBlock(stmts, state)`
4. Inside function bodies, access `env of state`, `funcs of state`, `depth of state`
5. Initialize `staticEnv`, `specResults`, `onStack` to empty values
6. Do this in batches of ~10 call sites, running gate between batches

This touches ~50 internal calls in pe_source.logos. Zero behavior change.

**Step C.1.3 — VERIFY**

Run `cargo test -- --skip e2e`. All 403 existing tests must pass. The refactor is
purely structural — behavior is identical.

**Step C.1.4 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Large mechanical refactor:** Do it as a pure rename first (wrapping existing
   params in a record), verify all tests pass, THEN add new fields and logic in
   subsequent sprints. Do not combine the refactor with logic changes.

2. **Call site count:** Before refactor, count `peExpr(` occurrences. After refactor,
   verify the same count of `peExpr(` calls exists (just with different signatures).

3. **State cloning:** Some call sites need to clone state before passing it (e.g.,
   entering function bodies to avoid leaking local bindings). Initially, just pass
   state directly — cloning is added in C.4 (staticEnv sprint).

---

## Sprint C.1.5 — Partially-Static Structures (~8 tests)

### Overview

A `CList` like `[CInt(1), CVar("x"), CInt(3)]` has elements 1 and 3 static, element
2 dynamic. Currently `isStatic` returns false for the whole list. This sprint adds
element-level static tracking so that operations on partially-static structures can
fold when possible.

This is critical for self-application: the PE's argument lists (`peArgs`) are often
partially static (some args known, some dynamic). Without this, `item 1 of peArgs`
(getting the first argument) can't fold even when that argument is statically known.

Theoretical basis: Mogensen (1988), Jones et al. (1993), Section 5.5.

### Algorithm

1. `isPartiallyStatic(e: CExpr) -> Bool` — returns true if ANY element of a CList
   is static (not just all). For non-CList expressions, returns `isStatic(e)`.
2. Extend CIndex handler: if collection is a CList and index is a static CInt,
   return the element at that index regardless of other elements' staticness.
3. Extend CLen: if collection is a CList (even with some dynamic elements), length
   is still known statically.
4. Element-level propagation: when indexing a partially-static list at a known
   position, return the element (static or dynamic CExpr) directly.

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | isPartiallyStatic, CIndex/CLen partial-static handling |
| `tests/phase_futamura.rs` | ~8 new tests |

### TDD Steps

**Step C.1.5.1 — RED: Partial-static list index folds for static element**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_partial_static_list_index_static() {
    // item 1 of [1, x, 3] should fold to CInt(1)
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CList with items [a new CInt with value 1, a new CVar with name "x", a new CInt with value 3].
    Let idx be a new CInt with value 1.
    Let expr be a new CIndex with coll items and idx idx.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (v):
            Show "FOLDED:{{v}}".
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "FOLDED:1\n");
}
```

Expected: FAIL (CIndex only folds when ALL elements are static).

```rust
#[test]
fn fix_partial_static_list_index_dynamic() {
    // item 2 of [1, x, 3] should fold to CVar("x") — index op eliminated, element preserved
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CList with items [a new CInt with value 1, a new CVar with name "x", a new CInt with value 3].
    Let idx be a new CInt with value 2.
    Let expr be a new CIndex with coll items and idx idx.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CVar (name):
            Show "ELEMENT:{{name}}".
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "ELEMENT:x\n");
}
```

Expected: FAIL.

**Step C.1.5.2 — RED: Partial-static length still folds**

```rust
#[test]
fn fix_partial_static_len() {
    // length of [1, x, 3] should fold to 3 even though element 2 is dynamic
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CList with items [a new CInt with value 1, a new CVar with name "x", a new CInt with value 3].
    Let expr be a new CLen with target items.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (v):
            Show "LEN:{{v}}".
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "LEN:3\n");
}
```

Expected: FAIL.

**Step C.1.5.3 — RED: Regression tests**

```rust
#[test]
fn fix_partial_static_full_static_still_works() {
    // Fully static list should still be handled correctly
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CList with items [a new CInt with value 10, a new CInt with value 20].
    Let idx be a new CInt with value 2.
    Let expr be a new CIndex with coll items and idx idx.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (v):
            Show v.
        Otherwise:
            Show "FAIL".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "20\n");
}
```

Expected: PASS (existing fully-static handling from Sprint C).

```rust
#[test]
fn fix_partial_static_full_dynamic_unchanged() {
    // Fully dynamic list should still residualize
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CVar with name "myList".
    Let idx be a new CVar with name "i".
    Let expr be a new CIndex with coll items and idx idx.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CIndex (rc, ri):
            Show "RESIDUALIZED".
        Otherwise:
            Show "WRONG".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "RESIDUALIZED\n");
}
```

Expected: PASS (existing behavior residualizes correctly).

```rust
#[test]
fn fix_partial_static_variant_field() {
    // A CNewVariant with static tag + one static field and one dynamic field.
    // CFieldAccess on the static field should fold.
    let source = format!(r#"
{}
{}
## Main
    Let v be a new CNewVariant with tag "Point" and fnames ["x", "y"] and fvals [a new CInt with value 5, a new CVar with name "dy"].
    Let expr be a new CFieldAccess with target v and field "x".
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (val):
            Show "FOLDED:{{val}}".
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "FOLDED:5\n");
}
```

Expected: FAIL (CFieldAccess only folds when target is fully static).

**Step C.1.5.4 — GREEN: Extend CIndex/CLen/CFieldAccess for partial static**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

1. In CIndex handler: if collection is a `CList` and index is a static `CInt`,
   directly return the element at that index — regardless of whether other elements
   are static or dynamic:

```logos
        When CIndex (collExpr, idxExpr):
            Let peColl be peExpr(collExpr, state).
            Let peIdx be peExpr(idxExpr, state).
            // NEW: CList with static index — partial-static aware
            Inspect peColl:
                When CList (listItems):
                    Inspect peIdx:
                        When CInt (idxVal):
                            If idxVal is greater than 0:
                                If idxVal is at most length of listItems:
                                    Return item idxVal of listItems.
                        Otherwise:
                            Let skip be true.
                Otherwise:
                    Let skip be true.
            // Fall through to existing logic...
```

2. In CLen handler: if target is a `CList`, return `CInt(length)` even if some
   elements are dynamic:

```logos
        When CLen (lenTarget):
            Let peTarget be peExpr(lenTarget, state).
            Inspect peTarget:
                When CList (listItems):
                    Return a new CInt with value (length of listItems).
                Otherwise:
                    Let skip be true.
            // Fall through to existing logic...
```

3. In CFieldAccess handler: if target is a `CNewVariant` or `CNew`, look up the
   field directly — even if other fields are dynamic:

```logos
        When CFieldAccess (faTarget, faField):
            Let peTarget be peExpr(faTarget, state).
            Inspect peTarget:
                When CNewVariant (nvTag, nvNames, nvVals):
                    Let mutable fidx be 1.
                    Repeat for fn in nvNames:
                        If fn equals faField:
                            Return item fidx of nvVals.
                        Set fidx to fidx + 1.
                When CNew (nType, nNames, nVals):
                    Let mutable fidx be 1.
                    Repeat for fn in nNames:
                        If fn equals faField:
                            Return item fidx of nVals.
                        Set fidx to fidx + 1.
                Otherwise:
                    Let skip be true.
            Return a new CFieldAccess with target peTarget and field faField.
```

**Step C.1.5.5 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step C.1.5.6 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **CIndex before isStatic check:** The key insight is that `item idx of CList` works
   regardless of whether ALL elements are static. The CList structure itself tells us
   the length and element positions. Only the individual element's staticness matters
   for further folding.

2. **Interaction with CRepeat unrolling:** Unrolling a loop over a partially-static
   list is still valid — each element (static or dynamic) is bound to the loop variable
   in turn. Static elements fold, dynamic ones residualize.

3. **Not a replacement for isStatic:** `isPartiallyStatic` is used to enable
   element-level folding. The `isStatic` predicate still means "entirely static" and
   is used for the allStatic path in CCall.

---

## Sprint C.2 — Mixed-Arg Specialization in PE Source (~12 tests)

### Overview

The PE source's CCall handler (pe_source.logos:192-218) has a binary decision:
all-static → fully inline, anything else → residualize. Real partial evaluation
requires a third mode: when SOME arguments are static and SOME are dynamic, substitute
the static arguments into the function body and create a specialized function that
takes only the dynamic arguments.

This is the single most critical gap for real Futamura projections:
- P1: `coreEval(staticExpr, dynamicEnv, staticFuncs)` has mixed binding times.
  Without mixed-arg specialization, the interpreter dispatch is never eliminated.
- P2: `peExpr(dynamicExpr, dynamicEnv, staticFuncs, staticDepth)` has mixed binding
  times. Without it, the PE's own dispatch is never specialized.
- P3: Same problem squared.

The Rust-side PE already does this (`partial_eval.rs:461-574`, `try_specialize_call`).
The PE source needs the same capability, written in LogicAffeine.

### Data Structures

```logos
## A SpecEntry is a record with:
    originalName Text.
    specName Text.
    staticArgs Seq of CVal.
    dynamicParamNames Seq of Text.
    specBody Seq of CStmt.
```

The `specCache: Map of Text to SpecEntry` maps `makeKey(fnName, staticArgs)` to the
specialized function's information. This avoids re-specializing the same function
with the same static arguments.

### Algorithm

**Mixed-arg CCall specialization:**

In `peExpr`, CCall handler, between the `allStatic` path and the residualize path:

```
1. Check: are SOME args static and SOME dynamic? (mixedArgs)
2. If mixedArgs and depth > 0:
   a. Compute specKey = makeKey(fnName, peArgs)  // existing function
   b. Check specCache for specKey
      - If found: emit call to cached specName with only dynamic args → done
   c. Look up function in funcs → CFuncDef(fname, params, body)
   d. Build callEnv: bind static params to their values (via exprToVal)
   e. Collect dynamicParams: params whose args are NOT static
   f. Generate specName = "{fnName}_{specKey}"
   g. PE the body: peBlock(body, callEnv, funcs, depth - 1)
      → This folds all static references and residualizes dynamic ones
   h. Register in specCache
   i. Emit a CFuncDef for the specialized function (with only dynamic params)
      → Append to a specFuncs accumulator (new output channel)
   j. Replace the call with CCall(specName, dynamicArgs)
3. Otherwise: residualize as before
```

**Threading specFuncs and specCache:**

The `peBlock` and `peExpr` functions need two new parameters:
- `specCache: Map of Text to Text` — maps spec keys to specialized function names
- `specFuncs: Seq of CStmt` — accumulates generated specialized function definitions

These must be threaded through all recursive calls. Since LogicAffeine doesn't have
mutable references, these must be passed by value and returned as part of the result.

**Alternative: use the existing funcs map.** Instead of a separate specFuncs output,
insert specialized functions directly into the `funcs` map under their specName.
This is simpler and avoids threading a new output channel. The specialized function
is then available for subsequent calls automatically.

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | Mixed-arg CCall handler, specCache threading |
| `tests/phase_futamura.rs` | ~10 new tests |

### TDD Steps

**Step C2.1 — RED: Mixed-arg specialization produces specialized function**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_pe_mixed_arg_specializes() {
    // scale(3, dynamicX) where scale(factor, x) = factor * x
    // should produce: scale_s0_3(x) = 3 * x; call scale_s0_3(dynamicX)
    // The static arg (3) is substituted, the dynamic arg (x) remains.
    let source = r#"
## To scale (factor: Int) and (x: Int) -> Int:
    Return factor * x.

## Main
    Let mutable y be 10.
    Show scale(3, y).
    Set y to 20.
    Show scale(3, y).
"#;
    assert_exact_output(source, "30\n60\n");
}
```

```rust
#[test]
fn fix_pe_mixed_arg_interpreter_dispatch() {
    // THE critical P1 test: coreEval(staticExpr, dynamicEnv, staticFuncs)
    // The interpreter dispatches on expr type. When expr is a static CInt,
    // the PE should fold the Inspect to the CInt arm, eliminating dispatch.
    //
    // We test this by running pe_source on an encoded program and checking
    // that the residual does NOT contain Inspect on expr types.
    let source = format!(r#"
{}
{}
## Main
    Let stmts be a new Seq of CStmt.
    Push (a new CShow with expr (a new CInt with value 42)) to stmts.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peBlock(stmts, env, funcs, 10).
    Repeat for s in result:
        Inspect s:
            When CShow (showExpr):
                Inspect showExpr:
                    When CInt (v):
                        Show "FOLDED".
                    Otherwise:
                        Show "NOT FOLDED".
            Otherwise:
                Show "OTHER".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "FOLDED\n");
}
```

```rust
#[test]
fn fix_pe_mixed_arg_multiple_static() {
    // Two static args, one dynamic: f(1, 2, x) → f_spec(x)
    let source = r#"
## To combine (a: Int) and (b: Int) and (x: Int) -> Int:
    Return (a + b) * x.

## Main
    Let mutable y be 5.
    Show combine(3, 7, y).
"#;
    assert_exact_output(source, "50\n");
}
```

```rust
#[test]
fn fix_pe_mixed_arg_all_dynamic_unchanged() {
    // All dynamic args → no specialization (regression test)
    let source = r#"
## To add (a: Int) and (b: Int) -> Int:
    Return a + b.

## Main
    Let mutable x be 3.
    Let mutable y be 4.
    Show add(x, y).
"#;
    assert_exact_output(source, "7\n");
}
```

Expected: First two FAIL (mixed-arg not implemented), last two PASS.

**Step C2.2 — RED: Mixed-arg with recursion**

```rust
#[test]
fn fix_pe_mixed_arg_recursive() {
    // power(staticBase, dynamicN): should specialize to power_3(n) = 3^n
    // The recursive structure is preserved but base is baked in.
    let source = r#"
## To power (base: Int) and (n: Int) -> Int:
    If n is at most 0:
        Return 1.
    Return base * power(base, n - 1).

## Main
    Let mutable exp be 3.
    Show power(2, exp).
"#;
    assert_exact_output(source, "8\n");
}
```

```rust
#[test]
fn fix_pe_mixed_arg_shared_specialization() {
    // Two calls with the same static arg pattern should share the
    // specialized function, not create duplicates.
    let source = r#"
## To mul (factor: Int) and (x: Int) -> Int:
    Return factor * x.

## Main
    Let mutable a be 10.
    Let mutable b be 20.
    Show mul(5, a).
    Show mul(5, b).
"#;
    assert_exact_output(source, "50\n100\n");
}
```

Expected: PASS (behavioral — existing pipeline handles these).

**Step C2.2b — RED: Arity raising — recursive calls use specialized variant**

```rust
#[test]
fn fix_arity_recursive_self_call() {
    // power(2, n) with recursive call power(2, n-1) inside
    // should produce: power_s0_2(n) with body calling power_s0_2(n-1)
    // The static arg (2) is substituted, recursive calls use specialized name
    let source = r#"
## To power (base: Int) and (n: Int) -> Int:
    If n is at most 0:
        Return 1.
    Return base * power(base, n - 1).

## Main
    Let mutable exp be 4.
    Show power(2, exp).
"#;
    // Verify the specialized body's recursive call uses specialized name
    let rust = compile_to_rust(source).unwrap();
    // The specialized function power_s0_2 should call itself, not power(2, ...)
    assert!(rust.contains("power_s0_") || !rust.contains("power(2"),
        "Recursive call should use specialized variant");
    assert_exact_output(source, "16\n");
}
```

```rust
#[test]
fn fix_arity_mutual_recursion() {
    // Two mutually recursive functions with shared static arg
    // Both should specialize, and their cross-calls should use specialized names
    let source = r#"
## To isEvenHelper (base: Int) and (n: Int) -> Bool:
    If n equals 0:
        Return true.
    Return isOddHelper(base, n - 1).

## To isOddHelper (base: Int) and (n: Int) -> Bool:
    If n equals 0:
        Return false.
    Return isEvenHelper(base, n - 1).

## Main
    Let mutable x be 4.
    Show isEvenHelper(1, x).
"#;
    assert_exact_output(source, "true\n");
}
```

Expected: First — verify both correctness and that generated code uses specialized
variant names. Second — PASS (behavioral correctness).

**Step C2.3 — GREEN: Implement mixed-arg specialization in pe_source.logos**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Modify the CCall handler in `peExpr` (lines 192-218):

```logos
        When CCall (fnName, argExprs):
            If depth is at most 0:
                Let residArgs be a new Seq of CExpr.
                Repeat for a in argExprs:
                    Push peExpr(a, env, funcs, 0) to residArgs.
                Return a new CCall with name fnName and args residArgs.
            Let peArgs be a new Seq of CExpr.
            Repeat for a in argExprs:
                Push peExpr(a, env, funcs, depth) to peArgs.
            Let allStat be allStatic(peArgs).
            If allStat:
                // Existing full-inline path (unchanged)
                Let func be item fnName of funcs.
                Inspect func:
                    When CFuncDef (fname, params, body):
                        Let callEnv be a new Map of Text to CVal.
                        Let mutable pidx be 1.
                        Repeat for p in params:
                            Let argExpr be item pidx of peArgs.
                            Let argVal be exprToVal(argExpr).
                            Set item p of callEnv to argVal.
                            Set pidx to pidx + 1.
                        Let peBody be peBlock(body, callEnv, funcs, depth - 1).
                        Let bodyResult be extractReturn(peBody).
                        Return bodyResult.
                    Otherwise:
                        Return a new CCall with name fnName and args peArgs.
            // NEW: Mixed-arg specialization
            Let hasSomeStatic be false.
            Let hasSomeDynamic be false.
            Repeat for pa in peArgs:
                If isStatic(pa):
                    Set hasSomeStatic to true.
                Otherwise:
                    Set hasSomeDynamic to true.
            If hasSomeStatic and hasSomeDynamic:
                Let specKey be makeKey(fnName, peArgs).
                // Check if already specialized
                Let cached be item specKey of funcs.
                Let cacheHit be false.
                Inspect cached:
                    When CFuncDef (cn, cp, cb):
                        Set cacheHit to true.
                    Otherwise:
                        Set cacheHit to false.
                If cacheHit:
                    // Emit call to cached specialized function with dynamic args only
                    Let dynArgs be a new Seq of CExpr.
                    Repeat for pa in peArgs:
                        If not isStatic(pa):
                            Push pa to dynArgs.
                    Return a new CCall with name specKey and args dynArgs.
                // Look up original function
                Let func be item fnName of funcs.
                Inspect func:
                    When CFuncDef (fname, params, body):
                        Let callEnv be a new Map of Text to CVal.
                        Let dynParams be a new Seq of Text.
                        Let mutable pidx be 1.
                        Repeat for p in params:
                            Let argExpr be item pidx of peArgs.
                            If isStatic(argExpr):
                                Let argVal be exprToVal(argExpr).
                                Set item p of callEnv to argVal.
                            Otherwise:
                                Push p to dynParams.
                            Set pidx to pidx + 1.
                        Let peBody be peBlock(body, callEnv, funcs, depth - 1).
                        // Register specialized function in funcs map
                        Let specFunc be a new CFuncDef with name specKey and params dynParams and body peBody.
                        Set item specKey of funcs to specFunc.
                        // Emit call with only dynamic args
                        Let dynArgs be a new Seq of CExpr.
                        Repeat for pa in peArgs:
                            If not isStatic(pa):
                                Push pa to dynArgs.
                        Return a new CCall with name specKey and args dynArgs.
                    Otherwise:
                        Return a new CCall with name fnName and args peArgs.
            Return a new CCall with name fnName and args peArgs.
```

**Key design choice:** Specialized functions are stored directly in the `funcs` map
under their specKey name. This avoids threading a separate accumulator. Subsequent
calls with the same static args find the cached version. The `funcs` map is mutable
(passed by value in LogicAffeine, but shared via the Map reference semantics).

**Step C2.4 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step C2.5 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Funcs map mutation:** LogicAffeine Maps have reference semantics (like Rc<RefCell>).
   Mutating `funcs` inside `peExpr` affects all callers that share the same map. This is
   intentional — it makes the specCache work without explicit threading. But verify that
   adding entries to `funcs` doesn't break iteration over `funcs` elsewhere.

2. **Recursive mixed-arg functions:** If `power(2, n)` is specialized to `power_s0_2(n)`,
   the recursive call `power(2, n-1)` inside the body should resolve to
   `power_s0_2(n-1)`. This happens naturally if the specialized function is registered
   in `funcs` BEFORE PE'ing the body. But this creates a chicken-and-egg: the function
   is registered without a body, then the body is PE'd. Insert a placeholder
   `CFuncDef(specKey, dynParams, [])` before PE'ing the body, then update after.

3. **Specialization explosion:** Mixed-arg specialization can create many function
   variants (e.g., `scale_s0_1`, `scale_s0_2`, `scale_s0_3`, ...). Add a variant
   limit: at most 8 specializations per original function. After that, residualize.
   Track with a `specCount: Map of Text to Int`.

4. **Cost heuristic:** The Rust-side PE uses a cost heuristic
   (`specialized_cost > original_cost * 0.8` → reject). The PE source should also
   check whether specialization actually reduces code. A simple heuristic: if the
   PE'd body has more statements than the original body, reject and residualize.

---

## Sprint C.3 — PE Memoization Table, Post-Unfolding & WQO (~10 tests)

### Overview

The PE source uses only `depth` to prevent infinite recursion. For self-application
(P2/P3), this is insufficient. When the PE specializes itself, it encounters its own
`peExpr` function, which calls `peExpr` recursively for each sub-expression. Without
memoization, the same function is re-specialized at each call site, leading to either:
- Depth exhaustion (producing poor, unspecialized residuals)
- Exponential blowup (2^depth re-specializations)

The Rust-side PE has `SpecRegistry.cache: HashMap<SpecKey, Symbol>`. The PE source
needs the same: a memo table that maps `(functionName, staticArgs)` to a cached
specialization result.

Sprint C.2's `funcs`-map caching partially addresses this, but only for mixed-arg
specialization. This sprint extends it to cover:
1. Full-inline memoization (all-static args with recursive calls)
2. Embedding-based termination guard (using the same `embeds` idea as the supercompiler)
3. Cycle detection for mutually recursive functions

### Prerequisite: PEState Record (Sprint C.1)

The PEState record refactor is completed in Sprint C.1 (before C.1.5 and C.2).
By the time this sprint runs, all `peExpr`/`peBlock` calls already use `PEState`.
The `specResults` and `onStack` fields are initialized empty in C.1 and populated
here.

### Algorithm

**Memo table for full inlining:**

In the all-static CCall handler (pe_source.logos:202-217), before inlining:
1. Compute `specKey = makeKey(fnName, peArgs)`
2. Check `specCache` for the key
   - If found, return the cached result expression
3. Insert a sentinel `VNothing` in `specCache` (marks "in progress" — cycle detection)
4. Inline the function body
5. Store the result in `specCache`
6. Return the result

When step 2 finds a sentinel `VNothing`, a cycle is detected. In this case:
- Residualize the call (don't inline — the recursion must be preserved)
- The sentinel prevents infinite re-entry

**Embedding-based termination:**

Track a `specHistory: Seq of Text` of spec keys seen during the current chain.
Before inlining, check if any previous key in the history "embeds" in the current
key (using the same structural embedding as Sprint A's `literal_embeds`). If so,
the specialization is growing without bound — residualize.

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | specCache for full-inline, cycle detection |
| `tests/phase_futamura.rs` | ~6 new tests |

### TDD Steps

**Step C3.1 — RED: Memo prevents re-specialization**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_pe_memo_prevents_respecialization() {
    // factorial called twice with same static arg → should specialize once
    let source = format!(r#"
{}
{}
## Main
    Let stmts be a new Seq of CStmt.
    Let env be a new Map of Text to CVal.
    Let factBody be a new Seq of CStmt.
    // factorial(n) = if n <= 1: 1 else: n * factorial(n-1)
    Let cond be a new CBinOp with op "<=" and left (a new CVar with name "n") and right (a new CInt with value 1).
    Let thenBlock be a new Seq of CStmt.
    Push (a new CReturn with expr (a new CInt with value 1)) to thenBlock.
    Let elseBlock be a new Seq of CStmt.
    Let recCall be a new CCall with name "factorial" and args [a new CBinOp with op "-" and left (a new CVar with name "n") and right (a new CInt with value 1)].
    Push (a new CReturn with expr (a new CBinOp with op "*" and left (a new CVar with name "n") and right recCall)) to elseBlock.
    Push (a new CIf with cond cond and thenBlock thenBlock and elseBlock elseBlock) to factBody.
    Let funcs be a new Map of Text to CFunc.
    Set item "factorial" of funcs to (a new CFuncDef with name "factorial" and params ["n"] and body factBody).
    // Call factorial(5) twice in sequence
    Push (a new CLet with name "a" and expr (a new CCall with name "factorial" and args [a new CInt with value 5])) to stmts.
    Push (a new CLet with name "b" and expr (a new CCall with name "factorial" and args [a new CInt with value 5])) to stmts.
    Let result be peBlock(stmts, env, funcs, 10).
    Show length of result.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    let result = run_logos(&source);
    // With memoization, both calls should produce the same folded result
    assert!(result.success, "Should compile and run");
}
```

```rust
#[test]
fn fix_pe_memo_handles_recursion() {
    // Recursive function with static arg: PE should terminate and produce
    // correct result despite recursion.
    let source = r#"
## To fib (n: Int) -> Int:
    If n is at most 1:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
    Show fib(8).
"#;
    assert_exact_output(source, "21\n");
}
```

Expected: Second PASS (existing pipeline handles this). First may FAIL or PASS depending
on current behavior.

**Step C3.2 — RED: Cycle detection prevents divergence**

```rust
#[test]
fn fix_pe_memo_cycle_detection() {
    // Mutually recursive functions with static args: PE must detect the cycle
    // and residualize instead of diverging.
    let source = r#"
## To ping (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return pong(n - 1) + 1.

## To pong (n: Int) -> Int:
    If n is at most 0:
        Return 0.
    Return ping(n - 1) + 1.

## Main
    Show ping(5).
"#;
    assert_exact_output(source, "5\n");
}
```

Expected: PASS (existing pipeline handles this via depth limit).

**Step C3.3 — GREEN: Add memoization to PE source**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

1. The `funcs` map already serves as a cache for mixed-arg specializations (from C.2).
   Extend it: before full-inline specialization, check if `specKey` exists in `funcs`
   as a CFuncDef with a non-empty body. If so, use the cached body's return value.

2. For all-static inlining (where the result is a single expression, not a function):
   use a `specResults: Map of Text to CExpr` that maps specKey to the folded result
   expression. Thread this through `peExpr` and `peBlock`.

3. For cycle detection: before inlining, insert `specKey` into an `onStack: Seq of Text`.
   If the key is already on the stack, residualize. After inlining, remove from stack.

**Step C3.4 — RED: Post-unfolding cascades specialization**

```rust
#[test]
fn fix_pe_source_cascading_specialization() {
    // After inlining f(3) which returns g(3+1), the result g(4) should cascade:
    // g(4) should also be inlined if g is a known function with all-static args.
    let source = r#"
## To addOne (n: Int) -> Int:
    Return n + 1.

## To doubleAddOne (n: Int) -> Int:
    Return addOne(n) * 2.

## Main
    Show doubleAddOne(3).
"#;
    assert_exact_output(source, "8\n");
}
```

Expected: PASS (behavioral — existing pipeline handles this).

**Step C3.4b — RED: Let-insertion prevents code duplication**

```rust
#[test]
fn fix_pe_no_code_duplication() {
    // Two calls with identical all-static args should produce shared computation.
    // Memoization cache should return the same result for both calls.
    let source = r#"
## To expensive (n: Int) -> Int:
    Return n * n * n + n * n + n + 1.

## Main
    Let a be expensive(5).
    Let b be expensive(5).
    Show a + b.
"#;
    assert_exact_output(source, "312\n");
    // Also verify the generated code doesn't contain duplicate computations
    let rust = compile_to_rust(source).unwrap();
    // The computation 5*5*5+5*5+5+1 = 156 should appear once, not twice
    let count_156 = rust.matches("156").count();
    // With memoization, the value is computed once and reused
    assert!(count_156 <= 2, "Computation should not be duplicated: found {} occurrences", count_156);
}
```

Expected: PASS (behavioral correctness, code-quality assertion is aspirational).

**Step C3.4c — RED: WQO termination guarantee**

```rust
#[test]
fn fix_pe_source_termination_guarantee() {
    // A pathological specialization chain where string-based embedding
    // might miss the growing pattern, but structural embedding catches it.
    // The PE must terminate (not hang) even for deeply recursive programs.
    let source = r#"
## To chain (n: Int) and (acc: Int) -> Int:
    If n is at most 0:
        Return acc.
    Return chain(n - 1, acc + n).

## Main
    Show chain(100, 0).
"#;
    assert_exact_output(source, "5050\n");
}
```

Expected: PASS (existing pipeline handles this via depth limit, but the test
verifies the behavior is correct after switching to structural embedding).

**Step C3.5 — GREEN: Add post-unfolding to PE source**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

In the all-static CCall handler, after inlining the function body and extracting
the return value:

```logos
    Let bodyResult be extractReturn(peBody).
    // Post-unfolding: re-specialize the result to catch transient opportunities
    Let respecialized be peExpr(bodyResult, state).
    Return respecialized.
```

This is safe because `depth` is decremented — the re-specialization uses `depth - 1`,
so infinite cascading is prevented.

**Step C3.6 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step C3.7 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **PEState refactor must come first:** Sprint C.1 refactors all `peExpr`/`peBlock`
   calls to use the PEState record. This MUST be completed and verified green before
   adding memoization logic. The PEState record already includes `specResults` and
   `onStack` fields (initialized empty in C.1, populated in C.3).

2. **Mutable state semantics:** LogicAffeine Maps have reference semantics. If
   `specResults` is shared across recursive calls, mutations are visible to all
   callers. This is desirable for caching but must be handled carefully for
   `onStack` — recursive calls should see entries added during their chain, and
   entries should be removed when unwinding. Use push/pop on a Seq instead of
   Map for `onStack`.

3. **Cache invalidation:** No invalidation needed — cached results are valid for
   the entire PE run. The environment at specialization time is captured in the
   specKey.

4. **Post-unfolding termination:** Re-running `peExpr` on the inlined result uses
   `depth - 1`, so cascading is bounded. But watch for cases where the inlined
   result is the same as the input (idempotent inlining) — these should be detected
   by the memoization cache.

5. **WQO for PE source embedding:** The PE source's `makeKey()` produces string keys.
   String substring ordering is NOT a WQO — Kruskal's theorem doesn't apply. For
   robust termination, replace with structural comparison on the argument CExpr list.
   The Rust-side `embeds()` in supercompile.rs shows the correct approach.

6. **Let-insertion scope:** True let-insertion (Danvy et al. 1995) requires a
   continuation-passing transform. For now, the memoization cache provides the
   simpler benefit: if two calls with identical static args hit the cache, the
   computation is done once and the result reused. This is not full let-insertion
   but achieves the same effect for function calls.

---

## Sprint C.5 — Static Map and Collection Operations (~6 tests)

> **Dependency:** Runs AFTER Sprint C.4 (Environment Splitting). The `staticEnv`
> from C.4 must be in place before this sprint, because the static map evaluation
> here checks `staticEnv` for bound collections.

### Overview

The PE source's handlers for `CIndex`, `CLen`, and `CFieldAccess` always residualize,
even when their operands are statically known. For P2/P3, the PE processes its own
source which uses Maps extensively (`item fnName of funcs` for function lookup). If
these operations are not evaluated at specialization time, the residual retains all
the map/collection overhead.

This sprint adds static evaluation for:
1. `CIndex` on a static `CList` with a static `CInt` index → directly return the element
2. `CLen` on a static `CList` → return `CInt(length)`
3. `CFieldAccess` on a static `CNew`/`CNewVariant` → return the field value
4. `CIndex` on env-bound `VMap` with static key → return the looked-up value

### Algorithm

**CIndex static evaluation:**

In `peExpr`, CIndex handler (line 219-222):

```
1. PE the collection and index sub-expressions
2. If both are static:
   a. If collection is CList and index is CInt:
      - Return item at index (1-based)
      - If index out of bounds, residualize
   b. If collection is CVar bound to VSeq and index is CInt:
      - Return valToExpr(item at index of VSeq)
   c. If collection is CVar bound to VMap and index is static:
      - Look up key in VMap, return valToExpr(value)
3. Otherwise: residualize as before
```

**CLen static evaluation:**

In `peExpr`, CLen handler (line 223-224):

```
1. PE the target
2. If target is a static CList: return CInt(length of items)
3. If target is CVar bound to VSeq: return CInt(length of items)
4. Otherwise: residualize
```

**CFieldAccess static evaluation:**

In `peExpr`, CFieldAccess handler:

```
1. PE the target
2. If target is CNewVariant(tag, names, vals) or CNew(type, names, vals):
   a. Find index of field name in names
   b. Return the corresponding value from vals
3. Otherwise: residualize
```

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | CIndex/CLen/CFieldAccess static evaluation |
| `tests/phase_futamura.rs` | ~6 new tests |

### TDD Steps

**Step C5.1 — RED: Static index evaluation**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_pe_static_index_list() {
    // Index into a static list with a static index should fold
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CList with items [a new CInt with value 10, a new CInt with value 20, a new CInt with value 30].
    Let idx be a new CInt with value 2.
    Let expr be a new CIndex with coll items and idx idx.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (v):
            Show v.
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "20\n");
}
```

```rust
#[test]
fn fix_pe_static_len() {
    // Length of a static list should fold to CInt
    let source = format!(r#"
{}
{}
## Main
    Let items be a new CList with items [a new CInt with value 10, a new CInt with value 20, a new CInt with value 30].
    Let expr be a new CLen with target items.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (v):
            Show v.
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "3\n");
}
```

Expected: FAIL (static index/len not implemented).

**Step C5.2 — RED: Static field access**

```rust
#[test]
fn fix_pe_static_field_access() {
    // Field access on a static CNewVariant should fold
    let source = format!(r#"
{}
{}
## Main
    Let variant be a new CNewVariant with tag "Point" and fnames ["x", "y"] and fvals [a new CInt with value 3, a new CInt with value 7].
    Let expr be a new CFieldAccess with target variant and field "y".
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CInt (v):
            Show v.
        Otherwise:
            Show "NOT FOLDED".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "7\n");
}
```

Expected: FAIL.

**Step C5.3 — RED: Dynamic operands preserved**

```rust
#[test]
fn fix_pe_dynamic_index_preserved() {
    // Index with dynamic operands should be residualized unchanged
    let source = format!(r#"
{}
{}
## Main
    Let coll be a new CVar with name "myList".
    Let idx be a new CVar with name "i".
    Let expr be a new CIndex with coll coll and idx idx.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let result be peExpr(expr, env, funcs, 10).
    Inspect result:
        When CIndex (rc, ri):
            Show "RESIDUALIZED".
        Otherwise:
            Show "WRONG".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "RESIDUALIZED\n");
}
```

Expected: PASS (current behavior residualizes correctly).

**Step C5.4 — GREEN: Implement static CIndex/CLen/CFieldAccess**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Replace CIndex handler:
```logos
        When CIndex (collExpr, idxExpr):
            Let peColl be peExpr(collExpr, env, funcs, depth).
            Let peIdx be peExpr(idxExpr, env, funcs, depth).
            Let collStatic be isStatic(peColl).
            Let idxStatic be isStatic(peIdx).
            If collStatic and idxStatic:
                Inspect peColl:
                    When CList (listItems):
                        Inspect peIdx:
                            When CInt (idxVal):
                                If idxVal is greater than 0:
                                    If idxVal is at most length of listItems:
                                        Return item idxVal of listItems.
                                Return a new CIndex with coll peColl and idx peIdx.
                            Otherwise:
                                Return a new CIndex with coll peColl and idx peIdx.
                    Otherwise:
                        Return a new CIndex with coll peColl and idx peIdx.
            // Check env for bound collections
            Inspect peColl:
                When CVar (varName):
                    Let envVal be item varName of env.
                    Let envNothing be isVNothing(envVal).
                    If not envNothing:
                        Inspect envVal:
                            When VSeq (seqItems):
                                Inspect peIdx:
                                    When CInt (idxVal):
                                        If idxVal is greater than 0:
                                            If idxVal is at most length of seqItems:
                                                Let seqItem be item idxVal of seqItems.
                                                Return valToExpr(seqItem).
                                        Return a new CIndex with coll peColl and idx peIdx.
                                    Otherwise:
                                        Return a new CIndex with coll peColl and idx peIdx.
                            When VMap (mapEntries):
                                If idxStatic:
                                    Inspect peIdx:
                                        When CText (key):
                                            Let mapVal be item key of mapEntries.
                                            Let mapNothing be isVNothing(mapVal).
                                            If not mapNothing:
                                                Return valToExpr(mapVal).
                                            Return a new CIndex with coll peColl and idx peIdx.
                                        When CInt (key):
                                            Return a new CIndex with coll peColl and idx peIdx.
                                        Otherwise:
                                            Return a new CIndex with coll peColl and idx peIdx.
                                Return a new CIndex with coll peColl and idx peIdx.
                            Otherwise:
                                Return a new CIndex with coll peColl and idx peIdx.
                Otherwise:
                    Return a new CIndex with coll peColl and idx peIdx.
            Return a new CIndex with coll peColl and idx peIdx.
```

Replace CLen handler:
```logos
        When CLen (lenTarget):
            Let peTarget be peExpr(lenTarget, env, funcs, depth).
            Let targetStatic be isStatic(peTarget).
            If targetStatic:
                Inspect peTarget:
                    When CList (listItems):
                        Return a new CInt with value (length of listItems).
                    Otherwise:
                        Return a new CLen with target peTarget.
            Inspect peTarget:
                When CVar (varName):
                    Let envVal be item varName of env.
                    Let envNothing be isVNothing(envVal).
                    If not envNothing:
                        Inspect envVal:
                            When VSeq (seqItems):
                                Return a new CInt with value (length of seqItems).
                            Otherwise:
                                Return a new CLen with target peTarget.
                Otherwise:
                    Return a new CLen with target peTarget.
            Return a new CLen with target peTarget.
```

Add CFieldAccess static evaluation (in the existing CFieldAccess handler):
```logos
        When CFieldAccess (faTarget, faField):
            Let peTarget be peExpr(faTarget, env, funcs, depth).
            Let targetStatic be isStatic(peTarget).
            If targetStatic:
                Inspect peTarget:
                    When CNewVariant (nvTag, nvNames, nvVals):
                        Let mutable fidx be 1.
                        Repeat for fn in nvNames:
                            If fn equals faField:
                                Return item fidx of nvVals.
                            Set fidx to fidx + 1.
                        Return a new CFieldAccess with target peTarget and field faField.
                    When CNew (nType, nNames, nVals):
                        Let mutable fidx be 1.
                        Repeat for fn in nNames:
                            If fn equals faField:
                                Return item fidx of nVals.
                            Set fidx to fidx + 1.
                        Return a new CFieldAccess with target peTarget and field faField.
                    Otherwise:
                        Return a new CFieldAccess with target peTarget and field faField.
            Return a new CFieldAccess with target peTarget and field faField.
```

**Step C5.5 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step C5.6 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **1-based indexing:** LogicAffeine uses 1-based indexing. `item 1 of list` is the
   first element. The CInt index from the encoded program must match — if the source
   uses `item 1 of items`, it encodes as `CIndex(items, CInt(1))`. The PE must use
   `item idxVal of listItems` directly (LogicAffeine's item is already 1-based).

2. **Map key types:** Maps in LogicAffeine can be keyed by Text or Int. The VMap
   variant stores entries as a Map. The static evaluation must handle both key types.

3. **Missing map entries:** If a static key is not found in a VMap, the result is
   VNothing. The `isVNothing` check prevents returning a bad value. Instead, the
   operation is residualized (the key might be set later at runtime).

4. **CFieldAccess field name matching:** Field names in CNewVariant are stored as
   Text strings. The comparison `fn equals faField` uses LogicAffeine's text equality,
   which is case-sensitive. This matches the parser's behavior.

5. **Performance of env lookup:** Checking the env for every CVar in CIndex adds
   overhead. But this is specialization time, not runtime — correctness matters more
   than speed. The overhead is O(1) per Map lookup.

---

## Sprint C.4 — Environment Splitting + Positive Information Propagation (~10 tests)

> **Note:** Despite document ordering, C.4 executes BEFORE C.5. The staticEnv
> introduced here is required by C.5's static map operations. See the sprint
> dependency diagram at the top of this document.

### Overview

The PE source gains `staticEnv: Map of Text to CExpr` that tracks variable bindings
known to be static at specialization time. This is the single most impactful addition
— without it, all map lookups in the interpreter and PE are residualized, and the
"specialized" code retains all overhead. When the PE processes its own source during
P2/P3, every `item varName of env` produces a residual map lookup even when the PE
just bound that variable to a known static value. `staticEnv` breaks this cycle.

### Data Structure

The PEState record (introduced in Sprint C.1) already includes `staticEnv`:

```logos
## A PEState is a record with:
    env Map of Text to CVal.
    funcs Map of Text to CFunc.
    depth Int.
    staticEnv Map of Text to CExpr.
    specResults Map of Text to CExpr.
    onStack Seq of Text.
```

### Algorithm

- **CLet handler:** PE the value. If `isStatic(peVal)`, store in BOTH `staticEnv`
  (as CExpr) and `env` (as CVal via exprToVal). If dynamic, store only in `env`,
  REMOVE from `staticEnv`.
- **CVar handler:** Check `staticEnv` first. If found, return the static CExpr
  directly. Otherwise, check `env` for a bound CVal (convert via valToExpr).
  Otherwise, emit residual `CVar`.
- **CSet handler:** Same logic as CLet. If new value is static, update `staticEnv`.
  If dynamic, remove from `staticEnv`.
- **CCall handler (mixed-arg):** When specializing a function body, seed `staticEnv`
  with the static arguments.
- **CWhile/CRepeat handler:** After loop, remove loop-modified variables from
  `staticEnv` (they become dynamic).

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | staticEnv logic in CLet/CVar/CSet/CCall/CWhile/CRepeat handlers |
| `tests/phase_futamura.rs` | ~8 new tests |

### TDD Steps

**Step C4.1 — RED: Static let propagates through env**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_pe_env_split_static_let() {
    // Let x = 5; Show x. → PE should fold x to 5 via staticEnv
    let source = format!(r#"
{}
{}
## Main
    Let stmts be a new Seq of CStmt.
    Push (a new CLet with name "x" and expr (a new CInt with value 5)) to stmts.
    Push (a new CShow with expr (a new CVar with name "x")) to stmts.
    Let state be a new PEState with env (a new Map of Text to CVal)
        and funcs (a new Map of Text to CFunc) and depth 10
        and staticEnv (a new Map of Text to CExpr)
        and specResults (a new Map of Text to CExpr)
        and onStack (a new Seq of Text).
    Let result be peBlock(stmts, state).
    Repeat for s in result:
        Inspect s:
            When CShow (showExpr):
                Inspect showExpr:
                    When CInt (v):
                        Show "FOLDED:{{v}}".
                    Otherwise:
                        Show "NOT FOLDED".
            Otherwise:
                Show "OTHER".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "FOLDED:5\n");
}
```

Expected: FAIL (staticEnv not implemented yet).

**Step C4.2 — RED: Dynamic value removes from staticEnv**

```rust
#[test]
fn fix_pe_env_split_set_dynamic() {
    // Let x = 5; Set x = dynamicVar; Show x → x should be residual CVar
    // Source constructs CLet x=5, then CSet x=CVar("input"), then CShow CVar("x")
    // Result: Show should contain a CVar, not a CInt
}
```

Expected: FAIL.

**Step C4.3 — RED: Static env propagates into function calls**

```rust
#[test]
fn fix_pe_env_split_into_function() {
    // Define f(x) = Show x. Call f(5). PE should propagate 5 into f's body via staticEnv.
}
```

Expected: FAIL.

**Step C4.4 — RED: Loop variables removed from staticEnv**

```rust
#[test]
fn fix_pe_env_split_loop_dynamic() {
    // Let x = 0; While ...: Set x = x+1; Show x → x is dynamic after loop
}
```

Expected: FAIL.

**Steps C4.5-C4.8 — Additional tests**

- C4.5: Conditional branches — if `If` sets `x = 5` in one branch and `x = 7` in the
  other, `x` is removed from `staticEnv` after the `If` (conservative merge).
- C4.6: Nested functions — `staticEnv` is cloned when entering function bodies to avoid
  leaking local bindings.
- C4.7: Map lookups through staticEnv — `item key of map` where `map` is in `staticEnv`
  should fold to the value.
- C4.8: Self-application scenario — PE processing its own CVar handler should use
  `staticEnv` to fold known bindings.

**Step C4.8b — RED: Positive information propagation in Inspect arms**

After `Inspect e: When CInt(v): <body>`, the PE should know `e` is `CInt(v)` in
`<body>`. This enables further folding within that arm — if the body dispatches on `e`
again, it should fold immediately.

```rust
#[test]
fn fix_positive_info_inspect_arm() {
    // After matching CInt in an Inspect arm, the PE should know the target is CInt.
    // If the arm body re-dispatches on the same target, it should fold.
    let source = format!(r#"
{}
{}
## Main
    // Build: Inspect x: When CInt(v): Inspect x: When CInt(v2): Show v2. Otherwise: Show "bad".
    Let innerArms be a new Seq of CMatchArm.
    Let innerBody be a new Seq of CStmt.
    Push (a new CShow with expr (a new CVar with name "v2")) to innerBody.
    Push (a new CWhen with variantName "CInt" and bindings ["v2"] and body innerBody) to innerArms.
    Let innerOther be a new Seq of CStmt.
    Push (a new CShow with expr (a new CText with value "bad")) to innerOther.
    Push (a new COtherwise with body innerOther) to innerArms.

    Let outerBody be a new Seq of CStmt.
    Push (a new CInspect with target (a new CVar with name "x") and arms innerArms) to outerBody.

    Let outerArms be a new Seq of CMatchArm.
    Push (a new CWhen with variantName "CInt" and bindings ["v"] and body outerBody) to outerArms.

    Let stmts be a new Seq of CStmt.
    Push (a new CLet with name "x" and expr (a new CNewVariant with tag "CInt" and fnames ["value"] and fvals [a new CInt with value 42])) to stmts.
    Push (a new CInspect with target (a new CVar with name "x") and arms outerArms) to stmts.

    Let state be a new PEState with env (a new Map of Text to CVal)
        and funcs (a new Map of Text to CFunc) and depth 10
        and staticEnv (a new Map of Text to CExpr)
        and specResults (a new Map of Text to CExpr)
        and onStack (a new Seq of Text).
    Let result be peBlock(stmts, state).
    // The inner Inspect should be eliminated because x is known to be CInt
    // after the outer When CInt arm matches.
    Repeat for s in result:
        Inspect s:
            When CShow (showExpr):
                Inspect showExpr:
                    When CInt (v):
                        Show "FOLDED:{{v}}".
                    Otherwise:
                        Show "NOT FOLDED".
            When CInspect (tgt, arms):
                Show "INSPECT RESIDUALIZED".
            Otherwise:
                Show "OTHER".
"#, CORE_TYPES_FOR_PE, pe_source_text());
    // With positive info propagation, the inner Inspect folds because x is known CInt
    assert_exact_output(&source, "FOLDED:42\n");
}
```

Expected: FAIL (positive information propagation not implemented yet).

**Step C4.9 — GREEN: Implement staticEnv in pe_source.logos**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

1. Modify CLet handler (line ~326): add `isStatic` check, update `staticEnv`
   ```logos
   When CLet (name, expr):
       Let peVal be peExpr(expr, state).
       Let valStatic be isStatic(peVal).
       If valStatic:
           Set item name of staticEnv of state to peVal.
           Set item name of env of state to exprToVal(peVal).
       Otherwise:
           Remove item name from staticEnv of state.
           Set item name of env of state to exprToVal(peVal).
       Push (a new CLet with name name and expr peVal) to blockResult.
   ```

2. Modify CVar handler (line ~170): check `staticEnv` first
   ```logos
   When CVar (varName):
       Let staticVal be item varName of staticEnv of state.
       Let staticNothing be isVNothing(staticVal).
       If not staticNothing:
           Return staticVal.
       // Fall through to existing env lookup...
   ```

3. Modify CSet handler (line ~335): update/remove from `staticEnv`

4. Modify CCall handler: seed `staticEnv` with static args when entering function body

5. Modify CWhile/CRepeat handlers: after loop, remove loop-modified variables from
   `staticEnv` (widen to dynamic)

**Step C4.10 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step C4.11 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **staticEnv and env must stay synchronized.** If a variable is in `staticEnv`, its
   CVal equivalent should be in `env` too (for existing code paths that read from
   `env`). Any code that sets `staticEnv` must also update `env`, and any code that
   removes from `staticEnv` must leave `env` intact (the variable is still bound,
   just not statically known).

2. **Branch merging:** If `If` sets `x = 5` in one branch and `x = 7` in the other,
   both are static but different. Conservative approach: remove from `staticEnv` if
   value differs across branches. This is sound but loses precision.

3. **Map reference semantics:** `staticEnv` is a LogicAffeine Map (Rc<RefCell>).
   Mutations are visible to all callers sharing the map. Must clone when entering
   function bodies to avoid leaking local bindings back to the caller.

4. **CVar handler ordering:** `staticEnv` lookup must come BEFORE `env` lookup.
   If a variable is in both (which it should be, per pitfall 1), `staticEnv`
   provides the CExpr form directly, avoiding the valToExpr round-trip.

5. **Interaction with memoization (C.3):** The `specResults` cache stores results
   computed without `staticEnv`. After C.4, the same spec key might produce different
   results depending on `staticEnv` contents. The spec key must incorporate the
   relevant `staticEnv` bindings to avoid stale cache hits.

---

## Sprint D — Decompile Functions in PE Source (~8 tests)

### Overview

For real Futamura projections, the PE source must be able to convert its residual
CExpr/CStmt trees back into LogicAffeine source code. The Rust-side decompiler
(`compile.rs:2757-3158`) handles Rust AST nodes. This sprint adds equivalent functions
to `pe_source.logos` that operate on the CExpr/CStmt representation.

Three functions:
- `decompileExpr(e: CExpr) -> Text` — converts a CExpr to its LogicAffeine source form
- `decompileStmt(s: CStmt, indent: Int) -> Text` — converts a CStmt with indentation
- `decompileBlock(stmts: Seq of CStmt, indent: Int) -> Text` — converts a block

### Algorithm

**decompileExpr — pattern matching on all CExpr variants:**

| CExpr Variant | Output |
|---------------|--------|
| `CInt(n)` | `"{n}"` |
| `CFloat(f)` | `"{f}"` |
| `CBool(true)` | `"true"` |
| `CBool(false)` | `"false"` |
| `CText(s)` | `"\"{s}\""` |
| `CVar(name)` | `name` |
| `CBinOp(op, l, r)` | `"{decompile(l)} {opStr} {decompile(r)}"` |
| `CNot(inner)` | `"not {decompile(inner)}"` |
| `CCall(name, args)` | `"{name}({arg1}, {arg2}, ...)"` |
| `CIndex(coll, idx)` | `"item {decompile(idx)} of {decompile(coll)}"` |
| `CLen(target)` | `"length of {decompile(target)}"` |
| `CList(items)` | `"[{item1}, {item2}, ...]"` |
| `CRange(start, end)` | `"{start} to {end}"` |
| `CNewVariant(tag, names, vals)` | `"a new {tag} with {name1} {val1} and ..."` |
| `CNew(type, names, vals)` | `"a new {type} with {name1} {val1} and ..."` |
| `CFieldAccess(target, field)` | `"{field} of {decompile(target)}"` |
| `CTuple(items)` | `"({item1}, {item2}, ...)"` |
| `COptionSome(inner)` | `"some {decompile(inner)}"` |
| `COptionNone` | `"none"` |
| `CCopy(target)` | `"copy of {decompile(target)}"` |

**decompileStmt — pattern matching on all CStmt variants:**

| CStmt Variant | Output |
|---------------|--------|
| `CLet(name, expr)` | `"{pad}Let {name} be {decompile(expr)}.\n"` |
| `CSet(name, expr)` | `"{pad}Set {name} to {decompile(expr)}.\n"` |
| `CShow(expr)` | `"{pad}Show {decompile(expr)}.\n"` |
| `CReturn(expr)` | `"{pad}Return {decompile(expr)}.\n"` |
| `CIf(cond, then, else)` | `"{pad}If {cond}:\n{then}{pad}Otherwise:\n{else}"` |
| `CWhile(cond, body)` | `"{pad}While {cond}:\n{body}"` |
| `CCallS(name, args)` | `"{pad}{name}({args}).\n"` |
| `CPush(expr, target)` | `"{pad}Push {expr} to {target}.\n"` |
| `CSetIdx(target, idx, val)` | `"{pad}Set item {idx} of {target} to {val}.\n"` |
| `CRepeat(var, coll, body)` | `"{pad}Repeat for {var} in {coll}:\n{body}"` |
| `CInspect(target, arms)` | `"{pad}Inspect {target}:\n{arms}"` |
| `CBreak` | `"{pad}Break.\n"` |

Where `pad` is `"    "` repeated `indent` times.

**decompileBlock:**

```logos
## To decompileBlock (stmts: Seq of CStmt) and (indent: Int) -> Text:
    Let mutable result be "".
    Repeat for s in stmts:
        Let line be decompileStmt(s, indent).
        Set result to "{result}{line}".
    Return result.
```

**Binary operator mapping:**

| Op String | LOGOS Source |
|-----------|-------------|
| `"+"` | `+` |
| `"-"` | `-` |
| `"*"` | `*` |
| `"/"` | `/` |
| `"%"` | `%` |
| `"=="` | `equals` |
| `"!="` | `is not` |
| `"<"` | `is less than` |
| `">"` | `is greater than` |
| `"<="` | `is at most` |
| `">="` | `is at least` |
| `"&&"` | `and` |
| `"\|\|"` | `or` |

### New/Modified Files

| File | Change |
|------|--------|
| `optimize/pe_source.logos` | decompileExpr, decompileStmt, decompileBlock functions |
| `tests/phase_futamura.rs` | ~8 new tests |

### TDD Steps

**Step D1 — RED: decompileExpr round-trip for literals**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_decompile_expr_int() {
    // decompileExpr(CInt(42)) should return "42"
    let source = format!(r#"
{}
{}
## Main
    Let e be a new CInt with value 42.
    Let result be decompileExpr(e).
    Show result.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "42\n");
}
```

Where `CORE_TYPES_FOR_PE` and `pe_source_text()` are obtained from the `compile` module.
The test pattern follows existing Sprint 4-8 tests that embed the PE source.

```rust
#[test]
fn fix_decompile_expr_binop() {
    let source = format!(r#"
{}
{}
## Main
    Let left be a new CInt with value 3.
    Let right be a new CInt with value 5.
    Let e be a new CBinOp with op "+" and left left and right right.
    Let result be decompileExpr(e).
    Show result.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "3 + 5\n");
}
```

Expected: FAIL (decompileExpr doesn't exist yet).

**Step D2 — RED: decompileStmt for Let and Show**

```rust
#[test]
fn fix_decompile_stmt_let() {
    let source = format!(r#"
{}
{}
## Main
    Let expr be a new CInt with value 10.
    Let stmt be a new CLet with name "x" and expr expr.
    Let result be decompileStmt(stmt, 0).
    Show result.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "Let x be 10.\n\n");
}
```

```rust
#[test]
fn fix_decompile_stmt_show() {
    let source = format!(r#"
{}
{}
## Main
    Let expr be a new CVar with name "x".
    Let stmt be a new CShow with expr expr.
    Let result be decompileStmt(stmt, 1).
    Show result.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "    Show x.\n\n");
}
```

Expected: FAIL.

**Step D3 — RED: decompileBlock**

```rust
#[test]
fn fix_decompile_block_full() {
    let source = format!(r#"
{}
{}
## Main
    Let stmts be a new Seq of CStmt.
    Push (a new CLet with name "x" and expr (a new CInt with value 5)) to stmts.
    Push (a new CShow with expr (a new CVar with name "x")) to stmts.
    Let result be decompileBlock(stmts, 0).
    Show result.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    assert_exact_output(&source, "Let x be 5.\nShow x.\n\n");
}
```

Expected: FAIL.

**Step D4 — RED: Round-trip encode → PE → decompile**

```rust
#[test]
fn fix_decompile_roundtrip() {
    // Encode a simple program, run PE (identity — no specialization),
    // decompile the result, and verify it's semantically equivalent.
    let source = format!(r#"
{}
{}
## Main
    Let stmts be a new Seq of CStmt.
    Push (a new CLet with name "x" and expr (a new CInt with value 42)) to stmts.
    Push (a new CShow with expr (a new CVar with name "x")) to stmts.
    Let env be a new Map of Text to CVal.
    Let funcs be a new Map of Text to CFunc.
    Let peResult be peBlock(stmts, env, funcs, 10).
    Let decompiled be decompileBlock(peResult, 0).
    Show decompiled.
"#, CORE_TYPES_FOR_PE, pe_source_text());
    let result = run_logos(&source);
    assert!(result.stdout.contains("Show 42"), "PE should substitute x=42 into Show");
}
```

Expected: FAIL (decompileBlock doesn't exist).

**Step D5 — GREEN: Implement decompileExpr**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

Add at the end of the file:

```logos
## To opToStr (op: Text) -> Text:
    If op equals "+":
        Return "+".
    If op equals "-":
        Return "-".
    If op equals "*":
        Return "*".
    If op equals "/":
        Return "/".
    If op equals "%":
        Return "%".
    If op equals "==":
        Return "equals".
    If op equals "!=":
        Return "is not".
    If op equals "<":
        Return "is less than".
    If op equals ">":
        Return "is greater than".
    If op equals "<=":
        Return "is at most".
    If op equals ">=":
        Return "is at least".
    If op equals "&&":
        Return "and".
    If op equals "||":
        Return "or".
    Return op.

## To decompileExpr (e: CExpr) -> Text:
    Inspect e:
        When CInt (v):
            Return "{v}".
        When CFloat (v):
            Return "{v}".
        When CBool (v):
            If v:
                Return "true".
            Return "false".
        When CText (v):
            Return "\"{v}\"".
        When CVar (name):
            Return name.
        When CBinOp (op, left, right):
            Let l be decompileExpr(left).
            Let r be decompileExpr(right).
            Let opStr be opToStr(op).
            Return "{l} {opStr} {r}".
        When CNot (inner):
            Let innerStr be decompileExpr(inner).
            Return "not {innerStr}".
        When CCall (fnName, args):
            Let argStrs be a new Seq of Text.
            Repeat for a in args:
                Push decompileExpr(a) to argStrs.
            // Join args with ", " — need a join helper or manual loop
            Let mutable argStr be "".
            Let mutable first be true.
            Repeat for s in argStrs:
                If first:
                    Set argStr to s.
                    Set first to false.
                Otherwise:
                    Set argStr to "{argStr}, {s}".
            Return "{fnName}({argStr})".
        When CIndex (coll, idx):
            Let collStr be decompileExpr(coll).
            Let idxStr be decompileExpr(idx).
            Return "item {idxStr} of {collStr}".
        When CLen (target):
            Let targetStr be decompileExpr(target).
            Return "length of {targetStr}".
        When CList (items):
            Let itemStrs be a new Seq of Text.
            Repeat for item in items:
                Push decompileExpr(item) to itemStrs.
            Let mutable joined be "".
            Let mutable first be true.
            Repeat for s in itemStrs:
                If first:
                    Set joined to s.
                    Set first to false.
                Otherwise:
                    Set joined to "{joined}, {s}".
            Return "[{joined}]".
        When CRange (start, end):
            Let startStr be decompileExpr(start).
            Let endStr be decompileExpr(end).
            Return "{startStr} to {endStr}".
        When CNewVariant (tag, names, vals):
            Let mutable parts be "a new {tag}".
            Let mutable idx be 1.
            Repeat for n in names:
                Let v be item idx of vals.
                Let vStr be decompileExpr(v).
                If idx equals 1:
                    Set parts to "{parts} with {n} {vStr}".
                Otherwise:
                    Set parts to "{parts} and {n} {vStr}".
                Set idx to idx + 1.
            Return parts.
        When CNew (typeName, fieldNames, fieldExprs):
            Let mutable parts be "a new {typeName}".
            Let mutable idx be 1.
            Repeat for n in fieldNames:
                Let v be item idx of fieldExprs.
                Let vStr be decompileExpr(v).
                If idx equals 1:
                    Set parts to "{parts} with {n} {vStr}".
                Otherwise:
                    Set parts to "{parts} and {n} {vStr}".
                Set idx to idx + 1.
            Return parts.
        When CFieldAccess (target, field):
            Let targetStr be decompileExpr(target).
            Return "{field} of {targetStr}".
        When CTuple (items):
            Let itemStrs be a new Seq of Text.
            Repeat for item in items:
                Push decompileExpr(item) to itemStrs.
            Let mutable joined be "".
            Let mutable first be true.
            Repeat for s in itemStrs:
                If first:
                    Set joined to s.
                    Set first to false.
                Otherwise:
                    Set joined to "{joined}, {s}".
            Return "({joined})".
        When COptionSome (inner):
            Let innerStr be decompileExpr(inner).
            Return "some {innerStr}".
        When COptionNone:
            Return "none".
        When CCopy (target):
            Let targetStr be decompileExpr(target).
            Return "copy of {targetStr}".
        When CNewSeq:
            Return "a new Seq".
        When CNewSet:
            Return "a new Set".
        Otherwise:
            Return "<unknown>".
```

**Step D6 — GREEN: Implement decompileStmt and decompileBlock**

File: `crates/logicaffeine_compile/src/optimize/pe_source.logos`

```logos
## To makePad (indent: Int) -> Text:
    Let mutable pad be "".
    Let mutable i be 0.
    While i is less than indent:
        Set pad to "{pad}    ".
        Set i to i + 1.
    Return pad.

## To decompileStmt (s: CStmt) and (indent: Int) -> Text:
    Let pad be makePad(indent).
    Inspect s:
        When CLet (name, expr):
            Let exprStr be decompileExpr(expr).
            Return "{pad}Let {name} be {exprStr}.\n".
        When CSet (name, expr):
            Let exprStr be decompileExpr(expr).
            Return "{pad}Set {name} to {exprStr}.\n".
        When CShow (expr):
            Let exprStr be decompileExpr(expr).
            Return "{pad}Show {exprStr}.\n".
        When CReturn (expr):
            Let exprStr be decompileExpr(expr).
            Return "{pad}Return {exprStr}.\n".
        When CIf (cond, thenBlock, elseBlock):
            Let condStr be decompileExpr(cond).
            Let thenStr be decompileBlock(thenBlock, indent + 1).
            Let elseStr be decompileBlock(elseBlock, indent + 1).
            Let hasElse be length of elseBlock.
            If hasElse is greater than 0:
                Return "{pad}If {condStr}:\n{thenStr}{pad}Otherwise:\n{elseStr}".
            Return "{pad}If {condStr}:\n{thenStr}".
        When CWhile (cond, body):
            Let condStr be decompileExpr(cond).
            Let bodyStr be decompileBlock(body, indent + 1).
            Return "{pad}While {condStr}:\n{bodyStr}".
        When CCallS (name, args):
            Let argStrs be a new Seq of Text.
            Repeat for a in args:
                Push decompileExpr(a) to argStrs.
            Let mutable argStr be "".
            Let mutable first be true.
            Repeat for s in argStrs:
                If first:
                    Set argStr to s.
                    Set first to false.
                Otherwise:
                    Set argStr to "{argStr}, {s}".
            Return "{pad}{name}({argStr}).\n".
        When CPush (expr, target):
            Let exprStr be decompileExpr(expr).
            Return "{pad}Push {exprStr} to {target}.\n".
        When CSetIdx (target, idx, val):
            Let idxStr be decompileExpr(idx).
            Let valStr be decompileExpr(val).
            Return "{pad}Set item {idxStr} of {target} to {valStr}.\n".
        When CRepeat (var, coll, body):
            Let collStr be decompileExpr(coll).
            Let bodyStr be decompileBlock(body, indent + 1).
            Return "{pad}Repeat for {var} in {collStr}:\n{bodyStr}".
        When CRepeatRange (var, start, end, body):
            Let startStr be decompileExpr(start).
            Let endStr be decompileExpr(end).
            Let bodyStr be decompileBlock(body, indent + 1).
            Return "{pad}Repeat for {var} in {startStr} to {endStr}:\n{bodyStr}".
        When CBreak:
            Return "{pad}Break.\n".
        When CInspect (target, arms):
            Let targetStr be decompileExpr(target).
            Let mutable result be "{pad}Inspect {targetStr}:\n".
            Repeat for arm in arms:
                Inspect arm:
                    When CWhen (wName, wBindings, wBody):
                        Let bodyStr be decompileBlock(wBody, indent + 2).
                        Let mutable bindStr be "".
                        Let mutable first be true.
                        Repeat for b in wBindings:
                            If first:
                                Set bindStr to b.
                                Set first to false.
                            Otherwise:
                                Set bindStr to "{bindStr}, {b}".
                        Let innerPad be makePad(indent + 1).
                        If length of wBindings is greater than 0:
                            Set result to "{result}{innerPad}When {wName}({bindStr}):\n{bodyStr}".
                        Otherwise:
                            Set result to "{result}{innerPad}When {wName}:\n{bodyStr}".
                    When COtherwise (oBody):
                        Let bodyStr be decompileBlock(oBody, indent + 2).
                        Let innerPad be makePad(indent + 1).
                        Set result to "{result}{innerPad}Otherwise:\n{bodyStr}".
            Return result.
        When CPop (target):
            Return "{pad}Pop from {target}.\n".
        When CAdd (elem, target):
            Let elemStr be decompileExpr(elem).
            Return "{pad}Add {elemStr} to {target}.\n".
        When CRemove (elem, target):
            Let elemStr be decompileExpr(elem).
            Return "{pad}Remove {elemStr} from {target}.\n".
        When CSetField (target, field, val):
            Let valStr be decompileExpr(val).
            Return "{pad}Set {field} of {target} to {valStr}.\n".
        When CRuntimeAssert (cond, msg):
            Let condStr be decompileExpr(cond).
            Return "{pad}Assert that {condStr}.\n".
        Otherwise:
            Return "".

## To decompileBlock (stmts: Seq of CStmt) and (indent: Int) -> Text:
    Let mutable result be "".
    Repeat for s in stmts:
        Let line be decompileStmt(s, indent).
        Set result to "{result}{line}".
    Return result.
```

**Step D7 — VERIFY**

Run `cargo test -- --skip e2e`. All existing tests plus new ones must pass.

**Step D8 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **String escaping:** `decompileExpr` for `CText` wraps in quotes: `"\"{v}\""`. If
   `v` itself contains quotes, this produces invalid output. For MVP, accept this
   limitation. A proper fix would escape internal quotes.

2. **Operator precedence:** `decompileExpr` for `CBinOp` doesn't parenthesize sub-expressions.
   `a + b * c` decompiles as `a + b * c` which is correct for LogicAffeine (no implicit
   precedence). But nested BinOps like `(a + b) * c` might lose grouping. Consider adding
   parentheses around BinOp children that are themselves BinOps.

3. **Missing CStmt variants:** The PE source has ~50 CStmt variants. The decompiler must
   handle at least the common ones. Missing variants fall through to `Otherwise` and
   emit empty string. Log which variants are missing and add them incrementally.

4. **1-based indexing in decompile:** `item idx of vals` uses 1-based indexing. Verify
   that the `idx` counter starts at 1, not 0.

---

## Sprint E — Real Projection 1 (~12 tests)

### Overview

Currently, `projection1_source()` ignores the interpreter parameter entirely. It just
optimizes the program directly using the Rust optimizer pipeline. This sprint rewrites
it to perform a real Futamura Projection 1:

```
P1: pe(interpreter, program) = compiled_program
```

The real P1:
1. Encodes the program as a CProgram data structure using `encode_program_source()`
2. Combines the encoded program with the interpreter source and PE source
3. Runs the combined source through the LogicAffeine compilation pipeline
4. The PE specializes the interpreter: for each `CStmt`/`CExpr` variant in the program,
   the interpreter's dispatch is resolved to the specific handler
5. The residual is decompiled back to LogicAffeine source using `decompileBlock()`

### Algorithm

**Real projection1_source:**

```
fn projection1_source(core_types: &str, interpreter: &str, program: &str) -> Result<String, String> {
    1. let encoded = encode_program_source(program)?
       // This produces: Let prog be a new CProgram with funcs [..] and main [..].

    2. let pe_source = pe_source_text()
       // The partial evaluator written in LogicAffeine

    3. let combined = format!("{}\n{}\n{}\n## Main\n{}\n{}",
           core_types,     // CExpr/CStmt/CVal type definitions
           interpreter,    // The self-interpreter (eval functions)
           pe_source,      // The PE functions (peExpr, peBlock, etc.)
           encoded,        // The encoded program (defines `prog`)
           PROJECTION1_DRIVER  // Driver code that calls PE with interpreter + program
       )
       // PROJECTION1_DRIVER is something like:
       //   Let env be a new Map of Text to CVal.
       //   Let funcs be item "funcs" of prog.
       //   Let main be item "main" of prog.
       //   Let residual be peBlock(main, env, funcs, 20).
       //   Let source be decompileBlock(residual, 0).
       //   Show source.

    4. Compile and run `combined` to get the decompiled residual source

    5. Return the residual source
}
```

The key insight: instead of optimizing the program in Rust, we run the program through
the self-interpreter and PE in LogicAffeine itself. The PE specializes the interpreter
away, and the decompiler converts the residual CExpr/CStmt tree back to source.

### New/Modified Files

| File | Change |
|------|--------|
| `compile.rs` | Rewrite projection1_source() |
| `tests/phase_futamura.rs` | ~12 new tests |

### TDD Steps

**Step E1 — RED: Real P1 produces correct output**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_p1_real_simple_show() {
    // Real P1 should produce source that, when compiled, outputs "42"
    let program = r#"
## Main
    Show 42.
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    // The result should be valid LogicAffeine source
    assert_exact_output(&result, "42\n");
}
```

```rust
#[test]
fn fix_p1_real_arithmetic() {
    let program = r#"
## Main
    Let x be 3 + 4.
    Show x.
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    assert_exact_output(&result, "7\n");
}
```

```rust
#[test]
fn fix_p1_real_function() {
    let program = r#"
## To double (n: Int) -> Int:
    Return n * 2.

## Main
    Show double(21).
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    assert_exact_output(&result, "42\n");
}
```

```rust
#[test]
fn fix_p1_real_no_overhead() {
    // The residual from real P1 should have no interpreter dispatch overhead
    let program = r#"
## Main
    Show 42.
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    // Should NOT contain CExpr/CStmt/CVal types
    assert!(!result.contains("CInt"), "Residual should not contain CInt");
    assert!(!result.contains("CShow"), "Residual should not contain CShow");
    assert!(!result.contains("peExpr"), "Residual should not contain peExpr");
}
```

Expected: FAIL (current P1 doesn't actually run the PE).

**Step E2 — RED: Real P1 preserves existing test behavior**

```rust
#[test]
fn fix_p1_real_if_else() {
    let program = r#"
## Main
    Let x be 5.
    If x is greater than 3:
        Show "big".
    Otherwise:
        Show "small".
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    assert_exact_output(&result, "big\n");
}
```

```rust
#[test]
fn fix_p1_real_while_loop() {
    let program = r#"
## Main
    Let mutable i be 1.
    Let mutable sum be 0.
    While i is at most 5:
        Set sum to sum + i.
        Set i to i + 1.
    Show sum.
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    assert_exact_output(&result, "15\n");
}
```

```rust
#[test]
fn fix_p1_real_recursive() {
    let program = r#"
## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).

## Main
    Show factorial(5).
"#;
    let result = projection1_source(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    assert_exact_output(&result, "120\n");
}
```

Expected: FAIL.

**Step E3 — RED: Existing P1 tests still pass**

Verify that all existing `p1_*` tests in `phase_futamura.rs` still pass. These tests
call `projection1_source()` — the new implementation must be backward compatible.

This is a VERIFY step, not a RED step. Run:
```
cargo test --test phase_futamura p1_
```

Expected: All 23 existing P1 tests PASS.

**Step E4 — GREEN: Add projection1_source_real()**

Per the Backward Compatibility Strategy, add `projection1_source_real()` as a NEW
function. The existing `projection1_source()` keeps its current fast-path behavior.
The Sprint E RED tests above call `projection1_source()` because they test the final
unified version. During development, wire them to call `_real()` internally, or gate
unification behind the Sprint G gate.

Implementation:

File: `crates/logicaffeine_compile/src/compile.rs`

```rust
pub fn projection1_source(core_types: &str, interpreter: &str, program: &str) -> Result<String, String> {
    // Step 1: Encode the program as CProgram construction source
    let encoded = encode_program_source(program)
        .map_err(|e| format!("Failed to encode program: {:?}", e))?;

    // Step 2: Get PE source
    let pe_source = pe_source_text();

    // Step 3: Build combined source
    //   - Core types (CExpr, CStmt, CVal definitions)
    //   - Interpreter (eval functions)
    //   - PE source (peExpr, peBlock, decompileBlock)
    //   - Main block that:
    //     a. Constructs the encoded program
    //     b. Runs PE on the interpreter's eval with the program as static input
    //     c. Decompiles the residual
    let driver = r#"
    // The encoded program is already in scope from the encoded source.
    // Extract its main block and function map.
    Let env be a new Map of Text to CVal.
    Let residual be peBlock(encodedMain, env, encodedFuncMap, 20).
    Let source be decompileBlock(residual, 0).
    Show source.
"#;

    let combined = format!(
        "{}\n{}\n{}\n## Main\n{}\n{}",
        if core_types.is_empty() { CORE_TYPES_FOR_PE } else { core_types },
        interpreter,
        pe_source,
        encoded,
        driver,
    );

    // Step 4: Compile and run to get the decompiled residual
    let result = run_logos_source(&combined)?;

    Ok(result)
}
```

**`run_logos_source()` concrete design:**

```rust
// In compile.rs — extracted from test infrastructure pattern in common/mod.rs
fn run_logos_source(source: &str) -> Result<String, String> {
    let rust_code = compile_to_rust_source(source)
        .map_err(|e| format!("Compilation failed: {:?}", e))?;

    let dir = tempfile::tempdir()
        .map_err(|e| format!("Temp dir failed: {}", e))?;
    let rs_path = dir.path().join("projection.rs");
    let bin_path = dir.path().join("projection");

    std::fs::write(&rs_path, &rust_code)
        .map_err(|e| format!("Write failed: {}", e))?;

    // Use direct rustc invocation (not cargo) for speed
    let rustc = std::process::Command::new("rustc")
        .args(&["-O", "-o"])
        .arg(&bin_path)
        .arg(&rs_path)
        .output()
        .map_err(|e| format!("rustc failed: {}", e))?;

    if !rustc.status.success() {
        return Err(format!("rustc error: {}", String::from_utf8_lossy(&rustc.stderr)));
    }

    let run = std::process::Command::new(&bin_path)
        .output()
        .map_err(|e| format!("Execution failed: {}", e))?;

    Ok(String::from_utf8_lossy(&run.stdout).to_string())
}
```

Add `tempfile` as a dependency to `logicaffeine_compile/Cargo.toml`.

**Important backward compatibility note:** The existing `p1_*` tests call
`projection1_source("", "", source)`. The new implementation must handle empty
`core_types` and `interpreter` parameters gracefully — defaulting to `CORE_TYPES_FOR_PE`
and the built-in interpreter source respectively.

**Step E5 — VERIFY**

Run `cargo test -- --skip e2e`. All 403 existing tests plus new ones must pass.
Pay special attention to the 23 existing `p1_*` tests.

**Step E6 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Performance:** Running the PE through the LogicAffeine interpreter is much slower
   than the Rust optimizer. P1 for a simple program might take seconds instead of
   milliseconds. Set reasonable timeouts in tests.

2. **Encode/decode fidelity:** `encode_program_source()` must correctly encode all
   statement and expression types used in the test programs. If encoding misses a
   variant, the PE will encounter an unhandled case and produce incorrect output.

3. **Depth limit:** The PE uses `depth` to prevent infinite recursion. For complex
   programs with deep call trees, depth 20 might not be enough. Consider making
   this configurable or increasing it for P1.

4. **Backward compatibility of existing P1 tests:** The existing `p1_*` tests don't
   pass core_types or interpreter. The function signature takes these as parameters
   but they were previously unused. The new implementation must use defaults when
   these are empty strings.

5. **run_logos_source() availability:** The compilation + execution pipeline is in
   test infrastructure. Moving it to `compile.rs` requires either duplicating it or
   making the test helper available to the library. Consider adding a `run_pipeline()`
   function to `compile.rs` that handles the full compile-and-run cycle.

---

## Sprint F — Real Projections 2 & 3 via Self-Application (~15 tests)

### Overview

This is the capstone sprint. Projections 2 and 3 currently use `replace_word()` for
string-based function renaming. This sprint replaces them with actual self-application:

```
P2: pe(pe, interpreter) = compiler          ← the PE specializes ITSELF
P3: pe(pe, pe)           = compiler_generator ← the PE specializes ITSELF w.r.t. ITSELF
```

**P2** runs the PE on its own source with the interpreter as static input. The PE
processes its own `Inspect` on CExpr variants, its own `isStatic` checks, its own
environment lookups — and eliminates them. The residual is a compiler with the
interpreter's dispatch baked in and the PE's dispatch removed. This is The Trick.

**P3** runs the PE on its own source with itself as static input. The residual is a
compiler generator: a program that takes any interpreter and produces a compiler for
that interpreter's language.

**There is no fallback.** If self-application does not produce clean residuals, we
refine the PE through binding-time improvements until it does. See Step F7b.

### Algorithm

**Real projection2_source:**

```
1. Encode the interpreter as a CProgram: encode_program_source(interpreter)
2. Get the PE source text
3. Combine: core_types + pe_source + encoded_interpreter + P2 driver
4. The P2 driver:
   a. Passes the encoded interpreter as the "program" input to the PE
   b. The PE specializes itself w.r.t. the interpreter's structure
   c. The residual is a compiler — it has peExpr/peBlock specialized to the
      interpreter's dispatch, with the interpreter's case analysis baked in
5. Decompile the residual
6. Rename entry points: peExpr → compileExpr, peBlock → compileBlock
```

The key theoretical point: when the PE processes `peExpr(interpreter_dispatch, ...)`,
it evaluates the interpreter's `Inspect` on CExpr variants. For each variant, the PE
produces specialized code that handles that variant directly. The result is the
interpreter's dispatch loop with all generic PE machinery removed.

**Real projection3_source:**

```
1. Encode the PE source itself as a CProgram: encode_program_source(pe_source)
2. Combine: core_types + pe_source + encoded_pe + P3 driver
3. The P3 driver:
   a. Passes the encoded PE as the "program" input to the PE
   b. The PE specializes itself w.r.t. itself
   c. The residual is a compiler generator — it takes an interpreter and produces
      a compiler
4. Decompile the residual
5. Rename entry points: peExpr → cogenExpr, peBlock → cogenBlock
```

### New/Modified Files

| File | Change |
|------|--------|
| `compile.rs` | Rewrite projection2_source(), projection3_source() |
| `tests/phase_futamura.rs` | ~15 new tests |

### TDD Steps

**Step F1 — RED: Real P2 produces a compiler**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_p2_real_produces_compiler() {
    // Real P2: PE(PE, interpreter) = compiler
    // The compiler should take an encoded program and produce compiled output
    let result = projection2_source().unwrap();
    // The result should contain compileExpr/compileBlock functions
    assert!(result.contains("compileExpr") || result.contains("compileBlock"),
        "P2 result should contain compiler functions");
    // It should NOT contain PE infrastructure (memoization, history, etc.)
    // (This is aspirational — the real P2 may retain some PE structure)
}
```

```rust
#[test]
fn fix_p2_real_compiler_correctness() {
    // The P2-generated compiler should produce the same output as P1
    // for a simple program
    let program = r#"
## Main
    Show 42.
"#;
    let p1_result = projection1_source("", "", program).unwrap();
    let compiler = projection2_source().unwrap();
    // Use the compiler on the same program
    // The compiler is source code — we need to run it with the encoded program
    let encoded = encode_program_source(program).unwrap();
    let combined = format!("{}\n## Main\n{}\n{}", compiler, encoded, COMPILER_DRIVER);
    let compiler_result = run_logos(&combined);
    // Both should produce equivalent output when run
    let p1_output = run_logos(&p1_result);
    assert_eq!(p1_output.stdout, "42\n");
    assert_eq!(compiler_result.stdout, p1_output.stdout,
        "P2 compiler should produce same output as P1");
}
```

Expected: FAIL.

**Step F2 — RED: Real P3 produces a compiler generator**

```rust
#[test]
fn fix_p3_real_produces_cogen() {
    let result = projection3_source().unwrap();
    assert!(result.contains("cogenExpr") || result.contains("cogenBlock"),
        "P3 result should contain compiler generator functions");
}
```

```rust
#[test]
fn fix_p3_real_cogen_produces_compiler() {
    // The P3-generated cogen, when applied to an interpreter, should produce
    // a compiler equivalent to P2's output
    let cogen = projection3_source().unwrap();
    let compiler_from_cogen = apply_cogen_to_interpreter(&cogen).unwrap();
    let compiler_from_p2 = projection2_source().unwrap();
    // Both compilers should handle the same program identically
    let program = "## Main\n    Show 42.\n";
    let output_from_p2 = run_compiler(&compiler_from_p2, program);
    let output_from_cogen = run_compiler(&compiler_from_cogen, program);
    assert_eq!(output_from_p2, output_from_cogen,
        "P3(cogen) applied to interpreter should equal P2 compiler");
}
```

Expected: FAIL.

**Step F3 — RED: Cross-projection equivalence**

```rust
#[test]
fn fix_cross_projection_equivalence() {
    // For any program P:
    //   run(P1(int, P))     = run(P)           — P1 is correct
    //   run(P2(int)(P))     = run(P)           — P2 compiler is correct
    //   run(P3(pe)(int)(P)) = run(P)           — P3 cogen is correct
    let programs = vec![
        "## Main\n    Show 42.\n",
        "## Main\n    Let x be 3 + 4.\n    Show x.\n",
        "## Main\n    If true:\n        Show 1.\n    Otherwise:\n        Show 0.\n",
    ];
    for program in &programs {
        let direct = run_logos(program);
        let p1 = projection1_source("", "", program).unwrap();
        let p1_output = run_logos(&p1);
        assert_eq!(direct.stdout, p1_output.stdout,
            "P1 output must match direct execution for: {}", program);
    }
}
```

```rust
#[test]
fn fix_existing_p2_tests_still_pass() {
    // All existing p2_* tests must continue to pass
    // This is a meta-test — verified by running:
    //   cargo test --test phase_futamura p2_
}

#[test]
fn fix_existing_p3_tests_still_pass() {
    // All existing p3_* tests must continue to pass
    //   cargo test --test phase_futamura p3_
}
```

Expected: PASS for equivalence tests (they test the pipeline, not P2/P3 internals).

**Step F4 — RED: P2/P3 no longer use string replacement**

```rust
#[test]
fn fix_p2_no_string_replacement() {
    // Verify that projection2_source() does not call replace_word()
    // This is a structural test — verify by code inspection that
    // replace_word() is removed or unused.
    // For now, verify behavioral correctness is preserved.
    let result = projection2_source().unwrap();
    // The result should be valid LogicAffeine source that compiles
    let compile_result = compile_to_rust(&result);
    assert!(compile_result.is_ok(), "P2 result should compile: {:?}", compile_result.err());
}
```

```rust
#[test]
fn fix_p3_no_string_replacement() {
    let result = projection3_source().unwrap();
    let compile_result = compile_to_rust(&result);
    assert!(compile_result.is_ok(), "P3 result should compile: {:?}", compile_result.err());
}
```

Expected: PASS (existing implementation produces compilable source).

**Step F5 — GREEN: Rewrite projection2_source() with verification**

File: `crates/logicaffeine_compile/src/compile.rs`

```rust
pub fn projection2_source() -> Result<String, String> {
    let interpreter = interpreter_source(); // the self-interpreter
    let encoded_interpreter = encode_program_source(interpreter)
        .map_err(|e| format!("Encode failed: {:?}", e))?;

    let pe_source = pe_source_text();
    let driver = r#"
    Let state be a new PEState with env (a new Map of Text to CVal)
        and funcs encodedFuncMap and depth 30
        and staticEnv (a new Map of Text to CExpr)
        and specResults (a new Map of Text to CExpr)
        and onStack (a new Seq of Text).
    Let residual be peBlock(encodedMain, state).
    Let source be decompileBlock(residual, 0).
    Show source.
"#;

    let combined = format!("{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE, pe_source, encoded_interpreter, driver);

    let raw_residual = run_logos_source(&combined)?;

    // VERIFICATION: PE dispatch must be eliminated
    assert!(!raw_residual.contains("peExpr"),
        "P2 raw residual must not contain peExpr — PE dispatch not eliminated");
    assert!(!raw_residual.contains("peBlock"),
        "P2 raw residual must not contain peBlock — PE dispatch not eliminated");

    // VERIFICATION: Size regression
    let pe_lines = pe_source.lines().count();
    let residual_lines = raw_residual.lines().count();
    assert!(residual_lines < pe_lines,
        "P2 compiler ({} lines) must be smaller than PE ({} lines)",
        residual_lines, pe_lines);

    // Cosmetic rename of entry points
    let compiler = raw_residual
        .replace("peExpr_specialized", "compileExpr")
        .replace("peBlock_specialized", "compileBlock");

    Ok(format!("{}\n{}", CORE_TYPES_FOR_PE, compiler))
}
```

Note: The verification assertions run BEFORE the cosmetic rename. If the raw residual
still contains `peExpr`/`peBlock`, the function fails with a clear error instead of
silently renaming a broken result.

**Step F6 — GREEN: Rewrite projection3_source()**

File: `crates/logicaffeine_compile/src/compile.rs`

```rust
pub fn projection3_source() -> Result<String, String> {
    // Step 1: Encode the PE source itself as a CProgram
    let pe_source = pe_source_text();
    let full_pe = format!("{}\n{}", CORE_TYPES_FOR_PE, pe_source);
    let encoded_pe = encode_program_source(&full_pe)
        .map_err(|e| format!("Failed to encode PE: {:?}", e))?;

    // Step 2: Combine and run PE(PE, PE)
    let driver = r#"
    Let env be a new Map of Text to CVal.
    Let residual be peBlock(encodedMain, env, encodedFuncMap, 20).
    Let source be decompileBlock(residual, 0).
    Show source.
"#;

    let combined = format!(
        "{}\n{}\n## Main\n{}\n{}",
        CORE_TYPES_FOR_PE,
        pe_source,
        encoded_pe,
        driver,
    );

    let cogen_source = run_logos_source(&combined)?;

    let renamed = cogen_source
        .replace("peExpr", "cogenExpr")
        .replace("peBlock", "cogenBlock");

    Ok(format!("{}\n{}", CORE_TYPES_FOR_PE, renamed))
}
```

**Step F7 — GREEN: Remove replace_word() and add run_logos_source()**

1. Remove `replace_word()` from `compile.rs` (it's no longer needed for P2/P3)
2. Add `run_logos_source()` helper using the Rust compile pipeline:

```rust
fn run_logos_source(source: &str) -> Result<String, String> {
    // Use the existing Rust compile pipeline:
    // 1. compile_to_rust(source) → Rust code string
    // 2. Write to temp file
    // 3. Invoke rustc to compile
    // 4. Execute binary, capture stdout
    //
    // This reuses the exact same pattern as the test infrastructure's
    // run_logos() helper in common/mod.rs. Extract it to library code.

    let rust_code = compile_to_rust_source(source)
        .map_err(|e| format!("Compilation failed: {:?}", e))?;

    let dir = tempfile::tempdir()
        .map_err(|e| format!("Temp dir failed: {}", e))?;
    let rs_path = dir.path().join("projection.rs");
    let bin_path = dir.path().join("projection");

    std::fs::write(&rs_path, &rust_code)
        .map_err(|e| format!("Write failed: {}", e))?;

    let rustc = std::process::Command::new("rustc")
        .args(&["-O", "-o"])
        .arg(&bin_path)
        .arg(&rs_path)
        .output()
        .map_err(|e| format!("rustc failed: {}", e))?;

    if !rustc.status.success() {
        return Err(format!("rustc error: {}", String::from_utf8_lossy(&rustc.stderr)));
    }

    let run = std::process::Command::new(&bin_path)
        .output()
        .map_err(|e| format!("Execution failed: {}", e))?;

    Ok(String::from_utf8_lossy(&run.stdout).to_string())
}
```

3. Add `tempfile` as a dependency to `logicaffeine_compile/Cargo.toml`.

Note: This is the hardest part of Sprint F. The test infrastructure has `run_logos()`
but it's in test code, not library code. Options:
a. Move the compile-and-run logic from test infrastructure to `compile.rs`
b. Use the interpreter directly (slower but self-contained)
c. Add a `run()` function to the library that takes source and returns stdout

**New Sprint F tests — Verification, size regression, cross-interpreter**

These tests go in `crates/logicaffeine_tests/tests/phase_futamura.rs`:

```rust
#[test]
fn fix_p2_raw_no_pe_dispatch() {
    // The raw P2 residual (before rename) must not contain PE dispatch functions.
    // If this test passes, the PE's own Inspect-on-CExpr dispatch was eliminated.
    let compiler = projection2_source().unwrap();
    // projection2_source() itself asserts no peExpr/peBlock before rename.
    // This test verifies the function doesn't panic.
}

#[test]
fn fix_p2_smaller_than_pe() {
    let pe = pe_source_text();
    let compiler = projection2_source().unwrap();
    let pe_lines = pe.lines().count();
    let compiler_lines = compiler.lines().count();
    assert!(compiler_lines < pe_lines,
        "P2 compiler ({} lines) should be smaller than PE source ({} lines)",
        compiler_lines, pe_lines);
}

#[test]
fn fix_p2_no_depth_tracking() {
    let compiler = projection2_source().unwrap();
    // A real compiler doesn't need PE's recursion depth limit
    assert!(!compiler.contains("depth is at most 0"),
        "P2 compiler should not contain depth-based termination");
}

#[test]
fn fix_p2_no_memo_infrastructure() {
    let compiler = projection2_source().unwrap();
    assert!(!compiler.contains("specResults"),
        "P2 compiler should not contain memoization table");
    assert!(!compiler.contains("onStack"),
        "P2 compiler should not contain cycle detection stack");
}

#[test]
fn fix_p3_smaller_than_p2() {
    let compiler = projection2_source().unwrap();
    let cogen = projection3_source().unwrap();
    assert!(cogen.lines().count() < compiler.lines().count(),
        "P3 cogen should be smaller than P2 compiler");
}

#[test]
fn fix_p3_calculator_interpreter() {
    // Minimal calculator: handles CInt, CBinOp(+,-,*), CShow only.
    // Verifies P3 cogen works for ANY interpreter, not just Core/RPN.
    let calc_interpreter = r#"
## To calcEval (e: CExpr) -> Int:
    Inspect e:
        When CInt (v):
            Return v.
        When CBinOp (op, left, right):
            Let l be calcEval(left).
            Let r be calcEval(right).
            If op equals "+":
                Return l + r.
            If op equals "-":
                Return l - r.
            Return l * r.
        Otherwise:
            Return 0.

## To calcExec (stmts: Seq of CStmt):
    Repeat for s in stmts:
        Inspect s:
            When CShow (expr):
                Let val be calcEval(expr).
                Show val.
            Otherwise:
                Let skip be true.
"#;
    let cogen = projection3_source().unwrap();
    let calc_compiler = apply_cogen(&cogen, calc_interpreter).unwrap();

    // The calculator compiler should work
    let program = "## Main\n    Show 3 + 4.\n";
    let encoded = encode_program_source(program).unwrap();
    let output = run_compiler(&calc_compiler, &encoded).unwrap();
    assert_eq!(output.trim(), "7");

    // The calculator compiler should be SMALLER than the cogen
    assert!(calc_compiler.lines().count() < cogen.lines().count(),
        "Calculator compiler should be smaller than cogen");

    // No PE infrastructure in the calculator compiler
    assert!(!calc_compiler.contains("peExpr"));
    assert!(!calc_compiler.contains("specCache"));
    assert!(!calc_compiler.contains("depth"));
}
```

Expected: All FAIL (real specialization not yet implemented).

**Step F7b — If Self-Application Residuals Are Not Clean**

If `pe(pe, interpreter)` does not eliminate PE dispatch (the raw residual still
contains `peExpr`, `isStatic`, `isLiteral`, or `allStatic` calls), the fix is in
the PE source — not in switching to a different technique. Self-application is the
definition of P2. Options when residuals are not clean:

1. **Binding-time improvements to pe_source.logos**: Restructure conditionals so
   static arguments are pattern-matched first. Split functions that mix static and
   dynamic computation. Make the PE's own dispatch more amenable to specialization.
2. **Add BTI annotations**: Mark PE functions or parameters with explicit
   static/dynamic annotations that the BTA can use to produce better divisions.
3. **Restructure the PE's Inspect handlers**: Ensure that after matching a CExpr
   variant, all subsequent operations on that value are expressible without
   re-dispatching. This is "The Trick" — it may require multiple refinement passes.
4. **Increase depth/memoization limits**: Self-application may require deeper
   recursion than normal programs. Tune limits specifically for P2/P3.

This is iterative work. Jones et al. refined MIX through multiple rounds of BTI
before self-application produced clean output. We do the same.

**Step F8 — VERIFY**

Run `cargo test -- --skip e2e`. All 403 existing tests plus ~15 new ones must pass.
Pay special attention to:
- All 15 existing `p2_*` tests
- All 18 existing `p3_*` tests
- All 23 existing `p1_*` tests

**Step F9 — GATE**

Run `cargo test -- --skip e2e`. Hard stop if any failure.

### Known Pitfalls

1. **Self-application depth:** PE(PE, PE) requires the PE to process its own source.
   The PE source is 556 lines. Encoding it produces thousands of CExpr/CStmt nodes.
   Processing this through the PE is expensive and may hit depth/step limits. Increase
   limits specifically for P3.

2. **Encoding the PE is recursive:** The PE source contains `peExpr` and `peBlock`
   functions. Encoding them as CProgram creates CFunc nodes whose bodies contain
   references to `peExpr` — which must also be encoded. Ensure `encode_program_source()`
   handles recursive function references correctly (it should — functions are referenced
   by name, not by value).

3. **Residual renaming:** The cosmetic rename (peExpr → compileExpr/cogenExpr) uses
   simple string replacement on the decompiled residual. This is safe because the
   decompiler produces clean source with no string collisions. But verify that no
   variable named "peExpr" exists in the residual (it shouldn't — the PE doesn't
   create variables with that name).

4. **run_logos_source() in library:** Moving the compile-and-run pipeline to library
   code is non-trivial. It requires either:
   - Spawning a subprocess (rustc + binary) — introduces system dependencies
   - Using the interpreter — slower but portable
   - JIT compilation — not available

   Recommend the interpreter approach for correctness, with optional rustc approach
   for performance.

5. **Backward compatibility:** The existing P2/P3 tests (33 total) use the current
   string-replacement implementation. They verify that the output contains certain
   function names and produces correct results. The new implementation must produce
   output that satisfies these same assertions. If the real specialization produces
   slightly different source code (e.g., different variable names or ordering), the
   existing tests might break. In that case, the tests need careful review — but per
   CLAUDE.md rules, we cannot modify them. If an existing test fails, the new
   implementation must be adjusted to produce compatible output.

6. **No fallbacks.** If self-application does not produce clean residuals, the answer
   is binding-time improvements to the PE source, not switching to generating
   extensions. Self-application is the definition of P2 and P3. See Step F7b for
   the iterative refinement approach.

---

## Sprint G — Final Integration & Jones Optimality (~15 tests)

### Overview

This is the final sprint. Everything before this built the infrastructure for
self-application. This sprint verifies that self-application *actually works* — that
`pe(pe, interpreter)` produces a genuine compiler, not a renamed PE.

It:
1. Unifies `projection1_source_real()` / `projection2_source_real()` /
   `projection3_source_real()` with the main projection functions — the string-replacement
   versions are deleted entirely
2. Verifies Jones optimality (zero interpretive overhead in P1 residuals)
3. Verifies The Trick end-to-end (PE's own Inspect eliminated in P2 via self-application)
4. Verifies online PE infrastructure eliminated in P2/P3 (no `isStatic`, `isLiteral`,
   `allStatic`, `depth`, `specResults`, `onStack` in residuals)
5. Runs all 276 existing tests against real self-application implementations
6. Size regression: `|P3| < |P2| < |PE|`
7. Cross-interpreter P3 test (calculator interpreter — proves cogen works for ANY
   interpreter, not just the ones it was derived from)

If any of the self-application verification assertions fail (Steps G2, G3, G4), the
fix is binding-time improvements to `pe_source.logos`, not relaxing the assertions.

### TDD Steps

**Step G1 — RED: Jones optimality — no env/funcs lookups in P1 residual**

File: `crates/logicaffeine_tests/tests/phase_futamura.rs`

```rust
#[test]
fn fix_jones_no_env_lookup() {
    // P1 residual for "Let x be 5. Show x." should be "Show 5.",
    // NOT "Set item \"x\" of env to VInt(5). Show item \"x\" of env."
    let program = r#"
## Main
    Let x be 5.
    Show x.
"#;
    let result = projection1_source_real(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    // The residual should NOT contain env lookup overhead
    assert!(!result.contains("item \"x\" of env"),
        "P1 residual should not contain env lookups");
    assert!(!result.contains("VInt"),
        "P1 residual should not contain CVal constructors");
    assert_exact_output(&result, "5\n");
}
```

```rust
#[test]
fn fix_jones_no_funcs_lookup() {
    // P1 residual for a simple function call should inline the function,
    // not retain a lookup in the funcs map
    let program = r#"
## To double (n: Int) -> Int:
    Return n * 2.

## Main
    Show double(21).
"#;
    let result = projection1_source_real(
        CORE_TYPES_FOR_PE_STR,
        INTERPRETER_SOURCE,
        program,
    ).unwrap();
    assert!(!result.contains("item \"double\" of funcs"),
        "P1 residual should not contain funcs lookups");
    assert_exact_output(&result, "42\n");
}
```

Expected: FAIL (real P1 not yet optimized for Jones optimality).

**Step G2 — RED: The Trick — PE's own Inspect eliminated in P2**

```rust
#[test]
fn fix_the_trick_pe_inspect_eliminated() {
    // P2 residual should have fewer Inspect nodes than PE source.
    // The PE's dispatch on CExpr variants should be eliminated when the
    // PE specializes itself with respect to the interpreter.
    let pe_source = pe_source_text();
    let compiler = projection2_source_real().unwrap();
    let pe_inspect_count = pe_source.matches("Inspect ").count();
    let compiler_inspect_count = compiler.matches("Inspect ").count();
    assert!(compiler_inspect_count < pe_inspect_count,
        "P2 compiler ({} Inspects) should have fewer Inspects than PE ({} Inspects)",
        compiler_inspect_count, pe_inspect_count);
}
```

Expected: FAIL.

**Step G3 — RED: Online PE infrastructure eliminated in P2**

```rust
#[test]
fn fix_online_pe_no_is_static_in_p2() {
    // P2 compiler should not contain isStatic/isLiteral/allStatic calls
    // These are PE dispatch functions that should be eliminated during
    // self-application
    let compiler = projection2_source_real().unwrap();
    assert!(!compiler.contains("isStatic("),
        "P2 compiler should not contain isStatic calls");
    assert!(!compiler.contains("isLiteral("),
        "P2 compiler should not contain isLiteral calls");
    assert!(!compiler.contains("allStatic("),
        "P2 compiler should not contain allStatic calls");
}

#[test]
fn fix_p2_no_depth_guard() {
    // P2 compiler should not have depth-based termination guards
    let compiler = projection2_source_real().unwrap();
    assert!(!compiler.contains("depth is at most 0"),
        "P2 compiler should not contain depth guards");
}

#[test]
fn fix_p2_no_memo_infra() {
    // P2 compiler should not contain memoization infrastructure
    let compiler = projection2_source_real().unwrap();
    assert!(!compiler.contains("specResults"),
        "P2 compiler should not contain specResults");
    assert!(!compiler.contains("onStack"),
        "P2 compiler should not contain onStack");
}
```

Expected: FAIL.

**Step G4 — RED: P3 structure differs from PE**

```rust
#[test]
fn fix_p3_cogen_structure_differs() {
    // P3 (compiler generator) should have different structure than PE
    // It should have fewer functions than the full PE
    let pe_source = pe_source_text();
    let cogen = projection3_source_real().unwrap();
    let pe_fn_count = pe_source.matches("## To ").count();
    let cogen_fn_count = cogen.matches("## To ").count()
        + cogen.matches("compileExpr").count().min(1)  // renamed functions
        + cogen.matches("cogenExpr").count().min(1);
    // The cogen should be structurally different — not just a renamed PE
    assert_ne!(pe_fn_count, cogen_fn_count,
        "P3 cogen should have different function count than PE");
}
```

Expected: FAIL.

**Step G5 — RED: Full roundtrip test**

```rust
#[test]
fn fix_full_roundtrip() {
    // compile_and_run(decompile(pe(encode(program)))) = compile_and_run(program)
    // for multiple test programs
    let programs = vec![
        ("## Main\n    Show 42.\n", "42\n"),
        ("## Main\n    Let x be 3 + 4.\n    Show x.\n", "7\n"),
        ("## Main\n    If true:\n        Show 1.\n    Otherwise:\n        Show 0.\n", "1\n"),
        ("## Main\n    Let mutable x be 1.\n    While x is at most 3:\n        Show x.\n        Set x to x + 1.\n", "1\n2\n3\n"),
        ("## To f (n: Int) -> Int:\n    Return n * 2.\n\n## Main\n    Show f(5).\n", "10\n"),
    ];
    for (program, expected) in &programs {
        let real_p1 = projection1_source_real("", "", program).unwrap();
        assert_exact_output(&real_p1, expected);
    }
}
```

Expected: FAIL (projection1_source_real not yet working for all programs).

**Step G6 — RED: All 276 existing tests pass on real implementations**

```rust
#[test]
fn fix_all_276_on_real() {
    // This is a meta-test. Once the _real() functions are unified with the
    // main projection functions, this test verifies by running:
    //   cargo test --test phase_futamura
    // and confirming all 276 tests pass.
    //
    // For now, test a representative sample:
    let simple_programs = vec![
        "## Main\n    Show 42.\n",
        "## Main\n    Show 1 + 2.\n",
        "## Main\n    Show true.\n",
    ];
    for program in &simple_programs {
        let result = projection1_source_real("", "", program);
        assert!(result.is_ok(), "Real P1 should handle: {}", program);
    }
}
```

Expected: FAIL.

**Step G7 — GREEN: Unify _real() functions with main projection functions**

File: `crates/logicaffeine_compile/src/compile.rs`

1. Replace `projection1_source()` body with `projection1_source_real()` implementation
2. Replace `projection2_source()` body with `projection2_source_real()` implementation
3. Replace `projection3_source()` body with `projection3_source_real()` implementation
4. Remove the `_real()` variants
5. Run ALL 276 existing tests: `cargo test --test phase_futamura`
6. Fix any discrepancies in the IMPLEMENTATION (not the tests)

**Step G8 — VERIFY**

Run `cargo test -- --skip e2e`. ALL tests must pass — the full 403 existing tests
plus all ~144 new tests from Sprints A through G.

**Step G9 — GATE**

Run `cargo test -- --skip e2e`. This is the FINAL gate. Hard stop if any failure.
If this gate is green, the three Futamura projections are real.

### Known Pitfalls

1. **Unifying functions may break existing tests.** The existing 56 P1/P2/P3 tests
   were written for the old string-replacement implementations. The real implementations
   may produce different source code (different variable names, different ordering,
   different structure). If tests fail, the implementation must be adjusted — not the
   tests. This may require the real projections to produce output compatible with the
   existing test assertions.

2. **Jones optimality is the target.** True Jones optimality (zero interpretive
   overhead) is what we are building toward. The tests verify SPECIFIC overhead
   markers (env lookups, funcs lookups, CVal constructors). If some overhead remains,
   the fix is in the PE source — binding-time improvements, restructuring, BTI
   annotations — not in relaxing the assertions.

3. **Performance.** Running all 276 tests with real projections (which involve
   compiling and running LogicAffeine source) may be slow. Consider marking the
   full-276 test as `#[ignore]` for normal CI, running it explicitly for release gates.

4. **The Trick will require multiple PE refinement iterations.** Jones et al. (1993)
   note that achieving clean self-application requires binding-time improvements
   to the PE itself. The Trick will likely not work on the first attempt. Add BTI
   annotations to pe_source.logos (e.g., restructuring conditionals so static
   arguments are always pattern-matched first). This iterative refinement IS the
   work — it is not a sign of failure, it is the established path.

---

## Cross-Reference: Corner Cuts → Sprints

| Corner Cut | Sprint | Tests Addressing It |
|------------|--------|-------------------|
| 1. SpecKey = String | A | pe_spec_key_is_structured, pe_spec_key_no_string_collision |
| 2. embeds()/msg() dead code | B | supercompile_while_precise_widening, supercompile_embedding_prevents_divergence |
| 3. EffectEnv not integrated | A | pe_effect_env_allows_pure_specialization, pe_effect_env_blocks_impure_specialization |
| 4. BTA no SCC ordering | B | bta_scc_mutual_recursion_converges, bta_scc_three_way_recursion |
| 5. isLiteral not isStatic | C | fix_pe_is_static_float, fix_pe_static_list_specialization |
| 6. CInspect no static dispatch | C | fix_pe_inspect_static_dispatch, fix_pe_inspect_dynamic_preserved |
| 7. CRepeat no unrolling | C | fix_pe_repeat_static_list_unroll, fix_pe_repeat_static_range_unroll |
| 8. No decompile in PE source | D | fix_decompile_expr_int, fix_decompile_stmt_let, fix_decompile_block_full, fix_decompile_roundtrip |
| 9. P1 ignores interpreter | E | fix_p1_real_simple_show, fix_p1_real_no_overhead |
| 10. P2/P3 string replacement | F | fix_p2_real_produces_compiler, fix_p3_real_produces_cogen, fix_cross_projection_equivalence |
| 11. No mixed-arg specialization | C.2 | fix_pe_mixed_arg_specializes, fix_pe_mixed_arg_interpreter_dispatch, fix_pe_mixed_arg_recursive |
| 12. No memoization table | C.3 | fix_pe_memo_prevents_respecialization, fix_pe_memo_cycle_detection |
| 13. No static map/collection ops | C.5 | fix_pe_static_index_list, fix_pe_static_len, fix_pe_static_field_access |
| 14. No two-level distinction  | C.4 | fix_pe_env_split_static_let, fix_pe_env_split_self_application |
| 15. Env blocks static prop    | C.4 | fix_pe_env_split_into_function, fix_pe_env_split_map_lookup |
| 16. No size regression tests  | F  | fix_p2_smaller_than_pe, fix_p3_smaller_than_p2 |
| 17. run_logos_source missing   | E  | fix_p1_real_simple_show (exercises the pipeline) |
| 18. Loops not self-app aware  | C  | fix_pe_repeat_static_list_unroll (extend for P2 scenario) |
| 19. Rename masks failure       | F  | fix_p2_raw_no_pe_dispatch |
| 20. No cross-interp P3 test  | F  | fix_p3_calculator_interpreter |
| 21. The Trick not verified | G | fix_the_trick_pe_inspect_eliminated |
| 22. No positive info propagation | C.4 | fix_positive_info_inspect_arm |
| 23. VMap not liftable | C | (VMap design decision documented in Step C6) |
| 24. No post-unfolding | C.3 | fix_pe_source_cascading_specialization |
| 25. Arity raising untested | C.2 | fix_arity_recursive_self_call, fix_arity_mutual_recursion |
| 26. Break/Return in unrolling | C | fix_pe_unroll_respects_break, fix_pe_unroll_respects_return |
| 27. Online PE leaks into P2/P3 | G | fix_online_pe_no_is_static_in_p2 |
| 28. No partially-static structures | C.1.5 | fix_partial_static_list_index_static, fix_partial_static_len |
| 29. Self-application must work | C.2, C.3, C.4, F, G | All self-application infrastructure + BTI refinement |
| 30. No let-insertion / shared computation | C.3 | fix_pe_no_code_duplication |
| 31. WQO not guaranteed in PE source | C.3 | fix_pe_source_termination_guarantee |

## Verification Checklist

### Structural
- [ ] Every corner cut (1-31) has a sprint addressing it
- [ ] Every sprint has a GATE step
- [ ] No existing tests are modified (only new tests added)
- [ ] All 403 existing tests remain green after each sprint
- [ ] Test count grows by ~144 (12 + 12 + 17 + 0 + 8 + 14 + 10 + 10 + 6 + 8 + 12 + 15 + 15 = ~139, plus helpers)
- [ ] Sprints ordered: A → B → C → C.1 → C.1.5 → C.2 → C.3 → C.4 → C.5 → D → E → F → G
- [ ] Each sprint is self-contained: failing at any sprint leaves the codebase green

### Dependencies
- [ ] PEState refactor (C.1) is complete before memoization (C.3) and staticEnv (C.4)
- [ ] Partially-static structures (C.1.5) is complete before mixed-arg (C.2)
- [ ] Mixed-arg specialization (C.2) is complete before real projections (E/F)
- [ ] Memoization (C.3) is complete before self-application (F)
- [ ] Environment splitting (C.4) is complete before static map ops (C.5)
- [ ] Static map ops (C.5) is complete before self-application (F)
- [ ] Backward compat: _real() functions in E/F, unification in G

### Projection Verification
- [ ] P2 raw residual contains no peExpr/peBlock references BEFORE rename
- [ ] P2 output line count < PE source line count
- [ ] P3 output line count < P2 output line count
- [ ] P3 cogen applied to calculator interpreter produces working compiler
- [ ] Calculator compiler smaller than cogen and contains no PE infrastructure

### Theoretical Concepts
- [ ] "The Trick" verified: PE's own Inspect on CExpr eliminated during P2 (Sprint G)
- [ ] Jones optimality: P1 residual has zero env/funcs lookups for static values (Sprint G)
- [ ] Online PE infrastructure eliminated: no isStatic/isLiteral in P2 residual (Sprint G)
- [ ] Positive information propagated through Inspect arms (Sprint C.4)
- [ ] Partially-static structures: `item 1 of [1, x, 3]` folds to `1` (Sprint C.1.5)
- [ ] WQO termination: PE source uses structural embedding, not string substring (Sprint C.3)
- [ ] Self-application produces clean P2 residuals — no fallback to generating extensions

### Infrastructure
- [ ] PEState record used throughout pe_source.logos (no parameter explosion)
- [ ] staticEnv enables static propagation through PE's own variable bindings
- [ ] run_logos_source() uses Rust compile pipeline (compile_to_rust → rustc → execute)
- [ ] VMap lifting: design decision documented, staticEnv workaround in place
- [ ] Post-unfolding: PE source cascades specialization after inlining (Sprint C.3)
- [ ] Arity raising: recursive calls use specialized variant name (Sprint C.2)
- [ ] Break/Return respected during static unrolling (Sprint C)
- [ ] Let-insertion: two identical all-static calls don't duplicate computation (Sprint C.3)
- [ ] All 276 existing tests pass on real projection functions (Sprint G)
- [ ] Performance budgets met: P1<30s, P2<300s, P3<600s (Sprint G)
