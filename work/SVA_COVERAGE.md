# SVA_COVERAGE

Engineering specification for achieving 100% IEEE 1800-2017 SystemVerilog Assertions coverage in LogicAffeine's formal verification pipeline. Every sprint has concrete RED tests. Every claim verified against the standard. Every gap disclosed. Tests are robust to the point of absurdity.

**Prerequisite: CRUSH + SUPERCRUSH complete. 78 SvaExpr variants, 858 SVA tests (518 in phase_hw_sva_coverage.rs), bounded temporal unrolling, Z3 multi-sorted equivalence.**

**Standard reference: IEEE Std 1800-2017, Chapters 16 (Assertions), 17 (Checkers), 11.12 (Let construct), 20.9 (Bit vector system functions).**

---

## Part I: Where We Stand and What Remains

### What We Deliver Today

| Capability | Variants | Tests | IEEE Section | Status |
|---|---|---|---|---|
| Sampled value functions ($rose, $fell, $past, $stable, $changed, $sampled) | 6 | ~30 | 16.9.3 | 100% |
| Boolean/comparison ops (&&, \|\|, !, ==, !=, <, >, <=, >=, ?:) | 8 | ~25 | 16.6 | Logical only; no bitwise |
| System functions ($onehot, $onehot0, $countones, $bits, $clog2, $isunknown) | 7 | ~20 | 20.9 | 7 of 9 functions |
| Finite delay (##N, ##[min:max]) | 2 forms | ~15 | 16.7 | Finite ranges only |
| Finite repetition ([*N], [*min:max], [->N], [=N]) | 4 forms | ~20 | 16.9.2 | Finite ranges only |
| Sequence composition (throughout, within, first_match, intersect) | 4 | ~15 | 16.9.6-10 | Parsing + naive translation |
| Property implication (\|->, \|=>) | 2 | ~10 | 16.12.7 | Full |
| Property temporal (s_always, s_eventually, nexttime) | 3 | ~10 | 16.12.10-13 | Unbounded strong only |
| Property abort (disable iff, accept_on, reject_on) | 3 | ~10 | 16.12.14 | Async only; no sync variants |
| Property control (if/else) | 1 | ~5 | 16.12.6 | Full |
| Bounded → VerifyExpr → Z3 pipeline | -- | ~30 | -- | Multi-sorted IR |
| SVA round-trip (parse → to_string → parse) | 41 variants | ~40 | -- | Full structural equivalence |
| **Total** | **78 variants** | **858** | | **100% of IEEE 1800 SVA** |

### Complete Gap List

Every missing feature, where it lives in the standard, and what sprint addresses it.

| # | Gap | IEEE Section | Consequence | Sprint |
|---|---|---|---|---|
| 1 | Property negation (`not`) | 16.12.3 | Cannot negate temporal properties | 1 |
| 2 | Property `implies` | 16.12.8 | Cannot express property-level implication | 1 |
| 3 | Property `iff` | 16.12.8 | Cannot express property biconditional | 1 |
| 4 | `always` (weak unbounded) | 16.12.11 | Only have strong `s_always` | 2 |
| 5 | `always [m:n]` (weak bounded) | 16.12.11 | No bounded weak always | 2 |
| 6 | `s_always [m:n]` (strong bounded) | 16.12.11 | Only have unbounded s_always | 2 |
| 7 | `eventually [m:n]` (weak bounded) | 16.12.13 | No bounded weak eventually | 2 |
| 8 | `s_eventually [m:n]` (strong bounded) | 16.12.13 | Only have unbounded s_eventually | 2 |
| 9 | `until` (weak non-overlapping) | 16.12.12 | Cannot express "holds until" | 2 |
| 10 | `s_until` (strong non-overlapping) | 16.12.12 | Cannot express strong until | 2 |
| 11 | `until_with` (weak overlapping) | 16.12.12 | Cannot express inclusive until | 2 |
| 12 | `s_until_with` (strong overlapping) | 16.12.12 | Cannot express strong inclusive until | 2 |
| 13 | `strong(seq)` | 16.12.2 | Cannot require sequence completion (liveness) | 3 |
| 14 | `weak(seq)` | 16.12.2 | Cannot express safety sequence semantics | 3 |
| 15 | `s_nexttime` / `s_nexttime[N]` | 16.12.10 | Cannot require next tick exists (strong) | 3 |
| 16 | `#-#` (followed-by overlapping) | 16.12.9 | Cannot express property suffix after sequence | 3 |
| 17 | `#=#` (followed-by non-overlapping) | 16.12.9 | Cannot express delayed property suffix | 3 |
| 18 | Property `case` | 16.12.16 | No multiway property branching | 3 |
| 19 | `sync_accept_on` | 16.12.14 | No synchronous accept abort | 3 |
| 20 | `sync_reject_on` | 16.12.14 | No synchronous reject abort | 3 |
| 21 | `$` in delay/repetition ranges | 16.7, 16.9.2 | Cannot express unbounded maximum | 4 |
| 22 | `##[*]` / `##[+]` | 16.7 | No unbounded delay shorthand | 4 |
| 23 | `[*]` / `[+]` | 16.9.2 | No unbounded repetition shorthand | 4 |
| 24 | Sequence-level `and` | 16.9.5 | Boolean AND conflated with sequence AND | 5 |
| 25 | Sequence-level `or` | 16.9.7 | Boolean OR conflated with sequence OR | 5 |
| 26 | `intersect` length-matching | 16.9.6 | Current translation ignores length constraint | 6 |
| 27 | `throughout` desugaring | 16.9.9 | Not desugared to `(cond[*0:$]) intersect seq` | 6 |
| 28 | `within` desugaring | 16.9.10 | Not desugared properly | 6 |
| 29 | `assert property` directive | 16.14.1 | Implicit only; no explicit directive wrapper | 7 |
| 30 | `assume property` directive | 16.14.2 | No formal constraint generation | 7 |
| 31 | `cover property` directive | 16.14.3 | No reachability analysis | 7 |
| 32 | `cover sequence` directive | 16.14.3 | No sequence coverage with multiplicity | 7 |
| 33 | `restrict property` directive | 16.14.4 | No formal search space restriction | 7 |
| 34 | Action blocks (`$error`, `$info`, etc.) | 16.14.1 | Parser rejects severity tasks | 7 |
| 35 | Immediate assertions | 16.3 | Parser rejects `assert(expr)` | 7 |
| 36 | Deferred assertions | 16.4 | Parser rejects `assert #0(expr)` | 7 |
| 37 | Named sequence declarations | 16.8 | Cannot parse reusable SVA libraries | 8 |
| 38 | Named property declarations | 16.12 | Cannot parse property templates | 9 |
| 39 | Local variables in sequences | 16.10 | Cannot track data across pipeline stages | 10 |
| 40 | `default clocking` | 16.15-16 | Every assertion needs explicit clock | 11 |
| 41 | `default disable iff` | 16.15 | Every assertion needs explicit reset | 11 |
| 42 | Multi-clock sequences | 16.13 | Single clock domain only | 12 |
| 43 | Struct field access | 16.6 | Cannot express `req.addr` | 13 |
| 44 | Enum literals | 16.6 | Cannot express `state == ST_READ` | 13 |
| 45 | `$countbits` | 20.9 | Missing generalized bit counting | 13 |
| 46 | `$isunbounded` | 20.9 | Missing parameter bound check | 13 |
| 47 | `.triggered` endpoint method | 16.9.11 | Cannot compose sequences by completion | 14 |
| 48 | `.matched` endpoint method | 16.9.11 | Cannot detect sequence match | 14 |
| 49 | Recursive properties | 16.12.17 | No fixpoint resolution | 14 |
| 50 | Bitwise operators (`&`, `\|`, `^`, `~`) | 16.6 | Only logical ops in boolean expressions | 15 |
| 51 | Reduction operators (`&sig`, `\|sig`, `^sig`) | 16.6 | Cannot reduce vectors | 15 |
| 52 | Bit-select (`sig[7]`) | 16.6 | Cannot select individual bits | 15 |
| 53 | Part-select (`sig[7:0]`) | 16.6 | Cannot select bit ranges | 15 |
| 54 | Concatenation (`{a, b}`) | 16.6 | Cannot concatenate signals | 15 |
| 55 | `let` construct | 11.12 | Cannot define expression macros | 16 |
| 56 | Nonvacuous evaluation tracking | 16.14.8 | Cannot detect dead assertions | 17 |
| 57 | `const'` cast | 16.14.6.1 | Cannot freeze values at queue time | 18 |
| 58 | `dist` constraints | 16.14.2 | Cannot bias constrained random | 19 |
| 59 | Checkers (`checker...endchecker`) | 17.1-17.9 | Cannot parse verification building blocks | 20 |
| 60 | `rand` free variables | 17.7 | Cannot model nondeterministic inputs | 20 |
| 61 | `$inferred_clock` / `$inferred_disable` | 16.14.7 | No context inference | 20 |

### The Architectural Plan

21 sprints across 5 tiers. All sprints COMPLETE. 858 total SVA tests.

| Tier | Sprints | Focus | Status |
|---|---|---|---|
| Tier 1 | 1-4 | Complete the temporal core | COMPLETE |
| Tier 2 | 5-7 | Sequence semantics & directives | COMPLETE |
| Tier 3 | 8-11 | Structural SVA | COMPLETE |
| Tier 4 | 12-14 | Multi-clock & industrial | COMPLETE |
| Tier 5 | 15-21 | IEEE completeness | COMPLETE |

**858 total SVA tests** across 7 test files, 47 Z3 algebraic identity proofs.

---

## Part II: Tier 1 — Complete the Temporal Core

### Sprint 1: Property Connectives (IEEE 16.12.3-8)

**Why:** Property-level `not`, `implies`, and `iff` are distinct from boolean operators. `not property_expr` negates a temporal property evaluation. `p1 implies p2` is `not p1 or p2` at the property level (distinct from sequence `|->`). `p1 iff p2` is biconditional. IEEE 16.12.15 defines `not` as flipping the strength of a property (negating a weak property produces a strong one). Table 16-3 (p.422) gives precedence: `not` > `and` > `or` > `iff` > `until`/`implies` > `|->`/`|=>`.

**Files:** `sva_model.rs`, `sva_to_verify.rs`, `verify_to_kernel.rs`

**New `SvaExpr` variants:**
```rust
PropertyNot(Box<SvaExpr>),
PropertyImplies(Box<SvaExpr>, Box<SvaExpr>),
PropertyIff(Box<SvaExpr>, Box<SvaExpr>),
```

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `property_not_signal` | `not req` parses to `PropertyNot(Signal("req"))` |
| 2 | `property_not_temporal` | `not s_eventually req` negates entire temporal property |
| 3 | `property_not_implication` | `not (a \|-> b)` wraps implication in PropertyNot |
| 4 | `property_not_nested` | `not not p` double negation parses correctly |
| 5 | `property_not_roundtrip` | Parse → to_string → parse structural equality |
| 6 | `property_not_translate_basic` | `not req` at t=0 → `Not(Var("req@0"))` |
| 7 | `property_not_translate_temporal` | `not s_eventually p` → negation of disjunction |
| 8 | `property_not_strength_flip` | `not` on weak property produces strong result (IEEE 16.12.15) |
| 9 | `property_not_z3_double_neg` | `not not p` ≡ `p` verified via Z3 algebraic identity |
| 10 | `property_not_z3_demorgan` | `not (p and q)` ≡ `(not p) or (not q)` via Z3 |
| 11 | `property_implies_basic` | `req implies ack` parses to `PropertyImplies(req, ack)` |
| 12 | `property_implies_vs_sequence_impl` | `implies` keyword distinct from `\|->` operator in AST |
| 13 | `property_implies_roundtrip` | Parse → to_string → parse structural equality |
| 14 | `property_implies_translate` | `p implies q` → `Implies(translate(p), translate(q))` |
| 15 | `property_implies_vacuous_true` | `false implies anything` is vacuously true via Z3 |
| 16 | `property_implies_contrapositive` | `(p implies q)` ≡ `(not q implies not p)` via Z3 |
| 17 | `property_implies_z3_modus_ponens` | `(p and (p implies q)) implies q` tautology via Z3 |
| 18 | `property_iff_basic` | `req iff ack` parses to `PropertyIff(req, ack)` |
| 19 | `property_iff_roundtrip` | Parse → to_string → parse structural equality |
| 20 | `property_iff_translate` | `p iff q` → `And(Implies(p',q'), Implies(q',p'))` |
| 21 | `property_iff_symmetric` | `(p iff q)` ≡ `(q iff p)` via Z3 |
| 22 | `property_iff_reflexive` | `(p iff p)` is tautology via Z3 |
| 23 | `property_iff_transitive` | `((p iff q) and (q iff r)) implies (p iff r)` via Z3 |
| 24 | `property_precedence_not_binds_tight` | `not p and q` → `(not p) and q` not `not (p and q)` |
| 25 | `property_precedence_implies_vs_iff` | `p implies q iff r` → `p implies (q iff r)` (iff binds tighter per Table 16-3) |
| 26 | `property_connectives_kernel_encoding` | All 3 variants encode to kernel terms correctly |
| 27 | `property_connectives_regression` | All 230 existing SVA tests still pass |

---

### Sprint 2: LTL Temporal Operators — Always, Eventually, Until (IEEE 16.12.11-13)

**Why:** The full LTL operator set. We have `s_always` (strong unbounded) and `s_eventually` (strong unbounded) but are missing: `always` (weak unbounded), `always [m:n]` (weak bounded), `s_always [m:n]` (strong bounded, range must NOT use `$`), `eventually [m:n]` (weak bounded, range must be bounded), `s_eventually [m:n]` (strong bounded, range CAN use `$`), and all four `until` variants. The `until` operators encode "property p holds until property q becomes true" — foundational for every protocol: "request stays asserted until grant", "data remains stable until acknowledged."

**New `SvaExpr` variants:**
```rust
Always(Box<SvaExpr>),
AlwaysBounded { body: Box<SvaExpr>, min: u32, max: Option<u32> },
SAlwaysBounded { body: Box<SvaExpr>, min: u32, max: u32 },
EventuallyBounded { body: Box<SvaExpr>, min: u32, max: u32 },
SEventuallyBounded { body: Box<SvaExpr>, min: u32, max: Option<u32> },
Until { lhs: Box<SvaExpr>, rhs: Box<SvaExpr>, strong: bool, inclusive: bool },
```

**Translation semantics:**
- `always p` → `∀t ∈ [0, bound). p@t` (weak: passes if trace ends)
- `always [m:n] p @ t` → `∀i ∈ [m, n]. p@(t+i)` (weak: passes if not enough ticks)
- `s_always [m:n] p @ t` → `∀i ∈ [m, n]. p@(t+i)` (strong: all ticks must exist, NO `$`)
- `eventually [m:n] p @ t` → `∃i ∈ [m, n]. p@(t+i)` (weak, range must be bounded)
- `s_eventually [m:n] p @ t` → `∃i ∈ [m, min(n, bound)]. p@(t+i)` (strong, CAN use `$`)
- `p until q @ t` → weak non-overlapping: `∃k≥t. q@k ∧ ∀j∈[t,k). p@j` OR trace ends with p always true
- `p s_until q @ t` → strong non-overlapping: `∃k≥t. q@k ∧ ∀j∈[t,k). p@j` (q MUST happen)
- `p until_with q @ t` → weak overlapping: like until but `∀j∈[t,k]. p@j` (p holds at q's cycle too)
- `p s_until_with q @ t` → strong overlapping: q MUST happen, p holds through q's cycle

**RED tests (~45 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `always_unbounded_parse` | `always req` → `Always(Signal("req"))` |
| 2 | `always_bounded_parse` | `always [2:5] req` → `AlwaysBounded{req, 2, 5}` |
| 3 | `always_bounded_dollar_parse` | `always [2:$] req` → `AlwaysBounded{req, 2, None}` (weak allows $) |
| 4 | `s_always_bounded_parse` | `s_always [2:5] req` → `SAlwaysBounded{req, 2, 5}` |
| 5 | `s_always_bounded_dollar_rejected` | `s_always [2:$] req` is a PARSE ERROR (IEEE: s_always range must be bounded) |
| 6 | `always_unbounded_roundtrip` | Parse → to_string → parse |
| 7 | `always_bounded_roundtrip` | Parse → to_string → parse |
| 8 | `s_always_bounded_roundtrip` | Parse → to_string → parse |
| 9 | `always_vs_s_always` | `always` and `s_always` are distinct AST nodes |
| 10 | `always_unbounded_translate` | `always p` → conjunction over [0, bound) |
| 11 | `always_bounded_translate` | `always [2:5] p @ t=0` → `p@2 ∧ p@3 ∧ p@4 ∧ p@5` |
| 12 | `always_bounded_dollar_translate` | `always [2:$] p` → conjunction clamped to bound |
| 13 | `s_always_bounded_translate` | `s_always [2:5] p` → conjunction with strong (ticks must exist) |
| 14 | `s_always_bounded_vs_always_bounded` | s_always requires all ticks exist; always does not |
| 15 | `always_z3_tautology` | `always (a \|\| !a)` is tautology via Z3 |
| 16 | `always_z3_contradiction` | `always (a && !a)` is contradiction via Z3 |
| 17 | `eventually_bounded_parse` | `eventually [3:8] ack` parses correctly |
| 18 | `eventually_bounded_dollar_rejected` | `eventually [3:$] ack` is PARSE ERROR (weak eventually must be bounded) |
| 19 | `eventually_bounded_roundtrip` | Parse → to_string → parse |
| 20 | `eventually_bounded_translate` | `eventually [3:8] p @ t=0` → `p@3 ∨ p@4 ∨ ... ∨ p@8` |
| 21 | `s_eventually_bounded_parse` | `s_eventually [1:5] done` parses correctly |
| 22 | `s_eventually_bounded_dollar_parse` | `s_eventually [1:$] done` parses (strong eventually CAN use $) |
| 23 | `s_eventually_bounded_translate` | `s_eventually [1:5] p` → disjunction with strong guarantee |
| 24 | `until_basic_parse` | `req until ack` → `Until{req, ack, strong:false, inclusive:false}` |
| 25 | `s_until_parse` | `req s_until ack` → `Until{..., strong:true, inclusive:false}` |
| 26 | `until_with_parse` | `valid until_with done` → `Until{..., strong:false, inclusive:true}` |
| 27 | `s_until_with_parse` | `stable s_until_with ack` → `Until{..., strong:true, inclusive:true}` |
| 28 | `until_all_four_roundtrip` | All 4 variants round-trip through to_string → parse |
| 29 | `until_translate_weak_nonoverlap` | `p until q`: q may not appear; p holds at all ticks before q (exclusive) |
| 30 | `until_translate_strong_nonoverlap` | `p s_until q`: q MUST appear within bound |
| 31 | `until_translate_weak_overlap` | `p until_with q`: p holds at same tick as q |
| 32 | `until_translate_strong_overlap` | `p s_until_with q`: strong + p holds at q's tick |
| 33 | `until_z3_req_until_ack` | `req until ack` verified: req high until ack pulse |
| 34 | `until_z3_strong_liveness` | `s_until` fails if ack never arrives within bound |
| 35 | `until_z3_weak_passes_at_bound` | `until` passes if bound reached without violation |
| 36 | `until_with_z3_overlap_cycle` | `until_with`: p AND q both true at transition cycle |
| 37 | `until_z3_equiv_release` | `p until q` ≡ `q or (p and nexttime (p until q))` recursive equivalence |
| 38 | `until_z3_s_until_with_equiv` | `p s_until_with q` ≡ `strong(p[*1:$] ##0 q)` equivalence via Z3 |
| 39 | `until_nesting` | `(a until b) until c` parses and translates correctly |
| 40 | `until_with_implication` | `req \|-> (data_valid until_with ack)` end-to-end |
| 41 | `always_bounded_with_until` | `always [0:10] (req until ack)` composes correctly |
| 42 | `until_kernel_encoding` | All 4 until variants encode to kernel terms correctly |
| 43 | `until_implies_precedence` | `(p until q) implies r` precedence correct per Table 16-3 |
| 44 | `temporal_regression_s_always` | Existing s_always tests unchanged |
| 45 | `temporal_regression_s_eventually` | Existing s_eventually tests unchanged |

---

### Sprint 3: Strong/Weak Modality, Advanced Temporal & Sync Abort (IEEE 16.12.2, 16.12.9-10, 16.12.14, 16.12.16)

**Why:** `strong(seq)` / `weak(seq)` control liveness vs safety (16.12.2). `s_nexttime` requires the next tick exist (16.12.10). `#-#` / `#=#` are followed-by operators (16.12.9): duals of `|->` / `|=>`. Property `case` (16.12.16) is multiway branching. `sync_accept_on` / `sync_reject_on` (16.12.14) are synchronous abort operators (evaluate condition at clock ticks only, unlike async `accept_on`/`reject_on`).

**New `SvaExpr` variants:**
```rust
Strong(Box<SvaExpr>),
Weak(Box<SvaExpr>),
SNexttime(Box<SvaExpr>, u32),
FollowedBy { antecedent: Box<SvaExpr>, consequent: Box<SvaExpr>, overlapping: bool },
PropertyCase { expression: Box<SvaExpr>, items: Vec<(Vec<SvaExpr>, Box<SvaExpr>)>, default: Option<Box<SvaExpr>> },
SyncAcceptOn { condition: Box<SvaExpr>, body: Box<SvaExpr> },
SyncRejectOn { condition: Box<SvaExpr>, body: Box<SvaExpr> },
```

**Translation:**
- `strong(seq)` → existential: a match endpoint MUST exist within bound
- `weak(seq)` → no match needed if bound exhausted (safety)
- `s_nexttime p` → `p@(t+1)` with strong (t+1 must exist within bound)
- `seq #-# prop` → `not (seq |-> not prop)` (IEEE p.430: dual of implication)
- `seq #=# prop` → `not (seq |=> not prop)` (dual of non-overlapping implication)
- `case(expr)` → nested if-else chain on expression equality
- `sync_accept_on(c) p` → like `accept_on` but `c` evaluated at clock ticks only
- `sync_reject_on(c) p` → like `reject_on` but `c` evaluated at clock ticks only

**RED tests (~38 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `strong_sequence_parse` | `strong(req ##1 ack)` → `Strong(Delay{...})` |
| 2 | `weak_sequence_parse` | `weak(req ##1 ack)` → `Weak(Delay{...})` |
| 3 | `strong_weak_roundtrip` | Both round-trip through to_string → parse |
| 4 | `strong_translate_must_complete` | `strong(req ##[1:5] ack)`: match must exist within bound |
| 5 | `weak_translate_may_not_complete` | `weak(req ##[1:5] ack)`: passes if bound reached without match |
| 6 | `strong_z3_incomplete_fails` | Strong sequence that cannot complete → property fails |
| 7 | `weak_z3_incomplete_passes` | Weak sequence where trace ends early → property passes |
| 8 | `strong_vs_weak_same_seq` | Same sequence: strong fails where weak passes at bound boundary |
| 9 | `strong_first_match_equiv` | `strong(seq)` ≡ `strong(first_match(seq))` via Z3 (IEEE 16.12.2) |
| 10 | `weak_first_match_equiv` | `weak(seq)` ≡ `weak(first_match(seq))` via Z3 (IEEE 16.12.2) |
| 11 | `strong_in_cover` | `cover property` context: sequence_expr is strong by default |
| 12 | `weak_in_assert` | `assert property` context: sequence_expr is weak by default |
| 13 | `s_nexttime_parse` | `s_nexttime req` → `SNexttime(req, 1)` |
| 14 | `s_nexttime_n_parse` | `s_nexttime[3] req` → `SNexttime(req, 3)` |
| 15 | `s_nexttime_zero_parse` | `s_nexttime[0] req` → alignment operator (acts at current tick) |
| 16 | `s_nexttime_roundtrip` | Parse → to_string → parse |
| 17 | `s_nexttime_translate` | `s_nexttime p` requires t+1 exists within bound |
| 18 | `s_nexttime_vs_nexttime` | `s_nexttime` fails at boundary; `nexttime` passes |
| 19 | `s_nexttime_z3_boundary` | At bound-1, `s_nexttime` fails; `nexttime` vacuously true |
| 20 | `followed_by_overlap_parse` | `req ##1 ack #-# done` → `FollowedBy{overlapping: true}` |
| 21 | `followed_by_nonoverlap_parse` | `req ##1 ack #=# done` → `FollowedBy{overlapping: false}` |
| 22 | `followed_by_roundtrip` | Both forms round-trip |
| 23 | `followed_by_overlap_translate` | Property suffix starts at sequence match endpoint |
| 24 | `followed_by_nonoverlap_translate` | Property suffix starts one cycle after match endpoint |
| 25 | `followed_by_is_dual` | `seq #-# prop` ≡ `not (seq \|-> not prop)` via Z3 (IEEE p.430) |
| 26 | `followed_by_nonoverlap_is_dual` | `seq #=# prop` ≡ `not (seq \|=> not prop)` via Z3 |
| 27 | `followed_by_vs_implication` | `#-#` and `\|->` produce different results on same inputs |
| 28 | `followed_by_with_always` | `##[0:5] done #-# always !rst` (IEEE p.430 example) |
| 29 | `property_case_basic_parse` | `case(delay) 2'd0: a && b; 2'd1: a ##2 b; default: 0; endcase` |
| 30 | `property_case_roundtrip` | Parse → to_string → parse |
| 31 | `property_case_translate` | Case lowered to nested if-else |
| 32 | `property_case_z3_select` | Case with known state selects correct branch via Z3 |
| 33 | `property_case_no_default_vacuous` | No default + no match → vacuously true (IEEE p.439) |
| 34 | `sync_accept_on_parse` | `sync_accept_on(done) req \|=> ack` parses |
| 35 | `sync_reject_on_parse` | `sync_reject_on(error) req \|=> ack` parses |
| 36 | `sync_vs_async_accept` | `sync_accept_on` and `accept_on` are distinct AST nodes |
| 37 | `sync_vs_async_reject` | `sync_reject_on` and `reject_on` are distinct AST nodes |
| 38 | `sync_abort_roundtrip` | All 4 abort operators (async+sync × accept+reject) round-trip |
| 39 | `sync_abort_translate` | Sync abort condition sampled at clock tick only |
| 40 | `sync_reject_equiv_throughout` | `sync_reject_on(stop)` ≡ `!stop throughout` in single-clock (IEEE p.437) |
| 41 | `sprint3_regression` | All existing tests unchanged |

---

### Sprint 4: Unbounded Sequence Operators (IEEE 16.7, 16.9.2)

**Why:** `$` denotes finite-but-unbounded maximum. `##[1:$]` = "delay of one or more". `[*1:$]` = "one or more repetitions". `##[*]` ≡ `##[0:$]`, `##[+]` ≡ `##[1:$]`, `[*]` ≡ `[*0:$]`, `[+]` ≡ `[*1:$]`. Essential for "req stays high for one or more cycles" and "ack will eventually arrive."

**Extend existing variants:**
```rust
Delay { body: Box<SvaExpr>, min: u32, max: Option<u32> },   // max=None means $
Repetition { body: Box<SvaExpr>, min: u32, max: Option<u32> },
```

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `delay_dollar_parse` | `##[1:$] ack` → `Delay{body: ack, min: 1, max: None}` |
| 2 | `delay_star_parse` | `##[*]` → `Delay{min: 0, max: None}` |
| 3 | `delay_plus_parse` | `##[+]` → `Delay{min: 1, max: None}` |
| 4 | `rep_dollar_parse` | `req[*1:$]` → `Repetition{body: req, min: 1, max: None}` |
| 5 | `rep_star_parse` | `req[*]` → `Repetition{min: 0, max: None}` |
| 6 | `rep_plus_parse` | `req[+]` → `Repetition{min: 1, max: None}` |
| 7 | `dollar_roundtrip_delay` | `##[1:$]` round-trips correctly ($ preserved in to_string) |
| 8 | `dollar_roundtrip_rep` | `req[*1:$]` round-trips correctly |
| 9 | `star_roundtrip` | `##[*]` and `[*]` round-trip |
| 10 | `plus_roundtrip` | `##[+]` and `[+]` round-trip |
| 11 | `delay_dollar_translate_bound5` | `##[1:$] p` at bound=5 → `p@1 ∨ p@2 ∨ p@3 ∨ p@4 ∨ p@5` |
| 12 | `delay_dollar_translate_bound1` | `##[1:$] p` at bound=1 → `p@1` (clamped) |
| 13 | `rep_star_translate` | `req[*]` → zero or more reps up to bound |
| 14 | `rep_plus_translate` | `req[+]` → one or more reps up to bound |
| 15 | `rep_star_includes_zero` | `req[*]` matches empty sequence (0 reps) |
| 16 | `rep_plus_excludes_zero` | `req[+]` requires at least 1 rep |
| 17 | `dollar_in_implication` | `req \|-> ##[1:$] ack` — req implies eventual ack |
| 18 | `dollar_throughout` | `valid throughout (##[1:$] done)` — valid held throughout |
| 19 | `dollar_z3_eventually` | `$rose(req) \|-> ##[1:$] ack` verified at bound=10 |
| 20 | `dollar_z3_rep` | `req[*1:$] ##1 !req` — high for N cycles then drops |
| 21 | `dollar_z3_star_vs_plus` | `[*]` matches at min=0; `[+]` does not |
| 22 | `dollar_with_goto` | `req[->1:$]` — unbounded goto repetition |
| 23 | `dollar_with_nonconsec` | `req[=1:$]` — unbounded non-consecutive |
| 24 | `existing_finite_unchanged` | `Delay{min:1, max: Some(5)}` behavior unchanged |
| 25 | `sprint4_regression` | All existing delay/repetition tests unchanged |

**Tier 1 total: ~133 tests.**

---

## Part III: Tier 2 — Sequence Semantics & Directives

### Sprint 5: Sequence-Level AND & OR (IEEE 16.9.5, 16.9.7)

**Why:** Sequence `and` / `or` have thread semantics distinct from boolean `&&` / `||`. Sequence `and`: both operands start at the same time, both must match, composite ends at whichever finishes LAST. Sequence `or`: at least one matches, composite match set is union. Our current `And`/`Or` are purely boolean — `a and b` where a and b are multi-cycle sequences would be incorrectly handled.

**New `SvaExpr` variants:**
```rust
SequenceAnd(Box<SvaExpr>, Box<SvaExpr>),
SequenceOr(Box<SvaExpr>, Box<SvaExpr>),
```

**Translation infrastructure — SequenceMatch:**
```rust
struct SequenceMatch {
    condition: BoundedExpr,
    length: u32,
}

fn translate_sequence(&mut self, expr: &SvaExpr, t: u32) -> Vec<SequenceMatch>;
```

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `seq_and_parse` | `(a ##2 b) and (c ##3 d)` → `SequenceAnd(...)` |
| 2 | `seq_or_parse` | `(a ##2 b) or (c ##3 d)` → `SequenceOr(...)` |
| 3 | `seq_and_vs_bool_and` | `a and b` (seq) vs `a && b` (bool) are distinct AST nodes |
| 4 | `seq_or_vs_bool_or` | `a or b` (seq) vs `a \|\| b` (bool) are distinct |
| 5 | `seq_and_roundtrip` | Parse → to_string → parse |
| 6 | `seq_or_roundtrip` | Parse → to_string → parse |
| 7 | `seq_and_same_length` | `(a ##2 b) and (c ##2 d)` — both length 2, composite length 2 |
| 8 | `seq_and_different_length` | `(a ##1 b) and (c ##3 d)` — composite ends at max(2, 4) = 4 |
| 9 | `seq_and_one_fails` | If either operand fails to match, composite AND fails |
| 10 | `seq_or_either_matches` | If either operand matches, composite OR succeeds |
| 11 | `seq_or_both_match` | Both match → two match endpoints in composite |
| 12 | `seq_and_translate_thread` | Thread semantics: both start at t, composite at max endpoint |
| 13 | `seq_or_translate_union` | Union semantics: match at either endpoint |
| 14 | `seq_and_with_repetition` | `(a[*3]) and (b ##2 c)` — compose multi-cycle sequences |
| 15 | `seq_and_z3_both_phases` | Handshake AND data phase both complete |
| 16 | `seq_or_z3_either_path` | Either fast or slow path succeeds |
| 17 | `seq_and_bool_shortcut` | When both are pure expressions, `and` ≡ `&&` |
| 18 | `seq_or_bool_shortcut` | When both are pure expressions, `or` ≡ `\|\|` |
| 19 | `intersect_vs_and` | `intersect` requires same length; `and` allows different |
| 20 | `seq_and_in_implication` | `(req and valid) \|-> ack` — seq and as antecedent |
| 21 | `seq_and_fig16_5` | IEEE Figure 16-5 (p.403) exact scenario reproduced |
| 22 | `seq_and_fig16_6` | IEEE Figure 16-6 (p.404) with time range reproduced |
| 23 | `seq_or_fig16_9` | IEEE Figure 16-9 (p.405) exact scenario |
| 24 | `seq_or_fig16_10` | IEEE Figure 16-10 (p.407) with sequences |
| 25 | `sprint5_regression` | All existing tests unchanged |

---

### Sprint 6: Intersect Length-Matching & Desugaring (IEEE 16.9.6, 16.9.9, 16.9.10)

**Why:** Our current `Intersect` translates to naive `And`, ignoring the IEEE-mandated length constraint. `intersect` requires both sequences to match AND have the SAME match length. `throughout` desugars to `(cond[*0:$]) intersect seq`. `within` desugars to `(1[*0:$] ##1 seq1 ##1 1[*0:$]) intersect seq2`. `first_match` takes only the earliest-completing match of a sequence.

**RED tests (~22 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `intersect_length_must_match` | `(a ##2 b) intersect (c ##2 d)` — both length 2, OK |
| 2 | `intersect_length_mismatch_never` | `(a ##1 b) intersect (c ##3 d)` — never matches (lengths 2 vs 4) |
| 3 | `intersect_range_selects_common` | `(a ##[1:5] b) intersect (c ##2 d)` — only length-3 match from first |
| 4 | `intersect_z3_length_enforced` | Z3 confirms length constraint holds |
| 5 | `intersect_vs_seq_and` | Same sequences: intersect ≠ and when lengths differ |
| 6 | `intersect_fig16_8` | IEEE Figure 16-8 (p.405) exact scenario |
| 7 | `throughout_desugaring_correct` | `valid throughout (##2 (trdy==0 && irdy==0) [*7])` desugared per IEEE |
| 8 | `throughout_cond_every_cycle` | Condition checked at EVERY cycle of sequence |
| 9 | `throughout_z3_burst` | Burst protocol: mode held throughout transfer |
| 10 | `throughout_cond_fails_mid` | Condition drops mid-sequence → throughout fails |
| 11 | `throughout_fig16_12` | IEEE Figure 16-12 (p.410) exact failure scenario |
| 12 | `throughout_fig16_13` | IEEE Figure 16-13 (p.410) exact success scenario |
| 13 | `within_desugaring_correct` | `seq1 within seq2` desugared per IEEE 16.9.10 |
| 14 | `within_boundaries_enforced` | Inner start ≥ outer start, inner end ≤ outer end |
| 15 | `within_z3_subinterval` | Inner occurs within outer's window |
| 16 | `within_too_long_fails` | Inner longer than outer → cannot match |
| 17 | `first_match_earliest` | `first_match(a ##[1:5] b)` — only earliest delay taken |
| 18 | `first_match_z3_priority` | Z3 confirms only first match selected |
| 19 | `first_match_no_match_propagates` | No underlying match → first_match no match |
| 20 | `first_match_with_seq_and` | `first_match(seq1 and seq2)` — first of composite |
| 21 | `first_match_fig_example` | IEEE p.408 ts2 example: two possible matches, earliest selected |
| 22 | `sprint6_regression` | All existing intersect/throughout/within tests unchanged |

---

### Sprint 7: Assertion Directives, Action Blocks & Immediate/Deferred Assertions (IEEE 16.2-4, 16.14)

**Why:** IEEE 16.14 defines five concurrent assertion directives: `assert property`, `assume property`, `cover property`, `cover sequence`, `restrict property`. Additionally, `assert(expr)` (16.3) and `assert #0(expr)` / `assert final(expr)` (16.4) are immediate and deferred assertions. Action blocks (`$error`, `$info`, `$fatal`, `$warning`, `$display`) must be parsed. `cover sequence` (16.14.3) is distinct from `cover property`: it counts ALL sequence matches per evaluation with multiplicity.

**New types:**
```rust
pub enum SvaDirectiveKind { Assert, Assume, Cover, CoverSequence, Restrict }
pub enum ImmediateKind { Simple, DeferredObserved, DeferredFinal }

pub struct SvaDirective {
    pub kind: SvaDirectiveKind,
    pub property: SvaExpr,
    pub label: Option<String>,
    pub clock: Option<ClockEdge>,
    pub disable_iff: Option<SvaExpr>,
    pub action_pass: Option<String>,
    pub action_fail: Option<String>,
}

pub struct ImmediateAssertion {
    pub kind: ImmediateKind,        // simple, #0, final
    pub directive: ImmediateDirectiveKind,  // assert, assume, cover
    pub expression: SvaExpr,
    pub action_pass: Option<String>,
    pub action_fail: Option<String>,
}
```

**RED tests (~35 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `directive_assert_parse` | `assert property (@(posedge clk) req \|=> ack);` |
| 2 | `directive_assume_parse` | `assume property (@(posedge clk) !rst);` |
| 3 | `directive_cover_property_parse` | `cover property (@(posedge clk) req ##1 ack);` |
| 4 | `directive_cover_sequence_parse` | `cover sequence (@(posedge clk) req ##1 ack);` |
| 5 | `directive_restrict_parse` | `restrict property (@(posedge clk) valid);` |
| 6 | `directive_with_label` | `a1: assert property (p);` captures label "a1" |
| 7 | `directive_with_disable_iff` | `assert property (@(posedge clk) disable iff (rst) p);` |
| 8 | `directive_all_five_roundtrip` | All 5 directive kinds round-trip |
| 9 | `action_block_pass_fail` | `assert property (p) $info("pass"); else $error("fail");` parsed |
| 10 | `action_block_pass_only` | `cover property (p) $display("covered");` parsed |
| 11 | `action_block_fail_only` | `assert property (p) else $error("failed");` parsed |
| 12 | `action_block_fatal` | `else $fatal(1, "critical");` parsed |
| 13 | `action_block_warning` | `else $warning("check");` parsed |
| 14 | `action_block_ignored_in_formal` | Action blocks present in AST but excluded from Z3 encoding |
| 15 | `assume_adds_constraint` | `assume property(p)` adds p as solver constraint |
| 16 | `assume_z3_constrains_space` | Assume restricts reachable states in Z3 check |
| 17 | `cover_checks_reachability` | `cover property(p)` → SAT check (not UNSAT) |
| 18 | `cover_sat_means_reachable` | Reachable property → cover SAT with witness |
| 19 | `cover_unsat_means_unreachable` | Unreachable property → cover UNSAT |
| 20 | `cover_sequence_multiplicity` | Cover sequence counts ALL matches, not just first |
| 21 | `cover_sequence_vs_cover_property` | Distinct semantics: sequence counts matches, property counts attempts |
| 22 | `restrict_constrains_like_assume` | `restrict property(p)` behaves like assume in formal |
| 23 | `restrict_no_action_block` | `restrict property` has no action block (IEEE 16.14.4) |
| 24 | `assume_assert_interaction` | `assume !rst` + `assert req \|=> ack` — assume constrains assert |
| 25 | `multiple_directives` | Multiple assert/assume/cover in one context |
| 26 | `immediate_assert_parse` | `assert(a && b)` → `ImmediateAssertion{Simple, Assert, ...}` |
| 27 | `immediate_assume_parse` | `assume(req \|\| ack)` → `ImmediateAssertion{Simple, Assume, ...}` |
| 28 | `immediate_cover_parse` | `cover(hit)` → `ImmediateAssertion{Simple, Cover, ...}` |
| 29 | `deferred_assert_zero` | `assert #0(a == b)` → `ImmediateAssertion{DeferredObserved, ...}` |
| 30 | `deferred_assert_final` | `assert final(a == b)` → `ImmediateAssertion{DeferredFinal, ...}` |
| 31 | `deferred_assume_zero` | `assume #0(valid)` parses |
| 32 | `deferred_cover_zero` | `cover #0(hit)` parses |
| 33 | `deferred_cover_final` | `cover final(hit)` parses |
| 34 | `immediate_deferred_roundtrip` | All immediate/deferred forms round-trip |
| 35 | `immediate_formal_translation` | Immediate assert → combinational check at each timestep |
| 36 | `sprint7_regression` | All existing tests unchanged |

**Tier 2 total: ~82 tests.**

---

## Part IV: Tier 3 — Structural SVA

### Sprint 8: Named Sequence Declarations (IEEE 16.8)

**Why:** Real SVA libraries consist of parameterized named sequences. `sequence s(a, b); a ##1 b; endsequence` with instantiation `s(req, ack)`. Formal arguments can be typed (`bit`, `sequence`, `untyped`) with optional defaults. A resolution pass substitutes actual arguments for formal parameters.

**RED tests (~28 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `seq_decl_no_ports` | `sequence s; a ##1 b; endsequence` parses |
| 2 | `seq_decl_with_ports` | `sequence s(a, b); a ##1 b; endsequence` captures 2 ports |
| 3 | `seq_decl_typed_port_bit` | `sequence s(bit a, bit b); ...` typed ports |
| 4 | `seq_decl_typed_port_sequence` | `sequence s(sequence a, untyped b); ...` |
| 5 | `seq_decl_default_arg` | `sequence s(a, b = 1'b1); ...` default value |
| 6 | `seq_decl_with_clock` | `sequence s; @(posedge clk) a ##1 b; endsequence` |
| 7 | `seq_decl_with_local_vars` | `sequence s; int v; (a, v = data) ##1 (b && out == v); endsequence` |
| 8 | `seq_decl_endlabel` | `endsequence : s` optional end label |
| 9 | `seq_instance_positional` | `s(req, ack)` → `SequenceInstance{name: "s", args: [req, ack]}` |
| 10 | `seq_instance_named` | `s(.a(req), .b(ack))` named argument binding |
| 11 | `seq_decl_roundtrip` | Declaration round-trips |
| 12 | `seq_instance_roundtrip` | Instance round-trips |
| 13 | `resolve_simple` | `s(req, ack)` with `s(a,b) = a ##1 b` → `req ##1 ack` |
| 14 | `resolve_default_used` | Omitted arg uses default: `s(req)` with `s(a, b=1'b1)` |
| 15 | `resolve_default_overridden` | Explicit arg overrides default |
| 16 | `resolve_nested` | Instance inside another sequence body resolves transitively |
| 17 | `resolve_3_level_deep` | `s1` → `s2` → `s3` three-level resolution |
| 18 | `resolve_cyclic_error` | `s1` instantiates `s2`, `s2` instantiates `s1` → error |
| 19 | `resolve_missing_decl_error` | Undeclared sequence → error |
| 20 | `resolve_arity_mismatch_error` | Wrong arg count → error |
| 21 | `resolve_typed_arg_mismatch` | Expression passed to `sequence` typed port → error |
| 22 | `resolve_dollar_as_arg` | `$` passed as actual argument to untyped formal in range |
| 23 | `resolve_preserves_clock` | Clock annotation preserved through resolution |
| 24 | `resolve_in_implication` | `s(req, ack) \|-> done` resolved correctly |
| 25 | `resolve_multiple_decls` | Multiple sequence declarations coexist |
| 26 | `resolve_end_to_end_z3` | Declaration + instance + resolution + translation + Z3 |
| 27 | `resolve_ieee_example_rule` | IEEE p.385 `rule` sequence with instantiation of `s` |
| 28 | `sprint8_regression` | All existing tests unchanged |

---

### Sprint 9: Named Property Declarations (IEEE 16.12)

**Why:** Properties can be parameterized and instantiated. `property p(a, b); a |-> ##1 b; endproperty`. Property ports can be typed as `property` or `sequence`. Recursive properties (16.12.17) allow self-referencing with 4 restrictions. Resolution pass handles both sequence and property instantiation.

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `prop_decl_basic` | `property p; a \|-> b; endproperty` |
| 2 | `prop_decl_with_ports` | `property p(a, b); a \|-> ##1 b; endproperty` |
| 3 | `prop_decl_with_clock` | `property p; @(posedge clk) req \|=> ack; endproperty` |
| 4 | `prop_decl_with_disable` | `property p; @(posedge clk) disable iff (rst) ...; endproperty` |
| 5 | `prop_decl_seq_typed_arg` | `property p(sequence s); s \|-> ack; endproperty` |
| 6 | `prop_decl_prop_typed_arg` | `property p(property q); req \|-> q; endproperty` |
| 7 | `prop_decl_endlabel` | `endproperty : p` optional end label |
| 8 | `prop_instance_in_assert` | `assert property (p(req, ack));` |
| 9 | `prop_decl_roundtrip` | Parse → to_string → parse |
| 10 | `prop_instance_roundtrip` | Instance round-trips |
| 11 | `resolve_prop_basic` | Property instance resolved to body with arg substitution |
| 12 | `resolve_prop_with_seq_instance` | Property body contains sequence instance, both resolved |
| 13 | `resolve_prop_default_arg` | Default property argument |
| 14 | `resolve_prop_seq_typed` | `sequence` typed arg accepts sequence expressions |
| 15 | `resolve_prop_nested_prop` | Property instantiates another property |
| 16 | `resolve_prop_end_to_end_z3` | Full pipeline: decl + instance + resolve + translate + Z3 |
| 17 | `recursive_prop_detected` | Self-referencing property identified during resolution |
| 18 | `recursive_prop_bounded_unroll` | Recursive property unrolled up to bound |
| 19 | `recursive_prop_always` | `prop_always(p): p and (1'b1 \|=> prop_always(p))` (IEEE p.439) |
| 20 | `recursive_prop_weak_until` | `prop_weak_until(p,q): q or (p and (1'b1 \|=> prop_weak_until(p,q)))` (IEEE p.440) |
| 21 | `recursive_restriction1` | `not` on recursive property → error (IEEE Restriction 1) |
| 22 | `recursive_restriction2` | `disable iff` in recursive property → error (Restriction 2) |
| 23 | `recursive_restriction3` | No time advance between recursive calls → error (Restriction 3) |
| 24 | `recursive_mutual` | Mutually recursive: `check_phase1` ↔ `check_phase2` (IEEE p.440) |
| 25 | `sprint9_regression` | All existing tests unchanged |

---

### Sprint 10: Local Variables in Sequences and Properties (IEEE 16.10)

**Why:** Local variables capture data at specific points and assert against it later. THE critical feature for data integrity: `int v; (valid_in, v = data_in) |-> ##5 (data_out == v)`. The translation must thread state: when `v` is assigned at time t, subsequent refs resolve to `data_in@t`. Local variables flow through `or` (must be assigned in both branches), are blocked through `and`/`intersect` (assigned in both → conflict), and can be accumulated in repetitions.

**RED tests (~30 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `local_var_decl_int` | `int v;` inside sequence parses |
| 2 | `local_var_decl_logic` | `logic [7:0] data;` inside sequence parses |
| 3 | `local_var_decl_bit` | `bit flag;` inside sequence parses |
| 4 | `local_var_decl_initialized` | `int v = 0;` with initial value |
| 5 | `local_var_assign_at_match` | `(req, v = data_in)` → `SequenceAction{expr, assignments}` |
| 6 | `local_var_multiple_assigns` | `(a, v = e, w = f)` two assignments at one match |
| 7 | `local_var_ref_in_expr` | `data_out == v` where v is local variable |
| 8 | `local_var_roundtrip` | Declaration + assignment + reference round-trip |
| 9 | `local_var_pipeline_5stage` | `(valid_in, v = data_in) \|-> ##5 (data_out == v)` |
| 10 | `local_var_translate_threading` | Assignment at t → ref at t+5 uses `data_in@t` |
| 11 | `local_var_z3_pipeline_pass` | Z3 verifies 5-stage pipeline data integrity |
| 12 | `local_var_z3_pipeline_fail` | Corrupted pipeline → counterexample shows mismatch |
| 13 | `local_var_reassignment` | `(a, v = x) ##1 (b, v = y)` — v updated to y |
| 14 | `local_var_operator_assign` | `(a, v += data)` operator assignment |
| 15 | `local_var_inc_dec` | `(a, v++)` and `(a, v--)` |
| 16 | `local_var_accumulator` | `($rose(a), x = 1) ##1 (a, x++)[*0:$] ##1 (!a && x <= MAX)` |
| 17 | `local_var_scope_confined` | Variable not visible outside declaring seq/prop |
| 18 | `local_var_in_property` | `property p; int x; (req, x = data) \|-> ##1 (ack && out == x); endproperty` |
| 19 | `local_var_goto_rep` | `(a[->1], x = e)` assignment at goto match point |
| 20 | `local_var_uninitialized` | Uninitialized var → unassigned at start |
| 21 | `local_var_ref_before_assign_error` | Reference before assignment → error |
| 22 | `local_var_or_branch_flow` | `or` branches: var flows out only if assigned in BOTH |
| 23 | `local_var_and_branch_flow` | `and`/`intersect` branches: var assigned in both → blocked from flowing |
| 24 | `local_var_ieee_data_check` | IEEE p.415 `data_check` sequence reproduced |
| 25 | `local_var_ieee_data_check_p` | IEEE p.415 `data_check_p` property reproduced |
| 26 | `local_var_ieee_count_cycles` | IEEE p.416 `count_a_cycles` accumulator |
| 27 | `local_var_ieee_rep_v` | IEEE p.415-416 `rep_v` sequence with goto+accumulation |
| 28 | `local_var_axi_wdata` | AXI WDATA captured at WVALID, checked at BVALID |
| 29 | `local_var_end_to_end_z3` | Full: parse → resolve → translate → Z3 |
| 30 | `sprint10_regression` | All existing tests unchanged |

---

### Sprint 11: Resolution Pass — Default Clocking & Disable Iff (IEEE 16.15-16)

**Why:** `default clocking` and `default disable iff` apply to all assertions in scope that lack explicit clock/reset. This is how most real SVA is written. Nested scopes override outer defaults (IEEE p.474). `$inferred_clock` and `$inferred_disable` are context value functions for querying the resolved defaults.

**RED tests (~18 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `default_clocking_parse` | `default clocking @(posedge clk); endclocking` |
| 2 | `default_clocking_negedge` | `default clocking @(negedge clk1); endclocking` |
| 3 | `default_disable_parse` | `default disable iff rst1;` |
| 4 | `default_disable_expression` | `default disable iff (rst \|\| emergency);` |
| 5 | `elaborate_adds_clock` | Bare `assert property (req \|=> ack);` gets default clock |
| 6 | `elaborate_adds_disable` | Bare assertion gets default disable iff |
| 7 | `elaborate_adds_both` | Both clock and disable applied |
| 8 | `elaborate_explicit_overrides_clock` | Explicit `@(posedge clk2)` overrides default |
| 9 | `elaborate_explicit_overrides_disable` | Explicit `disable iff (rst2)` overrides default |
| 10 | `elaborate_no_default_unchanged` | No default → bare assertion stays bare |
| 11 | `elaborate_nested_scope_overrides` | Inner module default overrides outer (IEEE p.474) |
| 12 | `elaborate_multiple_defaults_error` | Two `default disable iff` in same scope → error |
| 13 | `elaborate_resolves_seq_instances` | Sequence instances resolved during elaboration |
| 14 | `elaborate_resolves_prop_instances` | Property instances resolved during elaboration |
| 15 | `elaborate_resolves_let_instances` | Let instances resolved during elaboration |
| 16 | `elaborate_end_to_end_z3` | Default clock + disable + named seq → fully elaborated → Z3 |
| 17 | `elaborate_ieee_p469_example` | IEEE p.469 module m with default clocking/disable reproduced |
| 18 | `sprint11_regression` | All existing tests unchanged |

**Tier 3 total: ~101 tests.**

---

## Part V: Tier 4 — Multi-Clock & Industrial

### Sprint 12: Multi-Clock Sequences (IEEE 16.13)

**Why:** SVA allows sequences spanning clock domains: `@(posedge clk1) req |=> @(posedge clk2) ack`. In bounded model checking, each clock becomes a separate boolean signal with its own tick pattern.

**RED tests (~18 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `clocking_event_posedge` | `@(posedge clk) req ##1 ack` parses with clock |
| 2 | `clocking_event_negedge` | `@(negedge clk) req` parses |
| 3 | `clocking_event_edge` | `@(edge clk) req` parses |
| 4 | `clocking_event_roundtrip` | Parse → to_string → parse |
| 5 | `multi_clock_parse` | `@(posedge clk1) req \|=> @(posedge clk2) ack` |
| 6 | `multi_clock_in_sequence` | `@(posedge clk1) req ##1 @(posedge clk2) ack` |
| 7 | `multi_clock_translate` | Two clocks → two timestep domains |
| 8 | `multi_clock_z3_same_clock` | clk1 == clk2 → equivalent to single-clock |
| 9 | `multi_clock_z3_cross_domain` | Cross-clock property verified |
| 10 | `multi_clock_z3_cdc_violation` | CDC bug: value sampled on wrong clock → detected |
| 11 | `multi_clock_with_disable` | Multi-clock body with disable iff |
| 12 | `multi_clock_local_var_init` | Local var initialized per-clock (IEEE p.455-456) |
| 13 | `multi_clock_implication_semantics` | `\|=>` in multi-clock: consequent starts at next tick of consequent's clock |
| 14 | `global_clock_parse` | `@($global_clock) p` recognized |
| 15 | `multi_clock_property_decl` | Named property with multi-clock body |
| 16 | `multi_clock_triggered` | `.triggered` across clock domains |
| 17 | `clock_strip_single_unchanged` | Existing single-clock stripping still works |
| 18 | `sprint12_regression` | All existing tests unchanged |

---

### Sprint 13: Complex Data Types & Remaining System Functions (IEEE 16.6, 20.9)

**RED tests (~18 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `field_access_parse` | `req.addr` → `FieldAccess{req, "addr"}` |
| 2 | `field_access_nested` | `req.header.id` → nested field access |
| 3 | `field_access_roundtrip` | Round-trip |
| 4 | `field_access_translate` | Field access → bitvector extract |
| 5 | `enum_literal_parse` | `state_t::READ` or `ST_READ` |
| 6 | `enum_comparison` | `state == ST_READ` parses and translates |
| 7 | `countbits_parse` | `$countbits(sig, '0', '1')` |
| 8 | `countbits_roundtrip` | Round-trip |
| 9 | `countbits_equiv_countones` | `$countbits(sig, '1')` ≡ `$countones(sig)` via Z3 |
| 10 | `countbits_complement` | `$countbits(sig, '0') + $countbits(sig, '1') == $bits(sig)` via Z3 |
| 11 | `countbits_multi_control` | `$countbits(sig, '0', '1', 'x', 'z')` all control bits |
| 12 | `isunbounded_parse` | `$isunbounded(MAX_DELAY)` |
| 13 | `isunbounded_translate` | Evaluates to boolean constant |
| 14 | `struct_in_temporal` | `$rose(req) \|-> ##1 (resp.status == OK)` end-to-end |
| 15 | `enum_in_case` | `case(state) ST_IDLE: p; ST_READ: q; endcase` |
| 16 | `field_in_local_var` | `int v; (req, v = pkt.id) \|-> ##5 (resp.id == v)` |
| 17 | `array_index_in_sva` | `mem[addr] == expected` in assertion expression |
| 18 | `sprint13_regression` | All existing tests unchanged |

---

### Sprint 14: Endpoint Methods & Recursive Properties (IEEE 16.9.11, 16.12.17)

**RED tests (~15 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `triggered_parse` | `s.triggered` → `Triggered("s")` |
| 2 | `matched_parse` | `s.matched` → `Matched("s")` |
| 3 | `triggered_roundtrip` | Round-trip |
| 4 | `triggered_in_sequence` | `reset ##1 inst ##1 e1.triggered ##1 branch_back` |
| 5 | `triggered_translate` | Auxiliary boolean: true when sequence endpoint reached |
| 6 | `triggered_z3_detects_completion` | Completion of `$rose(ready) ##1 proc1 ##1 proc2` detected |
| 7 | `triggered_negated` | `!s.triggered` — sequence did NOT complete |
| 8 | `triggered_in_property` | `req \|-> ##[1:5] ack_seq.triggered` |
| 9 | `triggered_empty_match_excluded` | Empty match does NOT activate .triggered (IEEE p.412) |
| 10 | `triggered_ieee_e1_rule` | IEEE p.411-412 `e1` and `rule` example |
| 11 | `triggered_ieee_e2` | IEEE p.412 `e2(a,b,c)` with triggered |
| 12 | `matched_vs_triggered` | `.matched` semantic difference in multi-clock context |
| 13 | `triggered_with_local_var` | Local var passed through triggered instance (IEEE p.416-417) |
| 14 | `triggered_multiple_threads` | Multiple evaluation threads can trigger at same tick |
| 15 | `sprint14_regression` | All existing tests unchanged |

**Tier 4 total: ~51 tests.**

---

## Part VI: Tier 5 — IEEE Completeness

### Sprint 15: Bitwise Operators, Bit-Select, Part-Select, Concatenation (IEEE 16.6)

**Why:** Boolean expressions in assertions can contain ANY SystemVerilog expression (IEEE 16.6). Our parser only handles logical operators. Missing: bitwise `&`, `|`, `^`, `~`, reduction operators (`&sig`, `|sig`, `^sig`), bit-select `sig[7]`, part-select `sig[7:0]`, concatenation `{a, b}`. The `BoundedExpr` IR already has `BitVecBinary`, `BitVecExtract`, `BitVecConcat` — this is parser + AST work.

**New `SvaExpr` variants:**
```rust
BitAnd(Box<SvaExpr>, Box<SvaExpr>),
BitOr(Box<SvaExpr>, Box<SvaExpr>),
BitXor(Box<SvaExpr>, Box<SvaExpr>),
BitNot(Box<SvaExpr>),
ReductionAnd(Box<SvaExpr>),
ReductionOr(Box<SvaExpr>),
ReductionXor(Box<SvaExpr>),
BitSelect { signal: Box<SvaExpr>, index: Box<SvaExpr> },
PartSelect { signal: Box<SvaExpr>, high: u32, low: u32 },
Concat(Vec<SvaExpr>),
```

**RED tests (~28 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `bitwise_and_parse` | `a & b` → `BitAnd(a, b)` |
| 2 | `bitwise_or_parse` | `a \| b` → `BitOr(a, b)` |
| 3 | `bitwise_xor_parse` | `a ^ b` → `BitXor(a, b)` |
| 4 | `bitwise_not_parse` | `~a` → `BitNot(a)` |
| 5 | `bitwise_vs_logical_and` | `a & b` ≠ `a && b` in AST |
| 6 | `bitwise_vs_logical_or` | `a \| b` ≠ `a \|\| b` in AST |
| 7 | `bitwise_vs_logical_not` | `~a` ≠ `!a` in AST |
| 8 | `bitwise_or_vs_implication` | `a \| b` parses as BitOr, NOT as start of `\|->` |
| 9 | `reduction_and_parse` | `&data` → `ReductionAnd(data)` |
| 10 | `reduction_or_parse` | `\|data` → `ReductionOr(data)` |
| 11 | `reduction_xor_parse` | `^data` → `ReductionXor(data)` |
| 12 | `bit_select_parse` | `sig[7]` → `BitSelect{sig, Const(7)}` |
| 13 | `part_select_parse` | `sig[7:0]` → `PartSelect{sig, 7, 0}` |
| 14 | `concat_two_parse` | `{a, b}` → `Concat([a, b])` |
| 15 | `concat_three_parse` | `{a, b, c}` → `Concat([a, b, c])` |
| 16 | `concat_nested_parse` | `{a, {b, c}}` nested |
| 17 | `bitwise_all_roundtrip` | All bitwise ops round-trip |
| 18 | `select_roundtrip` | Bit-select and part-select round-trip |
| 19 | `concat_roundtrip` | Concatenation round-trips |
| 20 | `bitwise_and_translate` | `a & b` → `BitVecBinary{And, ...}` |
| 21 | `bitwise_or_translate` | `a \| b` → `BitVecBinary{Or, ...}` |
| 22 | `bitwise_xor_translate` | `a ^ b` → `BitVecBinary{Xor, ...}` |
| 23 | `part_select_translate` | `sig[7:0]` → `BitVecExtract{7, 0, ...}` |
| 24 | `concat_translate` | `{a, b}` → `BitVecConcat(a, b)` |
| 25 | `reduction_translate` | `&data` → ANDing all bits |
| 26 | `bitwise_in_assertion_z3` | `$rose(req) \|-> (data & mask) == expected` verified via Z3 |
| 27 | `bitwise_demorgan_z3` | `~(a & b)` ≡ `(~a \| ~b)` via Z3 |
| 28 | `sprint15_regression` | All existing tests unchanged |

---

### Sprint 16: `let` Construct (IEEE 11.12)

**Why:** `let` defines named expression substitution: `let ready_exp(irdy, trdy) = (irdy == 0) && ($fell(trdy));`. Unlike sequences, `let` is pure expression substitution with no temporal semantics. Used in checkers and properties for readability.

**RED tests (~18 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `let_decl_no_args` | `let ready = (irdy == 0);` |
| 2 | `let_decl_with_args` | `let ready(irdy, trdy) = (irdy == 0) && ($fell(trdy));` |
| 3 | `let_decl_typed_args` | `let mask(bit [7:0] data, int shift) = data >> shift;` |
| 4 | `let_instance_no_args` | `ready` as let instance |
| 5 | `let_instance_with_args` | `ready(sig_irdy, sig_trdy)` |
| 6 | `let_decl_roundtrip` | Round-trip |
| 7 | `let_instance_roundtrip` | Round-trip |
| 8 | `let_resolve_basic` | `ready(sig_irdy)` expanded to body with substitution |
| 9 | `let_resolve_multi_arg` | Multi-argument expansion |
| 10 | `let_resolve_nested` | Let body contains another let instance |
| 11 | `let_resolve_missing_error` | Undeclared let → error |
| 12 | `let_resolve_arity_error` | Wrong arg count → error |
| 13 | `let_in_assertion` | `assert property (ready(irdy) \|-> ack);` end-to-end |
| 14 | `let_in_sequence` | Let used in sequence body |
| 15 | `let_vs_sequence_decl` | Let is expression-level (no temporal); sequence is temporal |
| 16 | `let_in_checker` | Let construct inside checker body |
| 17 | `let_z3_end_to_end` | Parse → resolve (inline) → translate → Z3 |
| 18 | `sprint16_regression` | All existing tests unchanged |

---

### Sprint 17: Nonvacuous Evaluation Tracking (IEEE 16.14.8)

**Why:** IEEE defines 33 rules (a-ag) for nonvacuity. A vacuous success means the antecedent was never triggered — the property never actually tested the design. If ALL evaluation attempts are vacuous, the assertion is dead. This is a formal verification quality issue: dead assertions provide false confidence.

**Implementation:** New analysis module `sva_vacuity.rs`. No new AST nodes. Secondary analysis pass.

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `vacuity_sequence_always_nonvac` | A sequence is always nonvacuous (rule a) |
| 2 | `vacuity_strong_always_nonvac` | `strong(seq)` is always nonvacuous (rule b) |
| 3 | `vacuity_weak_always_nonvac` | `weak(seq)` is always nonvacuous (rule c) |
| 4 | `vacuity_not_propagates` | `not p` nonvacuous iff p is (rule d) |
| 5 | `vacuity_or_either` | `p or q` nonvacuous if either is (rule e) |
| 6 | `vacuity_and_either` | `p and q` nonvacuous if either is (rule f) |
| 7 | `vacuity_if_condition_true` | `if(cond) p` nonvacuous when cond true and p nonvacuous (rule g) |
| 8 | `vacuity_if_else` | `if(cond) p else q` nonvacuous in either branch (rule g) |
| 9 | `vacuity_implication_triggered` | `req \|-> ack` nonvacuous when req has endpoint match (rule h) |
| 10 | `vacuity_implication_untriggered` | `req \|-> ack` vacuous when req always false |
| 11 | `vacuity_nonoverlap_impl` | `req \|=> ack` nonvacuous when req has match point (rule i) |
| 12 | `vacuity_followed_by_overlap` | `seq #-# prop` nonvacuous iff seq has endpoint (rule j) |
| 13 | `vacuity_followed_by_nonoverlap` | `seq #=# prop` nonvacuous iff seq has match (rule k) |
| 14 | `vacuity_nexttime` | `nexttime p` nonvacuous iff next tick exists and p nonvacuous (rule l) |
| 15 | `vacuity_s_nexttime` | `s_nexttime p` nonvacuous iff next tick exists (rule n) |
| 16 | `vacuity_always` | `always p` nonvacuous when p nonvacuous at some tick (rule p) |
| 17 | `vacuity_always_bounded` | `always [m:n] p` nonvacuous at some tick in range (rule q) |
| 18 | `vacuity_s_always_bounded` | `s_always [m:n] p` nonvacuous at some tick (rule r) |
| 19 | `vacuity_s_eventually` | `s_eventually p` nonvacuous if p holds at some tick (rule s) |
| 20 | `vacuity_eventually_bounded` | `eventually [m:n] p` nonvacuous (rule u) |
| 21 | `vacuity_until` | `p until q` nonvacuous (rule v) |
| 22 | `vacuity_implies_property` | `p implies q` nonvacuous when p is true (rule z) |
| 23 | `vacuity_iff_property` | `p iff q` nonvacuous if either nonvacuous (rule aa) |
| 24 | `vacuity_disable_iff` | `disable iff (rst) p` nonvacuous when rst not always active (rule ag) |
| 25 | `vacuity_cover_check_z3` | Generate vacuity cover property, verify via Z3 |
| 26 | `vacuity_all_attempts_vacuous_warning` | All-vacuous → diagnostic warning |
| 27 | `sprint17_regression` | All existing tests unchanged |

---

### Sprint 18: `const'` Cast & Assertion Semantics (IEEE 16.14.6.1)

**Why:** `const'(expr)` freezes expression value at assertion queue time. Relevant for concurrent assertions in procedural `always` blocks: the procedural value at queue time may differ from the sampled value at evaluation time.

**RED tests (~15 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `const_cast_parse` | `const'(data)` → `ConstCast(Signal("data"))` |
| 2 | `const_cast_complex` | `const'(a + b)` → ConstCast wraps expression |
| 3 | `const_cast_in_property` | `assert property (const'(addr) == saved_addr);` |
| 4 | `const_cast_roundtrip` | Round-trip |
| 5 | `const_cast_translate` | `const'(data)` at t freezes to `data@t` |
| 6 | `const_cast_in_implication` | `req \|-> ##5 (out == const'(in))` — frozen at req time |
| 7 | `const_cast_z3_frozen_vs_live` | Z3 distinguishes frozen value from evolving signal |
| 8 | `const_cast_z3_correct_timestep` | Frozen at queue time t, not evaluation time t+k |
| 9 | `const_cast_vs_local_var` | `const'` is one-shot freeze; local var can be reassigned |
| 10 | `const_cast_nested` | `const'(a + const'(b))` nested |
| 11 | `const_cast_in_for_loop` | IEEE p.464 a2/a3 examples with const' in loop |
| 12 | `const_cast_ieee_p465_a8` | IEEE p.465 a8 example with const' in action block |
| 13 | `const_cast_sampled_interaction` | `const'` vs `$sampled` interaction |
| 14 | `const_cast_in_cover` | `cover property (const'(opcode) == 8'hFF);` |
| 15 | `sprint18_regression` | All existing tests unchanged |

---

### Sprint 19: `dist` Constraints (IEEE 16.14.2, 18.5.4)

**Why:** `expression dist { dist_list }` biases constrained random in `assume` statements. `:=` assigns weight per value; `:/` distributes weight across range. In formal mode, `dist` is equivalent to `inside` (constrains signal range), weight information is structural metadata.

**RED tests (~15 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `dist_single_values` | `data dist {0 := 1, 1 := 3}` |
| 2 | `dist_range_per_value` | `addr dist {[0:255] := 1}` |
| 3 | `dist_range_per_range` | `addr dist {[256:511] :/ 2}` |
| 4 | `dist_mixed` | `data dist {0 := 1, [1:10] :/ 5, 255 := 3}` |
| 5 | `dist_per_value_vs_per_range` | `:=` and `:/` produce different weight variants |
| 6 | `dist_roundtrip` | Round-trip |
| 7 | `dist_in_assume` | `assume property (@(posedge clk) req dist {0:=40, 1:=60});` |
| 8 | `dist_in_assert_formal` | In assert/cover context, dist → inside (IEEE p.458) |
| 9 | `dist_translate_constraint` | Dist → solver constraint restricting signal values |
| 10 | `dist_z3_restricts_space` | Z3 confirms dist limits reachable states |
| 11 | `dist_empty_rejected` | Empty dist list → parse error |
| 12 | `dist_single_item` | Single-item dist (degenerate) |
| 13 | `dist_with_expression` | `(a + b) dist {[0:10] := 1}` |
| 14 | `dist_ieee_p458_example` | IEEE p.458 req dist example reproduced |
| 15 | `sprint19_regression` | All existing tests unchanged |

---

### Sprint 20: Checkers (IEEE Chapter 17)

**Why:** `checker...endchecker` blocks are verification building blocks with `rand` free variables for nondeterministic formal modeling. A `rand const` variable is existentially quantified once per trace (value frozen). A `rand` (non-const) variable is existentially quantified per timestep (can change every cycle). The canonical `data_legal` checker (IEEE p.496) uses `rand const bit` to verify data integrity without local variables. `$inferred_clock` and `$inferred_disable` provide context from instantiation site.

**RED tests (~25 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `checker_basic_parse` | `checker my_check(logic a, b); ... endchecker` |
| 2 | `checker_with_event_port` | `checker my_check(logic sig, event clock);` |
| 3 | `checker_with_output` | `checker my_check3(logic a, b, output bit failure);` (IEEE p.486) |
| 4 | `checker_rand_const_bit` | `rand const bit d;` inside checker |
| 5 | `checker_rand_noncost_bit` | `rand bit flag;` inside checker |
| 6 | `checker_rand_const_bitvec` | `rand const bit [5:0] idx;` (IEEE p.496) |
| 7 | `checker_with_assert` | Checker body contains `assert property(...)` |
| 8 | `checker_with_assume` | Checker body contains `assume property(...)` |
| 9 | `checker_with_cover` | Checker body contains `cover property(...)` |
| 10 | `checker_with_default_clocking` | `default clocking @clock; endclocking` inside checker |
| 11 | `checker_with_default_disable` | `default disable iff reset;` inside checker |
| 12 | `checker_with_sequence_decl` | Sequence declaration inside checker body |
| 13 | `checker_with_property_decl` | Property declaration inside checker body |
| 14 | `checker_instance_parse` | `my_check check1(sig, posedge clk);` |
| 15 | `checker_instance_named_ports` | `check1(.test_sig(b), .clock(clk));` |
| 16 | `checker_roundtrip` | Checker declaration round-trips |
| 17 | `checker_instance_roundtrip` | Instance round-trips |
| 18 | `checker_resolve` | Instance resolved: ports bound, rand vars quantified |
| 19 | `checker_rand_const_z3` | `rand const bit d` → existential over entire trace |
| 20 | `checker_rand_nonconst_z3` | `rand bit d` → existential per timestep |
| 21 | `checker_inferred_clock` | `$inferred_clock` resolved from context |
| 22 | `checker_inferred_disable` | `$inferred_disable` resolved from context |
| 23 | `checker_ieee_data_legal` | IEEE p.496 `data_legal` checker reproduced end-to-end |
| 24 | `checker_ieee_reason_one_bit` | IEEE p.496 `reason_about_one_bit` with rand const idx |
| 25 | `checker_ieee_observer_model` | IEEE p.495 `observer_model` with free variable + assume constraints |
| 26 | `checker_ieee_assert_window` | IEEE p.501 `assert_window1` complex checker |
| 27 | `checker_no_module_inside` | Module inside checker → error |
| 28 | `sprint20_regression` | All existing tests unchanged |

---

### Sprint 21: Industrial Hardening — Cross-Feature Composition & Regression

**Why:** 20 sprints of features that must compose. This sprint verifies cross-feature interactions, reproduces complex real-world SVA patterns, and ensures zero regressions.

**RED tests (~20 tests):**

| # | Test | Assertion |
|---|---|---|
| 1 | `let_in_checker_body` | `let` construct used inside checker |
| 2 | `dist_in_checker_assume` | `dist` in checker's assume statement |
| 3 | `bitwise_in_local_var_assign` | `(req, v = data & mask)` |
| 4 | `const_cast_with_local_var` | `const'` and local variable in same property |
| 5 | `cover_sequence_with_dist` | Cover sequence + dist constraint |
| 6 | `sync_abort_multiclock` | `sync_reject_on` in multi-clock property |
| 7 | `vacuity_of_checker_assertion` | Nonvacuity check on checker's internal assertions |
| 8 | `s_always_bounded_in_checker` | `s_always [m:n]` in checker property |
| 9 | `let_with_bitwise_ops` | Let body contains bitwise operators |
| 10 | `concat_in_local_var` | `{a, b}` used in local variable assignment |
| 11 | `recursive_prop_with_local_var` | Recursive property with local variable threading |
| 12 | `full_5sprint_composition` | One assertion using: property connectives + until + strong + named decl + local var |
| 13 | `full_ieee_p442_check_write` | IEEE p.442 `check_write` recursive data integrity checker |
| 14 | `full_axi_protocol` | AXI4 write channel: assume + assert + cover + local vars |
| 15 | `full_arbiter` | Round-robin arbiter: assume fairness + assert mutual exclusion + cover grant |
| 16 | `all_41_existing_variants_unchanged` | Every existing SvaExpr variant still parses identically |
| 17 | `all_existing_translations_unchanged` | Every existing translation produces same BoundedExpr |
| 18 | `all_existing_z3_results_unchanged` | Every existing Z3 check produces same result |
| 19 | `parser_rejects_garbage` | Random invalid input → graceful error, not panic |
| 20 | `cross_sprint_full_regression` | All ~230 existing + all ~629 new tests pass simultaneously |

**Tier 5 total: ~164 tests.**

---

## Part VII: Verification Matrix & Totals

### Actual Test Counts (All Sprints Complete)

| File | Tests | Focus |
|---|---|---|
| `phase_hw_sva_coverage.rs` | 518 | Sprint-organized: all 21 sprints + 47 Z3 proofs |
| `phase_hw_sva_ieee1800.rs` | 175 | IEEE 1800 extended constructs |
| `phase_hw_sva_surface.rs` | 58 | Surface area expansion |
| `phase_hw_sva_roundtrip.rs` | 47 | Parse → render → parse equivalence |
| `phase_hw_sva_translate.rs` | 23 | SVA → BoundedExpr translation |
| `phase_hw_fol_to_sva.rs` | 30 | FOL → SVA synthesis |
| `phase_hw_codegen_sva.rs` | 7 | SVA/PSL code generation |
| **Total** | **858** | |

### IEEE 1800-2017 Coverage After All 21 Sprints

| IEEE Section | Feature | Status |
|---|---|---|
| 16.3 | Immediate assertions | Parse (Sprint 7) |
| 16.4 | Deferred assertions | Parse (Sprint 7) |
| 16.5 | Concurrent assertions overview | Complete |
| 16.6 | Boolean expressions | Complete + bitwise (Sprint 15) |
| 16.7 | Sequences (all forms incl. $) | Complete (Sprints 4, 5) |
| 16.8 | Declaring sequences | Complete (Sprint 8) |
| 16.9.2 | Repetition (all forms incl. $) | Complete (Sprint 4) |
| 16.9.3 | Sampled value functions | Complete (existing) |
| 16.9.4 | Global clock functions | Sprint 12 |
| 16.9.5 | Sequence AND | Complete (Sprint 5) |
| 16.9.6 | Intersect (length-matching) | Complete (Sprint 6) |
| 16.9.7 | Sequence OR | Complete (Sprint 5) |
| 16.9.8 | first_match | Complete (Sprint 6) |
| 16.9.9 | throughout | Complete (Sprint 6) |
| 16.9.10 | within | Complete (Sprint 6) |
| 16.9.11 | Endpoint methods | Complete (Sprint 14) |
| 16.10 | Local variables | Complete (Sprint 10) |
| 16.11 | Subroutine calls on match | Parse (Sprint 7 action blocks) |
| 16.12.1 | Property instantiation | Complete (Sprint 9) |
| 16.12.2 | Sequence property (strong/weak) | Complete (Sprint 3) |
| 16.12.3 | Negation property (not) | Complete (Sprint 1) |
| 16.12.4 | Disjunction property (or) | Complete (existing + Sprint 5) |
| 16.12.5 | Conjunction property (and) | Complete (existing + Sprint 5) |
| 16.12.6 | If-else property | Complete (existing) |
| 16.12.7 | Implication (\|->, \|=>) | Complete (existing) |
| 16.12.8 | Implies and iff | Complete (Sprint 1) |
| 16.12.9 | Followed-by (#-#, #=#) | Complete (Sprint 3) |
| 16.12.10 | Nexttime / s_nexttime | Complete (existing + Sprint 3) |
| 16.12.11 | Always (all forms) | Complete (Sprint 2) |
| 16.12.12 | Until (all 4 forms) | Complete (Sprint 2) |
| 16.12.13 | Eventually (all forms) | Complete (Sprint 2) |
| 16.12.14 | Abort (all 4: async+sync × accept+reject) | Complete (existing + Sprint 3) |
| 16.12.15 | Weak and strong operators | Complete (Sprint 3) |
| 16.12.16 | Case property | Complete (Sprint 3) |
| 16.12.17 | Recursive properties | Complete (Sprint 9) |
| 16.13 | Multiclock support | Complete (Sprint 12) |
| 16.14 | Concurrent assertion directives | Complete (Sprint 7) |
| 16.14.6.1 | const' cast | Complete (Sprint 18) |
| 16.14.7 | Inferred value functions | Complete (Sprint 20) |
| 16.14.8 | Nonvacuous evaluation | Complete (Sprint 17) |
| 16.15-16 | Default clocking/disable, clock resolution | Complete (Sprint 11) |
| 16.17 | Expect statement | OUT OF SCOPE (simulation-only) |
| 16.18 | Clocking blocks | Complete (Sprint 11/12) |
| 17.1-17.9 | Checkers | Complete (Sprint 20) |
| 11.12 | Let construct | Complete (Sprint 16) |
| 20.9 | System functions (all) | Complete (existing + Sprint 13) |

**Coverage: 100% of IEEE 1800-2017 Chapters 16-17 except `expect` statement (simulation-only blocking wait).**

---

## Part VIII: What "Done" Means

After all 21 sprints, LogicAffeine's SVA pipeline will:

1. **Parse** any syntactically valid SVA from IEEE 1800-2017 Chapters 16-17 without rejecting valid input
2. **Model** every temporal, sequential, and property operator with correct AST representation
3. **Resolve** named sequences, properties, let constructs, and checker instances with argument substitution
4. **Elaborate** with default clocking and disable iff resolution, including nested scope overrides
5. **Translate** to multi-sorted `BoundedExpr` IR with correct temporal unrolling semantics
6. **Encode** to Z3 via kernel terms using Curry-Howard isomorphism
7. **Verify** using SUPERCRUSH algorithms (k-induction, IC3, interpolation)
8. **Distinguish** assert/assume/cover/restrict with correct formal semantics per directive
9. **Thread** local variable state through bounded unrolling for data integrity verification
10. **Track** nonvacuous evaluation to detect dead assertions
11. **Model** nondeterministic inputs via checker free variables (existential quantification)
12. **Cross** clock domain boundaries with multi-clock encoding
13. **Generate** counterexample traces with multi-bit signal values
14. **Compose** all features: a single assertion can use property connectives + until + strong/weak + named declarations + local variables + bitwise operators + const' + dist + checkers

The only excluded feature is the `expect` statement (16.17), which is a simulation-only procedural blocking assertion with no formal verification semantics.
