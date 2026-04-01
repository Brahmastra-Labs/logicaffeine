# LogicAffeine Quantum Backend: Cirq Implementation Plan

**The world's first readable quantum language.**
English specifications compiled to formally verified quantum circuits.

---

## Architecture

The Cirq backend mirrors the proven SVA backend pattern — domain IR, translator, emitter, pipeline, verification bridge — adapted for quantum circuit synthesis.

### Source Files

```
crates/logicaffeine_compile/src/codegen_cirq/
  mod.rs              — module exports
  cirq_model.rs       — domain IR: CirqGate, CirqOperation, CirqMoment, CirqCircuit
  kripke_to_cirq.rs   — Kripke-Quantum Isomorphism: modal force → gates, worlds → qubits
  cirq_emitter.rs     — Python/Cirq code generation
  cirq_to_verify.rs   — symbolic matrix multiplication, Z3 amplitude encoding
  quantum_pipeline.rs — public API: compile, emit, verify
```

### Test Files

```
crates/logicaffeine_tests/tests/
  phase_cirq_model.rs       — Sprints 1-2: IR types, structural equivalence
  phase_cirq_kripke_map.rs  — Sprint 3: modal force → gate mapping
  phase_cirq_translate.rs   — Sprint 4: full Kripke → CirqModel translation
  phase_cirq_emit.rs        — Sprints 5-6: Python emission
  phase_cirq_optimize.rs    — Sprint 7: moment packing, depth optimization
  phase_cirq_pipeline.rs    — Sprint 8: pipeline API
  phase_cirq_verify.rs      — Sprints 9-10: symbolic matrices, tensor products
  phase_cirq_z3_verify.rs   — Sprints 11-12: Z3 amplitude checking, E2E
```

### The Kripke-Quantum Isomorphism

This is the foundational mapping from modal logic to quantum mechanics:

| Kripke Semantics | Quantum Mechanics | Cirq Target |
|:---|:---|:---|
| Possible worlds W | Basis states \|x> in C^{2^n} | `cirq.LineQubit` |
| Accessibility relation R | Unitary operators U | Gate application |
| Box (force > 0.5, necessity) | Deterministic transition | `cirq.X` (Pauli-X) |
| Diamond (force = 0.5, possibility) | Superposition | `cirq.H` (Hadamard) |
| Graded modality (force = theta) | Rotation R_x(theta) | `cirq.rx(theta)` |
| LTL Next (X phi) | Discrete time slice | `cirq.Moment` |
| Always, q1 iff q2 | Entanglement generation | `cirq.CNOT` |
| Measure(q) | Projective measurement | `cirq.measure(q)` |
| World variable w_i | Qubit index i | `LineQubit(i)` |

### How We Test Generated Python

Three levels of verification, from fastest to most rigorous:

1. **String pattern assertions** — `output.contains("cirq.H(q[0])")`. Fast, no external deps.
2. **Python syntax validation** — `python3 -c "compile(source, '<test>', 'exec')"` subprocess. Skipped if Python unavailable.
3. **Z3 symbolic verification** — encode circuit as matrix multiplication over Z3 Reals, check amplitudes match spec. Feature-gated behind `verification`.

### Reference Pattern: The SVA Backend

| SVA File | Cirq Analog | Role |
|:---|:---|:---|
| `sva_model.rs` | `cirq_model.rs` | Domain-specific IR with parse/emit/structural-equiv |
| `sva_to_verify.rs` | `cirq_to_verify.rs` | Translate domain IR to Z3-ready verification IR |
| `fol_to_verify.rs` | `kripke_to_cirq.rs` | Translate Kripke FOL to domain IR |
| `hw_pipeline.rs` | `quantum_pipeline.rs` | Public API orchestrating the full pipeline |

---

## Phase I: Foundational Topology (Lexicon & IR)

