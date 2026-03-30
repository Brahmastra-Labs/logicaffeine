# HARDWARE.md: Logos for Hardware Formal Verification

## 1. Vision: The Logical Grounding Layer

Logos is uniquely positioned to transform hardware formal verification. No other tool combines:

1. **Kripke semantics** baked into the language kernel (modal worlds, accessibility relations, discourse representation)
2. **All three Futamura projections** operational (436 tests, genuine self-application verified)
3. **Z3 integration** with a decoupled verification IR (Tarski invariant)
4. **Natural language input** via a dual-mode AST (English <-> First-Order Logic)

### The Gap in Current Hardware Verification

NVIDIA's **AssertionForge** (2024-2025) represents the state of the art for LLM-assisted formal verification. Its pipeline: English hardware spec → LLM builds a Knowledge Graph → LLM generates SVAs → JasperGold model-checks against RTL. The problem, which NVIDIA acknowledges: the entire English → KG step is LLM-driven with **zero formal grounding**. Ambiguity and incompleteness of specifications is the core challenge.

Prior work on NL-to-SVA — **TempCNL** (Manchester), **FLAG** (2025) — uses controlled natural languages or template-based approaches. None employ a general-purpose NL-to-FOL transpiler with a proof kernel, and none close the loop with formal equivalence checking.

**What nobody does**: check SVAs against a *formally parsed specification*. Everyone checks SVAs against the RTL hardware model. Because they don't have the formal specification. LOGOS gives them one.

### The LOGOS Pipeline

```
English Hardware Spec (human or LLM)
         |
    [1] LOGOS Parser + Kripke Lowering
         |
  Temporal FOL (Kripke frame with explicit worlds)
         |
    [2] Knowledge Graph Construction
         |
  Enriched KG (signals, temporal deps, protocol states)
         |
    [3] LLM + KG → SVA Candidates
         |
  Candidate SVAs (unverified)
         |
    [4] Z3 Semantic Equivalence: SVA ≡ FOL?
         |           |
      PASS          FAIL + counterexample → back to LLM
         |
  Verified SVAs
         |
    [5] JasperGold / Yosys / Formal Tool
         |
  RTL Model Check Result
```

Step [4] is the key innovation. LOGOS provides the formally parsed specification (the Kripke-lowered FOL). An LLM generates SVA candidates. Z3 asks: does the SVA's semantics *match* the FOL's semantics? Not just "is the SVA internally consistent" — but "does this SVA express the same property as the formal spec?" This catches semantic mismatches before the expensive model checker runs.

### Correct-by-Construction via Projection

For the cases where we bypass the LLM entirely: P1 (First Futamura Projection) specializes the Kripke interpreter with a specific hardware spec to produce a zero-overhead monitor. The monitor is a *specialized version* of the verified kernel — mathematically identical to the input's logical intent, with no translation error.

---

## 2. What Already Exists

Every component below is production-tested. The hardware extension *reuses* this infrastructure.

### 2.1 Kripke Semantics

**File:** `crates/logicaffeine_language/src/semantics/kripke.rs`

The Kripke lowering pass transforms modal operators into explicit possible-world quantification:

```
Surface:  Diamond(Fly(x))
Deep:     Exists w'(Accessible(w0, w') And Fly(x, w'))

Surface:  Box(Fly(x))
Deep:     ForAll w'(Accessible(w0, w') Implies Fly(x, w'))
```

**`KripkeContext`** tracks world variables (`w0`, `w1`, `w2`, ...) and the current world during lowering. `lower_modal()` dispatches on `ModalDomain` to generate the correct accessibility predicate:

```rust
let access_name = match vector.domain {
    ModalDomain::Alethic => interner.intern("Accessible_Alethic"),
    ModalDomain::Deontic => interner.intern("Accessible_Deontic"),
};
```

Force determines quantifier type: `force > 0.5` yields universal (necessity/Box), `force <= 0.5` yields existential (possibility/Diamond).

### 2.2 Modal Vector Theory

**File:** `crates/logicaffeine_language/src/ast/logic.rs` (line 369)

```rust
pub struct ModalVector {
    pub domain: ModalDomain,   // Alethic | Deontic
    pub force: f32,             // 1.0 = necessity, 0.5 = possibility, graded
    pub flavor: ModalFlavor,    // Root (narrow scope) | Epistemic (wide scope)
}
```

The continuous force value (0.0-1.0) enables graded modality. For hardware, this maps to probabilistic verification and non-deterministic bus contention modeling.

### 2.3 Discourse Representation Structures

**File:** `crates/logicaffeine_language/src/drs.rs`

`WorldState` manages modal contexts across sentences with accessibility barriers:

| BoxType | Accessibility | Hardware Analogue |
|---------|---------------|-------------------|
| `Main` | Global | Top-level module |
| `ModalScope` | Blocked from reality | Hypothetical state |
| `ConditionalAntecedent` | Antecedent -> consequent | Implication trigger |
| `NegationScope` | Impenetrable | Safety property (never) |

Modal subordination ("A wolf might walk in. It would eat you.") maps to temporal chaining in hardware ("If req rises, ack would follow").

### 2.4 Temporal Operators (Current)

**File:** `crates/logicaffeine_language/src/ast/logic.rs` (line 165)

```rust
pub enum TemporalOperator {
    Past,     // P(phi): "it was the case that phi"
    Future,   // F(phi): "it will be the case that phi"
}
```

Priorian tense logic with Reichenbach time constraints (Event time, Reference time, Speech time). Sufficient for linguistic temporality but not for hardware — needs LTL extension.

### 2.5 Verification IR

**File:** `crates/logicaffeine_verify/src/ir.rs`

```rust
pub enum VerifyType { Int, Bool, Object }
pub enum VerifyOp { Add, Sub, Mul, Div, Eq, Neq, Gt, Lt, Gte, Lte, And, Or, Implies }
pub enum VerifyExpr {
    Int(i64), Bool(bool), Var(String),
    Binary { op, left, right },
    Not(Box<VerifyExpr>),
    ForAll { vars, body },
    Exists { vars, body },
    Apply { name, args },  // uninterpreted functions
}
```

The `Apply` variant handles modals/temporals as opaque functions — Z3 reasons structurally. The Tarski invariant guarantees this IR has zero dependency on the language or compile crates.

### 2.6 Proof Engine

**File:** `crates/logicaffeine_proof/src/lib.rs`

Backward-chaining proof search with inference rules including:
- `ModalAccess` — access world from modal scope
- `ModalGeneralization` — generalize across accessible worlds
- `TemporalTransitivity` — chain temporal relations
- `OracleVerification` — Z3 fallback for undecidable goals

### 2.7 Futamura Projections

**File:** `crates/logicaffeine_compile/src/compile.rs` (lines 4261-4762)

All three projections operational. PE written in LOGOS itself (pe_source.logos, pe_bti_source.logos, pe_mini_source.logos). Programs encoded as CExpr/CStmt algebraic data. 436 tests, 0 regressions.

- **P1:** `projection1_source_real(core_types, interpreter, program)` — encode program, run PE, decompile residual
- **P2:** `projection2_source_real()` — specialize PE with interpreter, extract specialized evaluators
- **P3:** `projection3_source_real()` — specialize PE with PE, produce compiler generator

`run_logos_source()` bridges LOGOS PE to execution: compile to Rust, build temp Cargo project, run binary, capture output.

### 2.8 Kernel

**File:** `crates/logicaffeine_kernel/src/`

Pure Calculus of Constructions with decision procedures: `ring.rs` (polynomial equality), `lia.rs` (linear integer arithmetic), `cc.rs` (congruence closure with E-graphs), `omega.rs` (Presburger arithmetic), `simp.rs` (rewriting), `positivity.rs`, `termination.rs`.

The Milner invariant guarantees no lexicon dependency — the kernel is purely logical.

---

## 3. Architectural Changes

**0 new crates. 7 crates modified** (language, lexicon, proof, verify, compile, tests + lsp for completions). **2 new files** within existing crates (`verify/src/sva.rs`, `language/src/semantics/knowledge_graph.rs`). All 4 invariants preserved.

### 3a. `logicaffeine_language` — Temporal Logic Extension

#### Extend `TemporalOperator`

**File:** `crates/logicaffeine_language/src/ast/logic.rs` (line 165)

```rust
pub enum TemporalOperator {
    // Existing (Priorian tense logic)
    Past,
    Future,
    // LTL operators (Pnueli, 1977)
    Always,      // G(phi) — globally/invariant
    Eventually,  // F(phi) — finally/liveness
    Next,        // X(phi) — next state
}
```

#### New Binary Temporal Operators

```rust
pub enum BinaryTemporalOp {
    Until,      // phi U psi — phi holds until psi becomes true
    Release,    // phi R psi — dual of Until (psi holds until phi releases it)
    WeakUntil,  // phi W psi — phi holds until psi, or phi holds forever
}
```

#### New `LogicExpr` Variant

```rust
LogicExpr::TemporalBinary {
    operator: BinaryTemporalOp,
    left: &'a LogicExpr<'a>,   // 8 bytes
    right: &'a LogicExpr<'a>,  // 8 bytes
}
// Total: 1 + 8 + 8 = 17 bytes + padding = 24 bytes. Within 48-byte budget.
```

#### New `ModalDomain::Temporal`

```rust
pub enum ModalDomain {
    Alethic,
    Deontic,
    Temporal,  // accessibility = state transition relation
}
```

Adding `Temporal` to `ModalDomain` slots directly into the existing Kripke lowering dispatch at `kripke.rs:305`. The accessibility predicate becomes `Accessible_Temporal(w, w')` — which IS the hardware state transition relation.

#### Kripke Lowering for Temporal Domain

Extend `lower_modal()` and add `lower_temporal()` in `kripke.rs`:

```
G(phi) -> ForAll w'(Accessible_Temporal(w, w') -> phi(w'))
F(phi) -> Exists w'(Reachable_Temporal(w, w') And phi(w'))
X(phi) -> ForAll w'(Next_Temporal(w, w') -> phi(w'))

phi U psi -> psi(w) Or (phi(w) And Exists w'(Next_Temporal(w, w') And (phi U psi)(w')))
```

Where:
- `Accessible_Temporal` = reflexive-transitive closure of transition (for G/F)
- `Next_Temporal` = single-step transition (for X)
- `Reachable_Temporal` = transitive closure (for F)

#### Extended `KripkeContext`

```rust
pub struct KripkeContext {
    world_counter: u32,
    current_world: Symbol,
    clock_counter: u32,           // NEW: discrete timestep tracking
    domain_hint: Option<ModalDomain>, // NEW: disambiguate temporal vs modal lowering
}
```

#### CTL Path Quantifiers

CTL composes path quantifiers (A = all paths, E = some path) with temporal operators. This maps directly onto existing Kripke infrastructure:

| CTL | Decomposition | Kripke Encoding |
|-----|---------------|-----------------|
| `AG(phi)` | Box_Temporal(Always(phi)) | `ForAll w'(Accessible_Temporal(w,w') -> phi(w'))` |
| `EF(phi)` | Diamond_Temporal(Eventually(phi)) | `Exists w'(Reachable_Temporal(w,w') And phi(w'))` |
| `AX(phi)` | Box_Temporal(Next(phi)) | `ForAll w'(Next_Temporal(w,w') -> phi(w'))` |
| `EU(phi,psi)` | Diamond_Temporal(Until(phi,psi)) | `Exists path(phi U psi along path)` |

No new AST node needed — CTL is Box/Diamond composed with the extended temporal operators.

---

### 3b. `logicaffeine_lexicon` — Hardware Vocabulary

#### Sort Extension

