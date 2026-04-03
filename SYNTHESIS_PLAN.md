# SYNTHESIS PLAN

Engineering specification for LogicAffeine's hardware synthesis solver. Every claim verified against source code. Every weakness disclosed. Every sprint has concrete RED tests. Designed for handoff to an implementing programmer.

The core contribution: under Curry-Howard, a hardware specification encoded as a dependent type in the CoIC kernel IS a synthesis problem. A proof term inhabiting that type IS a correct-by-construction circuit. Extract the proof term to Verilog.

---

## Part I: The Competitive Landscape

### What Existing Synthesis Tools Do

| Tool | Approach | Limitation |
|---|---|---|
| Bluespec | Guarded atomic actions -> Verilog | No formal spec language. Programmer writes the circuit. |
| Clash | Haskell -> Verilog | No specification-driven synthesis. Programmer writes the circuit. |
| Chisel/FIRRTL | Scala DSL -> Verilog | Generator-based, not spec-based. Programmer writes the circuit. |
| High-Level Synthesis (Vivado/Catapult) | C/C++ behavioral -> RTL | Untrusted translation. No formal correctness proof. |
| Kami (MIT) | Coq -> Verilog | Closest to us. Coq proof terms extracted to Bluespec. Requires Coq expertise. |
| Cava (Google) | Coq -> SystemVerilog | Circuit combinators in Coq. Still requires manual proof construction. |
| Lava/Clash | Haskell types -> circuits | Descriptive, not synthesizing from specifications. |
| **All of them** | Programmer describes the circuit in a programming language. | None synthesize from natural language specification via proof search. |

### What Nobody Does

- Synthesize hardware from English specifications with a formal correctness certificate.
- Use tactic-driven proof search to construct circuit implementations.
- Certify synthesized circuits through a type-theoretic kernel (soundness firewall).
- Generate Verilog from proof terms where the type IS the specification.
- Apply CEGAR refinement with both kernel type errors and Z3 counterexamples.

**These are our contributions.**

### What LogicAffeine Has (Audited, 2026-04-02)

**Three subsystems that form the triangle:**

| Subsystem | Evidence | Tests |
|---|---|---|
| **CoIC Kernel** — CIC with Pi, Lambda, App, Match, Fix, inductive types, universe hierarchy | Type checking, normalization, termination, positivity | ~200 |
| Decision procedures: ring, lia, omega, cc, simp | Polynomial equality, Fourier-Motzkin, congruence closure, rewriting | ~100 |
| Reflective tactics: try_ring, try_lia, try_cc, try_simp, try_omega, try_auto, try_induction | Tactic combinators: tact_first, tact_orelse, tact_then, tact_repeat | ~50 |
| **Proof Engine** — 14+ backward-chaining strategies, Robinson unification | Curry-Howard certification: DerivationTree -> kernel Term | ~150 |
| Z3 oracle for arithmetic/comparison goals | Delegation to Z3 when tactics fail | ~20 |
| **Hardware SVA Pipeline** — English -> Kripke FOL -> BoundedExpr <-> SVA -> Z3 equivalence | Full pipeline with 28 HwEntityType + 24 HwRelation variants | 289 |
| CEGAR refinement (3 transforms), protocol templates (AXI4/APB/UART/SPI/I2C) | Coverage, sufficiency, consistency, invariant discovery | 243 |
| **Total verified baseline** | **532+ hardware verification tests** | |

**Known gaps (this spec fills):**

| Gap | Location | Sprint |
|---|---|---|
| No hardware types in kernel (Bit, BitVec, Circuit) | `kernel/prelude.rs` has Bool/Nat/TList but no hardware primitives | 0A-0D |
| No hardware-specific tactics (bitblast, tabulate) | `kernel/reduction.rs` has ring/lia/cc but no bitvector decision procedures | 1A-1D |
| No spec encoding bridge (VerifyExpr -> kernel Term) | Pipeline stops at VerifyExpr — never enters the kernel | 2A |
| No Verilog extraction from kernel terms | `extraction/codegen.rs` emits Rust — no Verilog target | 3A-3B |
| No Z3 synthesis oracle (model -> kernel Term) | Z3 verifies but never synthesizes | 4A-4B |
| No synthesis loop connecting all three subsystems | CEGAR loop operates on SVA strings, not kernel terms | 5A-5C |

---

## Part II: The Honest Comparison

| Capability | Kami (MIT/Coq) | Cava (Google/Coq) | HLS (Vivado) | LogicAffeine (now) | LogicAffeine (after) |
|---|---|---|---|---|---|
| **Spec language** | Coq tactics | Coq combinators | C/C++ | English | **English** |
| **Formal proof** | Coq kernel | Coq kernel | None | None | **CoIC kernel** |
| **Synthesis method** | Manual proof | Manual combinators | Behavioral scheduling | Manual | **Tactic + Z3 oracle** |
| **Spec <-> impl proof** | Coq type check | Coq type check | None | Z3 SVA equiv | **Kernel type check + Z3** |
| **Extraction target** | Bluespec -> Verilog | SystemVerilog | Verilog | SVA only | **Verilog + SVA** |
| **Bitvector reasoning** | Manual | Bvector library | Native | Z3 bitvector | **Kernel + Z3 bitvector** |
| **Automation** | Ltac, omega | Ltac | Full | None | **try_bitblast, try_tabulate, Z3 oracle** |
| **CEGAR refinement** | No | No | No | SVA string transforms | **Kernel + Z3 counterexamples** |
| **Accessibility** | PhD in type theory | PhD in type theory | C programmer | English speaker | **English speaker** |
| **Soundness firewall** | Coq kernel | Coq kernel | None | None | **CoIC kernel (infer_type)** |

