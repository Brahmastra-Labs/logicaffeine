# LogicAffeine Quantum Backend: Cirq Implementation Plan v2

**The world's first readable quantum language.**
English specifications compiled to formally verified quantum circuits.

---

## Preamble: What This Plan Delivers

17 sprints. 314 tests across 13 test files. 11 source files mirroring the proven SVA backend architecture. Every type, function, and test grounded in the actual codebase — no aspirational references to types that don't exist yet.

This plan addresses every deficiency of v1: the missing AST prerequisites, the sparse test coverage, the hand-waved Kripke mapping, the exponential blowup, the absent optimization/vacuity/serialization/kernel layers, and the single reference circuit. Version 2 is what God would write.

---

## Architecture

The Cirq backend mirrors the SVA backend one-to-one. Every file in `codegen_sva/` has a quantum counterpart:

### Source Files

```
crates/logicaffeine_compile/src/codegen_cirq/
  mod.rs                — module exports
  cirq_model.rs         — domain IR: Qubit, CirqGate (38 variants), CirqOperation,
                          CirqMoment, CirqCircuit; parse, display, structural equiv
  kripke_to_cirq.rs     — KripkeToCirqTranslator: modal force -> gates, worlds -> qubits
  cirq_emitter.rs       — Python/Cirq code generation
  openqasm_emitter.rs   — OpenQASM 3.0 code generation
  cirq_optimize.rs      — algebraic identity laws (HH=I, XX=I, ...) + moment packing
  cirq_dead_gate.rs     — dead gate analysis (vacuity analog)
  quantum_pipeline.rs   — public API: compile, emit, verify
  cirq_to_verify.rs     — symbolic matrices, tensor products, Z3 amplitude encoding
  cirq_to_kernel.rs     — CirqCircuit -> kernel Term (Curry-Howard bridge)
  cirq_serialize.rs     — JSON serialization/deserialization
```

### SVA-to-Cirq File Correspondence

| SVA File | Cirq Counterpart | Role |
|:---|:---|:---|
| `sva_model.rs` (2635 lines, 78 variants) | `cirq_model.rs` | Domain IR with parse/display/structural-equiv |
| `sva_to_verify.rs` (1869 lines) | `cirq_to_verify.rs` | Translate domain IR to Z3-ready verification IR |
| `fol_to_verify.rs` (~800 lines) | `kripke_to_cirq.rs` | Translate Kripke FOL to domain IR |
| `hw_pipeline.rs` (349 lines) | `quantum_pipeline.rs` | Public API orchestrating the full pipeline |
| `sva_vacuity.rs` (186 lines) | `cirq_dead_gate.rs` | Dead gate / vacuity analysis |
| `verify_to_kernel.rs` (297 lines) | `cirq_to_kernel.rs` | Domain IR to Curry-Howard kernel terms |
| *(none)* | `cirq_optimize.rs` | Circuit optimization with algebraic identity laws |
| *(none)* | `openqasm_emitter.rs` | OpenQASM 3.0 emission |
| *(none)* | `cirq_serialize.rs` | Circuit serialization |

### Test Files

```
crates/logicaffeine_tests/tests/
  phase_cirq_ast.rs           — Sprint 0: AST/parser/lexicon extensions
  phase_cirq_model.rs         — Sprints 1-2: IR types, display, structural equiv, roundtrip
  phase_cirq_kripke_map.rs    — Sprints 3-4: modal force -> gate, full Kripke translation
  phase_cirq_emit.rs          — Sprints 5-6: Python/Cirq code generation
  phase_cirq_openqasm.rs      — Sprint 7: OpenQASM 3.0 emission
  phase_cirq_optimize.rs      — Sprint 8: identity laws, moment packing, combined
  phase_cirq_dead_gate.rs     — Sprint 9: dead gate detection and removal
  phase_cirq_pipeline.rs      — Sprint 10: pipeline API end-to-end
  phase_cirq_verify.rs        — Sprints 11-12: symbolic matrices, tensor products
  phase_cirq_z3_verify.rs     — Sprints 13-14: Z3 amplitude checking, equivalence
  phase_cirq_kernel.rs        — Sprint 15: Curry-Howard kernel bridge
  phase_cirq_reference.rs     — Sprint 16: reference circuits (EPR, GHZ, QFT, ...)
  phase_cirq_serialize.rs     — Sprint 17: serialization roundtrip
```

### Existing Infrastructure We Reuse

| Component | Location | What We Reuse |
|:---|:---|:---|
| Kripke lowering | `logicaffeine_language/src/semantics/kripke.rs` | `KripkeContext`, `apply_kripke_lowering()`, world variables, temporal operators |
| compile_kripke_with | `logicaffeine_language/src/compile.rs` | Entry point: spec -> AST with Kripke lowering |
| ModalVector | `logicaffeine_language/src/ast/logic.rs` | `force: f32`, `domain: ModalDomain`, `flavor: ModalFlavor` |
| TemporalOperator | `logicaffeine_language/src/ast/logic.rs` | `Past`, `Future`, `Always`, `Eventually`, `Next` |
| VerifyExpr IR | `logicaffeine_verify/src/ir.rs` | `Apply()` for complex amplitudes, `Binary()` for constraints |
| Z3 equivalence | `logicaffeine_verify/src/equivalence.rs` | `check_equivalence()` pattern for amplitude checking |
| Kernel types | `logicaffeine_kernel/src/prelude.rs:2269+` | `Bit(B0/B1)`, `BVec`, `Circuit(MkCircuit)`, gate operations |
| SVA pipeline pattern | `codegen_sva/hw_pipeline.rs` | `compile_hw_spec()` pattern for `compile_quantum_spec()` |
| SvaTranslator pattern | `codegen_sva/sva_to_verify.rs` | `translate()`, `translate_property()` method patterns |

---

## The Kripke-Quantum Isomorphism

### The Classical Kripke Frame (What Exists Now)

A Kripke frame is $\mathcal{F} = \langle W, R \rangle$ where $W$ is a set of possible worlds and $R \subseteq W \times W$ is the accessibility relation. In the existing codebase:

