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
    let served: std::collections::HashSet<usize> = groups
        .get(p)
        .map(|g| g.iter().copied().collect())
        .unwrap_or_default();
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/controller_gen/tests.rs"]
mod tests;