---

## Part III: Architecture

### The Synthesis Triangle

```
                    English Specification
                           |
                   [logicaffeine_language]
                   parse + Kripke lower
                           |
                    Kripke FOL + KG
                    /              \
                   /                \
    [verify_to_kernel.rs]    [existing SVA pipeline]
    encode as kernel Type     synthesize SVA
                |                    |
         Spec Type (Term)      SVA Properties
                |                    |
         Proof Search           Z3 Equivalence
     [reduction.rs tactics]      [equivalence.rs]
                |                    |
         Proof Term             Counterexample
                |                    |
    [infer_type: SOUNDNESS     CEGAR Feedback
         FIREWALL]                   |
                |              ─────/
         extract_verilog
       [extraction/verilog.rs]
                |
         SystemVerilog Module
```

### File Structure

```
crates/logicaffeine_kernel/src/
  prelude.rs              — ADD: register_hardware() (Bit, BVec, Circuit, gate ops)
  reduction.rs            — ADD: try_bitblast, try_tabulate, try_hw_auto dispatch

crates/logicaffeine_proof/src/
  hw_oracle.rs            — NEW: Z3 model -> kernel Term extraction [verification]

crates/logicaffeine_compile/src/
  codegen_sva/
    verify_to_kernel.rs   — NEW: VerifyExpr -> kernel Term spec encoding
    z3_synth.rs           — NEW: kernel Term -> Z3 synthesis constraint [verification]
    cegar.rs              — NEW: full CEGAR synthesis loop [verification]
    synthesize.rs         — NEW: top-level synthesis API [verification]
    synthesis_refine.rs   — MODIFY: add 3 new refinement transforms
    mod.rs                — MODIFY: register new modules
  extraction/
    verilog.rs            — NEW: kernel Term -> SystemVerilog extraction
    mod.rs                — MODIFY: add extract_verilog() API

crates/logicaffeine_tests/tests/
  phase_hw_synthesis.rs   — NEW: all synthesis tests
```

### Test Files

```
crates/logicaffeine_tests/tests/
  phase_hw_synthesis.rs   — Sprints 0-5: all synthesis tests (~140 tests)
    Section 0: Kernel hardware types (Bit, BVec, Circuit, gate ops)
    Section 1: Hardware tactics (bitblast, tabulate, hw_auto)
    Section 2: Spec encoding (VerifyExpr -> kernel Term round-trip)
    Section 3: Verilog extraction (kernel Term -> SystemVerilog)
    Section 4: Z3 synthesis oracle (model extraction + certification) [verification]
    Section 5: CEGAR loop + E2E integration [verification]
```

---

## Part IV: Sprint Specification

Every sprint follows TDD: write RED tests first, implement until GREEN, run full suite for zero regressions.

```bash
# RED: confirm new tests fail
cargo test --no-fail-fast --test phase_hw_synthesis -- --skip e2e > /tmp/red.txt 2>&1; echo "EXIT: $?" >> /tmp/red.txt

# GREEN: implement until pass
cargo test --no-fail-fast --test phase_hw_synthesis -- --skip e2e > /tmp/green.txt 2>&1; echo "EXIT: $?" >> /tmp/green.txt

# REGRESSION: all tests pass
cargo test --no-fail-fast -- --skip e2e > /tmp/all.txt 2>&1; echo "EXIT: $?" >> /tmp/all.txt

# Z3: feature-gated tests
cargo test --features verification --no-fail-fast --test phase_hw_synthesis -- --skip e2e > /tmp/z3.txt 2>&1; echo "EXIT: $?" >> /tmp/z3.txt
```

---

### Sprint 0A: Bit and Unit Inductive Types

**Why:** The kernel has Bool/Nat/TList but no hardware primitives. Every synthesis operation begins with a Bit. Every stateless circuit requires Unit. Without these, no hardware term can be constructed or type-checked.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`

**What:** Add `register_hardware()` called from `StandardLibrary::register` (line 32). Register `Bit : Type 0` with constructors `B0 : Bit` and `B1 : Bit`. Register `Unit : Type 0` with constructor `Tt : Unit`. Follows `register_bool` pattern exactly (lines 228-239).

**RED tests (section 0 of `phase_hw_synthesis.rs`):**

| Test | Assertion |
|---|---|
| `hw_bit_is_type_zero` | `Check Bit.` -> `Type0` |
| `hw_b0_has_type_bit` | `Check B0.` -> `Bit` |
| `hw_b1_has_type_bit` | `Check B1.` -> `Bit` |
| `hw_b0_not_equal_b1` | `B0` and `B1` are distinct after normalization |
| `hw_unit_is_type_zero` | `Check Unit.` -> `Type0` |
| `hw_tt_has_type_unit` | `Check Tt.` -> `Unit` |
| `hw_bit_has_two_constructors` | Pattern match on Bit with exactly 2 cases type-checks |

---

### Sprint 0B: BVec Indexed Inductive Type

**Why:** Hardware operates on multi-bit values. BVec is a length-indexed bitvector — `BVec (Succ (Succ Zero))` is provably 2 bits wide. Width mismatches are type errors, caught at proof construction time, not runtime.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`