- `KripkeContext.world_counter` generates fresh world variables $w_0, w_1, w_2, \ldots$
- `KripkeContext.current_world` tracks the evaluation world
- `lower_modal()` at `kripke.rs:405` splits on force:
  - `force > 0.5` (necessity/Box): $\forall w'(Accessible(w, w') \to P(w'))$
  - `force \leq 0.5` (possibility/Diamond): $\exists w'(Accessible(w, w') \wedge P(w'))$
- `lower_expr()` at `kripke.rs:88` handles temporal operators:
  - `Always`: $\forall w'(Accessible\_Temporal(w, w') \to \varphi(w'))$
  - `Eventually`: $\exists w'(Reachable\_Temporal(w, w') \wedge \varphi(w'))$
  - `Next`: tick clock, $\forall w'(Next\_Temporal(w, w') \to \varphi(w'))$

### The Quantum Kripke Frame (What We Build)

$\mathcal{QKF} = \langle \mathcal{H}, \mathcal{U}, \Phi, \mathcal{T} \rangle$ where:

**State Space ($\mathcal{H}$):** The set of possible worlds $W$ maps to a complex Hilbert space $\mathcal{H} \cong \mathbb{C}^{2^n}$ for an $n$-qubit system. The computational basis states $\{|x\rangle : x \in \{0,1\}^n\}$ are the classical worlds. A generic world (state) is a superposition $|\psi\rangle = \sum_x \alpha_x |x\rangle$ where $\sum |\alpha_x|^2 = 1$.

**Accessibility ($\mathcal{U}$):** The relation $R$ is defined by unitary operators $\mathcal{U} \subset U(2^n)$. State $|\psi_j\rangle$ is accessible from $|\psi_i\rangle$ iff there exists $U \in \mathcal{U}$ such that $|\psi_j\rangle = U|\psi_i\rangle$. Unitarity guarantees reversibility.

**Modal Force ($\Phi$):** Necessity/possibility is quantized into probability amplitudes. A proposition $P$ holds in state $|\psi\rangle$ with probabilistic force $\text{Pr}(P) = \langle\psi|P_M|\psi\rangle$ where $P_M$ is the projection operator.

**Temporal Dynamics ($\mathcal{T}$):** LTL `Next` maps to applying a unitary within a discrete time slice (Cirq `Moment`). `Always` maps to the property holding across all moments. `Eventually` maps to measurement.

### The Force-to-Gate Mapping (Precise)

Given `ModalVector { domain: Temporal, force: f, flavor: _ }`:

| Force ($f$) | Quantum Interpretation | Gate | Matrix | Verification |
|:---|:---|:---|:---|:---|
| $0.0$ | Impossibility (no transition) | $I$ | $\begin{pmatrix}1&0\\0&1\end{pmatrix}$ | $\text{Pr}(\|1\rangle) = 0$ |
| $0.5$ | Equal possibility (superposition) | $H$ | $\frac{1}{\sqrt{2}}\begin{pmatrix}1&1\\1&-1\end{pmatrix}$ | $\text{Pr}(\|0\rangle) = \text{Pr}(\|1\rangle) = 0.5$ |
| $1.0$ | Necessity (deterministic flip) | $X$ | $\begin{pmatrix}0&1\\1&0\end{pmatrix}$ | $\text{Pr}(\|1\rangle) = 1$ |
| $0 < f < 1$ | Graded modality | $R_y(2\arcsin\sqrt{f})$ | rotation | $\text{Pr}(\|1\rangle) = f$ |

The $R_y$ formulation is exact: $R_y(\theta)|0\rangle = \cos(\theta/2)|0\rangle + \sin(\theta/2)|1\rangle$, so $\text{Pr}(|1\rangle) = \sin^2(\theta/2) = f$ when $\theta = 2\arcsin(\sqrt{f})$.

**Entanglement mapping:** The pattern `Always, q1 iff q2` — an invariant biconditional — maps to CNOT (entanglement generation). The biconditional ensures the two qubits are always correlated.

**Temporal mapping:** `Next` creates a moment boundary. `Measure` collapses superposition.

### Scalability Boundary Statement

**$n \leq 5$ qubits:** Full symbolic matrix verification. $2^5 = 32$, so $32 \times 32$ matrices with 1024 Z3 Real entries. Z3 handles this in under a second.

**$n \leq 10$ qubits:** Sparse matrix representation with lazy expansion. Most gates are tensor products of $I$ with a small unitary. Only materialize non-identity blocks. Z3 handles $2^{10} = 1024$ Real variables in seconds.

**$n > 10$ qubits:** Full symbolic verification explodes exponentially. The pipeline falls back to:
1. Per-gate algebraic identity checking (always $O(1)$ per gate pair)
2. Dead gate analysis (linear scan)
3. Stabilizer formalism for Clifford-only circuits (polynomial) — future extension
4. ZX-calculus rewrite rules — future extension

For $n > 10$, the pipeline will emit Python/QASM without modification (synthesis always works) but return `CirqError::VerificationError("Circuit exceeds symbolic verification limit")` if full Z3 verification is requested. This is honest. No hand-waving.

---

## Phase 0: AST, Parser, and Lexicon Extensions

**This phase does not exist in v1. It is the most critical prerequisite.**

The `LogicExpr` enum at `crates/logicaffeine_language/src/ast/logic.rs` has no quantum variants. `ModalVector.force` exists but encodes possibility/necessity, not qubit rotations directly. We must add quantum-specific AST nodes.

### Sprint 0: AST/Parser/Lexicon Extensions

**Files modified:**
- `crates/logicaffeine_language/src/ast/logic.rs` — new `LogicExpr` variants
- `crates/logicaffeine_language/src/semantics/kripke.rs` — new match arms in `lower_expr()`
- `crates/logicaffeine_language/src/parser/` — quantum pattern recognition

**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_ast.rs`

#### AST Extensions

Add to `LogicExpr` enum in `ast/logic.rs`:

```rust
/// Quantum variable declaration: `Let q : Qubit.`
QuantumDecl {
    variable: Symbol,
},

/// Quantum gate application: `Apply H to q1.`
QuantumGate {
    gate: QuantumGateKind,
    qubits: &'a [Symbol],
    parameter: Option<f64>,
},

/// Quantum measurement: `Measure(q1, q2).`
QuantumMeasure {
    qubits: &'a [Symbol],
    key: Option<Symbol>,
},
```

New supporting enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantumGateKind {
    I, H, X, Y, Z,
    S, T, Sdg, Tdg,
    Rx, Ry, Rz,
    CNOT, CZ, SWAP,
    Toffoli, Fredkin,
    Superposition,   // maps to H
    Entanglement,    // maps to CNOT
}
```

#### Kripke Lowering Extension

Add match arms in `lower_expr()` at `kripke.rs:88`:

```rust
LogicExpr::QuantumDecl { variable } => {
    // Pass through — circuit builder consumes these
    expr
}
LogicExpr::QuantumGate { gate, qubits, parameter } => {
    // Pass through with world annotation
    expr
}
LogicExpr::QuantumMeasure { qubits, key } => {
    // Measurement annotated with current world
    expr
}
```

#### Parser Extensions

Recognize three patterns:
1. `Let q1 : Qubit.` -> `QuantumDecl { variable: q1 }`
2. `Apply Superposition to q1 with Force: 0.5.` -> `QuantumGate { gate: Superposition, qubits: [q1], parameter: Some(0.5) }`
3. `Measure(q1, q2).` -> `QuantumMeasure { qubits: [q1, q2], key: None }`
4. `Next, Apply Entanglement(q1, q2).` -> `Temporal { operator: Next, body: QuantumGate { ... } }`

#### RED Tests (20)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 0: AST / PARSER / LEXICON EXTENSIONS
// ═══════════════════════════════════════════════════════════════

// SECTION 1: QUANTUM DECLARATIONS (5 tests)

#[test]
fn parse_let_qubit_declaration()
// "Let q1 : Qubit." -> QuantumDecl { variable: q1 }

#[test]
fn parse_multiple_qubit_declarations()
// Two "Let q : Qubit." statements both produce QuantumDecl nodes

#[test]
fn qubit_declaration_stores_symbol()
// variable symbol matches "q1"

#[test]
fn qubit_declaration_in_discourse()
// Works in multi-sentence context with other declarations

#[test]
fn qubit_invalid_sort_rejected()
// "Let q1 : Blarg." does NOT produce QuantumDecl

// SECTION 2: QUANTUM GATE APPLICATION (8 tests)

#[test]
fn parse_apply_superposition()
// "Apply Superposition to q1 with Force: 0.5."

#[test]
fn parse_apply_entanglement()
// "Apply Entanglement(q1, q2)."

#[test]
fn parse_apply_hadamard_direct()
// "Apply H to q1."

#[test]
fn parse_apply_with_force_parameter()
// Force: 0.3 -> parameter: Some(0.3)

#[test]
fn parse_apply_next_temporal_wrapping()
// "Next, Apply ..." -> Temporal { Next, QuantumGate { ... } }

#[test]
fn gate_kind_superposition_resolves()
// QuantumGateKind::Superposition

#[test]
fn gate_kind_entanglement_resolves()
// QuantumGateKind::Entanglement

#[test]
fn gate_qubits_preserve_order()
// [q1, q2] order matches declaration order

// SECTION 3: QUANTUM MEASUREMENT (4 tests)

#[test]
fn parse_measure_single_qubit()
// "Measure(q1)."

#[test]
fn parse_measure_multiple_qubits()
// "Measure(q1, q2)."

#[test]
fn parse_measure_with_key()
// "Measure(q1, q2) as result."

#[test]
fn measurement_qubit_symbols_match()
// qubits: [q1, q2] symbol resolution correct

// SECTION 4: EPR PAIR END-TO-END PARSE (3 tests)

#[test]
fn parse_epr_pair_full_spec()
// Full 5-line EPR spec parses without error

#[test]
fn epr_pair_has_two_qubit_decls()
// Exactly 2 QuantumDecl nodes in AST

#[test]
fn epr_pair_has_temporal_structure()
// Next operator wraps entanglement gate
```

---

## Phase I: Foundational Topology (IR Construction)

### Sprint 1: CirqModel IR Construction

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_model.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_model.rs`

#### Type Definitions

```rust
/// Qubit addressing in a quantum circuit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Qubit {
    /// Linear qubit: q(i) — index-based addressing.
    Line(usize),
    /// Grid qubit: q(row, col) — 2D addressing for surface codes.
    Grid(usize, usize),
    /// Named qubit: preserves the LOGOS variable name.
    Named(String),
}

/// Quantum gate type with full parameter support.
/// 38 variants covering the standard universal gate set plus
/// Google Sycamore native gates.
#[derive(Debug, Clone, PartialEq)]
pub enum CirqGate {
    // ── Single-Qubit Gates (15) ──
    I,                          // Identity
    H,                          // Hadamard
    X, Y, Z,                   // Pauli gates
    S, T,                       // Phase gates
    Sdg, Tdg,                   // Phase gate adjoints
    Rx(f64), Ry(f64), Rz(f64), // Rotation gates
    Phase(f64),                 // P(phi) = diag(1, exp(i*phi))
    SqrtX,                      // sqrt(X)

    // ── Two-Qubit Gates (10) ──
    CNOT,                       // Controlled-NOT
    CZ,                         // Controlled-Z
    SWAP,                       // Qubit swap
    ISWAP,                      // iSWAP
    SqrtISWAP,                  // sqrt(iSWAP) — Sycamore native
    CPhase(f64),                // Controlled-Phase
    XX(f64),                    // XX interaction
    YY(f64),                    // YY interaction
    ZZ(f64),                    // ZZ interaction
    CRx(f64),                   // Controlled-Rx

    // ── Three-Qubit Gates (3) ──
    Toffoli,                    // Doubly-controlled X
    Fredkin,                    // Controlled SWAP
    CCZ,                        // Doubly-controlled Z

    // ── Measurement (1) ──
    Measure { key: String },    // Projective measurement

    // ── Barrier (1) ──
    Barrier,                    // No-op fence preventing optimization across boundary

    // ── Custom (1) ──
    Custom { name: String, params: Vec<f64> },
}

/// A single gate application to specific qubits.
#[derive(Debug, Clone, PartialEq)]
pub struct CirqOperation {
    pub gate: CirqGate,
    pub qubits: Vec<Qubit>,
}

/// A time slice (moment) containing non-overlapping operations.
#[derive(Debug, Clone, PartialEq)]
pub struct CirqMoment {
    pub operations: Vec<CirqOperation>,
}

/// A complete quantum circuit.
#[derive(Debug, Clone, PartialEq)]
pub struct CirqCircuit {
    pub qubits: Vec<Qubit>,
    pub moments: Vec<CirqMoment>,
    pub metadata: CircuitMetadata,
}

/// Circuit metadata for provenance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CircuitMetadata {
    pub name: Option<String>,
    pub source_spec: Option<String>,
    pub qubit_count: usize,
}

/// Error type for circuit construction and validation.
#[derive(Debug, Clone, PartialEq)]
pub struct CirqError {
    pub kind: CirqErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CirqErrorKind {
    QubitConflict,
    InvalidQubitCount,
    ParseError,
    TranslationError,
    EmitError,
    VerificationError,
    OptimizationError,
    SerializationError,
}
```

#### Functions

```rust
pub fn gate_qubit_count(gate: &CirqGate) -> usize;
pub fn gate_display_name(gate: &CirqGate) -> String;
pub fn validate_operation(op: &CirqOperation) -> Result<(), CirqError>;
pub fn validate_moment(moment: &CirqMoment) -> Result<(), CirqError>;
pub fn validate_circuit(circuit: &CirqCircuit) -> Result<(), CirqError>;
pub fn cirq_circuit_to_string(circuit: &CirqCircuit) -> String;
pub fn parse_cirq_circuit(input: &str) -> Result<CirqCircuit, CirqError>;
pub fn cirq_circuits_structurally_equivalent(a: &CirqCircuit, b: &CirqCircuit) -> bool;
pub fn circuit_depth(circuit: &CirqCircuit) -> usize;
pub fn circuit_gate_count(circuit: &CirqCircuit) -> usize;
pub fn circuit_qubit_set(circuit: &CirqCircuit) -> HashSet<Qubit>;
pub fn circuit_append_moment(circuit: &mut CirqCircuit, moment: CirqMoment) -> Result<(), CirqError>;
pub fn circuit_compose(a: &CirqCircuit, b: &CirqCircuit) -> Result<CirqCircuit, CirqError>;
```

#### RED Tests — Sprint 1 (24)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 1: IR CONSTRUCTION
// ═══════════════════════════════════════════════════════════════

// SECTION 1: QUBIT TYPES (4 tests)

#[test]
fn cirq_line_qubit_creation()
// Qubit::Line(0) -> index 0, display "q(0)"

#[test]
fn cirq_grid_qubit_creation()
// Qubit::Grid(2, 3) -> row 2, col 3

#[test]
fn cirq_named_qubit_creation()
// Qubit::Named("q1") -> preserves name

#[test]
fn cirq_qubit_equality()
// Line(0) == Line(0), Line(0) != Line(1), Line(0) != Grid(0,0)

// SECTION 2: GATE PROPERTIES (8 tests)

#[test]
fn cirq_gate_h_is_single_qubit()
// gate_qubit_count(H) == 1

#[test]
fn cirq_gate_cnot_is_two_qubit()
// gate_qubit_count(CNOT) == 2

#[test]
fn cirq_gate_toffoli_is_three_qubit()
// gate_qubit_count(Toffoli) == 3

#[test]
fn cirq_gate_rx_carries_angle()
// Rx(PI/2) preserves the angle value

#[test]
fn cirq_gate_measure_has_key()
// Measure { key: "result" } stores the measurement key

#[test]
fn cirq_gate_display_name_h()
// gate_display_name(H) == "H"

#[test]
fn cirq_gate_display_name_rx()
// gate_display_name(Rx(1.5708)) == "Rx(1.5708)"

#[test]
fn cirq_gate_display_name_measure()
// gate_display_name(Measure{key:"r"}) == "M(r)"

// SECTION 3: OPERATION VALIDATION (4 tests)

#[test]
fn cirq_operation_binds_gate_to_qubits()
// CirqOperation { gate: H, qubits: [Line(0)] } stores both correctly

#[test]
fn cirq_operation_wrong_qubit_count_errors()
// H with 2 qubits -> CirqError::InvalidQubitCount

#[test]
fn cirq_operation_cnot_correct_qubits()
// CNOT with [Line(0), Line(1)] -> valid

#[test]
fn cirq_operation_measure_any_qubit_count()
// Measure can have 1, 2, or more qubits

// SECTION 4: MOMENT CONSTRUCTION (4 tests)

#[test]
fn cirq_moment_allows_disjoint_qubits()
// H(q0) and X(q1) in same moment -> valid

#[test]
fn cirq_moment_rejects_overlapping_qubits()
// H(q0) and X(q0) in same moment -> CirqError::QubitConflict

#[test]
fn cirq_moment_empty_is_valid()
// Moment with no operations -> valid

#[test]
fn cirq_moment_three_parallel_gates()
// H(q0), X(q1), Z(q2) in same moment -> valid (all disjoint)

// SECTION 5: CIRCUIT CONSTRUCTION (4 tests)

#[test]
fn cirq_circuit_appends_moments_in_order()
// [M0, M1] -> moments.len() == 2, ordering preserved

#[test]
fn cirq_circuit_qubit_count()
// Circuit using q0..q3 -> qubit_count == 4

#[test]
fn cirq_circuit_empty_valid()
// Empty circuit -> valid

#[test]
fn cirq_circuit_compose_two_circuits()
// A + B -> combined moments in sequence
```

---

### Sprint 2: Display, Structural Equivalence, Metrics, and Roundtrip

**Test file:** `phase_cirq_model.rs` (continued)

#### RED Tests — Sprint 2 (24)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 2: DISPLAY, STRUCTURAL EQUIVALENCE, METRICS, ROUNDTRIP
// ═══════════════════════════════════════════════════════════════

// SECTION 6: CIRCUIT DISPLAY (6 tests)

#[test]
fn cirq_circuit_to_string_single_h()
// H(q0) -> formatted circuit diagram

#[test]
fn cirq_circuit_to_string_cnot()
// CNOT(q0,q1) -> shows control/target

#[test]
fn cirq_circuit_to_string_epr_pair()
// H-CNOT-Measure -> multi-line diagram

#[test]
fn cirq_circuit_to_string_empty()
// Empty circuit -> "(empty circuit)"

#[test]
fn cirq_circuit_to_string_rotation()
// Rx(1.57)(q0) -> shows angle

#[test]
fn cirq_circuit_to_string_parallel()
// H(q0), X(q1) in same moment -> side by side

// SECTION 7: STRUCTURAL EQUIVALENCE (10 tests)

#[test]
fn cirq_equiv_identical_circuits()
// Two identical H-CNOT circuits -> true

#[test]
fn cirq_equiv_different_gates()
// H(q0) vs X(q0) -> false

#[test]
fn cirq_equiv_different_qubit_order()
// CNOT(q0,q1) vs CNOT(q1,q0) -> false (control/target swapped)

#[test]
fn cirq_equiv_different_moment_count()
// 2 moments vs 3 moments -> false

#[test]
fn cirq_equiv_different_qubit_type()
// Line(0) vs Grid(0,0) -> false

#[test]
fn cirq_equiv_empty_circuits()
// empty == empty -> true

#[test]
fn cirq_equiv_rotation_same_angle()
// Rx(PI) == Rx(PI) -> true

#[test]
fn cirq_equiv_rotation_different_angle()
// Rx(PI) != Rx(PI/2) -> false

#[test]
fn cirq_equiv_measure_same_key()
// Measure("a") == Measure("a") -> true

#[test]
fn cirq_equiv_measure_different_key()
// Measure("a") != Measure("b") -> false

// SECTION 8: CIRCUIT METRICS (4 tests)

#[test]
fn cirq_circuit_depth_three_moments()
// 3 moments -> depth() == 3

#[test]
fn cirq_circuit_gate_count_four_ops()
// 4 total operations -> gate_count() == 4

#[test]
fn cirq_circuit_qubit_set_correct()
// Returns exact set of used qubits

#[test]
fn cirq_circuit_depth_zero_empty()
// Empty circuit -> depth() == 0

// SECTION 9: ROUNDTRIP (4 tests)

#[test]
fn cirq_roundtrip_single_gate()
// construct -> to_string -> parse -> equiv == true

#[test]
fn cirq_roundtrip_epr_pair()
// EPR circuit -> display -> parse -> equiv

#[test]
fn cirq_roundtrip_rotation_preserves_angle()
// Rx(PI/4) angle survives roundtrip

#[test]
fn cirq_roundtrip_measure_preserves_key()
// Measure key survives roundtrip
```

---

### Sprint 3: Kripke-to-Cirq Modal Force Mapping

**File:** `crates/logicaffeine_compile/src/codegen_cirq/kripke_to_cirq.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_kripke_map.rs`

#### Core Translator Struct

```rust
pub struct KripkeToCirqTranslator<'a> {
    interner: &'a Interner,
    world_to_qubit: HashMap<Symbol, usize>,
    current_moment_ops: Vec<CirqOperation>,
    moments: Vec<CirqMoment>,
    next_qubit: usize,
    declared_qubits: Vec<Symbol>,
}

impl<'a> KripkeToCirqTranslator<'a> {
    pub fn new(interner: &'a Interner) -> Self;
    pub fn translate(&mut self, expr: &LogicExpr<'_>) -> Result<CirqCircuit, CirqError>;
    fn translate_expr(&mut self, expr: &LogicExpr<'_>) -> Result<(), CirqError>;
    fn map_force_to_gate(&self, force: f32) -> CirqGate;
    fn allocate_qubit(&mut self, symbol: Symbol) -> usize;
    fn flush_moment(&mut self);
    fn resolve_qubit(&self, symbol: Symbol) -> Option<usize>;
}
```

#### The Force Mapping Function

```rust
pub fn map_force_to_gate(force: f32) -> CirqGate {
    const EPSILON: f32 = 1e-6;
    if (force - 0.0).abs() < EPSILON {
        CirqGate::I
    } else if (force - 0.5).abs() < EPSILON {
        CirqGate::H
    } else if (force - 1.0).abs() < EPSILON {
        CirqGate::X
    } else {
        // Graded modality: Ry(2 * arcsin(sqrt(f)))
        // This is exact: Pr(|1>) = sin^2(theta/2) = f
        let theta = 2.0 * (force as f64).sqrt().asin();
        CirqGate::Ry(theta)
    }
}
```

#### RED Tests — Sprint 3 (18)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 3: THE KRIPKE-QUANTUM ISOMORPHISM
// ═══════════════════════════════════════════════════════════════

// SECTION 1: DIRECT FORCE MAPPING (7 tests)

#[test]
fn force_zero_maps_to_identity()
// force: 0.0 -> CirqGate::I (impossibility)

#[test]
fn force_half_maps_to_hadamard()
// force: 0.5 -> CirqGate::H (equal superposition)

#[test]
fn force_one_maps_to_pauli_x()
// force: 1.0 -> CirqGate::X (deterministic transition)

#[test]
fn force_quarter_maps_to_ry_rotation()
// force: 0.25 -> CirqGate::Ry(PI/3) where Pr(|1>) = 0.25

#[test]
fn force_three_quarters_maps_to_ry()
// force: 0.75 -> CirqGate::Ry(2*arcsin(sqrt(0.75)))

#[test]
fn force_near_zero_maps_to_identity()
// force: 0.000001 -> CirqGate::I (epsilon tolerance)

#[test]
fn force_near_half_maps_to_hadamard()
// force: 0.499999 -> CirqGate::H (epsilon tolerance)

// SECTION 2: KRIPKE PATTERN RECOGNITION (7 tests)

#[test]
fn modal_box_temporal_maps_gate()
// Box with Temporal domain -> gate on qubit

#[test]
fn modal_diamond_temporal_maps_gate()
// Diamond with Temporal domain -> gate on qubit

#[test]
fn always_iff_maps_to_cnot()
// "Always, q1 iff q2" pattern -> CirqGate::CNOT (entanglement)

#[test]
fn next_operator_creates_new_moment()
// TemporalOperator::Next -> flush_moment(), new CirqMoment boundary

#[test]
fn eventually_maps_to_measurement()
// TemporalOperator::Eventually -> CirqGate::Measure

#[test]
fn world_variable_to_qubit_allocation()
// w0 -> qubit 0, w1 -> qubit 1

#[test]
fn world_variables_allocated_in_order()
// Encounter order determines qubit index assignment

// SECTION 3: QUBIT DECLARATION (4 tests)

#[test]
fn quantum_decl_registers_qubit()
// QuantumDecl { q1 } -> allocate_qubit(q1) returns 0

#[test]
fn quantum_decl_two_qubits_distinct()
// q1 and q2 get indices 0 and 1

#[test]
fn quantum_decl_duplicate_is_error()
// Declaring same qubit twice -> CirqError::TranslationError

#[test]
fn quantum_gate_uses_declared_qubit()
// QuantumGate { H, [q1] } -> operation on correct qubit index
```

---

### Sprint 4: Full Kripke-to-CirqModel Translation

**Test file:** `phase_cirq_kripke_map.rs` (continued)

#### RED Tests — Sprint 4 (17)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 4: FULL KRIPKE -> CIRQ TRANSLATION
// ═══════════════════════════════════════════════════════════════

// SECTION 4: SINGLE-QUBIT TRANSLATION (5 tests)

#[test]
fn translate_single_qubit_superposition()
// QuantumGate { Superposition, [q1], force: 0.5 } -> circuit has H(q0)

#[test]
fn translate_single_qubit_pauli_x()
// QuantumGate { X, [q1] } -> circuit has X(q0)

#[test]
fn translate_graded_rotation()
// force: 0.25 -> circuit has Ry gate on q0

#[test]
fn translate_qubit_index_matches_decl_order()
// First declared qubit -> index 0

#[test]
fn translate_unknown_qubit_is_error()
// Reference to undeclared qubit -> CirqError::TranslationError

// SECTION 5: TWO-QUBIT TRANSLATION (5 tests)

#[test]
fn translate_entanglement_to_cnot()
// QuantumGate { Entanglement, [q1, q2] } -> CNOT(q0, q1)

#[test]
fn translate_two_qubit_preserves_order()
// [q1, q2] -> CNOT(0, 1), [q2, q1] -> CNOT(1, 0)

#[test]
fn translate_cz_gate()
// QuantumGate { CZ, [q1, q2] } -> CZ(q0, q1)

#[test]
fn translate_swap_gate()
// QuantumGate { SWAP, [q1, q2] } -> SWAP(q0, q1)

#[test]
fn translate_entanglement_requires_two_qubits()
// Entanglement with 1 qubit -> CirqError

// SECTION 6: TEMPORAL STRUCTURE (4 tests)

#[test]
fn translate_next_creates_moment_boundary()
// Next wrapping -> new CirqMoment

#[test]
fn translate_sequential_gates_same_moment()
// Adjacent gates without Next -> packed into same moment if qubits disjoint

#[test]
fn translate_epr_pair_three_moments()
// Full EPR: [H(q0)], [CNOT(q0,q1)], [Measure(q0,q1)]

#[test]
fn translate_multiple_next_multiple_moments()
// Next-Next -> 2 moment boundaries

// SECTION 7: MEASUREMENT TRANSLATION (3 tests)

#[test]
fn translate_measure_single_qubit()
// Measure(q1) -> Measure { key: "q1" }(q0)

#[test]
fn translate_measure_multiple_qubits()
// Measure(q1, q2) -> Measure { key: "result" }(q0, q1)

#[test]
fn translate_measure_key_from_variable_name()
// Key derives from qubit variable name
```

---

## Phase II: Synthesis Engine (Emit & Optimize)

### Sprint 5-6: Python/Cirq Code Generation

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_emitter.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_emit.rs`

#### Functions

```rust
pub fn emit_cirq_python(circuit: &CirqCircuit) -> String;
fn emit_imports() -> String;
fn emit_qubit_declarations(circuit: &CirqCircuit) -> String;
fn emit_circuit_body(circuit: &CirqCircuit) -> String;
fn emit_gate_python(gate: &CirqGate, qubits: &[Qubit]) -> String;
fn emit_simulation_footer() -> String;
```

#### RED Tests — Sprints 5-6 (30)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINTS 5-6: PYTHON CODE GENERATION
// ═══════════════════════════════════════════════════════════════

// SECTION 1: IMPORT AND SETUP (4 tests)

#[test]
fn emit_imports_cirq()
// Output starts with "import cirq"

#[test]
fn emit_declares_line_qubits()
// 2-qubit circuit -> "cirq.LineQubit(0)" and "cirq.LineQubit(1)"

#[test]
fn emit_declares_grid_qubits()
// Grid qubit -> "cirq.GridQubit(0, 1)"

#[test]
fn emit_creates_circuit_object()
// Output contains "circuit = cirq.Circuit()"

// SECTION 2: INDIVIDUAL GATE EMISSION (10 tests)

#[test]
fn emit_hadamard_gate()
// H(q0) -> "cirq.H(q[0])"

#[test]
fn emit_pauli_x_gate()
// X(q0) -> "cirq.X(q[0])"

#[test]
fn emit_pauli_y_gate()
// Y(q0) -> "cirq.Y(q[0])"

#[test]
fn emit_pauli_z_gate()
// Z(q0) -> "cirq.Z(q[0])"

#[test]
fn emit_s_gate()
// S(q0) -> "cirq.S(q[0])"

#[test]
fn emit_t_gate()
// T(q0) -> "cirq.T(q[0])"

#[test]
fn emit_cnot_gate()
// CNOT(q0,q1) -> "cirq.CNOT(q[0], q[1])"

#[test]
fn emit_cz_gate()
// CZ(q0,q1) -> "cirq.CZ(q[0], q[1])"

#[test]
fn emit_rx_gate_with_angle()
// Rx(1.5708) -> "cirq.rx(1.5708)(q[0])"

#[test]
fn emit_measurement()
// Measure{key}(q0,q1) -> "cirq.measure(q[0], q[1], key='result')"

// SECTION 3: STRUCTURE AND FORMATTING (6 tests)

#[test]
fn emit_moment_separation()
// 2 moments -> two circuit.append() calls

#[test]
fn emit_simulator_invocation()
// "simulator = cirq.Simulator()" present

#[test]
fn emit_print_final_state()
// "print(result.final_state_vector)" present

#[test]
fn emit_empty_circuit()
// Empty circuit -> valid Python with import/create, no append

#[test]
fn emit_toffoli_gate()
// Toffoli(q0,q1,q2) -> "cirq.TOFFOLI(q[0], q[1], q[2])"

#[test]
fn emit_swap_gate()
// SWAP(q0,q1) -> "cirq.SWAP(q[0], q[1])"

// SECTION 4: FULL PROGRAM EMISSION (6 tests)

#[test]
fn emit_epr_pair_full_program()
// H-CNOT-Measure -> matches spec Section 6 Python output

#[test]
fn emit_epr_pair_valid_python_syntax()
// python3 compile() succeeds (skip if Python unavailable)

#[test]
fn emit_ghz_three_qubit_program()
// H, CNOT, CNOT -> correct GHZ program

#[test]
fn emit_rotation_circuit_angles_correct()
// Rx, Ry, Rz angles all present in output

#[test]
fn emit_empty_circuit_valid_python()
// Empty circuit -> parseable Python

#[test]
fn emit_custom_gate_emits_name()
// Custom { name: "U3" } uses its name string

// SECTION 5: ROUNDTRIP EMISSION (4 tests)

#[test]
fn emit_roundtrip_epr_contains_h()
// EPR output contains "cirq.H"

#[test]
fn emit_roundtrip_epr_contains_cnot()
// EPR output contains "cirq.CNOT"

#[test]
fn emit_roundtrip_epr_contains_measure()
// EPR output contains "cirq.measure"

#[test]
fn emit_roundtrip_qubit_count_matches()
// N qubits in circuit -> N LineQubits in output
```

---

### Sprint 7: OpenQASM 3.0 Emission

**File:** `crates/logicaffeine_compile/src/codegen_cirq/openqasm_emitter.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_openqasm.rs`

#### Functions

```rust
pub fn emit_openqasm3(circuit: &CirqCircuit) -> String;
fn emit_qasm_header() -> String;
fn emit_qasm_qubit_declarations(circuit: &CirqCircuit) -> String;
fn emit_qasm_gate(gate: &CirqGate, qubits: &[Qubit]) -> String;
fn emit_qasm_measurement(qubits: &[Qubit], key: &str) -> String;
```

#### RED Tests — Sprint 7 (20)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 7: OPENQASM 3.0 EMISSION
// ═══════════════════════════════════════════════════════════════

// SECTION 1: QASM STRUCTURE (5 tests)

#[test]
fn qasm_header_version_3()
// "OPENQASM 3.0;" as first line

#[test]
fn qasm_includes_stdgates()
// 'include "stdgates.inc";' present

#[test]
fn qasm_qubit_declaration()
// "qubit[2] q;" for 2-qubit circuit

#[test]
fn qasm_bit_declaration_for_measurement()
// "bit[2] c;" when measurement present

#[test]
fn qasm_empty_circuit_valid()
// Valid QASM 3.0 for empty circuit

// SECTION 2: GATE MAPPING (8 tests)

#[test]
fn qasm_hadamard_gate()
// H(q0) -> "h q[0];"

#[test]
fn qasm_pauli_x_gate()
// X(q0) -> "x q[0];"

#[test]
fn qasm_cnot_gate()
// CNOT(q0,q1) -> "cx q[0], q[1];"

#[test]
fn qasm_rx_gate()
// Rx(theta) -> "rx(theta) q[0];"

#[test]
fn qasm_ry_gate()
// Ry(theta) -> "ry(theta) q[0];"

#[test]
fn qasm_rz_gate()
// Rz(theta) -> "rz(theta) q[0];"

#[test]
fn qasm_cz_gate()
// CZ(q0,q1) -> "cz q[0], q[1];"

#[test]
fn qasm_measurement()
// Measure -> "c[0] = measure q[0];"

// SECTION 3: FULL PROGRAMS (4 tests)

#[test]
fn qasm_epr_pair_full()
// Full EPR pair as valid QASM 3.0

#[test]
fn qasm_ghz_state_three_qubits()
// GHZ state as valid QASM 3.0

#[test]
fn qasm_rotation_circuit()
// Multiple Rx/Ry/Rz with correct angles

#[test]
fn qasm_barrier_emits()
// Barrier -> "barrier q[0], q[1];"

// SECTION 4: ROUNDTRIP (3 tests)

#[test]
fn qasm_roundtrip_epr_contains_h()
// EPR QASM output contains "h q[0];"

#[test]
fn qasm_roundtrip_gate_count_matches()
// Same number of gate lines as operations

#[test]
fn qasm_roundtrip_qubit_declaration_count()
// qubit[N] matches circuit qubit count
```

---

### Sprint 8: Circuit Optimization

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_optimize.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_optimize.rs`

#### Algebraic Identity Laws

```
HH = I                    Hadamard is its own inverse
XX = I                    Pauli-X is its own inverse
YY = I                    Pauli-Y is its own inverse
ZZ = I                    Pauli-Z is its own inverse
SS^dag = I                S and S-dagger cancel
TT^dag = I                T and T-dagger cancel
CNOT-CNOT = I             CNOT is its own inverse (same qubits)
SWAP-SWAP = I             SWAP is its own inverse
Rx(a) Rx(b) = Rx(a+b)    Rotation composition
Ry(a) Ry(b) = Ry(a+b)
Rz(a) Rz(b) = Rz(a+b)
Rx(2*PI) = I              Full rotation is identity
```

#### Functions

```rust
pub fn optimize_circuit(circuit: &CirqCircuit) -> CirqCircuit;
pub fn optimize_depth(circuit: &CirqCircuit) -> CirqCircuit;
pub fn apply_identity_laws(circuit: &CirqCircuit) -> CirqCircuit;
pub fn cancel_adjacent_gates(circuit: &CirqCircuit) -> CirqCircuit;
fn gates_cancel(a: &CirqGate, b: &CirqGate) -> bool;
fn is_identity_gate(gate: &CirqGate) -> bool;
```

#### RED Tests — Sprint 8 (25)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 8: CIRCUIT OPTIMIZATION
// ═══════════════════════════════════════════════════════════════

// SECTION 1: IDENTITY LAWS (12 tests)

#[test]
fn optimize_hh_cancels_to_identity()
// H-H on same qubit -> removed

#[test]
fn optimize_xx_cancels()
// X-X on same qubit -> removed

#[test]
fn optimize_yy_cancels()
// Y-Y -> removed

#[test]
fn optimize_zz_cancels()
// Z-Z -> removed

#[test]
fn optimize_s_sdg_cancels()
// S-Sdg -> removed

#[test]
fn optimize_t_tdg_cancels()
// T-Tdg -> removed

#[test]
fn optimize_cnot_cnot_cancels()
// CNOT-CNOT same qubits -> removed

#[test]
fn optimize_swap_swap_cancels()
// SWAP-SWAP same qubits -> removed

#[test]
fn optimize_rx_composition()
// Rx(a) then Rx(b) on same qubit -> Rx(a+b)

#[test]
fn optimize_ry_composition()
// Ry(a) then Ry(b) -> Ry(a+b)

#[test]
fn optimize_rz_composition()
// Rz(a) then Rz(b) -> Rz(a+b)

#[test]
fn optimize_rx_full_rotation_removed()
// Rx(2*PI) -> removed (identity)

// SECTION 2: DEPTH OPTIMIZATION (8 tests)

#[test]
fn optimize_depth_packs_independent()
// H(q0), X(q1) in 2 moments -> packed into 1 moment

#[test]
fn optimize_depth_preserves_dependencies()
// H(q0), CNOT(q0,q1) -> must stay 2 moments (q0 conflict)

#[test]
fn optimize_depth_three_independent()
// 3 single-qubit gates on q0,q1,q2 in 3 moments -> depth 1

#[test]
fn optimize_depth_cnot_chain_no_reduce()
// CNOT(q0,q1) then CNOT(q1,q2) -> 2 moments (q1 shared)

#[test]
fn optimize_depth_mixed_parallel()
// Some packable, some not -> correct mixed result

#[test]
fn optimize_preserves_gate_sequence()
// Same gates before/after optimization (different moment grouping)

#[test]
fn optimize_preserves_measurement_order()
// Measurement stays at end

#[test]
fn optimize_empty_circuit_unchanged()
// Optimization of empty -> empty

// SECTION 3: COMBINED OPTIMIZATION (5 tests)

#[test]
fn optimize_full_pipeline_epr()
// EPR pair: no cancellations, depth already optimal

#[test]
fn optimize_full_pipeline_hh_then_cnot()
// H-H cancels, CNOT remains

#[test]
fn optimize_barrier_prevents_cancellation()
// H-Barrier-H: barrier blocks H-H cancellation

#[test]
fn optimize_identity_gates_removed()
// I gates removed from output

#[test]
fn optimize_does_not_reorder_measurements()
// Measurements stay in place regardless of optimization
```

---

### Sprint 9: Dead Gate Analysis

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_dead_gate.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_dead_gate.rs`

Analog of `sva_vacuity.rs`. In quantum circuits, "dead gates" are operations that provably have no effect on the measurement outcome.

#### Types

```rust
pub enum DeadGateStatus {
    Live,
    Dead(DeadGateReason),
    Unknown,
}

pub enum DeadGateReason {
    IdentityGate,
    UnmeasuredQubit,
    PostMeasurement,
    CancelledBySuccessor,
    IsolatedQubit,
}

pub fn analyze_dead_gates(circuit: &CirqCircuit) -> Vec<(usize, usize, DeadGateStatus)>;
pub fn has_dead_gates(circuit: &CirqCircuit) -> bool;
pub fn remove_dead_gates(circuit: &CirqCircuit) -> CirqCircuit;
```

#### RED Tests — Sprint 9 (15)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 9: DEAD GATE ANALYSIS
// ═══════════════════════════════════════════════════════════════

// SECTION 1: IDENTITY DETECTION (3 tests)

#[test]
fn dead_gate_identity_is_dead()
// I(q0) -> Dead(IdentityGate)

#[test]
fn dead_gate_h_is_live()
// H(q0) then Measure(q0) -> Live

#[test]
fn dead_gate_rx_zero_is_dead()
// Rx(0.0) -> Dead(IdentityGate)

// SECTION 2: UNMEASURED QUBITS (4 tests)

#[test]
fn dead_gate_unmeasured_qubit()
// H(q0), Measure(q1) -> H on q0 is Dead(UnmeasuredQubit)

#[test]
fn dead_gate_measured_qubit_is_live()
// H(q0), Measure(q0) -> Live

#[test]
fn dead_gate_entangled_unmeasured_live()
// H(q0), CNOT(q0,q1), Measure(q1) -> H on q0 is Live (entangled with measured q1)

#[test]
fn dead_gate_isolated_unmeasured_dead()
// H(q0), H(q1), Measure(q1) -> q0 ops are Dead(IsolatedQubit)

// SECTION 3: POST-MEASUREMENT (4 tests)

#[test]
fn dead_gate_after_measurement()
// Measure(q0), H(q0) -> H is Dead(PostMeasurement)

#[test]
fn dead_gate_before_measurement_live()
// H(q0), Measure(q0) -> H is Live

#[test]
fn dead_gate_different_qubit_after_measure()
// Measure(q0), H(q1) -> H(q1) is Live (different qubit)

#[test]
fn dead_gate_cnot_after_measure_on_control()
// Measure(q0), CNOT(q0,q1) -> Dead(PostMeasurement)

// SECTION 4: CANCELLATION AND AGGREGATION (4 tests)

#[test]
fn dead_gate_hh_cancellation()
// H(q0), H(q0) -> both Dead(CancelledBySuccessor)

#[test]
fn dead_gate_partial_cancellation()
// H(q0), H(q0), X(q0), Measure(q0) -> HH dead, X live

#[test]
fn has_dead_gates_epr_false()
// EPR pair: no dead gates

#[test]
fn remove_dead_gates_strips_identity()
// I gates removed, circuit smaller
```

---

## Phase III: Pipeline & Verification

### Sprint 10: Pipeline API

**File:** `crates/logicaffeine_compile/src/codegen_cirq/quantum_pipeline.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_pipeline.rs`

#### Public API

```rust
pub fn compile_quantum_spec(source: &str) -> Result<String, CirqError>;
pub fn translate_to_cirq_model(source: &str) -> Result<CirqCircuit, CirqError>;
pub fn emit_cirq_from_spec(source: &str) -> Result<String, CirqError>;
pub fn emit_qasm_from_spec(source: &str) -> Result<String, CirqError>;
pub fn optimize_and_emit(source: &str) -> Result<String, CirqError>;

#[cfg(feature = "verification")]
pub fn verify_quantum_spec(source: &str) -> Result<VerificationResult, CirqError>;
#[cfg(feature = "verification")]
pub fn check_quantum_equivalence(
    spec: &str,
    circuit: &CirqCircuit,
) -> Result<EquivalenceResult, CirqError>;
```

Pipeline wiring: `compile_kripke_with()` -> `KripkeToCirqTranslator::translate()` -> `emit_cirq_python()` / `emit_openqasm3()`.

#### RED Tests — Sprint 10 (18)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 10: PUBLIC PIPELINE API
// ═══════════════════════════════════════════════════════════════

// SECTION 1: COMPILATION (4 tests)

#[test]
fn pipeline_compile_quantum_spec_returns_fol()
// English quantum spec -> Kripke FOL string

#[test]
fn pipeline_translate_to_cirq_model()
// English spec -> CirqCircuit (moments and operations present)

#[test]
fn pipeline_emit_python_from_spec()
// English spec -> Python string containing "import cirq"

#[test]
fn pipeline_emit_qasm_from_spec()
// English spec -> QASM string containing "OPENQASM 3.0"

// SECTION 2: EPR PAIR E2E (4 tests)

#[test]
fn pipeline_epr_pair_e2e_python()
// EPR spec -> full Python program

#[test]
fn pipeline_epr_pair_e2e_qasm()
// EPR spec -> full QASM program

#[test]
fn pipeline_epr_pair_circuit_has_3_moments()
// H, CNOT, Measure = 3 moments

#[test]
fn pipeline_epr_pair_qubit_count_is_2()
// 2 qubits declared

// SECTION 3: ERROR HANDLING (4 tests)

#[test]
fn pipeline_error_on_empty_input()
// "" -> CirqError::ParseError

#[test]
fn pipeline_error_on_invalid_spec()
// "Blarg blarg" -> CirqError::ParseError

#[test]
fn pipeline_error_on_undeclared_qubit()
// Missing "Let q1 : Qubit." -> CirqError::TranslationError

#[test]
fn pipeline_error_on_wrong_gate_arity()
// H(q1, q2) -> CirqError::TranslationError (H is single-qubit)

// SECTION 4: PIPELINE COMPOSITION (6 tests)

#[test]
fn pipeline_optimize_then_emit()
// Optimize + emit: dead gates removed in output

#[test]
fn pipeline_qubit_allocation_matches_world_count()
// N world variables in spec -> N qubits in output

#[test]
fn pipeline_ghz_three_qubit_e2e()
// GHZ spec -> correct circuit

#[test]
fn pipeline_rotation_e2e()
// Graded modality -> Ry in output

#[test]
fn pipeline_measurement_key_in_output()
// Measurement key present in Python output

#[test]
fn pipeline_metadata_preserved()
// Source spec stored in circuit metadata
```

---

### Sprints 11-12: Symbolic Matrices and Tensor Products

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_to_verify.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_verify.rs`

#### Types

```rust
/// Symbolic complex number for Z3 encoding.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolicComplex {
    pub re: VerifyExpr,
    pub im: VerifyExpr,
}

