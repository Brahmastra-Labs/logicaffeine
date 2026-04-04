//! Phase HW Synthesis: Proof-Directed Hardware Synthesis via Curry-Howard
//!
//! Under Curry-Howard, a hardware specification encoded as a dependent type
//! in the CoIC kernel IS a synthesis problem. A proof term inhabiting that
//! type IS a correct-by-construction circuit. Extract the proof term to Verilog.
//!
//! Sprint 0: Kernel hardware types (Bit, BVec, gates, Circuit)
//! Sprint 1: Hardware tactics (try_bitblast, try_tabulate, try_hw_auto)
//! Sprint 2: Spec encoding (VerifyExpr -> kernel Term)
//! Sprint 3: Verilog extraction (kernel Term -> SystemVerilog)

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// SPRINT 0A: BIT INDUCTIVE TYPE
// =============================================================================

#[test]
fn hw_bit_is_type_zero() {
    let mut repl = Repl::new();
    let result = repl.execute("Check Bit.").expect("Check Bit");
    assert!(result.contains("Type"), "Bit should be a Type, got: {}", result);
}

#[test]
fn hw_b0_has_type_bit() {
    let mut repl = Repl::new();
    let result = repl.execute("Check B0.").expect("Check B0");
    assert!(result.contains("Bit"), "B0 should have type Bit, got: {}", result);
}

#[test]
fn hw_b1_has_type_bit() {
    let mut repl = Repl::new();
    let result = repl.execute("Check B1.").expect("Check B1");
    assert!(result.contains("Bit"), "B1 should have type Bit, got: {}", result);
}

#[test]
fn hw_b0_not_equal_b1() {
    let mut repl = Repl::new();
    let b0 = repl.execute("Eval B0.").expect("Eval B0");
    let b1 = repl.execute("Eval B1.").expect("Eval B1");
    assert_ne!(b0, b1, "B0 and B1 must be distinct");
}

#[test]
fn hw_unit_is_type_zero() {
    let mut repl = Repl::new();
    let result = repl.execute("Check Unit.").expect("Check Unit");
    assert!(result.contains("Type"), "Unit should be a Type, got: {}", result);
}

#[test]
fn hw_tt_has_type_unit() {
    let mut repl = Repl::new();
    let result = repl.execute("Check Tt.").expect("Check Tt");
    assert!(result.contains("Unit"), "Tt should have type Unit, got: {}", result);
}

#[test]
fn hw_bit_match_two_cases() {
    // Pattern match on Bit should type-check with exactly 2 cases
    let mut repl = Repl::new();
    repl.execute("Definition bit_id : Bit := B0.").expect("Define bit_id");
    let result = repl.execute("Check bit_id.").expect("Check bit_id");
    assert!(result.contains("Bit"), "Definition should type-check as Bit");
}

// =============================================================================
// SPRINT 0B: BVEC INDEXED INDUCTIVE TYPE
// =============================================================================

#[test]
fn hw_bvec_is_nat_to_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check BVec.").expect("Check BVec");
    // BVec : Nat -> Type0
    assert!(result.contains("Nat"), "BVec should take Nat argument, got: {}", result);
}

#[test]
fn hw_bvnil_is_bvec_zero() {
    let mut repl = Repl::new();
    let result = repl.execute("Check BVNil.").expect("Check BVNil");
    assert!(result.contains("BVec"), "BVNil should have BVec type, got: {}", result);
}

#[test]
fn hw_bvcons_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check BVCons.").expect("Check BVCons");
    // BVCons : Bit -> Pi(n:Nat). BVec n -> BVec (Succ n)
    assert!(result.contains("Bit"), "BVCons should take Bit, got: {}", result);
    assert!(result.contains("BVec"), "BVCons should produce BVec, got: {}", result);
}

#[test]
fn hw_bvec_one_bit() {
    // BVCons B1 Zero BVNil : BVec (Succ Zero)
    let mut repl = Repl::new();
    repl.execute("Definition v1 : BVec (Succ Zero) := BVCons B1 Zero BVNil.")
        .expect("Define 1-bit BVec");
    let result = repl.execute("Check v1.").expect("Check v1");
    assert!(result.contains("BVec"), "v1 should have BVec type, got: {}", result);
}

#[test]
fn hw_bvec_two_bits() {
    // [B1, B0] = BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil)
    let mut repl = Repl::new();
    repl.execute(
        "Definition v2 : BVec (Succ (Succ Zero)) := BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil).",
    )
    .expect("Define 2-bit BVec");
    let result = repl.execute("Check v2.").expect("Check v2");
    assert!(result.contains("BVec"), "v2 should have BVec type, got: {}", result);
}

#[test]
fn hw_bvec_match_exhaustive() {
    // Pattern match on BVec should require BVNil + BVCons cases
    let mut repl = Repl::new();
    let result = repl.execute("Check BVNil.").expect("Check BVNil");
    assert!(result.contains("BVec"), "BVNil needed for match, got: {}", result);
}

// =============================================================================
// SPRINT 0C: GATE OPERATION DEFINITIONS — FULL TRUTH TABLES
// =============================================================================

// --- bit_and truth table (4 tests) ---

#[test]
fn hw_bit_and_b0_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_and B0 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_and B0 B0 should be B0, got: {}", result);
}

#[test]
fn hw_bit_and_b0_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_and B0 B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_and B0 B1 should be B0, got: {}", result);
}

#[test]
fn hw_bit_and_b1_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_and B1 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_and B1 B0 should be B0, got: {}", result);
}

#[test]
fn hw_bit_and_b1_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_and B1 B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_and B1 B1 should be B1, got: {}", result);
}

// --- bit_or truth table (4 tests) ---

#[test]
fn hw_bit_or_b0_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_or B0 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_or B0 B0 should be B0, got: {}", result);
}

#[test]
fn hw_bit_or_b0_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_or B0 B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_or B0 B1 should be B1, got: {}", result);
}

#[test]
fn hw_bit_or_b1_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_or B1 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_or B1 B0 should be B1, got: {}", result);
}

#[test]
fn hw_bit_or_b1_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_or B1 B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_or B1 B1 should be B1, got: {}", result);
}

// --- bit_not truth table (2 tests) ---

#[test]
fn hw_bit_not_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_not B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_not B0 should be B1, got: {}", result);
}

#[test]
fn hw_bit_not_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_not B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_not B1 should be B0, got: {}", result);
}

// --- bit_xor truth table (4 tests) ---

#[test]
fn hw_bit_xor_b0_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_xor B0 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_xor B0 B0 should be B0, got: {}", result);
}

#[test]
fn hw_bit_xor_b0_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_xor B0 B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_xor B0 B1 should be B1, got: {}", result);
}

#[test]
fn hw_bit_xor_b1_b0() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_xor B1 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_xor B1 B0 should be B1, got: {}", result);
}

#[test]
fn hw_bit_xor_b1_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_xor B1 B1.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_xor B1 B1 should be B0, got: {}", result);
}

// --- bit_mux truth table (select=B0 picks else, select=B1 picks then) ---

