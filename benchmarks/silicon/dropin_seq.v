// Drop-in SEQUENTIAL demo — the reference design for flip-flop configuration.
//
// The combinational `dropin_demo.v` has 14 LUTs and ZERO flip-flops, which is why our config
// backend has never had to emit FF configuration and silently skipped it. A design that runs LOGOS
// is entirely sequential, so this is the smallest design that exercises every FF config bit our
// backend must generate:
//
//   ZINI       — from each flop's INIT (0 vs 1)
//   ZRST       — reset-to-0 (FDRE/FDCE) vs set-to-1 (FDSE/FDPE)
//   FFSYNC     — synchronous (FDRE/FDSE) vs asynchronous (FDCE/FDPE) set/reset
//   SRUSEDMUX  — a real set/reset net, versus none
//   CEUSEDMUX  — a real clock-enable net, versus none
//   NOCLKINV   — the un-inverted clock (the common case)
//
// It deliberately spans FOUR control sets — {no CE, no SR}, {CE}, {SR}, {CE and SR} — so the
// packer's control-set grouping is exercised too: those flops cannot share a SLICE.
module board_top(
  input        clk,
  input        rst,
  input  [7:0] sw,
  output [7:0] led
);
  // Control set A: plain flops, no enable, no reset.
  reg [1:0] plain = 2'b00;
  always @(posedge clk) plain <= {plain[0], ^sw};

  // Control set B: clock-enable only.
  reg [1:0] enabled = 2'b00;
  always @(posedge clk) if (sw[0]) enabled <= {enabled[0], &sw};

  // Control set C: synchronous reset only (FDRE — reset to 0).
  reg [1:0] resettable = 2'b00;
  always @(posedge clk) if (rst) resettable <= 2'b00; else resettable <= {resettable[0], |sw};

  // Control set D: both enable and synchronous SET (FDSE — set to 1, so ZRST differs).
  reg [1:0] settable = 2'b11;
  always @(posedge clk) if (rst) settable <= 2'b11; else if (sw[1]) settable <= {settable[0], sw[7]};

  // A little combinational depth so there are LUT-to-LUT nets to route as well.
  wire mix = (^sw) ^ (&sw) ^ (|sw);

  assign led = {settable, resettable, enabled, plain[1], plain[0] ^ mix};
endmodule