**File:** `crates/logicaffeine_lexicon/src/types.rs`

Add `Signal` to the `Sort` enum (currently 13 variants: Entity, Physical, Animate, Human, Plant, Place, Time, Abstract, Information, Event, Celestial, Value, Group).

#### Lexicon Audit: What Already Exists

Several hardware-relevant entries already exist in `lexicon.json`:

| Lemma | Current State | Action Needed |
|-------|---------------|---------------|
| Signal | Exists as noun (Abstract sort) + verb (Activity) | Reclassify noun sort: Abstract -> Signal |
| Wire | Exists as noun (Physical sort) | Reclassify sort: Physical -> Signal |
| Register | Exists as verb (Achievement) | Add noun entry with Signal sort |
| Trigger | Exists as verb (Achievement) | No change needed |
| Rise | Exists as verb (Achievement, irregular: rose/risen/rising) | No change needed |
| Fall | Exists as verb (Achievement, irregular: fell/fallen/falling) | No change needed |
| Stable | Exists as adjective (Intersective, Gradable) | No change needed |
| Valid | Exists as adjective (Intersective) | No change needed |
| Ready | Exists as adjective (Intersective) | No change needed |

#### New Nouns to Add

| Lemma | Sort | Features | Notes |
|-------|------|----------|-------|
| Clock | Signal | Inanimate, Count | Does not exist |
| Bus | Signal | Inanimate, Count | Does not exist (only in plural rules) |
| Latch | Signal | Inanimate, Count | Does not exist |
| Flip-flop | Signal | Inanimate, Count | Does not exist |

#### New Verbs to Add

| Lemma | Vendler Class | Transitivity | Notes |
|-------|---------------|--------------|-------|
| Assert | Achievement | Transitive | Currently a TokenType keyword, not a verb — needs dual handling |
| Toggle | Achievement | Intransitive | Does not exist |
| Propagate | Activity | Intransitive | Does not exist |
| Acknowledge | Achievement | Transitive | Does not exist |
| Stabilize | Achievement | Intransitive | Does not exist |

#### New Adjectives to Add

| Lemma | Type | Notes |
|-------|------|-------|
| High | Intersective | Does not exist (only in compound "high school") |
| Low | Intersective | Does not exist |

Note: "Rising"/"Falling" already exist as gerund forms of Rise/Fall verbs and do not need separate adjective entries.

#### Block Types for Hardware Context

**File:** `crates/logicaffeine_language/src/token.rs` (line 78, `BlockType` enum)

Currently 11 variants: Theorem, Main, Definition, Proof, Example, Logic, Note, Function, TypeDef, Policy, Requires, No.

Add two new variants:

```rust
Hardware,   // ## Hardware — signal declarations, clock definitions
Property,   // ## Property "name" — temporal assertions
```

Both parse in `ParserMode::Declarative` (natural language mode), following the pattern of Theorem/Logic blocks.

#### Disambiguating "always", "eventually", and "never"

**Current state:**
- "always" and "eventually" are in the `adverbs` array in `lexicon.json` — they are plain adverbs, NOT TokenType keywords
- "never" is `TokenType::Never` (Negative Polarity Item at `token.rs:136`)

**Disambiguation strategy:**
- Inside `## Property` blocks: the parser promotes "always" to `TemporalOperator::Always`, "eventually" to `TemporalOperator::Eventually`, and "never" remains NPI but generates `G(Not(...))` (always-not)
- Outside `## Property` blocks: they remain adverbs/NPI with no temporal operator semantics
- This follows the existing pattern where `ParserMode` (Declarative vs Imperative) changes keyword interpretation

---

### 3c. `logicaffeine_proof` — Temporal Inference Rules

**File:** `crates/logicaffeine_proof/src/lib.rs`

#### New Rules

```rust
// In InferenceRule enum:
TemporalInduction,      // G(P) iff P(s0) And ForAll s(P(s) -> P(next(s)))
TemporalUnfolding,      // G(P) iff P And X(G(P))
EventualityProgress,    // F(P) proven by finding a witness state
UntilInduction,         // P U Q by induction on trace length
```

**Temporal Induction** is the workhorse for safety properties. To prove `G(P)` (P holds always):
1. Prove `P(s0)` — P holds in the initial state
2. Prove `ForAll s(P(s) -> P(next(s)))` — P is preserved by every transition

This is the standard k-induction scheme used by all hardware model checkers.

#### New `ProofExpr` Variant

```rust
ProofExpr::TemporalBinary {
    operator: String,   // "Until", "Release", "WeakUntil"
    left: Box<ProofExpr>,
    right: Box<ProofExpr>,
}
```

Conversion `LogicExpr::TemporalBinary` -> `ProofExpr::TemporalBinary` lives in `language/src/proof_convert.rs`, preserving the Liskov invariant (proof crate has no language dependency).

---

### 3d. `logicaffeine_verify` — Bitvector & Bounded Model Checking

**File:** `crates/logicaffeine_verify/src/ir.rs`

#### New Types

```rust
pub enum VerifyType {
    Int,
    Bool,
    Object,
    BitVector(u32),                           // fixed-width bitvector (Z3 BitVecSort)
    Array(Box<VerifyType>, Box<VerifyType>),   // Z3 ArraySort (register files, memories)
    State,                                     // uninterpreted sort for FSM states
}
```

`BitVector(n)` is essential — hardware signals are fixed-width. Z3's native bitvector theory handles overflow, masking, and shifting correctly.

`Array(idx, elem)` models register files and memories as Z3 arrays with `select`/`store`.

#### New Expressions

```rust
pub enum VerifyExpr {
    // ... existing variants ...

    // Bitvector literals and operations
    BitVecConst { width: u32, value: u64 },
    BitVecOp { op: BitVecOp, left: Box<VerifyExpr>, right: Box<VerifyExpr> },
    BitVecExtract { high: u32, low: u32, operand: Box<VerifyExpr> },
    BitVecConcat(Box<VerifyExpr>, Box<VerifyExpr>),

    // Temporal (for BMC encoding)
    AtState { state: Box<VerifyExpr>, expr: Box<VerifyExpr> },
    Transition { from: Box<VerifyExpr>, to: Box<VerifyExpr> },

    // Array theory
    Select { array: Box<VerifyExpr>, index: Box<VerifyExpr> },
    Store { array: Box<VerifyExpr>, index: Box<VerifyExpr>, value: Box<VerifyExpr> },
}

pub enum BitVecOp {
    And, Or, Xor, Not,
    Shl, Shr, AShr,
    Add, Sub, Mul,
    ULt, SLt, ULe, SLe,
    Eq,
}
```

#### Bounded Model Checking in VerificationSession

**File:** `crates/logicaffeine_verify/src/solver.rs`

BMC unrolls the transition relation K steps and checks if any K-length trace violates the property:

```rust
impl VerificationSession {
    /// Verify a temporal property via bounded model checking.
    /// Unrolls the transition relation `bound` steps.
    pub fn verify_temporal(
        &self,
        initial: &VerifyExpr,
        transition: &VerifyExpr,
        property: &VerifyExpr,
        bound: u32,
    ) -> VerificationResult { ... }
}
```

Internally: declare state variables `s_0, s_1, ..., s_K`, assume `initial(s_0)`, assume `transition(s_i, s_{i+1})` for each step, check `Not(property)` satisfiability. If SAT, return counterexample trace.

#### Z3 Encoding

Extend the `Encoder` in `solver.rs` with:
- `z3::ast::BV` for BitVecConst/BitVecOp/BitVecExtract/BitVecConcat
- `z3::ast::Array` for Select/Store
- Unrolled state variables for BMC

---

### 3e. `logicaffeine_compile` — SVA/PSL Codegen

#### Directory Structure

```
crates/logicaffeine_compile/src/codegen_sva/
    mod.rs       — top-level SVA generation entry point
    emit.rs      — SVA syntax emission (property, sequence, assert, cover)
    types.rs     — SVA type mapping (logic, reg, wire, bit widths)
    psl.rs       — PSL (IEEE 1850) output variant
    monitor.rs   — Rust runtime monitor code generation
```

Feature flag in `Cargo.toml`:
```toml
[features]
codegen-sva = []
```

#### Temporal FOL to SVA Mapping

| Logos Temporal FOL | SVA Output |
|--------------------|------------|
| `G(P)` | `property p; @(posedge clk) P; endproperty` |
| `G(P -> X(Q))` | `property p; @(posedge clk) P \|=> Q; endproperty` |
| `G(P -> F(Q))` | `property p; @(posedge clk) P \|-> ##[1:$] Q; endproperty` |
| `G(P -> X^n(Q))` | `property p; @(posedge clk) P \|-> ##n Q; endproperty` |
| `G(Not(P And Q))` | `property p; @(posedge clk) !(P && Q); endproperty` |
| `P U Q` | `sequence s; P ##[1:$] Q; endsequence` |
| `F(P)` | `property p; @(posedge clk) s_eventually(P); endproperty` |

#### Assertion Wrappers

```systemverilog
// Safety property (assert)
assert property (p_data_integrity) else $error("Data integrity violation");

// Liveness property (cover)
cover property (p_handshake_complete);

// Environment assumption (assume)
assume property (p_valid_input);
```

The backend determines assertion type through a two-level dispatch:

1. **Explicit keywords** (highest priority): "assuming" -> `assume`, "cover" -> `cover`
2. **Temporal operator** (default): `G(...)` / "always" / "never" -> `assert` (safety), `F(...)` / "eventually" -> `cover` (liveness)
3. **Modal force override** (in `## Property` blocks using modal verbs): `force > 0.8` -> `assert`, `0.5 < force <= 0.8` -> `cover`, `force <= 0.5` -> `assume` (see Appendix B)

#### PSL Output

IEEE 1850 Property Specification Language as an alternative target:

```psl
-- Safety
assert always (req -> next_event(ack)[1:5]);

-- Liveness
assert eventually! (done);
```

#### Rust Monitor Generation

For runtime verification (simulation or emulation), emit a Rust struct that checks properties cycle-by-cycle:

```rust
pub struct DataIntegrityMonitor {
    prev_data_in: u8,
    prev_valid: bool,
}

impl DataIntegrityMonitor {
    pub fn check(&mut self, data_in: u8, data_out: u8, valid: bool) -> bool {
        let ok = !self.prev_valid || data_out == self.prev_data_in;
        self.prev_data_in = data_in;
        self.prev_valid = valid;
        ok
    }
}
```

This is what P1 produces when specializing the Kripke interpreter — a tight, zero-overhead monitor with all interpretive machinery folded away.

---

### 3f. `logicaffeine_tests` — Hardware Test Phases

| Test File | Coverage | Phase |
|-----------|----------|-------|
| `phase_hw_temporal.rs` | LTL/CTL operators, Kripke lowering for temporal domain, nested modals | 1 |
| `phase_hw_lexicon.rs` | Hardware vocabulary parsing, "always"/"eventually" disambiguation | 2 |
| `phase_hw_verify.rs` | Z3 bitvector reasoning, BMC unrolling, counterexample generation | 3 |
| `phase_hw_codegen_sva.rs` | SVA/PSL output correctness, assertion wrapper selection | 4 |
| `phase_hw_futamura.rs` | P1 specialized monitors | 5 |
| `phase_hw_filter.rs` | Consistency filter: reject contradictory/vacuous properties | 6 |
| `phase_hw_knowledge_graph.rs` | KG extraction from FOL, signal/state/edge nodes, JSON serialization | 7 |
| `phase_hw_equivalence.rs` | SVA semantic model, parse_sva, sva_to_verify_expr, Z3 FOL≡SVA | 8 |