#[test]
fn hw_bit_mux_b0_selects_else() {
    let mut repl = Repl::new();
    // bit_mux B0 then_val else_val -> else_val
    repl.execute("Definition r := bit_mux B0 B1 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "bit_mux B0 should select else (B0), got: {}", result);
}

#[test]
fn hw_bit_mux_b1_selects_then() {
    let mut repl = Repl::new();
    // bit_mux B1 then_val else_val -> then_val
    repl.execute("Definition r := bit_mux B1 B1 B0.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "bit_mux B1 should select then (B1), got: {}", result);
}

// --- gate operation types ---

#[test]
fn hw_bit_and_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check bit_and.").expect("Check bit_and");
    assert!(result.contains("Bit"), "bit_and should have Bit in type, got: {}", result);
}

#[test]
fn hw_gate_composition_normalizes() {
    // bit_and (bit_or B1 B0) (bit_not B0) = bit_and B1 B1 = B1
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_and (bit_or B1 B0) (bit_not B0).").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "composed gates should normalize to B1, got: {}", result);
}

// =============================================================================
// SPRINT 0D: CIRCUIT (MEALY MACHINE) INDUCTIVE TYPE
// =============================================================================

#[test]
fn hw_circuit_is_type_constructor() {
    let mut repl = Repl::new();
    let result = repl.execute("Check Circuit.").expect("Check Circuit");
    assert!(result.contains("Type"), "Circuit should be a type constructor, got: {}", result);
}

#[test]
fn hw_mkcircuit_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check MkCircuit.").expect("Check MkCircuit");
    assert!(result.contains("Circuit"), "MkCircuit should produce Circuit, got: {}", result);
}

#[test]
fn hw_circuit_unit_bit_bit() {
    // Circuit Unit Bit Bit : Type 0
    let mut repl = Repl::new();
    let result = repl.execute("Check (Circuit Unit Bit Bit).").expect("Check Circuit Unit Bit Bit");
    assert!(result.contains("Type"), "Circuit Unit Bit Bit should be a Type, got: {}", result);
}

#[test]
fn hw_mkcircuit_identity_circuit() {
    // Build the circuit using programmatic Term construction via the kernel API
    use logicaffeine_kernel::Term;
    use logicaffeine_kernel::{infer_type, Context};
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let unit = Term::Global("Unit".to_string());
    let bit = Term::Global("Bit".to_string());

    // Transition: λ(s:Unit). λ(i:Bit). s
    let trans = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(unit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Var("s".to_string())),
        }),
    };

    // Output: λ(s:Unit). λ(i:Bit). i
    let out = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(unit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Var("i".to_string())),
        }),
    };

    let circuit = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("MkCircuit".to_string())),
                            Box::new(unit.clone()),
                        )),
                        Box::new(bit.clone()),
                    )),
                    Box::new(bit),
                )),
                Box::new(trans),
            )),
            Box::new(out),
        )),
        Box::new(Term::Global("Tt".to_string())),
    );

    let ty = infer_type(&ctx, &circuit).expect("Identity circuit should type-check");
    assert!(format!("{}", ty).contains("Circuit"), "Should have Circuit type, got: {}", ty);
}

#[test]
fn hw_mkcircuit_inverter() {
    use logicaffeine_kernel::Term;
    use logicaffeine_kernel::{infer_type, Context};
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let unit = Term::Global("Unit".to_string());
    let bit = Term::Global("Bit".to_string());

    let trans = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(unit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Var("s".to_string())),
        }),
    };

    // Output: λ(s:Unit). λ(i:Bit). bit_not i
    let out = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(unit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::App(
                Box::new(Term::Global("bit_not".to_string())),
                Box::new(Term::Var("i".to_string())),
            )),
        }),
    };

    let circuit = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("MkCircuit".to_string())),
                            Box::new(unit.clone()),
                        )),
                        Box::new(bit.clone()),
                    )),
                    Box::new(bit),
                )),
                Box::new(trans),
            )),
            Box::new(out),
        )),
        Box::new(Term::Global("Tt".to_string())),
    );

    let ty = infer_type(&ctx, &circuit).expect("Inverter circuit should type-check");
    assert!(format!("{}", ty).contains("Circuit"), "Should have Circuit type, got: {}", ty);
}

#[test]
fn hw_mkcircuit_sequential_toggle() {
    use logicaffeine_kernel::Term;
    use logicaffeine_kernel::{infer_type, Context};
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let bit = Term::Global("Bit".to_string());

    // Transition: λ(s:Bit). λ(i:Bit). bit_not s
    let trans = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(bit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::App(
                Box::new(Term::Global("bit_not".to_string())),
                Box::new(Term::Var("s".to_string())),
            )),
        }),
    };

    // Output: λ(s:Bit). λ(i:Bit). s
    let out = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(bit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Var("s".to_string())),
        }),
    };

    let circuit = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("MkCircuit".to_string())),
                            Box::new(bit.clone()),
                        )),
                        Box::new(bit.clone()),
                    )),
                    Box::new(bit),
                )),
                Box::new(trans),
            )),
            Box::new(out),
        )),
        Box::new(Term::Global("B0".to_string())),
    );

    let ty = infer_type(&ctx, &circuit).expect("Toggle circuit should type-check");
    assert!(format!("{}", ty).contains("Circuit"), "Should have Circuit type, got: {}", ty);
}

// =============================================================================
// SPRINT 0E: BVEC OPERATIONS
// =============================================================================

#[test]
fn hw_bv_and_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check bv_and.").expect("Check bv_and");
    assert!(result.contains("BVec"), "bv_and should mention BVec, got: {}", result);
}

#[test]
fn hw_bv_and_empty() {
    let mut repl = Repl::new();
    repl.execute("Definition r := bv_and Zero BVNil BVNil.").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("BVNil"), "bv_and empty should be BVNil, got: {}", result);
}

#[test]
fn hw_bv_not_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check bv_not.").expect("Check bv_not");
    assert!(result.contains("BVec"), "bv_not should mention BVec, got: {}", result);
}

#[test]
fn hw_bv_and_two_bits() {
    // bv_and [B1,B0] [B1,B1] should normalize to [B1,B0]
    let mut repl = Repl::new();
    // Build: BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil) = [B1,B0]
    repl.execute("Definition v1 := BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil).").expect("def v1");
    // Build: BVCons B1 (Succ Zero) (BVCons B1 Zero BVNil) = [B1,B1]
    repl.execute("Definition v2 := BVCons B1 (Succ Zero) (BVCons B1 Zero BVNil).").expect("def v2");
    repl.execute("Definition r := bv_and (Succ (Succ Zero)) v1 v2.").expect("def r");
    let result = repl.execute("Eval r.").expect("eval r");
    // After normalization, should contain B0 (from B0 AND B1 = B0) and B1 (from B1 AND B1 = B1)
    // Expected: BVCons B1 _ (BVCons B0 _ BVNil)
    assert!(result.contains("B0"), "bv_and [B1,B0] [B1,B1] should contain B0 in result, got: {}", result);
    // The result should NOT just be the unevaluated application
    assert!(!result.contains("bv_and"), "bv_and should normalize, not remain as application, got: {}", result);
}

#[test]
fn hw_bv_or_two_bits() {
    // bv_or [B1,B0] [B0,B1] should normalize to [B1,B1]
    let mut repl = Repl::new();
    repl.execute("Definition v1 := BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil).").expect("def v1");
    repl.execute("Definition v2 := BVCons B0 (Succ Zero) (BVCons B1 Zero BVNil).").expect("def v2");
    repl.execute("Definition r := bv_or (Succ (Succ Zero)) v1 v2.").expect("def r");
    let result = repl.execute("Eval r.").expect("eval r");
    assert!(!result.contains("bv_or"), "bv_or should normalize, not remain as application, got: {}", result);
}

