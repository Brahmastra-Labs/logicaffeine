use super::*;

#[test]
fn universal_reversible_generators_are_certified_bijections() {
    // CNOT / Toffoli / Fredkin — the universal reversible gate set — are all bijections, and the
    // certified inverse genuinely undoes the gate on every input.
    for (name, table) in [
        ("CNOT", cnot()),
        ("Toffoli", toffoli()),
        ("Fredkin", fredkin()),
    ] {
        let n = table.len().trailing_zeros();
        match certify_gate(n, &table) {
            GateCert::Reversible { inverse } => {
                for (input, &output) in table.iter().enumerate() {
                    assert_eq!(inverse[output], input, "{name}: inverse must undo the gate");
                }
            }
            other => panic!("{name} must be reversible, got {other:?}"),
        }
    }
}

#[test]
fn the_universal_generators_are_involutions() {
    // Each of CNOT, Toffoli, Fredkin is its own inverse — the certified inverse table IS the gate.
    for (name, table) in [
        ("CNOT", cnot()),
        ("Toffoli", toffoli()),
        ("Fredkin", fredkin()),
    ] {
        let n = table.len().trailing_zeros();
        if let GateCert::Reversible { inverse } = certify_gate(n, &table) {
            assert_eq!(inverse, table, "{name} must be an involution");
        } else {
            panic!("{name} must be reversible");
        }
    }
}

#[test]
fn an_erasing_gate_is_rejected_with_its_collision() {
    // AND-into-place `(a,b) ↦ a∧b` erases: inputs 00, 01, 10 all collide to output 0. A
    // non-bijection MUST be caught — the honest irreversibility the adiabatic tier forbids.
    let and_erase: Vec<usize> = (0..4).map(|s| (s & 1) & ((s >> 1) & 1)).collect();
    match certify_gate(2, &and_erase) {
        GateCert::Collision { output, .. } => {
            assert_eq!(output, 0, "the erased outputs collide at 0")
        }
        other => panic!("erasing AND must be rejected, got {other:?}"),
    }
}

#[test]
fn cnot_is_a_certified_linear_reversible_block() {
    // CNOT is LINEAR: out0 = a, out1 = a⊕b — matrix [[1,0],[1,1]] over GF(2), which ∈ GL(2,2).
    assert!(
        certify_linear(2, &[0b01, 0b11]),
        "CNOT's GF(2) matrix must be invertible"
    );
    // A rank-deficient linear map (out1 = out0 = a) erases b — must be rejected.
    assert!(
        !certify_linear(2, &[0b01, 0b01]),
        "a rank-deficient linear map must be rejected"
    );
}
