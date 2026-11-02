//! crdt_rtl — the CRDT merge unit in silicon.
//!
//! A CRDT's convergence is a **join-semilattice merge**: for a G-Counter the join is the
//! per-replica `max` (and its value is the sum of the merged lanes). That merge is exactly
//! what `logicaffeine_data::crdt::GCounter::merge` computes in software. This module emits a
//! fixed-width (4 replicas × 32 bit) hardware merge unit and proves, under iverilog, that
//! its output is byte-identical to that oracle — so a replica's state converges the same way
//! whether it merged on the CPU runtime or in the chip's coprocessor.
//!
//! It is *one node's* merge (the commutative/associative/idempotent join). The multi-node
//! distribution protocol (who exchanges state with whom) stays the message-passing layer —
//! honestly a projection, not this block.

/// The join for one lane — per-replica `max`, exactly `GCounter::merge`.
pub fn merge_lane(a: u32, b: u32) -> u32 {
    a.max(b)
}

/// Emit the combinational G-Counter merge unit: `merged[i] = max(a[i], b[i])` across the
/// four 32-bit replica lanes, plus `value = sum(merged)` (34 bits: 32 + ceil(log2 4)).
pub fn emit_crdt_merge_verilog() -> String {
    r#"// crdt_merge — the CRDT (G-Counter) join-semilattice merge in silicon.
// merged[i] = max(a[i], b[i]) per replica lane; value = sum(merged).
// Byte-identical to logicaffeine_data::crdt::GCounter::merge (proven by
// crdt_merge_unit_matches_the_gcounter_oracle). 4 replicas x 32-bit.
module crdt_merge(
  input  [127:0] a,        // {a3,a2,a1,a0}, one 32-bit count per replica
  input  [127:0] b,
  output [127:0] merged,   // per-replica max (the lattice join)
  output [33:0]  value     // sum of the four merged lanes
);
  wire [31:0] a0 = a[31:0],  a1 = a[63:32],  a2 = a[95:64],  a3 = a[127:96];
  wire [31:0] b0 = b[31:0],  b1 = b[63:32],  b2 = b[95:64],  b3 = b[127:96];
  wire [31:0] m0 = (a0 >= b0) ? a0 : b0;
  wire [31:0] m1 = (a1 >= b1) ? a1 : b1;
  wire [31:0] m2 = (a2 >= b2) ? a2 : b2;
  wire [31:0] m3 = (a3 >= b3) ? a3 : b3;
  assign merged = {m3, m2, m1, m0};
  assign value  = m0 + m1 + m2 + m3;   // 34-bit LHS ⇒ no truncation
endmodule
"#
    .to_string()
}

/// Emit the **DPU near-data ingest tile**: the same per-lane join as `crdt_merge`, but
/// wrapped in a clocked accumulator — on each `posedge clk` with `in_valid`, the state
/// folds the arriving beat into the running join (`acc <= merge(acc, in_state)`); `rst`
/// clears it. This is the sequential *merge-on-ingest* primitive a DPU runs at the data
/// edge: because the join is commutative/associative/idempotent (certified in
/// `the_crdt_convergence_order_is_a_total_order_proven_by_our_sat`), folding a stream
/// through it is **order-invariant** — correct even when the controller delivers reads
/// out of logical order. Unlike the combinational `crdt_merge` unit, this has genuine
/// register-to-register paths, so it is the block that carries meaning through STA/signoff.
pub fn emit_dpu_ingest_verilog() -> String {
    r#"// dpu_ingest — the DPU near-data compute tile: sequential CRDT merge-on-ingest.
// On each posedge clk with in_valid, acc <= per-replica-max(acc, in_state) — the same
// join-semilattice merge as crdt_merge (proven == GCounter::merge), now folded over a
// stream. Order-invariant by the certified lattice laws, so it is correct even when the
// deterministic controller delivers ingest beats out of logical order. 4 replicas x 32-bit.
module dpu_ingest(
  input          clk,
  input          rst,        // synchronous clear of the accumulator
  input          in_valid,   // a beat of near-data arrived this cycle
  input  [127:0] in_state,   // {s3,s2,s1,s0}, one 32-bit count per replica
  output [127:0] state,      // the accumulated join so far
  output [33:0]  value       // sum of the four accumulated lanes
);
  reg  [127:0] acc;
  wire [31:0] a0 = acc[31:0],      a1 = acc[63:32],      a2 = acc[95:64],      a3 = acc[127:96];
  wire [31:0] b0 = in_state[31:0], b1 = in_state[63:32], b2 = in_state[95:64], b3 = in_state[127:96];
  wire [31:0] m0 = (a0 >= b0) ? a0 : b0;
  wire [31:0] m1 = (a1 >= b1) ? a1 : b1;
  wire [31:0] m2 = (a2 >= b2) ? a2 : b2;
  wire [31:0] m3 = (a3 >= b3) ? a3 : b3;
  always @(posedge clk) begin
    if (rst)           acc <= 128'd0;
    else if (in_valid) acc <= {m3, m2, m1, m0};
  end
  assign state = acc;
  assign value = acc[31:0] + acc[63:32] + acc[95:64] + acc[127:96];  // 34-bit LHS ⇒ no truncation
endmodule
"#
    .to_string()
}