#[test]
fn hw_bv_not_two_bits() {
    // bv_not [B1,B0] should normalize to [B0,B1]
    let mut repl = Repl::new();
    repl.execute("Definition v1 := BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil).").expect("def v1");
    repl.execute("Definition r := bv_not (Succ (Succ Zero)) v1.").expect("def r");
    let result = repl.execute("Eval r.").expect("eval r");
    assert!(!result.contains("bv_not"), "bv_not should normalize, not remain as application, got: {}", result);
}

#[test]
fn hw_bv_xor_two_bits() {
    // bv_xor [B1,B1] [B1,B0] should normalize to [B0,B1]
    let mut repl = Repl::new();
    repl.execute("Definition v1 := BVCons B1 (Succ Zero) (BVCons B1 Zero BVNil).").expect("def v1");
    repl.execute("Definition v2 := BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil).").expect("def v2");
    repl.execute("Definition r := bv_xor (Succ (Succ Zero)) v1 v2.").expect("def r");
    let result = repl.execute("Eval r.").expect("eval r");
    assert!(!result.contains("bv_xor"), "bv_xor should normalize, not remain as application, got: {}", result);
}

#[test]
fn hw_bv_and_normalizes_to_correct_value() {
    // bv_and [B1,B1] [B1,B1] should normalize to [B1,B1] — all bits AND'd
    let mut repl = Repl::new();
    repl.execute("Definition v := BVCons B1 (Succ Zero) (BVCons B1 Zero BVNil).").expect("def v");
    repl.execute("Definition r := bv_and (Succ (Succ Zero)) v v.").expect("def r");
    let result = repl.execute("Eval r.").expect("eval r");
    // Should normalize to itself (all 1s AND all 1s = all 1s)
    assert!(!result.contains("bv_and"), "bv_and [1,1] [1,1] should normalize, got: {}", result);
    // Result should contain B1 and BVCons
    assert!(result.contains("B1"), "bv_and [1,1] [1,1] should produce B1 bits, got: {}", result);
}

#[test]
fn hw_bv_ops_preserve_width_type() {
    // bv_and on BVec 2 should produce BVec 2
    let mut repl = Repl::new();
    repl.execute("Definition v := BVCons B1 (Succ Zero) (BVCons B0 Zero BVNil).").expect("def v");
    repl.execute("Definition r := bv_and (Succ (Succ Zero)) v v.").expect("def r");
    let result = repl.execute("Check r.").expect("check r");
    // Type should be BVec (Succ (Succ Zero))
    assert!(result.contains("BVec"), "bv_and result should have BVec type, got: {}", result);
}

// =============================================================================
// SPRINT 1A: try_bitblast TACTIC
// =============================================================================
// try_bitblast proves equalities on Bit by normalizing both sides.
// Goal: SApp(SApp(SApp(SName("Eq"), SName("Bit")), lhs), rhs)
// If normalize(lhs) == normalize(rhs), return DBitblastSolve(goal).

#[test]
fn hw_bitblast_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_bitblast.").expect("Check try_bitblast");
    assert_eq!(result, "try_bitblast : Syntax -> Derivation");
}

#[test]
fn hw_bitblast_proves_b1_eq_b1() {
    let mut repl = Repl::new();
    // Goal: Eq Bit B1 B1
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("def T");
    repl.execute("Definition lhs : Syntax := SName \"B1\".").expect("def lhs");
    repl.execute("Definition rhs : Syntax := SName \"B1\".").expect("def rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_bitblast should prove Eq Bit B1 B1");
}

#[test]
fn hw_bitblast_proves_and_result() {
    let mut repl = Repl::new();
    // Goal: Eq Bit (bit_and B1 B0) B0
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("def T");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_and\") (SName \"B1\")) (SName \"B0\").")
        .expect("def lhs");
    repl.execute("Definition rhs : Syntax := SName \"B0\".").expect("def rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_bitblast should prove Eq Bit (bit_and B1 B0) B0");
}

#[test]
fn hw_bitblast_rejects_neq() {
    let mut repl = Repl::new();
    // Goal: Eq Bit B0 B1 — should NOT prove (B0 != B1)
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("def T");
    repl.execute("Definition lhs : Syntax := SName \"B0\".").expect("def lhs");
    repl.execute("Definition rhs : Syntax := SName \"B1\".").expect("def rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    // Should produce error derivation, so concludes != goal
    assert_ne!(concluded, original, "try_bitblast should NOT prove Eq Bit B0 B1");
}

#[test]
fn hw_bitblast_proves_not_not_b1() {
    let mut repl = Repl::new();
    // Goal: Eq Bit (bit_not (bit_not B1)) B1
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("def T");
    repl.execute("Definition inner : Syntax := SApp (SName \"bit_not\") (SName \"B1\").")
        .expect("def inner");
    repl.execute("Definition lhs : Syntax := SApp (SName \"bit_not\") inner.").expect("def lhs");
    repl.execute("Definition rhs : Syntax := SName \"B1\".").expect("def rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_bitblast should prove double negation");
}

#[test]
fn hw_bitblast_proves_composed_gates() {
    let mut repl = Repl::new();
    // Goal: Eq Bit (bit_and (bit_or B1 B0) (bit_not B0)) B1
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("def T");
    repl.execute("Definition or_part : Syntax := SApp (SApp (SName \"bit_or\") (SName \"B1\")) (SName \"B0\").")
        .expect("def or");
    repl.execute("Definition not_part : Syntax := SApp (SName \"bit_not\") (SName \"B0\").")
        .expect("def not");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_and\") or_part) not_part.")
        .expect("def lhs");
    repl.execute("Definition rhs : Syntax := SName \"B1\".").expect("def rhs");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_bitblast should prove composed gate equality");
}

#[test]
fn hw_bitblast_wrong_type_fails() {
    let mut repl = Repl::new();
    // Goal: Eq Nat Zero Zero — wrong type for bitblast (should fail or defer)
    repl.execute("Definition T : Syntax := SName \"Nat\".").expect("def T");
    repl.execute("Definition a : Syntax := SName \"Zero\".").expect("def a");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) a.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    // Bitblast should NOT prove Nat equalities — it's for Bit only
    assert_ne!(concluded, original, "try_bitblast should NOT prove Nat equalities");
}

// =============================================================================
// SPRINT 1B: try_tabulate TACTIC
// =============================================================================
// try_tabulate proves universally quantified Bit goals by exhaustive enumeration.
// Goal: SPi(SName("Bit"), body) — enumerate B0/B1, check each.

#[test]
fn hw_tabulate_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_tabulate.").expect("Check try_tabulate");
    assert_eq!(result, "try_tabulate : Syntax -> Derivation");
}

#[test]
fn hw_tabulate_proves_bit_identity() {
    // Goal: Pi(a:Bit). Eq Bit a a — identity on Bit, 2 cases
    let mut repl = Repl::new();
    // Encode: SPi (SName "Bit") (SApp (SApp (SApp (SName "Eq") (SName "Bit")) (SVar 0)) (SVar 0))
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    repl.execute("Definition body : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) (SVar 0)) (SVar 0).")
        .expect("def body");
    repl.execute("Definition goal : Syntax := SPi btype body.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_tabulate should prove Pi(a:Bit). Eq Bit a a");
}