/// Symbolic matrix for quantum gate representation.
#[derive(Debug, Clone)]
pub struct SymbolicMatrix {
    pub rows: usize,
    pub cols: usize,
    pub entries: Vec<Vec<SymbolicComplex>>,
}

/// Symbolic state vector.
pub type SymbolicStateVector = Vec<SymbolicComplex>;

/// Maximum qubits for full symbolic verification.
pub const MAX_SYMBOLIC_QUBITS: usize = 10;
```

#### Functions

```rust
// Gate matrices
pub fn hadamard_matrix() -> SymbolicMatrix;
pub fn pauli_x_matrix() -> SymbolicMatrix;
pub fn pauli_y_matrix() -> SymbolicMatrix;
pub fn pauli_z_matrix() -> SymbolicMatrix;
pub fn cnot_matrix() -> SymbolicMatrix;
pub fn cz_matrix() -> SymbolicMatrix;
pub fn swap_matrix() -> SymbolicMatrix;
pub fn toffoli_matrix() -> SymbolicMatrix;
pub fn rx_matrix(theta: f64) -> SymbolicMatrix;
pub fn ry_matrix(theta: f64) -> SymbolicMatrix;
pub fn rz_matrix(theta: f64) -> SymbolicMatrix;
pub fn identity_matrix(dim: usize) -> SymbolicMatrix;