### 3g. `logicaffeine_verify` — SVA Semantic Model (New)

**File:** `crates/logicaffeine_verify/src/sva.rs`

New module for SVA semantic modeling. Preserves the Tarski invariant — depends only on `VerifyExpr`/`VerifyType` from the same crate, not on language or compile.

Contains:
- `SvaExpr` enum: Signal, Const, Rose, Fell, Past, And, Or, Not, Eq, Implication, Delay, Repetition, SEventually
- `parse_sva()`: parse SVA subset text into `SvaExpr`
- `sva_to_verify_expr()`: translate `SvaExpr` to `VerifyExpr` for Z3

### 3h. `logicaffeine_language` — Knowledge Graph (New)

**File:** `crates/logicaffeine_language/src/semantics/knowledge_graph.rs`

New module for extracting a knowledge graph from Kripke-lowered FOL. Preserves the Liskov invariant — the KG types are defined within the language crate, not the proof crate.

Contains:
- `HwKnowledgeGraph`, `KgSignal`, `KgState`, `KgProperty`, `KgEdge` types
- `extract_knowledge_graph()`: walks `LogicExpr` tree to build KG
- `to_json()`: serialize for LLM consumption
- Reuses `WorldState.telescope_candidates` from DRS for cross-world signal tracking

---

## 4. The Futamura Strategy

We do not hand-write the SVA generator. We *derive* it. P1 is the immediate target. P2 and P3 are the research horizon — start with P1 and assess results before committing to the full tower.

### 4.1 The Kripke Interpreter

Write `kripke_interpreter.logos` — a LOGOS program that interprets a Timed-Kripke frame:

```logos
## A HwSignal is one of:
    A HwBit with name Text.
    A HwVec with name Text and width Int.

## A HwTransition is one of:
    A HwTrans with from Text and guard CExpr and to Text and actions Seq of CStmt.

## A HwFSM is one of:
    A HwMachine with name Text and states Seq of Text
        and signals Seq of HwSignal and initial Text
        and transitions Seq of HwTransition.

## A HwProperty is one of:
    A HwAlways with body CExpr.
    A HwEventually with body CExpr.
    A HwNext with body CExpr.
    A HwUntil with left CExpr and right CExpr.

## A HwWorld is one of:
    A HwState with name Text and signals Map of Text to Int and tick Int.
```

The interpreter evaluates `HwProperty` against `HwFSM` by:
1. Starting at `initial` state
2. Following transitions whose `guard` evaluates to true
3. At each state (world), checking the property body

This interpreter is purely functional — no side effects, no IO. It is a textbook target for partial evaluation.

### 4.2 First Projection: Specialized Monitor

```
P1: PE(kripke_interpreter, hardware_spec) -> specialized_monitor
```

Given a specific `HwFSM` (e.g., an AXI bus protocol), P1 folds away:
- The FSM structure (states, transitions are constants)
- The signal declarations (widths are known)
- The interpreter dispatch (no generic evaluation loop)

The residual is a *direct* state machine that checks the property against the *specific* hardware — zero interpretive overhead. Jones optimality (already verified for LOGOS) guarantees no unnecessary env/funcs lookups remain.

### 4.3 Second Projection: Hardware Compiler (Research Horizon)

```
P2: PE(PE, kripke_interpreter) -> hardware_compiler
```

By specializing the partial evaluator itself with the Kripke interpreter, we generate a **Hardware-to-Monitor Compiler**. This compiler takes any `HwFSM` and instantly emits the specialized monitor — no PE runtime cost.

**Status**: Aspirational. Depends on P1 results. The existing P2 infrastructure (verified in `phase_futamura.rs`) handles simpler interpreters; the Kripke interpreter's temporal recursion may require PE extensions.

### 4.4 Third Projection: Compiler Generator (Research Horizon)

```
P3: PE(PE, PE) -> compiler_generator
```

The compiler generator takes *any* interpreter (Kripke, RTL, cycle-accurate, etc.) and produces a compiler for it. If a new hardware description language or verification standard emerges, describe its formal semantics as a LOGOS interpreter, feed it to the P3-generated compiler generator, and receive a fully verified compiler.

**Status**: Long-term research goal. Depends on P2. The existing P3 infrastructure works for expression-level PEs; scaling to the full Kripke interpreter is an open question.

### 4.5 Encoding Hardware for PE

Hardware types are encoded using the existing CExpr/CStmt framework from `CORE_TYPES_FOR_PE`:

- `HwSignal` -> `CNewVariant("HwBit", ["name"], [CText("clk")])` or `CNewVariant("HwVec", ["name", "width"], [CText("data"), CInt(8)])`
- `HwTransition` -> `CNewVariant("HwTrans", ["from", "guard", "to", "actions"], [...])`
- `HwProperty` -> `CNewVariant("HwAlways", ["body"], [CExpr...])`

The PE already handles CNewVariant folding (Sprint J, Phase G: CMapGet on CNewVariant). `encode_program_source()` generates the construction statements.

---

## 5. The Verification Pipeline (LLM-in-the-Loop)

This is the core practical innovation — what differentiates LOGOS from every existing NL-to-SVA tool.

### 5.1 The Five-Stage Pipeline

```
[Stage 1] English Hardware Spec
              |
         LOGOS Parser + Kripke Lowering (logicaffeine_language)
              |
[Stage 2] Temporal FOL (Kripke frame with explicit worlds)
              |
         Knowledge Graph Construction
              |
[Stage 3] Enriched Knowledge Graph
              |
         LLM generates SVA candidates from KG
              |
[Stage 4] SVA Candidates (unverified)
              |
         Z3 Semantic Equivalence: SVA ≡ FOL?
              |           |
           PASS          FAIL + counterexample → refine LLM prompt
              |
[Stage 5] Verified SVA → JasperGold / Yosys
```

### 5.2 Stage 1: Formal Parsing (What We Have)

The LOGOS parser produces Kripke-lowered FOL from English. This is the *ground truth* — the formally parsed specification that no other tool in the literature provides.

If the English does not parse into well-formed temporal FOL, reject immediately. The parser checks:
- Signal names are declared in the `## Hardware` block
- Temporal operators are well-scoped (no `Next` without a clock context)
- Bit-width compatibility (8-bit signal compared to 8-bit literal, not 16-bit)

### 5.3 Stage 2: Knowledge Graph Construction (New)

The FOL output is structured but not queryable. We extract a **Knowledge Graph** from the Kripke-lowered FOL to give the LLM structured context.

**KG Nodes:**
- **Signal nodes**: Each declared signal with type, width, and role (input/output/internal)
- **State nodes**: Each Kripke world (FSM state) extracted from the temporal structure
- **Property nodes**: Each `## Property` block with its temporal operator type (safety/liveness/fairness)
- **Predicate nodes**: Each atomic proposition (e.g., `High(req, w)`, `Eq(data_out, data_in, w)`)

**KG Edges:**
- **temporal**: `w0 --Next_Temporal--> w1` (state transitions)
- **triggers**: `High(req, w) --implies--> High(ack, w')` (implication chains from the FOL)
- **constrains**: Signal `req` constrains signal `ack` via property P (cross-signal dependencies)
- **type_of**: `data_in --BitVector(8)--> type` (width/type information)

**Implementation**: The KG is extracted from the `LogicExpr` tree by walking the Kripke-lowered output. The DRS (`drs.rs`) already tracks discourse referents across worlds — we reuse `WorldState.telescope_candidates` for cross-world signal tracking.

**Crate location**: `logicaffeine_language` (KG extraction is a semantic analysis pass, not a verification or compilation concern). New file: `semantics/knowledge_graph.rs`.

```rust
pub struct HwKnowledgeGraph {
    pub signals: Vec<KgSignal>,
    pub states: Vec<KgState>,
    pub properties: Vec<KgProperty>,
    pub edges: Vec<KgEdge>,
}

pub struct KgSignal {
    pub name: Symbol,
    pub width: u32,
    pub role: SignalRole,  // Input, Output, Internal, Clock
}

pub struct KgState {
    pub world: Symbol,     // w0, w1, ...
    pub predicates: Vec<KgPredicate>,  // what holds at this state
}

pub struct KgEdge {
    pub from: KgNodeId,
    pub to: KgNodeId,
    pub relation: KgRelation,  // Temporal, Triggers, Constrains, TypeOf
}
```

**Serialization**: The KG serializes to JSON for LLM consumption. The LLM receives structured context, not raw FOL text.

### 5.4 Stage 3: LLM SVA Generation (External)

The LLM receives:
1. The original English spec (for context)
2. The KG in JSON (for structure)
3. A prompt template requesting SVA for each property node

This stage is external to LOGOS — any LLM can be plugged in. The key is that the KG gives the LLM *formally grounded* context, not just the raw English.

### 5.5 Stage 4: Z3 Semantic Equivalence (The Key Innovation)

**The question nobody else asks**: Does the LLM-generated SVA express the *same* property as the formally parsed FOL?

Current industry practice: SVAs are checked against RTL (the hardware implementation). Nobody checks SVAs against the specification, because nobody *has* a formal specification. LOGOS provides one.

**Equivalence query:**

Given:
- `phi_FOL`: the LOGOS-generated temporal FOL (ground truth)
- `phi_SVA`: the SVA candidate's semantics translated to VerifyExpr

Ask Z3:
```
ForAll signals(phi_FOL(signals) <-> phi_SVA(signals))
```

If valid: the SVA faithfully captures the spec. If invalid: Z3 returns a counterexample — concrete signal values where the SVA and spec diverge.

**SVA Semantic Model**: To ask this question, we need to translate SVA semantics into `VerifyExpr`. This is NOT a full SVA parser — it's a semantic model of the SVA subset we generate/accept:

```rust
// New file: crates/logicaffeine_verify/src/sva.rs

pub enum SvaExpr {
    Signal(String),
    Const(u64, u32),          // value, width
    Rose(Box<SvaExpr>),       // $rose()
    Fell(Box<SvaExpr>),       // $fell()
    Past(Box<SvaExpr>, u32),  // $past(sig, n)
    And(Box<SvaExpr>, Box<SvaExpr>),
    Or(Box<SvaExpr>, Box<SvaExpr>),
    Not(Box<SvaExpr>),
    Eq(Box<SvaExpr>, Box<SvaExpr>),
    Implication {              // |-> or |=>
        antecedent: Box<SvaExpr>,
        consequent: Box<SvaExpr>,
        overlapping: bool,     // |-> (overlapping) vs |=> (non-overlapping)
    },
    Delay {                    // ##[min:max]
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>,      // None = unbounded ($)
    },
    Repetition {               // [*min:max]
        body: Box<SvaExpr>,
        min: u32,
        max: Option<u32>,
    },
    SEventually(Box<SvaExpr>),
}

/// Translate SVA semantics into VerifyExpr for Z3 equivalence checking
pub fn sva_to_verify_expr(sva: &SvaExpr, bound: u32) -> VerifyExpr { ... }

/// Parse a subset of SVA text into SvaExpr
pub fn parse_sva(text: &str) -> Result<SvaExpr, SvaParseError> { ... }
```

The translation unrolls temporal operators to bounded depth (matching BMC) and encodes signal references as bitvector variables.

**Consistency check** (in addition to equivalence):

```rust
let mut session = VerificationSession::new();
// Declare all signals from ## Hardware block
for sig in &kg.signals {
    session.declare(&sig.name, VerifyType::BitVector(sig.width));
}

// Check all properties are mutually consistent
for (i, p1) in properties.iter().enumerate() {
    for p2 in properties[i+1..].iter() {
        session.assume(p1);
        let result = session.verify(p2);
        // If p1 contradicts p2, reject
    }
}
```

