# SUPERCRUSH

Engineering specification for LogicAffeine's full multi-sorted hardware verification pipeline. Picks up where CRUSH ends. Every claim verified against source code. Every weakness disclosed. Every sprint has concrete RED tests.

**Prerequisite: CRUSH complete (532+ tests, propositional temporal logic, boolean-only equivalence).**

---

## Part I: Where CRUSH Ends and SUPERCRUSH Begins

### What CRUSH Delivers

| Capability | Tests | Status |
|---|---|---|
| Boolean Z3 equivalence (spec <-> SVA) | 20 | Sound for propositional fragment |
| SVA parsing (21 variants) + IEEE 1800 extensions | 88 | Full round-trip |
| FOL -> SVA formal synthesis | 30 | Z3-proven correct |
| CEGAR refinement | 15 | Iterative spec-SVA alignment |
| Kripke temporal lowering (G, F, X, U, R, W) | 17 | All LTL operators |
| Knowledge graph (28 entity types, 24 relations) | 34 | Formal hardware ontology |
| Verilog parser + RTL KG linking | 40 | Spec-to-RTL traceability |
| Specification coverage + sufficiency | 30 | Gap detection before verification |
| Property consistency + decomposition | 18 | Z3-verified compositionality |
| Invariant discovery from KG | 10 | Candidate generation + Z3 confirmation |
| Waveform generation from counterexamples | 10 | VCD + ASCII timing diagrams |
| Protocol templates (AXI, APB, UART, SPI, I2C) | 25 | Z3 self-consistency certificates |
| **Total** | **532+** | **Propositional temporal logic** |

### What CRUSH Explicitly Does Not Do

Line 671 of CRUSH: *"Honest scope. Propositional temporal logic. Bitvector/arithmetic/quantifier support is future work, not hidden behind overapproximation."*

| Limitation | Consequence |
|---|---|
| Equivalence checker is boolean-only | Cannot verify `data_out == data_in + 1` or `addr[31:16] == base_addr` |
| Quantifiers are stubs | `forall x. P(x)` encodes as just `P(x)` with free `x` |
| No integer arithmetic in equivalence | `count > threshold` and `count < threshold` both become `true` |
| No bitvector semantics in equivalence | 32-bit overflow, sign extension, bit masking all invisible |
| BMC only | Cannot prove a property holds *forever* -- only up to bound *k* |
| No liveness verification | "Eventually, every acknowledgment holds" cannot be verified |
| Single clock domain | Multi-clock designs not modeled |
| No CDC analysis | Clock domain crossing bugs invisible |
| No reactive synthesis | Can verify assertions but cannot synthesize implementations |
| No proof certificates | Verification results must be trusted, not independently checked |

### The Single Architectural Bottleneck

The `VerifyExpr` IR already has bitvector, array, and quantifier variants. The Z3 encoder in `solver.rs` already handles all of them correctly. The bottleneck is one function:

```
equivalence.rs:encode_to_z3()  →  returns Bool<'ctx>
                                   non-boolean ops → true
                                   all variables declared as Bool
                                   counterexample extraction: as_bool() only
```

Meanwhile, `solver.rs:Encoder::encode()` returns `Dynamic<'ctx>` and correctly handles:

| Feature | solver.rs | equivalence.rs |
|---|---|---|
| Int arithmetic (Add, Sub, Mul, Div) | Z3 Int ops | `true` |
| Int comparison (Gt, Lt, Gte, Lte) | Z3 Bool from Int | `true` |
| BitVec ops (17 variants) | Z3 BV theory | `true` |
| BitVecExtract, BitVecConcat | Z3 extract/concat | `true` |
| Array Select/Store | Z3 array theory | `true` |
| Uninterpreted functions | Z3 FuncDecl | `true` |
| Quantifiers (ForAll, Exists) | Stub (drops vars) | `true` |

**SUPERCRUSH Tier 1 is: make `equivalence.rs` as capable as `solver.rs`.**

The IR is ready. The solver is ready. The gap is plumbing.

### What Else Exists (Verified in Source)

| Component | Location | Relevance |
|---|---|---|
| Proof engine with `TemporalInduction`, `UntilInduction` | `logicaffeine_proof/src/lib.rs` | Foundation for k-induction, proof certificates |
| Certifier bridge (Curry-Howard) | `logicaffeine_proof/src/certifier.rs` | Proof trees → kernel terms |
| Kernel: congruence closure, Fourier-Motzkin, Omega test | `logicaffeine_proof/src/` | Decision procedures for arithmetic |
| Z3 Oracle integration | `logicaffeine_proof/src/oracle.rs` | Proof search fallback |
| All 3 Futamura projections | `logicaffeine_compile/src/` | Verified compiler construction |
| `VerifyType::BitVector(u32)`, `VerifyType::Array(_, _)` | `logicaffeine_verify/src/ir.rs` | Multi-sorted type system ready |
| `BitVecOp` (17 ops) | `logicaffeine_verify/src/ir.rs` | Full bitvector operation set |

---

## Part II: Tier 1 -- Complete the Foundations

### Sprint S0A: Multi-Sorted Equivalence Checker

**Why:** `equivalence.rs:encode_to_z3()` returns `Bool<'ctx>` and maps non-boolean ops to `true`. This is the root cause of every false positive. `solver.rs:Encoder::encode()` already returns `Dynamic<'ctx>` and handles all sorts correctly. Factor out or replicate.

**Files:** `crates/logicaffeine_verify/src/equivalence.rs` (major rewrite)

**What:**

1. `encode_to_z3()` returns `Dynamic<'ctx>` instead of `Bool<'ctx>`. Use `solver.rs:Encoder` as reference.
2. Variable declarations are type-inferred: analyze expression tree, determine `Int` vs `Bool` vs `BitVec(n)` vs `Array(_, _)`.
3. Equivalence formula `not(A <-> B)` uses `Dynamic::_eq()` for non-boolean sorts, `.iff()` for boolean.
4. Reject incompatible sorts with `EquivalenceResult::Unknown { reason }` instead of overapproximating.

**RED tests (extend `phase_hw_z3_equiv.rs`, ~40 tests):**

| Test | Assertion |
|---|---|
| `z3_integer_addition_equivalence` | `(x + 5 == 10)` equiv `(x == 5)` |
| `z3_integer_subtraction_equivalence` | `(x - 3 == 7)` equiv `(x == 10)` |
| `z3_integer_inequality_detected` | `(x > 5)` not-equiv `(x < 5)` |
| `z3_integer_multiplication` | `(x * 2 == 10)` equiv `(x == 5)` |
| `z3_mixed_bool_int` | `(valid AND count > 0)` not-equiv `(valid AND count < 0)` |
| `z3_integer_division` | `(x / 2 == 3)` equiv `(x == 6)` with integer semantics |
| `z3_comparison_chain` | `(x > 0 AND x < 10)` not-equiv `(x > 0 AND x < 5)` |
| `z3_bitvector_and_mask` | `bv_and(x, 0xFF)` equiv `bv_extract(x, 7, 0)` for BV(16) |
| `z3_bitvector_add` | `bv_add(x, y)` not-equiv `bv_sub(x, y)` |
| `z3_bitvector_shift_left` | `bv_shl(x, 1)` equiv `bv_mul(x, 2)` for BV(8) |
| `z3_bitvector_overflow_detected` | `bv_add(0xFF, 1)` wraps to 0 for BV(8) -- not equiv to 256 |
| `z3_signed_vs_unsigned` | `bv_slt(x, y)` not-equiv `bv_ult(x, y)` for negative values |
| `z3_bitvector_concat_extract` | `bv_extract(bv_concat(a, b), 15, 8)` equiv `a` for BV(8) |
| `z3_bitvector_sign_extension` | `bv_ashr(x, 7)` distinguishes positive/negative for BV(8) |
| `z3_bitvector_xor_self` | `bv_xor(x, x)` equiv `bv_const(0, n)` |
| `z3_bitvector_not_not` | `bv_not(bv_not(x))` equiv `x` |
| `z3_array_select_store` | `select(store(a, i, v), i)` equiv `v` |
| `z3_array_non_aliasing` | `select(store(a, i, v), j)` with `i != j` equiv `select(a, j)` |
| `z3_array_store_overwrite` | `store(store(a, i, v1), i, v2)` equiv `store(a, i, v2)` |
| `z3_array_different_indices` | Two stores at different indices both visible |
| `z3_uninterp_func_distinct` | `Apply("F", [x])` not-equiv `Apply("G", [x])` |
| `z3_uninterp_func_congruence` | `Apply("F", [x])` equiv `Apply("F", [x])` |
| `z3_counterexample_has_integer_values` | Trace contains `SignalValue::Int(n)` not just bools |
| `z3_counterexample_has_bitvec_values` | Trace contains `SignalValue::BitVec { width, value }` |
| `z3_sort_mismatch_returns_unknown` | Bool var in Int context → `Unknown` |
| `z3_nested_arithmetic` | `((x + y) * 2 == z)` not-equiv `((x + y) == z)` |
| `z3_demorgan_bitvec` | `bv_not(bv_and(a,b))` equiv `bv_or(bv_not(a), bv_not(b))` |
| `z3_bitvector_width_mismatch_rejected` | BV(8) op BV(16) → `Unknown` |
| `z3_implication_with_arithmetic` | `(x > 0) -> (x >= 1)` is valid |
| `z3_empty_signals_still_works` | No signals, pure arithmetic → still checks equivalence |
| `z3_large_bitvector_32bit` | 32-bit bitvector operations don't overflow Z3 |
| `z3_mixed_bitvec_and_bool` | `(valid AND bv_eq(data, 0xFF))` works across sorts |
| `z3_iff_for_integers` | `(x == 5) <-> (x == 5)` is tautology |
| `z3_bool_equiv_unchanged` | Existing boolean tests still pass identically |
| `z3_int_counterexample_trace_readable` | Trace cycle shows `count: 7` not `count: true` |
| `z3_bitvec_counterexample_trace_hex` | Trace shows bitvector values in hex |
| `z3_multiple_sorts_single_formula` | Formula mixing Bool, Int, BV(8) -- all correctly sorted |
| `z3_arithmetic_in_temporal_context` | `G(count > 0)` through bounded unrolling with integer count |
| `z3_bitvec_in_temporal_context` | `G(bv_eq(status, 0x01))` through bounded unrolling |
| `z3_performance_boolean_not_regressed` | Boolean-only case not measurably slower |

---

### Sprint S0B: Fix Quantifier Encoding

