use super::*;
use crate::codegen_sva::rtl::parse_transition_system;
use crate::codegen_sva::signal_design::design_phase_plan;
use logicaffeine_proof::bmc::InductionOutcome;

fn graph(n: usize, conflicts: &[(usize, usize)]) -> Intersection {
    Intersection {
        movements: (0..n).map(|i| format!("m{i}")).collect(),
        conflicts: conflicts
            .iter()
            .map(|&(a, b)| (a.min(b), a.max(b)))
            .collect(),
    }
}

/// The crown jewel: design (SAT) → generate (codegen) → prove (k-induction), all green.
#[test]
fn generated_controller_is_proven_conflict_free() {
    let graphs = [
        graph(3, &[]),                                               // conflict-free → 1 phase
        graph(2, &[(0, 1)]),                                         // 2 phases
        graph(3, &[(0, 1), (1, 2), (0, 2)]),                         // triangle → 3
        graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]),                 // even cycle → 2
        graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),         // odd cycle → 3
        graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]), // K4 → 4
        graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),                 // star → 2
    ];
    for g in graphs {
        let plan = design_phase_plan(&g).unwrap();
        let verilog = generate_controller(&g, &plan);
        let ts = parse_transition_system(&verilog).unwrap_or_else(|e| {
            panic!(
                "generated controller did not parse: {}\n{verilog}",
                e.message
            )
        });
        assert_eq!(
            ts.prove_invariant(1),
            InductionOutcome::Proven,
            "generated controller must be PROVEN conflict-free:\n{verilog}"
        );
    }
}

#[test]
fn generated_verilog_is_well_formed() {
    let g = graph(3, &[(0, 1), (1, 2), (0, 2)]);
    let plan = design_phase_plan(&g).unwrap();
    let v = generate_controller(&g, &plan);
    assert!(v.contains("module controller(input clk);"), "{v}");
    assert!(v.contains("endmodule"));
    assert!(v.contains("assert property"));
    for i in 0..3 {
        assert!(v.contains(&format!("reg mv{i};")), "missing mv{i}:\n{v}");
    }
    // The triangle's safety property names all three conflicts.
    assert!(
        v.contains("~(mv0 & mv1)") && v.contains("~(mv1 & mv2)") && v.contains("~(mv0 & mv2)"),
        "{v}"
    );
}

#[test]
fn a_deliberately_unsafe_controller_is_caught() {
    // Sanity that the property has teeth: hand-mangle the generated RTL so two conflicting
    // movements are both green, and confirm the prover rejects it.
    let g = graph(2, &[(0, 1)]);
    let plan = design_phase_plan(&g).unwrap();
    let good = generate_controller(&g, &plan);
    // Force every phase to green BOTH movements.
    let bad = good
        .replace("mv0 <= 1'd0;", "mv0 <= 1'd1;")
        .replace("mv1 <= 1'd0;", "mv1 <= 1'd1;")
        .replace("mv0 = 1'd0;", "mv0 = 1'd1;")
        .replace("mv1 = 1'd0;", "mv1 = 1'd1;");
    assert_ne!(bad, good, "the mangle must change something");
    let ts = parse_transition_system(&bad).expect("still parses");
    assert_ne!(
        ts.prove_invariant(1),
        InductionOutcome::Proven,
        "a both-green controller must NOT prove safe:\n{bad}"
    );
}

#[test]
fn phase_bits_are_correct() {
    assert_eq!(phase_bits(1), 1);
    assert_eq!(phase_bits(2), 1);
    assert_eq!(phase_bits(3), 2);
    assert_eq!(phase_bits(4), 2);
    assert_eq!(phase_bits(5), 3);
}

#[test]
fn single_movement_controller_is_valid_and_proven() {
    let g = graph(1, &[]);
    let plan = design_phase_plan(&g).unwrap();
    let v = generate_controller(&g, &plan);
    let ts = parse_transition_system(&v).unwrap_or_else(|e| {
        panic!(
            "single-movement controller did not parse: {}\n{v}",
            e.message
        )
    });
    assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven, "{v}");
}

#[test]
fn six_movement_controller_is_proven() {
    // A denser graph than the loop covers: 6 movements, two triangles sharing nothing.
    let g = graph(6, &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)]);
    let plan = design_phase_plan(&g).unwrap();
    assert_eq!(
        plan.num_phases, 3,
        "two disjoint triangles still need 3 phases"
    );
    let v = generate_controller(&g, &plan);
    let ts =
        parse_transition_system(&v).unwrap_or_else(|e| panic!("did not parse: {}\n{v}", e.message));
    assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven, "{v}");
}
