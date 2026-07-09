# PE_IMPROVE — Making the Partial Evaluator State-of-the-Art (with the World's Best TDD Plan)

> Engineering plan to take the LOGOS partial evaluator from "complete for the operations it folds" to
> *genuinely complete* — full operation coverage, the cleanest Futamura projections in existence, and a
> first-class interpreter it can specialize. Internal-engineering document: every claim cites
> `path:line`; **every phase is defined by its tests first**; no corners cut.
>
> The testing sections (§4, and the per-phase plans in §5) are the heart of this document. A partial
> evaluator is the kind of program where a plausible-looking output is the most dangerous failure mode,
> so the tests must prove the two things we actually care about — *it preserved meaning* and *it
> actually specialized* — not weak proxies like "it parsed" or "it compiled."

---

## 1. Architecture and goal — two world-class engines, one language

Most languages pick a side: an interpreter (Python, Ruby) *or* an optimizing compiler (Rust, C++).
LOGOS commits to **both, first-class** — that is the whole bet. They are two sides of one coin:

- **The interpreter.** A tree-walking interpreter (`interpreter.rs:82`, `RuntimeValue`) that walks the
  real AST directly. It is what runs LOGOS **in the browser, compiled to WASM** — the interactive "run
  it right now" experience. It is correct and portable, **but it is slow** (interpretive dispatch on
  every AST node). **We are replacing it** with a world-class fast interpreter.
- **The optimizing compiler.** real AST → the **advanced 14-pass optimizer** `optimize_program`
  (`optimize/mod.rs:52`) → **Rust codegen** → `rustc` → native. This is the deployment/performance side
  and is **already its own world-class artifact** — abstract interpretation, supercompilation
  (whistle+MSG), GVN/LICM/closed-form/deforestation/DCE/CTFE. **This plan does not touch it.** It is the
  other coin, and it stays excellent.

**This plan is about the interpreter coin.** The way we make the interpreter world-class is the
partial-evaluation / Futamura machinery from `EXODIA.md`: a **self-interpreter** written in LOGOS (over
the `CExpr` IR) plus a **LOGOS-level PE** (`pe_source.logos`) that *specializes the interpreter to a
program*, dissolving interpretive overhead (`PE(interp, program)` = the program with dispatch removed —
Jones optimality). That specialized residual is what the new fast interpreter runs (and, per EXODIA's
later phases, ultimately JITs via copy-and-patch). The slow tree-walker becomes two things at once: the
*reference implementation* we replace, and the *independent oracle* we prove the fast one equal to
before we flip the switch.

So "wire it into the interpreter properly" means: the self-interpreter becomes a first-class artifact,
the PE specializes it to full Jones-optimality across the **whole** language, we prove the new
interpreter behaves identically to the tree-walker on everything, and it **replaces** the tree-walker as
the WASM/browser/interactive engine. The compiler coin is untouched; both ends of the coin end up
world-class.

EXODIA marks this Tier-0 foundation "COMPLETE," but it is complete only for the **subset of operations
the PE actually folds**. The self-interpreter executes ~33 `CExpr` and ~54 `CStmt` variants with a rich
`applyBinOp` over Int/Float/Bool/Text/Duration/Date/Moment; the PE folds a strictly smaller subset.
Outside it, `P1` stays *correct* but residualizes — interpretive overhead remains, so the "fast
interpreter" would be fast only on a fragment. Closing that gap so the PE-specialized interpreter is
*uniformly* fast — and provably identical to the tree-walker — is this plan.

### 1.1 The IR

`phase_futamura.rs` `CORE_TYPES`: `CExpr` (~33 variants), `CStmt` (~54), `CVal` (~20). `PEState =
{env, funcs, depth, staticEnv, specResults, onStack}` (`pe_source.logos:1`).

---

## 2. Definition of "done" — what "best in the world" means

1. **Full operation coverage** — the PE folds *every* operation the self-interpreter executes: binop
   parity, partially-static data, real loop specialization, map/set/text folding, known-closure
   inlining, flow-sensitive refinement.
2. **Cleanest Futamura projections** — ONE canonical PE; BTI/mini variants *mechanically derived*, never
   hand-copied; projection driver is library code taking the interpreter as input.