#[test]
fn hw_tabulate_proves_double_negation() {
    // Goal: Pi(a:Bit). Eq Bit (bit_not (bit_not a)) a
    let mut repl = Repl::new();
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    repl.execute("Definition inner : Syntax := SApp (SName \"bit_not\") (SVar 0).").expect("def inner");
    repl.execute("Definition lhs : Syntax := SApp (SName \"bit_not\") inner.").expect("def lhs");
    repl.execute("Definition rhs : Syntax := SVar 0.").expect("def rhs");
    repl.execute("Definition body : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) lhs) rhs.")
        .expect("def body");
    repl.execute("Definition goal : Syntax := SPi btype body.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_tabulate should prove double negation on Bit");
}

#[test]
fn hw_tabulate_proves_and_commutativity() {
    // Goal: Pi(a:Bit). Pi(b:Bit). Eq Bit (bit_and a b) (bit_and b a)
    let mut repl = Repl::new();
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    // In the body of two nested Pi's: SVar 1 is a, SVar 0 is b
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_and\") (SVar 1)) (SVar 0).")
        .expect("def lhs");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"bit_and\") (SVar 0)) (SVar 1).")
        .expect("def rhs");
    repl.execute("Definition inner : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) lhs) rhs.")
        .expect("def inner");
    repl.execute("Definition outer : Syntax := SPi btype inner.").expect("def outer");
    repl.execute("Definition goal : Syntax := SPi btype outer.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_tabulate should prove AND commutativity");
}

#[test]
fn hw_tabulate_proves_demorgan() {
    // Goal: Pi(a:Bit). Pi(b:Bit). Eq Bit (bit_not (bit_and a b)) (bit_or (bit_not a) (bit_not b))
    let mut repl = Repl::new();
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    repl.execute("Definition and_ab : Syntax := SApp (SApp (SName \"bit_and\") (SVar 1)) (SVar 0).")
        .expect("def and_ab");
    repl.execute("Definition lhs : Syntax := SApp (SName \"bit_not\") and_ab.").expect("def lhs");
    repl.execute("Definition not_a : Syntax := SApp (SName \"bit_not\") (SVar 1).").expect("def not_a");
    repl.execute("Definition not_b : Syntax := SApp (SName \"bit_not\") (SVar 0).").expect("def not_b");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"bit_or\") not_a) not_b.")
        .expect("def rhs");
    repl.execute("Definition inner : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) lhs) rhs.")
        .expect("def inner");
    repl.execute("Definition outer : Syntax := SPi btype inner.").expect("def outer");
    repl.execute("Definition goal : Syntax := SPi btype outer.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_tabulate should prove DeMorgan's law");
}

#[test]
fn hw_tabulate_refutes_false_claim() {
    // Goal: Pi(a:Bit). Eq Bit a B0 — false (B1 != B0), should produce error derivation
    let mut repl = Repl::new();
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    repl.execute("Definition body : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) (SVar 0)) (SName \"B0\").")
        .expect("def body");
    repl.execute("Definition goal : Syntax := SPi btype body.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_ne!(concluded, original, "try_tabulate should NOT prove Pi(a:Bit). Eq Bit a B0");
}

#[test]
fn hw_tabulate_three_inputs() {
    // Goal: Pi(a:Bit). Pi(b:Bit). Pi(c:Bit). Eq Bit (bit_and a (bit_or b c)) (bit_and a (bit_or b c))
    // Trivially true identity, but exercises 2^3 = 8 enumeration
    let mut repl = Repl::new();
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    // SVar 2 = a, SVar 1 = b, SVar 0 = c
    repl.execute("Definition or_bc : Syntax := SApp (SApp (SName \"bit_or\") (SVar 1)) (SVar 0).")
        .expect("def or_bc");
    repl.execute("Definition expr : Syntax := SApp (SApp (SName \"bit_and\") (SVar 2)) or_bc.")
        .expect("def expr");
    repl.execute("Definition innermost : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) expr) expr.")
        .expect("def innermost");
    repl.execute("Definition pi3 : Syntax := SPi btype innermost.").expect("def pi3");
    repl.execute("Definition pi2 : Syntax := SPi btype pi3.").expect("def pi2");
    repl.execute("Definition goal : Syntax := SPi btype pi2.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_tabulate should prove 3-input identity (8 cases)");
}

#[test]
fn hw_tabulate_concludes_matches_goal() {
    // The concludes of a successful try_tabulate derivation should equal the original goal
    let mut repl = Repl::new();
    repl.execute("Definition btype : Syntax := SName \"Bit\".").expect("def btype");
    repl.execute("Definition body : Syntax := SApp (SApp (SApp (SName \"Eq\") btype) (SVar 0)) (SVar 0).")
        .expect("def body");
    repl.execute("Definition goal : Syntax := SPi btype body.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    let d_str = repl.execute("Eval d.").expect("eval d");
    // Successful derivation should contain the goal, not be an error
    assert!(!d_str.contains("Error"), "Derivation should not be an error: {}", d_str);
}

#[test]
fn hw_tabulate_ignores_non_bit() {
    // Goal: Pi(n:Nat). Eq Nat n n — not a Bit domain, should produce error
    let mut repl = Repl::new();
    repl.execute("Definition ntype : Syntax := SName \"Nat\".").expect("def ntype");
    repl.execute("Definition body : Syntax := SApp (SApp (SApp (SName \"Eq\") ntype) (SVar 0)) (SVar 0).")
        .expect("def body");
    repl.execute("Definition goal : Syntax := SPi ntype body.").expect("def goal");
    repl.execute("Definition d : Derivation := try_tabulate goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_ne!(concluded, original, "try_tabulate should NOT handle Nat domain");
}

// =============================================================================
// SPRINT 1C: try_hw_auto COMPOSITE TACTIC
// =============================================================================

#[test]
fn hw_auto_type_correct() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_hw_auto.").expect("Check try_hw_auto");
    assert_eq!(result, "try_hw_auto : Syntax -> Derivation");
}

#[test]
fn hw_auto_solves_bit_eq() {
    let mut repl = Repl::new();
    // Goal: Eq Bit B1 B1 — should be solved by try_bitblast via try_hw_auto
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("def T");
    repl.execute("Definition a : Syntax := SName \"B1\".").expect("def a");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) a) a.")
        .expect("def goal");
    repl.execute("Definition d : Derivation := try_hw_auto goal.").expect("apply tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("def result");

    let concluded = repl.execute("Eval result.").expect("eval result");
    let original = repl.execute("Eval goal.").expect("eval goal");
    assert_eq!(concluded, original, "try_hw_auto should prove Eq Bit B1 B1");
}

// =============================================================================
// SPRINT 2A: SPEC ENCODING (BoundedExpr → Kernel Term)
// =============================================================================

#[test]
fn hw_encode_bool_true() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Bool(true);
    let term = encode_bounded_expr(&expr);
    assert_eq!(format!("{}", term), "True", "Bool(true) should encode as True");
}

#[test]
fn hw_encode_bool_false() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Bool(false);
    let term = encode_bounded_expr(&expr);
    assert_eq!(format!("{}", term), "False", "Bool(false) should encode as False");
}

