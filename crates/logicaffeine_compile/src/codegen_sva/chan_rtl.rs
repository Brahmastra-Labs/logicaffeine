//! chan_rtl — the deterministic channel in silicon: a bounded FIFO.
//!
//! The language's `Pipe` is a FIFO, bounded, move-semantics channel
//! (`logicaffeine_runtime::channel::Chan`): values leave in the order they entered, a full
//! buffer refuses a sender (`has_room = len < capacity`), an empty buffer refuses a
//! receiver. This module emits that exact discipline as a synthesizable sync FIFO and proves,
//! under iverilog, that replaying an arbitrary send/recv trace produces the same pop sequence
//! as the software channel's bounded `VecDeque` — the channel's determinism, in hardware.
//!
//! It is *one channel's* buffer. The multi-node routing/rendezvous (who is connected to whom,
//! select, blocking hand-off across cores) stays the scheduler/message-passing layer — a
//! projection, not this block.

const DEPTH: usize = 8;

/// Emit the bounded sync FIFO: combinational front-read (`dout`), combinational `full`/`empty`/
/// handshake, registered pointers + count. `pushed = push & !full`, `popped = pop & !empty`.
pub fn emit_chan_fifo_verilog() -> String {
    r#"// chan_fifo — the language Pipe (bounded FIFO channel) in silicon.
// FIFO order, bounded (full refuses a sender, empty refuses a receiver) — the discipline of
// logicaffeine_runtime::channel::Chan. Proven ≡ the software channel by chan_fifo_matches_the_runtime_channel.
// DEPTH = 8, WIDTH = 32.
module chan_fifo(
  input             clk,
  input             rst,
  input             push,
  input  [31:0]     din,
  input             pop,
  output [31:0]     dout,     // combinational: the oldest buffered value (FIFO front)
  output            pushed,   // this cycle's push was accepted (buffer had room)
  output            popped,   // this cycle's pop was accepted (buffer was non-empty)
  output            empty,
  output            full,
  output [3:0]      count
);
  reg [31:0] mem [0:7];
  reg [3:0]  cnt;             // 0..8
  reg [2:0]  head, tail;      // wrap mod 8
  assign empty  = (cnt == 4'd0);
  assign full   = (cnt == 4'd8);
  assign popped = pop  && !empty;
  assign pushed = push && !full;
  assign dout   = mem[head];
  assign count  = cnt;
  always @(posedge clk) begin
    if (rst) begin
      cnt <= 4'd0; head <= 3'd0; tail <= 3'd0;
    end else begin
      if (popped) head <= head + 3'd1;
      if (pushed) begin mem[tail] <= din; tail <= tail + 3'd1; end
      cnt <= cnt - (popped ? 4'd1 : 4'd0) + (pushed ? 4'd1 : 4'd0);
    end
  end
endmodule
"#
    .to_string()
}

/// A self-checking testbench: replay an N-step (push, pop, din) trace and compare the DUT's
/// pop sequence against the software channel oracle from `chan_{ctrl,din,pops}.hex`.
pub fn emit_chan_fifo_testbench(n: usize, npops: usize, final_count: usize) -> String {
    format!(
        r#"// tb_chan_fifo — HW bounded FIFO vs the software channel (VecDeque) oracle.
`timescale 1ns/1ps
module tb_chan_fifo;
  localparam N = {n};
  localparam M = {npops};
  reg clk = 0; always #5 clk = ~clk;
  reg rst = 1, push = 0, pop = 0;
  reg [31:0] din;
  reg [7:0]  ctrl [0:{clast}];
  reg [31:0] dvec [0:{clast}];
  reg [31:0] pops [0:{plast}];
  wire [31:0] dout; wire pushed, popped, empty, full; wire [3:0] count;
  integer i, k, errors;
  chan_fifo dut(.clk(clk), .rst(rst), .push(push), .din(din), .pop(pop),
                .dout(dout), .pushed(pushed), .popped(popped),
                .empty(empty), .full(full), .count(count));
  initial begin
    errors = 0; k = 0;
    $readmemh("chan_ctrl.hex", ctrl);
    $readmemh("chan_din.hex", dvec);
    $readmemh("chan_pops.hex", pops);
    repeat (3) @(posedge clk); #1 rst = 0;
    for (i = 0; i < N; i = i + 1) begin
      push = ctrl[i][0]; pop = ctrl[i][1]; din = dvec[i]; #1;   // settle combinational
      if (popped) begin
        if (k >= M || dout !== pops[k]) begin errors = errors + 1; $display("FAIL pop %0d", k); end
        k = k + 1;
      end
      @(posedge clk); #1;                                       // advance FIFO state
    end
    push = 0; pop = 0; #1;
    if (k !== M) begin errors = errors + 1; $display("FAIL: %0d pops, expected %0d", k, M); end
    if (count !== {final_count}) begin errors = errors + 1; $display("FAIL final count %0d != {final_count}", count); end
    if (errors == 0) $display("PASS: {n}-step FIFO trace matches the software channel oracle (%0d pops)", k);
    else $display("FAIL: %0d FIFO mismatches", errors);
    $finish;
  end
endmodule
"#,
        n = n,
        npops = npops,
        final_count = final_count,
        clast = n - 1,
        plast = if npops == 0 { 0 } else { npops - 1 },
    )
}

/// Reproducible (push, pop, din) trace + the software-channel oracle (bounded VecDeque, FIFO)
/// popped-value sequence. Returns (ctrl.hex, din.hex, pops.hex, npops, final_count).
fn chan_trace(n: usize) -> (String, String, String, usize, usize) {
    use std::collections::VecDeque;
    let mut st: u64 = 0x00F1_F0D0_0DBA_5E12;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st >> 32
    };
    let (mut ctrl, mut din, mut pops) = (String::new(), String::new(), String::new());
    let mut q: VecDeque<u32> = VecDeque::new();
    let mut npops = 0usize;
    for _ in 0..n {
        let r = next();
        // Bias toward keeping the buffer busy: push ~62%, pop ~62% (both can fire).
        let push = (r % 8) < 5;
        let pop = ((r >> 3) % 8) < 5;
        let d = next() as u32;
        ctrl.push_str(&format!("{:02x}\n", (push as u8) | ((pop as u8) << 1)));
        din.push_str(&format!("{:08x}\n", d));
        let do_pop = pop && !q.is_empty();
        let do_push = push && q.len() < DEPTH;
        if do_pop {
            pops.push_str(&format!("{:08x}\n", *q.front().unwrap()));
            q.pop_front();
            npops += 1;
        }
        if do_push {
            q.push_back(d);
        }
    }
    (ctrl, din, pops, npops, q.len())
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/chan_rtl/tests.rs"]
mod tests;
