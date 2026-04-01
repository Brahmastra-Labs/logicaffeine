# CRUSH ASSERTIONFORGE

Engineering specification for LogicAffeine's hardware verification pipeline. Every claim verified against source code. Every weakness disclosed. Every sprint has concrete RED tests. Designed for handoff to an implementing programmer.

---

## Part I: The Competitive Landscape

### What AssertionForge Does (LAD 2025, Stanford)

Python prototype (~5,000 LOC). Pipeline:

1. **Spec Processing:** PDF -> PyPDF2 -> GraphRAG with 180-line hardware prompt. 35+ entity types (Port, Register, FIFO, Clock, Interrupt, Controller, Arbiter, Decoder, Mux, Counter, FSM, Bus, Memory, Signal, Module...). 59+ relation types (DataFlow, ControlFlow, Drives, Reads, Writes, Arbitrates, connectsTo, configures, generatesInterrupt, triggersOperation...).
2. **RTL Analysis:** `RTLAnalyzer` (Python, 2200+ LOC). Module/port/signal/FSM extraction. Clock domain detection. Protocol pattern detection (handshake, pipeline). Verification suggestion generation.
3. **KG Linking:** Text similarity heuristics + regex matching between spec KG and RTL KG.
4. **Context Generation:** Six strategies -- RAG, BFS expansion, guided random walk with gateway nodes, path-based traversal, motif detection, community detection. Token-budgeted dynamic prompts.
5. **SVA Generation:** GPT-4o text -> nine regex extraction strategies. No grammar, no AST.
6. **Verification:** Optional JasperGold (functional/statement/toggle/expression/branch coverage).

### What Commercial Tools Do (JasperGold, Questa, OneSpin)

| Tool | Key Capability We Lack |
|---|---|
| JasperGold | Property decomposition with proof strategies. Coverage convergence analysis. |
| Questa Formal | Assertion synthesis from coverage holes. Bug hunting mode. |
| OneSpin | Operational SVA from RTL analysis. Gate-level equivalence. |
| SymbiYosys | Bounded/unbounded strategy selection. Open-source BMC. |
| **All of them** | Check SVA against RTL implementation. None check SVA against specification. |

### What Nobody Does

- Formally verify that an SVA correctly expresses the specification.
- Synthesize SVA from formal specification without LLM involvement.
- Generate specification-level counterexample waveforms.
- Automatically discover invariants from formal knowledge graphs.
- Analyze property sufficiency against specification before running verification.

**These are our contributions.**

### What LogicAffeine Has (Audited, 2026-03-31)

**Genuinely working (verified by tests):**

| Component | Evidence | Tests |
|---|---|---|
| English -> FOL parser with Kripke semantics | `compile_kripke()` produces world-quantified FOL | 20 |
| LTL: Always (G), Eventually (F), Next (X), Until (U) | Correct Kripke lowering to accessibility predicates | 14 |
| SVA parser: 21 `SvaExpr` variants | Recursive descent, full round-trip | 58 |
| SVA -> BoundedExpr (all 21 variants) | Correct temporal semantics per timestep | 22 |
| BoundedExpr -> VerifyExpr bridge | Maps to Z3-ready IR | 10 |
| Z3 boolean equivalence | `not(A <-> B)` satisfiability + counterexample extraction | 20 |
| Z3 BMC temporal safety | Initial + transition + property violation detection | 2 |
| Knowledge graph from Kripke FOL | Signals, properties, edges (Triggers/Constrains) | 14 |
| SVA emission (Assert/Cover/Assume) | SystemVerilog, PSL, Rust monitor formats | 7 |
| Bitvector/Array IR + Z3 encoding | Types declared, encoder wired | 8 |
| **Total** | **289 tests, 16 files, 3,799 LOC** | |

**Known limitations (must fix):**

