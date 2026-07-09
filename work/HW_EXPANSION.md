# HW EXPANSION

Engineering specification for expanding LogicAffeine's hardware Knowledge Graph ontology into a compositional verification compendium. Every claim verified against source code. Every weakness disclosed. Every sprint has concrete RED tests. Designed for handoff to an implementing programmer.

The core contribution: transform a flat, 71%-dead ontology into a nested, composable type system where interfaces, assumptions, and contracts are first-class citizens. Verify the leaves. Compose them into a verified trunk. Beat state-space explosion through structure, not brute force.

---

## Part I: The Competitive Landscape

### What Existing Verification Ontologies Do

| Tool | Ontology | Limitation |
|---|---|---|
| AssertionForge (Stanford) | 35 entity types, 59 relation types. All string labels in LLM prompts. | No formal grounding. Labels are GPT-4o prompt tokens, not typed enums. |
| JasperGold (Cadence) | Proprietary property decomposition. Strategy selection via heuristics. | Closed-source. No spec-level ontology. Operates on RTL, not specifications. |
| Questa Formal (Siemens) | Assertion synthesis from coverage holes. Bug hunting mode. | No compositional reasoning. No assume-guarantee decomposition. |
| OneSpin (Siemens) | Operational SVA from RTL analysis. | Structural only. No specification-level knowledge graph. |
| UVM/OVM | Transaction-level modeling. Scoreboard concepts. Sequence/driver/monitor hierarchy. | Simulation framework, not formal. No typed entity/relation ontology. |
| **All of them** | Model hardware structure and/or RTL behavior. | None model verification contracts (assumptions, guarantees, coverage) as typed graph entities. None support cross-domain invariant discovery (CDC + Power + Security from one KG). |

### What Nobody Does

- Formally grounded composable ontology with Assume/Guarantee as first-class typed entities.
- Nested sub-ontologies (CDC, Power, Security) that compose into a unified knowledge graph.
- Automatic invariant discovery across domain boundaries (CDC synchronizer -> timing invariant, power isolation -> clamping invariant, taint source/sink -> non-interference invariant).
- Bridge functions that unify existing domain-specific analysis (CdcReport, PowerReport, ComponentSpec) into a single typed KG.
- Nested Rust enums where domain-specific pattern matching doesn't pay the memory cost of unrelated domains.

**These are our contributions.**

### What LogicAffeine Has (Audited, 2026-04-03)

**Three layers that form the ontology pipeline:**

| Layer | Evidence | Tests |
|---|---|---|
| **HwEntityType** — 28 variants in 6 categories (Structural, Control, Temporal, Protocol, Data, Property) | Typed enum with parameterized attributes, serde round-trip | 87 |
| **HwRelation** — 24 variants in 6 categories (DataFlow, ControlFlow, Temporal, Structural, Protocol, Specification) | Parameterized relations, serde round-trip | (included above) |
| **KG Extraction** — `extract_from_kripke_ast` walks Kripke-lowered FOL | Signal role inference, property classification, handshake/mutex detection | 19 |
| **Invariant Discovery** — `discover_invariants` from KG structure | 4 InvariantSource patterns (Mutex, Handshake, Pipeline, Reset) | 11 |
| **Domain Modules** — CDC, Power, Security, Compositional analysis | Working types and analysis, completely disconnected from KG | 48 |
| **Total** | | **165** |

**The uncomfortable truth (audited):**

| Metric | Value |
|---|---|
| HwEntityType variants defined | 28 |
| HwEntityType variants used in production extraction | **6** (Clock, SafetyProperty, LivenessProperty, ResponseProperty, MutexProperty, Handshake) |
| HwEntityType variants only in tests | **22** (79% dead) |
| HwRelation variants defined | 24 |
| HwRelation variants used in production extraction | **7** (Triggers, Constrains, Contains, HandshakesWith, EventuallyFollows, Precedes + legacy migration) |
| HwRelation variants only in tests | **17** (71% dead) |
| Helper enums used in production | **0** (PortDirection, SignalType, ResetPolarity, CounterDirection, ArbitrationScheme all test-only) |
| Domain modules connected to KG | **0** (CDC, Power, Security, Compositional all have own types) |

**Known gaps (this spec fills):**

| Gap | Location | Sprint |
|---|---|---|
| 22 entity types defined but never extracted | `knowledge_graph.rs` extraction only produces 6 types | 0A, 0B |
| 17 relation types defined but never created | `knowledge_graph.rs` extraction only produces 7 types | 0A, 0B |
| CDC analysis disconnected from KG | `cdc.rs` has CdcReport but no KG bridge | 1A |
| Power analysis disconnected from KG | `power.rs` has PowerReport but no KG bridge | 1B |
| Security analysis disconnected from KG | `security.rs` has TaintedSignal but no KG bridge | 1B |
| Compositional verification disconnected from KG | `compositional.rs` has ComponentSpec but no KG entities | 2A |
| No Assume/Guarantee property types | Property category has Safety/Liveness/Fairness/Response/Mutex/Stability but no Assumption/Guarantee | 2A |
| No Coverage/Invariant/BoundedLiveness properties | Can't represent vacuity witnesses or bounded liveness | 2A |
| No CDC/Power/Security entity types in KG | Can't represent synchronizers, isolation cells, taint sources | 1A, 1B |
| No cross-domain relations (CrossesDomain, PoweredBy, LeaksTo) | Can't model domain boundaries | 1A, 1B |
| No clock tree relations (ClockGates, DerivedFrom) | Can't model clock hierarchy | 2B |
| No verification closure relations (CoveredBy, WaivedBy) | Can't track verification completeness | 2B |
| Flat enum pays largest-variant memory penalty for all nodes | Single 28-variant enum, growing to 43+ | 3A |
| Invariant discovery limited to 4 patterns | MutexPattern, HandshakePattern, PipelineStability, ResetInit | 2D |

---

## Part II: The Honest Comparison

| Capability | AssertionForge | JasperGold | LogicAffeine (now) | LogicAffeine (after) |
|---|---|---|---|---|
| **Entity type count** | 35 (LLM labels) | Proprietary | 28 (6 used) | **43+ (all used)** |
| **Relation type count** | 59 (string labels) | Proprietary | 24 (7 used) | **40+ (all used)** |
| **Type system** | Python strings | Proprietary | Flat Rust enum | **Nested sub-enums** |
| **Formal grounding** | None (prompt tokens) | N/A | Serde-typed | **Serde-typed + parameterized** |
| **Compositional properties** | No | Manual | No | **AssumptionProperty, GuaranteeProperty** |
| **Coverage/vacuity** | No | Yes (RTL) | No | **CoverageProperty (spec-level)** |
| **CDC entities** | No | N/A | Separate CdcReport | **Unified KG with Synchronizer, AsyncFifo** |
| **Power entities** | No | N/A | Separate PowerReport | **Unified KG with PowerDomain, IsolationCell** |
| **Security entities** | No | No | Separate TaintedSignal | **Unified KG with TaintSource, TaintSink** |
| **Cross-domain invariants** | No | No | No | **Automatic from unified KG** |
| **Domain-specific extraction** | LLM | RTL | Kripke AST only | **Kripke AST + domain bridges** |
| **Assume-Guarantee** | No | Manual | Separate crate | **First-class KG entities + Refines/Abstracts** |
| **Ontology tests** | 0 | N/A | 87 | **213+** |
| **Invariant discovery sources** | 0 | N/A | 4 | **9** |

