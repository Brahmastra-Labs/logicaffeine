use super::*;
use logicaffeine_proof::revgate::{cnot, fredkin, toffoli};

#[test]
fn in_place_and_is_injective_with_the_mask_ancilla() {
    // CORNER 5 (phase 1) — the hardware twin of corner-1 minimal-bit journaling. The VM's in-place
    // `a = a & b` (MaskReconstruct) keeps `(a&b, b)` and ERASES a's bit wherever `b = 0`. AUGMENTED
    // with the ancilla `saved = a & ~b` (exactly the journaled bits), the map (a,b) ↦ (a&b, b, saved)
    // is INJECTIVE — the exhaustive hardware proof that the minimal-bit journal SUFFICES to reverse
    // the erasing AND. (1-bit a = bit0, 1-bit b = bit1; augmented output a&b=bit0, b=bit1, saved=bit2.)
    let augmented: Vec<usize> = (0..4)
        .map(|x| {
            let a = x & 1;
            let b = (x >> 1) & 1;
            let post = (a & b) | (b << 1); // kept post-state
            let ancilla = a & (1 - b); // saved = a where b = 0
            post | (ancilla << 2)
        })
        .collect();
    let ob = discharge_journaled_injective("and_inplace", 2, &augmented)
        .expect("the mask ancilla makes in-place AND injective");
    for x in 0..4 {
        assert_eq!(
            ob.reverse[&augmented[x]], x,
            "the reverse module must reconstruct input {x}"
        );
    }
    assert!(
        ob.sva_property.contains("injective"),
        "emits the injectivity obligation"
    );
}

#[test]
fn in_place_and_without_ancilla_is_irreversible() {
    // WITHOUT the ancilla, in-place AND erases: (a=1,b=0) and (a=0,b=0) both map to (0,0). The
    // obligation FAILS LOUD — the hardware twin of "a lossy op needs its Bennett history bits".
    let no_ancilla: Vec<usize> = (0..4)
        .map(|x| {
            let a = x & 1;
            let b = (x >> 1) & 1;
            (a & b) | (b << 1) // just (a&b, b) — no ancilla
        })
        .collect();
    let err = discharge_journaled_injective("and_no_ancilla", 2, &no_ancilla).unwrap_err();
    assert!(
        err.contains("NOT injective") && err.contains("erasure"),
        "an insufficient ancilla must fail loud, got: {err}"
    );
}

#[test]
fn in_place_shift_is_injective_with_the_dropped_bits_ancilla() {
    // The ShiftReconstruct twin: in-place `a = a >> 1` keeps `a>>1` and erases the low bit. AUGMENTED
    // with the dropped low bit as ancilla, (a) ↦ (a>>1, a&1) is injective — every 2-bit `a` round-trips.
    let augmented: Vec<usize> = (0..4)
        .map(|a| {
            let post = a >> 1; // kept quotient (bits 0..1)
            let dropped = a & 1; // the erased low bit
            post | (dropped << 2) // ancilla at bit2
        })
        .collect();
    let ob = discharge_journaled_injective("shr_inplace", 2, &augmented)
        .expect("the dropped-bit ancilla makes in-place SHR injective");
    for a in 0..4 {
        assert_eq!(
            ob.reverse[&augmented[a]], a,
            "reverse reconstructs a = {a} from (a>>1, a&1)"
        );
    }
}

#[test]
fn a_reversible_sequential_transition_is_a_permutation() {
    // CORNER 5 (phase 2) — SEQUENTIAL reversibility. A CLOCKED unit is reversible iff its next-state
    // function is a PERMUTATION of the state space (its transition graph is disjoint cycles) — the
    // same reverse-module obligation as a combinational unit, now on the NEXT-STATE table. A 3-bit
    // reversible counter `s := (s+1) mod 8` has next-state [1,2,…,7,0]; its reverse module is one
    // full REVERSE clock, the decrement.
    let counter_next: Vec<usize> = (0..8).map(|s| (s + 1) % 8).collect();
    let ob = discharge_reverse_module("counter", 3, &counter_next, None)
        .expect("a reversible counter's clocked transition is a permutation");
    assert_eq!(
        ob.reverse,
        (0..8).map(|s| (s + 7) % 8).collect::<Vec<_>>(),
        "the reverse module is the decrement — one reverse clock"
    );
    // An ERASING sequential unit `s := s & 6` collapses states (0,1 ↦ 0; 2,3 ↦ 2; …) — NOT a
    // permutation, so no reverse clock exists and the obligation fails loud.
    let erasing_next: Vec<usize> = (0..8).map(|s| s & 6).collect();
    let err = discharge_reverse_module("erasing_reg", 3, &erasing_next, None).unwrap_err();
    assert!(
        err.contains("irreversible") && err.contains("erasure"),
        "got: {err}"
    );
}

#[test]
fn reverse_module_obligation_discharges_for_the_universal_gates() {
    // CNOT / Toffoli / Fredkin each discharge the obligation: exhaustive bit-blast finds the
    // reverse module, and (for linear CNOT) the GF(2) certificate agrees. Each is an involution,
    // so its reverse module equals the gate, and R(U(x)) == x holds for every input.
    let cases: [(&str, Vec<usize>, Option<Vec<u64>>); 3] = [
        ("cnot", cnot(), Some(vec![0b01, 0b11])),
        ("toffoli", toffoli(), None),
        ("fredkin", fredkin(), None),
    ];
    for (name, table, rows) in cases {
        let ob =
            discharge_reverse_module(name, table.len().trailing_zeros(), &table, rows.as_deref())
                .unwrap_or_else(|e| panic!("{name} must discharge: {e}"));
        assert_eq!(
            ob.reverse, table,
            "{name} is an involution — its reverse module is itself"
        );
        assert!(
            ob.sva_property.contains("reverse_module"),
            "{name} emits the SVA obligation"
        );
        for (x, &u) in table.iter().enumerate() {
            assert_eq!(ob.reverse[u], x, "{name}: R(U({x})) must equal {x}");
        }
    }
}

#[test]
fn reverse_module_obligation_rejects_an_erasing_unit() {
    // AND-into-place erases (00,01,10 ↦ 0) — no reverse module exists; the obligation fails loud.
    let and_erase: Vec<usize> = (0..4).map(|s| (s & 1) & ((s >> 1) & 1)).collect();
    let err = discharge_reverse_module("and", 2, &and_erase, None).unwrap_err();
    assert!(
        err.contains("irreversible") && err.contains("erasure"),
        "got: {err}"
    );
}

#[test]
fn reverse_module_obligation_catches_a_method_disagreement() {
    // The AAA bar: CNOT's truth table IS a permutation (exhaustive says reversible), but fed a
    // SINGULAR GF(2) matrix the two methods DISAGREE — and that must be caught, never rubber-stamped.
    let err = discharge_reverse_module("cnot", 2, &cnot(), Some(&[0b01, 0b01])).unwrap_err();
    assert!(
        err.contains("DISAGREE"),
        "a method disagreement must fail loud, got: {err}"
    );
}