| Issue | Location | Sprint |
|---|---|---|
| `encode_to_z3()` overapproximation -- non-boolean ops become `true`, produces false positives | `equivalence.rs:237` | 0A |
| `FolTranslator` catch-all -- unhandled variants silently become `Bool(true)` | `fol_to_verify.rs:203` | 0A |
| Quantifier stub -- ForAll/Exists drop bound variables | `solver.rs:795` | 0B |
| KG signals all `width=1, role=Internal` -- no role/width inference | `knowledge_graph.rs` | 0C |
| KG only 2 edge patterns (Triggers, Constrains) | `knowledge_graph.rs` | 0E |
| Release/WeakUntil in enum but not lowered | `kripke.rs` | 0D |
| Bitvector/Array equivalence untested end-to-end | `equivalence.rs` | 0A |
| KG has 4 signal roles -- AssertionForge has 35+ entity types | `knowledge_graph.rs` | 0E |

---

## Part II: The Honest Comparison

| Capability | AssertionForge | JasperGold | LogicAffeine (now) | LogicAffeine (after) |
|---|---|---|---|---|
| **Spec formalization** | None | None | **FOL + Kripke** | FOL + Kripke (full LTL) |
| **Entity types** | 35 (LLM labels) | N/A | 4 (skeletal) | **28+ (formal, attributed)** |
| **Relation types** | 59 (string labels) | N/A | 2 (Triggers, Constrains) | **24+ (parameterized)** |
| **SVA parsing** | 9 regex | N/A | **21-variant AST** | 28+ variant AST |
| **SVA synthesis** | GPT-4o | N/A | Manual | **Formal from FOL** |
| **SVA <-> Spec proof** | Never | Never | **Z3 boolean** (buggy) | Z3 boolean+integer (sound) |
| **SVA <-> RTL proof** | Never | Yes | Via Z3 BMC | Via Z3 BMC |
| **Counterexample** | None | RTL traces | **Z3 traces** | Traces + VCD waveforms |
| **Property consistency** | No | Yes | No | **Z3-based** |
| **Spec decomposition** | No | Manual | No | **Automated + Z3-verified** |
| **Coverage metric** | Against RTL | Against RTL | None | **Against specification** |
| **Invariant discovery** | No | Limited | No | **KG-driven + Z3-verified** |
| **Sufficiency analysis** | No | No | No | **Formal gap detection** |
| **CEGAR refinement** | No | No | No | **Automated** |
| **Protocol templates** | LLM-gen | Built-in | AXI tests | **Z3-proven** |
| **HW test count** | 0 | N/A | **289** | **532+** |

---

## Part III: Sprint Specification

Every sprint follows TDD: write RED tests first, implement until GREEN, run full suite for zero regressions.

```bash
# RED: confirm new tests fail
cargo test --no-fail-fast --test <file> -- --skip e2e > /tmp/red.txt 2>&1; echo "EXIT: $?" >> /tmp/red.txt

# GREEN: implement until pass
cargo test --no-fail-fast --test <file> -- --skip e2e > /tmp/green.txt 2>&1; echo "EXIT: $?" >> /tmp/green.txt

# REGRESSION: all tests pass
cargo test --no-fail-fast -- --skip e2e > /tmp/all.txt 2>&1; echo "EXIT: $?" >> /tmp/all.txt

# Z3: feature-gated tests
cargo test --features verification --no-fail-fast -- --skip e2e > /tmp/z3.txt 2>&1; echo "EXIT: $?" >> /tmp/z3.txt
```

---

### Sprint 0A: Eliminate Z3 False Positives

**Why:** `encode_to_z3()` returns `true` for non-boolean ops. `x > 5` and `y < 10` both become `true` so Z3 says they're equivalent. Academically indefensible.

**Files:** `crates/logicaffeine_verify/src/equivalence.rs`, `crates/logicaffeine_compile/src/codegen_sva/fol_to_verify.rs`

**What:** Replace catch-all with explicit Int encoding. Uninterpreted functions get unique Z3 function symbols, not `true`. Reject unsupported variants with `EquivalenceResult::Unknown` instead of silently overapproximating.

**RED tests (extend `phase_hw_z3_equiv.rs`):**

| Test | Assertion |
|---|---|
| `z3_does_not_overapproximate_arithmetic` | `Gt(x,5)` not-equiv `Lt(y,3)` |
| `z3_integer_equality_works` | `Eq(x,5)` equiv `Eq(x,5)` |
| `z3_integer_inequality_detected` | `Gt(x,5)` not-equiv `Lt(x,5)` |
| `z3_mixed_boolean_integer` | `And(valid, Gt(count,0))` not-equiv `And(valid, Lt(count,0))` |
| `z3_uninterpreted_functions_not_conflated` | `Apply("Foo",[x])` not-equiv `Apply("Bar",[x])` |