### Sprint 1: CirqModel IR Construction

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_model.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_model.rs`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 1: IR CONSTRUCTION
// ═══════════════════════════════════════════════════════════════

#[test]
fn cirq_line_qubit_creation()
// Construct Qubit::Line(0), verify index == 0 and display == "q(0)"

#[test]
fn cirq_grid_qubit_creation()
// Construct Qubit::Grid(0, 1), verify row == 0 and col == 1

#[test]
fn cirq_gate_hadamard_is_single_qubit()
// CirqGate::H requires exactly 1 qubit, name == "H"

#[test]
fn cirq_gate_cnot_is_two_qubit()
// CirqGate::CNOT requires exactly 2 qubits (control + target)

#[test]
fn cirq_gate_rx_carries_angle()
// CirqGate::Rx(PI/2) preserves the angle value

#[test]
fn cirq_operation_binds_gate_to_qubits()
// CirqOperation { gate: H, qubits: [Line(0)] } stores both correctly

#[test]
fn cirq_moment_enforces_disjoint_qubits()
// Two operations on same qubit in one moment → error

#[test]
fn cirq_moment_allows_parallel_on_different_qubits()
// H(q0) and X(q1) in same moment → succeeds (disjoint qubits)

#[test]
fn cirq_circuit_appends_moments_in_order()
// Circuit with 2 moments: moments.len() == 2, ordering preserved

#[test]
fn cirq_circuit_qubit_count()
// Circuit using qubits 0..3: qubit_count() == 4

#[test]
fn cirq_measurement_has_key()
// CirqGate::Measure { key: "result" } stores the measurement key
```

#### GREEN Implementation

Create `cirq_model.rs` with:

```rust
pub enum Qubit {
    Line(usize),
    Grid(usize, usize),
}

pub enum CirqGate {
    // Single-qubit
    I,                          // Identity
    H,                          // Hadamard
    X, Y, Z,                   // Pauli gates
    S, T,                       // Phase gates
    Rx(f64), Ry(f64), Rz(f64), // Rotation gates
    // Two-qubit
    CNOT,                       // Controlled-NOT
    CZ,                         // Controlled-Z
    SWAP,                       // Qubit swap
    // Three-qubit
    Toffoli,                    // Doubly-controlled X
    // Measurement
    Measure { key: String },    // Projective measurement
}

pub struct CirqOperation {
    pub gate: CirqGate,
    pub qubits: Vec<Qubit>,
}

pub struct CirqMoment {
    pub operations: Vec<CirqOperation>,
}

pub struct CirqCircuit {
    pub qubits: Vec<Qubit>,
    pub moments: Vec<CirqMoment>,
}

pub struct CirqError { pub message: String }

pub fn gate_qubit_count(gate: &CirqGate) -> usize;
pub fn validate_moment(moment: &CirqMoment) -> Result<(), CirqError>;
```

---

### Sprint 2: Structural Equivalence and Display

**Test file:** `phase_cirq_model.rs` (continued)

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 2: STRUCTURAL EQUIVALENCE
// ═══════════════════════════════════════════════════════════════

#[test]
fn cirq_circuits_structurally_equivalent_identical()
// Two identical H-CNOT circuits → equivalent == true

#[test]
fn cirq_circuits_not_equivalent_different_gates()
// H(q0) vs X(q0) → equivalent == false

#[test]
fn cirq_circuits_not_equivalent_different_qubit_order()
// CNOT(q0, q1) vs CNOT(q1, q0) → false (control/target swapped)

#[test]
fn cirq_gate_display_name()
// H.display_name() == "H", Rx(1.57).display_name() == "Rx(1.5708)"

#[test]
fn cirq_circuit_depth()
// Circuit with 3 moments → depth() == 3

#[test]
fn cirq_circuit_gate_count()
// Total operation count across all moments
```

#### GREEN Implementation

Add to `cirq_model.rs`:
- `fn cirq_circuits_equivalent(a: &CirqCircuit, b: &CirqCircuit) -> bool`
- `fn display_name(&self) -> String` on `CirqGate`
- `fn depth(&self) -> usize` and `fn gate_count(&self) -> usize` on `CirqCircuit`

---

### Sprint 3: Kripke-to-Cirq Modal Force Mapping

**File:** `crates/logicaffeine_compile/src/codegen_cirq/kripke_to_cirq.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_kripke_map.rs`

This is the heart of the isomorphism. The `ModalVector.force` field already exists in the AST at `crates/logicaffeine_language/src/ast/logic.rs`.

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 3: THE KRIPKE-QUANTUM ISOMORPHISM
// ═══════════════════════════════════════════════════════════════

#[test]
fn force_half_maps_to_hadamard()
// force: 0.5 → CirqGate::H (superposition = equal possibility)

#[test]
fn force_one_maps_to_pauli_x()
// force: 1.0 → CirqGate::X (deterministic transition = necessity)

#[test]
fn force_graded_maps_to_rx_rotation()
// force: 0.3 → CirqGate::Rx(0.3 * PI) (graded modality = partial rotation)

#[test]
fn force_zero_maps_to_identity()
// force: 0.0 → CirqGate::I (no transition = impossibility)

#[test]
fn always_iff_maps_to_cnot()
// "Always, q1 iff q2" pattern → CirqGate::CNOT (entanglement from invariant biconditional)

#[test]
fn next_operator_creates_new_moment()
// TemporalOperator::Next → new CirqMoment boundary (discrete time slice)

#[test]
fn eventually_maps_to_measurement()
// TemporalOperator::Eventually → CirqGate::Measure (observe whether property holds)
```

