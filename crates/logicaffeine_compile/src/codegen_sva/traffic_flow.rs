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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/traffic_flow/tests.rs"]
mod tests;