---

### Sprint 0B: Fix Quantifier Encoding

**Why:** ForAll/Exists drop bound variables. `forall x. x>0` becomes just `x>0` with free `x`.

**Files:** `crates/logicaffeine_verify/src/equivalence.rs`

**What:** Reject quantified formulas in `check_equivalence()` with `EquivalenceResult::Unknown`. Our pipeline unrolls quantifiers before reaching Z3.

**RED tests (extend `phase_hw_z3_equiv.rs`):**

| Test | Assertion |
|---|---|
| `z3_rejects_quantified_formulas` | `forall x. x>0` not-equiv `true` (was falsely equivalent) |
| `z3_rejects_existential` | `exists x. x>0` not-equiv `true` |

---

### Sprint 0C: Fix KG Signal Role Inference

**Why:** All signals get `width=1, role=Internal`. Zero useful information for linking or coverage.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:** Infer roles from structural position in FOL. Antecedent-only -> Input. Consequent-only -> Output. Both -> Internal. Name contains "clk"/"clock" -> Clock. Extract property names from predicates.

**RED tests (extend `phase_hw_kg_extract.rs`):**

| Test | Assertion |
|---|---|
| `kg_clock_signal_detected_by_name` | Predicate with "clk" -> `SignalRole::Clock` |
| `kg_antecedent_is_input` | "if request then grant" -> request is Input |
| `kg_consequent_is_output` | "if request then grant" -> grant is Output |
| `kg_bidirectional_stays_internal` | Signal in both positions -> Internal |
| `kg_property_name_from_predicate` | Not hardcoded "Safety" -- uses predicate name |

---

### Sprint 0D: Complete LTL Lowering

**Why:** Release and WeakUntil in enum but not in `kripke.rs` lowering.

**Files:** `crates/logicaffeine_language/src/semantics/kripke.rs`

**What:** `Release(P,Q)` -> `Q(w) AND (P(w) OR forall w'(Next(w,w') -> Release(P,Q)(w')))`. `WeakUntil(P,Q)` -> `Q(w) OR (P(w) AND forall w'(Next(w,w') -> WeakUntil(P,Q)(w')))`.

**RED tests (extend `phase_hw_temporal.rs`):**

| Test | Assertion |
|---|---|
| `kripke_release_lowers_correctly` | Output contains `Next_Temporal` |
| `kripke_weak_until_lowers_correctly` | Parses and lowers without error |
| `kripke_release_produces_conjunction` | Release body has AND at top level |

---

### Sprint 0E: Formal Hardware Ontology

**Why:** AssertionForge has 35+ entity types (LLM prompt labels). We have 4 signal roles. To crush them, we need formally-grounded types with attributes that participate in verification -- not just labels.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:** Replace `SignalRole` and `KgRelation` with rich typed enums.

**New `HwEntityType` (28 variants):**

```rust
pub enum HwEntityType {
    // Structural (8)
    Module { name: String, is_top: bool },
    Port { direction: PortDirection, width: u32, domain: Option<String> },
    Signal { width: u32, signal_type: SignalType, domain: Option<String> },
    Register { width: u32, reset_value: Option<u64>, clock: Option<String> },
    Memory { depth: u32, width: u32, ports: u8 },
    Fifo { depth: u32, width: u32 },
    Bus { width: u32, protocol: Option<String> },
    Parameter { value: String },

    // Control (5)
    Fsm { states: Vec<String>, initial: Option<String> },
    Counter { width: u32, direction: CounterDirection },
    Arbiter { scheme: ArbitrationScheme, ports: u8 },
    Decoder { input_width: u32, output_width: u32 },
    Mux { inputs: u8, select_width: u32 },

    // Temporal (3)
    Clock { frequency: Option<String>, domain: String },
    Reset { polarity: ResetPolarity, synchronous: bool },
    Interrupt { priority: Option<u8>, edge_triggered: bool },

    // Protocol (3)
    Handshake { valid_signal: String, ready_signal: String },
    Pipeline { stages: u32, stall_signal: Option<String> },
    Transaction { request: String, response: String },

    // Data (3)
    DataPath { width: u32, signed: bool },
    Address { width: u32, base: Option<u64>, range: Option<u64> },
    Configuration { fields: Vec<String> },

    // Property (6)
    SafetyProperty { formula: String },
    LivenessProperty { formula: String },
    FairnessProperty { formula: String },
    ResponseProperty { trigger: String, response: String, bound: Option<u32> },
    MutexProperty { signals: Vec<String> },
    StabilityProperty { signal: String, condition: String },
}
```

