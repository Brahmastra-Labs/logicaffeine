// ours_fifo_blind — the smallest thing on Board A that a host can SEE.
//
// `ours_fifo_tx` couples three things at once: the FT245 handshake, an FSM built out of LUTs, and
// the flip-flops that drive the data bus. When nothing comes out, all three are suspects. This
// removes two of them. It never reads TXE#: it simply strobes WR# once every eight cycles and lets
// the FIFO drop what it cannot take, which is fine when the only question is whether ANY byte
// reaches the host.
//
// The ring is initialised with a single LOW bit, so `prog_wrn` is the ring bit itself — an
// active-low strobe with no inverter in the way. The only combinational logic in the design is the
// one inverter feeding `tog`.
module ours_fifo_blind (
  input        clk,            // 100 MHz (R4)
  output [7:0] prog_d,
  input        prog_txen,      // deliberately unused: this design does not look before it writes
  output       prog_wrn,
  output       prog_rdn,
  output       prog_oen,
  output       prog_siwun,
  output       prog_spien
);
  assign prog_rdn   = 1'b1;    // we never read
  assign prog_oen   = 1'b1;    // so the FTDI must never drive the bus
  assign prog_siwun = 1'b1;
  assign prog_spien = 1'b1;    // select DPTI (the parallel FIFO)

  reg [7:0] ring = 8'b1111_1110;
  always @(posedge clk) ring <= {ring[6:0], ring[7]};
  assign prog_wrn = ring[0];   // low for one cycle in eight — 10 ns, past the FT245's minimum

  reg tog = 1'b0;
  always @(posedge clk) if (ring[4]) tog <= ~tog;
  assign prog_d = {8{tog}};    // 0x00 / 0xFF, changing while WR# is high
endmodule