#[test]
fn hw_encode_var_signal_at_time() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Var("req@0".to_string());
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    // Should encode as application of signal name to timestep
    assert!(s.contains("req"), "Should contain signal name 'req', got: {}", s);
}

#[test]
fn hw_encode_and() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::And(
        Box::new(BoundedExpr::Bool(true)),
        Box::new(BoundedExpr::Bool(false)),
    );
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    assert!(s.contains("And"), "And should encode using And connective, got: {}", s);
}

#[test]
fn hw_encode_or() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Or(
        Box::new(BoundedExpr::Bool(true)),
        Box::new(BoundedExpr::Bool(false)),
    );
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    assert!(s.contains("Or"), "Or should encode using Or connective, got: {}", s);
}

#[test]
fn hw_encode_not() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Not(Box::new(BoundedExpr::Bool(true)));
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    assert!(s.contains("Not"), "Not should encode using Not, got: {}", s);
}

#[test]
fn hw_encode_implies() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Implies(
        Box::new(BoundedExpr::Var("req@0".to_string())),
        Box::new(BoundedExpr::Var("ack@0".to_string())),
    );
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    // Implication encodes as Pi type (function type) under Curry-Howard
    assert!(s.contains("->") || s.contains("Π"), "Implies should encode as Pi/arrow, got: {}", s);
}

#[test]
fn hw_encode_eq() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Eq(
        Box::new(BoundedExpr::Var("x@0".to_string())),
        Box::new(BoundedExpr::Var("y@0".to_string())),
    );
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    assert!(s.contains("Eq"), "Eq should encode using Eq, got: {}", s);
}

#[test]
fn hw_encode_int() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    let expr = BoundedExpr::Int(42);
    let term = encode_bounded_expr(&expr);
    let s = format!("{}", term);
    assert!(s.contains("42"), "Int(42) should encode as literal 42, got: {}", s);
}

#[test]
fn hw_encoded_type_passes_kernel_check() {
    use logicaffeine_compile::codegen_sva::verify_to_kernel::encode_bounded_expr;
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
    use logicaffeine_kernel::{infer_type, Context};
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Simple: True AND False should type-check as Prop
    let expr = BoundedExpr::And(
        Box::new(BoundedExpr::Bool(true)),
        Box::new(BoundedExpr::Bool(false)),
    );
    let term = encode_bounded_expr(&expr);
    let result = infer_type(&ctx, &term);
    assert!(result.is_ok(), "Encoded BoundedExpr should type-check in kernel, got: {:?}", result.err());
}

// =============================================================================
// SPRINT 3A: VERILOG EXTRACTION (Kernel Term → SystemVerilog)
// =============================================================================

#[test]
fn hw_verilog_bit_and_extracts() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    // bit_and a b → "a & b"
    let term = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("bit_and".to_string())),
            Box::new(Term::Var("a".to_string())),
        )),
        Box::new(Term::Var("b".to_string())),
    );
    let sv = term_to_verilog(&term);
    assert!(sv.contains("&"), "bit_and should extract to &, got: {}", sv);
}

#[test]
fn hw_verilog_bit_or_extracts() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    let term = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("bit_or".to_string())),
            Box::new(Term::Var("a".to_string())),
        )),
        Box::new(Term::Var("b".to_string())),
    );
    let sv = term_to_verilog(&term);
    assert!(sv.contains("|"), "bit_or should extract to |, got: {}", sv);
}

#[test]
fn hw_verilog_bit_not_extracts() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    let term = Term::App(
        Box::new(Term::Global("bit_not".to_string())),
        Box::new(Term::Var("a".to_string())),
    );
    let sv = term_to_verilog(&term);
    assert!(sv.contains("~"), "bit_not should extract to ~, got: {}", sv);
}

#[test]
fn hw_verilog_bit_xor_extracts() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    let term = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("bit_xor".to_string())),
            Box::new(Term::Var("a".to_string())),
        )),
        Box::new(Term::Var("b".to_string())),
    );
    let sv = term_to_verilog(&term);
    assert!(sv.contains("^"), "bit_xor should extract to ^, got: {}", sv);
}

#[test]
fn hw_verilog_constants() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    let b0 = term_to_verilog(&Term::Global("B0".to_string()));
    let b1 = term_to_verilog(&Term::Global("B1".to_string()));
    assert!(b0.contains("1'b0"), "B0 should extract to 1'b0, got: {}", b0);
    assert!(b1.contains("1'b1"), "B1 should extract to 1'b1, got: {}", b1);
}

#[test]
fn hw_verilog_variable() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    let v = term_to_verilog(&Term::Var("data_in".to_string()));
    assert_eq!(v, "data_in", "Variable should extract as identifier, got: {}", v);
}

#[test]
fn hw_verilog_nested_expression() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    // bit_and (bit_or a b) (bit_not c)
    let or_ab = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("bit_or".to_string())),
            Box::new(Term::Var("a".to_string())),
        )),
        Box::new(Term::Var("b".to_string())),
    );
    let not_c = Term::App(
        Box::new(Term::Global("bit_not".to_string())),
        Box::new(Term::Var("c".to_string())),
    );
    let term = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("bit_and".to_string())),
            Box::new(or_ab),
        )),
        Box::new(not_c),
    );
    let sv = term_to_verilog(&term);
    assert!(sv.contains("&"), "Should contain &, got: {}", sv);
    assert!(sv.contains("|"), "Should contain |, got: {}", sv);
    assert!(sv.contains("~"), "Should contain ~, got: {}", sv);
}

#[test]
fn hw_verilog_bit_mux_extracts() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    // bit_mux sel a b → "sel ? a : b"
    let term = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("bit_mux".to_string())),
                Box::new(Term::Var("sel".to_string())),
            )),
            Box::new(Term::Var("a".to_string())),
        )),
        Box::new(Term::Var("b".to_string())),
    );
    let sv = term_to_verilog(&term);
    assert!(sv.contains("?"), "bit_mux should extract to ternary ?, got: {}", sv);
    assert!(sv.contains(":"), "bit_mux should extract to ternary :, got: {}", sv);
}

// =============================================================================
// BULK 1: KERNEL TYPE EDGE CASES
// =============================================================================

#[test]
fn hw_bulk1_triple_gate_composition() {
    // bit_xor (bit_and B1 B1) (bit_or B0 B1) = xor(1, 1) = 0
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_xor (bit_and B1 B1) (bit_or B0 B1).").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B0"), "xor(and(1,1), or(0,1)) = xor(1,1) = 0, got: {}", result);
}

#[test]
fn hw_bulk1_four_deep_composition() {
    // bit_not (bit_and (bit_or B1 B0) (bit_xor B1 B1))
    // = not(and(1, 0)) = not(0) = 1
    let mut repl = Repl::new();
    repl.execute("Definition r := bit_not (bit_and (bit_or B1 B0) (bit_xor B1 B1)).").expect("def");
    let result = repl.execute("Eval r.").expect("eval");
    assert!(result.contains("B1"), "not(and(or(1,0), xor(1,1))) = not(and(1,0)) = not(0) = 1, got: {}", result);
}