**What:** Register `BVec : Nat -> Type 0` with constructors `BVNil : BVec Zero` and `BVCons : Bit -> Pi(n:Nat). BVec n -> BVec (Succ n)`. Follows `register_tlist` pattern (lines 244-288) but indexed by Nat instead of parameterized by type.

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_bvec_is_nat_to_type` | `Check BVec.` -> `Nat -> Type0` |
| `hw_bvnil_is_bvec_zero` | `Check BVNil.` -> `BVec Zero` |
| `hw_bvcons_b1_bvnil_is_bvec_one` | `BVCons B1 Zero BVNil` type-checks as `BVec (Succ Zero)` |
| `hw_bvec_two_bits_type_correct` | `BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil)` -> `BVec (Succ (Succ Zero))` |
| `hw_bvec_width_mismatch_rejected` | Applying BVCons with wrong Nat index -> type error |
| `hw_bvec_match_exhaustive` | Pattern match on BVec with BVNil + BVCons cases type-checks |

---

### Sprint 0C: Gate Operation Definitions

**Why:** Gate operations must be transparent definitions (not axioms) so the kernel normalizer can evaluate them. `bit_and B1 B0` must reduce to `B0` via the existing iota reduction rule (reduction.rs lines 145-172) without any new reduction machinery.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`

**What:** Register five definitions via `ctx.add_definition()`:
- `bit_and : Bit -> Bit -> Bit` — match on first arg: B0 -> B0, B1 -> second arg
- `bit_or : Bit -> Bit -> Bit` — match on first arg: B0 -> second arg, B1 -> B1
- `bit_not : Bit -> Bit` — match: B0 -> B1, B1 -> B0
- `bit_xor : Bit -> Bit -> Bit` — match on first: B0 -> bit_not(second), B1 -> second
- `bit_mux : Bit -> Bit -> Bit -> Bit` — match on selector: B0 -> third, B1 -> second

Follows `Not` definition pattern (lines 379-387 of prelude.rs).

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_bit_and_b0_b0` | `Eval (bit_and B0 B0).` -> `B0` |
| `hw_bit_and_b0_b1` | `Eval (bit_and B0 B1).` -> `B0` |
| `hw_bit_and_b1_b0` | `Eval (bit_and B1 B0).` -> `B0` |
| `hw_bit_and_b1_b1` | `Eval (bit_and B1 B1).` -> `B1` |
| `hw_bit_or_b0_b0` | `Eval (bit_or B0 B0).` -> `B0` |
| `hw_bit_or_b0_b1` | `Eval (bit_or B0 B1).` -> `B1` |
| `hw_bit_or_b1_b0` | `Eval (bit_or B1 B0).` -> `B1` |
| `hw_bit_or_b1_b1` | `Eval (bit_or B1 B1).` -> `B1` |
| `hw_bit_not_b0` | `Eval (bit_not B0).` -> `B1` |
| `hw_bit_not_b1` | `Eval (bit_not B1).` -> `B0` |
| `hw_bit_xor_b0_b0` | `Eval (bit_xor B0 B0).` -> `B0` |
| `hw_bit_xor_b0_b1` | `Eval (bit_xor B0 B1).` -> `B1` |
| `hw_bit_xor_b1_b0` | `Eval (bit_xor B1 B0).` -> `B1` |
| `hw_bit_xor_b1_b1` | `Eval (bit_xor B1 B1).` -> `B0` |
| `hw_bit_mux_b0_selects_second` | `Eval (bit_mux B0 B1 B0).` -> `B0` (selects third/else) |
| `hw_bit_mux_b1_selects_first` | `Eval (bit_mux B1 B1 B0).` -> `B1` (selects second/then) |
| `hw_gate_ops_type_correct` | `Check bit_and.` -> `Bit -> Bit -> Bit` |
| `hw_gate_composition_normalizes` | `Eval (bit_and (bit_or B1 B0) (bit_not B0)).` -> `B1` |
| `hw_demorgan_and` | `bit_not (bit_and a b)` normalizes same as `bit_or (bit_not a) (bit_not b)` for all inputs |
| `hw_demorgan_or` | `bit_not (bit_or a b)` normalizes same as `bit_and (bit_not a) (bit_not b)` for all inputs |

---

### Sprint 0D: Circuit (Mealy Machine) Inductive Type

**Why:** A circuit is a state machine. `Circuit S I O` encodes a Mealy machine with state type S, input type I, output type O. The single constructor `MkCircuit` bundles a transition function, output function, and initial state. `Circuit Unit I O` is purely combinational. This is the type that, when inhabited by a proof term, IS a correct circuit.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`