---

## Part III: Architecture

### The Compositional KG Pipeline

```
                    English Specification
                           |
                   [logicaffeine_language]
                   parse + Kripke lower
                           |
                    Kripke FOL AST
                           |
              extract_from_kripke_ast()
              [knowledge_graph.rs]
                           |
              HwKnowledgeGraph (unified)
               /     |      |      \
              /      |      |       \
    cdc_to_kg()  power_to_kg()  security_to_kg()  compositional_to_kg()
    [cdc.rs]     [power.rs]     [security.rs]      [compositional.rs]
              \      |      |       /
               \     |      |      /
              Enriched KG (all domains)
               /           |           \
              /            |            \
    discover_invariants()  emit_sva()   to_json()
    [invariants.rs]        [mod.rs]     (LLM context)
              |
    CandidateInvariant[]
    (9 InvariantSource variants)
```

### File Structure

```
crates/logicaffeine_language/src/semantics/
  knowledge_graph.rs    -- MODIFY: nested sub-enums, new variants, extraction patterns
                           Current: 981 LOC, flat HwEntityType(28), HwRelation(24)
                           After: ~1400 LOC, nested HwEntityType(9 categories, 43+ variants),
                                  HwRelation(10 categories, 40+ variants)

crates/logicaffeine_compile/src/codegen_sva/
  cdc.rs                -- MODIFY: add pub fn cdc_to_kg(report, kg)
  power.rs              -- MODIFY: add pub fn power_to_kg(report, kg)
  invariants.rs         -- MODIFY: add 5 InvariantSource variants + discovery patterns

crates/logicaffeine_verify/src/
  security.rs           -- MODIFY: add pub fn security_to_kg(signals, kg)
  compositional.rs      -- MODIFY: add pub fn compositional_to_kg(specs, kg)

crates/logicaffeine_tests/tests/
  phase_hw_ontology.rs       -- MODIFY: update for nested enums (Sprint 3B)
  phase_hw_ontology_exp.rs   -- NEW: expansion tests (Sprints 0A-2D)
  phase_hw_ontology_v2.rs    -- NEW: nested enum tests (Sprint 3A)
```

### Test Files

```
crates/logicaffeine_tests/tests/
  phase_hw_ontology_exp.rs    -- Sprints 0A-2D: all expansion tests (~98 tests)
    Section 0A: Activate structural/control entity extraction
    Section 0B: Activate remaining entity + relation extraction
    Section 1A: CDC domain KG bridge
    Section 1B: Power + Security domain KG bridge
    Section 2A: Compositional Assume-Guarantee KG integration
    Section 2B: Missing relations for existing categories
    Section 2C: CDC + Power extraction from Kripke AST
    Section 2D: Invariant source expansion
  phase_hw_ontology_v2.rs     -- Sprints 3A-3B: nested enum tests (~28 tests)
    Section 3A: Nested sub-enums + convenience constructors
    Section 3B: Migration validation
```

---

## Part IV: Sprint Specification

Every sprint follows TDD: write RED tests first, implement until GREEN, run full suite for zero regressions.

```bash
# RED: confirm new tests fail
cargo test --no-fail-fast --test phase_hw_ontology_exp -- --skip e2e > /tmp/red.txt 2>&1; echo "EXIT: $?" >> /tmp/red.txt

# GREEN: implement until pass
cargo test --no-fail-fast --test phase_hw_ontology_exp -- --skip e2e > /tmp/green.txt 2>&1; echo "EXIT: $?" >> /tmp/green.txt

# REGRESSION: all tests pass
cargo test --no-fail-fast -- --skip e2e > /tmp/all.txt 2>&1; echo "EXIT: $?" >> /tmp/all.txt
```

---

### Sprint 0A: Activate Structural + Control Entity Extraction

**Why:** 13 entity types (Module, Port, Signal, Register, Memory, Fifo, Bus, Parameter, Fsm, Counter, Arbiter, Decoder, Mux) are defined in `HwEntityType` but never created by `extract_from_kripke_ast`. The extraction function produces only 6 entity types. These 13 can be activated via naming heuristics and AST pattern matching, following the existing clock/handshake detection patterns.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:** Extend `extract_from_kripke_ast` with naming-heuristic entity creation:

- **Signal**: Every worlded predicate already produces a `KgSignal`. Also emit `HwEntityType::Signal` with the inferred width and signal type. This bridges legacy signals to the typed ontology.
- **Port**: When a signal has role Input/Output, also emit `HwEntityType::Port` with matching `PortDirection`.
- **Register**: Signal name contains "reg", "ff", "latch", "register", or "_q" suffix -> `HwEntityType::Register`.
- **Counter**: Signal name contains "count", "cnt", "counter" -> `HwEntityType::Counter`.
- **Fsm**: Multiple mutex-group signals with "state", "s0", "s1", "idle", "active" naming -> `HwEntityType::Fsm` with detected states.
- **Memory**: Signal name contains "mem", "ram", "rom", "sram" -> `HwEntityType::Memory`.
- **Fifo**: Signal name contains "fifo", "queue" -> `HwEntityType::Fifo`.
- **Bus**: Signal name contains "bus", "axi", "apb", "ahb" -> `HwEntityType::Bus`.
- **Mux**: Signal name contains "mux", "sel" with associated data signals -> `HwEntityType::Mux`.
- **Decoder**: Signal name contains "decode", "dec" -> `HwEntityType::Decoder`.
- **Arbiter**: Signal name contains "arb", "arbiter" -> `HwEntityType::Arbiter`.
- **Module**: Top-level scope (if detectable from spec structure) -> `HwEntityType::Module`.
- **Parameter**: Signal name is ALL_CAPS or contains "PARAM", "WIDTH", "DEPTH" -> `HwEntityType::Parameter`.