// Composition
pub fn tensor_product(a: &SymbolicMatrix, b: &SymbolicMatrix) -> SymbolicMatrix;
pub fn symbolic_matmul(a: &SymbolicMatrix, b: &SymbolicMatrix) -> SymbolicMatrix;
pub fn symbolic_matvec(mat: &SymbolicMatrix, vec: &SymbolicStateVector) -> SymbolicStateVector;

// Circuit -> unitary
pub fn gate_to_matrix(gate: &CirqGate) -> SymbolicMatrix;
pub fn moment_to_unitary(moment: &CirqMoment, n_qubits: usize) -> SymbolicMatrix;
pub fn circuit_to_symbolic_unitary(circuit: &CirqCircuit) -> SymbolicMatrix;
pub fn apply_circuit_to_state(circuit: &CirqCircuit) -> SymbolicStateVector;
pub fn initial_state_vector(n_qubits: usize) -> SymbolicStateVector;

// Scalability check
pub fn can_symbolically_verify(circuit: &CirqCircuit) -> bool;
```

Encoding strategy: amplitudes as `SymbolicComplex { re, im }` where `re`/`im` are `VerifyExpr::Int` for exact rationals or `VerifyExpr::Apply("sqrt2_inv", [])` for $1/\sqrt{2}$. Z3's Real sort handles these without modifying `VerifyExpr`.

#### RED Tests — Sprints 11-12 (30)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINTS 11-12: SYMBOLIC MATRICES & TENSOR PRODUCTS
// ═══════════════════════════════════════════════════════════════

// SECTION 1: INDIVIDUAL GATE MATRICES (8 tests)

#[test]
fn symbolic_hadamard_matrix_entries()
// H[0][0].re = 1/sqrt(2), H[0][1].re = 1/sqrt(2), etc.

#[test]
fn symbolic_pauli_x_matrix_entries()
// X = [[0,1],[1,0]] symbolically

#[test]
fn symbolic_pauli_z_matrix_entries()
// Z = [[1,0],[0,-1]] symbolically

#[test]
fn symbolic_cnot_matrix_4x4()
// CNOT is 4x4 with correct entries

#[test]
fn symbolic_identity_matrix_2x2()
// I_2 = [[1,0],[0,1]]

#[test]
fn symbolic_rx_at_pi_over_2()
// Rx(PI/2) has correct cos/sin entries

#[test]
fn symbolic_initial_state_1qubit()
// |0> = [(1,0), (0,0)] (complex)

#[test]
fn symbolic_initial_state_2qubit()
// |00> = [(1,0), (0,0), (0,0), (0,0)] for 2-qubit system

// SECTION 2: TENSOR PRODUCTS (6 tests)

#[test]
fn tensor_h_kron_i()
// H tensor I -> 4x4 matrix

#[test]
fn tensor_i_kron_x()
// I tensor X -> 4x4 matrix

#[test]
fn tensor_h_kron_h()
// H tensor H -> 4x4 (uniform superposition generator)

#[test]
fn tensor_product_dimensions()
// (2x2) tensor (2x2) -> 4x4

#[test]
fn tensor_product_dimensions_3qubit()
// (2x2) tensor (4x4) -> 8x8

#[test]
fn tensor_identity_is_identity()
// I tensor I = I_4

// SECTION 3: MATRIX MULTIPLICATION (6 tests)

#[test]
fn symbolic_matvec_h_on_zero()
// H|0> = [1/sqrt(2), 1/sqrt(2)]

#[test]
fn symbolic_matvec_x_on_zero()
// X|0> = [0, 1]

#[test]
fn symbolic_matmul_hh_equals_identity()
// H * H = I (algebraic verification)

#[test]
fn symbolic_matmul_cnot_times_h_kron_i()
// CNOT * (H tensor I) for EPR pair

#[test]
fn symbolic_matvec_epr_pair()
// CNOT * (H tensor I) |00> = [1/sqrt(2), 0, 0, 1/sqrt(2)]

#[test]
fn symbolic_matvec_dimensions_match()
// 4x4 matrix * 4-vector -> 4-vector

// SECTION 4: CIRCUIT TO UNITARY (6 tests)

#[test]
fn circuit_to_unitary_single_h()
// H(q0) on 2-qubit system -> symbolic unitary == H tensor I

#[test]
fn circuit_to_unitary_epr()
// EPR circuit -> CNOT * (H tensor I)

#[test]
fn circuit_to_unitary_two_moments()
// [M1, M2] -> U_M2 * U_M1 (later moments left-multiply)

#[test]
fn circuit_to_unitary_parallel_gates()
// H(q0), X(q1) same moment -> H tensor X

#[test]
fn circuit_to_state_epr()
// U * |00> = [1/sqrt(2), 0, 0, 1/sqrt(2)] (Bell state |Phi+>)

#[test]
fn circuit_scalability_check()
// can_symbolically_verify(11-qubit circuit) == false

// SECTION 5: VERIFY BRIDGE (4 tests)

#[test]
fn circuit_to_verify_expr_produces_apply()
// CirqCircuit -> VerifyExpr with Apply nodes for amplitude encoding

#[test]
fn amplitude_to_verify_probability()
// |alpha|^2 encoded as VerifyExpr::Binary { Mul, re, re } + Binary { Mul, im, im }

#[test]
fn state_vector_normalization_constraint()
// sum |alpha_i|^2 = 1 as VerifyExpr constraint

#[test]
fn measurement_probability_constraint()
// Pr(outcome) = |alpha_outcome|^2 as VerifyExpr constraint
```