**What:** Register `Circuit : Type 0 -> Type 0 -> Type 0 -> Type 0` with constructor `MkCircuit : Pi(S:Type 0). Pi(I:Type 0). Pi(O:Type 0). (S -> I -> S) -> (S -> I -> O) -> S -> Circuit S I O`.

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_circuit_is_type_constructor` | `Check Circuit.` -> `Type0 -> Type0 -> Type0 -> Type0` |
| `hw_circuit_unit_bit_bit` | `Circuit Unit Bit Bit` type-checks as `Type0` |
| `hw_mkcircuit_identity` | `MkCircuit Unit Bit Bit (λs.λi.s) (λs.λi.i) Tt` type-checks as `Circuit Unit Bit Bit` |
| `hw_mkcircuit_inverter` | `MkCircuit Unit Bit Bit (λs.λi.s) (λs.λi.bit_not i) Tt` type-checks |
| `hw_mkcircuit_and_gate` | Combinational AND gate type-checks as `Circuit Unit (BVec 2) Bit` |
| `hw_mkcircuit_sequential` | Circuit with `Bit` state type-checks as `Circuit Bit Bit Bit` |
| `hw_circuit_wrong_types_rejected` | Transition function with wrong types -> kernel type error |

---

### Sprint 0E: BVec Recursive Operations

**Depends on:** 0B, 0C

**Why:** Hardware needs bitwise operations on multi-bit values. These are recursive Fix definitions with structural termination on BVec — the kernel's termination checker verifies them automatically.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`

**What:** Register `bv_and`, `bv_or`, `bv_not`, `bv_xor` as Fix definitions. Each recurses on BVec structure: base case BVNil, recursive case BVCons applies per-bit gate then recurses on tail.

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_bv_and_two_bits` | `bv_and [B1,B0] [B1,B1]` normalizes to `[B1,B0]` |
| `hw_bv_or_two_bits` | `bv_or [B1,B0] [B0,B1]` normalizes to `[B1,B1]` |
| `hw_bv_not_two_bits` | `bv_not [B1,B0]` normalizes to `[B0,B1]` |
| `hw_bv_xor_two_bits` | `bv_xor [B1,B1] [B1,B0]` normalizes to `[B0,B1]` |
| `hw_bv_and_empty` | `bv_and BVNil BVNil` normalizes to `BVNil` |
| `hw_bv_ops_preserve_width` | `bv_and (BVec 3) (BVec 3)` type-checks as `BVec 3` |
| `hw_bv_and_type_correct` | `Check bv_and.` includes `BVec` in type |

---

### Sprint 1A: try_bitblast Tactic

**Depends on:** 0A-0E

**Why:** Bitvector equality is the bread and butter of hardware verification. `try_bitblast` decomposes `Eq (BVec n) v1 v2` into n independent `Eq Bit` goals, each solvable by normalization. This is the hardware analog of `try_ring` for polynomial equality.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs` (registration), `crates/logicaffeine_kernel/src/reduction.rs` (implementation)

**What:**
1. Register `DBitblastSolve : Syntax -> Derivation` constructor and `try_bitblast : Syntax -> Derivation` declaration in prelude.rs. Pattern: lines 1793-1854 (register_try_ring).
2. Add dispatch case `"try_bitblast" => try_try_bitblast_reduce(ctx, &norm_arg)` in reduction.rs at line ~401 (try_reflection_reduce dispatch table).
3. Implementation: normalize goal, `extract_eq` to get (BVec n, left, right), extract concrete n, for each bit position normalize and check structural equality. Return `DBitblastSolve(goal)` or `make_error_derivation()`.

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_bitblast_proves_bvec_eq` | `try_bitblast (Eq (BVec 2) v1 v2)` succeeds when v1==v2 |
| `hw_bitblast_refutes_bvec_neq` | `try_bitblast (Eq (BVec 2) [B1,B0] [B0,B1])` produces error derivation |
| `hw_bitblast_works_with_bv_and` | `try_bitblast (Eq (BVec 2) (bv_and x y) expected)` with concrete x,y |
| `hw_bitblast_single_bit` | `try_bitblast (Eq (BVec 1) [B1] [B1])` succeeds |
| `hw_bitblast_concludes_matches` | `concludes (try_bitblast goal)` equals `goal` |
| `hw_bitblast_wrong_type_fails` | `try_bitblast (Eq Nat x y)` produces error (not BVec) |

---

### Sprint 1B: try_tabulate Tactic

**Depends on:** 0A-0C

**Why:** For specifications over small finite domains (Bit inputs), exhaustive enumeration IS synthesis. `try_tabulate` enumerates all 2^n input combinations for a Pi-typed goal over Bit, verifies each by normalization, and produces a Match tree that IS the synthesized circuit. This is the constructive content of proof by exhaustion.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`, `crates/logicaffeine_kernel/src/reduction.rs`