#### GREEN Implementation

Create `kripke_to_cirq.rs`:
- `fn map_force_to_gate(force: f32) -> CirqGate`
- `struct KripkeToCirqTranslator` with `world_to_qubit: HashMap<Symbol, usize>`
- Pattern matching on `LogicExpr::Modal { vector, operand }` → `CirqOperation`
- Pattern matching on `LogicExpr::Temporal { operator, body }` → moment boundaries
- World variable `w_i` from `KripkeContext.world_counter` → `LineQubit(i)`

---

### Sprint 4: Full Kripke-to-CirqModel Translation

**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_translate.rs`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 4: FULL KRIPKE → CIRQ TRANSLATION
// ═══════════════════════════════════════════════════════════════

#[test]
fn translate_single_qubit_superposition()
// Kripke expr with force: 0.5 on one world → circuit has H(q0)

#[test]
fn translate_two_qubit_entanglement()
// "Always, q1 iff q2" → circuit has H(q0) then CNOT(q0, q1)

#[test]
fn translate_epr_pair_spec()
// The EPR pair from spec Section 6: superposition → entanglement → measure
// Circuit must have 3 moments with H, CNOT, Measure operations

#[test]
fn translate_preserves_qubit_ordering()
// Spec with 3 qubits: indices assigned in encounter order

#[test]
fn translate_graded_rotation()
// Modal with force: 0.25 → Rx(PI/4) gate appears

#[test]
fn translate_measurement_uses_qubit_name_as_key()
// Measurement key derives from world variable name

#[test]
fn translate_world_declarations_count_qubits()
// N distinct world variables → N declared qubits
```

#### GREEN Implementation

Flesh out `kripke_to_cirq.rs`:
- `fn translate_kripke_to_cirq(expr: &LogicExpr, interner: &Interner) -> Result<CirqCircuit, CirqError>`
- Internal world-to-qubit allocation via `HashMap<Symbol, usize>`
- Pattern matching on Modal, Temporal, BinaryOp(Iff) nodes
- Moment boundary logic driven by `TemporalOperator::Next`

---

## Phase II: Synthesis Engine (Emit & Optimize)

### Sprint 5: Basic Python Emitter

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_emitter.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_emit.rs`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 5: PYTHON CODE GENERATION
// ═══════════════════════════════════════════════════════════════

#[test]
fn emit_imports_cirq()
// Output starts with "import cirq"

#[test]
fn emit_declares_line_qubits()
// 2-qubit circuit → output contains "cirq.LineQubit(0)" and "cirq.LineQubit(1)"

#[test]
fn emit_declares_grid_qubits()
// Grid qubit → output contains "cirq.GridQubit(0, 1)"

#[test]
fn emit_creates_circuit_object()
// Output contains "circuit = cirq.Circuit()"

#[test]
fn emit_hadamard_gate()
// H(q0) → output contains "cirq.H(q[0])"

#[test]
fn emit_pauli_x_gate()
// X(q0) → output contains "cirq.X(q[0])"

#[test]
fn emit_cnot_gate()
// CNOT(q0, q1) → output contains "cirq.CNOT(q[0], q[1])"

#[test]
fn emit_rx_gate_with_angle()
// Rx(1.5708) → output contains "cirq.rx(1.5708)"

#[test]
fn emit_measurement()
// Measure{key:"result"}(q0, q1) → "cirq.measure(q[0], q[1], key='result')"

#[test]
fn emit_moment_separation()
// 2 moments → two circuit.append() calls or explicit cirq.Moment usage

#[test]
fn emit_simulator_invocation()
// Output contains "simulator = cirq.Simulator()" and "simulator.simulate(circuit)"

#[test]
fn emit_print_final_state()
// Output contains "print(result.final_state_vector)"

#[test]
fn emit_empty_circuit()
// Empty circuit → valid Python with import/circuit creation, no append calls
```