**New `HwRelation` (24 variants):**

```rust
pub enum HwRelation {
    // Data Flow (5)
    Drives, DrivesRegistered { clock: String }, DataFlow, Reads, Writes,
    // Control Flow (4)
    Controls, Selects, Enables, Resets,
    // Temporal (5)
    Triggers { delay: Option<u32> }, Constrains,
    Follows { min: u32, max: u32 }, Precedes, Preserves,
    // Structural (4)
    Contains, Instantiates, ConnectsTo, BelongsToDomain { domain: String },
    // Protocol (3)
    HandshakesWith, Acknowledges, Pipelines { stages: u32 },
    // Specification (3)
    MutuallyExcludes, EventuallyFollows, AssumedBy,
}
```

**Pattern recognition in `extract_from_kripke_ast`:**

| FOL Pattern | Entity/Relation Extracted |
|---|---|
| Predicate with "clk"/"clock" in name | `Clock` entity |
| `G(not(P AND Q))` | `MutexProperty` + `Constrains` edge |
| `G(P -> X(Q))` | `ResponseProperty{bound:1}` + `Triggers{delay:1}` |
| `G(P -> F(Q))` | `ResponseProperty{bound:None}` + `EventuallyFollows` |
| `P U Q` | `Precedes` edge |
| `G(P -> X(X(X(Q))))` | `Pipeline{stages:3}` entity |
| Valid/ready predicate pair | `Handshake` entity + `HandshakesWith` edge |
| Multiple state predicates in case-like structure | `Fsm` entity |
| `G(P -> $stable(Q))` | `StabilityProperty` + `Preserves` edge |

**RED tests (new file `phase_hw_ontology.rs`, ~20 tests):**

| Test | Assertion |
|---|---|
| `ontology_has_28_entity_variants` | `std::mem::variant_count::<HwEntityType>() >= 28` |
| `ontology_has_24_relation_variants` | `std::mem::variant_count::<HwRelation>() >= 24` |
| `ontology_all_variants_serialize` | Every variant round-trips through serde JSON |
| `extract_clock_entity` | "Always, every clk is valid" -> `Clock` entity |
| `extract_mutex_property` | Negated conjunction -> `MutexProperty` entity |
| `extract_response_property` | `G(P -> X(Q))` -> `ResponseProperty{bound:1}` |
| `extract_handshake_entity` | valid/ready pair -> `Handshake` entity |
| `extract_pipeline_from_chained_next` | `G(P -> X(X(Q)))` -> `Pipeline{stages:2}` |
| `extract_stability_property` | "if valid then stable data" -> `StabilityProperty` |
| `extract_triggers_with_delay` | `G(P -> X(Q))` -> `Triggers{delay:Some(1)}` |
| `extract_follows_with_bound` | `G(P -> ##[1:5] Q)` -> `Follows{1,5}` |
| `extract_precedes_from_until` | `P U Q` -> `Precedes` edge |
| `extract_constrains_from_mutex` | `not(P AND Q)` -> `Constrains` edge |
| `extract_handshakes_with_relation` | valid/ready -> `HandshakesWith` |
| `extract_eventually_follows` | `G(P -> F(Q))` -> `EventuallyFollows` |
| `extract_fsm_from_state_group` | Multiple state predicates -> `Fsm` entity |
| `extract_axi_full_spec` | AXI write spec -> Handshake + Response + Pipeline entities |
| `extract_apb_full_spec` | APB spec -> proper entity + relation set |
| `kg_json_includes_entity_type` | JSON output has "entity_type" field with variant name |
| `kg_json_includes_relation_params` | JSON output includes delay/bound parameters |

---

### Sprint 1A: Verify SVA Surface Tests GREEN