3. **Jones-optimal `P1` for the full language** — `PE(interp, program)` has zero interpretive overhead
   (no `env`/`funcs` map lookups, no PE-dispatch names) for **every** program, proven by tests.
4. **Wired** — first-class self-interpreter, proven *behaviorally identical* to the tree-walker, with
   `PE(interp, program)` as the execution path that **replaces the slow tree-walker** as the
   WASM/browser interpreter. (The compiler coin stays separate and untouched.)

---

## 3. The concrete gaps (grounded, with line references)

Read first-hand from `pe_source.logos` (full). Each maps to a phase in §5.

| # | Gap | Where | Consequence |
|---|---|---|---|
| G1 | **All-or-nothing staticness.** `isStatic → true` only for fully-evaluated literal trees; `Otherwise → false`. | `pe_source.logos:214-263` | Any aggregate with one residual field is wholly Dynamic. The interpreter's `env` can never be *partially* static — the keystone for Jones-optimality. |
| G2 | **`evalBinOp` folds only Int/Bool/Text.** No Float/Duration/Date/Moment; no algebraic identities. | `:150-212` | PE folds fewer ops than the interpreter executes (`applyBinOp` covers all). `CFloat` is "static" yet never folds. |
| G3 | **`CWhile` never unrolls;** `CRepeat` only static `CList`; `CRepeatRange` only static `Int` range `<64`. | `:1113-1166`, `:1235`, `:1285-1309` | Dynamic-but-bounded and shape-static loops stay fully residual. |
| G4 | **No MSG.** Real whistle (`exprEmbeds`/`argsStrictlyEmbed`) exists but on blow it *residualizes*, never generalizes. | `:462-556`; cf. Rust `msg()` `supercompile.rs:873` | Collapsible recursion is cut instead of generalized → less-clean projections. |
| G5 | **No flow-sensitive refinement** at `CIf`. (`CInspect` *does* bind matched fields — the model.) | `:1091-1112`; `:1377-1477` | `If x == 3` (static `x`) doesn't refine the branch; dead arms survive. |
| G6 | **Map/Set/closure folding gaps.** `CMapGet` only variant/struct; `CLen` only list/tuple; `CContains` only empty→false; `CCallExpr` never inlines a known closure. | `:860-890`, `:850-859`, `:919-935`, `:980-985` | Real maps/sets/text and higher-order code stay interpretive. |
| G7 | **Three divergent PE copies.** `pe_mini` drops struct dispatch, `CFieldAccess`, `CLen` fold, `CRepeatRange` unroll vs `pe_source`. | `pe_source` vs `pe_mini_source` vs `pe_bti_source` | Same program ⇒ different residual depending on the PE. |
| G8 | **Self-interpreter not first-class.** `coreEval`/`coreExecBlock` live in a test + inline strings; a *different program* from the tree-walker. | `phase_futamura.rs`; `compile.rs:~4360-4433` | No single source of truth; free to drift from the interpreter we actually ship. |
| G9 | **Termination robustness** (EXODIA §0.1). Unbounded `specResults`; `onStack` per-all-static + collision-prone; `collectSetVars`/`extractReturn` fragile on nested early returns. | `:558-566`, `:628-686`, `:1006-1029`, `:1611-1620` | State growth / edge-case miscompiles under self-application. |

---

## 4. Testing philosophy — the world's best TDD for a partial evaluator

### 4.1 The three properties we actually care about

A partial evaluator transforms a program. There are exactly three things that can go wrong, so there
are exactly three properties every test gate must defend. **If a test does not target one of these, it
is not pulling its weight.**

1. **Correctness — semantic preservation (the prime directive).**
   `∀ program p, ∀ input in:  run( PE(p), in ) ≡ run( p, in )`.
   The PE may *never* change observable behavior — same return value, same output stream, same effects,
   same errors (including "this diverges" and "this raises"). This is the property that a
   plausible-but-wrong residual violates, and it is the one we test hardest.