#### GREEN Implementation

Create `cirq_emitter.rs`:
- `pub fn emit_cirq_python(circuit: &CirqCircuit) -> String`
- Internal: `emit_imports()`, `emit_qubit_declarations()`, `emit_circuit_body()`, `emit_simulation_footer()`
- Each `CirqGate` variant maps to its Python Cirq syntax string

---

### Sprint 6: EPR Pair End-to-End Emission

**Test file:** `phase_cirq_emit.rs` (continued)

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 6: FULL PROGRAM EMISSION
// ═══════════════════════════════════════════════════════════════

#[test]
fn emit_epr_pair_full_program()
// H-CNOT-Measure circuit → matches spec Section 6 Python output

#[test]
fn emit_epr_pair_is_valid_python_syntax()
// Emit EPR pair → python3 -c "compile(source, '<test>', 'exec')" succeeds
// (skipped if Python not available)

#[test]
fn emit_three_qubit_ghz_state()
// H(q0), CNOT(q0,q1), CNOT(q0,q2) → structurally correct GHZ program

#[test]
fn emit_rotation_circuit()
// Rx, Ry, Rz gates → all angles emitted correctly in Python

#[test]
fn emit_empty_circuit_is_valid()
// Empty circuit still produces parseable Python
```

#### GREEN Implementation

Refine `cirq_emitter.rs` for corner cases, full programs, and the EPR pair reference output from the spec.

---

### Sprint 7: Moment Packing and Depth Optimization

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_model.rs` (optimization functions)
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_optimize.rs`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 7: DEPTH OPTIMIZATION
// ═══════════════════════════════════════════════════════════════

#[test]
fn moment_packing_parallel_single_qubit_gates()
// H(q0) and X(q1) in separate moments → optimize packs into 1 moment

#[test]
fn moment_packing_does_not_merge_conflicting_qubits()
// H(q0) then CNOT(q0,q1) → remain separate (q0 conflict)

#[test]
fn depth_reduction_three_independent_gates()
// 3 single-qubit gates on q0,q1,q2 in 3 moments → optimized to depth 1

#[test]
fn optimization_preserves_gate_sequence()
// Same gates and qubits before/after optimization (different moment grouping)

#[test]
fn optimization_cnot_chain_cannot_be_parallelized()
// CNOT(q0,q1) then CNOT(q1,q2) → must stay separate (q1 shared)
```

#### GREEN Implementation

Add to `cirq_model.rs`:
- `pub fn optimize_depth(circuit: &CirqCircuit) -> CirqCircuit`
- Greedy moment packing: scan operations, assign to earliest moment with no qubit conflict
- Tracks occupied qubits per moment via `HashSet<usize>`

---

### Sprint 8: Pipeline API

**File:** `crates/logicaffeine_compile/src/codegen_cirq/quantum_pipeline.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_pipeline.rs`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 8: PUBLIC PIPELINE API
// ═══════════════════════════════════════════════════════════════

#[test]
fn pipeline_compile_quantum_spec_returns_fol()
// English quantum spec → Kripke FOL string (temporal accessibility present)

#[test]
fn pipeline_translate_to_cirq_model()
// English spec → CirqCircuit (moments and operations present)

#[test]
fn pipeline_emit_python_from_spec()
// English spec → Python string containing "import cirq" and expected gates

#[test]
fn pipeline_epr_pair_e2e()
// EPR pair spec from Section 6 → full Python program

#[test]
fn pipeline_error_on_invalid_spec()
// Unparseable spec → CirqError::ParseError

#[test]
fn pipeline_qubit_allocation_matches_world_count()
// N world variables in spec → N qubits declared in output
```

#### GREEN Implementation

Create `quantum_pipeline.rs`:

```rust
pub enum CirqError {
    ParseError(String),
    TranslationError(String),
    EmitError(String),
    VerificationError(String),
}

