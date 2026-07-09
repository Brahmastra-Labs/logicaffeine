//! Traffic FLOW / capacity analysis as model checking.
//!
//! An approach's queue is a bit-vector counter; "can it JAM (overflow capacity)?" is a
//! **reachability** question — exactly what our BMC engine answers, no new mathematics. Each
//! model is synthesizable Verilog, so it rides the existing RTL path
//! (`parse_transition_system` → `bmc` / `prove_invariant`) and the queue level renders straight
//! into the waveform.
//!
//! * An approach whose **service keeps up with demand** only ever drains → `prove_invariant`
//!   certifies it never jams (for all time).
//! * An approach that is **under-served** (green only part of the cycle while demand keeps
//!   arriving) grows without bound until it jams → `bmc` finds the exact cycle (the jam trace).
//!
//! `capacity` here is the queue width's range; the `jam` threshold is the level we forbid.

/// A queue served only every other cycle while one vehicle arrives each cycle: net inflow is
/// positive, so the queue climbs to `jam`. BMC finds the cycle it overflows.
pub fn congested_approach(width: u32, jam: u64) -> String {
    let hi = width - 1;
    format!(
        "module flow(input clk);\n\
         \u{20}\u{20}reg [{hi}:0] q;\n\
         \u{20}\u{20}reg phase;\n\
         \u{20}\u{20}initial begin q = {width}'d0; phase = 1'd0; end\n\
         \u{20}\u{20}always @(posedge clk) begin\n\
         \u{20}\u{20}\u{20}\u{20}phase <= ~phase;\n\
         \u{20}\u{20}\u{20}\u{20}if (phase == 1'd1) q <= q + {width}'d1;\n\
         \u{20}\u{20}end\n\
         \u{20}\u{20}assert property (q < {width}'d{jam});\n\
         endmodule\n"
    )
}

/// A queue with a starting backlog and service that exceeds arrivals: it only ever drains, so it
/// provably never reaches `jam` — for all reachable states (k-induction).
pub fn balanced_approach(width: u32, jam: u64, backlog: u64) -> String {
    let hi = width - 1;
    format!(
        "module flow(input clk);\n\
         \u{20}\u{20}reg [{hi}:0] q;\n\
         \u{20}\u{20}initial begin q = {width}'d{backlog}; end\n\
         \u{20}\u{20}always @(posedge clk)\n\
         \u{20}\u{20}\u{20}\u{20}if (q != {width}'d0) q <= q - {width}'d1;\n\
         \u{20}\u{20}assert property (q < {width}'d{jam});\n\
         endmodule\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen_sva::rtl::parse_transition_system;
    use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};

    #[test]
    fn congested_approach_jams_and_bmc_finds_the_cycle() {
        let ts = parse_transition_system(&congested_approach(3, 7))
            .unwrap_or_else(|e| panic!("congested model did not parse: {}", e.message));
        match ts.bmc(24) {
            BmcOutcome::CounterexampleAt { .. } => {}
            other => panic!("an under-served approach must jam, got {other:?}"),
        }
    }

    #[test]
    fn balanced_approach_provably_never_jams() {
        let ts = parse_transition_system(&balanced_approach(3, 7, 5))
            .unwrap_or_else(|e| panic!("balanced model did not parse: {}", e.message));
        assert_eq!(
            ts.prove_invariant(1),
            InductionOutcome::Proven,
            "a draining queue must be PROVEN jam-free"
        );
    }

    #[test]
    fn a_well_sized_queue_does_not_overflow_within_the_horizon() {
        // The same congestion against a larger capacity survives longer before jamming — the
        // sizing lever. Within a short horizon it stays clean.
        let ts = parse_transition_system(&congested_approach(4, 15)).unwrap();
        assert_eq!(
            ts.bmc(8),
            BmcOutcome::NoneWithin(8),
            "a 4-bit queue should not jam within 8 cycles"
        );
    }

    #[test]
    fn jam_is_pushed_later_by_more_capacity() {
        // Wider queue (bigger jam threshold) jams strictly later than a narrow one.
        let narrow = parse_transition_system(&congested_approach(3, 7)).unwrap();
        let wide = parse_transition_system(&congested_approach(4, 15)).unwrap();
        let cycle = |o: BmcOutcome| match o {
            BmcOutcome::CounterexampleAt { k, .. } => k,
            _ => u32::MAX,
        };
        let narrow_k = cycle(narrow.bmc(40));
        let wide_k = cycle(wide.bmc(40));
        assert!(
            narrow_k < wide_k,
            "more capacity must delay the jam: narrow@{narrow_k} vs wide@{wide_k}"
        );
    }
}