2. **Optimality — specialization actually happened.**
   A PE that returns its input unchanged is perfectly *correct* and completely *useless*. So every
   correctness test is paired with an assertion that the static work is **gone**: constants folded,
   known branches pruned, bounded loops unrolled, and — for `P1` — *no interpreter dispatch left in the
   residual* (Jones optimality). We measure this structurally, not by eyeballing.

3. **Totality — the PE halts.**
   On every input, including adversarial recursion and loops designed to blow the whistle, the PE
   terminates (and terminates *quickly* — within the documented depth/whistle budget). A PE that loops
   forever on a pathological program is unshippable.

Every phase gate in §5 is a matrix of (1)×(2) over that phase's operations, plus (3) on that phase's
adversarial corpus.

### 4.2 The oracle architecture — triangulation (so the oracle itself is trustworthy)

The subtle trap: if we only assert `run(P1(p)) == self_interp(p)` and *both* run through the same
self-interpreter, a bug in the self-interpreter's *semantics* is invisible — both sides agree on the
wrong answer. So we triangulate three **independent** evaluators:

```
        tree-walking interpreter (interpreter.rs)   ← independent, production, the semantic ground truth
                       ║  (Phase D: differential-tested to agree)
        self-interpreter (interp_source.logos)      ← the spec the PE must preserve
                       ║  (Phases A–C: PE must not change its observable result)
        residual:  run( PE(p) )                      ← what the PE produced
```

- The **tree-walker is the independent ground truth** for the self-interpreter (Phase D differential
  harness). It was written separately, in Rust, on the real AST — it cannot share a bug with the LOGOS
  self-interpreter by construction.
- The **self-interpreter is the spec** the PE must preserve (`run(P1(p)) ≡ self_interp(p)`).
- Therefore `run(P1(p)) ≡ self_interp(p) ≡ tree_walk(p) ≡ run(p)` is a *chain of independent checks*,
  and a discrepancy anywhere localizes the bug.

### 4.3 The test arsenal (six techniques, used together)

1. **Differential execution.** For each program, run all available evaluators and assert identical
   observable behavior (value + output + error). This catches correctness bugs directly.
2. **Generative / property-based.** Generate random well-formed `CExpr`/`CStmt` programs with a tagged
   static/dynamic environment (a deterministic generator seeded per-index, since `Math.random` is
   unavailable in this harness). Assert *preservation* (correctness) and *folding* (every
   statically-determined subterm disappears from the residual). This is how we get coverage we could
   never hand-write.
3. **Metamorphic relations** (no oracle needed — relate two PE runs):
   - **Futamura equation:** `run(P1(p), in) == self_interp(p, in)`.
   - **Substitution consistency:** specializing `f` with arg `a` static `≡` substituting `a` into `f`
     then specializing.
   - **Idempotence:** `PE(PE(p)) ≡ PE(p)` — the residual is a fixpoint; a second pass finds no static
     work. (Strong optimality signal: if a second pass *does* fold more, the first pass left money on
     the table.)
   - **Dead-static invariance:** prepending an unused static computation changes neither the output nor
     (after folding) the residual.
   - **Dynamic-wrapper invariance:** wrapping the program so all inputs become dynamic must yield a
     residual whose *folded core* is unchanged.