pub fn compile_quantum_spec(source: &str) -> Result<String, CirqError>;
pub fn translate_to_cirq_model(source: &str) -> Result<CirqCircuit, CirqError>;
pub fn emit_cirq_from_spec(source: &str) -> Result<String, CirqError>;
```

Wire together: `compile_kripke_with` → `translate_kripke_to_cirq` → `emit_cirq_python`.

---

## Phase III: Rigorous Verification (Z3 Integration)

### Sprint 9: Symbolic Matrix Representation

**File:** `crates/logicaffeine_compile/src/codegen_cirq/cirq_to_verify.rs`
**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_verify.rs`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 9: SYMBOLIC MATRICES
// ═══════════════════════════════════════════════════════════════

#[test]
fn symbolic_hadamard_matrix()
// hadamard_matrix() → 2x2 with entries 1/sqrt(2) encoded as VerifyExpr

#[test]
fn symbolic_pauli_x_matrix()
// pauli_x_matrix() → [[0, 1], [1, 0]] symbolically

#[test]
fn symbolic_cnot_matrix()
// cnot_matrix() → 4x4 CNOT matrix symbolically

#[test]
fn symbolic_identity_matrix()
// identity_matrix(2) → [[1,0],[0,1]]

#[test]
fn symbolic_rx_matrix_at_pi_over_2()
// rx_matrix(PI/2) → correct rotation matrix entries

#[test]
fn symbolic_state_vector_zero()
// initial_state_vector(2) → |00> = [1, 0, 0, 0] for 2-qubit system

#[test]
fn symbolic_matrix_multiply_2x2()
// hadamard * [1, 0] → [1/sqrt(2), 1/sqrt(2)]
```

#### GREEN Implementation

Create `cirq_to_verify.rs`:

```rust
pub struct SymbolicMatrix {
    pub rows: usize,
    pub cols: usize,
    pub entries: Vec<Vec<VerifyExpr>>,
}

pub fn hadamard_matrix() -> SymbolicMatrix;
pub fn pauli_x_matrix() -> SymbolicMatrix;
pub fn cnot_matrix() -> SymbolicMatrix;
pub fn rx_matrix(theta: f64) -> SymbolicMatrix;
pub fn identity_matrix(dim: usize) -> SymbolicMatrix;
pub fn initial_state_vector(n_qubits: usize) -> Vec<VerifyExpr>;
pub fn symbolic_matvec_multiply(mat: &SymbolicMatrix, vec: &[VerifyExpr]) -> Vec<VerifyExpr>;
```

Encoding strategy: amplitudes as `VerifyExpr::Apply("complex", [re, im])` where `re`/`im` are `VerifyExpr::Int` for exact rationals or `VerifyExpr::Apply("sqrt2_inv", [])` for 1/sqrt(2). Z3's Real sort handles these without modifying `VerifyExpr`.

---

### Sprint 10: Tensor Product and Multi-Qubit Composition

**Test file:** `phase_cirq_verify.rs` (continued)

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 10: TENSOR PRODUCTS & CIRCUIT COMPOSITION
// ═══════════════════════════════════════════════════════════════

#[test]
fn tensor_product_2x2_identity()
// tensor_product(H, I_2) → 4x4 matrix H tensor I

#[test]
fn tensor_product_identity_pauli_x()
// tensor_product(I_2, X) → 4x4 matrix I tensor X

#[test]
fn circuit_to_unitary_single_hadamard()
// H(q0) on 2-qubit system → symbolic unitary == H tensor I

#[test]
fn circuit_to_unitary_epr_pair()
// H(q0), CNOT(q0,q1) → symbolic unitary == CNOT * (H tensor I)

#[test]
fn circuit_to_unitary_applies_moments_in_order()
// Moments [M1, M2] → unitary == U_M2 * U_M1 (later moments left-multiply)

#[test]
fn circuit_to_state_epr_pair()
// U * |00> for EPR pair → [1/sqrt(2), 0, 0, 1/sqrt(2)] (Bell state |Phi+>)
```

#### GREEN Implementation