---

### Sprints 13-14: Z3 Amplitude Checking and Equivalence

**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_z3_verify.rs`
**Feature-gated:** `#[cfg(feature = "verification")]`

#### Functions (in `cirq_to_verify.rs` and `quantum_pipeline.rs`)

```rust
#[cfg(feature = "verification")]
pub fn cirq_to_verify_expr(circuit: &CirqCircuit) -> VerifyExpr;

#[cfg(feature = "verification")]
pub fn check_amplitude_equivalence(
    spec_amplitudes: &[VerifyExpr],
    circuit: &CirqCircuit,
) -> EquivalenceResult;
```

Method: construct $\neg(\text{spec\_amplitudes} \leftrightarrow \text{circuit\_amplitudes})$, check satisfiability. UNSAT means equivalent. SAT produces counterexample.

#### RED Tests — Sprints 13-14 (22)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINTS 13-14: Z3 QUANTUM VERIFICATION
// ═══════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_quantum {

// SECTION 1: AMPLITUDE VERIFICATION (8 tests)

#[test]
fn z3_verify_epr_amplitudes()
// EPR: Pr(|00>) = 0.5, Pr(|11>) = 0.5, Pr(|01>) = 0, Pr(|10>) = 0 -> SAT (correct)

#[test]
fn z3_verify_hadamard_superposition()
// H|0>: |amp(|0>)|^2 = 0.5, |amp(|1>)|^2 = 0.5 -> SAT