**What:** Run tests, confirm 60+ existing tests pass. No code changes.

---

### Sprint 1B: IEEE 1800 Extended SVA Constructs

**Files:** `crates/logicaffeine_compile/src/codegen_sva/sva_model.rs`, `sva_to_verify.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_sva_ieee1800.rs`

**New variants (10):** `NotEq`, `LessThan`, `GreaterThan`, `LessEqual`, `GreaterEqual`, `Ternary`, `Throughout`, `Within`, `FirstMatch`, `Intersect`

**RED tests (~30):** Parse each, round-trip each, translate each, Z3-verify equivalences (`a!=b` equiv `!(a==b)`, ternary equiv if/else).

---

### Sprint 2A: Verilog Declaration Parser

**New file:** `crates/logicaffeine_compile/src/codegen_sva/rtl_extract.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_rtl_extract.rs`

**Structs:** `RtlModule`, `RtlPort`, `RtlSignal`, `RtlParam` with `PortDirection`, `SignalType` enums.

**Capabilities:** Module boundaries, ANSI/non-ANSI ports, `[N:M]` widths, wire/reg/logic, parameters, clock detection from `always @(posedge)`, comment/string skipping.

**RED tests (~25):** Empty module, ports with widths, signal types, clock detection, APB/UART/AXI headers, error handling.

---

### Sprint 2B: RTL KG + Spec-RTL Linking

**Depends on:** 0C, 0E, 2A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/rtl_kg.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_rtl_kg.rs`

**What:** Convert `RtlModule` -> `HwKnowledgeGraph` using Sprint 0E types. Link spec KG to RTL KG via exact match -> case-insensitive -> configurable mapping.

**RED tests (~15):** Port -> KgSignal conversion, role assignment, linking strategies, unmatched reporting.

---

### Sprint 3A: FOL -> SVA Formal Synthesis

**Depends on:** 0A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/fol_to_sva.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_fol_to_sva.rs`

| Kripke Pattern | SVA Output |
|---|---|
| `forall w'(Accessible_Temporal -> P(w'))` | `assert property(@(posedge clk) P)` |
| `exists w'(Reachable_Temporal AND P(w'))` | `cover property(s_eventually(P))` |
| `forall w'(Next_Temporal -> P(w'))` | `nexttime(P)` |
| `P -> Q` with worlds | `P \|-> Q` |
| `P -> exists w'(Reachable AND Q)` | `P \|-> s_eventually(Q)` |
| `not(P AND Q)` with worlds | `!(P && Q)` |

**The key test:**
```rust
#[cfg(feature = "verification")]
fn synthesis_plus_z3_is_the_full_pipeline() {
    let spec = "Always, if every request holds, then every acknowledgment holds.";
    let sva = synthesize_sva_from_spec(spec, "clk").unwrap();
    let result = check_z3_equivalence(spec, &sva.body, 5).unwrap();
    assert!(matches!(result, EquivalenceResult::Equivalent));
}
```

**RED tests (~30):** Temporal patterns (always/eventually/next/implication/mutex), KG signal name extraction, Z3 post-synthesis verification, incorrect synthesis caught.

---

### Sprint 3B: CEGAR Refinement

**Depends on:** 0A, 3A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/synthesis_refine.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_synthesis_refine.rs`

**Algorithm:** Synthesize -> Z3 check -> if not-equiv: classify too-strong/too-weak -> apply transformation -> re-check -> repeat or report gap.

**RED tests (~15, all `#[cfg(feature = "verification")]`):** Detect too-strong, detect too-weak, refine overlapping->non-overlapping, refine immediate->eventual, bounded convergence, unrefinable gap report.

---

### Sprint 4A: Specification Coverage Metrics

**Depends on:** 0C, 0E

**New file:** `crates/logicaffeine_compile/src/codegen_sva/coverage.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_coverage.rs`

```rust
pub struct SpecCoverage {
    pub signal_coverage: f64,
    pub property_coverage: f64,
    pub edge_coverage: f64,
    pub temporal_coverage: f64,
    pub uncovered_signals: Vec<String>,
    pub uncovered_properties: Vec<String>,
}
```

**RED tests (~20):** Full/partial/zero coverage, safety/liveness covered, uncovered lists, edge coverage, JSON output.

---