/// A self-checking testbench: apply N (a,b) vectors and compare `merged`/`value` against the
/// oracle from `crdt_{a,b,m}.hex` (computed in Rust). Prints PASS/FAIL.
pub fn emit_crdt_merge_testbench(n: usize) -> String {
    format!(
        r#"// tb_crdt_merge — HW G-Counter merge vs the software CRDT oracle (per-replica max).
`timescale 1ns/1ps
module tb_crdt_merge;
  reg  [127:0] a, b;
  reg  [127:0] oa [0:{last}];
  reg  [127:0] ob [0:{last}];
  reg  [127:0] om [0:{last}];
  wire [127:0] merged;
  wire [33:0]  value;
  reg  [33:0]  ev;
  integer i, errors = 0;
  crdt_merge dut(.a(a), .b(b), .merged(merged), .value(value));
  initial begin
    $readmemh("crdt_a.hex", oa);
    $readmemh("crdt_b.hex", ob);
    $readmemh("crdt_m.hex", om);
    for (i = 0; i < {n}; i = i + 1) begin
      a = oa[i]; b = ob[i]; #1;
      ev = {{2'b0, om[i][31:0]}} + {{2'b0, om[i][63:32]}} + {{2'b0, om[i][95:64]}} + {{2'b0, om[i][127:96]}};
      if (merged !== om[i]) begin errors = errors + 1; $display("FAIL merged %0d", i); end
      if (value  !== ev)    begin errors = errors + 1; $display("FAIL value %0d", i); end
    end
    if (errors == 0) $display("PASS: {n} CRDT merges (per-replica max join + value) match the software oracle");
    else $display("FAIL: %0d CRDT merge mismatches", errors);
    $finish;
  end
endmodule
"#,
        last = n - 1,
        n = n,
    )
}

/// Reproducible test vectors (a, b) and their oracle merge m = per-lane max — a plain LCG so
/// the sequence is stable and needs no external crate.
fn crdt_vectors(n: usize) -> (String, String, String) {
    let mut st: u64 = 0x00C0_FFEE_D0D0_CAFE;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (st >> 33) as u32
    };
    let (mut ha, mut hb, mut hm) = (String::new(), String::new(), String::new());
    for _ in 0..n {
        let a = [next(), next(), next(), next()];
        let b = [next(), next(), next(), next()];
        let m = [
            merge_lane(a[0], b[0]),
            merge_lane(a[1], b[1]),
            merge_lane(a[2], b[2]),
            merge_lane(a[3], b[3]),
        ];
        // {lane3,lane2,lane1,lane0} — lane3 is the most-significant 32 bits (bits [127:96]).
        ha.push_str(&format!(
            "{:08x}{:08x}{:08x}{:08x}\n",
            a[3], a[2], a[1], a[0]
        ));
        hb.push_str(&format!(
            "{:08x}{:08x}{:08x}{:08x}\n",
            b[3], b[2], b[1], b[0]
        ));
        hm.push_str(&format!(
            "{:08x}{:08x}{:08x}{:08x}\n",
            m[3], m[2], m[1], m[0]
        ));
    }
    (ha, hb, hm)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/crdt_rtl/tests.rs"]
mod tests;