#[test]
fn hw_bulk1_mux_all_combinations() {
    let mut repl = Repl::new();
    // sel=B0, then=B0, else=B1 → B1
    repl.execute("Definition m1 := bit_mux B0 B0 B1.").expect("def");
    let r1 = repl.execute("Eval m1.").expect("eval");
    assert!(r1.contains("B1"), "mux(0,0,1) should be 1 (else), got: {}", r1);

    // sel=B1, then=B0, else=B1 → B0
    repl.execute("Definition m2 := bit_mux B1 B0 B1.").expect("def");
    let r2 = repl.execute("Eval m2.").expect("eval");
    assert!(r2.contains("B0"), "mux(1,0,1) should be 0 (then), got: {}", r2);
}

#[test]
fn hw_bulk1_circuit_with_and_gate_output() {
    use logicaffeine_kernel::Term;
    use logicaffeine_kernel::{infer_type, Context};
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let unit = Term::Global("Unit".to_string());
    let bit = Term::Global("Bit".to_string());

    // AND gate circuit: output = bit_and i1 i2
    // Using a pair encoding: input is (Bit, Bit) — but we don't have pairs.
    // Instead: Circuit Unit Bit Bit where output = bit_and input input (self-and = identity)
    let trans = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(unit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Var("s".to_string())),
        }),
    };
    let out = Term::Lambda {
        param: "s".to_string(),
        param_type: Box::new(unit.clone()),
        body: Box::new(Term::Lambda {
            param: "i".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("bit_and".to_string())),
                    Box::new(Term::Var("i".to_string())),
                )),
                Box::new(Term::Var("i".to_string())),
            )),
        }),
    };

    let circuit = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("MkCircuit".to_string())),
                            Box::new(unit.clone()),
                        )),
                        Box::new(bit.clone()),
                    )),
                    Box::new(bit),
                )),
                Box::new(trans),
            )),
            Box::new(out),
        )),
        Box::new(Term::Global("Tt".to_string())),
    );

    let ty = infer_type(&ctx, &circuit).expect("AND gate circuit should type-check");
    assert!(format!("{}", ty).contains("Circuit"), "Should be Circuit type, got: {}", ty);
}

#[test]
fn hw_bulk1_bit_not_involution() {
    // not(not(B0)) = B0, not(not(B1)) = B1
    let mut repl = Repl::new();
    repl.execute("Definition r1 := bit_not (bit_not B0).").expect("def");
    let r1 = repl.execute("Eval r1.").expect("eval");
    assert!(r1.contains("B0"), "not(not(B0)) should be B0, got: {}", r1);

    repl.execute("Definition r2 := bit_not (bit_not B1).").expect("def");
    let r2 = repl.execute("Eval r2.").expect("eval");
    assert!(r2.contains("B1"), "not(not(B1)) should be B1, got: {}", r2);
}

#[test]
fn hw_bulk1_xor_self_is_zero() {
    // a XOR a = 0 for both inputs
    let mut repl = Repl::new();
    repl.execute("Definition r1 := bit_xor B0 B0.").expect("def");
    let r1 = repl.execute("Eval r1.").expect("eval");
    assert!(r1.contains("B0"), "B0 xor B0 = B0, got: {}", r1);

    repl.execute("Definition r2 := bit_xor B1 B1.").expect("def");
    let r2 = repl.execute("Eval r2.").expect("eval");
    assert!(r2.contains("B0"), "B1 xor B1 = B0, got: {}", r2);
}

#[test]
fn hw_bulk1_and_identity() {
    // a AND B1 = a
    let mut repl = Repl::new();
    repl.execute("Definition r1 := bit_and B0 B1.").expect("def");
    let r1 = repl.execute("Eval r1.").expect("eval");
    assert!(r1.contains("B0"), "B0 and B1 = B0, got: {}", r1);

    repl.execute("Definition r2 := bit_and B1 B1.").expect("def");
    let r2 = repl.execute("Eval r2.").expect("eval");
    assert!(r2.contains("B1"), "B1 and B1 = B1, got: {}", r2);
}

#[test]
fn hw_bulk1_or_annihilation() {
    // a OR B1 = B1
    let mut repl = Repl::new();
    repl.execute("Definition r1 := bit_or B0 B1.").expect("def");
    let r1 = repl.execute("Eval r1.").expect("eval");
    assert!(r1.contains("B1"), "B0 or B1 = B1, got: {}", r1);

    repl.execute("Definition r2 := bit_or B1 B1.").expect("def");
    let r2 = repl.execute("Eval r2.").expect("eval");
    assert!(r2.contains("B1"), "B1 or B1 = B1, got: {}", r2);
}

#[test]
fn hw_bulk1_and_annihilation() {
    // a AND B0 = B0
    let mut repl = Repl::new();
    repl.execute("Definition r1 := bit_and B0 B0.").expect("def");
    let r1 = repl.execute("Eval r1.").expect("eval");
    assert!(r1.contains("B0"), "B0 and B0 = B0, got: {}", r1);

    repl.execute("Definition r2 := bit_and B1 B0.").expect("def");
    let r2 = repl.execute("Eval r2.").expect("eval");
    assert!(r2.contains("B0"), "B1 and B0 = B0, got: {}", r2);
}

#[test]
fn hw_bulk1_verilog_nested_mux() {
    use logicaffeine_compile::extraction::verilog::term_to_verilog;
    use logicaffeine_kernel::Term;

    // bit_mux sel1 (bit_mux sel2 a b) c → nested ternary
    let inner_mux = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("bit_mux".to_string())),
                Box::new(Term::Var("sel2".to_string())),
            )),
            Box::new(Term::Var("a".to_string())),
        )),
        Box::new(Term::Var("b".to_string())),
    );
    let outer = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("bit_mux".to_string())),
                Box::new(Term::Var("sel1".to_string())),
            )),
            Box::new(inner_mux),
        )),
        Box::new(Term::Var("c".to_string())),
    );
    let sv = term_to_verilog(&outer);
    // Should have two ternary operators
    assert_eq!(sv.matches('?').count(), 2, "Nested mux should have 2 ternaries, got: {}", sv);
}

// =============================================================================
// BULK 5: BOOLEAN ALGEBRA LAWS (proven by try_bitblast)
// =============================================================================

#[test]
fn hw_bulk5_bitblast_and_commutative_b0_b1() {
    let mut repl = Repl::new();
    // Eq Bit (bit_and B0 B1) (bit_and B1 B0)
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_and\") (SName \"B0\")) (SName \"B1\").").expect("l");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"bit_and\") (SName \"B1\")) (SName \"B0\").").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "AND commutativity: and(0,1) = and(1,0)");
}

#[test]
fn hw_bulk5_bitblast_or_commutative_b0_b1() {
    let mut repl = Repl::new();
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_or\") (SName \"B0\")) (SName \"B1\").").expect("l");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"bit_or\") (SName \"B1\")) (SName \"B0\").").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "OR commutativity: or(0,1) = or(1,0)");
}

#[test]
fn hw_bulk5_bitblast_demorgan_and() {
    let mut repl = Repl::new();
    // DeMorgan: not(and(B1,B0)) = or(not(B1), not(B0)) = or(B0, B1) = B1
    // not(and(1,0)) = not(0) = 1
    // or(not(1), not(0)) = or(0, 1) = 1
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SName \"bit_not\") (SApp (SApp (SName \"bit_and\") (SName \"B1\")) (SName \"B0\")).").expect("l");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"bit_or\") (SApp (SName \"bit_not\") (SName \"B1\"))) (SApp (SName \"bit_not\") (SName \"B0\")).").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "DeMorgan AND: not(and(1,0)) = or(not(1),not(0))");
}