**Vacuity detection** (catches the most common LLM failure):

A property `G(P -> Q)` where `P` can never happen is vacuously true. Check:
1. Assert `Exists s(P(s))` — the antecedent is satisfiable
2. If unsatisfiable, reject: "This property is vacuously true: the trigger condition can never occur"

### 5.6 Stage 5: Formal Tool Integration (External)

The verified SVA feeds into JasperGold, Yosys, or any IEEE 1800 formal tool. This stage is external to LOGOS.

**JasperGold integration** via TCL scripting:
```tcl
# Auto-generated by LOGOS
analyze -sv {logos_generated.sv}
elaborate -top <module>
# Properties generated by LOGOS, verified against spec by Z3
check_property -all
```

**Yosys/sby integration** for open-source flows:
```
[tasks]
prove

[options]
mode prove
depth 20

[engines]
smtbmc z3

[script]
read -sv logos_generated.sv
prep -top <module>

[files]
logos_generated.sv
```

### 5.7 Comparison: LOGOS vs AssertionForge vs TempCNL vs FLAG

| Capability | AssertionForge (NVIDIA) | TempCNL (Manchester) | FLAG (2025) | LOGOS |
|------------|------------------------|---------------------|-------------|------|
| Input language | English | Controlled NL (templates) | Controlled NL | Unrestricted English |
| Formal parsing | No (LLM-driven) | Pattern matching | Template matching | Kripke-lowered FOL |
| Knowledge Graph | Yes (LLM-built) | No | No | Yes (FOL-derived) |
| Proof kernel | No | No | No | CoC + backward chaining |
| SVA checking | Against RTL only | Against RTL only | Against RTL only | **Against spec (Z3) + RTL** |
| Vacuity detection | No | No | Limited | Z3-based |
| Counterexamples | From model checker | From model checker | From model checker | **From spec checker + model checker** |
| Partial evaluation | No | No | No | Futamura P1/P2/P3 |

The unique contribution: **Z3 semantic equivalence checking of SVA against the formally parsed specification.** Nobody else has the specification in a form that admits this query.

---

## 6. English Input Examples

### Example 1: Data Integrity

**Logos:**
```
## Hardware
Let data_in be a signal of 8 bits.
Let data_out be a signal of 8 bits.
Let clk be a clock signal.
Let valid be a signal of 1 bit.

## Property "Data Integrity"
Always, if valid is high, then data_out equals data_in after one clock.
```

**Kripke lowering:**
```
ForAll w'(Accessible_Temporal(w, w') ->
    (High(valid, w) -> Eq(data_out, data_in, next(w))))
```

**SVA output:**
```systemverilog
property p_data_integrity;
    @(posedge clk) valid |=> (data_out == data_in);
endproperty
assert property (p_data_integrity);
```

### Example 2: Request-Acknowledge Handshake

**Logos:**
```
## Property "Handshake"
Always, if the request signal rises, then the acknowledge signal becomes high
within 5 clocks.
```

**SVA output:**
```systemverilog
property p_handshake;
    @(posedge clk) $rose(req) |-> ##[1:5] ack;
endproperty
assert property (p_handshake);
```

### Example 3: Mutual Exclusion

**Logos:**
```
## Property "Mutex"
Never do grant_a and grant_b hold simultaneously.
```

**SVA output:**
```systemverilog
property p_mutex;
    @(posedge clk) !(grant_a && grant_b);
endproperty
assert property (p_mutex);
```

### Example 4: Liveness

**Logos:**
```
## Property "Progress"
If a request occurs, then eventually an acknowledgment occurs.
```

**SVA output:**
```systemverilog
property p_progress;
    @(posedge clk) req |-> s_eventually(ack);
endproperty
assert property (p_progress);
```

### Example 5: Until

**Logos:**
```
## Property "Hold"
The request signal remains high until the grant signal becomes high.
```

**SVA output:**
```systemverilog
sequence s_hold;
    req[*1:$] ##1 grant;
endsequence
cover property (@(posedge clk) s_hold);
```

---

## 7. How to Expose

### 7.1 CLI (`largo`)

New subcommand in the `cli` feature (`logicaffeine_cli`):

```bash
# Parse English spec → Kripke FOL → SVA (direct generation, no LLM)
largo hw emit spec.logos --format sva --output assertions.sv
largo hw emit spec.logos --format psl --output assertions.psl

# Parse English spec → Kripke FOL → Knowledge Graph JSON
largo hw kg spec.logos --output kg.json

# Verify SVA against spec (Z3 semantic equivalence)
largo hw check spec.logos --sva candidate.sv

# Full pipeline: parse → verify consistency → emit SVA
largo hw pipeline spec.logos --output verified.sv

# Futamura P1: specialize monitor for a given hardware spec
largo hw specialize spec.logos --interpreter kripke --output monitor.rs
```

### 7.2 Programmatic API

Public functions exposed from `logicaffeine_compile`:

```rust
// crates/logicaffeine_compile/src/lib.rs (new public API)

/// Parse English hardware spec → Kripke-lowered FOL (as formatted string)
pub fn compile_hw_spec(source: &str) -> Result<HwSpecOutput, HwError>;

/// Extract Knowledge Graph from hardware spec
pub fn extract_hw_knowledge_graph(source: &str) -> Result<HwKnowledgeGraph, HwError>;

/// Emit SVA from hardware spec (direct, no LLM)
pub fn emit_hw_sva(source: &str) -> Result<String, HwError>;

/// Check SVA text against spec for semantic equivalence
pub fn check_sva_equivalence(spec_source: &str, sva_text: &str) -> Result<EquivalenceResult, HwError>;

/// Full pipeline: parse → consistency check → emit verified SVA
pub fn hw_pipeline(source: &str) -> Result<HwPipelineOutput, HwError>;

pub struct HwPipelineOutput {
    pub fol: String,              // Kripke-lowered FOL
    pub kg_json: String,          // Knowledge Graph as JSON
    pub sva: String,              // Generated SVA
    pub consistency: Vec<String>, // Any warnings from consistency check
}

pub struct EquivalenceResult {
    pub equivalent: bool,
    pub counterexample: Option<CounterExample>,  // signal values where they diverge
    pub details: String,
}
```

### 7.3 First Milestone (Demo for Alexander)

The minimum working demo that proves the thesis:

**Milestone 0: Temporal FOL output** (Phase 1 only)
```
Input:  "Always, if req is high, then next, ack is high."
Output: ∀w'(Accessible_Temporal(w₀, w') → (High(req, w') → ∀w''(Next_Temporal(w', w'') → High(ack, w''))))
```
This proves LOGOS can parse hardware specs into Kripke-lowered temporal FOL. No SVA generation needed. The FOL itself is the contribution.

**Milestone 1: English → SVA** (Phases 1-4)
```
Input:
  ## Hardware
  Let req be a signal of 1 bit.
  Let ack be a signal of 1 bit.
  Let clk be a clock signal.

  ## Property "Handshake"
  Always, if the request signal rises, then the acknowledge signal becomes high within 5 clocks.

Output:
  property p_handshake;
      @(posedge clk) $rose(req) |-> ##[1:5] ack;
  endproperty
  assert property (p_handshake);
```

**Milestone 2: Z3 equivalence checking** (Phases 1-4 + 8)
```
$ largo hw check spec.logos --sva candidate.sv
✓ p_handshake: SVA ≡ FOL (equivalent)
✗ p_mutex: SVA ≢ FOL — counterexample: grant_a=1, grant_b=1, cycle=3
```
This is the demo that shows nobody else can do this.

**Milestone 3: Full pipeline with KG** (All phases)
```
$ largo hw pipeline spec.logos --output verified.sv --kg kg.json
Parsed 3 properties from spec
Built knowledge graph: 4 signals, 3 properties, 7 edges
Consistency check: ✓ all properties mutually consistent
Vacuity check: ✓ no vacuous properties
Generated SVA: verified.sv (3 assertions)
Knowledge graph: kg.json (for LLM consumption)
```

### 7.4 Full Pipeline Example (All 5 Stages)

**Input** (`axi_spec.logos`):
```
## Hardware
Let AWVALID be a signal of 1 bit.
Let AWREADY be a signal of 1 bit.
Let WVALID be a signal of 1 bit.
Let WREADY be a signal of 1 bit.
Let BVALID be a signal of 1 bit.
Let BREADY be a signal of 1 bit.
Let clk be a clock signal.

## Property "Write Address Handshake"
Always, if AWVALID is high, then eventually AWREADY is high.

## Property "Write Data Follows Address"
Always, if AWVALID is high and AWREADY is high,
then eventually WVALID is high.

## Property "Write Response"
Always, if WVALID is high and WREADY is high,
then eventually BVALID is high.
```

**Stage 1: Kripke FOL** (automatic):
```
∀w'(Accessible_Temporal(w₀, w') →
    (High(AWVALID, w') → ∃w''(Reachable_Temporal(w', w'') ∧ High(AWREADY, w''))))

∀w'(Accessible_Temporal(w₀, w') →
    ((High(AWVALID, w') ∧ High(AWREADY, w')) →
        ∃w''(Reachable_Temporal(w', w'') ∧ High(WVALID, w''))))
...
```

**Stage 2: Knowledge Graph** (automatic, JSON):
```json
{
  "signals": [
    {"name": "AWVALID", "width": 1, "role": "input"},
    {"name": "AWREADY", "width": 1, "role": "output"},
    ...
  ],
  "properties": [
    {"name": "Write Address Handshake", "type": "liveness", "operator": "G(P → F(Q))"},
    ...
  ],
  "edges": [
    {"from": "AWVALID", "to": "AWREADY", "relation": "triggers", "property": "Write Address Handshake"},
    {"from": "Write Address Handshake", "to": "Write Data Follows Address", "relation": "temporal_order"},
    ...
  ]
}
```

**Stage 3: LLM generates SVA** (external, using KG as context):
```systemverilog
property p_write_addr_handshake;
    @(posedge clk) AWVALID |-> s_eventually(AWREADY);
endproperty

property p_write_data_follows_addr;
    @(posedge clk) (AWVALID && AWREADY) |-> s_eventually(WVALID);
endproperty

property p_write_response;
    @(posedge clk) (WVALID && WREADY) |-> s_eventually(BVALID);
endproperty
```

**Stage 4: Z3 equivalence check** (automatic):
```
✓ p_write_addr_handshake: SVA ≡ FOL (equivalent at bound 20)
✓ p_write_data_follows_addr: SVA ≡ FOL (equivalent at bound 20)
✓ p_write_response: SVA ≡ FOL (equivalent at bound 20)
Consistency: ✓ all 3 properties mutually consistent
Vacuity: ✓ all antecedents satisfiable
```

**Stage 5: JasperGold** (external):
```tcl
analyze -sv {axi_verified.sv}
elaborate -top axi_master
check_property -all
```

### 7.5 Phase Dependency Graph

```
Phase 1 (Temporal Operators) ──────────┬──────────────────────┐
         |                              |                      |
Phase 2 (Lexicon) ─────────────────────┤                      |
         |                              |                      |
Phase 3 (Verify IR) ──────────────────┐|                      |
         |                             ||                      |
Phase 4 (SVA Codegen) ←── 1,2,3 ─────┤|                      |
         |                             ||                      |
Phase 5 (Futamura P1) ←── 1 ─────────┘|                      |
         |                              |                      |
Phase 6 (Filter) ←── 1,2,3 ───────────┘                      |
         |                                                     |
Phase 7 (Knowledge Graph) ←── 1,2 ────────────────────────────┘
         |
Phase 8 (SVA Equivalence) ←── 3,4
```