Add to `cirq_to_verify.rs`:
- `pub fn tensor_product(a: &SymbolicMatrix, b: &SymbolicMatrix) -> SymbolicMatrix`
- `pub fn circuit_to_symbolic_unitary(circuit: &CirqCircuit) -> SymbolicMatrix`
- `pub fn apply_circuit_to_state(circuit: &CirqCircuit) -> Vec<VerifyExpr>`
- Internal: `fn gate_to_matrix(gate: &CirqGate) -> SymbolicMatrix`
- Internal: `fn moment_to_unitary(moment: &CirqMoment, n_qubits: usize) -> SymbolicMatrix`

---

### Sprint 11: Z3 Amplitude Checking

**Test file:** `crates/logicaffeine_tests/tests/phase_cirq_z3_verify.rs`
**Feature-gated:** `#[cfg(feature = "verification")]`

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 11: Z3 QUANTUM VERIFICATION
// ═══════════════════════════════════════════════════════════════

#[cfg(feature = "verification")]
mod z3_quantum {

#[test]
fn z3_verify_epr_pair_amplitudes()
// Symbolic EPR state: assert amplitude(|00>) == 1/sqrt(2),
// amplitude(|11>) == 1/sqrt(2), amplitude(|01>) == 0, amplitude(|10>) == 0
// Z3 returns SAT (assertions are consistent)

#[test]
fn z3_verify_hadamard_creates_superposition()
// H|0>: assert |amplitude(|0>)|^2 == 0.5, |amplitude(|1>)|^2 == 0.5
// Z3 returns SAT

#[test]
fn z3_verify_pauli_x_flips_state()
// X|0>: assert final state == |1>
// Z3 returns SAT

#[test]
fn z3_verify_wrong_amplitude_is_unsat()
// H|0>: assert amplitude(|0>) == 1.0 (claiming NO superposition)
// Z3 returns UNSAT (incorrect claim detected)

#[test]
fn z3_verify_entanglement_is_not_separable()
// EPR state: assert separability constraint (tensor product form exists)
// Z3 returns UNSAT (entangled state cannot factor)

#[test]
fn z3_spec_matches_circuit_epr()
// LOGOS spec probability requirements vs synthesized circuit amplitudes
// Z3 confirms semantic equivalence

}
```

#### GREEN Implementation

Add to `cirq_to_verify.rs` and `quantum_pipeline.rs`:
- `pub fn cirq_to_verify_expr(circuit: &CirqCircuit) -> VerifyExpr` (bridge to Z3)
- `pub fn check_amplitude_equivalence(spec: &[VerifyExpr], circuit: &CirqCircuit) -> EquivalenceResult`
- Z3 encoding uses Real sort for amplitude components
- Method: construct `not(spec_amplitudes <-> circuit_amplitudes)`, check satisfiability
- All gated behind `#[cfg(feature = "verification")]`

---

### Sprint 12: Full Pipeline E2E Verification

**Test file:** `phase_cirq_z3_verify.rs` (continued)

#### RED Tests

```rust
// ═══════════════════════════════════════════════════════════════
// SPRINT 12: END-TO-END PIPELINE
// ═══════════════════════════════════════════════════════════════

#[test]
fn e2e_english_to_cirq_python_epr()
// Full pipeline: EPR spec text → Python string with H, CNOT, measure

#[test]
fn e2e_english_to_cirq_python_ghz()
// Full pipeline: GHZ 3-qubit spec → correct Python

#[test]
fn e2e_cirq_python_executes_correctly()
// Emit EPR Python → execute via python3 subprocess → verify state vector output
// Skip if Python/Cirq unavailable

#[cfg(feature = "verification")]
#[test]
fn e2e_spec_verification_confirms_epr()
// English spec → CirqCircuit → symbolic unitary → Z3 amplitude check → equivalent

#[cfg(feature = "verification")]
#[test]
fn e2e_wrong_circuit_fails_verification()
// Spec says entangled but circuit missing CNOT → Z3 detects mismatch

#[test]
fn e2e_roundtrip_model_to_python_and_back()
// Build CirqCircuit → emit Python → parse output for gate names → verify match
```

#### GREEN Implementation

Wire everything in `quantum_pipeline.rs`:
- `pub fn verify_quantum_spec(source: &str) -> Result<VerificationResult, CirqError>`
- `pub fn compile_and_verify_quantum(source: &str) -> Result<(String, VerificationResult), CirqError>`

---

## Sprint Dependency Graph

