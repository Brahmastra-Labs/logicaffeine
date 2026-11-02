// hart_rom — the boot ROM in block RAM, read through two ports: the instruction fetch's (a word index now, its word
// the next cycle) and the data side's, which reads at the edge E hands an access to M, as the data cache does. Its
// contents are the boot ROM's image, loaded from INIT when the bitstream configures.
module hart_rom #(
  parameter WORDS = 2048,
  parameter INIT = "boot_rom.hex"
) (
  input             clk,
  input             f_en,
  input      [63:0] f_index,
  output reg [63:0] f_word,
  input             d_en,
  input      [63:0] d_index,
  output reg [63:0] d_word
);
  localparam BITS = $clog2(WORDS);
  (* ram_style = "block" *) reg [63:0] mem [0:WORDS - 1];
  initial $readmemh(INIT, mem);
  always @(posedge clk) begin
    if (f_en) f_word <= mem[f_index[BITS-1:0]];
    if (d_en) d_word <= mem[d_index[BITS-1:0]];
  end
endmodule