#[test]
fn z3_verify_pauli_x_flips()
// X|0> = |1> -> SAT

#[test]
fn z3_verify_identity_preserves_state()
// I|0> = |0> -> SAT

#[test]
fn z3_verify_ghz_amplitudes()
// GHZ: Pr(|000>) = 0.5, Pr(|111>) = 0.5, rest 0 -> SAT

#[test]
fn z3_verify_wrong_amplitude_unsat()
// Claiming H|0> has amp(|0>) = 1.0 (no superposition) -> UNSAT

#[test]
fn z3_verify_rx_rotation_probability()
// Rx(PI/3)|0> has correct Pr(|1>) -> SAT

#[test]
fn z3_verify_ry_probability_matches_force()
// Ry from force=0.25 -> Pr(|1>) == 0.25 -> SAT

// SECTION 2: ENTANGLEMENT VERIFICATION (4 tests)

#[test]
fn z3_verify_epr_not_separable()
// EPR state cannot factor as tensor product -> UNSAT (separability constraint fails)

#[test]
fn z3_verify_product_state_is_separable()
// H|0> tensor |0> IS separable -> SAT

#[test]
fn z3_verify_ghz_not_separable()
// GHZ is entangled -> UNSAT

#[test]
fn z3_verify_cnot_on_00_is_product()
// CNOT|00> = |00> is separable (no entanglement on computational basis) -> SAT