### Sprint 5A: Pre-Verified Protocol Templates

**Depends on:** 1B, 3A

**New dir:** `crates/logicaffeine_compile/src/codegen_sva/protocols/`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_protocols.rs`

**Protocols:** AXI4 (write/read channels), APB (setup/access), UART (start/stop/busy), SPI (MOSI stability, chip select), I2C (start/stop conditions, ACK).

Each: parameterizable `SvaProperty` + English spec + `#[cfg(feature = "verification")]` Z3 certificate.

**RED tests (~25):** Template generation, SVA parse validity, Z3 self-consistency.

---

### Sprint 6A: Waveform Generation from Counterexamples

**Depends on:** 0A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/waveform.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_waveform.rs`

**Why:** When Z3 finds a spec-SVA divergence, render the counterexample as a timing diagram. No existing tool generates waveforms from specification-level counterexamples.

```rust
pub fn trace_to_vcd(trace: &Trace, signals: &[KgSignal]) -> String;
pub fn trace_to_ascii_waveform(trace: &Trace) -> String;
```

**RED tests (~10):**

| Test | Assertion |
|---|---|
| `waveform_has_all_signals` | Every signal in trace appears in waveform |
| `vcd_header_correct` | Valid VCD $timescale, $scope, $var declarations |
| `vcd_timestamps_correct` | Value changes at correct `#N` timestamps |
| `ascii_readable` | Contains signal names, `_/^\_` transitions |
| `ascii_marks_divergence` | Divergence cycle visually indicated |
| `empty_trace_empty_output` | No panic on empty input |
| `multi_cycle_correct_length` | 10-cycle trace -> 10 columns |
| `signal_names_match_kg` | KG signal names used, not internal `@t` names |
| `mutex_violation_both_high` | Both grants show high in same cycle |
| `liveness_violation_never_asserted` | Ack never goes high across all cycles |

---

### Sprint 6B: Multi-Property Consistency Checking

**Depends on:** 0A

**New file:** `crates/logicaffeine_verify/src/consistency.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_consistency.rs`

**Why:** Check if a set of properties can all hold simultaneously. JasperGold does this; AssertionForge doesn't.

```rust
pub enum ConsistencyResult {
    Consistent,
    Inconsistent { conflicting: Vec<(usize, usize)>, witness: Trace },
    Unknown,
}
pub fn check_consistency(props: &[VerifyExpr], signals: &[String], bound: usize) -> ConsistencyResult;
```

**RED tests (~8, all `#[cfg(feature = "verification")]`):**

| Test | Assertion |
|---|---|
| `consistent_mutex_and_handshake` | Compatible properties -> `Consistent` |
| `inconsistent_p_and_not_p` | P and not-P -> `Inconsistent` with pair (0,1) |
| `inconsistent_mutex_forced_overlap` | `not(a AND b)` with `G(a AND b)` -> `Inconsistent` |
| `identifies_conflicting_pair` | Returns correct indices |
| `witness_shows_contradiction` | Trace has concrete signal values |
| `consistent_axi_set` | 3 AXI write properties consistent |
| `consistent_empty_set` | No properties -> `Consistent` |
| `three_way_conflict` | Three pairwise-ok but jointly-inconsistent props |

---

### Sprint 6C: Hierarchical Spec Decomposition