```
Sprint 1 (IR types)
    |
    v
Sprint 2 (structural equiv + display)
    |
    +------------------+
    |                  |
    v                  v
Sprint 3            Sprint 5
(force mapping)     (Python emitter)
    |                  |
    v                  v
Sprint 4            Sprint 6
(full translate)    (EPR + full emission)
    |     |            |
    |     +------------+
    |           |
    v           v
Sprint 7     Sprint 8
(optimize)   (pipeline API)
    |           |
    +-----------+
         |
         v
    Sprint 9 (symbolic matrices)
         |
         v
    Sprint 10 (tensor products)
         |
         v
    Sprint 11 (Z3 amplitude checking)    [feature = "verification"]
         |
         v
    Sprint 12 (E2E verification)         [feature = "verification"]
```

Sprints 3-4 (translation) and 5-6 (emission) can proceed in parallel.
Sprints 9-12 are sequential — each builds on the previous.

---

## Module Registration

### `codegen_cirq/mod.rs`

```rust
pub mod cirq_model;
pub mod cirq_emitter;
pub mod kripke_to_cirq;
pub mod cirq_to_verify;
pub mod quantum_pipeline;
```

### In `logicaffeine_compile/src/lib.rs`

Add alongside the existing `pub mod codegen_sva;`:

```rust
pub mod codegen_cirq;
```

No feature gate needed — the SVA backend is not feature-gated either. Verification-specific code within `cirq_to_verify.rs` and `quantum_pipeline.rs` uses `#[cfg(feature = "verification")]` internally.

---

## Test Count Summary

| Phase | Sprint | Test File | Tests |
|:---|:---|:---|---:|
| I | 1 | `phase_cirq_model.rs` | 11 |
| I | 2 | `phase_cirq_model.rs` | 6 |
| I | 3 | `phase_cirq_kripke_map.rs` | 7 |
| I | 4 | `phase_cirq_translate.rs` | 7 |
| II | 5 | `phase_cirq_emit.rs` | 13 |
| II | 6 | `phase_cirq_emit.rs` | 5 |
| II | 7 | `phase_cirq_optimize.rs` | 5 |
| II | 8 | `phase_cirq_pipeline.rs` | 6 |
| III | 9 | `phase_cirq_verify.rs` | 7 |
| III | 10 | `phase_cirq_verify.rs` | 6 |
| III | 11 | `phase_cirq_z3_verify.rs` | 6 |
| III | 12 | `phase_cirq_z3_verify.rs` | 6 |
| **Total** | **12 sprints** | **8 test files** | **85** |

---

## Formal Synthesis Example: The EPR Pair (Section 6)

This is the reference circuit that threads through every sprint:

**English Specification:**
```logos
Let q1 : Qubit.
Let q2 : Qubit.
Apply Superposition to q1 with Force: 0.5.
Next, Apply Entanglement(q1, q2).
Measure(q1, q2).
```

**Expected CirqModel:**
```
Moment 0: H(q0)
Moment 1: CNOT(q0, q1)
Moment 2: Measure(q0, q1, key="result")
```

**Expected Python Output:**
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

**Z3 Verification Obligation:**
```
U = CNOT * (H tensor I)
U|00> = 1/sqrt(2) * (|00> + |11>)
Pr(|00>) = 0.5, Pr(|11>) = 0.5, Pr(|01>) = 0, Pr(|10>) = 0
```

Z3 confirms: `not(spec_amplitudes <-> circuit_amplitudes)` is UNSAT. QED.

---

## Existing Infrastructure We Reuse

| Component | Location | Reuse |
|:---|:---|:---|
| Kripke lowering | `semantics/kripke.rs` | World variables, temporal operators, modal forces |
| VerifyExpr IR | `logicaffeine_verify/src/ir.rs` | `Apply()` for complex amplitudes, `Binary()` for constraints |
| Z3 equivalence | `logicaffeine_verify/src/equivalence.rs` | `check_equivalence()` pattern for amplitude checking |
| Bounded translation pattern | `codegen_sva/sva_to_verify.rs` | `SvaTranslator` pattern for `CirqTranslator` |
| Pipeline API pattern | `codegen_sva/hw_pipeline.rs` | `compile_hw_spec()` pattern for `compile_quantum_spec()` |
| compile_kripke_with | `logicaffeine_language` | Entry point for spec → AST with Kripke lowering |