**Parallelizable**:
- Phases 1-3 can develop concurrently (different crates)
- Phase 5 (Futamura) only needs Phase 1
- Phase 7 (KG) only needs Phases 1-2

**Critical path**: Phase 1 → Phase 4 → Phase 8 (temporal operators → SVA codegen → equivalence checking)

---

## 8. Implementation Phases (TDD)

Each phase follows RED/GREEN: write failing tests first, then implement until they pass. Tests live in `crates/logicaffeine_tests/tests/`. All test functions use `#[test]` and follow existing naming conventions.

---

### Phase 1: Temporal Operators

**Crates:** `logicaffeine_language`, `logicaffeine_proof`
**Test file:** `phase_hw_temporal.rs`

**Steps:**
1. Extend `TemporalOperator` with `Always`, `Eventually`, `Next`
2. Add `BinaryTemporalOp` enum and `LogicExpr::TemporalBinary`
3. Add `ModalDomain::Temporal`
4. Extend Kripke lowering for temporal domain in `kripke.rs`
5. Add temporal inference rules to proof engine
6. Add `ProofExpr::TemporalBinary`
7. Update `proof_convert.rs`

**RED Tests:**

```rust
// phase_hw_temporal.rs
use logicaffeine_language::compile::compile_kripke;

// --- 1. AST node sizes remain within budget ---

#[test]
fn temporal_binary_fits_48_byte_budget() {
    // Adding TemporalBinary must not blow LogicExpr past 48 bytes
    assert!(std::mem::size_of::<logicaffeine_language::ast::logic::LogicExpr>() <= 48);
}

// --- 2. Kripke lowering for G (Always) ---

#[test]
fn kripke_always_lowers_to_universal_temporal() {
    // G(P) -> ForAll w'(Accessible_Temporal(w0, w') -> P(w'))
    let result = compile_kripke("Always, every signal is valid.");
    let output = result.unwrap();
    assert!(output.contains("Accessible_Temporal"));
    assert!(output.contains("∀"));
    assert!(output.contains("w"));
}

#[test]
fn kripke_always_generates_temporal_not_alethic() {
    let result = compile_kripke("Always, the request is high.");
    let output = result.unwrap();
    assert!(output.contains("Accessible_Temporal"));
    assert!(!output.contains("Accessible_Alethic"));
    assert!(!output.contains("Accessible_Deontic"));
}

// --- 3. Kripke lowering for F (Eventually) ---

#[test]
fn kripke_eventually_lowers_to_existential_temporal() {
    // F(P) -> Exists w'(Reachable_Temporal(w0, w') And P(w'))
    let result = compile_kripke("Eventually, the acknowledge signal is high.");
    let output = result.unwrap();
    assert!(output.contains("Reachable_Temporal"));
    assert!(output.contains("∃"));
}

// --- 4. Kripke lowering for X (Next) ---

#[test]
fn kripke_next_lowers_to_single_step() {
    // X(P) -> ForAll w'(Next_Temporal(w0, w') -> P(w'))
    let result = compile_kripke("Next, the output equals the input.");
    let output = result.unwrap();
    assert!(output.contains("Next_Temporal"));
}

// --- 5. Binary temporal: Until ---

#[test]
fn kripke_until_lowers_correctly() {
    // P U Q -> Q(w) Or (P(w) And Exists w'(Next_Temporal(w,w') And (P U Q)(w')))
    let result = compile_kripke("The request is high until the grant is high.");
    let output = result.unwrap();
    // Until generates both immediate check and recursive next-step
    assert!(output.contains("Next_Temporal"));
}

// --- 6. Nested temporal + modal ---

#[test]
fn kripke_nested_always_implies_next() {
    // G(P -> X(Q)) — safety with one-cycle delay
    let result = compile_kripke("Always, if the valid signal is high, then next, the output is stable.");
    let output = result.unwrap();
    assert!(output.contains("Accessible_Temporal"));
    assert!(output.contains("Next_Temporal"));
}

// --- 7. Temporal preserves world threading ---

#[test]
fn kripke_temporal_threads_worlds_through_predicates() {
    let result = compile_kripke("Always, the data is valid.");
    let output = result.unwrap();
    // All predicates must carry world arguments
    assert!(output.contains("Valid(data, w"));
}

// --- 8. Proof engine temporal rules ---

#[test]
fn proof_temporal_induction_rule_exists() {
    use logicaffeine_proof::InferenceRule;
    // The rule must exist as a variant
    let _rule = InferenceRule::TemporalInduction;
}

#[test]
fn proof_temporal_unfolding_rule_exists() {
    use logicaffeine_proof::InferenceRule;
    let _rule = InferenceRule::TemporalUnfolding;
}

#[test]
fn proof_convert_handles_temporal_binary() {
    use logicaffeine_language::proof_convert::logic_expr_to_proof_expr;
    // Construct a TemporalBinary LogicExpr and convert it
    // Must not panic, must produce ProofExpr::TemporalBinary
    // (Exact construction depends on arena setup — see phase_kripke.rs pattern)
}

// --- 9. CTL composition ---

#[test]
fn kripke_ag_is_box_temporal_always() {
    // AG(P) = Box(Temporal, Always(P))
    // "It must always be the case that the signal is valid" in temporal context
    let result = compile_kripke("It must always be that the signal is valid.");
    let output = result.unwrap();
    assert!(output.contains("∀"));
    assert!(output.contains("Accessible_Temporal"));
}

#[test]
fn kripke_ef_is_diamond_temporal_eventually() {
    // EF(P) = Diamond(Temporal, Eventually(P))
    // "It can eventually be the case that the signal is high"
    let result = compile_kripke("The signal can eventually be high.");
    let output = result.unwrap();
    assert!(output.contains("∃"));
    assert!(output.contains("Reachable_Temporal"));
}
```

---

### Phase 2: Hardware Lexicon

**Crates:** `logicaffeine_lexicon`, `logicaffeine_language`
**Test file:** `phase_hw_lexicon.rs`

**Steps:**
1. Add `Sort::Signal` to lexicon types and reclassify Signal/Wire nouns
2. Add new hardware nouns/verbs/adjectives to `lexicon.json`
3. Add `BlockType::Hardware` and `BlockType::Property` to `token.rs`
4. Implement "always"/"eventually" disambiguation in `## Property` blocks
5. Parse signal declarations with bit widths

**RED Tests:**

```rust
// phase_hw_lexicon.rs
use logicaffeine_language::compile::{compile, compile_kripke};

// --- 1. Hardware block parsing ---

#[test]
fn hardware_block_parses_signal_declarations() {
    let input = r#"
## Hardware
Let data_in be a signal of 8 bits.
Let clk be a clock signal.
"#;
    let result = compile(input);
    assert!(result.is_ok(), "Hardware block should parse: {:?}", result.err());
}

#[test]
fn property_block_parses_temporal_assertion() {
    let input = r#"
## Property "Data Valid"
Always, the data is valid.
"#;
    let result = compile_kripke(input);
    assert!(result.is_ok(), "Property block should parse: {:?}", result.err());
}

// --- 2. "always" disambiguation ---

#[test]
fn always_in_property_block_is_temporal_operator() {
    let input = r#"
## Property "Invariant"
Always, the signal is high.
"#;
    let result = compile_kripke(input);
    let output = result.unwrap();
    // In property block: "always" -> TemporalOperator::Always -> Kripke lowering
    assert!(output.contains("Accessible_Temporal"));
}

#[test]
fn always_outside_property_block_remains_adverb() {
    // "always" in a Logic block should stay a scopal adverb, not temporal
    let input = "John always runs.";
    let result = compile(input);
    let output = result.unwrap();
    assert!(!output.contains("Accessible_Temporal"));
}

// --- 3. "eventually" disambiguation ---

#[test]
fn eventually_in_property_block_is_temporal_operator() {
    let input = r#"
## Property "Liveness"
Eventually, the acknowledge signal is high.
"#;
    let result = compile_kripke(input);
    let output = result.unwrap();
    assert!(output.contains("Reachable_Temporal"));
}

// --- 4. "never" generates G(Not(...)) ---

#[test]
fn never_in_property_block_generates_always_not() {
    let input = r#"
## Property "Safety"
Never is the error signal high.
"#;
    let result = compile_kripke(input);
    let output = result.unwrap();
    // never -> G(Not(...))
    assert!(output.contains("Accessible_Temporal"));
    assert!(output.contains("¬"));
}

// --- 5. Hardware vocabulary parses ---

#[test]
fn clock_noun_parses() {
    let input = "The clock signal rises.";
    let result = compile(input);
    assert!(result.is_ok());
}

#[test]
fn bus_noun_parses() {
    let input = "The bus carries data.";
    let result = compile(input);
    assert!(result.is_ok());
}

#[test]
fn acknowledge_verb_parses() {
    let input = "The peripheral acknowledges the request.";
    let result = compile(input);
    assert!(result.is_ok());
}

#[test]
fn high_low_adjectives_parse() {
    let input = "The signal is high.";
    let result = compile(input);
    assert!(result.is_ok());

    let input2 = "The signal is low.";
    let result2 = compile(input2);
    assert!(result2.is_ok());
}

// --- 6. Signal sort classification ---

#[test]
fn signal_noun_has_signal_sort() {
    use logicaffeine_lexicon::types::Sort;
    use logicaffeine_lexicon::lookup_noun;
    let entry = lookup_noun("signal").expect("signal should be in lexicon");
    assert_eq!(entry.sort, Sort::Signal);
}

#[test]
fn clock_noun_has_signal_sort() {
    use logicaffeine_lexicon::types::Sort;
    use logicaffeine_lexicon::lookup_noun;
    let entry = lookup_noun("clock").expect("clock should be in lexicon");
    assert_eq!(entry.sort, Sort::Signal);
}

// --- 7. Bit-width signal declarations ---

#[test]
fn signal_bit_width_declaration_parses() {
    let input = r#"
## Hardware
Let data be a signal of 32 bits.
"#;
    let result = compile(input);
    assert!(result.is_ok());
}
```

---

### Phase 3: Verification IR Extensions

**Crate:** `logicaffeine_verify`
**Test file:** `phase_hw_verify.rs`
**Requires:** `verification` feature flag and Z3 installed

**Steps:**
1. Add `VerifyType::BitVector`, `Array`, `State`
2. Add `VerifyExpr` bitvector and temporal variants
3. Add `BitVecOp` enum
4. Extend Z3 `Encoder` with BV/Array sort encoding
5. Implement BMC unrolling in `VerificationSession`
6. Implement vacuity detection

**RED Tests:**