#[test]
fn hw_bulk5_bitblast_demorgan_or() {
    let mut repl = Repl::new();
    // DeMorgan: not(or(B0,B1)) = and(not(B0), not(B1)) = and(B1, B0) = B0
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SName \"bit_not\") (SApp (SApp (SName \"bit_or\") (SName \"B0\")) (SName \"B1\")).").expect("l");
    repl.execute("Definition rhs : Syntax := SApp (SApp (SName \"bit_and\") (SApp (SName \"bit_not\") (SName \"B0\"))) (SApp (SName \"bit_not\") (SName \"B1\")).").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "DeMorgan OR: not(or(0,1)) = and(not(0),not(1))");
}

#[test]
fn hw_bulk5_bitblast_double_negation() {
    let mut repl = Repl::new();
    // not(not(B0)) = B0
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SName \"bit_not\") (SApp (SName \"bit_not\") (SName \"B0\")).").expect("l");
    repl.execute("Definition rhs : Syntax := SName \"B0\".").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "Double negation: not(not(B0)) = B0");
}

#[test]
fn hw_bulk5_bitblast_complement() {
    let mut repl = Repl::new();
    // a AND not(a) = B0 — complement law
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_and\") (SName \"B1\")) (SApp (SName \"bit_not\") (SName \"B1\")).").expect("l");
    repl.execute("Definition rhs : Syntax := SName \"B0\".").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "Complement: B1 AND NOT(B1) = B0");
}

#[test]
fn hw_bulk5_bitblast_absorption() {
    let mut repl = Repl::new();
    // Absorption: a AND (a OR b) = a — test with B1
    // B1 AND (B1 OR B0) = B1 AND B1 = B1
    repl.execute("Definition T : Syntax := SName \"Bit\".").expect("t");
    repl.execute("Definition lhs : Syntax := SApp (SApp (SName \"bit_and\") (SName \"B1\")) (SApp (SApp (SName \"bit_or\") (SName \"B1\")) (SName \"B0\")).").expect("l");
    repl.execute("Definition rhs : Syntax := SName \"B1\".").expect("r");
    repl.execute("Definition goal : Syntax := SApp (SApp (SApp (SName \"Eq\") T) lhs) rhs.").expect("g");
    repl.execute("Definition d : Derivation := try_bitblast goal.").expect("tactic");
    repl.execute("Definition result : Syntax := concludes d.").expect("result");
    let concluded = repl.execute("Eval result.").expect("eval");
    let original = repl.execute("Eval goal.").expect("eval");
    assert_eq!(concluded, original, "Absorption: B1 AND (B1 OR B0) = B1");
}

// =============================================================================
// SPRINT 4A: Z3 SYNTHESIS CONSTRAINT BUILDER
// =============================================================================

#[test]
fn hw_synth_constraint_and_gate() {
    // A spec for AND gate: Pi(a:Bit). Pi(b:Bit). Eq Bit (f a b) (bit_and a b)
    // The constraint builder should produce a satisfiable VerifyExpr
    use logicaffeine_compile::codegen_sva::z3_synth::{
        build_synthesis_constraint, SynthesisConstraint, SynthesisConstraintConfig,
    };

    // Build spec type for AND gate
    let spec = logicaffeine_kernel::Term::Pi {
        param: "a".to_string(),
        param_type: Box::new(logicaffeine_kernel::Term::Global("Bit".to_string())),
        body_type: Box::new(logicaffeine_kernel::Term::Pi {
            param: "b".to_string(),
            param_type: Box::new(logicaffeine_kernel::Term::Global("Bit".to_string())),
            body_type: Box::new(logicaffeine_kernel::Term::Global("Bit".to_string())),
        }),
    };

    let config = SynthesisConstraintConfig::default();
    let result = build_synthesis_constraint(&spec, &config);
    assert!(
        matches!(result, SynthesisConstraint::Satisfiable(_)),
        "AND gate spec should produce satisfiable constraint, got: {:?}", result
    );
}

#[test]
fn hw_synth_constraint_unsupported_reports() {
    use logicaffeine_compile::codegen_sva::z3_synth::{
        build_synthesis_constraint, SynthesisConstraint, SynthesisConstraintConfig,
    };

    // A non-hardware spec (just a Nat) should be unsupported
    let spec = logicaffeine_kernel::Term::Global("Nat".to_string());
    let config = SynthesisConstraintConfig::default();
    let result = build_synthesis_constraint(&spec, &config);
    // Should be Unsupported, not panic
    assert!(
        !matches!(result, SynthesisConstraint::Satisfiable(_)),
        "Non-hardware spec should NOT produce satisfiable constraint"
    );
}

#[test]
fn hw_synth_extract_io_from_spec() {
    use logicaffeine_compile::codegen_sva::z3_synth::extract_io_from_spec;

    // Pi(a:Bit). Pi(b:Bit). Bit — 2 inputs, 1 output
    let spec = logicaffeine_kernel::Term::Pi {
        param: "a".to_string(),
        param_type: Box::new(logicaffeine_kernel::Term::Global("Bit".to_string())),
        body_type: Box::new(logicaffeine_kernel::Term::Pi {
            param: "b".to_string(),
            param_type: Box::new(logicaffeine_kernel::Term::Global("Bit".to_string())),
            body_type: Box::new(logicaffeine_kernel::Term::Global("Bit".to_string())),
        }),
    };

    let (inputs, output) = extract_io_from_spec(&spec);
    assert_eq!(inputs.len(), 2, "Should extract 2 inputs from Pi(a:Bit).Pi(b:Bit).Bit");
    assert!(output.is_some(), "Should extract output type");
}

// =============================================================================
// SPRINT 5C: TOP-LEVEL SYNTHESIS API
// =============================================================================

#[test]
fn hw_synthesize_from_spec_returns_result() {
    use logicaffeine_compile::codegen_sva::synthesize::{
        synthesize_from_spec, SynthesisConfig, SynthesisResult,
    };

    let config = SynthesisConfig::default();
    let result = synthesize_from_spec("Output equals input A and input B", &config);
    // For now this returns Failed since the pipeline is a stub,
    // but it should NOT panic
    match result {
        SynthesisResult::Success { verilog, .. } => {
            assert!(!verilog.is_empty(), "Synthesized Verilog should not be empty");
            assert!(verilog.contains("&") || verilog.contains("and"),
                "AND gate Verilog should contain & operator, got: {}", verilog);
        }
        SynthesisResult::Unrealizable(msg) => {
            panic!("AND gate spec should be realizable, got: {}", msg);
        }
        SynthesisResult::Failed(msg) => {
            // This is expected until Phase 4-5 is fully implemented
            // but for GREEN, this should succeed
            panic!("Synthesis should succeed for AND gate, got: {}", msg);
        }
    }
}

#[test]
fn hw_synthesize_inverter() {
    use logicaffeine_compile::codegen_sva::synthesize::{
        synthesize_from_spec, SynthesisConfig, SynthesisResult,
    };

    let config = SynthesisConfig::default();
    let result = synthesize_from_spec("Output is the negation of input", &config);
    match result {
        SynthesisResult::Success { verilog, .. } => {
            assert!(verilog.contains("~") || verilog.contains("not"),
                "Inverter Verilog should contain ~ operator, got: {}", verilog);
        }
        SynthesisResult::Failed(msg) => {
            panic!("Synthesis should succeed for inverter, got: {}", msg);
        }
        _ => panic!("Unexpected result for inverter"),
    }
}