// SECTION 3: EQUIVALENCE CHECKING (6 tests)

#[test]
fn z3_spec_matches_circuit_epr()
// LOGOS spec probability requirements match synthesized EPR circuit amplitudes

#[test]
fn z3_wrong_circuit_fails_verification()
// Spec says entangled but circuit missing CNOT -> mismatch detected

#[test]
fn z3_equivalent_circuits_confirmed()
// Two different circuits with same unitary -> equivalent

#[test]
fn z3_different_circuits_detected()
// H(q0) vs X(q0) -> not equivalent

#[test]
fn z3_rotation_equivalence()
// Rx(PI) equivalent to X (up to global phase)

#[test]
fn z3_empty_circuit_is_identity()
// Empty circuit equivalent to identity operation

// SECTION 4: SCALABILITY LIMITS (4 tests)

#[test]
fn z3_verify_3_qubit_feasible()
// 3 qubits verifies within timeout

#[test]
fn z3_verify_5_qubit_feasible()
// 5 qubits verifies (32x32 matrix)

#[test]
fn z3_reject_12_qubit_symbolic()
// 12 qubits -> CirqError::VerificationError (exceeds MAX_SYMBOLIC_QUBITS)

#[test]
fn z3_algebraic_verify_large_circuit()
// Large circuit: per-gate identity check only, no full symbolic

} // mod z3_quantum
```

---

## Phase IV: Integration (Kernel, Reference, Serialization)

### Sprint 15: Kernel Bridge (Curry-Howard)

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_to_kernel.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_kernel.rs`

The kernel at `prelude.rs:2269` defines:
- `Bit : Type 0` with constructors `B0`, `B1`
- `BVec : Nat -> Type 0` with `BVNil`, `BVCons`
- `Circuit S I O : Type 0` with `MkCircuit` (transition function, output function, initial state)
- Gate operations: `bit_and`, `bit_or`, `bit_not`, `bit_xor`, `bit_mux`

The quantum kernel bridge maps:
- Qubit measurement outcome -> `Bit` (classical 0/1)
- Quantum state register -> `BVec` (length-indexed)
- CirqCircuit -> `Circuit` with unitary transition function

#### Functions

```rust
pub fn encode_cirq_circuit(circuit: &CirqCircuit) -> Term;
pub fn encode_gate_as_term(gate: &CirqGate) -> Term;
pub fn encode_measurement_as_term(qubits: &[Qubit]) -> Term;
pub fn encode_state_vector(n_qubits: usize) -> Term;
```

#### RED Tests — Sprint 15 (15)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 15: CURRY-HOWARD KERNEL BRIDGE
// ═══════════════════════════════════════════════════════════════

// SECTION 1: GATE ENCODING (5 tests)

#[test]
fn kernel_encode_hadamard()
// H -> App(Global("H_gate"), ...) term

#[test]
fn kernel_encode_pauli_x()
// X -> App(Global("X_gate"), ...) term

#[test]
fn kernel_encode_cnot()
// CNOT -> term with two qubit arguments

#[test]
fn kernel_encode_rx_with_parameter()
// Rx(theta) -> term preserving angle as Lit(Float)

#[test]
fn kernel_encode_measure()
// Measure -> App(Global("Measure"), ...) term

// SECTION 2: CIRCUIT ENCODING (5 tests)

#[test]
fn kernel_encode_single_moment_circuit()
// 1 moment -> MkCircuit term with transition function

#[test]
fn kernel_encode_epr_pair()
// EPR -> MkCircuit with H then CNOT transition composition

#[test]
fn kernel_encode_circuit_initial_state()
// Initial state is all-zeros BVec

#[test]
fn kernel_encode_circuit_qubit_count()
// N qubits -> BVec of length N

#[test]
fn kernel_encode_empty_circuit()
// Empty circuit -> identity transition function

// SECTION 3: CURRY-HOWARD TYPE CHECKING (5 tests)

#[test]
fn kernel_circuit_proof_type_check()
// Encoded circuit type-checks in kernel

#[test]
fn kernel_circuit_transition_type()
// transition : S -> I -> S

#[test]
fn kernel_circuit_output_type()
// output : S -> I -> O

#[test]
fn kernel_gate_sequence_composes()
// Two gates -> composed transition function

#[test]
fn kernel_measurement_returns_bit()
// Measure output type is Bit
```

---

### Sprint 16: Reference Circuits

**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_reference.rs`

Each reference circuit gets 4 tests: LOGOS parse, circuit structure, Python emit, QASM emit.

#### RED Tests — Sprint 16 (24)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 16: REFERENCE CIRCUITS
// ═══════════════════════════════════════════════════════════════

// SECTION 1: EPR PAIR — Bell State |Phi+> (4 tests)

#[test]
fn ref_epr_parse()
// LOGOS spec -> CirqCircuit with H, CNOT, Measure

#[test]
fn ref_epr_structure()
// 3 moments, 2 qubits, gate types: [H, CNOT, Measure]

#[test]
fn ref_epr_python()
// Emitted Python matches golden reference

#[test]
fn ref_epr_qasm()
// Emitted QASM is valid OpenQASM 3.0

// SECTION 2: GHZ STATE — 3-Qubit Entanglement (4 tests)

#[test]
fn ref_ghz_parse()
// H(q0), CNOT(q0,q1), CNOT(q0,q2) from spec

#[test]
fn ref_ghz_structure()
// 3 moments, 3 qubits

#[test]
fn ref_ghz_python()
// Python output has 3 LineQubits

#[test]
fn ref_ghz_qasm()
// QASM output has qubit[3]

// SECTION 3: DEUTSCH-JOZSA — Quantum Oracle Query (4 tests)

#[test]
fn ref_deutsch_jozsa_parse()
// Spec produces initial H layer, oracle, final H layer, measure

#[test]
fn ref_deutsch_jozsa_structure()
// Correct moment structure: H layer -> oracle -> H layer -> measure

#[test]
fn ref_deutsch_jozsa_python()
// Python has correct gate sequence

#[test]
fn ref_deutsch_jozsa_qasm()
// QASM has correct gate sequence

// SECTION 4: QUANTUM TELEPORTATION — 3-Qubit Protocol (4 tests)

#[test]
fn ref_teleport_parse()
// 3-qubit teleportation: Bell pair prep + Bell measurement + correction

#[test]
fn ref_teleport_structure()
// Correct moment count and gate types

#[test]
fn ref_teleport_python()
// Python with Bell pair and measurement

#[test]
fn ref_teleport_qasm()
// QASM with correct protocol structure

// SECTION 5: QFT — Quantum Fourier Transform, 2-Qubit (4 tests)

#[test]
fn ref_qft_2qubit_parse()
// 2-qubit QFT spec parsed

#[test]
fn ref_qft_2qubit_structure()
// H, CPhase(PI/2), H, SWAP

#[test]
fn ref_qft_2qubit_python()
// Python with controlled phase gates

#[test]
fn ref_qft_2qubit_qasm()
// QASM with cp gates

// SECTION 6: GROVER'S ALGORITHM — 2-Qubit, 1 Iteration (4 tests)

#[test]
fn ref_grover_2qubit_parse()
// 2-qubit Grover spec parsed

#[test]
fn ref_grover_2qubit_structure()
// H layer, oracle, diffusion operator

#[test]
fn ref_grover_2qubit_python()
// Python with oracle + diffusion

#[test]
fn ref_grover_2qubit_qasm()
// QASM with oracle + diffusion
```

---

### Sprint 17: Serialization

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_serialize.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_serialize.rs`

#### Functions

```rust
pub fn serialize_circuit(circuit: &CirqCircuit) -> String;
pub fn deserialize_circuit(json: &str) -> Result<CirqCircuit, CirqError>;
```

#### RED Tests — Sprint 17 (12)

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 17: SERIALIZATION
// ═══════════════════════════════════════════════════════════════