4. **Structural residual assertions** (optimality). Assert the residual *contains* the expected folded
   form and *does not contain* the eliminated form — using `contains()` substring checks on decompiled
   output (resilient, per the project's `phase30_iteration` pattern), never brittle full-string golden
   equality (which tests the renderer, not the PE).
5. **Jones-optimality oracle.** A residual analyzer that counts interpreter-dispatch constructs
   (`env`/`funcs` map lookups, `peExpr`/`peBlock`/`coreEval` names, `Inspect`-on-AST-tag) and asserts
   **zero** for `P1`. This is the executable definition of "the interpreter dissolved."
6. **Adversarial corpus + coverage matrix.** A hand-curated set of pathological programs ("robust to
   absurdity") *and* a mechanical matrix: every `CExpr`/`CStmt` variant × {fully-static, fully-dynamic,
   partially-static} has a test asserting the right fold/residual.

### 4.4 How we avoid testing the wrong thing (anti-patterns — banned)

| Anti-pattern | Why it's worthless | What to do instead |
|---|---|---|
| "Residual parses / compiles without error" | Says nothing about meaning or specialization | Differential-run it; assert output equality |
| Full-string golden equality on decompiled residual | Tests the *renderer*; breaks on cosmetic changes; passes on semantically-wrong output of the same shape | `contains()` of the key folded/eliminated tokens + a run-equality check |
| Asserting only the static case | Misses the dynamic and *partially-static* cases where PEs actually break | Coverage matrix: static × dynamic × partial for every variant |
| Asserting output equality only (no optimality) | A no-op PE passes | Pair every correctness assertion with a structural/Jones assertion |
| Trusting the self-interpreter as ground truth | Shares bugs with the PE's spec | Triangulate against the independent tree-walker (§4.2) |
| Testing return value but not effects/output/errors | Misses `Show` ordering, IO, divergence, raised errors | Differential on the *full observable behavior* |

### 4.5 Shared harness (build once, reuse every phase)

- `pe_test_support` (new test module): `run_all(program, env) -> Observation { value, output, error }`
  for {tree-walk, self-interp, run(P1)}; `assert_same_behavior(...)`; `decompile(residual) -> String`;
  `count_dispatch(residual) -> usize` (the Jones oracle); `gen_program(seed, shape) -> CProgram` (the
  generator); and the curated `adversarial_corpus()`.

---

## 5. Phased roadmap — each phase is its tests

Every phase is **RED-first** (write failing tests, watch them fail, then implement — per `CLAUDE.md`;
never edit a RED test to pass it). The gate is the listed tests green **and** the full suite green with
zero regressions.

---

### Phase A — Soundness & termination foundation (closes G9)

**Property under test:** totality + preservation under self-application; no state explosion.
**Oracle:** self-interp result before/after, plus a step/size budget on the PE itself.

**`phase_pe_termination.rs` (RED first):**
- `pe_terminates_on_mutual_recursion` — `f`→`g`→`f` with growing static args; assert PE halts within the
  whistle budget and the residual run-equals the interpreter.
- `pe_return_inside_inspect_arm` — `CReturn` nested in a `CInspect` arm; assert `extractReturn` finds it
  and the residual returns the right value (guards the `__no_return__` sentinel path, `:1611`).
- `pe_return_inside_nested_if` — return buried in `CIf`/`CIf`; same assertion.
- `pe_while_invalidates_nested_writes` — a var written via nested `CIf` inside a dynamic `CWhile`, then
  read after; assert the post-loop value is *not* wrongly treated as static (correctness), i.e. the
  residual reads the runtime value.
- `pe_specresults_bounded` — a program that would memo thousands of distinct keys; assert `specResults`
  stays within the documented bound and the run is still correct.
- `pe_memo_key_no_collision` — two genuinely different all-static calls that previously could collide on
  `makeKey`; assert distinct specialization (residual differs, both correct).
- `pe_self_application_stable` — `PE(pe_source, pe_source)` halts and the doubly-specialized PE still
  specializes a sample program correctly (the genuine-self-application stress).

**Edge cases to enumerate as tests:** early return with dead code after it; inner `Let x` shadowing an
outer static `x`; `Break` mid-unroll; a function reached by two distinct static paths; `CWhile` whose
body never writes the condition var (must still terminate, not unroll forever).

**Gate:** above green; existing 544 Futamura + 44 partial_eval + 66 supercompile tests green.

---

### Phase B1 — `evalBinOp` parity + identities (closes G2)

**Property:** for every binary op the interpreter's `applyBinOp` executes, the PE folds it to the *same*
value when operands are static — and matches the interpreter's behavior on the *undefined* cases.
**Oracle:** the self-interpreter's `applyBinOp` is the spec — fold result must equal what the
interpreter would compute.

**`phase_pe_binop.rs` (RED first), one test per (op, type):**
- Int: `+ - * / % < > <= >= == != ^ << >>` — including **div-by-zero / mod-by-zero must match the
  interpreter** (it returns `VNothing`; the PE must *not* fold to a wrong value — assert the residual
  behaves identically, i.e. does not fold to garbage).
- Int **wrapping overflow** — `i64::MAX + 1` folds to exactly the interpreter's wrapped value.
- Float: `+ - * / < > <= >= == !=` — including **NaN ≠ NaN**, `+Inf`, `-0.0` bit-equality (match
  interpreter's float semantics exactly).
- Duration / Date / Moment: every op the interpreter supports (`Date + Duration`, `Date − Date`,
  `Moment` comparisons, etc.).
- **Type-mismatch** (`Int + Text`, `Bool < Int`): interpreter yields `VNothing` → PE must **residualize,
  not fold**.
- Algebraic identities (only where type-sound and effect-safe): `x+0`, `0+x`, `x-0`, `x-x`, `x*1`,
  `1*x`, `x*0`→0, `x/1`, `x&&true`, `true&&x`, `x||false`, `!!x`. **Effect-safety edge case:**
  `f() * 0` must **not** drop the call if `f` may have effects — assert the residual still calls `f`
  (this is the classic identity-elimination soundness trap).

**Gate:** every interpreter-supported binop folds when static and matches the interpreter; identities
fire only when sound; type-mismatch and div-by-zero residualize correctly.

---

### Phase B2 — Partially-static data (closes G1; **the keystone**)

**Property:** an aggregate (struct/list/map/tuple) with *some* static fields and *some* residual fields
specializes the static parts and residualizes only the dynamic parts — and reading a static field of a
partially-static aggregate folds. This is what makes interpreter specialization Jones-optimal (the
interpreter's `env` becomes partially static: names/shape static, runtime values dynamic).
**Oracle:** differential execution (preservation) + structural assertion (the static field folded).

**`phase_pe_partial.rs` (RED first):**
- `partial_struct_static_field_folds` — `CNew` with field `a=5` (static), `b=x` (dynamic); `FieldAccess
  .a` folds to `5`; `.b` residualizes; **run-equals** interpreter.
- `partial_struct_setfield_invalidates_only_mutated` — `SetField .a` then read `.a` (now dynamic) and
  `.b` (still static); assert exactly the mutated field lost its static fact.
- `partial_list_index_static` — list `[1, x, 3]`; `Index 1`→`1`, `Index 2`→residual, `Index 3`→`3`.
- `partial_map_static_key` — map with one static and one dynamic entry; static-key `MapGet` folds.
- `partial_nested` — struct-in-list-in-struct, static spine + dynamic leaf; the static path folds, the
  leaf residualizes; run-equals.
- `partial_flows_into_call` — pass a partially-static struct to `f`; `f` specializes on the static
  field; dynamic field stays a parameter.
- `partial_valToExpr_faithful` — round-trip a partially-static value back to residual `CExpr` and
  assert run-equality (the reconstruction is faithful).
- **Aliasing edge case** `partial_alias_mutation_invalidates` — `Let a be s. Push x to a.` must
  invalidate `s`'s static facts (reference semantics, `LogosSeq` is `Rc<RefCell>`). At minimum:
  *conservative and correct* — assert no stale static fact survives a mutation through an alias.

**Edge cases:** empty aggregate; aggregate all-static (must behave exactly as today — no regression);
aggregate all-dynamic; mutation inside a loop; a static field whose value is itself partially static.

**Gate:** the coverage matrix for aggregates × {static, dynamic, **partial**} passes; `P1` of a tiny
interpreter loop over a record shows the env dispatch *dissolved* (Jones oracle = 0) — the headline
demonstration that the keystone works.

---

### Phase B3 — Loop specialization + MSG (closes G3, G4)

**Property:** statically-bounded loops fully unroll (no loop in residual); dynamic loops with a
static-shaped body specialize to a clean residual loop *without state explosion* (whistle + MSG
converge); all of it preserves meaning.
**Oracle:** differential (preservation) + structural (loop gone / loop present-but-specialized) +
budget (totality).

**`phase_pe_loops.rs` (RED first):**
- `while_static_trip_count_unrolls` — `CWhile` with a statically-decreasing counter; residual has **no
  `CWhile`**; run-equals.
- `repeat_range_zero_iterations_eliminated` — empty range → loop removed entirely.
- `repeat_break_stops_unroll` — `Break` in iteration 2 of a static unroll stops unrolling there (guards
  `:1259-1262`); run-equals.
- `repeat_return_stops_unroll` — `Return` mid-unroll truncates correctly.
- `loop_large_static_range_does_not_explode` — range of 10⁶: assert the PE **does not** unroll
  unboundedly (respects the budget) and falls back to a residual loop — totality over optimality.
- `dynamic_loop_msg_generalizes` — a loop whose body specializes slightly differently each iteration;
  assert the whistle blows, **MSG generalizes** (fresh residual var appears, e.g. `__msg_0`), the
  residual is a single clean loop, and run-equals. (This is the test that proves G4 is fixed — without
  MSG this either residualizes verbatim or loops.)
- `msg_idempotent` — `PE(PE(loop)) == PE(loop)` (the generalized form is a fixpoint).

**Edge cases:** nested loops; loop-carried dynamic accumulator; loop that never iterates; loop whose
bound is partially static (B2 interaction); mutual recursion masquerading as a loop.

**Gate:** static loops vanish; dynamic loops converge via MSG; large static loops respect the budget;
all run-equal.

---

### Phase B4 — Flow-sensitive refinement (closes G5)

**Property:** guard facts refine `staticEnv` inside the taken branch (positive info) and its negation in
the other branch (negative info); provably-dead arms are eliminated; facts never leak across branches.
**Oracle:** differential + structural (dead arm gone).

**`phase_pe_refine.rs` (RED first):**
- `if_eq_static_prunes_branch` — `If x == 3` with static `x=3` → residual is the then-branch only.
- `if_eq_static_false_takes_else` — static `x=4` → else only.
- `else_branch_gets_negative_fact` — in the else of `If x == 3`, `x != 3` is usable to fold a nested
  guard.
- `no_fact_leak_across_branches` — a then-branch fact must **not** apply in the else (assert the else is
  unchanged).
- `refinement_invalidated_by_later_mutation` — `If x==3: Set x to 4. ... use x` — the `x→3` fact is
  dropped after the `Set`; residual reads the new value.
- `compound_guard_and_or_not` — `If a and b`, `If a or b`, `If not a` refine soundly (or conservatively
  — never *unsoundly*).
- `refine_composes_with_inspect` — guard refinement + `CInspect` field binding together (`:1377`).

**Edge cases:** guard comparing two dynamics (no fact); guard with a call (`If f() == 3` — refine the
*result* binding, not re-call `f`); deeply nested guards; contradictory guards (`If x==3` inside `If
x==4` → dead, eliminated).

**Gate:** dead arms eliminated under static guards; no cross-branch leakage; mutation invalidation
correct; run-equal throughout.

---

### Phase B5 — Maps / Sets / Closures (closes G6)

**Property:** static map/set/text operations fold; a statically-known closure is inlined at the call
site (enabling higher-order specialization, the runway for EXODIA defunctionalization).
**Oracle:** differential + structural.

**`phase_pe_collections_hof.rs` (RED first):**
- `mapget_static_hit` / `mapget_static_miss` — present key folds to value; absent key matches the
  interpreter's miss behavior.
- `len_on_map_set_text` — `CLen` folds for maps, sets, and text (not just list/tuple).
- `contains_literal_membership` — `CContains` folds true/false on a non-empty static collection.
- `closure_inlined_when_static` — known `CClosure` at `CCallExpr` is inlined; residual has no
  `CCallExpr` indirection; run-equals.
- `closure_partial_capture` — closure capturing one static + one dynamic var → partial inline (B2
  interaction).
- `closure_specialized_args` — closure called with static+dynamic args specializes on the static.

**Edge cases:** recursive closure (must terminate via the whistle); closure stored in a struct then
called; closure returned from a function; map with duplicate-key construction; set folding semantics.

**Gate:** the map/set/text/closure coverage matrix passes; HOF programs lose their indirection.

---

### Phase C — Cleanest projections: unify the three PEs (closes G7)

**Property:** there is **one** canonical PE; the BTI and mini variants are *mechanically derived* and
produce **byte-identical residual** to the canonical PE on the corpus; self-application still works.
**Oracle:** cross-PE residual equality + differential run-equality.

**`phase_pe_unify.rs` (RED first):**
- `derived_bti_matches_canonical` — for every program in the corpus, `decompile(PE_canonical(p)) ==
  decompile(PE_bti(p))`.
- `derived_mini_matches_canonical` — same for the mini variant (this currently *fails* because mini
  drops features — the RED test that forces the unification).
- `variants_are_regenerable` — the derived variants equal the checked-in/generated artifacts (no manual
  drift possible).
- `projections_reproduced_from_one_source` — `projection2`/`projection3` built from the canonical PE
  still pass their existing assertions.
- `self_application_after_unify` — genuine self-application still halts and specializes correctly.

**Gate:** one PE source of truth; all three "variants" agree byte-for-byte; projections green.

---

### Phase D — First-class interpreter & wiring (closes G8; the "wire it in" deliverable)

**Property:** the self-interpreter is a first-class artifact that **provably agrees with the production
tree-walker** across all operations and effects; `PE(interp, program)` is a real, Jones-optimal
execution path ready to replace the tree-walker as the WASM/browser interpreter.
**Oracle:** the full triangulation (§4.2).

**`phase_pe_differential.rs` (RED first) — the centerpiece:**
- `interp_vs_treewalk_corpus` — over the generated + adversarial corpus, `self_interp(p, in) ==
  tree_walk(p, in)` for **value, output stream, and error/divergence**. (This will surface real
  semantic drift between the two interpreters — expect to fix bugs here; that is the point.)
- `interp_vs_treewalk_effects` — `Show` ordering, `ReadFile`/`WriteFile`, console IO match.
- `interp_vs_treewalk_temporal` — Float/Date/Duration/Moment formatting and arithmetic match exactly.
- `full_chain_equivalence` — `run(P1(p), in) == self_interp(p, in) == tree_walk(p, in) == run(p, in)`.
- `concurrency_observational` — `CConcurrent`/`CParallel`/`CSelect` are tested for observational
  equivalence under a deterministic schedule (pin the schedule so the test is meaningful, not flaky).

**`phase_pe_jones.rs` (RED first) — Jones optimality on the full language:**
- `p1_no_dispatch_full_corpus` — for every program, `count_dispatch(P1(p)) == 0` (no `env`/`funcs`
  lookups, no `peExpr`/`peBlock`/`coreEval` names, strict `Inspect` reduction).
- `p1_residual_run_equals_source` — and it still computes the right answer (optimality never at the cost
  of correctness).

**Gate:** the differential corpus passes (the two interpreters are proven equal); `P1` is Jones-optimal
for every program; the PE-specialized interpreter produces output identical to the tree-walker across
the full corpus — the safety case for replacing it.

---

### Phase E — Replace the slow interpreter (the deliverable; staged, gated on A–D + benchmarks)

This is the payoff: the **PE-specialized self-interpreter replaces the tree-walking interpreter** as the
WASM/browser/interactive engine — making the *interpreter* coin world-class. Steps: (1) promote + harden
the self-interpreter and prove it identical to the tree-walker (Phase D); (2) make `PE(interp, program)`
the execution path — run the Jones-optimal, dispatch-free residual instead of tree-walking the raw AST;
(3) per EXODIA Phases 1–3, layer the Oracle (abstract-interp guard elimination) and the copy-and-patch
JIT on top for the final speed. The advanced compile-to-Rust optimizer is **untouched** — it remains the
other, independent coin. **Deferred** until A–D land and benchmarks show the specialized interpreter
beats the tree-walker with identical behavior.

*Open decision:* the residual's runtime in WASM — a compact bytecode VM (cf. `VM_PLAN.md`), a
WASM-targeting JIT, or a stripped minimal core executing the dispatch-free residual directly.

**Gate (when reached):** the specialized interpreter passes the *entire* existing interpreter test
suite with zero behavioral diffs vs the tree-walker, and is measurably faster on the benchmark corpus.

---

## 6. Critical files

- `crates/logicaffeine_compile/src/optimize/pe_source.logos` — canonical PE; B1–B5 land here.
- `…/optimize/pe_bti_source.logos`, `…/pe_mini_source.logos` — collapsed/derived in Phase C.
- `…/optimize/decompile_source.logos` — residual rendering; keep in sync; the decompiler is what the
  structural/Jones assertions read.
- `crates/logicaffeine_compile/src/compile.rs` — projection drivers (`projection*_source[_real]`,
  `3286-4814`), loaders (`pe_source_text` `:3962`), `encode_program_source` (`:1249`); refactored C/D.
- `…/optimize/mod.rs` — `optimize_program` (the advanced compile-to-Rust optimizer, **kept**) /
  `optimize_for_projection`; the Phase E seam.
- `crates/logicaffeine_compile/src/interpreter.rs` — production tree-walker; the **independent oracle**.
- `crates/logicaffeine_tests/tests/phase_futamura.rs` — IR types, self-interpreter, 544 tests; source
  for promoting `interp_source.logos`.
- `…/optimize/{bta.rs,supercompile.rs,partial_eval.rs}` — the advanced Rust engine; `supercompile.rs:873`
  is the MSG reference for B3.
- **New test modules:** `pe_test_support` (harness), `phase_pe_{termination,binop,partial,loops,refine,
  collections_hof,unify,differential,jones}.rs`, `phase_pe_coverage.rs` (the variant matrix).

## 7. Reused machinery (do not reinvent)

- Whistle: `exprEmbeds`/`argsStrictlyEmbed` (`pe_source.logos:462-556`) — correct; B3 adds MSG on top.
- MSG reference: `msg()`/`msg_inner()` (`supercompile.rs:873-923`).
- `CInspect` field binding (`pe_source.logos:1377`) — the working model for B4 guard refinement.
- The self-interpreter `applyBinOp` — the **spec** for B1; match it exactly.
- Tree-walker `interpreter.rs` — the **independent oracle** for Phase D.
- Polyvariant BTA `bta.rs::analyze_with_sccs` — binding-time facts feeding specialization.

## 8. Verification commands

- RED→GREEN per phase: `cargo test --no-fail-fast -- --skip e2e phase_pe_ phase_futamura
  > /tmp/pe.txt 2>&1; echo "EXIT: $?" >> /tmp/pe.txt` (one suite at a time).
- Full regression before any phase is "done" (per `CLAUDE.md`, Z3 env):
  `Z3_SYS_Z3_HEADER=/opt/homebrew/include/z3.h BINDGEN_EXTRA_CLANG_ARGS="-I/opt/homebrew/include"
  LIBRARY_PATH="/opt/homebrew/lib" cargo test --features verification --no-fail-fast
  > /tmp/test_all.txt 2>&1; echo "EXIT: $?" >> /tmp/test_all.txt` — zero regressions.
- Never run multiple test suites concurrently; never run git (per `CLAUDE.md`).

## 9. Sequencing

```
A (soundness/termination)
  └─▶ B1 binop parity ─▶ B2 partial statics (KEYSTONE) ─▶ B3 loops+MSG
                                  │                            │
                                  └──────────┬─────────────────┘
                                             ▼
                          B4 flow refinement · B5 maps/sets/closures
                                             ▼
                 C (unify 3 PEs → 1 canonical, byte-identical projections)
                                             ▼
   D (first-class interp_source.logos + triangulation harness + Jones-optimal P1)
                                             ▼
   E (replace the slow tree-walker: PE-specialized interpreter becomes the WASM/browser engine,
      +Oracle/JIT per EXODIA — deferred, gated on benchmarks; the compile-to-Rust optimizer is untouched)
```

**B2 (partially-static data) is the keystone.** It turns `PE(interp, program)` from "folds the easy
fragment" into a *complete, Jones-optimal* specializer, and it is the precondition for EXODIA's Oracle
(abstract-interpretation-driven specialization) to have anything to bite on. Everything else is
foundation beneath it (A), parity around it (B1/B3/B4/B5), or consequence above it (C/D/E). And every
one of those phases is *defined by the tests in §5* — we will know it is tested because the test names
the property (preservation, specialization, totality), runs it against an independent oracle, and
enumerates the edge cases we expect to break it.