```rust
// phase_hw_verify.rs
#![cfg(feature = "verification")]
use logicaffeine_verify::{
    VerifyExpr, VerifyType, VerifyOp, BitVecOp,
    VerificationSession, VerificationResult,
};

// --- 1. Bitvector type creation ---

#[test]
fn bitvector_type_exists() {
    let _ty = VerifyType::BitVector(8);
    let _ty32 = VerifyType::BitVector(32);
}

#[test]
fn array_type_exists() {
    let _ty = VerifyType::Array(
        Box::new(VerifyType::BitVector(5)),  // 5-bit index (32 entries)
        Box::new(VerifyType::BitVector(8)),  // 8-bit values
    );
}

// --- 2. Bitvector operations ---

#[test]
fn bitvec_const_and_eq_verify() {
    let mut session = VerificationSession::new();
    session.declare("x", VerifyType::BitVector(8));
    session.assume(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("x".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 42 }),
    });
    let result = session.verify(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("x".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 42 }),
    });
    assert!(matches!(result, VerificationResult::Valid));
}

#[test]
fn bitvec_overflow_wraps() {
    // 8-bit: 255 + 1 = 0 (wrap-around)
    let mut session = VerificationSession::new();
    session.declare("x", VerifyType::BitVector(8));
    session.assume(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("x".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 255 }),
    });
    let sum = VerifyExpr::BitVecOp {
        op: BitVecOp::Add,
        left: Box::new(VerifyExpr::Var("x".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 1 }),
    };
    let result = session.verify(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(sum),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 0 }),
    });
    assert!(matches!(result, VerificationResult::Valid));
}

#[test]
fn bitvec_extract_slice() {
    // Extract bits [3:0] from an 8-bit value
    let mut session = VerificationSession::new();
    session.declare("x", VerifyType::BitVector(8));
    session.assume(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("x".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 0xAB }),
    });
    let lo = VerifyExpr::BitVecExtract {
        high: 3, low: 0,
        operand: Box::new(VerifyExpr::Var("x".into())),
    };
    let result = session.verify(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(lo),
        right: Box::new(VerifyExpr::BitVecConst { width: 4, value: 0xB }),
    });
    assert!(matches!(result, VerificationResult::Valid));
}

// --- 3. Bounded Model Checking ---

#[test]
fn bmc_simple_counter_invariant() {
    // Counter starts at 0, increments each cycle, prove counter < 256 for 8-bit
    let mut session = VerificationSession::new();

    // Initial state: counter = 0
    let initial = VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("counter_0".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 0 }),
    };

    // Transition: counter_{i+1} = counter_i + 1
    let transition = |i: u32| VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var(format!("counter_{}", i + 1))),
        right: Box::new(VerifyExpr::BitVecOp {
            op: BitVecOp::Add,
            left: Box::new(VerifyExpr::Var(format!("counter_{}", i))),
            right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 1 }),
        }),
    };

    // Unroll 5 steps
    for i in 0..5 {
        session.declare(&format!("counter_{}", i), VerifyType::BitVector(8));
    }
    session.declare("counter_5", VerifyType::BitVector(8));
    session.assume(&initial);
    for i in 0..5 {
        session.assume(&transition(i));
    }

    // Property: counter_5 == 5
    let result = session.verify(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("counter_5".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 5 }),
    });
    assert!(matches!(result, VerificationResult::Valid));
}

#[test]
fn bmc_mutex_violation_detected() {
    // Two grants cannot both be high — verify Z3 finds violation when both assumed
    let mut session = VerificationSession::new();
    session.declare("grant_a", VerifyType::BitVector(1));
    session.declare("grant_b", VerifyType::BitVector(1));

    // Assume both grants are 1 (violation scenario)
    session.assume(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("grant_a".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 1, value: 1 }),
    });
    session.assume(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(VerifyExpr::Var("grant_b".into())),
        right: Box::new(VerifyExpr::BitVecConst { width: 1, value: 1 }),
    });

    // Property: NOT(grant_a AND grant_b)
    let mutex = VerifyExpr::Not(Box::new(VerifyExpr::Binary {
        op: BitVecOp::And.into(),
        left: Box::new(VerifyExpr::Var("grant_a".into())),
        right: Box::new(VerifyExpr::Var("grant_b".into())),
    }));
    let result = session.verify(&mutex);
    // Should FAIL — the assumptions violate mutex
    assert!(matches!(result, VerificationResult::Invalid { .. }));
}

// --- 4. Vacuity detection ---

#[test]
fn vacuity_detection_catches_unreachable_antecedent() {
    // If antecedent can never happen, property is vacuously true — reject
    let mut session = VerificationSession::new();
    session.declare("x", VerifyType::BitVector(8));

    // Assume x > 300 (impossible for 8-bit unsigned: max 255)
    session.assume(&VerifyExpr::Binary {
        op: BitVecOp::ULt.into(),
        left: Box::new(VerifyExpr::BitVecConst { width: 8, value: 255 }),
        right: Box::new(VerifyExpr::Var("x".into())),
    });

    // Any property under unreachable assumption is vacuously true
    let result = session.verify(&VerifyExpr::Bool(false));
    // Should detect vacuity (assumptions are unsatisfiable)
    assert!(matches!(result, VerificationResult::Valid));
    // But vacuity check should flag it:
    let is_vacuous = session.check_vacuity();
    assert!(is_vacuous);
}

// --- 5. Array theory ---

#[test]
fn array_select_store_roundtrip() {
    let mut session = VerificationSession::new();
    session.declare("mem", VerifyType::Array(
        Box::new(VerifyType::BitVector(5)),
        Box::new(VerifyType::BitVector(8)),
    ));

    // Store 42 at address 3, then select address 3 — should equal 42
    let stored = VerifyExpr::Store {
        array: Box::new(VerifyExpr::Var("mem".into())),
        index: Box::new(VerifyExpr::BitVecConst { width: 5, value: 3 }),
        value: Box::new(VerifyExpr::BitVecConst { width: 8, value: 42 }),
    };
    let selected = VerifyExpr::Select {
        array: Box::new(stored),
        index: Box::new(VerifyExpr::BitVecConst { width: 5, value: 3 }),
    };
    let result = session.verify(&VerifyExpr::Binary {
        op: BitVecOp::Eq.into(),
        left: Box::new(selected),
        right: Box::new(VerifyExpr::BitVecConst { width: 8, value: 42 }),
    });
    assert!(matches!(result, VerificationResult::Valid));
}
```

---

### Phase 4: SVA/PSL Codegen

**Crate:** `logicaffeine_compile`
**Test file:** `phase_hw_codegen_sva.rs`
**Feature flag:** `codegen-sva`

**Steps:**
1. Create `codegen_sva/` directory (mod.rs, emit.rs, types.rs, psl.rs, monitor.rs)
2. Implement temporal FOL -> SVA property/sequence mapping
3. Implement assertion wrapper selection (assert/cover/assume)
4. Add PSL output variant
5. Add Rust runtime monitor generation

**RED Tests:**

```rust
// phase_hw_codegen_sva.rs
#![cfg(feature = "codegen-sva")]
use logicaffeine_compile::codegen_sva::emit_sva;

// --- 1. Always -> assert property ---

#[test]
fn sva_always_generates_assert_property() {
    let input = r#"
## Property "Invariant"
Always, if valid is high, then data_out equals data_in.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("property"));
    assert!(sva.contains("assert property"));
    assert!(sva.contains("@(posedge clk)"));
    assert!(sva.contains("|=>") || sva.contains("|->"));
}

// --- 2. Eventually -> cover property ---

#[test]
fn sva_eventually_generates_cover_or_s_eventually() {
    let input = r#"
## Property "Liveness"
If a request occurs, then eventually an acknowledgment occurs.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("s_eventually") || sva.contains("cover property"));
}

// --- 3. Never -> assert with negation ---

#[test]
fn sva_never_generates_assert_not() {
    let input = r#"
## Property "Mutex"
Never do grant_a and grant_b hold simultaneously.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("assert property"));
    assert!(sva.contains("!") || sva.contains("not"));
    assert!(sva.contains("grant_a"));
    assert!(sva.contains("grant_b"));
}

// --- 4. Delay operator ---

#[test]
fn sva_after_n_clocks_generates_delay() {
    let input = r#"
## Property "Delay"
Always, if valid is high, then data_out equals data_in after 2 clocks.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("##2") || sva.contains("##[2"));
}

// --- 5. Bounded liveness ---

#[test]
fn sva_within_n_clocks_generates_bounded_range() {
    let input = r#"
## Property "Handshake"
Always, if the request rises, then the acknowledge becomes high within 5 clocks.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("##[1:5]") || sva.contains("##[0:5]"));
}

// --- 6. Until -> sequence ---

#[test]
fn sva_until_generates_sequence() {
    let input = r#"
## Property "Hold"
The request remains high until the grant becomes high.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("sequence") || sva.contains("[*1:$]"));
}

// --- 7. Property naming ---

#[test]
fn sva_property_name_matches_spec() {
    let input = r#"
## Property "Data Integrity"
Always, the data is valid.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("p_data_integrity") || sva.contains("data_integrity"));
}

// --- 8. PSL output variant ---

#[test]
fn psl_always_generates_assert_always() {
    use logicaffeine_compile::codegen_sva::emit_psl;
    let input = r#"
## Property "Invariant"
Always, the signal is valid.
"#;
    let psl = emit_psl(input).unwrap();
    assert!(psl.contains("assert always"));
}

// --- 9. Rust monitor generation ---

#[test]
fn monitor_generates_check_function() {
    use logicaffeine_compile::codegen_sva::emit_monitor;
    let input = r#"
## Property "Data Check"
Always, if valid is high, then data_out equals data_in.
"#;
    let monitor = emit_monitor(input).unwrap();
    assert!(monitor.contains("fn check"));
    assert!(monitor.contains("struct"));
}

// --- 10. Multiple properties in one spec ---

#[test]
fn sva_multiple_properties_all_emitted() {
    let input = r#"
## Property "Safety"
Always, the signal is valid.

## Property "Liveness"
Eventually, the done signal is high.
"#;
    let sva = emit_sva(input).unwrap();
    assert!(sva.contains("p_safety") || sva.contains("safety"));
    assert!(sva.contains("p_liveness") || sva.contains("liveness"));
}
```

---

### Phase 5: Futamura Integration

**Crate:** `logicaffeine_compile`
**Test file:** `phase_hw_futamura.rs`

**Steps:**
1. Write `kripke_interpreter.logos` — hardware Kripke frame interpreter
2. Define `HwSignal`, `HwTransition`, `HwFSM`, `HwProperty` types in LOGOS
3. Demonstrate P1: `PE(kripke_interp, spec)` -> specialized monitor
4. Demonstrate P2: `PE(PE, kripke_interp)` -> hardware compiler
5. Demonstrate P3 applicability for new hardware DSLs

**RED Tests:**