// SECTION 1: SERIALIZE (4 tests)

#[test]
fn serialize_empty_circuit()
// Empty circuit -> valid JSON

#[test]
fn serialize_single_gate()
// H(q0) -> JSON with gate and qubit fields

#[test]
fn serialize_epr_pair()
// EPR -> JSON with 3 moments

#[test]
fn serialize_rotation_preserves_angle()
// Rx(PI/4) angle present in JSON

// SECTION 2: DESERIALIZE (4 tests)

#[test]
fn deserialize_empty_circuit()
// JSON -> empty CirqCircuit

#[test]
fn deserialize_single_gate()
// JSON -> H(q0) circuit

#[test]
fn deserialize_epr_pair()
// JSON -> EPR circuit

#[test]
fn deserialize_invalid_json_errors()
// Malformed JSON -> CirqError::SerializationError

// SECTION 3: ROUNDTRIP (4 tests)

#[test]
fn serialize_roundtrip_epr()
// serialize -> deserialize -> structurally equivalent

#[test]
fn serialize_roundtrip_rotation()
// Angle preserved through JSON roundtrip

#[test]
fn serialize_roundtrip_measurement()
// Measurement key preserved through JSON roundtrip

#[test]
fn serialize_roundtrip_custom_gate()
// Custom gate name and params preserved
```

---

## Sprint Dependency Graph

```
Sprint 0 (AST / parser / lexicon)
    |
    v
Sprint 1 (IR types — CirqGate, CirqOperation, CirqMoment, CirqCircuit)
    |
    v
Sprint 2 (display, structural equivalence, metrics, roundtrip)
    |
    +------------------+------------------+------------------+
    |                  |                  |                  |
    v                  v                  v                  v
Sprint 3            Sprint 5            Sprint 8           Sprint 9
(force mapping)     (Python emitter)    (optimization)     (dead gate)
    |                  |                  |                  |
    v                  v                  |                  |
Sprint 4            Sprint 6            |                  |
(full translate)    (full emission)     |                  |
    |                  |                  |                  |
    +-------+----------+                  |                  |
            |                             |                  |
            v                             |                  |
         Sprint 7                         |                  |
         (OpenQASM)                       |                  |
            |                             |                  |
            +-----------------------------+------------------+
                            |
                            v
                       Sprint 10
                       (pipeline API)
                            |
                            v
                       Sprint 11
                       (symbolic matrices)
                            |
                            v
                       Sprint 12
                       (tensor products)
                            |
                            v
                       Sprint 13                    [feature = "verification"]
                       (Z3 amplitude checking)
                            |
                            v
                       Sprint 14                    [feature = "verification"]
                       (Z3 equivalence)
                            |
                            v
                       Sprint 15
                       (kernel bridge)
                            |
                            v
                       Sprint 16
                       (reference circuits)
                            |
                            v
                       Sprint 17
                       (serialization)
```

**Parallelism:** Sprints 3/4, 5/6, 7, 8, and 9 can all proceed in parallel after Sprint 2.
Sprints 11-14 are strictly sequential — each builds on the previous.

---

## Module Registration

### `codegen_cirq/mod.rs`

```rust
pub mod cirq_model;
pub mod kripke_to_cirq;
pub mod cirq_emitter;
pub mod openqasm_emitter;
pub mod cirq_optimize;
pub mod cirq_dead_gate;
pub mod quantum_pipeline;
pub mod cirq_to_verify;
pub mod cirq_to_kernel;
pub mod cirq_serialize;
```

### In `logicaffeine_compile/src/lib.rs`

Add alongside the existing `pub mod codegen_sva;` at line 124:

```rust
pub mod codegen_cirq;
```

No feature gate needed — the SVA backend is not feature-gated either. Verification-specific code within `cirq_to_verify.rs` and `quantum_pipeline.rs` uses `#[cfg(feature = "verification")]` internally.

---

## How We Test Generated Python

Three levels of verification, from fastest to most rigorous:

1. **String pattern assertions** — `output.contains("cirq.H(q[0])")`. Fast, no external deps.
2. **Python syntax validation** — `python3 -c "compile(source, '<test>', 'exec')"` subprocess. Skipped if Python unavailable.
3. **Z3 symbolic verification** — encode circuit as matrix multiplication over Z3 Reals, check amplitudes match spec. Feature-gated behind `verification`.

---

## Test Count Summary

| Phase | Sprint | Test File | Tests |
|:---|:---|:---|---:|
| 0 | 0 | `phase_cirq_ast.rs` | 20 |
| I | 1 | `phase_cirq_model.rs` | 24 |
| I | 2 | `phase_cirq_model.rs` | 24 |
| I | 3 | `phase_cirq_kripke_map.rs` | 18 |
| I | 4 | `phase_cirq_kripke_map.rs` | 17 |
| II | 5-6 | `phase_cirq_emit.rs` | 30 |
| II | 7 | `phase_cirq_openqasm.rs` | 20 |
| II | 8 | `phase_cirq_optimize.rs` | 25 |
| II | 9 | `phase_cirq_dead_gate.rs` | 15 |
| III | 10 | `phase_cirq_pipeline.rs` | 18 |
| III | 11-12 | `phase_cirq_verify.rs` | 30 |
| III | 13-14 | `phase_cirq_z3_verify.rs` | 22 |
| IV | 15 | `phase_cirq_kernel.rs` | 15 |
| IV | 16 | `phase_cirq_reference.rs` | 24 |
| IV | 17 | `phase_cirq_serialize.rs` | 12 |
| **Total** | **17 sprints** | **13 test files** | **314** |

---

## Formal Synthesis Example: The EPR Pair Threading Through All Sprints

The EPR pair is the golden reference that threads through every sprint:

### English Specification

```logos
Let q1 : Qubit.
Let q2 : Qubit.
Apply Superposition to q1 with Force: 0.5.
Next, Apply Entanglement(q1, q2).
Measure(q1, q2).
```

### Sprint 0 (AST)

Parse into: `[QuantumDecl{q1}, QuantumDecl{q2}, QuantumGate{Superposition, [q1], 0.5}, Temporal{Next, QuantumGate{Entanglement, [q1,q2]}}, QuantumMeasure{[q1,q2]}]`

### Sprints 1-2 (IR)

Build:
```
CirqCircuit {
    qubits: [Line(0), Line(1)],
    moments: [
        Moment { [H(q0)] },
        Moment { [CNOT(q0, q1)] },
        Moment { [Measure("result", q0, q1)] },
    ],
}
```

Verify structural equivalence on clone. Verify display produces diagram. Roundtrip through display and parse.

### Sprints 3-4 (Translation)

`KripkeToCirqTranslator` maps: force=0.5 -> H, Entanglement -> CNOT, Measure -> Measure. `Next` creates moment boundary.

### Sprints 5-6 (Python)

```python
import cirq

q = [cirq.LineQubit(i) for i in range(2)]
circuit = cirq.Circuit()

circuit.append(cirq.H(q[0]))
circuit.append(cirq.CNOT(q[0], q[1]))
circuit.append(cirq.measure(q[0], q[1], key='result'))

simulator = cirq.Simulator()
result = simulator.simulate(circuit)
print(result.final_state_vector)
```

### Sprint 7 (OpenQASM)

```qasm
OPENQASM 3.0;
include "stdgates.inc";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c[0] = measure q[0];
c[1] = measure q[1];
```

### Sprint 8 (Optimize)

EPR pair is already optimal. Verify optimization is identity (no changes).

### Sprint 9 (Dead Gate)

No dead gates. All operations contribute to measurement outcomes.

### Sprint 10 (Pipeline)

`emit_cirq_from_spec(epr_spec)` returns the Python string from Sprint 5-6.

### Sprints 11-12 (Symbolic)

$U = CNOT \cdot (H \otimes I)$

$U|00\rangle = \frac{1}{\sqrt{2}}(|00\rangle + |11\rangle)$

### Sprints 13-14 (Z3)

Z3 confirms: $\neg(\text{Pr}(|00\rangle) = 0.5 \wedge \text{Pr}(|11\rangle) = 0.5 \wedge \text{Pr}(|01\rangle) = 0 \wedge \text{Pr}(|10\rangle) = 0)$ is **UNSAT**. QED.

### Sprint 15 (Kernel)

EPR encoded as `MkCircuit S I O transition output initial` where transition composes H then CNOT gate terms.

### Sprint 16 (Reference)

EPR is reference circuit #1. All 4 tests (parse, structure, Python, QASM) pass.

### Sprint 17 (Serialize)

EPR serializes to JSON and deserializes back to structurally equivalent circuit.

---

## NISQ Awareness

The spec targets "NISQ & FTQC Regimes." The architectural boundary:

- **Formal verification** proves correctness of the **ideal (noiseless) circuit**. This is what LogicAffeine does — the Z3 proof is about mathematical equivalence between specification and circuit.
- **Noise simulation** estimates fidelity under realistic conditions. This is what Cirq does at runtime via `cirq.DensityMatrixSimulator()`.

When `CircuitMetadata.noise_model` is specified, the emitted Python uses `cirq.DensityMatrixSimulator()` instead of `cirq.Simulator()`. But verification always operates on the ideal circuit.

---

## Critical Files Modified Outside codegen_cirq/

| File | Modification | Sprint |
|:---|:---|:---|
| `crates/logicaffeine_language/src/ast/logic.rs` | Add `QuantumDecl`, `QuantumGate`, `QuantumMeasure` variants to `LogicExpr`; add `QuantumGateKind` enum | 0 |
| `crates/logicaffeine_language/src/semantics/kripke.rs` | Add match arms in `lower_expr()` at line 88 for quantum variants | 0 |
| `crates/logicaffeine_language/src/parser/` | Quantum pattern recognition for `Let q : Qubit.`, `Apply X to q.`, `Measure(q).` | 0 |
| `crates/logicaffeine_compile/src/lib.rs` | Add `pub mod codegen_cirq;` alongside line 124's `pub mod codegen_sva;` | 1 |