**Depends on:** 0A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/decompose.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_decompose.rs`

**Why:** Decompose complex properties into independently verifiable sub-properties. Verify decomposition is sound with Z3.

```rust
pub fn decompose_conjunctive(expr: &VerifyExpr) -> Vec<VerifyExpr>;
pub fn verify_decomposition_sound(original: &VerifyExpr, parts: &[VerifyExpr], bound: u32) -> bool;
```

**RED tests (~10):**

| Test | Assertion |
|---|---|
| `conjunction_splits` | `And(P,Q)` -> `[P, Q]` |
| `nested_conjunction_flattens` | `And(And(P,Q),R)` -> `[P, Q, R]` |
| `single_returns_self` | `P` -> `[P]` |
| `implication_not_split` | `Implies(P,Q)` -> `[Implies(P,Q)]` |
| `z3_conjunction_sound` | Z3: `G(P AND Q)` <-> `G(P) AND G(Q)` |
| `z3_disjunction_unsound` | Z3: `G(P OR Q)` != `G(P) OR G(Q)` |
| `axi_decomposes_to_channels` | AXI spec -> write + read channel sub-props |
| `preserves_signal_refs` | All signal names present in sub-properties |
| `result_serializable` | Decomposition JSON round-trips |
| `deep_nesting` | 5-level And nesting flattens correctly |

---

### Sprint 6D: Property Sufficiency Analysis

**Depends on:** 0E

**New file:** `crates/logicaffeine_compile/src/codegen_sva/sufficiency.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_sufficiency.rs`

**Why:** Before running verification, check if your properties are sufficient to cover the spec. Nobody does this.

```rust
pub struct SufficiencyReport {
    pub lonely_signals: Vec<String>,
    pub unconstrained_outputs: Vec<String>,
    pub missing_handshakes: Vec<(String, String)>,
    pub coverage_ratio: f64,
    pub recommendations: Vec<String>,
}
pub fn analyze_sufficiency(kg: &HwKnowledgeGraph) -> SufficiencyReport;
```

**RED tests (~10):**

| Test | Assertion |
|---|---|
| `all_covered_ratio_1` | All signals in properties -> 1.0 |
| `lonely_signal_detected` | Signal in no property appears in list |
| `unconstrained_output` | Output with no driving property listed |
| `missing_handshake_pair` | req without ack property -> pair listed |
| `ratio_correct` | 2/4 edges covered -> 0.5 |
| `recommendations_actionable` | Suggestions mention specific signal names |
| `empty_kg_zero` | Empty KG -> 0.0 ratio, empty lists |
| `axi_spec_analyzed` | Complex spec produces meaningful report |
| `report_serializable` | JSON round-trip |
| `partial_coverage` | Mixed covered/uncovered correct |

---

### Sprint 6E: Invariant Discovery from KG

**Depends on:** 0E, 0A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/invariants.rs`
**New test file:** `crates/logicaffeine_tests/tests/phase_hw_invariants.rs`

**Why:** Automatically extract candidate invariants from KG structure, verify them with Z3. Nobody else discovers invariants from formal specification graphs.

```rust
pub struct CandidateInvariant {
    pub expr: VerifyExpr,
    pub source: InvariantSource,
    pub verified: Option<bool>,
}
pub enum InvariantSource { MutexPattern, HandshakePattern, PipelineStability, ResetInit }
pub fn discover_invariants(kg: &HwKnowledgeGraph) -> Vec<CandidateInvariant>;
pub fn verify_invariant(inv: &CandidateInvariant, bound: u32) -> bool;
```

**RED tests (~10, Z3 tests feature-gated):**

| Test | Assertion |
|---|---|
| `discover_mutex_from_constrains` | Constrains edge -> `not(P AND Q)` invariant |
| `discover_handshake_from_entity` | Handshake entity -> response invariant |
| `discover_pipeline_stability` | Pipeline entity -> stage stability invariant |
| `discover_reset_init` | Reset entity -> initialization invariant |
| `z3_verifies_discovered_invariant` | Discovered mutex invariant verified true |
| `empty_kg_no_invariants` | No entities -> empty list |
| `multiple_from_complex_spec` | Rich spec -> 3+ invariants |
| `source_correctly_tagged` | Each invariant has correct source |
| `verified_marked_true` | Z3-confirmed invariant -> `Some(true)` |
| `unverifiable_marked_false` | Bad invariant -> `Some(false)` |

---

## Part IV: Sprint Sequencing

```
Sprint 0A (fix Z3 false positives) <-- CRITICAL, DO FIRST
    |
    +-- Sprint 0B (fix quantifiers)
    +-- Sprint 0C (fix KG roles)
    +-- Sprint 0D (Release/WeakUntil)
    +-- Sprint 0E (formal ontology: 28+ entity types, 24+ relations)
    |
    +-- Sprint 1A (verify existing SVA tests GREEN)
    +-- Sprint 1B (IEEE 1800 extended SVA)
    |       +-- Sprint 5A (protocol templates) [needs 1B, 3A]
    |
    +-- Sprint 2A (Verilog parser)
    |       +-- Sprint 2B (RTL KG + linking) [needs 0C, 0E, 2A]
    |
    +-- Sprint 3A (FOL -> SVA synthesis) [needs 0A]
    |       +-- Sprint 3B (CEGAR refinement) [needs 3A]
    |
    +-- Sprint 4A (spec coverage) [needs 0C, 0E]
    +-- Sprint 6A (waveform generation) [needs 0A]
    +-- Sprint 6B (property consistency) [needs 0A]
    +-- Sprint 6C (spec decomposition) [needs 0A]
    +-- Sprint 6D (property sufficiency) [needs 0E]
    +-- Sprint 6E (invariant discovery) [needs 0E, 0A]
```