```rust
// phase_hw_futamura.rs
use logicaffeine_compile::compile::run_logos_source;

// --- 1. Hardware types defined in LOGOS ---

#[test]
fn hw_types_define_and_construct() {
    // HwSignal, HwTransition, HwFSM, HwProperty types can be defined and instantiated
    let source = r#"
## A HwSignal is one of:
    A HwBit with name Text.
    A HwVec with name Text and width Int.

## Main
Let clk be a new HwBit with name "clk".
Let data be a new HwVec with name "data" and width 8.
Show the name of clk.
Show the width of data.
"#;
    let output = run_logos_source(source).unwrap();
    assert!(output.contains("clk"));
    assert!(output.contains("8"));
}

#[test]
fn hw_fsm_encodes_state_machine() {
    let source = r#"
## A HwState is one of:
    A HwS with name Text.

## A HwTrans is one of:
    A HwT with from Text and to Text.

## Main
Let idle be a new HwS with name "IDLE".
Let active be a new HwS with name "ACTIVE".
Let t1 be a new HwT with from "IDLE" and to "ACTIVE".
Show the name of idle.
Show the to of t1.
"#;
    let output = run_logos_source(source).unwrap();
    assert!(output.contains("IDLE"));
    assert!(output.contains("ACTIVE"));
}

// --- 2. Kripke interpreter in LOGOS ---

#[test]
fn kripke_interpreter_evaluates_always_property() {
    // A simple Kripke interpreter that checks G(P) over a fixed trace
    let source = r#"
## To checkAlways given trace (Seq of Bool):
    Repeat for each val in trace:
        If val is false:
            Return false.
    Return true.

## Main
Let trace be (true, true, true, true).
Let result be checkAlways given trace.
Show result.
"#;
    let output = run_logos_source(source).unwrap();
    assert!(output.contains("true"));
}

#[test]
fn kripke_interpreter_detects_always_violation() {
    let source = r#"
## To checkAlways given trace (Seq of Bool):
    Repeat for each val in trace:
        If val is false:
            Return false.
    Return true.

## Main
Let trace be (true, true, false, true).
Let result be checkAlways given trace.
Show result.
"#;
    let output = run_logos_source(source).unwrap();
    assert!(output.contains("false"));
}

#[test]
fn kripke_interpreter_evaluates_eventually_property() {
    let source = r#"
## To checkEventually given trace (Seq of Bool):
    Repeat for each val in trace:
        If val is true:
            Return true.
    Return false.

## Main
Let trace be (false, false, true, false).
Let result be checkEventually given trace.
Show result.
"#;
    let output = run_logos_source(source).unwrap();
    assert!(output.contains("true"));
}

// --- 3. P1: Specialize Kripke interpreter with specific trace ---

#[test]
fn p1_specializes_kripke_check_with_known_property() {
    // PE(kripke_interpreter, always_true_property) should fold to `true`
    // This test uses the existing projection1_source_real infrastructure
    use logicaffeine_compile::compile::{
        projection1_source_real, pe_source_text, core_types_for_pe,
    };

    let interpreter = r#"
## To checkProp given signals (Seq of Int):
    Let allOk be true.
    Repeat for each s in signals:
        If s is less than 0:
            Set allOk to false.
    Return allOk.

## Main
Let sigs be (1, 2, 3, 4, 5).
Let r be checkProp given sigs.
Show r.
"#;
    let result = projection1_source_real(core_types_for_pe(), "", interpreter);
    assert!(result.is_ok());
    let residual = result.unwrap();
    // Specialized program should fold the check to "true" (all signals >= 0)
    assert!(residual.contains("true") || residual.contains("Show true"));
}

// --- 4. P1: Specialized monitor has no interpreter dispatch ---

#[test]
fn p1_residual_has_no_generic_dispatch() {
    use logicaffeine_compile::compile::{
        projection1_source_real, core_types_for_pe,
    };

    let interpreter = r#"
## To evalSignal given val (Int):
    If val is greater than 0:
        Return true.
    Return false.

## Main
Let result be evalSignal given 42.
Show result.
"#;
    let result = projection1_source_real(core_types_for_pe(), "", interpreter);
    assert!(result.is_ok());
    let residual = result.unwrap();
    // No generic evalSignal function definition should remain
    // The call should be folded to the result
    assert!(!residual.contains("## To evalSignal"));
}
```

---

### Phase 6: Hallucination Filter

**Crates:** `logicaffeine_language`, `logicaffeine_verify`, `logicaffeine_compile`
**Test file:** `phase_hw_filter.rs`
**Requires:** `verification` feature flag

**Steps:**
1. Add `verify_hw_spec()` to `logicaffeine_compile::verification` — top-level entry point that composes parser + Kripke lowering + Z3 consistency
2. Add `verify_specification_consistency()` to `VerificationSession` in `logicaffeine_verify`
3. Implement vacuity detection for antecedent reachability
4. Add `HwVerificationError` enum with `Contradiction { counterexample }` and `Vacuous { property }` variants
5. Wire into the compilation pipeline as optional pass (triggered by `## Hardware` + `## Property` blocks)

**RED Tests:**

```rust
// phase_hw_filter.rs
#![cfg(feature = "verification")]

// --- 1. Well-formed specs pass the filter ---

#[test]
fn filter_passes_consistent_spec() {
    use logicaffeine_compile::verification::verify_hw_spec;
    let input = r#"
## Hardware
Let req be a signal of 1 bit.
Let ack be a signal of 1 bit.

## Property "Handshake"
Always, if req is high, then eventually ack is high.
"#;
    let result = verify_hw_spec(input);
    assert!(result.is_ok());
}

// --- 2. Contradictory specs are rejected ---

#[test]
fn filter_rejects_contradictory_properties() {
    use logicaffeine_compile::verification::verify_hw_spec;
    let input = r#"
## Hardware
Let sig be a signal of 1 bit.

## Property "P1"
Always, sig is high.

## Property "P2"
Always, sig is low.
"#;
    let result = verify_hw_spec(input);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("contradictory") || err.to_string().contains("Contradictory"));
}

// --- 3. Vacuously true specs are flagged ---

#[test]
fn filter_flags_vacuous_property() {
    use logicaffeine_compile::verification::verify_hw_spec;
    let input = r#"
## Hardware
Let x be a signal of 1 bit.

## Property "Vacuous"
Always, if x is high and x is low simultaneously, then the output is valid.
"#;
    // Antecedent (x high AND x low) is impossible — vacuously true
    let result = verify_hw_spec(input);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("vacuous") || err.to_string().contains("Vacuous"));
}

// --- 4. Malformed English is rejected at parse gate ---

#[test]
fn filter_rejects_unparseable_spec() {
    use logicaffeine_compile::verification::verify_hw_spec;
    let input = r#"
## Property "Nonsense"
The quantum banana oscillates retroactively through the flux capacitor.
"#;
    let result = verify_hw_spec(input);
    assert!(result.is_err());
}

// --- 5. Undeclared signals are rejected ---

#[test]
fn filter_rejects_undeclared_signal() {
    use logicaffeine_compile::verification::verify_hw_spec;
    let input = r#"
## Hardware
Let req be a signal of 1 bit.

## Property "Missing"
Always, if req is high, then ack is high.
"#;
    // "ack" not declared in ## Hardware block
    let result = verify_hw_spec(input);
    assert!(result.is_err());
}

// --- 6. Counterexample returned on failure ---

#[test]
fn filter_returns_counterexample_for_invalid_spec() {
    use logicaffeine_compile::verification::{verify_hw_spec, HwVerificationError};
    let input = r#"
## Hardware
Let a be a signal of 1 bit.
Let b be a signal of 1 bit.

## Property "P1"
Always, a is high.

## Property "P2"
Always, b is high.

## Property "P3"
Never do a and b hold simultaneously.
"#;
    // P1 + P2 + P3 contradict: a=1, b=1 violates mutex
    let result = verify_hw_spec(input);
    assert!(result.is_err());
    if let Err(HwVerificationError::Contradiction { counterexample, .. }) = result {
        assert!(!counterexample.assignments.is_empty());
    }
}
```

---

### Phase 7: Knowledge Graph Extraction

**Crate:** `logicaffeine_language`
**Test file:** `phase_hw_knowledge_graph.rs`

**Steps:**
1. Define `HwKnowledgeGraph`, `KgSignal`, `KgState`, `KgEdge` types in `semantics/knowledge_graph.rs`
2. Walk Kripke-lowered `LogicExpr` to extract signal nodes, state nodes, property nodes
3. Extract temporal edges (`Accessible_Temporal`, `Next_Temporal`, `Reachable_Temporal`)
4. Extract implication edges from conditional FOL structures
5. Serialize KG to JSON for LLM consumption

**RED Tests:**

```rust
// phase_hw_knowledge_graph.rs
use logicaffeine_language::semantics::knowledge_graph::{
    extract_knowledge_graph, HwKnowledgeGraph, KgRelation,
};

// --- 1. KG extracts signals from ## Hardware block ---

#[test]
fn kg_extracts_declared_signals() {
    let input = r#"
## Hardware
Let req be a signal of 1 bit.
Let ack be a signal of 1 bit.
Let data be a signal of 8 bits.
"#;
    let kg = extract_knowledge_graph(input).unwrap();
    assert_eq!(kg.signals.len(), 3);
    assert!(kg.signals.iter().any(|s| s.name == "req" && s.width == 1));
    assert!(kg.signals.iter().any(|s| s.name == "data" && s.width == 8));
}

// --- 2. KG extracts properties ---

#[test]
fn kg_extracts_property_nodes() {
    let input = r#"
## Hardware
Let req be a signal of 1 bit.
Let ack be a signal of 1 bit.

## Property "Handshake"
Always, if req is high, then eventually ack is high.

## Property "Mutex"
Never do req and ack hold simultaneously.
"#;
    let kg = extract_knowledge_graph(input).unwrap();
    assert_eq!(kg.properties.len(), 2);
    assert!(kg.properties.iter().any(|p| p.name == "Handshake"));
    assert!(kg.properties.iter().any(|p| p.name == "Mutex"));
}

// --- 3. KG extracts temporal edges ---

#[test]
fn kg_extracts_temporal_dependencies() {
    let input = r#"
## Hardware
Let req be a signal of 1 bit.
Let ack be a signal of 1 bit.

## Property "Response"
Always, if req is high, then next, ack is high.
"#;
    let kg = extract_knowledge_graph(input).unwrap();
    // Should have a temporal edge from req to ack
    let temporal_edges: Vec<_> = kg.edges.iter()
        .filter(|e| matches!(e.relation, KgRelation::Temporal))
        .collect();
    assert!(!temporal_edges.is_empty());
}

// --- 4. KG extracts cross-signal constraints ---

#[test]
fn kg_extracts_constraint_edges() {
    let input = r#"
## Hardware
Let grant_a be a signal of 1 bit.
Let grant_b be a signal of 1 bit.

## Property "Mutex"
Never do grant_a and grant_b hold simultaneously.
"#;
    let kg = extract_knowledge_graph(input).unwrap();
    let constraint_edges: Vec<_> = kg.edges.iter()
        .filter(|e| matches!(e.relation, KgRelation::Constrains))
        .collect();
    assert!(!constraint_edges.is_empty());
}

// --- 5. KG serializes to JSON ---

#[test]
fn kg_serializes_to_json() {
    let input = r#"
## Hardware
Let req be a signal of 1 bit.

## Property "Safety"
Always, req is high.
"#;
    let kg = extract_knowledge_graph(input).unwrap();
    let json = kg.to_json();
    assert!(json.contains("req"));
    assert!(json.contains("Safety"));
    // Valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_object());
}

// --- 6. KG includes signal roles ---

#[test]
fn kg_identifies_clock_signals() {
    let input = r#"
## Hardware
Let clk be a clock signal.
Let data be a signal of 8 bits.
"#;
    let kg = extract_knowledge_graph(input).unwrap();
    let clk = kg.signals.iter().find(|s| s.name == "clk").unwrap();
    assert_eq!(clk.role, SignalRole::Clock);
    let data = kg.signals.iter().find(|s| s.name == "data").unwrap();
    assert_eq!(data.role, SignalRole::Internal);
}
```

---

### Phase 8: SVA Semantic Model & Equivalence Checking

**Crate:** `logicaffeine_verify`
**Test file:** `phase_hw_equivalence.rs`
**Requires:** `verification` feature flag

**Steps:**
1. Define `SvaExpr` AST in `verify/src/sva.rs`
2. Implement `parse_sva()` for a subset of SVA syntax
3. Implement `sva_to_verify_expr()` — translate SVA semantics to VerifyExpr
4. Implement Z3 semantic equivalence: `verify_equivalence(fol: &VerifyExpr, sva: &VerifyExpr) -> VerificationResult`
5. Wire into the pipeline: accept SVA text, check against FOL ground truth

**Leverages existing infrastructure:**
- `VerificationSession` with `declare`, `assume`, `verify` methods (`solver.rs:387-577`)
- `Encoder` translating `VerifyExpr` to Z3 AST (`solver.rs:586-640`)
- `VerificationErrorKind::ContradictoryAssertion` with `CounterExample` (`error.rs:65, 165`)

**RED Tests:**