**RED tests (Section 0A of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_0a_signal_entity_from_worlded_predicate` | Any worlded predicate produces `HwEntityType::Signal` entity alongside legacy `KgSignal` |
| `exp_0a_port_entity_from_input_role` | Signal with antecedent-only position -> `HwEntityType::Port { direction: Input, .. }` |
| `exp_0a_port_entity_from_output_role` | Signal with consequent-only position -> `HwEntityType::Port { direction: Output, .. }` |
| `exp_0a_register_from_naming` | Spec with "data_reg" signal -> `HwEntityType::Register` entity in KG |
| `exp_0a_counter_from_naming` | Spec with "cnt" or "counter" signal -> `HwEntityType::Counter` entity in KG |
| `exp_0a_fsm_from_state_group` | Spec with "state_idle", "state_run", "state_done" mutex group -> `HwEntityType::Fsm` with 3 states |
| `exp_0a_memory_from_naming` | Spec with "mem_data" signal -> `HwEntityType::Memory` entity |
| `exp_0a_fifo_from_naming` | Spec with "fifo_wr" signal -> `HwEntityType::Fifo` entity |
| `exp_0a_bus_from_naming` | Spec with "axi_data" signal -> `HwEntityType::Bus` entity |
| `exp_0a_mux_from_naming` | Spec with "mux_sel" signal -> `HwEntityType::Mux` entity |
| `exp_0a_decoder_from_naming` | Spec with "decode_out" signal -> `HwEntityType::Decoder` entity |
| `exp_0a_arbiter_from_naming` | Spec with "arb_grant" signal -> `HwEntityType::Arbiter` entity |
| `exp_0a_parameter_from_caps` | Spec with "WIDTH" signal -> `HwEntityType::Parameter` entity |
| `exp_0a_at_least_8_distinct_entity_types` | Single rich spec produces >= 8 distinct entity type categories |
| `exp_0a_existing_extraction_unbroken` | Clock, SafetyProperty, LivenessProperty, MutexProperty, Handshake, ResponseProperty all still produced |

---

### Sprint 0B: Activate Remaining Entity + Relation Extraction

**Depends on:** 0A

**Why:** 9 more entity types (Reset, Interrupt, Pipeline, Transaction, DataPath, Address, Configuration, FairnessProperty, StabilityProperty) and 10 relation types (Drives, DrivesRegistered, Controls, Selects, Enables, Follows, Preserves, BelongsToDomain, Acknowledges, Pipelines) remain dead. The extraction pipeline should recognize these from temporal patterns and naming conventions.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:**

Entity extraction:
- **Reset**: Signal name contains "rst", "reset", "rst_n" -> `HwEntityType::Reset`. Detect polarity from "_n" suffix (ActiveLow) vs plain (ActiveHigh).
- **Interrupt**: Signal name contains "irq", "interrupt", "int" -> `HwEntityType::Interrupt`.
- **Pipeline**: Chained Next patterns `G(P -> X(X(...X(Q)...)))` -> `HwEntityType::Pipeline { stages: depth }`. Detect stall signal from "stall" naming.
- **Transaction**: Paired request/response signals not matching handshake pattern (longer latency) -> `HwEntityType::Transaction`.
- **DataPath**: Signal with "data" in name + width context -> `HwEntityType::DataPath`.
- **Address**: Signal with "addr", "address" in name -> `HwEntityType::Address`.
- **Configuration**: Signal with "cfg", "config", "ctrl_reg" in name -> `HwEntityType::Configuration`.
- **FairnessProperty**: Detect `GF(P)` pattern (Kripke: nested ∀w∃w' structure) -> `HwEntityType::FairnessProperty`.
- **StabilityProperty**: Detect `G(P -> stable(Q))` or "stable" naming -> `HwEntityType::StabilityProperty`.

Relation extraction:
- **Drives**: From conditional patterns where antecedent signal drives consequent -> `HwRelation::Drives`.
- **DrivesRegistered**: Drives through a detected register (Next temporal) -> `HwRelation::DrivesRegistered { clock }`.
- **Controls**: Signal with "enable", "en", "select", "ctrl" controlling another -> `HwRelation::Controls`.
- **Selects**: Mux select signal choosing between data paths -> `HwRelation::Selects`.
- **Enables**: Enable signal gating another signal -> `HwRelation::Enables`.
- **Follows**: Bounded temporal sequence `P ##[min:max] Q` pattern -> `HwRelation::Follows { min, max }`.
- **Preserves**: Stability pattern (signal unchanged across cycle) -> `HwRelation::Preserves`.
- **BelongsToDomain**: Signals sharing a clock domain -> `HwRelation::BelongsToDomain { domain }`.
- **Acknowledges**: Response signal in handshake pair -> `HwRelation::Acknowledges`.
- **Pipelines**: Pipeline entity stages -> `HwRelation::Pipelines { stages }`.

**RED tests (Section 0B of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_0b_reset_entity_from_naming` | Spec with "rst_n" -> `Reset { polarity: ActiveLow, synchronous: true }` |
| `exp_0b_reset_active_high` | Spec with "reset" (no _n) -> `Reset { polarity: ActiveHigh, .. }` |
| `exp_0b_interrupt_from_naming` | Spec with "irq" -> `Interrupt` entity |
| `exp_0b_pipeline_from_chained_next` | `G(P -> X(X(Q)))` -> `Pipeline { stages: 2 }` |
| `exp_0b_pipeline_with_stall` | Pipeline + "stall" signal -> `Pipeline { stall_signal: Some("stall") }` |
| `exp_0b_fairness_from_gf_pattern` | `GF(P)` pattern -> `FairnessProperty` entity |
| `exp_0b_stability_from_naming` | "stable_data" + conditional -> `StabilityProperty` entity |
| `exp_0b_drives_relation` | Conditional `If P then Q` -> `Drives` typed edge from P to Q |
| `exp_0b_drives_registered_with_clock` | `If P then X(Q)` + clock domain -> `DrivesRegistered { clock }` edge |
| `exp_0b_controls_from_naming` | "enable_x" controlling "data_x" -> `Controls` edge |
| `exp_0b_belongs_to_domain` | Signals sharing "sys_clk" -> `BelongsToDomain { domain: "sys_clk" }` edges |
| `exp_0b_at_least_15_entity_types` | Rich spec produces >= 15 distinct entity type categories |
| `exp_0b_at_least_12_relation_types` | Rich spec produces >= 12 distinct relation type categories |
| `exp_0b_all_prior_tests_unbroken` | Sprint 0A tests still pass (no regressions) |

---

### Sprint 1A: CDC Domain KG Bridge

**Depends on:** None (parallel with Phase 0)

**Why:** `cdc.rs` has complete CDC analysis producing `CdcReport` with crossings, violations, and patterns. But these are completely disconnected from the Knowledge Graph ontology. The KG cannot represent synchronizers, async FIFOs, or domain crossing relationships. To unify all domain analysis into one queryable graph, we need CDC entity types in `HwEntityType`, CDC relation types in `HwRelation`, and a bridge function.

**Files:**
- `crates/logicaffeine_language/src/semantics/knowledge_graph.rs` (add 3 entity + 2 relation variants)
- `crates/logicaffeine_compile/src/codegen_sva/cdc.rs` (add `cdc_to_kg()`)

**What:**

New `HwEntityType` variants:
```rust
Synchronizer { stages: u32, source_domain: String, dest_domain: String },
AsyncFifo { depth: u32, source_domain: String, dest_domain: String },
GrayCodeCounter { width: u32, domain: String },
```

New `HwRelation` variants:
```rust
CrossesDomain { source_domain: String, dest_domain: String },
SynchronizesWith { mechanism: String },
```

Bridge function in `cdc.rs`:
```rust
pub fn cdc_to_kg(report: &CdcReport, kg: &mut HwKnowledgeGraph) {
    // CdcPattern::TwoFlopSync -> Synchronizer { stages: 2 }
    // CdcPattern::ThreeFlopSync -> Synchronizer { stages: 3 }
    // CdcPattern::GrayCode -> GrayCodeCounter
    // CdcCrossing -> CrossesDomain edge
    // CdcPattern present for crossing -> SynchronizesWith edge
    // CdcViolation (missing sync) -> CrossesDomain WITHOUT SynchronizesWith
}
```

**RED tests (Section 1A of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_1a_synchronizer_entity_constructible` | `HwEntityType::Synchronizer { stages: 2, .. }` compiles + serializes |
| `exp_1a_async_fifo_entity_constructible` | `HwEntityType::AsyncFifo { depth: 16, .. }` compiles + serializes |
| `exp_1a_gray_code_entity_constructible` | `HwEntityType::GrayCodeCounter { width: 4, .. }` compiles + serializes |
| `exp_1a_crosses_domain_relation_constructible` | `HwRelation::CrossesDomain { .. }` compiles + serializes |
| `exp_1a_synchronizes_with_relation_constructible` | `HwRelation::SynchronizesWith { .. }` compiles + serializes |
| `exp_1a_all_cdc_entities_round_trip_json` | All 3 CDC entities survive `serde_json::to_string` -> `serde_json::from_str` |
| `exp_1a_all_cdc_relations_round_trip_json` | Both CDC relations survive round-trip |
| `exp_1a_cdc_to_kg_adds_entities` | `cdc_to_kg(&report, &mut kg)` with TwoFlopSync pattern -> `kg.entities` contains `Synchronizer` |
| `exp_1a_cdc_to_kg_adds_crossing_edges` | `cdc_to_kg` with crossing -> `kg.typed_edges` contains `CrossesDomain` |
| `exp_1a_cdc_to_kg_adds_sync_edges` | `cdc_to_kg` with synced crossing -> `SynchronizesWith` edge present |
| `exp_1a_missing_sync_no_sync_edge` | Violation (MissingSynchronizer) -> `CrossesDomain` without `SynchronizesWith` |
| `exp_1a_variant_count_31_entities_26_relations` | Total HwEntityType >= 31 variants, HwRelation >= 26 variants |

---

### Sprint 1B: Power + Security Domain KG Bridge

**Depends on:** None (parallel with Phase 0 and Sprint 1A)

**Why:** `power.rs` has `PowerReport` (domains, crossings, violations) and `security.rs` has `TaintedSignal` (labeled signals). Both are disconnected from the KG. Adding Power and Security entity/relation types + bridge functions unifies all domain analysis into one graph.

**Files:**
- `crates/logicaffeine_language/src/semantics/knowledge_graph.rs` (add 6 entity + 4 relation variants)
- `crates/logicaffeine_compile/src/codegen_sva/power.rs` (add `power_to_kg()`)
- `crates/logicaffeine_verify/src/security.rs` (add `security_to_kg()`)

**What:**

New `HwEntityType` variants:
```rust
// Power (4)
KgPowerDomain { name: String, default_state: String },
IsolationCell { signal: String, clamp_value: Option<String> },
RetentionRegister { signal: String, domain: String },
LevelShifter { signal: String, source_domain: String, dest_domain: String },

// Security (2)
TaintSource { signal: String, label: String },
TaintSink { signal: String, label: String },
```

Note: `KgPowerDomain` avoids name collision with the existing `power.rs` `PowerDomain` struct.

New `HwRelation` variants:
```rust
// Power (2)
PoweredBy { domain: String },
IsolatedFrom { domain: String },

// Security (2)
LeaksTo,
MaskedBy { gate_signal: String },
```

Bridge functions:
```rust
// power.rs
pub fn power_to_kg(report: &PowerReport, kg: &mut HwKnowledgeGraph) {
    // PowerDomain -> KgPowerDomain entity + PoweredBy edges for domain signals
    // PowerViolation::MissingIsolation -> signal + missing IsolationCell
    // PowerViolation::MissingLevelShifter -> signal + missing LevelShifter
    // PowerCrossing -> IsolatedFrom edge (if isolation present)
}

// security.rs
pub fn security_to_kg(signals: &[TaintedSignal], kg: &mut HwKnowledgeGraph) {
    // SecurityLabel::Secret -> TaintSource entity
    // SecurityLabel::Public -> TaintSink entity
    // If tainted secret flows to public output -> LeaksTo edge
    // If gated by mask -> MaskedBy edge
}
```

**RED tests (Section 1B of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_1b_kg_power_domain_constructible` | `HwEntityType::KgPowerDomain { .. }` compiles + serializes |
| `exp_1b_isolation_cell_constructible` | `HwEntityType::IsolationCell { .. }` compiles + serializes |
| `exp_1b_retention_register_constructible` | `HwEntityType::RetentionRegister { .. }` compiles + serializes |
| `exp_1b_level_shifter_constructible` | `HwEntityType::LevelShifter { .. }` compiles + serializes |
| `exp_1b_taint_source_constructible` | `HwEntityType::TaintSource { .. }` compiles + serializes |
| `exp_1b_taint_sink_constructible` | `HwEntityType::TaintSink { .. }` compiles + serializes |
| `exp_1b_powered_by_relation` | `HwRelation::PoweredBy { .. }` compiles + round-trips |
| `exp_1b_isolated_from_relation` | `HwRelation::IsolatedFrom { .. }` compiles + round-trips |
| `exp_1b_leaks_to_relation` | `HwRelation::LeaksTo` compiles + round-trips |
| `exp_1b_masked_by_relation` | `HwRelation::MaskedBy { .. }` compiles + round-trips |
| `exp_1b_power_to_kg_populates_entities` | `power_to_kg(&report, &mut kg)` -> KgPowerDomain entities in `kg.entities` |
| `exp_1b_power_to_kg_populates_edges` | `power_to_kg` -> PoweredBy edges in `kg.typed_edges` |
| `exp_1b_security_to_kg_taint_source` | `security_to_kg(&[secret_signal], &mut kg)` -> TaintSource entity |
| `exp_1b_security_to_kg_taint_sink` | `security_to_kg(&[public_signal], &mut kg)` -> TaintSink entity |
| `exp_1b_variant_count_37_entities_30_relations` | Total HwEntityType >= 37, HwRelation >= 30 |

---

### Sprint 2A: Compositional Assume-Guarantee KG Integration

**Depends on:** 1B (for security bridge patterns)

**Why:** The Property category has 6 types (Safety, Liveness, Fairness, Response, Mutex, Stability) but cannot represent the **difference between what a module is responsible for (Guarantee) and what it expects from the environment (Assumption)**. Without this distinction, the KG cannot support compositional verification — you cannot verify leaves independently and compose them. Additionally, `CoverageProperty` is essential for detecting vacuity (a property that passes because its precondition never fires), and `InvariantProperty` captures single-state structural conditions distinct from trace-level `SafetyProperty`. `BoundedLivenessProperty` maps to SVA's `s_eventually` (strong eventually, must complete within simulation).

**Files:**
- `crates/logicaffeine_language/src/semantics/knowledge_graph.rs` (add 5 entity + 5 relation variants)
- `crates/logicaffeine_verify/src/compositional.rs` (add `compositional_to_kg()`)

**What:**

New `HwEntityType` variants:
```rust
// Compositional Properties (5)
AssumptionProperty { formula: String, component: String },
GuaranteeProperty { formula: String, component: String },
CoverageProperty { formula: String, target: String },
InvariantProperty { formula: String, source: String },
BoundedLivenessProperty { formula: String, bound: u32 },
```

New `HwRelation` variants:
```rust
// Compositional Relations (5)
Refines,       // Module A refines (implements) Spec B
Abstracts,     // Spec B is an over-approximation of Module A
DependsOn,     // Module A's liveness depends on Module B's guarantee
Monitors,      // Property/checker monitors a structural entity
DrivesStimulus, // Assumption drives stimulus to an input port
```

Bridge function:
```rust
// compositional.rs
pub fn compositional_to_kg(specs: &[ComponentSpec], kg: &mut HwKnowledgeGraph) {
    for spec in specs {
        // spec.assumes -> AssumptionProperty entities
        // spec.guarantees -> GuaranteeProperty entities
        // Cross-component: if B.assumes matches A.guarantees -> DependsOn edge
        // spec.invariant -> InvariantProperty entity
    }
}
```

**RED tests (Section 2A of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_2a_assumption_property_constructible` | `HwEntityType::AssumptionProperty { .. }` compiles + serializes |
| `exp_2a_guarantee_property_constructible` | `HwEntityType::GuaranteeProperty { .. }` compiles + serializes |
| `exp_2a_coverage_property_constructible` | `HwEntityType::CoverageProperty { .. }` compiles + serializes |
| `exp_2a_invariant_property_constructible` | `HwEntityType::InvariantProperty { .. }` compiles + serializes |
| `exp_2a_bounded_liveness_constructible` | `HwEntityType::BoundedLivenessProperty { formula: "F[<=5](ack)".into(), bound: 5 }` compiles + serializes |
| `exp_2a_refines_relation` | `HwRelation::Refines` compiles + round-trips |
| `exp_2a_abstracts_relation` | `HwRelation::Abstracts` compiles + round-trips |
| `exp_2a_depends_on_relation` | `HwRelation::DependsOn` compiles + round-trips |
| `exp_2a_monitors_relation` | `HwRelation::Monitors` compiles + round-trips |
| `exp_2a_drives_stimulus_relation` | `HwRelation::DrivesStimulus` compiles + round-trips |
| `exp_2a_compositional_to_kg_assumptions` | `compositional_to_kg(&[spec], &mut kg)` -> AssumptionProperty entities from `spec.assumes` |
| `exp_2a_compositional_to_kg_guarantees` | `compositional_to_kg` -> GuaranteeProperty entities from `spec.guarantees` |
| `exp_2a_compositional_to_kg_depends_on` | Two specs where B.assumes matches A.guarantees -> DependsOn edge from B to A |
| `exp_2a_bounded_liveness_from_response` | `ResponseProperty { bound: Some(5) }` in KG -> also generates `BoundedLivenessProperty { bound: 5 }` |
| `exp_2a_coverage_from_assumption` | Each AssumptionProperty auto-generates a CoverageProperty (vacuity witness) |
| `exp_2a_variant_count_42_entities_35_relations` | Total HwEntityType >= 42, HwRelation >= 35 |

---

### Sprint 2B: Missing Relations for Existing Categories

**Depends on:** 0B (for activated relation infrastructure)

**Why:** The existing categories are missing relations that model clock hierarchies (gating, derivation), event generation (interrupt/overflow), and verification closure (coverage tracking, waivers). These fill structural gaps identified in the Gemini brainstorm.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:**

New `HwRelation` variants:
```rust
ClockGates { gating_signal: String },     // Clock gating relationship
DerivedFrom { divisor: Option<u32> },      // Clock derivation (e.g., clk_div2 from sys_clk)
Generates { event_type: String },          // Interrupt/overflow/event generation
CoveredBy { property_name: String },       // Verification closure: entity covered by property
WaivedBy { reason: String },               // Verification closure: known waiver
```

Extraction patterns in `extract_from_kripke_ast`:
- **ClockGates**: "clk_en" or "clock_gate" signal -> `ClockGates` edge to gated clock.
- **DerivedFrom**: "clk_div2", "clk_div4" naming -> `DerivedFrom { divisor: Some(2/4) }` edge to parent clock.
- **Generates**: Counter + "overflow" or interrupt + "trigger" naming -> `Generates` edge.

**RED tests (Section 2B of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_2b_clock_gates_constructible` | `HwRelation::ClockGates { gating_signal: "clk_en".into() }` compiles + round-trips |
| `exp_2b_derived_from_constructible` | `HwRelation::DerivedFrom { divisor: Some(2) }` compiles + round-trips |
| `exp_2b_generates_constructible` | `HwRelation::Generates { event_type: "overflow".into() }` compiles + round-trips |
| `exp_2b_covered_by_constructible` | `HwRelation::CoveredBy { property_name: "mutex_ab".into() }` compiles + round-trips |
| `exp_2b_waived_by_constructible` | `HwRelation::WaivedBy { reason: "known limitation".into() }` compiles + round-trips |
| `exp_2b_clock_gates_extracted` | Spec with "clk_en" gating "sys_clk" -> `ClockGates` edge |
| `exp_2b_derived_from_extracted` | Spec with "clk_div2" derived from "sys_clk" -> `DerivedFrom { divisor: Some(2) }` |
| `exp_2b_generates_from_overflow` | Counter entity + "overflow" signal -> `Generates { event_type: "overflow" }` |
| `exp_2b_variant_count_42_entities_40_relations` | Total HwEntityType >= 42, HwRelation >= 40 |

---

### Sprint 2C: CDC + Power Extraction from Kripke AST

**Depends on:** 1A, 1B (bridge functions must exist first)

**Why:** Sprints 1A/1B added bridge functions that convert domain-specific analysis results (`CdcReport`, `PowerReport`) to KG entities. This sprint adds **direct extraction from the Kripke AST itself** — detecting CDC and power patterns from English specifications via naming heuristics, not just from RTL analysis. This means the KG gets populated even when the only input is an English spec.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:** Extend `extract_from_kripke_ast` with domain-specific naming detection:

- **Synchronizer**: Signal name contains "sync", "synchronize", "synchronizer" -> `Synchronizer` entity.
- **Power Domain**: Signal name contains "power_domain", "pd_", "vdd_" -> `KgPowerDomain` entity.
- **Isolation Cell**: Signal name contains "iso", "isolation", "clamp" -> `IsolationCell` entity.
- **Retention Register**: Signal name contains "ret", "retention" -> `RetentionRegister` entity.
- **Taint Source**: Signal name contains "secret", "key", "private" -> `TaintSource` entity.
- **Taint Sink**: Signal name contains "public", "output", "observable" -> `TaintSink` entity.
- **CrossesDomain**: Two clock signals detected + signals in different domains -> `CrossesDomain` edges.

**RED tests (Section 2C of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_2c_synchronizer_from_naming` | Spec with "sync_data" signal -> `Synchronizer` entity in KG |
| `exp_2c_power_domain_from_naming` | Spec with "pd_core" signal -> `KgPowerDomain` entity |
| `exp_2c_isolation_from_naming` | Spec with "iso_cell" signal -> `IsolationCell` entity |
| `exp_2c_retention_from_naming` | Spec with "ret_reg" signal -> `RetentionRegister` entity |
| `exp_2c_taint_source_from_naming` | Spec with "secret_key" signal -> `TaintSource` entity |
| `exp_2c_taint_sink_from_naming` | Spec with "public_out" signal -> `TaintSink` entity |
| `exp_2c_crosses_domain_from_multi_clock` | Spec with "clk_a" and "clk_b" + cross-domain signal -> `CrossesDomain` edge |
| `exp_2c_cdc_and_power_coexist` | Spec with both CDC and power naming -> both entity types present |
| `exp_2c_existing_extraction_unbroken` | All prior KG extraction tests pass (no regressions) |
| `exp_2c_at_least_20_entity_types_extractable` | Rich spec with all naming patterns -> >= 20 distinct entity types |

---

### Sprint 2D: Invariant Source Expansion

**Depends on:** 2A (for compositional properties), 2C (for CDC/power entities)

**Why:** `discover_invariants` in `invariants.rs` currently discovers candidates from only 4 sources (MutexPattern, HandshakePattern, PipelineStability, ResetInit). The new entity types enable 5 additional invariant discovery patterns: CDC synchronization timing, power isolation clamping, security non-interference, clock gating correctness, and assumption discharge.

**Files:** `crates/logicaffeine_compile/src/codegen_sva/invariants.rs`

**What:**

New `InvariantSource` variants:
```rust
pub enum InvariantSource {
    // Existing (4)
    MutexPattern,
    HandshakePattern,
    PipelineStability,
    ResetInit,
    // New (5)
    CdcSynchronization,    // Synchronizer entity -> "data stable after N cycles"
    PowerIsolation,        // IsolationCell entity -> "output clamped when domain off"
    SecurityNonInterference, // TaintSource + TaintSink -> "no information leak"
    ClockGating,           // ClockGates relation -> "gated clock off when enable low"
    AssumptionDischarge,   // AssumptionProperty -> "environment satisfies assumption"
}
```

Discovery patterns in `discover_invariants`:
- **CdcSynchronization**: `Synchronizer { stages: N }` entity -> invariant: after N clock cycles in destination domain, data is stable.
- **PowerIsolation**: `IsolationCell { clamp_value }` entity -> invariant: when power domain is off, output equals clamp value.
- **SecurityNonInterference**: `TaintSource` + `TaintSink` pair -> invariant: no path from source to sink without `MaskedBy`.
- **ClockGating**: `ClockGates { gating_signal }` relation -> invariant: when gating signal low, gated clock is inactive.
- **AssumptionDischarge**: `AssumptionProperty { formula, component }` -> invariant: environment behavior satisfies the assumption formula.

**RED tests (Section 2D of `phase_hw_ontology_exp.rs`):**

| Test | Assertion |
|---|---|
| `exp_2d_cdc_invariant_from_synchronizer` | KG with `Synchronizer { stages: 2 }` -> `CandidateInvariant` with source `CdcSynchronization` |
| `exp_2d_power_invariant_from_isolation` | KG with `IsolationCell` -> `CandidateInvariant` with source `PowerIsolation` |
| `exp_2d_security_invariant_from_taint_pair` | KG with `TaintSource` + `TaintSink` -> `CandidateInvariant` with source `SecurityNonInterference` |
| `exp_2d_clock_gating_invariant` | KG with `ClockGates` relation -> `CandidateInvariant` with source `ClockGating` |
| `exp_2d_assumption_discharge_invariant` | KG with `AssumptionProperty` -> `CandidateInvariant` with source `AssumptionDischarge` |
| `exp_2d_invariant_source_correctly_tagged` | Each new invariant has the correct `InvariantSource` variant |
| `exp_2d_existing_4_patterns_unbroken` | MutexPattern, HandshakePattern, PipelineStability, ResetInit all still discovered from same inputs |
| `exp_2d_multiple_domain_invariants` | KG with CDC + Power + Security entities -> invariants from all 3 domains |
| `exp_2d_invariant_count_increases` | KG with new entities produces more invariants than KG with only legacy entities |
| `exp_2d_invariant_expr_well_formed` | All generated `CandidateInvariant.expr` are valid `VerifyExpr` (not empty/trivial) |

---

### Sprint 3A: Nested Sub-Enums + Convenience Constructors

**Depends on:** ALL Phase 0, 1, 2 sprints (restructure after all variants proven)

**Why:** With 42+ entity types and 40+ relations, the flat enum is unwieldy. In Rust, enum memory footprint equals the largest variant. If one variant holds `Vec<String>` and another holds a single `bool`, every node pays the `Vec<String>` cost. Nested sub-enums isolate domain-specific logic, keep pattern matching clean, and ensure that when the CEGAR loop only examines `Control` entities, it doesn't match against `Power` or `Security` variants. Additionally, domain-specific extraction passes (CDC analyzer, Power analyzer) can match on `HwEntityType::Cdc(_)` without exhaustive listing.

**Files:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

**What:**

Define 9 entity sub-enums:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StructuralEntity {
    Module { name: String, is_top: bool },
    Port { direction: PortDirection, width: u32, domain: Option<String> },
    Signal { width: u32, signal_type: SignalType, domain: Option<String> },
    Register { width: u32, reset_value: Option<u64>, clock: Option<String> },
    Memory { depth: u32, width: u32, ports: u8 },
    Fifo { depth: u32, width: u32 },
    Bus { width: u32, protocol: Option<String> },
    Parameter { value: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlEntity {
    Fsm { states: Vec<String>, initial: Option<String> },
    Counter { width: u32, direction: CounterDirection },
    Arbiter { scheme: ArbitrationScheme, ports: u8 },
    Decoder { input_width: u32, output_width: u32 },
    Mux { inputs: u8, select_width: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TemporalEntity {
    Clock { frequency: Option<String>, domain: String },
    Reset { polarity: ResetPolarity, synchronous: bool },
    Interrupt { priority: Option<u8>, edge_triggered: bool },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProtocolEntity {
    Handshake { valid_signal: String, ready_signal: String },
    Pipeline { stages: u32, stall_signal: Option<String> },
    Transaction { request: String, response: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DataEntity {
    DataPath { width: u32, signed: bool },
    Address { width: u32, base: Option<u64>, range: Option<u64> },
    Configuration { fields: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PropertyEntity {
    SafetyProperty { formula: String },
    LivenessProperty { formula: String },
    FairnessProperty { formula: String },
    ResponseProperty { trigger: String, response: String, bound: Option<u32> },
    MutexProperty { signals: Vec<String> },
    StabilityProperty { signal: String, condition: String },
    AssumptionProperty { formula: String, component: String },
    GuaranteeProperty { formula: String, component: String },
    CoverageProperty { formula: String, target: String },
    InvariantProperty { formula: String, source: String },
    BoundedLivenessProperty { formula: String, bound: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PowerEntity {
    KgPowerDomain { name: String, default_state: String },
    IsolationCell { signal: String, clamp_value: Option<String> },
    RetentionRegister { signal: String, domain: String },
    LevelShifter { signal: String, source_domain: String, dest_domain: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CdcEntity {
    Synchronizer { stages: u32, source_domain: String, dest_domain: String },
    AsyncFifo { depth: u32, source_domain: String, dest_domain: String },
    GrayCodeCounter { width: u32, domain: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SecurityEntity {
    TaintSource { signal: String, label: String },
    TaintSink { signal: String, label: String },
}
```

Root enum:
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HwEntityType {
    Structural(StructuralEntity),
    Control(ControlEntity),
    Temporal(TemporalEntity),
    Protocol(ProtocolEntity),
    Data(DataEntity),
    Property(PropertyEntity),
    Power(PowerEntity),
    Cdc(CdcEntity),
    Security(SecurityEntity),
}
```

Convenience constructors (preserve ergonomics, aid migration):
```rust
impl HwEntityType {
    pub fn module(name: String, is_top: bool) -> Self {
        HwEntityType::Structural(StructuralEntity::Module { name, is_top })
    }
    pub fn clock(frequency: Option<String>, domain: String) -> Self {
        HwEntityType::Temporal(TemporalEntity::Clock { frequency, domain })
    }
    pub fn safety_property(formula: String) -> Self {
        HwEntityType::Property(PropertyEntity::SafetyProperty { formula })
    }
    // ... one per variant
}
```

Similarly, define 10 relation sub-enums (DataFlowRelation, ControlFlowRelation, TemporalRelation, StructuralRelation, ProtocolRelation, SpecificationRelation, PowerRelation, CdcRelation, SecurityRelation, VerificationRelation) with a nested `HwRelation` root.

**RED tests (Section 3A of `phase_hw_ontology_v2.rs`):**

| Test | Assertion |
|---|---|
| `v2_3a_nested_structural_constructible` | `HwEntityType::Structural(StructuralEntity::Module { .. })` compiles |
| `v2_3a_nested_control_constructible` | `HwEntityType::Control(ControlEntity::Fsm { .. })` compiles |
| `v2_3a_nested_temporal_constructible` | `HwEntityType::Temporal(TemporalEntity::Clock { .. })` compiles |
| `v2_3a_nested_protocol_constructible` | `HwEntityType::Protocol(ProtocolEntity::Handshake { .. })` compiles |
| `v2_3a_nested_data_constructible` | `HwEntityType::Data(DataEntity::DataPath { .. })` compiles |
| `v2_3a_nested_property_constructible` | `HwEntityType::Property(PropertyEntity::SafetyProperty { .. })` compiles |
| `v2_3a_nested_power_constructible` | `HwEntityType::Power(PowerEntity::KgPowerDomain { .. })` compiles |
| `v2_3a_nested_cdc_constructible` | `HwEntityType::Cdc(CdcEntity::Synchronizer { .. })` compiles |
| `v2_3a_nested_security_constructible` | `HwEntityType::Security(SecurityEntity::TaintSource { .. })` compiles |
| `v2_3a_convenience_constructor_module` | `HwEntityType::module("m".into(), true)` == `HwEntityType::Structural(StructuralEntity::Module { .. })` |
| `v2_3a_convenience_constructor_clock` | `HwEntityType::clock(None, "sys".into())` works |
| `v2_3a_convenience_constructor_safety` | `HwEntityType::safety_property("G(P)".into())` works |
| `v2_3a_nested_entity_serializes` | All 9 category variants round-trip through serde_json |
| `v2_3a_nested_relation_serializes` | All 10 relation category variants round-trip through serde_json |
| `v2_3a_category_match_exhaustive` | `match entity { Structural(_) | Control(_) | ... }` exhaustive with 9 arms |
| `v2_3a_sub_enum_variant_counts` | StructuralEntity has 8, ControlEntity has 5, TemporalEntity has 3, ProtocolEntity has 3, DataEntity has 3, PropertyEntity has 11, PowerEntity has 4, CdcEntity has 3, SecurityEntity has 2 = 42 total |
| `v2_3a_extraction_produces_nested` | `extract_from_kripke_ast` returns `HwEntityType::Temporal(TemporalEntity::Clock { .. })` (not flat) |
| `v2_3a_invariant_discovery_works` | `discover_invariants` works with nested KG entities |
| `v2_3a_cdc_bridge_works` | `cdc_to_kg` produces nested `HwEntityType::Cdc(_)` entities |
| `v2_3a_power_bridge_works` | `power_to_kg` produces nested `HwEntityType::Power(_)` entities |

---

### Sprint 3B: Migrate All Tests to Nested API

**Depends on:** 3A

**Why:** Sprint 3A introduces the nested structure. This sprint migrates all 87+ existing ontology tests and all downstream consumers from `HwEntityType::Variant { .. }` to the nested form or convenience constructors. No backward compatibility shims — clean break.

**Files:**
- `crates/logicaffeine_tests/tests/phase_hw_ontology.rs` (migrate all 87 tests)
- `crates/logicaffeine_tests/tests/phase_hw_kg_extract.rs` (update pattern matches)
- `crates/logicaffeine_tests/tests/phase_hw_invariants.rs` (update pattern matches)
- `crates/logicaffeine_compile/src/codegen_sva/invariants.rs` (update pattern matches)
- `crates/logicaffeine_tests/tests/phase_hw_ontology_exp.rs` (update if needed)

**What:** Systematic replacement:
```rust
// Before (flat):
HwEntityType::Clock { frequency: None, domain: "sys".into() }
// After (nested):
HwEntityType::Temporal(TemporalEntity::Clock { frequency: None, domain: "sys".into() })
// Or via convenience:
HwEntityType::clock(None, "sys".into())
```

Pattern matches update:
```rust
// Before:
matches!(e, HwEntityType::MutexProperty { .. })
// After:
matches!(e, HwEntityType::Property(PropertyEntity::MutexProperty { .. }))
```

**RED tests (Section 3B of `phase_hw_ontology_v2.rs`):**

| Test | Assertion |
|---|---|
| `v2_3b_all_87_original_tests_pass` | Run existing phase_hw_ontology.rs — all pass with nested types |
| `v2_3b_all_19_kg_extract_tests_pass` | Run phase_hw_kg_extract.rs — all pass |
| `v2_3b_all_11_invariant_tests_pass` | Run phase_hw_invariants.rs — all pass |
| `v2_3b_all_expansion_tests_pass` | Run phase_hw_ontology_exp.rs — all pass with nested types |
| `v2_3b_no_flat_references_in_tests` | Grep: zero occurrences of `HwEntityType::Clock {` (should all be nested or convenience) |
| `v2_3b_json_format_consistent` | KG JSON output produces valid JSON with nested entity structure |
| `v2_3b_cdc_bridge_produces_nested` | `cdc_to_kg()` output entities are `HwEntityType::Cdc(_)` |
| `v2_3b_full_regression_zero_failures` | `cargo test --no-fail-fast -- --skip e2e` exits 0 |

---

## Part V: Sprint Sequencing

```
Phase 0: Activate Dead Code          Phase 1: Bridge Domains
  Sprint 0A ──────────┐                Sprint 1A (CDC bridge)
         |             |                     |
  Sprint 0B ──────┐   |                Sprint 1B (Power+Security bridge)
         |        |   |                     |
         |        |   |               ┌─────┤
         |        |   |               |     |
         |     Sprint 2B              |  Sprint 2A (Compositional)
         |     (missing rels)         |     |
         |                            |     |
         |                      Sprint 2C   |
         |                      (AST extract)|
         |                            |     |
         |                            └──┬──┘
         |                               |
         |                          Sprint 2D
         |                          (invariants)
         |                               |
         └───────────────────────────────┘
                         |
                    Sprint 3A
                    (nested enums)
                         |
                    Sprint 3B
                    (test migration)
```

**Parallel tracks:**
- Phase 0 (0A -> 0B) and Phase 1 (1A, 1B) can execute in parallel — completely independent
- Within Phase 1: 1A and 1B can execute in parallel
- Sprint 2A and Sprint 2B can execute in parallel (after their respective deps)

**Critical path:** 1A -> 2C -> 2D -> 3A -> 3B (longest chain)

**Quick wins:** Sprints 0A, 1A, 1B can start immediately (no prerequisites)

---

## Part VI: Expected Outcomes

| Sprint | New Tests | Cumulative | New/Modified Files |
|---|---|---|---|
| 0A | +15 | 15 | `knowledge_graph.rs` |
| 0B | +14 | 29 | `knowledge_graph.rs` |
| 1A | +12 | 41 | `knowledge_graph.rs`, `cdc.rs` |
| 1B | +15 | 56 | `knowledge_graph.rs`, `power.rs`, `security.rs` |
| 2A | +16 | 72 | `knowledge_graph.rs`, `compositional.rs` |
| 2B | +9 | 81 | `knowledge_graph.rs` |
| 2C | +10 | 91 | `knowledge_graph.rs` |
| 2D | +10 | 101 | `invariants.rs` |
| 3A | +20 | 121 | `knowledge_graph.rs` |
| 3B | +8 | 129 | all test files, `invariants.rs` |

**Final state: 129 new tests. 2 new test files. 7 modified source files.**

Combined with existing 165 ontology/KG/invariant tests: **294+ ontology tests.**

Combined with full hardware verification suite (1165 tests): **1294+ total hardware tests.**

**Ontology expansion:**
- HwEntityType: 28 -> 42 variants (9 nested sub-enums)
- HwRelation: 24 -> 40 variants (10 nested sub-enums)
- InvariantSource: 4 -> 9 variants
- Production extraction coverage: 6/28 (21%) -> 42/42 (100%)
- Domain modules connected to KG: 0/4 -> 4/4 (100%)

---

## Part VII: The Compositional Argument

### Why This Ontology Enables Compositional Verification

The core insight: **a Knowledge Graph that models interfaces, assumptions, and contracts as first-class citizens enables assume-guarantee reasoning without requiring the user to manually decompose specifications.**

1. **Assumption/Guarantee as entities.** When `ComponentSpec.assumes` becomes an `AssumptionProperty` entity in the KG, and `ComponentSpec.guarantees` becomes a `GuaranteeProperty`, the compositional verification algorithm can query the graph to find dependencies: "Which component's guarantee satisfies this component's assumption?" The `DependsOn` edge encodes exactly this relationship.

2. **Coverage as vacuity detection.** Every `AssumptionProperty` automatically generates a `CoverageProperty`. If the assumption is never triggered during verification, the guarantee is vacuously true — it passes because it was never tested. The `CoverageProperty` catches this.

3. **Cross-domain invariants.** A unified KG containing CDC, Power, and Security entities enables invariant discovery patterns that span domain boundaries. Example: a `Synchronizer` entity in the CDC sub-graph, connected via `CrossesDomain` to signals in two `PowerDomain` entities — the invariant discovery engine can generate: "if source domain is powered off, synchronizer output must be stable (not transitioning)." This cross-domain invariant is invisible to domain-specific analyzers working in isolation.

4. **Nested enums as domain firewalls.** When the CEGAR loop operates on a `Control` entity, it matches `HwEntityType::Control(ctrl)` and never sees `Power` or `Security` variants. This is not just ergonomic — it prevents accidental conflation of domain-specific logic. The CDC analyzer matches `HwEntityType::Cdc(cdc)` and operates in its own domain. Composition happens at the KG level through typed edges, not through monolithic pattern matches.

### What This Is NOT

- This is NOT a new verification algorithm. The verification infrastructure exists (Z3, BMC, k-induction, CEGAR).
- This is NOT a UML-style architecture diagram. Every entity type participates in extraction, invariant discovery, or bridge functions.
- This IS a typed, composable, formally grounded ontology that transforms a flat collection of labels into a structured knowledge graph capable of supporting assume-guarantee reasoning, cross-domain invariant discovery, and domain-isolated analysis passes.