**Critical path:** 0A -> 3A -> 3B
**Parallelizable:** 0B/0C/0D/0E after 0A. 2A independent. 6A-6E after their deps.

---

## Part V: Expected Outcomes

| Sprint | Tests | Cumulative | New Files |
|---|---|---|---|
| 0A | +5 | 294 | -- |
| 0B | +2 | 296 | -- |
| 0C | +5 | 301 | -- |
| 0D | +3 | 304 | -- |
| 0E | +20 | 324 | `phase_hw_ontology.rs` |
| 1B | +30 | 354 | `phase_hw_sva_ieee1800.rs` |
| 2A | +25 | 379 | `rtl_extract.rs`, `phase_hw_rtl_extract.rs` |
| 2B | +15 | 394 | `rtl_kg.rs`, `phase_hw_rtl_kg.rs` |
| 3A | +30 | 424 | `fol_to_sva.rs`, `phase_hw_fol_to_sva.rs` |
| 3B | +15 | 439 | `synthesis_refine.rs`, `phase_hw_synthesis_refine.rs` |
| 4A | +20 | 459 | `coverage.rs`, `phase_hw_coverage.rs` |
| 5A | +25 | 484 | `protocols/`, `phase_hw_protocols.rs` |
| 6A | +10 | 494 | `waveform.rs`, `phase_hw_waveform.rs` |
| 6B | +8 | 502 | `consistency.rs`, `phase_hw_consistency.rs` |
| 6C | +10 | 512 | `decompose.rs`, `phase_hw_decompose.rs` |
| 6D | +10 | 522 | `sufficiency.rs`, `phase_hw_sufficiency.rs` |
| 6E | +10 | 532 | `invariants.rs`, `phase_hw_invariants.rs` |

**Final state: 532+ hardware verification tests. 13 new source modules. 13 new test files.**

---

## Part VI: Academic Positioning

> **Title:** Logos: Formally Verified SVA Synthesis with Specification-Level Counterexamples, Property Decomposition, and Invariant Discovery
>
> We present the Logos SVA backend, a hardware verification system that unifies English specification parsing, Kripke-grounded temporal logic, and Z3-based semantic verification into an end-to-end pipeline producing formally correct SystemVerilog Assertions. Unlike LLM-based approaches (AssertionForge) that generate assertions without correctness guarantees, and unlike commercial tools (JasperGold, Questa) that verify assertions against implementations without checking specification compliance, LogicAffeine verifies that assertions correctly express the designer's intent. Novel contributions: (1) formal synthesis of SVA from Kripke-lowered FOL with Z3 equivalence proof, (2) specification-level counterexample waveforms showing where assertions diverge from intent, (3) Z3-verified property decomposition for compositional verification, (4) automated invariant discovery from formal knowledge graphs with 28+ entity types, and (5) specification sufficiency analysis detecting coverage gaps before verification begins.

**Why this is taken seriously:**

1. **Novel problem.** Nobody formally verifies SVA against specification. We do.
2. **Sound methodology.** Z3 equivalence is provably sound for our fragment. No overapproximation (Sprint 0A).
3. **Industrial-grade ontology.** 28+ entity types with formal attributes, not LLM prompt labels. 24+ parameterized relations.
4. **Capabilities nobody has.** Waveform from spec counterexample. Invariant discovery from KG. Sufficiency analysis. CEGAR refinement. Property decomposition with soundness proof.
5. **Reproducible.** 532+ tests, all automated, no LLM dependency in the core pipeline.
6. **Honest scope.** Propositional temporal logic. Bitvector/arithmetic/quantifier support is future work, not hidden behind overapproximation.