```rust
// phase_hw_equivalence.rs
#![cfg(feature = "verification")]
use logicaffeine_verify::{
    VerifyExpr, VerifyType, VerificationSession,
    sva::{SvaExpr, parse_sva, sva_to_verify_expr},
};

// --- 1. SVA parsing ---

#[test]
fn sva_parse_simple_always() {
    let sva = parse_sva("@(posedge clk) valid |=> (data_out == data_in)").unwrap();
    assert!(matches!(sva, SvaExpr::Implication { .. }));
}

#[test]
fn sva_parse_delay() {
    let sva = parse_sva("@(posedge clk) $rose(req) |-> ##[1:5] ack").unwrap();
    if let SvaExpr::Implication { consequent, .. } = sva {
        assert!(matches!(*consequent, SvaExpr::Delay { .. }));
    } else {
        panic!("Expected implication");
    }
}

#[test]
fn sva_parse_rose_fell() {
    let sva = parse_sva("@(posedge clk) $rose(req)").unwrap();
    assert!(matches!(sva, SvaExpr::Rose(_)));
    let sva2 = parse_sva("@(posedge clk) $fell(ack)").unwrap();
    assert!(matches!(sva2, SvaExpr::Fell(_)));
}

#[test]
fn sva_parse_negation() {
    let sva = parse_sva("@(posedge clk) !(grant_a && grant_b)").unwrap();
    assert!(matches!(sva, SvaExpr::Not(_)));
}

// --- 2. SVA to VerifyExpr translation ---

#[test]
fn sva_to_verify_simple_eq() {
    let sva = SvaExpr::Eq(
        Box::new(SvaExpr::Signal("x".into())),
        Box::new(SvaExpr::Const(42, 8)),
    );
    let vexpr = sva_to_verify_expr(&sva, 1);
    // Should produce a VerifyExpr comparing x == 42
    assert!(matches!(vexpr, VerifyExpr::Binary { .. }));
}

#[test]
fn sva_to_verify_implication() {
    let sva = SvaExpr::Implication {
        antecedent: Box::new(SvaExpr::Signal("valid".into())),
        consequent: Box::new(SvaExpr::Signal("ready".into())),
        overlapping: true,
    };
    let vexpr = sva_to_verify_expr(&sva, 1);
    assert!(matches!(vexpr, VerifyExpr::Binary { .. }));
}

// --- 3. Semantic equivalence ---

#[test]
fn equivalence_identical_properties_pass() {
    // FOL: G(valid -> data_out == data_in) at bounded depth 1
    // SVA: valid |=> (data_out == data_in)
    // These should be equivalent
    let mut session = VerificationSession::new();
    session.declare("valid_0", VerifyType::Bool);
    session.declare("data_out_0", VerifyType::BitVector(8));
    session.declare("data_in_0", VerifyType::BitVector(8));
    session.declare("valid_1", VerifyType::Bool);
    session.declare("data_out_1", VerifyType::BitVector(8));
    session.declare("data_in_1", VerifyType::BitVector(8));

    let fol = VerifyExpr::implies(
        VerifyExpr::Var("valid_0".into()),
        VerifyExpr::eq(
            VerifyExpr::Var("data_out_1".into()),
            VerifyExpr::Var("data_in_0".into()),
        ),
    );

    let sva_expr = sva_to_verify_expr(
        &parse_sva("@(posedge clk) valid |=> (data_out == data_in)").unwrap(),
        1,
    );

    // Check FOL <-> SVA
    let biconditional = VerifyExpr::and(
        VerifyExpr::implies(fol.clone(), sva_expr.clone()),
        VerifyExpr::implies(sva_expr, fol),
    );
    let result = session.verify(&biconditional);
    assert!(result.is_ok(), "Equivalent properties should pass: {:?}", result.err());
}

#[test]
fn equivalence_mismatched_properties_fail_with_counterexample() {
    // FOL says "req implies ack within 5 cycles"
    // SVA says "req implies ack within 3 cycles" (WRONG)
    // Z3 should find a trace where ack arrives at cycle 4 (FOL satisfied, SVA not)
    let mut session = VerificationSession::new();
    session.declare("req_0", VerifyType::Bool);
    for i in 1..=5 {
        session.declare(&format!("ack_{}", i), VerifyType::Bool);
    }

    // FOL: req_0 -> (ack_1 || ack_2 || ack_3 || ack_4 || ack_5)
    let fol = VerifyExpr::implies(
        VerifyExpr::Var("req_0".into()),
        VerifyExpr::or(
            VerifyExpr::or(
                VerifyExpr::Var("ack_1".into()),
                VerifyExpr::Var("ack_2".into()),
            ),
            VerifyExpr::or(
                VerifyExpr::or(
                    VerifyExpr::Var("ack_3".into()),
                    VerifyExpr::Var("ack_4".into()),
                ),
                VerifyExpr::Var("ack_5".into()),
            ),
        ),
    );

    // SVA (wrong): req_0 -> (ack_1 || ack_2 || ack_3) — missing cycles 4 and 5
    let sva_wrong = VerifyExpr::implies(
        VerifyExpr::Var("req_0".into()),
        VerifyExpr::or(
            VerifyExpr::Var("ack_1".into()),
            VerifyExpr::or(
                VerifyExpr::Var("ack_2".into()),
                VerifyExpr::Var("ack_3".into()),
            ),
        ),
    );

    // Check SVA -> FOL (does SVA imply FOL? yes, if SVA passes, FOL passes)
    // Check FOL -> SVA (does FOL imply SVA? NO — ack at cycle 4 satisfies FOL but not SVA)
    let fol_implies_sva = VerifyExpr::implies(fol.clone(), sva_wrong.clone());
    let result = session.verify(&fol_implies_sva);
    assert!(result.is_err(), "FOL should NOT imply the weaker SVA");
}

#[test]
fn equivalence_mutex_property() {
    // FOL: G(!(grant_a && grant_b))
    // SVA: @(posedge clk) !(grant_a && grant_b)
    // These should be equivalent at each timestep
    let mut session = VerificationSession::new();
    session.declare("grant_a", VerifyType::Bool);
    session.declare("grant_b", VerifyType::Bool);

    let fol = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::Var("grant_a".into()),
        VerifyExpr::Var("grant_b".into()),
    ));

    let sva_expr = sva_to_verify_expr(
        &parse_sva("@(posedge clk) !(grant_a && grant_b)").unwrap(),
        1,
    );

    let biconditional = VerifyExpr::and(
        VerifyExpr::implies(fol.clone(), sva_expr.clone()),
        VerifyExpr::implies(sva_expr, fol),
    );
    let result = session.verify(&biconditional);
    assert!(result.is_ok());
}

// --- 4. End-to-end: English -> FOL -> SVA -> equivalence ---

#[test]
fn end_to_end_spec_to_sva_equivalence() {
    use logicaffeine_language::compile::compile_kripke;
    use logicaffeine_compile::codegen_sva::emit_sva;

    let input = r#"
## Hardware
Let req be a signal of 1 bit.
Let ack be a signal of 1 bit.

## Property "Mutex"
Never do req and ack hold simultaneously.
"#;

    // Step 1: Parse to FOL
    let fol_output = compile_kripke(input).unwrap();

    // Step 2: Generate SVA
    let sva_output = emit_sva(input).unwrap();

    // Step 3: Parse the SVA back
    let sva_parsed = parse_sva(&sva_output).unwrap();

    // Step 4: Check equivalence
    let sva_vexpr = sva_to_verify_expr(&sva_parsed, 1);
    // ... build FOL vexpr from fol_output ...
    // ... verify biconditional ...
    // This test verifies the full pipeline round-trips
}
```

---

## 9. Invariant Compliance

| Invariant | Rule | Preserved? | How |
|-----------|------|------------|-----|
| **Milner** | kernel independent of lexicon | Yes | Hardware vocabulary lives in `lexicon.json` and `logicaffeine_lexicon`. Kernel type theory unchanged. `Sort::Signal` is a lexicon concept. |
| **Liskov** | proof independent of language | Yes | Temporal inference rules added to proof engine with `String`-typed operator fields. `ProofExpr::TemporalBinary` is owned, no arena allocation. Conversion lives in `language/proof_convert.rs`. KG types live in language, not proof. |
| **Tarski** | verify independent of language and compile | Yes | New `VerifyType`/`VerifyExpr` variants are self-contained. SVA semantic model (`sva.rs`) depends only on verify-internal types. BMC encoding uses only verification IR. The bridge `verification.rs` in compile handles mapping. |
| **Lamport** | data independent of IO/time/network | Yes | No changes to `logicaffeine_data`. Hardware verification is pure computation. |

---

## Appendix A: The Timed-Kripke Frame (Formal Definition)

A **Timed-Kripke frame** is a tuple `(W, R, T, tick, V)` where:

- `W` is a set of hardware states (register/wire valuations)
- `R: W -> P(W)` is the next-state relation (combinational + sequential logic)
- `T = N` is discrete time (clock cycles)
- `tick: W -> T` maps states to their clock cycle
- `V: W -> (Prop -> Bool)` evaluates atomic propositions at each state

The accessibility relation:
```
Accessible_Temporal(w, w') iff R(w, w') and tick(w') = tick(w) + 1
```

For synchronous hardware, R is deterministic (each state has exactly one successor given the same inputs). For asynchronous or non-deterministic designs, R may be non-deterministic.

The graded modal force from `ModalVector` enables modeling uncertainty:
- `force = 1.0`: the transition *must* happen (deterministic RTL)
- `force = 0.5`: the transition *may* happen (non-deterministic input)
- `force = 0.3`: the transition is *unlikely* (fault injection)

## Appendix B: Mapping to Existing Modal Verb Infrastructure

The modal parsing at `parser/modal.rs` maps English modals to `ModalVector`. For hardware:

| English | ModalVector | Hardware Meaning |
|---------|-------------|-----------------|
| "must" | {Temporal, 1.0, Root} | Safety invariant (G) |
| "can" | {Temporal, 0.5, Root} | Reachability (EF) |
| "might" | {Temporal, 0.3, Epistemic} | Possible behavior (environment) |
| "should" | {Temporal, 0.6, Root} | Soft constraint (cover, not assert) |
| "shall" | {Temporal, 1.0, Root} | Formal requirement (assert) |
| "will" | {Temporal, 0.8, Root} | Expected behavior (assume) |

When modal verbs are used instead of explicit temporal operators, the `force` threshold serves as a fallback for assertion type (see Section 3e for the full dispatch):
- `force > 0.8`: `assert property` (hard requirement)
- `0.5 < force <= 0.8`: `cover property` (expected but not mandated)
- `force <= 0.5`: `assume property` (environment constraint)

Explicit temporal operators ("always", "eventually") and explicit keywords ("assuming", "cover") take priority over this force-based mapping.

## Appendix C: Research Directions

### C.1 Probabilistic Verification via Graded Modality

The continuous `force` value enables probabilistic model checking. A property with `force = 0.7` maps to "P holds with probability >= 0.7 in the next state." This connects to PCTL (Probabilistic CTL) and the PRISM model checker.

### C.2 Clock Domain Crossing

Multiple clocks require multiple Timed-Kripke frames with inter-frame accessibility:
```
Accessible_CDC(w_fast, w_slow) iff tick_slow(w_slow) = floor(tick_fast(w_fast) / ratio)
```

### C.3 Abstract Interpretation for State Space Reduction

The existing `abstract_interp.rs` (24KB, `Bound`/`Interval`/`AbstractState`) can abstract the hardware state space before BMC, reducing the number of Z3 variables.

### C.4 E-Graph Optimization of SVA

The kernel's `cc.rs` E-graph (union-find + hash-consing + congruence propagation) could optimize generated SVA by finding equivalent simpler formulations via equality saturation.
