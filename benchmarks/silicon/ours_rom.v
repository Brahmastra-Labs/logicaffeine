// ours_rom — a block RAM holding a program, streamed out byte by byte.
//
// This is the shape of every real design we care about: the whole-program engine keeps its
// bytecode and its heap in block RAM, and until our engine can configure one, none of them can be
// built. It is also directly observable — the host reads the ROM's contents in order, so a wrong
// `INIT` shows up as the wrong bytes rather than as silence.
//
// The contents are the ADDRESS itself, and the byte sent is the address's HIGH byte. A 2048-deep
// memory then streams 0,0,…,1,1,…,7,7,… as the sweep runs, so the three top address bits are
// visible on the wire — and those are exactly the ones a 36Kb block routes differently from the
// rest. Reading the low byte would prove the data path and say nothing about the address.
module ours_rom (
  input        clk,            // 100 MHz (R4)
  output [7:0] prog_d,
  input        prog_txen,      // unused: writes go out blind, see ours_fifo_blind
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

  reg [10:0] addr = 11'd0;
  always @(posedge clk) if (ring[4]) addr <= addr + 11'd1;

  (* ram_style = "block" *)
  reg [15:0] mem [0:2047];
  integer i;
  initial for (i = 0; i < 2048; i = i + 1) mem[i] = i;

  reg [15:0] word = 16'd0;
  always @(posedge clk) word <= mem[addr];
  assign prog_d = word[15:8];
endmodule