**Why:** `solver.rs` lines 795-801 drop bound variables. `forall x. x > 0` becomes just `x > 0` with free `x`. Z3 Rust bindings support `z3::ast::forall_const()` and `z3::ast::exists_const()`.

**Files:** `crates/logicaffeine_verify/src/solver.rs`, `crates/logicaffeine_verify/src/equivalence.rs`

**What:** Replace quantifier stubs with proper Z3 quantifier construction. Bound variables become Z3 constants in the quantifier scope. Type annotations from `VerifyType` determine Z3 sort.

**RED tests (extend `phase_hw_z3_equiv.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `z3_forall_not_true` | `forall x:Int. x > 0` is NOT equivalent to `true` |
| `z3_exists_satisfiable` | `exists x:Int. x == 5` is SAT |
| `z3_forall_valid` | `forall x:Int. (x > 0 -> x >= 0)` is valid |
| `z3_forall_invalid` | `forall x:Int. x > 0` is not valid (x could be negative) |
| `z3_nested_quantifiers` | `forall x. exists y. y > x` is valid over integers |
| `z3_quantifier_bitvec` | `forall bv:BV(8). bv_ule(bv, 0xFF)` is valid |
| `z3_quantifier_mixed_free_bound` | Free vars stay free, bound vars are quantified |
| `z3_exists_with_witness` | `exists x. x == 5 AND x > 3` -- counterexample shows witness |
| `z3_forall_array` | `forall i:Int. select(store(a, i, v), i) == v` is valid |
| `z3_quantifier_alternation` | `forall x. exists y. f(x) == y` is valid |
| `z3_empty_quantifier` | `forall. P` (no vars) equiv `P` |
| `z3_quantifier_in_equivalence` | Quantified formula in equivalence check works end-to-end |
| `z3_quantifier_scope_correct` | `(forall x. P(x)) AND Q(x)` -- second x is free |
| `z3_quantifier_counterexample` | Failed universal produces counterexample instantiation |
| `z3_quantifier_performance` | Quantified formula solves within 5s timeout |

---

### Sprint S0C: Extend BoundedExpr for Multi-Sorted Pipeline

**Why:** `BoundedExpr` only has `Bool/Int/Var/And/Or/Not/Implies/Eq`. The FOL → BoundedExpr → VerifyExpr pipeline cannot represent bitvector or array operations. SVA specs with data widths (`data_out[7:0] == 8'hFF`) cannot pass through.

**Files:** `crates/logicaffeine_compile/src/codegen_sva/sva_to_verify.rs`, `crates/logicaffeine_compile/src/codegen_sva/fol_to_verify.rs`

**What:**

New `BoundedExpr` variants:
```rust
BitVecConst { width: u32, value: u64 },
BitVecVar(String, u32),                    // variable with known width
BitVecBinary { op: BitVecOp, left, right },
BitVecExtract { high: u32, low: u32, operand },
BitVecConcat(Box<BoundedExpr>, Box<BoundedExpr>),
ArraySelect { array, index },
ArrayStore { array, index, value },
IntBinary { op: ArithOp, left, right },    // Add, Sub, Mul, Div
Comparison { op: CmpOp, left, right },     // Gt, Lt, Gte, Lte (returns Bool)
ForAll { var: String, sort: BoundedSort, body },
Exists { var: String, sort: BoundedSort, body },
```

Extend `SvaTranslator::translate()` to handle `SvaExpr::Const(value, width)` → `BoundedExpr::BitVecConst`.

Extend `FolTranslator::translate()` to handle arithmetic predicates from Kripke lowering. Replace catch-all `Bool(true)` with explicit `Unknown` variant or error propagation.

Extend `bounded_to_verify()` bridge for all new variants.

**RED tests (new file `phase_hw_bounded_multisort.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `bounded_bitvec_const` | `SvaExpr::Const(0xFF, 8)` → `BoundedExpr::BitVecConst{8, 255}` |
| `bounded_bitvec_eq` | `SvaExpr::Eq(Const(5,8), Signal("x"))` preserves width |
| `bounded_bitvec_to_verify` | `BoundedExpr::BitVecConst{8,255}` → `VerifyExpr::BitVecConst{8,255}` |
| `bounded_array_select` | Array select through pipeline |
| `bounded_int_arithmetic` | `x + 5` through BoundedExpr → VerifyExpr |
| `bounded_comparison` | `x > 5` produces Bool-sorted output |
| `bounded_fol_arithmetic_predicate` | "count is greater than 5" → `Comparison{Gt, ...}` not `Bool(true)` |
| `bounded_fol_no_catch_all` | Unhandled variant → error, not silent `true` |
| `bounded_mixed_sort_roundtrip` | Bool + Int + BV in one formula → correct VerifyExpr |
| `bounded_temporal_with_bitvec` | `G(bv_eq(data, 0xFF))` unrolls with bitvec at each timestep |
| `bounded_temporal_with_int` | `G(count > 0)` unrolls with int comparison at each timestep |
| `bounded_quantifier_roundtrip` | `ForAll{x, Int, body}` → `VerifyExpr::ForAll{...}` |
| `bounded_end_to_end_int_equiv` | English spec → FOL → Bounded → Verify → Z3 with integers |
| `bounded_end_to_end_bitvec_equiv` | English spec → FOL → Bounded → Verify → Z3 with bitvectors |
| `bounded_sva_const_width_preserved` | SVA `8'hFF` width survives all pipeline stages |
| `bounded_array_store_select` | Store then select in BoundedExpr → correct VerifyExpr |
| `bounded_nested_bitvec_ops` | `bv_and(bv_or(a, b), c)` nests correctly |
| `bounded_extract_concat` | Extract and concat compose correctly |
| `bounded_regression_boolean` | All existing boolean BoundedExpr tests unchanged |
| `bounded_regression_temporal` | All existing temporal unrolling tests unchanged |

---

### Sprint S0D: Multi-Sorted Counterexample Traces

**Why:** `extract_trace()` only handles `as_bool()`. Integer and bitvector counterexamples show up as missing signals. CRUSH Sprint 6A (waveform generation) needs multi-bit signals for VCD output.

**Files:** `crates/logicaffeine_verify/src/equivalence.rs`

**What:**

New signal value type:
```rust
pub enum SignalValue {
    Bool(bool),
    Int(i64),
    BitVec { width: u32, value: u64 },
    Unknown,
}

pub struct CycleState {
    pub cycle: usize,
    pub signals: HashMap<String, SignalValue>,
}
```

`extract_trace()` uses Z3 model evaluation: try `as_bool()`, then `as_i64()`, then `as_u64()` for bitvectors. Variable type annotations (from S0E) guide extraction.

VCD generation handles multi-bit signals: `$var wire 8 data_out $end` with hex value changes.

**RED tests (extend `phase_hw_waveform.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `trace_integer_signal` | Counter signal shows `SignalValue::Int(7)` not `Unknown` |
| `trace_bitvec_signal` | 8-bit data shows `SignalValue::BitVec{8, 0xAB}` |
| `trace_mixed_signals` | Bool + Int + BV in one trace, all correctly typed |
| `trace_multi_cycle_integer` | 5-cycle trace with incrementing integer values |
| `trace_bitvec_hex_display` | Display format shows `0xAB` not `171` |
| `vcd_multibit_header` | `$var wire 8 data_out $end` in VCD output |
| `vcd_multibit_value_change` | `b10101011 data_out` at correct timestamp |
| `vcd_32bit_signal` | 32-bit signal in VCD without truncation |
| `ascii_waveform_multibit` | Multi-bit signals show hex values in ASCII diagram |
| `ascii_waveform_integer` | Integer signals show decimal values |
| `trace_preserves_signal_names` | KG signal names, not internal `@t` suffixes |
| `trace_empty_still_works` | Empty counterexample → empty trace (no panic) |
| `trace_bool_unchanged` | Existing boolean traces produce identical output |
| `trace_bitvec_all_ones` | `0xFF` for 8-bit, `0xFFFFFFFF` for 32-bit |
| `trace_serializable` | `Trace` with mixed `SignalValue` types round-trips through JSON |

---

### Sprint S0E: Type Inference Engine

**Why:** The multi-sorted equivalence checker (S0A) needs to know each variable's Z3 sort. In CRUSH, everything was Bool. Now variables used in `bv_add` must be `BitVec(n)`, variables in `x > 5` must be `Int`.

**Files:** new `crates/logicaffeine_verify/src/type_infer.rs`

**What:**

```rust
pub fn infer_types(expr: &VerifyExpr) -> Result<HashMap<String, VerifyType>, TypeError> {
    // Walk expression tree
    // Unify constraints: var in bv_add(var, _) => BitVec(n)
    // Detect conflicts: var used as both Bool and Int => TypeError
    // Propagate widths: bv_add(x:BV(8), y:?) => y:BV(8)
}
```

Constraint-based inference:
1. Collect constraints from each expression node
2. Unify: `Var("x")` in `Binary{Add, Var("x"), Int(5)}` → `x: Int`
3. Width propagation: `BitVecBinary{Add, Var("x"), BitVecConst{8, _}}` → `x: BV(8)`
4. Conflict detection: `x` used as both `Bool` and `Int` → `TypeError`

**RED tests (new file `phase_hw_type_infer.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `infer_bool_from_and` | `And(Var("x"), Var("y"))` → both Bool |
| `infer_int_from_add` | `Add(Var("x"), Int(5))` → x is Int |
| `infer_bitvec_from_bv_add` | `BvAdd(Var("x"), BvConst(8, 0))` → x is BV(8) |
| `infer_bitvec_width_propagation` | Width from constant propagates to variable |
| `infer_array_from_select` | `Select(Var("a"), Int(0))` → a is Array(Int, ?) |
| `infer_conflict_detected` | `And(Var("x"), Add(Var("x"), Int(1)))` → TypeError |
| `infer_nested` | `And(Gt(Var("x"), Int(5)), Bool(true))` → x is Int |
| `infer_implication` | `Implies(Var("p"), Var("q"))` → both Bool |
| `infer_comparison_returns_bool` | `Gt(Var("x"), Int(5))` → result is Bool, x is Int |
| `infer_bitvec_comparison` | `BvULt(Var("x"), Var("y"))` → both BV(same width) |
| `infer_extract_width` | `BvExtract{7,0}(Var("x"))` → x has width >= 8, result is BV(8) |
| `infer_concat_width` | `BvConcat(BV(8), BV(8))` → result is BV(16) |
| `infer_empty_formula` | `Bool(true)` → empty map |
| `infer_free_variable_defaults_object` | Unconstrained var → `VerifyType::Object` |
| `infer_multiple_constraints_unify` | Same var in multiple contexts → single consistent type |

---

### Sprint S0F: Tier 1 Regression Gate

**Why:** All 532+ CRUSH tests must still pass. All existing `solver.rs` and `equivalence.rs` tests must still pass. Multi-sorted changes must not break boolean-only fast path.

**RED tests (~5):**

| Test | Assertion |
|---|---|
| `regression_all_crush_tests_pass` | `cargo test` exit 0 |
| `regression_boolean_equivalence_identical` | Same results as pre-SUPERCRUSH for boolean inputs |
| `regression_solver_unchanged` | `solver.rs` tests produce identical results |
| `regression_bmc_unchanged` | BMC temporal tests produce identical results |
| `regression_performance_boolean` | Boolean-only equivalence not > 10% slower |

**Tier 1 total: ~110 tests. Cumulative: ~642.**

---

## Part III: Tier 2 -- Advanced Verification Algorithms

### Sprint S1A: k-Induction

**Why:** BMC proves a property holds for *k* cycles. k-Induction proves it holds *forever*. Base case: property holds for steps 0..k. Inductive step: if property holds for k consecutive steps, it holds for step k+1.

**Files:** new `crates/logicaffeine_verify/src/kinduction.rs`

**Depends on:** S0A

**What:**

```rust
pub enum KInductionResult {
    Proven { k: u32 },                          // Property holds for all time
    Counterexample { k: u32, trace: Trace },    // Base case violation at step k
    InductionFailed { k: u32, trace: Trace },   // Inductive step failed (may need larger k)
    Unknown,
}

pub fn k_induction(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[String],
    max_k: u32,
) -> KInductionResult;
```

Algorithm:
1. For k = 1, 2, ..., max_k:
   a. **Base case:** Assert `init AND transition^k AND NOT(property)` at each step. If UNSAT for all steps → base passes.
   b. **Inductive step:** Assert `property` holds for k consecutive steps AND `transition` AND `NOT(property)` at step k+1. If UNSAT → proven.
2. If base fails → `Counterexample`. If induction fails for all k → `InductionFailed`.

**RED tests (new file `phase_hw_kinduction.rs`, ~25 tests):**

| Test | Assertion |
|---|---|
| `kind_simple_safety_proven` | `G(x >= 0)` with `x' = x + 1, init: x = 0` → `Proven{k:1}` |
| `kind_violation_detected` | `G(x < 10)` with unbounded increment → `Counterexample` |
| `kind_needs_k_2` | Property requires k=2 induction (data forwarding pipeline) |
| `kind_needs_k_3` | Three-stage pipeline needs k=3 |
| `kind_mutex_proven` | `G(not(grant_a AND grant_b))` with arbiter → `Proven` |
| `kind_counter_overflow` | 8-bit counter wraps → counterexample at step 256 |
| `kind_bitvec_property` | `G(bv_ule(counter, 0xFF))` for BV(8) → trivially proven |
| `kind_bitvec_arithmetic` | `G(bv_eq(out, bv_add(a, b)))` → verified for adder |
| `kind_induction_failed_not_false` | Induction failure ≠ property violation |
| `kind_counterexample_is_trace` | Counterexample has cycle states with signal values |
| `kind_init_matters` | Different init → different result |
| `kind_transition_relation` | Transition encodes state update correctly |
| `kind_multiple_signals` | 4-signal design verified |
| `kind_timeout_returns_unknown` | Slow formula → `Unknown` within time budget |
| `kind_k1_is_bmc1` | k=1 induction with no inductive step = BMC(1) |
| `kind_proven_is_sound` | If `Proven{k}`, BMC(1000) also passes (sanity) |
| `kind_integer_property` | Integer arithmetic property proven unbounded |
| `kind_array_property` | Array-based property (memory init) proven |
| `kind_empty_transition` | Identity transition (no state change) → trivially proven |
| `kind_latch_property` | Latch holds value → proven with k=1 |
| `kind_register_chain` | Shift register: data appears after N cycles |
| `kind_fairness_not_needed` | Pure safety -- no fairness constraint |
| `kind_strengthened_inductive` | Property not inductive at k=1, strengthened invariant helps |
| `kind_multiple_properties` | Verify conjunction of properties in one pass |
| `kind_incremental_k` | k=1 fails induction, k=2 succeeds |

---

### Sprint S1B: IC3/PDR (Property-Directed Reachability)

**Why:** The gold standard for unbounded safety verification (Bradley 2011). Maintains frame sequence F_0, F_1, ..., F_k where each frame over-approximates reachable states at step i. Converges when F_i = F_{i+1} (fixpoint = inductive invariant).

**Files:** new `crates/logicaffeine_verify/src/ic3.rs`

**Depends on:** S0A, S0B

**What:**

```rust
pub enum Ic3Result {
    Safe { invariant: VerifyExpr },            // Property holds, invariant found
    Unsafe { trace: Trace },                   // Counterexample trace
    Unknown,
}

pub fn ic3(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
) -> Ic3Result;
```

Core operations:
1. **Counterexample to Induction (CTI):** Find state satisfying F_i AND transition AND NOT(property)
2. **Generalization:** Minimize blocking clause (drop literals that aren't needed)
3. **Propagation:** Push clauses forward through frames
4. **Convergence:** Check if F_i = F_{i+1}

**RED tests (new file `phase_hw_ic3.rs`, ~30 tests):**

| Test | Assertion |
|---|---|
| `ic3_simple_mutex_safe` | Two-process mutex → `Safe` with invariant |
| `ic3_unsafe_detected` | Reachable bad state → `Unsafe` with trace |
| `ic3_invariant_is_inductive` | Returned invariant passes induction check |
| `ic3_invariant_implies_property` | Returned invariant implies the property |
| `ic3_trace_is_valid` | Counterexample trace: init → transition* → violation |
| `ic3_frame_monotone` | F_i implies F_{i+1} (internal consistency) |
| `ic3_converges_on_small` | 3-signal design converges within 10 frames |
| `ic3_blocking_clause_minimal` | Generalized clause has no redundant literals |
| `ic3_propagation_pushes_forward` | Clause learned at frame i appears at frame i+1 |
| `ic3_bitvec_safe` | 8-bit counter with overflow guard → `Safe` |
| `ic3_bitvec_unsafe` | 8-bit counter without guard → `Unsafe` at step 256 |
| `ic3_arbiter_fair` | Round-robin arbiter: no starvation → `Safe` |
| `ic3_fifo_no_overflow` | FIFO with depth check → `Safe` |
| `ic3_fifo_overflow` | FIFO without depth check → `Unsafe` |
| `ic3_pipeline_data_integrity` | 3-stage pipeline: data preserved → `Safe` |
| `ic3_init_state_matters` | Wrong initial state → `Unsafe` |
| `ic3_multiple_properties` | Conjunction of safety properties |
| `ic3_integer_arithmetic` | Integer counter property verified unbounded |
| `ic3_array_memory` | Memory initialized to zero, stays zero unless written |
| `ic3_state_machine_reachability` | FSM: unreachable bad state → `Safe` |
| `ic3_state_machine_bug` | FSM: reachable bad state → `Unsafe` with path |
| `ic3_generalization_correct` | Generalized clause still blocks the CTI |
| `ic3_relative_induction` | Clause is inductive relative to frame |
| `ic3_deep_bug` | Bug reachable only after 50+ steps → found |
| `ic3_timeout_returns_unknown` | Complex design → `Unknown` within budget |
| `ic3_deterministic` | Same input → same result (no non-determinism) |
| `ic3_empty_init` | Unconstrained init → explores all states |
| `ic3_axi_protocol` | AXI write channel safety properties → `Safe` |
| `ic3_register_file` | x0 always zero in RISC-V register file → `Safe` |
| `ic3_compared_to_bmc` | IC3 proves what BMC(100) can't (unbounded) |

---

### Sprint S1C: Interpolation-Based Model Checking

**Why:** Craig interpolation provides over-approximations of reachable states. Given `A AND B` is UNSAT, interpolant `I` satisfies `A -> I` and `I AND B` is UNSAT. Useful for computing reachable state abstractions.

**Files:** new `crates/logicaffeine_verify/src/interpolation.rs`

**Depends on:** S0A

**What:**

```rust
pub fn interpolate(a: &VerifyExpr, b: &VerifyExpr) -> Option<VerifyExpr>;
pub fn itp_model_check(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    bound: u32,
) -> VerificationResult;
```

**RED tests (new file `phase_hw_interpolation.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `interpolant_implies_a` | `A -> I` is valid |
| `interpolant_contradicts_b` | `I AND B` is UNSAT |
| `interpolant_over_common_vars` | Interpolant only uses shared variables |
| `interpolant_boolean` | Boolean formula interpolation |
| `interpolant_integer` | Integer formula interpolation |
| `interpolant_bitvec` | Bitvector formula interpolation |
| `itp_mc_safe` | Simple safety → verified via interpolation |
| `itp_mc_unsafe` | Reachable violation → detected |
| `itp_mc_fixpoint` | Interpolation sequence converges |
| `itp_mc_stronger_than_bmc` | Proves property BMC cannot |
| `itp_mc_refines` | Each iteration refines the overapproximation |
| `itp_mc_init_reachable` | Initial states included in every approximation |
| `itp_mc_property_preserved` | Every approximation implies property |
| `itp_mc_pipeline` | Pipeline design verified |
| `itp_mc_counter` | Counter with bound verified |
| `itp_mc_timeout` | Complex design → graceful timeout |
| `itp_no_interpolant_sat` | SAT formula → no interpolant (returns None) |
| `itp_trivial` | `false AND B` → interpolant is `false` |
| `itp_mc_multiple_properties` | Multiple properties in one pass |
| `itp_mc_compared_to_ic3` | Same result as IC3 on shared examples |

---

### Sprint S1D: Liveness-to-Safety Reduction

**Why:** Liveness properties (`G(F(ack))` -- "acknowledgment always eventually comes") cannot be checked by BMC or k-induction directly. Biere-Artho-Schuppan (2002) reduction: add shadow system, non-deterministic freeze, check that property *never* holds after freeze.

**Files:** new `crates/logicaffeine_verify/src/liveness.rs`

**Depends on:** S1A

**What:**

```rust
pub enum LivenessResult {
    Live,                                    // Property holds on all fair paths
    NotLive { trace: Trace, loop_point: usize },  // Lasso-shaped counterexample
    Unknown,
}

pub fn check_liveness(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    fairness: &[VerifyExpr],
    property: &VerifyExpr,        // The "eventually" part
    signals: &[SignalDecl],
) -> LivenessResult;
```

Reduction: construct safety property on doubled state space. If safety holds → liveness holds. If safety violated → extract lasso-shaped counterexample (prefix + loop).

**RED tests (new file `phase_hw_liveness.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `liveness_simple_ack` | `G(F(ack))` with fair scheduler → `Live` |
| `liveness_starvation` | Unfair arbiter → `NotLive` with lasso trace |
| `liveness_with_fairness` | Fairness constraint makes starvation impossible → `Live` |
| `liveness_lasso_correct` | Lasso trace: prefix reaches loop, loop satisfies fairness |
| `liveness_eventually_response` | `G(req -> F(ack))` → response liveness |
| `liveness_progress` | `G(F(progress_flag))` → system always progresses |
| `liveness_deadlock_detected` | System reaches state with no outgoing transitions → `NotLive` |
| `liveness_multiple_fairness` | Two fairness constraints interact correctly |
| `liveness_bitvec_signal` | Liveness on bitvector predicate: `G(F(bv_eq(status, 0x01)))` |
| `liveness_integer_signal` | `G(F(count == 0))` -- counter resets |
| `liveness_timeout` | Complex liveness → `Unknown` within budget |
| `liveness_safety_dual` | Safety and liveness on same design, both verified |
| `liveness_loop_point_valid` | Loop point index is within trace bounds |
| `liveness_reduction_sound` | Manual L2S agrees with automated reduction |
| `liveness_empty_fairness` | No fairness = all paths are fair |

---

### Sprint S1E: Assume-Guarantee Compositional Reasoning

**Why:** Monolithic verification doesn't scale. Decompose into per-component proofs with interface contracts.

**Files:** new `crates/logicaffeine_verify/src/compositional.rs`

**Depends on:** S0A, CRUSH Sprint 6C (decomposition)

**What:**

```rust
pub struct ComponentSpec {
    pub name: String,
    pub assumes: Vec<VerifyExpr>,     // What this component requires from its environment
    pub guarantees: Vec<VerifyExpr>,  // What this component provides
    pub init: VerifyExpr,
    pub transition: VerifyExpr,
}

pub enum CompositionalResult {
    AllVerified,
    ComponentFailed { name: String, trace: Trace },
    CircularDependency { components: Vec<String> },
    Unknown,
}

pub fn verify_compositional(components: &[ComponentSpec]) -> CompositionalResult;
```

**RED tests (new file `phase_hw_compositional.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `comp_two_modules` | Producer-consumer: both verified |
| `comp_circular_discharged` | A assumes B, B assumes A → circular discharged |
| `comp_interface_mismatch` | A's guarantee doesn't match B's assumption → failed |
| `comp_three_chain` | A → B → C pipeline compositionally verified |
| `comp_single_module` | One component → same as monolithic |
| `comp_failed_component_identified` | Failing component named in result |
| `comp_trace_shows_interface` | Counterexample shows interface signal values |
| `comp_assume_checked` | Assumptions verified against provider guarantees |
| `comp_guarantee_proven` | Each guarantee proven under its assumptions |
| `comp_bitvec_interface` | Bitvector signals at component boundaries |
| `comp_integer_interface` | Integer counter passed between components |
| `comp_axi_master_slave` | AXI master and slave composed |
| `comp_producer_consumer_fifo` | Producer → FIFO → Consumer |
| `comp_arbiter_clients` | Arbiter with 3 clients composed |
| `comp_empty_components` | No components → trivially verified |
| `comp_shared_signal` | Signal visible to multiple components |
| `comp_temporal_guarantees` | Guarantee is a temporal property (G, F) |
| `comp_circular_three_way` | Three-way circular dependency |
| `comp_scales_better` | Compositional faster than monolithic (measured) |
| `comp_result_serializable` | Result round-trips through JSON |

---

### Sprint S1F: Strategy Selection Engine

**Why:** Users shouldn't choose between BMC, k-induction, IC3, and interpolation. The engine should pick automatically based on property structure.

**Files:** new `crates/logicaffeine_verify/src/strategy.rs`

**Depends on:** S1A, S1B, S1C, S1D

**What:**

```rust
pub enum Strategy {
    Bmc(u32),
    KInduction(u32),
    Ic3,
    Interpolation(u32),
    LivenessToSafety,
    Portfolio { strategies: Vec<Strategy>, timeout_each_ms: u64 },
}

pub fn select_strategy(property: &VerifyExpr, signals: &[SignalDecl]) -> Strategy;
pub fn verify_auto(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
) -> VerificationResult;
```

Heuristics:
- Contains `Eventually`/`GlobFinally` → `LivenessToSafety`
- Pure boolean, few signals → `KInduction(10)`
- Bitvector arithmetic → `Ic3` (handles bit-level reasoning)
- Large state space → `Portfolio` with timeout cascade

**RED tests (extend `phase_hw_strategy.rs`, ~10 tests):**

| Test | Assertion |
|---|---|
| `strategy_safety_selects_ic3` | Safety property → `Ic3` |
| `strategy_liveness_selects_l2s` | Liveness property → `LivenessToSafety` |
| `strategy_small_selects_kind` | Few signals → `KInduction` |
| `strategy_bounded_selects_bmc` | Explicitly bounded → `Bmc` |
| `strategy_auto_proves_safety` | Auto-selected strategy proves safety property |
| `strategy_auto_finds_bug` | Auto-selected strategy finds bug |
| `strategy_auto_proves_liveness` | Auto liveness verification |
| `strategy_portfolio_fallback` | First strategy times out, second succeeds |
| `strategy_portfolio_fastest_wins` | Portfolio returns first result |
| `strategy_deterministic` | Same property → same strategy |

**Tier 2 total: ~120 tests. Cumulative: ~762.**

---

## Part IV: Tier 3 -- Industrial Capabilities

### Sprint S2A: Multi-Clock Domain Modeling

**Why:** Real designs have multiple clock domains. CRUSH models a single clock. `posedge clk1` and `posedge clk2` run at independent rates.

**Files:** new `crates/logicaffeine_verify/src/multiclock.rs`

**What:**

```rust
pub struct ClockDomain {
    pub name: String,
    pub frequency: Option<u64>,       // Hz, if known
    pub ratio: Option<(u32, u32)>,    // Ratio to reference clock
}

pub struct MultiClockModel {
    pub domains: Vec<ClockDomain>,
    pub init: VerifyExpr,
    pub transitions: HashMap<String, VerifyExpr>,  // Per-domain transitions
    pub property: VerifyExpr,
}
```

Each domain unrolls independently. Cross-domain references use interleaved scheduling (all possible orderings of clock edges within a window).

**RED tests (new file `phase_hw_multiclock.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `multiclock_two_domains` | Fast and slow clock modeled independently |
| `multiclock_ratio_2_1` | 2:1 clock ratio → correct interleaving |
| `multiclock_independent_unroll` | Each domain has its own timestep sequence |
| `multiclock_cross_domain_read` | Signal in domain A read by domain B |
| `multiclock_safety_per_domain` | Property on domain A verified with A's clock |
| `multiclock_async_interface` | Asynchronous boundary modeled |
| `multiclock_all_interleavings` | All valid edge orderings explored |
| `multiclock_deterministic` | Same model → same result |
| `multiclock_single_domain_fallback` | One domain → same as CRUSH behavior |
| `multiclock_three_domains` | Three independent clocks |
| `multiclock_domain_from_kg` | Clock domain extracted from KG entity |
| `multiclock_spec_mentions_clocks` | "On every clk_a edge" and "on every clk_b edge" |
| `multiclock_bitvec_cross_domain` | Bitvector data crossing domain boundary |
| `multiclock_bmc_per_domain` | BMC bound applies per domain |
| `multiclock_ic3_multiclock` | IC3 on multi-clock design |
| `multiclock_counterexample_shows_domains` | Trace labels which domain each event belongs to |
| `multiclock_ratio_3_2` | 3:2 ratio interleaving correct |
| `multiclock_gated_clock` | Clock gating → conditional activation |
| `multiclock_serializable` | Model round-trips through JSON |
| `multiclock_regression` | Single-clock tests unchanged |

---

### Sprint S2B: CDC (Clock Domain Crossing) Formal Verification

**Why:** CDC bugs are among the hardest to find in simulation. Metastability, data coherence, and synchronizer correctness require formal analysis.

**Files:** new `crates/logicaffeine_compile/src/codegen_sva/cdc.rs`

**Depends on:** S2A, CRUSH Sprint 2A (Verilog parser)

**What:**

```rust
pub enum CdcPattern {
    TwoFlopSync { source_domain: String, dest_domain: String },
    GrayCode { width: u32 },
    HandshakeCdc { req: String, ack: String },
    PulseSynchronizer,
}

pub struct CdcReport {
    pub crossings: Vec<CdcCrossing>,
    pub violations: Vec<CdcViolation>,
    pub patterns: Vec<CdcPattern>,
}

pub fn analyze_cdc(rtl: &RtlModule, clock_domains: &[ClockDomain]) -> CdcReport;
```

**RED tests (new file `phase_hw_cdc.rs`, ~25 tests):**

| Test | Assertion |
|---|---|
| `cdc_two_flop_detected` | Recognizes standard 2-flop synchronizer |
| `cdc_missing_sync_flagged` | Direct cross-domain wire → violation |
| `cdc_gray_code_width_correct` | Gray code encoder width matches |
| `cdc_handshake_recognized` | Req/ack handshake CDC pattern detected |
| `cdc_pulse_sync_detected` | Pulse synchronizer pattern recognized |
| `cdc_no_crossing_clean` | Single-domain design → empty report |
| `cdc_multiple_crossings` | 3 crossings detected independently |
| `cdc_violation_includes_path` | Violation shows source → dest signal path |
| `cdc_reconvergence_detected` | Multi-bit signal crossing without gray code → violation |
| `cdc_formal_metastability` | Z3 verifies: 2-flop output stable after 2 cycles |
| `cdc_formal_data_coherence` | Z3 verifies: gray code never has >1 bit change |
| `cdc_formal_handshake_safe` | Z3 verifies: handshake protocol prevents data loss |
| `cdc_report_serializable` | JSON round-trip |
| `cdc_from_spec_and_rtl` | English spec + RTL → CDC analysis |
| `cdc_sva_generated` | CDC property → SVA assertion |
| `cdc_3_flop_sync` | 3-flop synchronizer (high-frequency) recognized |
| `cdc_fifo_crossing` | Async FIFO recognized as safe CDC pattern |
| `cdc_reset_crossing` | Reset signal crossing domain → special handling |
| `cdc_bus_crossing_gray` | Multi-bit bus with gray code → safe |
| `cdc_bus_crossing_direct` | Multi-bit bus without encoding → violation |
| `cdc_domain_inference` | Clock domains inferred from `always @(posedge)` blocks |
| `cdc_mixed_sync_async` | Design with both sync and async crossings |
| `cdc_feedback_path` | Bidirectional crossing (req/ack) → handshake pattern |
| `cdc_glitch_detection` | Combinational logic crossing → glitch risk flagged |
| `cdc_regression_single_clock` | Single-clock designs → no CDC analysis needed |

---

### Sprint S2C: Power-Aware Formal Verification

**Why:** Power domains introduce isolation, retention, and level shifting requirements. Missing isolation cells cause functional bugs in power-managed designs.

**Files:** new `crates/logicaffeine_compile/src/codegen_sva/power.rs`

**Depends on:** S2A

**RED tests (new file `phase_hw_power.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `power_isolation_required` | Signal crossing power boundary without isolation → flagged |
| `power_isolation_present` | Isolation cell detected → clean |
| `power_retention_preserved` | Retention register value survives power cycle |
| `power_sequence_correct` | Power-on sequence: isolation → clamp → power → release |
| `power_domain_modeling` | On/Off/Retention states modeled as VerifyExpr |
| `power_formal_isolation` | Z3: output clamped when source domain off |
| `power_formal_retention` | Z3: retained value matches pre-power-off value |
| `power_multiple_domains` | 3 power domains with different states |
| `power_always_on` | Always-on domain → no isolation needed (from it) |
| `power_level_shifter` | Voltage level crossing detected |
| `power_sva_generated` | Power property → SVA assertion |
| `power_report_serializable` | JSON round-trip |
| `power_spec_integration` | English spec: "when domain A is off, output X is clamped" |
| `power_no_power_mgmt` | Design without power domains → empty analysis |
| `power_regression` | Non-power tests unchanged |

---

### Sprint S2D: Security Property Verification

**Why:** Hardware security bugs (Spectre, Meltdown, Rowhammer) are found post-silicon. Formal non-interference verification catches them at design time.

**Files:** new `crates/logicaffeine_verify/src/security.rs`

**Depends on:** S0A

**What:**

```rust
pub enum SecurityLabel { Public, Secret }

pub struct TaintedSignal {
    pub name: String,
    pub label: SecurityLabel,
}

pub enum SecurityResult {
    NonInterference,                            // No secret → public information flow
    InformationLeak { path: Vec<String> },      // Taint propagation path
    TimingLeak { condition: VerifyExpr },        // Secret-dependent timing
    Unknown,
}

pub fn check_non_interference(
    model: &VerifyExpr,
    signals: &[TaintedSignal],
) -> SecurityResult;
```

Non-interference: two executions differing only in secret inputs must produce identical public outputs. Encoded as: `(init_1 == init_2 on public) AND (secret_1 != secret_2) AND transition_1 AND transition_2 AND (public_out_1 != public_out_2)` is UNSAT.

**RED tests (new file `phase_hw_security.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `security_simple_noninterference` | Public output independent of secret → `NonInterference` |
| `security_direct_leak` | `output = secret XOR mask` → `InformationLeak` |
| `security_taint_propagation` | Secret propagates through XOR chain → path shown |
| `security_timing_leak` | `if secret then 2-cycle else 1-cycle` → `TimingLeak` |
| `security_constant_time` | Operations take same cycles regardless of secret → safe |
| `security_bitvec_leak` | Single bit of secret leaks to public bus |
| `security_no_secret_signals` | All public → trivially `NonInterference` |
| `security_multiple_secrets` | Two secret inputs, one leaks → correct path |
| `security_transitive_taint` | `a = secret; b = a + 1; output = b` → leak detected |
| `security_mask_blocks_taint` | `output = (secret AND 0) OR public` → no leak |
| `security_mux_leak` | Secret used as mux select → `TimingLeak` |
| `security_register_isolation` | Secret in register, public read blocked → safe |
| `security_fifo_isolation` | FIFO separating secret and public domains → safe |
| `security_aes_sbox` | AES S-box: input is secret, output is secret (not public) |
| `security_speculative_leak` | Speculative execution leaks secret via cache → detected |
| `security_report_serializable` | JSON round-trip |
| `security_from_spec` | English: "the key must not influence the status output" |
| `security_sva_generated` | Non-interference → SVA property |
| `security_multiple_outputs` | Multiple public outputs, one leaks → identified |
| `security_regression` | Non-security tests unchanged |

---

### Sprint S2E: RISC-V ISA Formal Verification Templates

**Why:** RISC-V is the dominant open-source ISA. Pre-verified instruction semantics as templates. Users describe their CPU, templates generate SVA.

**Files:** new `crates/logicaffeine_compile/src/codegen_sva/protocols/riscv.rs`

**Depends on:** S0A, CRUSH Sprint 5A

**What:**

```rust
pub struct RiscvConfig {
    pub xlen: u32,                           // 32 or 64
    pub extensions: Vec<RiscvExtension>,     // I, M, A, F, D, C
    pub reg_prefix: String,                  // "x" or "r"
    pub pc_name: String,
    pub mem_name: String,
}

pub fn riscv_alu_properties(config: &RiscvConfig) -> Vec<SvaProperty>;
pub fn riscv_decoder_properties(config: &RiscvConfig) -> Vec<SvaProperty>;
pub fn riscv_register_properties(config: &RiscvConfig) -> Vec<SvaProperty>;
```

**RED tests (new file `phase_hw_riscv.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `riscv_add_semantics` | `rd = rs1 + rs2` verified for RV32I ADD |
| `riscv_sub_semantics` | `rd = rs1 - rs2` verified for RV32I SUB |
| `riscv_and_or_xor` | Bitwise operations verified |
| `riscv_slt_signed` | Set-less-than uses signed comparison |
| `riscv_sltu_unsigned` | Set-less-than-unsigned uses unsigned comparison |
| `riscv_decoder_mutual_exclusion` | At most one instruction type active per cycle |
| `riscv_x0_always_zero` | Register x0 reads as zero always |
| `riscv_x0_write_ignored` | Write to x0 has no effect |
| `riscv_pc_alignment` | PC always 4-byte aligned (no C extension) |
| `riscv_pc_alignment_c` | PC always 2-byte aligned (with C extension) |
| `riscv_beq_semantics` | Branch-if-equal: PC = target when rs1 == rs2 |
| `riscv_lw_sw_roundtrip` | Store word then load word → same value |
| `riscv_memory_alignment` | LW address 4-byte aligned |
| `riscv_sva_generated` | Templates produce valid SVA |
| `riscv_rv32_config` | RV32I configuration produces 32-bit properties |
| `riscv_rv64_config` | RV64I configuration produces 64-bit properties |
| `riscv_m_extension` | MUL/DIV properties generated for M extension |
| `riscv_z3_alu_verified` | Z3 confirms ALU properties are self-consistent |
| `riscv_parameterized_xlen` | Properties work for both 32 and 64 |
| `riscv_regression` | Protocol template tests unchanged |

---

### Sprint S2F: Parameterized Verification

**Why:** Verify properties for *any* bus width N, *any* FIFO depth D. Not just specific instances.

**Files:** new `crates/logicaffeine_verify/src/parameterized.rs`

**Depends on:** S0B

**What:**

```rust
pub enum ParameterizedResult {
    UniversallyValid,                         // Holds for all parameter values
    ValidUpTo(u64),                           // Holds for parameter <= bound
    Counterexample { param_value: u64, trace: Trace },
    Unknown,
}

pub fn verify_parameterized(
    property: &VerifyExpr,
    parameter: &str,
    param_type: VerifyType,
) -> ParameterizedResult;
```

Uses Z3 quantifiers: `forall N:Int. N > 0 -> property(N)`. Falls back to bounded enumeration if quantifier reasoning times out.

**RED tests (new file `phase_hw_parameterized.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `param_fifo_depth_any` | FIFO property holds for any depth > 0 |
| `param_bus_width_any` | Bus property holds for any width |
| `param_counter_width_specific` | Overflow at 2^N -- parameterized |
| `param_cutoff_found` | Property holds for N <= 8, fails for N = 9 |
| `param_counterexample_has_value` | Shows specific parameter value that fails |
| `param_quantifier_encoding` | `forall N. property(N)` encoded correctly |
| `param_fallback_bounded` | Quantifier timeout → bounded enumeration |
| `param_bitvec_width` | Parameterized bitvector width |
| `param_array_depth` | Parameterized array size |
| `param_multiple_params` | Two parameters: width and depth |
| `param_constraint_on_param` | Parameter must be power of 2 |
| `param_zero_excluded` | Parameter > 0 required |
| `param_result_serializable` | JSON round-trip |
| `param_regression` | Non-parameterized tests unchanged |
| `param_compared_to_concrete` | Parameterized agrees with concrete instances |

**Tier 3 total: ~115 tests. Cumulative: ~877.**

---

## Part V: Tier 4 -- Nobody Believes This

### Sprint S3A: Verified SVA Compiler via Futamura Projections

**Why:** LogicAffeine has all 3 Futamura projections operational (436 tests). The SVA synthesis pipeline (CRUSH Sprint 3A) is an interpreter that takes a spec and produces SVA. Specializing this interpreter for a fixed spec yields a compiled SVA generator. Specializing the specializer for the synthesis interpreter yields a *compiler*. The compiler's correctness is inherited from the projection framework.

**Files:** new `crates/logicaffeine_compile/src/codegen_sva/verified_compiler.rs`

**Depends on:** CRUSH Sprint 3A (FOL → SVA synthesis), Futamura infrastructure

**What:**

```rust
/// P1: Specialize SVA synthesis for a fixed spec → compiled generator
pub fn compile_sva_generator(spec: &str) -> CompiledGenerator;

/// P2: Specialize specializer for SVA synthesis → SVA compiler
pub fn compile_sva_compiler() -> SvaCompiler;

/// The key property: compiled output equals interpreted output
pub fn verify_compiler_correctness(spec: &str) -> bool {
    let interpreted = synthesize_sva_from_spec(spec, "clk");
    let compiled = compile_sva_generator(spec).generate("clk");
    interpreted == compiled  // Must be identical
}
```

**RED tests (new file `phase_hw_verified_compiler.rs`, ~25 tests):**

| Test | Assertion |
|---|---|
| `p1_compiled_equals_interpreted` | P1 output == interpreted output for simple spec |
| `p1_compiled_equals_interpreted_axi` | P1 output == interpreted output for AXI spec |
| `p1_compiled_equals_interpreted_mutex` | P1 output == interpreted output for mutex spec |
| `p1_generator_no_synthesis_overhead` | Compiled generator has no spec-parsing code |
| `p1_generator_deterministic` | Same spec → same generator |
| `p2_compiler_exists` | P2 produces an SVA compiler object |
| `p2_compiler_produces_valid_sva` | Compiler output parses as valid SVA |
| `p2_compiler_equals_p1` | For any spec, P2-compiler(spec) == P1-generator(spec) |
| `p2_compiler_no_interpreter_overhead` | No interpretation loop in compiled code |
| `p2_compiler_handles_temporal` | Temporal specs compiled correctly |
| `p2_compiler_handles_arithmetic` | Arithmetic specs compiled correctly |
| `p2_compiler_handles_bitvector` | Bitvector specs compiled correctly |
| `p2_compiler_z3_verified` | P2-compiler output Z3-equivalent to spec |
| `p1_z3_verified` | P1-generator output Z3-equivalent to spec |
| `compiler_correctness_certificate` | Correctness is provable, not just tested |
| `compiler_performance` | Compiled path faster than interpreted path |
| `compiler_no_llm_dependency` | Zero LLM calls in compiled pipeline |
| `compiler_round_trip` | English → Compiler → SVA → Z3 → equivalent |
| `compiler_multiple_specs` | Compiler handles 5 different specs |
| `compiler_error_on_unsupported` | Unsupported fragment → error, not wrong SVA |
| `compiler_apb_template` | APB spec → compiled → verified |
| `compiler_uart_template` | UART spec → compiled → verified |
| `compiler_serializable` | Compiled generator serializable to disk |
| `compiler_regression` | Non-compiler SVA synthesis unchanged |
| `compiler_the_trick` | Compiler-compiled-compiler produces same SVA (P3-level) |

---

### Sprint S3B: Self-Certifying Proof Certificates

**Why:** When LogicAffeine says "equivalent" or "safe", you currently must trust it. Self-certifying certificates can be independently verified by a third party without trusting LogicAffeine's implementation. The proof engine (`logicaffeine_proof`) already has `DerivationTree`, `InferenceRule`, and a certifier bridge. Extend to cover hardware verification results.

**Files:** new `crates/logicaffeine_verify/src/certificate.rs`, extend `crates/logicaffeine_proof/src/certifier.rs`

**Depends on:** S0A, proof engine

**What:**

```rust
pub struct ProofCertificate {
    pub claim: VerifyClaim,
    pub steps: Vec<ProofStep>,
    pub axioms: Vec<AxiomReference>,
    pub checkable: bool,                // Certificate is self-contained
}

pub enum VerifyClaim {
    Equivalent(VerifyExpr, VerifyExpr),
    Safe { property: VerifyExpr, bound: Option<u32> },
    Live { property: VerifyExpr },
    Inconsistent(Vec<VerifyExpr>),
}

pub fn generate_certificate(result: &VerificationResult) -> Option<ProofCertificate>;
pub fn verify_certificate(cert: &ProofCertificate) -> bool;
```

Certificate format:
1. **Claim:** What is being proven
2. **Axioms:** Z3 theory axioms used (array extensionality, bitvector arithmetic, etc.)
3. **Steps:** Sequence of inference rule applications, each checkable
4. **Root:** Final step derives the claim

**RED tests (new file `phase_hw_certificates.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `cert_generated_for_equivalence` | Equivalence proof produces certificate |
| `cert_generated_for_safety` | k-induction proof produces certificate |
| `cert_generated_for_ic3` | IC3 proof produces certificate |
| `cert_verifiable` | `verify_certificate()` returns true for valid cert |
| `cert_rejects_tampered` | Modified certificate → verification fails |
| `cert_rejects_wrong_claim` | Certificate for A, claim is B → fails |
| `cert_includes_all_axioms` | No implicit assumptions -- all axioms listed |
| `cert_steps_are_local` | Each step checkable from immediate predecessors |
| `cert_root_matches_claim` | Final step derives exactly the claim |
| `cert_boolean_proof` | Boolean equivalence certificate |
| `cert_integer_proof` | Integer arithmetic certificate |
| `cert_bitvec_proof` | Bitvector certificate |
| `cert_serializable_json` | Certificate round-trips through JSON |
| `cert_serializable_compact` | Compact binary format |
| `cert_independent_checker` | Certificate checkable without Z3 |
| `cert_for_non_equivalence` | Not-equivalent result includes counterexample in cert |
| `cert_size_proportional` | Certificate size proportional to proof complexity |
| `cert_no_cert_for_unknown` | `Unknown` result → no certificate |
| `cert_temporal_proof` | Temporal property certificate includes unrolling |
| `cert_regression` | Non-certificate tests unchanged |

---

### Sprint S3C: Reactive Synthesis from LTL

**Why:** Verification checks if a design *satisfies* a spec. Synthesis *constructs* a design that satisfies the spec. Given `G(req -> F(ack))`, synthesize a controller that always grants acknowledgments. No existing hardware verification tool synthesizes from English specifications.

**Files:** new `crates/logicaffeine_verify/src/synthesis.rs`, new `crates/logicaffeine_verify/src/automata.rs`

**Depends on:** S0A, S0B, CRUSH temporal infrastructure

**What:**

```rust
pub enum SynthesisResult {
    Realizable { controller: Circuit },           // Controller exists
    Unrealizable { counter_strategy: Strategy },  // Environment can force violation
    Unknown,
}

pub struct Circuit {
    pub inputs: Vec<SignalDecl>,       // Environment signals
    pub outputs: Vec<SignalDecl>,      // Controller signals
    pub states: Vec<String>,
    pub init: String,
    pub transitions: Vec<(String, VerifyExpr, String, Vec<(String, VerifyExpr)>)>,
    // (from_state, guard, to_state, output_assignments)
}

pub fn synthesize_from_ltl(
    spec: &VerifyExpr,
    inputs: &[SignalDecl],
    outputs: &[SignalDecl],
) -> SynthesisResult;
```

Pipeline: LTL → Buchi automaton → Parity game → Winning strategy → Circuit → SVA

**RED tests (new file `phase_hw_synthesis.rs`, ~30 tests):**

| Test | Assertion |
|---|---|
| `synth_simple_buffer` | `G(req -> X(ack))` → 1-state controller |
| `synth_mutex_arbiter` | `G(not(grant_a AND grant_b))` with fairness → arbiter |
| `synth_controller_satisfies_spec` | Synthesized controller verified against spec |
| `synth_unrealizable_detected` | `G(ack) AND G(not(ack))` → `Unrealizable` |
| `synth_counter_strategy` | Unrealizable → counter-strategy shows how env wins |
| `synth_two_inputs_one_output` | Controller responds to two input signals |
| `synth_three_state_fsm` | Spec requires 3-state controller |
| `synth_liveness_controller` | `G(F(ack))` → controller that always eventually acks |
| `synth_response_bounded` | `G(req -> X(X(ack)))` → 2-cycle response controller |
| `synth_priority_arbiter` | Higher-priority-first arbiter synthesized |
| `synth_circuit_to_sva` | Synthesized circuit → SVA monitor |
| `synth_circuit_to_verilog` | Synthesized circuit → Verilog RTL |
| `synth_deterministic` | Same spec → same circuit |
| `synth_minimal_states` | Controller has minimal state count |
| `synth_bitvec_signals` | Synthesis with bitvector inputs/outputs |
| `synth_from_english` | English spec → LTL → synthesis |
| `synth_axi_handshake` | AXI handshake controller synthesized |
| `synth_round_robin` | Round-robin arbiter from fairness spec |
| `synth_pipeline_controller` | Pipeline stall controller synthesized |
| `synth_empty_spec` | Trivial spec → trivial controller |
| `synth_contradictory_spec` | Contradictory → `Unrealizable` |
| `synth_env_assumption` | Spec with environment assumption |
| `synth_multiple_outputs` | Controller manages 4 output signals |
| `synth_verified_by_bmc` | Synthesized controller passes BMC(100) |
| `synth_verified_by_ic3` | Synthesized controller passes IC3 |
| `synth_circuit_serializable` | Circuit round-trips through JSON |
| `synth_verilog_compilable` | Generated Verilog passes syntax check |
| `synth_performance` | Small spec synthesized within 10s |
| `synth_regression` | Non-synthesis tests unchanged |
| `synth_the_full_pipeline` | English → Kripke → LTL → synthesis → Verilog → SVA → Z3 ✓ |

---

### Sprint S3D: Automatic Abstraction for Infinite-State Systems

**Why:** Bitvector arithmetic creates enormous state spaces. Predicate abstraction reduces infinite-state to finite-state while preserving relevant properties. CEGAR loop refines if abstraction is too coarse.

**Files:** new `crates/logicaffeine_verify/src/abstraction.rs`

**Depends on:** S1B (IC3), S0A

**What:**

```rust
pub struct AbstractModel {
    pub predicates: Vec<VerifyExpr>,
    pub abstract_init: VerifyExpr,
    pub abstract_transition: VerifyExpr,
}

pub enum AbstractionResult {
    Safe,
    Unsafe { concrete_trace: Trace },
    SpuriousRefined { new_predicates: Vec<VerifyExpr> },
    Unknown,
}

pub fn abstract_model(
    concrete: &VerifyExpr,
    predicates: &[VerifyExpr],
) -> AbstractModel;

pub fn cegar_verify(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    initial_predicates: &[VerifyExpr],
    max_refinements: u32,
) -> AbstractionResult;
```

**RED tests (new file `phase_hw_abstraction.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `abstract_reduces_state_space` | 32-bit counter → boolean predicates |
| `abstract_preserves_safety` | Safe property still safe after abstraction |
| `abstract_spurious_detected` | Abstract counterexample not realizable in concrete |
| `abstract_refined` | Spurious → new predicate added → refined |
| `cegar_converges_simple` | CEGAR loop terminates on simple design |
| `cegar_finds_real_bug` | Real bug → concrete counterexample |
| `cegar_refinement_correct` | Each refinement eliminates the spurious trace |
| `cegar_predicate_minimal` | Refinement adds minimal predicates |
| `cegar_bitvec_abstracted` | 32-bit bitvector abstracted to sign/zero predicates |
| `cegar_array_abstracted` | Array contents abstracted to relevant indices |
| `cegar_combined_with_ic3` | Abstraction feeds IC3 for unbounded check |
| `cegar_max_refinements` | Respects max_refinements bound |
| `cegar_timeout` | Complex design → graceful timeout |
| `abstract_model_serializable` | JSON round-trip |
| `abstract_single_predicate` | One predicate → boolean abstraction |
| `abstract_many_predicates` | 10 predicates → 2^10 abstract states |
| `cegar_pipeline` | Pipeline design abstracted and verified |
| `cegar_counter_overflow` | Counter overflow detected via abstraction |
| `cegar_regression` | Non-abstraction tests unchanged |
| `cegar_compared_to_concrete` | CEGAR agrees with concrete verification |

---

### Sprint S3E: SMT-LIB2 Export

**Why:** Interoperability. Export VerifyExpr to standard SMT-LIB2 format readable by Z3, CVC5, Yices, Bitwuzla, Boolector. Enables users to independently verify with their preferred solver.

**Files:** new `crates/logicaffeine_verify/src/smtlib.rs`

**Depends on:** S0A, S0B

**What:**

```rust
pub fn to_smtlib2(expr: &VerifyExpr, declarations: &[(&str, VerifyType)]) -> String;
pub fn equivalence_to_smtlib2(a: &VerifyExpr, b: &VerifyExpr) -> String;
```

Output is a valid `.smt2` file with `(set-logic ALL)`, `(declare-fun ...)`, `(assert ...)`, `(check-sat)`, `(get-model)`.

**RED tests (new file `phase_hw_smtlib.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `smtlib_bool_formula` | `And(Var("p"), Not(Var("q")))` → valid SMT-LIB2 |
| `smtlib_int_formula` | `Gt(Add(Var("x"), Int(5)), Int(10))` → valid SMT-LIB2 |
| `smtlib_bitvec_formula` | `BvAdd(Var("x"), BvConst(8, 5))` → `(bvadd x #x05)` |
| `smtlib_array_formula` | `Select(Var("a"), Int(0))` → `(select a 0)` |
| `smtlib_quantifier` | `ForAll{x:Int, x > 0}` → `(forall ((x Int)) (> x 0))` |
| `smtlib_declarations` | All variables declared with correct sorts |
| `smtlib_equivalence_query` | Equivalence → `(assert (not (= a b)))` + `(check-sat)` |
| `smtlib_parseable_by_z3` | Output accepted by Z3 command-line (if available) |
| `smtlib_nested_bitvec` | Nested bitvector ops produce valid S-expressions |
| `smtlib_mixed_sorts` | Bool + Int + BV in one formula |
| `smtlib_iff` | Biconditional → `(= a b)` for booleans |
| `smtlib_apply` | Uninterpreted function → `(f arg1 arg2)` |
| `smtlib_temporal_unrolled` | Temporal formula pre-unrolled → flat SMT-LIB2 |
| `smtlib_round_trip` | Export → Z3 parse → same satisfiability |
| `smtlib_regression` | Non-export tests unchanged |

**Tier 4 total: ~110 tests. Cumulative: ~987.**

---

## Part VI: Tier 5 -- Ecosystem

### Sprint S4A: CI/CD Integration

**Why:** Formal verification must run automatically on every PR. SARIF format integrates with GitHub Security tab.

**Files:** new `crates/logicaffeine_compile/src/codegen_sva/ci.rs`

**What:**

```rust
pub struct CiReport {
    pub sarif: serde_json::Value,        // SARIF 2.1.0 output
    pub summary: String,                  // Human-readable summary
    pub properties_checked: usize,
    pub properties_passed: usize,
    pub properties_failed: usize,
    pub duration_ms: u64,
}

pub fn run_ci_verification(
    spec_files: &[&str],
    rtl_files: &[&str],
    config: &CiConfig,
) -> CiReport;
```

**RED tests (new file `phase_hw_ci.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `ci_sarif_valid` | Output conforms to SARIF 2.1.0 schema |
| `ci_sarif_has_results` | Each property → one SARIF result |
| `ci_pass_result` | Passing property → `result.level = "note"` |
| `ci_fail_result` | Failing property → `result.level = "error"` |
| `ci_summary_readable` | Human-readable summary includes counts |
| `ci_duration_tracked` | Duration > 0 and plausible |
| `ci_multiple_specs` | Multiple spec files processed |
| `ci_empty_spec` | No specs → empty report (no crash) |
| `ci_counterexample_in_sarif` | Failing property includes counterexample in message |
| `ci_property_location` | SARIF result includes spec file location |
| `ci_changed_files_only` | Config: only check properties for changed files |
| `ci_exit_code` | All pass → 0, any fail → 1 |
| `ci_report_serializable` | JSON round-trip |
| `ci_template_generated` | GitHub Actions workflow template produced |
| `ci_regression` | Non-CI tests unchanged |

---

### Sprint S4B: Incremental Verification

**Why:** Re-verifying everything on every change is wasteful. Track dependencies, cache results, invalidate only what changed.

**Files:** new `crates/logicaffeine_verify/src/incremental.rs`

**Depends on:** CRUSH Sprint 2B (RTL KG linking)

**What:**

```rust
pub struct VerificationCache {
    pub entries: HashMap<PropertyHash, CachedResult>,
}

pub struct CachedResult {
    pub result: VerificationResult,
    pub dependencies: Vec<DependencyHash>,    // Spec sentences, RTL signals
    pub timestamp: u64,
}

pub fn verify_incremental(
    properties: &[VerifyExpr],
    changed: &[ChangeEvent],
    cache: &mut VerificationCache,
) -> Vec<VerificationResult>;
```

**RED tests (new file `phase_hw_incremental.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `incr_unchanged_cached` | Unchanged property → cache hit, no Z3 call |
| `incr_changed_spec_invalidates` | Changed spec sentence → dependent properties re-verified |
| `incr_changed_rtl_invalidates` | Changed RTL signal → dependent properties re-verified |
| `incr_cache_hit_same_result` | Cached result == fresh result |
| `incr_new_property_verified` | New property not in cache → verified fresh |
| `incr_removed_property_evicted` | Removed property → evicted from cache |
| `incr_dependency_tracking` | Each result tracks its dependencies |
| `incr_transitive_invalidation` | A depends on B depends on C, C changes → A invalidated |
| `incr_partial_invalidation` | Change affects 2 of 5 properties → only 2 re-verified |
| `incr_cache_serializable` | Cache round-trips through disk |
| `incr_cache_load_restore` | Save cache → load → same results |
| `incr_empty_cache` | Empty cache → all properties verified fresh |
| `incr_all_changed` | Everything changed → same as fresh verification |
| `incr_performance_benefit` | Incremental faster than full (measured) |
| `incr_hash_deterministic` | Same property → same hash |
| `incr_hash_collision_safe` | Different properties with same hash → re-verified |
| `incr_bitvec_property_cached` | Bitvector property caching works |
| `incr_temporal_property_cached` | Temporal property caching works |
| `incr_regression` | Non-incremental tests unchanged |
| `incr_cache_eviction` | Cache size limit → LRU eviction |

---

### Sprint S4C: Formal-to-Simulation Bridge

**Why:** Counterexamples should drive simulation. Extract directed test vectors from Z3 traces. Generate SystemVerilog testbenches.

**Files:** new `crates/logicaffeine_compile/src/codegen_sva/testgen.rs`

**Depends on:** CRUSH Sprint 6A, S0D

**What:**

```rust
pub fn trace_to_testbench(trace: &Trace, module_name: &str) -> String;
pub fn trace_to_stimulus(trace: &Trace, signals: &[SignalDecl]) -> String;
```

Output: SystemVerilog testbench with `initial begin ... end` block driving signals per counterexample cycle.

**RED tests (new file `phase_hw_testgen.rs`, ~15 tests):**

| Test | Assertion |
|---|---|
| `testgen_module_instantiation` | Testbench instantiates DUT with correct ports |
| `testgen_clock_generation` | Clock driver with correct period |
| `testgen_stimulus_per_cycle` | Each trace cycle → signal assignments at correct time |
| `testgen_boolean_signal` | Boolean signal → `1'b0` / `1'b1` |
| `testgen_bitvec_signal` | 8-bit signal → `8'hAB` |
| `testgen_integer_signal` | Integer signal → decimal literal |
| `testgen_multi_cycle` | 10-cycle trace → 10 clock edges of stimulus |
| `testgen_valid_systemverilog` | Output is syntactically valid SystemVerilog |
| `testgen_display_violation` | `$display` at violation cycle |
| `testgen_finish_after_trace` | `$finish` after last cycle |
| `testgen_multiple_signals` | 5 signals driven in testbench |
| `testgen_empty_trace` | Empty trace → minimal testbench (no stimulus) |
| `testgen_counterexample_driven` | Testbench reproduces the counterexample scenario |
| `testgen_serializable` | Generated testbench is a plain string |
| `testgen_regression` | Non-testgen tests unchanged |

**Tier 5 total: ~50 tests. Cumulative: ~1,037.**

---

## Part VII: Sprint Sequencing

```
CRUSH COMPLETE (532+ tests, boolean-only)
    |
    +-- S0A (multi-sorted equivalence) <-- CRITICAL, DO FIRST
    |       |
    |       +-- S0B (quantifiers)
    |       |       +-- S2F (parameterized verification)
    |       |       +-- S3E (SMT-LIB2 export)
    |       |       +-- S3C (reactive synthesis) [also needs S0A]
    |       |
    |       +-- S0C (BoundedExpr extension)
    |       +-- S0D (multi-sorted traces)
    |       |       +-- S4C (formal-to-simulation bridge) [also needs CRUSH 6A]
    |       |
    |       +-- S0E (type inference)
    |       +-- S0F (regression gate)
    |       |
    |       +-- S1A (k-induction)
    |       |       +-- S1D (liveness-to-safety)
    |       |
    |       +-- S1B (IC3/PDR) [also needs S0B]
    |       |       +-- S3D (automatic abstraction)
    |       |
    |       +-- S1C (interpolation)
    |       |
    |       +-- S1E (assume-guarantee) [also needs CRUSH 6C]
    |       |
    |       +-- S1F (strategy selection) [needs S1A, S1B, S1C, S1D]
    |       |
    |       +-- S2A (multi-clock)
    |       |       +-- S2B (CDC) [also needs CRUSH 2A]
    |       |       +-- S2C (power-aware)
    |       |
    |       +-- S2D (security properties)
    |       +-- S2E (RISC-V templates) [also needs CRUSH 5A]
    |       |
    |       +-- S3A (verified compiler) [also needs CRUSH 3A, Futamura]
    |       +-- S3B (proof certificates) [also needs proof engine]
    |
    +-- S4A (CI/CD integration) [independent, can start after CRUSH]
    +-- S4B (incremental verification) [needs CRUSH 2B]
```

**Critical path:** S0A → S1B → S1F (longest chain to full algorithmic capability)

**Parallelizable after S0A:** S0B through S0F, S1A, S1C, S2A, S2D, S3A, S3B are all independent of each other.

---

## Part VIII: Test Count Summary

| Sprint | New Tests | Cumulative | New Files |
|---|---|---|---|
| **CRUSH baseline** | **532** | **532** | **13 source, 13 test** |
| S0A: Multi-sorted equivalence | +40 | 572 | equiv rewrite |
| S0B: Quantifiers | +15 | 587 | -- |
| S0C: BoundedExpr extension | +20 | 607 | -- |
| S0D: Multi-sorted traces | +15 | 622 | -- |
| S0E: Type inference | +15 | 637 | `type_infer.rs`, `phase_hw_type_infer.rs` |
| S0F: Regression gate | +5 | 642 | -- |
| S1A: k-Induction | +25 | 667 | `kinduction.rs`, `phase_hw_kinduction.rs` |
| S1B: IC3/PDR | +30 | 697 | `ic3.rs`, `phase_hw_ic3.rs` |
| S1C: Interpolation | +20 | 717 | `interpolation.rs`, `phase_hw_interpolation.rs` |
| S1D: Liveness | +15 | 732 | `liveness.rs`, `phase_hw_liveness.rs` |
| S1E: Assume-Guarantee | +20 | 752 | `compositional.rs`, `phase_hw_compositional.rs` |
| S1F: Strategy selection | +10 | 762 | `strategy.rs`, `phase_hw_strategy.rs` |
| S2A: Multi-clock | +20 | 782 | `multiclock.rs`, `phase_hw_multiclock.rs` |
| S2B: CDC | +25 | 807 | `cdc.rs`, `phase_hw_cdc.rs` |
| S2C: Power-aware | +15 | 822 | `power.rs`, `phase_hw_power.rs` |
| S2D: Security | +20 | 842 | `security.rs`, `phase_hw_security.rs` |
| S2E: RISC-V | +20 | 862 | `riscv.rs`, `phase_hw_riscv.rs` |
| S2F: Parameterized | +15 | 877 | `parameterized.rs`, `phase_hw_parameterized.rs` |
| S3A: Verified compiler | +25 | 902 | `verified_compiler.rs`, `phase_hw_verified_compiler.rs` |
| S3B: Proof certificates | +20 | 922 | `certificate.rs`, `phase_hw_certificates.rs` |
| S3C: Reactive synthesis | +30 | 952 | `synthesis.rs`, `automata.rs`, `phase_hw_synthesis.rs` |
| S3D: Abstraction | +20 | 972 | `abstraction.rs`, `phase_hw_abstraction.rs` |
| S3E: SMT-LIB2 | +15 | 987 | `smtlib.rs`, `phase_hw_smtlib.rs` |
| S4A: CI/CD | +15 | 1002 | `ci.rs`, `phase_hw_ci.rs` |
| S4B: Incremental | +20 | 1022 | `incremental.rs`, `phase_hw_incremental.rs` |
| S4C: Formal-to-simulation | +15 | 1037 | `testgen.rs`, `phase_hw_testgen.rs` |

**Final state: 1,037+ hardware verification tests. 22 new source modules. 22 new test files.**

**Combined with CRUSH: 35 source modules. 35 test files. Zero LLM dependencies in core pipeline.**

---

## Part IX: Academic Positioning

> **Title:** LogicAffeine: From English to Silicon -- Formally Verified Hardware Specification, Synthesis, and Unbounded Property Checking with Self-Certifying Proofs
>
> We present LogicAffeine, a hardware verification system that unifies English specification parsing, Kripke-grounded temporal logic, multi-sorted Z3 verification (Bool + Int + BitVec + Array), and reactive synthesis into an end-to-end pipeline from natural language to correct-by-construction hardware. Building on our prior work establishing formal SVA synthesis with boolean equivalence proofs, we extend the verification core to support full SMT theories -- bitvector arithmetic with overflow semantics, array theory for memory modeling, and quantifier reasoning for parameterized designs. Novel contributions: (1) multi-sorted specification equivalence checking with Z3 -- the first system to verify SVA with bitvector semantics against English specifications, (2) IC3/PDR integrated with specification-level reasoning for unbounded verification against intent, not just implementation, (3) reactive synthesis from natural language specifications -- constructing correct-by-construction circuits from English, (4) Futamura-verified SVA compiler where compiler correctness is inherited from the projection framework, (5) self-certifying proof certificates independently verifiable without trusting LogicAffeine, (6) formal CDC verification grounded in specification, and (7) security property verification with non-interference proofs for hardware designs.

### Competitive Position After SUPERCRUSH

| Capability | JasperGold | SymbiYosys | Questa | OneSpin | AssertionForge | **LogicAffeine** |
|---|---|---|---|---|---|---|
| **Spec → SVA proof** | Never | Never | Never | Never | Never | **Multi-sorted Z3** |
| **Unbounded safety** | IC3 | BMC only | IC3 | IC3 | Never | **IC3/PDR + k-ind** |
| **Liveness** | Yes | Limited | Yes | Yes | Never | **L2S reduction** |
| **Reactive synthesis** | No | No | No | No | No | **LTL game solving** |
| **Proof certificates** | No | No | No | No | No | **Self-certifying** |
| **CDC formal** | Separate tool | No | Separate tool | Separate tool | No | **Integrated** |
| **Security formal** | No | No | No | No | No | **Non-interference** |
| **Bitvector equiv** | N/A | N/A | N/A | N/A | No | **Z3 BV theory** |
| **Array reasoning** | N/A | N/A | N/A | N/A | No | **Z3 Array theory** |
| **Parameterized** | Limited | No | Limited | No | No | **Quantifier-based** |
| **Spec coverage** | No | No | No | No | No | **KG-driven** |
| **From English** | No | No | No | No | LLM (no proof) | **FOL + Kripke** |
| **Open-source core** | No | Yes | No | No | No | **Yes** |
| **RISC-V templates** | Third-party | Community | Third-party | No | No | **Built-in, Z3-verified** |
| **CI/CD native** | Jenkins plugin | Yes | Jenkins plugin | No | No | **SARIF + GitHub Actions** |
| **HW test count** | N/A | N/A | N/A | N/A | 0 | **1,037+** |

---

## Part X: Risk Assessment and Honest Limitations

### What SUPERCRUSH Does Not Attempt

| Out of Scope | Why |
|---|---|
| Gate-level equivalence checking | Different problem (OneSpin's domain). We verify spec↔SVA, not RTL↔gates. |
| Analog/mixed-signal verification | Requires SPICE-level modeling. Different tool category entirely. |
| Full RTL model checking (SVA↔RTL) | JasperGold/SymbiYosys do this. We verify SVA↔spec. Orthogonal, complementary. |
| Synthesis to FPGA/ASIC netlist | Vivado/Quartus/Design Compiler. Different layer of the stack. |
| Timing closure / STA | Requires physical design information. Post-layout concern. |
| Formal equivalence (RTL↔netlist) | Conformal/Formality. Different problem. |
| Emulation / hardware acceleration | Palladium/Veloce. Physical infrastructure, not software. |

### Technical Risks

| Risk | Severity | Mitigation |
|---|---|---|
| IC3 implementation complexity (~2000 LOC) | High | Well-studied algorithm. Reference implementations exist (IC3Ref, ABC). Incremental delivery: basic IC3 first, optimizations later. |
| Reactive synthesis is exponential in spec size | High | Practical only for small specs (< 10 temporal operators). State this upfront. Useful for controller sketches, not full designs. |
| Z3 quantifier performance | Medium | Quantifier instantiation strategies (E-matching, MBQI). Fall back to bounded enumeration. Honest about timeouts. |
| Liveness L2S state space blowup | Medium | Doubled state space. Mitigated by combining with IC3 (which handles large state spaces). |
| Interpolation support in Z3 Rust bindings | Medium | May need custom interpolation via proof extraction. Fall back to IC3 if unavailable. |
| Predicate abstraction refinement loop may not converge | Low | Bounded max_refinements. CEGAR is well-studied. Report `Unknown` if non-convergent. |
| SMT-LIB2 cross-solver compatibility | Low | Stick to standardized theories (QF_BV, QF_LIA, QF_AUFBV). Test with Z3 command-line. |

### Mitigation Strategy

Each tier is independently valuable:

- **After Tier 1:** Full multi-sorted verification. Already beyond every competitor for spec↔SVA checking.
- **After Tier 2:** Unbounded verification. Matches JasperGold's algorithmic portfolio.
- **After Tier 3:** Industrial features (CDC, power, security, RISC-V). No single tool covers all of these.
- **After Tier 4:** Features that don't exist anywhere. Verified compiler. Reactive synthesis from English. Proof certificates.
- **After Tier 5:** Developer-ready ecosystem. CI/CD, incremental, testbench generation.

If Tier 4 proves impractical at scale, everything through Tier 3 is solid engineering with known algorithmic foundations and peer-reviewed literature. No tier depends on speculative technology.

---

## Part XI: The Claim

After SUPERCRUSH:

1. **1,037+ hardware verification tests.** Every one automated. Every one reproducible. Zero LLM dependency in the core pipeline.
2. **Full SMT theory support.** Bool + Int + BitVec + Array + Quantifiers. Not behind overapproximation. Not stubbed. End-to-end through the pipeline.
3. **Unbounded verification.** k-Induction, IC3/PDR, interpolation, liveness-to-safety. Properties proven for all time, not just bounded.
4. **Capabilities nobody has.** Verified SVA compiler via Futamura. Self-certifying proof certificates. Reactive synthesis from English. Formal CDC integrated with specification. Security non-interference for hardware.
5. **Industrial scope.** Multi-clock, power-aware, RISC-V, parameterized, CDC, security. Not a research prototype.
6. **Developer ecosystem.** CI/CD with SARIF. Incremental verification. Testbench generation from counterexamples.
7. **Honest.** Every limitation disclosed. Every risk assessed. No gate-level. No analog. No netlist synthesis. Clear about what we do and don't do.

The only hardware verification system that goes from English to formally verified silicon properties with proof certificates. The only one that can synthesize correct-by-construction controllers from natural language. The only one where the compiler itself is verified.

People won't believe it because nobody has tried to do all of this in one system. That's not because it's impossible. It's because nobody had the foundation. We do: Kripke semantics, Futamura projections, a multi-sorted IR with Z3 encoding, a proof engine with Curry-Howard certification, and 532+ tests proving the propositional fragment works.

SUPERCRUSH is what happens when you stop hiding behind "future work" and build the future.