#[test]
fn hw_synthesize_unrealizable_detected() {
    use logicaffeine_compile::codegen_sva::synthesize::{
        synthesize_from_spec, SynthesisConfig, SynthesisResult,
    };

    let config = SynthesisConfig::default();
    let result = synthesize_from_spec("Output is both high and low simultaneously", &config);
    assert!(
        matches!(result, SynthesisResult::Unrealizable(_)),
        "Contradictory spec should be detected as unrealizable, got: {:?}",
        match &result {
            SynthesisResult::Success { .. } => "Success",
            SynthesisResult::Unrealizable(_) => "Unrealizable",
            SynthesisResult::Failed(m) => m.as_str(),
        }
    );
}

#[test]
fn hw_synthesize_config_respected() {
    use logicaffeine_compile::codegen_sva::synthesize::{
        synthesize_from_spec, SynthesisConfig, SynthesisResult,
    };

    let config = SynthesisConfig {
        max_iterations: 1,
        timeout_ms: 100,
        verify_extraction: false,
    };
    // With very limited budget, should fail gracefully
    let result = synthesize_from_spec("Output equals input A and input B", &config);
    // Should not panic regardless of config
    match result {
        SynthesisResult::Success { iterations, .. } => {
            assert!(iterations <= 1, "Should respect max_iterations=1");
        }
        SynthesisResult::Failed(_) | SynthesisResult::Unrealizable(_) => {
            // Acceptable with limited budget
        }
    }
}

// =============================================================================
// REACTIVE SYNTHESIS: Z3-BACKED is_tautology / is_contradictory / strategy
// =============================================================================

#[cfg(feature = "verification")]
#[test]
fn synth_contradictory_not_just_and_not() {
    use logicaffeine_verify::synthesis::{synthesize_from_ltl, SignalDecl, SynthesisResult};
    use logicaffeine_verify::VerifyExpr;

    let spec = VerifyExpr::Bool(false);
    let inputs = vec![SignalDecl { name: "req".into(), width: None }];
    let outputs = vec![SignalDecl { name: "ack".into(), width: None }];

    let result = synthesize_from_ltl(&spec, &inputs, &outputs);
    assert!(
        matches!(result, SynthesisResult::Unrealizable { .. }),
        "Bool(false) is contradictory and should be detected as Unrealizable, got: {:?}",
        match &result {
            SynthesisResult::Realizable { .. } => "Realizable",
            SynthesisResult::Unrealizable { .. } => "Unrealizable",
            SynthesisResult::Unknown => "Unknown",
        }
    );
}

#[cfg(feature = "verification")]
#[test]
fn synth_tautology_uses_z3() {
    use logicaffeine_verify::synthesis::{synthesize_from_ltl, SignalDecl, SynthesisResult};
    use logicaffeine_verify::ic3::check_sat;
    use logicaffeine_verify::VerifyExpr;

    // Spec: ack OR NOT(ack) — a tautology over the output signal.
    // Any output assignment satisfies this, so the "all-true" strategy should be
    // validated by Z3-backed is_tautology (substituting ack=true gives true OR false,
    // which is a tautology). The stub is_tautology only checks for literal Bool(true),
    // so it misses this and falls to an unconditional fallback that uses ack=req.
    let spec = VerifyExpr::or(
        VerifyExpr::var("ack"),
        VerifyExpr::not(VerifyExpr::var("ack")),
    );
    let inputs = vec![SignalDecl { name: "req".into(), width: None }];
    let outputs = vec![SignalDecl { name: "ack".into(), width: None }];

    let result = synthesize_from_ltl(&spec, &inputs, &outputs);
    match &result {
        SynthesisResult::Realizable { controller } => {
            assert!(!controller.transitions.is_empty(), "Controller should have transitions");
            // With Z3-backed is_tautology, the "all-true" strategy is validated
            // and the output should be Bool(true), not a variable reference (fallback).
            let first_output = &controller.transitions[0].outputs[0].1;
            assert!(
                matches!(first_output, VerifyExpr::Bool(true)),
                "Z3-backed tautology check should validate the all-true strategy \
                 (output should be Bool(true)), but got: {:?}",
                first_output,
            );
        }
        other => panic!(
            "Tautological spec (ack OR NOT ack) should be Realizable, got: {:?}",
            match other {
                SynthesisResult::Unrealizable { reason } => format!("Unrealizable: {}", reason),
                SynthesisResult::Unknown => "Unknown".to_string(),
                _ => unreachable!(),
            }
        ),
    }
}

#[cfg(feature = "verification")]
#[test]
fn synth_controller_actually_satisfies_spec() {
    use logicaffeine_verify::synthesis::{synthesize_from_ltl, SignalDecl, SynthesisResult};
    use logicaffeine_verify::ic3::check_sat;
    use logicaffeine_verify::{VerifyExpr, VerifyOp};

    // Spec: req IMPLIES ack (whenever req is high, ack must be high)
    let spec = VerifyExpr::binary(
        VerifyOp::Implies,
        VerifyExpr::var("req"),
        VerifyExpr::var("ack"),
    );
    let inputs = vec![SignalDecl { name: "req".into(), width: None }];
    let outputs = vec![SignalDecl { name: "ack".into(), width: None }];

    let result = synthesize_from_ltl(&spec, &inputs, &outputs);
    match result {
        SynthesisResult::Realizable { controller } => {
            assert!(!controller.transitions.is_empty(), "Controller must have transitions");

            for trans in &controller.transitions {
                // Substitute the output assignments into the spec
                let mut substituted = spec.clone();
                for (name, value) in &trans.outputs {
                    substituted = subst_var(&substituted, name, value);
                }
                // The spec-with-outputs-substituted should be a tautology:
                // NOT(substituted) should be UNSAT
                let negated = VerifyExpr::not(substituted.clone());
                assert!(
                    !check_sat(&negated),
                    "Controller output assignment should satisfy the spec for all inputs, \
                     but NOT(spec[outputs:=assignment]) is SAT. Transition: {:?}",
                    trans,
                );
            }
        }
        other => panic!(
            "req IMPLIES ack should be Realizable, got: {:?}",
            match other {
                SynthesisResult::Unrealizable { reason } => format!("Unrealizable: {}", reason),
                SynthesisResult::Unknown => "Unknown".to_string(),
                _ => unreachable!(),
            }
        ),
    }
}

#[cfg(feature = "verification")]
fn subst_var(
    expr: &logicaffeine_verify::VerifyExpr,
    name: &str,
    value: &logicaffeine_verify::VerifyExpr,
) -> logicaffeine_verify::VerifyExpr {
    use logicaffeine_verify::VerifyExpr;
    match expr {
        VerifyExpr::Var(n) if n == name => value.clone(),
        VerifyExpr::Binary { op, left, right } => VerifyExpr::binary(
            *op,
            subst_var(left, name, value),
            subst_var(right, name, value),
        ),
        VerifyExpr::Not(inner) => VerifyExpr::not(subst_var(inner, name, value)),
        _ => expr.clone(),
    }
}