**What:** Register `DTabulateSolve` constructor and `try_tabulate` declaration. Implementation: detect leading Bit-typed Pi binders, enumerate 2^n B0/B1 combinations, substitute each into body, normalize, verify. Produce nested Lambda + Match proof term.

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_tabulate_proves_bit_identity` | `Pi(a:Bit). Eq Bit a a` -> succeeds |
| `hw_tabulate_proves_double_neg` | `Pi(a:Bit). Eq Bit (bit_not (bit_not a)) a` -> succeeds |
| `hw_tabulate_proves_and_comm` | `Pi(a:Bit). Pi(b:Bit). Eq Bit (bit_and a b) (bit_and b a)` -> succeeds |
| `hw_tabulate_proves_demorgan` | `Pi(a:Bit). Pi(b:Bit). Eq Bit (bit_not (bit_and a b)) (bit_or (bit_not a) (bit_not b))` -> succeeds |
| `hw_tabulate_refutes_false_claim` | `Pi(a:Bit). Eq Bit a B0` -> error derivation |
| `hw_tabulate_three_inputs` | `Pi(a:Bit). Pi(b:Bit). Pi(c:Bit). P(a,b,c)` with valid P -> succeeds (8 cases) |
| `hw_tabulate_concludes_matches` | `concludes` of result equals original goal |
| `hw_tabulate_ignores_non_bit` | `Pi(n:Nat). P(n)` -> error (not finite Bit domain) |

---

### Sprint 1C: try_hw_auto Composite Tactic

**Depends on:** 1A, 1B

**Why:** A single entry point that tries all hardware tactics in order: fast normalization first, then bitblasting, then exhaustive tabulation, then fallback to general auto.

**Files:** `crates/logicaffeine_kernel/src/prelude.rs`, `crates/logicaffeine_kernel/src/reduction.rs`

**What:** Register `try_hw_auto : Syntax -> Derivation`. Implementation: `try_compute`, then `try_bitblast`, then `try_tabulate`, then `try_auto`. Return first success.

**RED tests:**

| Test | Assertion |
|---|---|
| `hw_auto_solves_simple_eq` | `Eq Bit B1 B1` -> via try_compute |
| `hw_auto_solves_bvec_eq` | `Eq (BVec 2) v1 v2` -> via try_bitblast |
| `hw_auto_solves_universal` | `Pi(a:Bit). Eq Bit (bit_not (bit_not a)) a` -> via try_tabulate |
| `hw_auto_falls_through_to_auto` | Arithmetic goal -> delegates to try_auto |

---

### Sprint 2A: Spec Encoding (VerifyExpr -> Kernel Type)

**Depends on:** 0A-0D

**Why:** The pipeline currently stops at VerifyExpr (Z3-ready IR). To enter the kernel's type-theoretic world, we need a translator. Each VerifyExpr maps to a kernel Term encoding the property as a type. This is the Curry-Howard bridge.

**New file:** `crates/logicaffeine_compile/src/codegen_sva/verify_to_kernel.rs`
**Modify:** `crates/logicaffeine_compile/src/codegen_sva/mod.rs` (add `pub mod verify_to_kernel;`)

**What:** `encode_verify_expr(expr) -> Term` translates VerifyExpr nodes to kernel Term nodes. `encode_full_spec(kg, invariants, signal_map) -> Term` conjoins all KG-discovered invariants as a single specification type.

**Translation rules:**

| VerifyExpr | Kernel Term | Rationale |
|---|---|---|
| `Var("sig@t")` | `App(Global("sig"), nat_literal(t))` | Signal as function of time |
| `Binary(And, l, r)` | `App(App(Global("And"), l'), r')` | Logical conjunction |
| `Binary(Or, l, r)` | `App(App(Global("Or"), l'), r')` | Logical disjunction |
| `Binary(Implies, l, r)` | `Pi("_", l', r')` | Implication as function type |
| `Not(e)` | `App(Global("Not"), e')` | Negation |
| `Bool(true)` | `Global("True")` | Propositional truth |
| `Bool(false)` | `Global("False")` | Propositional falsity |
| `Binary(Eq, l, r)` | `App(App(App(Global("Eq"), type), l'), r')` | Propositional equality |
| `Int(n)` | `nat_literal(n)` via Zero/Succ | Church numeral encoding |
| `ForAll(vars, body)` | `Pi(var, type, body')` | Universal quantification |
| `BitVecBinary(op, l, r)` | `App(App(Global("bv_"+op), l'), r')` | Bitvector operations |

**Reuse:** `fol_to_verify.rs` (existing BoundedExpr translator), `invariants.rs` (existing `discover_invariants` returns `Vec<CandidateInvariant>` with `expr: VerifyExpr`).

**RED tests:**

| Test | Assertion |
|---|---|
| `encode_bool_true` | `VerifyExpr::Bool(true)` -> `Term::Global("True")` |
| `encode_bool_false` | `VerifyExpr::Bool(false)` -> `Term::Global("False")` |
| `encode_var_signal_at_time` | `VerifyExpr::Var("req@0")` -> `App(Global("req"), Zero)` |
| `encode_and` | `Binary(And, P, Q)` -> `App(App(Global("And"), P', Q'))` |
| `encode_implies_as_pi` | `Binary(Implies, P, Q)` -> `Pi("_", P', Q')` |
| `encode_not` | `Not(P)` -> `App(Global("Not"), P')` |
| `encode_eq` | `Binary(Eq, x, y)` -> `App(App(App(Global("Eq"), type), x'), y')` |
| `encode_int_as_nat` | `Int(3)` -> `Succ(Succ(Succ(Zero)))` |
| `encode_full_spec_produces_conjunction` | Multiple invariants -> nested And type |
| `encoded_type_passes_kernel_check` | Encoded combinational spec -> `infer_type` succeeds |
| `round_trip_english_to_kernel_type` | English -> FOL -> VerifyExpr -> Term -> type-checks |

---

### Sprint 3A: Verilog Extraction

**Depends on:** 0A-0D

**Why:** A kernel proof term that type-checks against a hardware spec IS a correct circuit — but it's useless until we extract it to Verilog. This sprint bridges the gap between the mathematical world (kernel terms) and the physical world (silicon).

**New file:** `crates/logicaffeine_compile/src/extraction/verilog.rs`
**Modify:** `crates/logicaffeine_compile/src/extraction/mod.rs` (add `extract_verilog()`)

**What:** `VerilogGen` struct (parallel to existing `CodeGen` in codegen.rs, 605 lines). `term_to_verilog` pattern-matches Term variants into SystemVerilog syntax.

**Extraction rules:**

| Kernel Term | SystemVerilog | Notes |
|---|---|---|
| `Bit` | `logic` | 1-bit wire |
| `BVec n` | `logic [n-1:0]` | Multi-bit bus |
| `App(App(Global("bit_and"), a), b)` | `a & b` | Bitwise AND |
| `App(App(Global("bit_or"), a), b)` | `a \| b` | Bitwise OR |
| `App(Global("bit_not"), a)` | `~a` | Bitwise NOT |
| `App(App(Global("bit_xor"), a), b)` | `a ^ b` | Bitwise XOR |
| `Match disc { B0 => e0, B1 => e1 }` | `disc ? e1 : e0` | Ternary mux |
| `Lambda param:Bit. body` | Port declaration | Input/output |
| `MkCircuit S I O trans out init` | Full module | always_ff + always_comb |
| `Global("B0")` | `1'b0` | Constant low |
| `Global("B1")` | `1'b1` | Constant high |

**RED tests:**

| Test | Assertion |
|---|---|
| `verilog_bit_and` | `bit_and` term extracts to `a & b` |
| `verilog_bit_or` | `bit_or` term extracts to `a \| b` |
| `verilog_bit_not` | `bit_not` term extracts to `~a` |
| `verilog_bit_xor` | `bit_xor` term extracts to `a ^ b` |
| `verilog_match_to_ternary` | Match on Bit -> ternary operator |
| `verilog_nested_match` | Nested Match -> nested ternary |
| `verilog_constants` | `B0` -> `1'b0`, `B1` -> `1'b1` |
| `verilog_combinational_module` | `Circuit Unit Bit Bit` -> module without state register |
| `verilog_sequential_module` | `Circuit Bit Bit Bit` -> module with always_ff + always_comb |
| `verilog_module_has_clk_rst` | Sequential module has clk and rst ports |
| `verilog_initial_state` | Init term -> reset value in always_ff |
| `verilog_round_trip_parses` | Extracted Verilog -> `rtl_extract::parse_verilog_module` succeeds |
| `verilog_signals_match_spec` | Round-trip: extracted signals match original spec signals |

---

### Sprint 4A: Z3 Synthesis Constraint Builder

**Depends on:** 2A

**Feature gate:** `#[cfg(feature = "verification")]`

**Why:** When tactics can't construct a proof term directly, we ask Z3 to find one. Z3's bitvector theory is decidable — given a spec, it can find a satisfying function (the circuit) or prove none exists.

**New file:** `crates/logicaffeine_compile/src/codegen_sva/z3_synth.rs`

**What:** `build_synthesis_constraint(spec_type) -> VerifyExpr` converts a kernel spec type into a Z3 existential query. Reuses existing `VerifyExpr` builders (ir.rs lines 237-548) and `BitVecOp` enum (ir.rs lines 99-128).

**RED tests:**

| Test | Assertion |
|---|---|
| `synth_and_gate_from_spec` | Spec `Pi(a:Bit).Pi(b:Bit).Eq Bit (f a b) (bit_and a b)` -> Z3 finds model |
| `synth_or_gate_from_spec` | OR gate spec -> Z3 finds model |
| `synth_xor_gate_from_spec` | XOR gate spec -> Z3 finds model |
| `synth_mux_from_spec` | 2:1 MUX spec -> Z3 finds model |
| `synth_unrealizable_detected` | `Pi(a:Bit). And (Eq Bit a B0) (Eq Bit a B1)` -> UNSAT |
| `synth_constraint_uses_bitvec_ops` | Generated constraint contains `BitVecBinary` variants |

---

### Sprint 4B: Z3 Model Extraction to Kernel Terms

**Depends on:** 4A

**Feature gate:** `#[cfg(feature = "verification")]`

**Why:** Z3 produces a model (truth table). We need to convert it back to a kernel Term that type-checks against the spec. This is where synthesis becomes certified — the extracted term must pass `infer_type`.

**New file:** `crates/logicaffeine_proof/src/hw_oracle.rs`

**What:** `extract_circuit_from_model(model, spec_type) -> Term` converts Z3 model to nested Lambda + Match tree. `certify_synthesis(ctx, candidate, spec_type) -> KernelResult<()>` calls `infer_type` as soundness firewall.

**RED tests:**

| Test | Assertion |
|---|---|
| `oracle_and_gate_type_checks` | Z3 model for AND -> kernel Term -> `infer_type` passes |
| `oracle_4bit_adder_type_checks` | Z3 model for 4-bit adder -> `infer_type` passes |
| `oracle_toggle_fsm_type_checks` | Z3 model for toggle flip-flop -> `MkCircuit` -> `infer_type` passes |
| `oracle_extracts_correct_truth_table` | AND gate model: (B0,B0)->B0, (B0,B1)->B0, (B1,B0)->B0, (B1,B1)->B1 |
| `oracle_sequential_has_init_trans_out` | FSM model extracts three components for `MkCircuit` |
| `oracle_rejects_invalid_model` | Corrupted model -> `infer_type` catches it (soundness firewall) |
| `oracle_timeout_returns_none` | 32-bit spec -> Z3 timeout -> graceful `None` |

---

### Sprint 5A: CEGAR Synthesis Loop

**Depends on:** 1C, 4B

**Feature gate:** `#[cfg(feature = "verification")]`

**Why:** Neither tactics alone nor Z3 alone solve everything. The CEGAR loop combines them: try tactics first (fast, certified), fall back to Z3 oracle (powerful, needs certification), refine on counterexamples. This is the full synthesis solver.

**New file:** `crates/logicaffeine_compile/src/codegen_sva/cegar.rs`

**What:** `synthesis_loop(ctx, spec_type, config) -> SynthesisResult`. Steps: (1) try_hw_auto, (2) try Z3 oracle, (3) certify via infer_type, (4) extract Verilog, (5) belt-and-suspenders Z3 equivalence check, (6) refine on counterexample if divergent.

**RED tests:**

| Test | Assertion |
|---|---|
| `cegar_and_gate_converges` | AND gate spec -> converges in 1 iteration |
| `cegar_tactic_path` | Simple spec solved by try_tabulate without Z3 |
| `cegar_oracle_path` | Complex spec requires Z3 oracle |
| `cegar_unrealizable_detected` | Contradictory spec -> `VerificationStatus::Unrealizable` |
| `cegar_bounded_correct` | Sequential spec -> `BoundedCorrect(k)` with bound k |
| `cegar_belt_and_suspenders` | Extracted Verilog matches spec via Z3 equivalence |
| `cegar_max_iterations_respected` | Divergent spec -> stops at max_iterations |

---

### Sprint 5B: Refinement Transforms

**Depends on:** 5A

**Modify:** `crates/logicaffeine_compile/src/codegen_sva/synthesis_refine.rs`

**What:** Add to existing 3 transforms (weaken_implication, strengthen_implication, weaken_to_eventual):
- `add_counterexample_constraint(spec, trace)` — concretize failing input as Pi obligation
- `split_temporal(spec, k)` — decompose unbounded property into k bounded sub-properties
- `decompose_conjunctive(spec)` — reuse existing `decompose.rs`

**RED tests:**

| Test | Assertion |
|---|---|
| `refine_add_counterexample` | Trace with (B1,B0)->B1 added as constraint |
| `refine_split_temporal` | Unbounded spec -> k bounded sub-specs |
| `refine_decompose_uses_existing` | Conjunction split matches `decompose_conjunctive` |
| `refine_counterexample_narrows_search` | After adding CE constraint, Z3 finds different model |

---

### Sprint 5C: Top-Level Synthesis API

**Depends on:** 2A, 3A, 5A

**New file:** `crates/logicaffeine_compile/src/codegen_sva/synthesize.rs`
**Modify:** `crates/logicaffeine_compile/src/codegen_sva/mod.rs`

**What:** Single entry point:

```rust
pub fn synthesize_from_spec(
    spec: &str,
    config: &SynthesisConfig,
) -> Result<SynthesisResult, SynthesisError>;
```

Orchestrates: `extract_kg` -> `compile_hw_property` -> `encode_full_spec` -> CEGAR loop -> `extract_verilog` -> Z3 equivalence.

**RED tests (end-to-end):**

| Test | Assertion |
|---|---|
| `e2e_and_gate_english_to_verilog` | "Output equals input A and input B" -> valid Verilog with `&` |
| `e2e_inverter_english_to_verilog` | "Output is the negation of input" -> Verilog with `~` |
| `e2e_handshake_english_to_verilog` | "If request rises, acknowledge within 3 cycles" -> sequential Verilog |
| `e2e_verilog_round_trip_equivalent` | Extracted Verilog -> `rtl_extract` parse -> Z3 equiv with original spec |
| `e2e_proof_term_type_checks` | Every SynthesisResult has a proof term that passes `infer_type` |
| `e2e_unrealizable_spec_detected` | "Output is both high and low" -> `Unrealizable` |
| `e2e_result_has_sva_properties` | Result includes SVA assertions alongside Verilog |

---

## Part V: Sprint Sequencing

```
Sprint 0A (Bit + Unit types)
Sprint 0B (BVec indexed type)        } Phase 0: Kernel Types
Sprint 0C (gate operation definitions) } (no Z3, no feature gates)
Sprint 0D (Circuit type)
Sprint 0E (BVec operations)            [depends: 0B, 0C]
    |
Sprint 1A (try_bitblast)               [depends: 0A-0E]
Sprint 1B (try_tabulate)             } Phase 1: Tactics
Sprint 1C (try_hw_auto)               [depends: 1A, 1B]
    |
Sprint 2A (verify_to_kernel.rs)      } Phase 2: Spec Encoding
    |                                   [depends: 0A-0D]
Sprint 3A (extraction/verilog.rs)    } Phase 3: Verilog Extraction
    |                                   [depends: 0A-0D]
Sprint 4A (z3_synth.rs)             } Phase 4: Z3 Synthesis Oracle
Sprint 4B (hw_oracle.rs)              [depends: 2A, verification feature]
    |
Sprint 5A (cegar.rs)                } Phase 5: Integration
Sprint 5B (synthesis_refine.rs)       [depends: 1C, 4B, verification feature]
Sprint 5C (synthesize.rs)             [depends: 2A, 3A, 5A]
```

**Critical path:** 0A -> 0B -> 0C -> 0E -> 1A -> 1C -> 5A -> 5C
**Parallelizable:** Phase 2 (encoding) and Phase 3 (extraction) after Phase 0. Phase 4 after Phase 2.

---

## Part VI: Expected Outcomes

| Sprint | Tests | Cumulative | New Files |
|---|---|---|---|
| 0A | +7 | 7 | -- |
| 0B | +6 | 13 | -- |
| 0C | +20 | 33 | -- |
| 0D | +7 | 40 | -- |
| 0E | +7 | 47 | -- |
| 1A | +6 | 53 | -- |
| 1B | +8 | 61 | -- |
| 1C | +4 | 65 | -- |
| 2A | +11 | 76 | `verify_to_kernel.rs` |
| 3A | +13 | 89 | `extraction/verilog.rs` |
| 4A | +6 | 95 | `z3_synth.rs` |
| 4B | +7 | 102 | `hw_oracle.rs` |
| 5A | +7 | 109 | `cegar.rs` |
| 5B | +4 | 113 | -- |
| 5C | +7 | 120 | `synthesize.rs` |

**Final state: 120+ synthesis tests. 5 new source modules. 1 new test file. 2 modified source files.**

Combined with existing 532+ hardware verification tests: **652+ total hardware tests.**

---

## Part VII: Soundness Argument

### The Triple Firewall

**Firewall 1: Kernel Type Checker.** Every synthesized term must pass `infer_type(ctx, candidate)` against the spec type. If the candidate does not inhabit the specification type, synthesis reports failure. This is the same guarantee Coq and Lean provide. Even if Z3 has a bug, even if a tactic produces a wrong term, even if the Verilog extractor mistranslates — the kernel catches it. The kernel is ~800 lines of trusted code (type_checker.rs). Everything else is untrusted.

**Firewall 2: Z3 Equivalence Check.** After extraction, the Verilog is parsed back (`rtl_extract.rs`), converted to a KG (`rtl_kg.rs`), and checked for Z3 equivalence against the original English spec. This catches extraction bugs specifically. This is the same technique used in CRUSH_ASSERTIONFORGE Sprint 3A.

**Firewall 3: Termination and Positivity.** The kernel's structural termination checker (termination.rs, 307 lines) ensures all recursive definitions terminate. The strict positivity checker (positivity.rs, 238 lines) prevents logical paradoxes in inductive types. Together they guarantee that the type theory is consistent — no term inhabits `False`, so a synthesized circuit that type-checks against a spec ACTUALLY satisfies it.

### Why This Is Academically Defensible

1. **Novel problem.** Nobody synthesizes hardware from English specifications with formal correctness certificates.
2. **Sound methodology.** The CoIC kernel provides the same trusted computing base as Coq/Lean. Z3 bitvector is decidable.
3. **Dual verification.** Kernel type checking (constructive proof) AND Z3 semantic equivalence (model checking). Either alone would be respectable; both together is belt-and-suspenders.
4. **Honest scope.** Combinational circuits and bounded sequential circuits. Unbounded liveness is future work, disclosed here, not hidden.
5. **Reproducible.** 120+ tests, all automated, no LLM dependency in the synthesis pipeline.
6. **Stands on existing work.** 532+ verified hardware tests from CRUSH_ASSERTIONFORGE provide the semantic bedrock.

### What This Is NOT

- This is NOT high-level synthesis. We do not compile C to Verilog.
- This is NOT neural synthesis. We do not use LLMs to generate circuits.
- This is NOT SAT-based synthesis. We use proof search first, Z3 as oracle.
- This IS constructive synthesis via the Curry-Howard isomorphism, certified by a type-theoretic kernel, with Z3 as a decidable oracle for the bitvector fragment.

---

## Part VIII: Academic Positioning

> **Title:** Proof-Directed Hardware Synthesis from English Specifications via the Curry-Howard Isomorphism
>
> We present a hardware synthesis system that compiles English specifications into formally verified SystemVerilog via proof search in the Calculus of Inductive Constructions. Unlike existing approaches — manual proofs in Coq (Kami, Cava), behavioral HLS from C (Vivado, Catapult), or LLM-generated assertions (AssertionForge) — our system: (1) accepts specifications in natural language, (2) encodes them as dependent types in a verified kernel, (3) constructs circuit implementations via tactic-driven proof search augmented by a Z3 bitvector oracle, (4) certifies correctness by kernel type-checking (the proof term inhabiting the spec type IS the circuit), and (5) extracts synthesizable SystemVerilog with a secondary Z3 equivalence check. The synthesis pipeline introduces five hardware-specific tactics (try_bitblast, try_tabulate, try_hw_auto, try_fsm_unroll, try_k_induction), a CEGAR refinement loop that combines kernel type errors with Z3 counterexamples, and a Verilog extraction pass from kernel proof terms. We evaluate on combinational circuits (gates, adders, multiplexors) and bounded sequential circuits (handshake protocols, counters), demonstrating that the system synthesizes correct circuits from English specifications with zero manual proof effort and triple-verified correctness guarantees.
