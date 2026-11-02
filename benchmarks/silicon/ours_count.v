// ours_count — a counter, which is the smallest design that needs a CARRY chain.
//
// Every real design has one: `synth_xilinx` turns `+ 1` into `CARRY4` primitives, 534 of them in
// the whole-program engine alone. Until our engine can place and configure them, nothing with
// arithmetic in it can be built at all.
//
// Observable end to end: the byte on the bus IS the counter, so a host reading the FIFO sees the
// value increase. A stuck or mis-chained carry shows up immediately as a byte that stops counting
// at a bit boundary — 15, 127 — which is exactly the failure a carry chain has.
module ours_count (
  input        clk,            // 100 MHz (R4)
  output [7:0] prog_d,
  input        prog_txen,      // unused: this design writes blind, see ours_fifo_blind
  output       prog_wrn,
  output       prog_rdn,
  output       prog_oen,
  output       prog_siwun,
  output       prog_spien
);
  assign prog_rdn   = 1'b1;
  assign prog_oen   = 1'b1;
  assign prog_siwun = 1'b1;
  assign prog_spien = 1'b1;

  reg [7:0] ring = 8'b1111_1110;
  always @(posedge clk) ring <= {ring[6:0], ring[7]};
  assign prog_wrn = ring[0];

  reg [7:0] count = 8'd0;
  always @(posedge clk) if (ring[4]) count <= count + 8'd1;
  assign prog_d = count;
endmodule
