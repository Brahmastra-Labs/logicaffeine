//! Designer → certified controller: turn a synthesized [`PhasePlan`] into a Verilog phase-FSM and
//! let the existing RTL pipeline (`parse_transition_system` → `prove_invariant`) certify it.
//!
//! The controller cycles a `phase` counter; on entering each phase it greens *exactly* that
//! phase's movements and reds the rest. Because the plan is a conflict-free colouring, no
//! conflicting pair is ever green together — and because every transition fully rewrites all
//! movement registers to a (safe) phase configuration, the safety property is **1-inductive**, so
//! `prove_invariant(1)` proves it with no extra invariant strengthening. This closes the loop:
//! design (SAT) → generate (codegen) → prove (k-induction), entirely on our own stack.

use super::signal_design::{Intersection, PhasePlan};
use std::fmt::Write as _;

/// Generate a synthesizable Verilog controller (one 1-bit `mvN` register per movement, a `phase`
/// counter) whose conflict-freedom is provable by `prove_invariant`.
pub fn generate_controller(it: &Intersection, plan: &PhasePlan) -> String {
    let n = it.movements.len();
    let k = plan.num_phases.max(1);
    let groups = plan.groups();
    let bits = phase_bits(k);

    let mut v = String::new();
    let _ = writeln!(v, "module controller(input clk);");
    let _ = writeln!(v, "  reg [{}:0] phase;", bits - 1);
    for i in 0..n {
        let _ = writeln!(v, "  reg mv{i};");
    }
    let _ = writeln!(
        v,
        "  initial begin phase = {bits}'d0; {} end",
        config_assigns(&groups, 0, n, true)
    );
    let _ = writeln!(v, "  always @(posedge clk)");
    if k == 1 {
        // No conflicts: a single phase greens everything, every cycle.
        let _ = writeln!(
            v,
            "    begin phase <= {bits}'d0; {} end",
            config_assigns(&groups, 0, n, false)
        );
    } else {
        for p in 0..(k - 1) {
            let kw = if p == 0 { "if" } else { "else if" };
            let _ = writeln!(
                v,
                "    {kw} (phase == {bits}'d{p}) begin phase <= {bits}'d{}; {} end",
                p + 1,
                config_assigns(&groups, p + 1, n, false)
            );
        }
        // Last phase (and any unreachable out-of-range counter value) wraps to phase 0.
        let _ = writeln!(
            v,
            "    else begin phase <= {bits}'d0; {} end",
            config_assigns(&groups, 0, n, false)
        );
    }
    let _ = writeln!(v, "  assert property ({});", conflict_free_property(it));
    let _ = writeln!(v, "endmodule");
    v
}

/// Assign every movement register for phase `p`: `1` if served in that phase, else `0`.
/// `blocking` selects `=` (an `initial` block) vs `<=` (an `always` block).
fn config_assigns(groups: &[Vec<usize>], p: usize, n: usize, blocking: bool) -> String {
    let op = if blocking { "=" } else { "<=" };
    let served: std::collections::HashSet<usize> =
        groups.get(p).map(|g| g.iter().copied().collect()).unwrap_or_default();
    (0..n)
        .map(|i| format!("mv{i} {op} 1'd{};", usize::from(served.contains(&i))))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The safety property: no conflicting pair is green together. A conflict-free intersection has
/// nothing to violate, so a tautology stands in.
fn conflict_free_property(it: &Intersection) -> String {
    if it.conflicts.is_empty() {
        "mv0 | ~mv0".to_string()
    } else {
        it.conflicts
            .iter()
            .map(|&(a, b)| format!("~(mv{a} & mv{b})"))
            .collect::<Vec<_>>()
            .join(" & ")
    }
}

/// Bits needed for a phase counter over `k` phases (values `0..k`).
fn phase_bits(k: usize) -> usize {
    let mut bits = 1;
    while (1usize << bits) < k {
        bits += 1;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen_sva::rtl::parse_transition_system;
    use crate::codegen_sva::signal_design::design_phase_plan;
    use logicaffeine_proof::bmc::InductionOutcome;

    fn graph(n: usize, conflicts: &[(usize, usize)]) -> Intersection {
        Intersection {
            movements: (0..n).map(|i| format!("m{i}")).collect(),
            conflicts: conflicts.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect(),
        }
    }

    /// The crown jewel: design (SAT) → generate (codegen) → prove (k-induction), all green.
    #[test]
    fn generated_controller_is_proven_conflict_free() {
        let graphs = [
            graph(3, &[]),                                        // conflict-free → 1 phase
            graph(2, &[(0, 1)]),                                  // 2 phases
            graph(3, &[(0, 1), (1, 2), (0, 2)]),                  // triangle → 3
            graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]),          // even cycle → 2
            graph(5, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)]),  // odd cycle → 3
            graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]), // K4 → 4
            graph(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]),          // star → 2
        ];
        for g in graphs {
            let plan = design_phase_plan(&g).unwrap();
            let verilog = generate_controller(&g, &plan);
            let ts = parse_transition_system(&verilog)
                .unwrap_or_else(|e| panic!("generated controller did not parse: {}\n{verilog}", e.message));
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
        assert!(v.contains("~(mv0 & mv1)") && v.contains("~(mv1 & mv2)") && v.contains("~(mv0 & mv2)"), "{v}");
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
        let ts = parse_transition_system(&v)
            .unwrap_or_else(|e| panic!("single-movement controller did not parse: {}\n{v}", e.message));
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven, "{v}");
    }

    #[test]
    fn six_movement_controller_is_proven() {
        // A denser graph than the loop covers: 6 movements, two triangles sharing nothing.
        let g = graph(6, &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)]);
        let plan = design_phase_plan(&g).unwrap();
        assert_eq!(plan.num_phases, 3, "two disjoint triangles still need 3 phases");
        let v = generate_controller(&g, &plan);
        let ts = parse_transition_system(&v)
            .unwrap_or_else(|e| panic!("did not parse: {}\n{v}", e.message));
        assert_eq!(ts.prove_invariant(1), InductionOutcome::Proven, "{v}");
    }
}
