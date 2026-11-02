// hart_sram — the scratch RAM in block RAM: the boot ROM's stack and data before the DDR2 is calibrated. Like the data
// cache it reads at the edge E hands an access to M and writes at the end of M's cycle, so every access completes in
// the cycle M presents it; a store and the next access's read of the same word at one edge are merged the same way.
module hart_sram #(
  parameter WORDS = 1024
) (
  input             clk,
  input             rst,
  input             pre_valid,
  input      [63:0] pre_index,
  input             req,
  input      [63:0] index,
  input             we,
  input      [7:0]  be,
  input      [63:0] wdata,
  output     [63:0] rdata
);
  localparam BITS = $clog2(WORDS);
  wire [63:0] data_out;
  reg        merge;
  reg [7:0]  merge_be;
  reg [63:0] merge_data;
  wire       store = req && we;

  genvar mi;
  generate for (mi = 0; mi < 8; mi = mi + 1) begin : m
    assign rdata[8 * mi +: 8] = (merge && merge_be[mi]) ? merge_data[8 * mi +: 8] : data_out[8 * mi +: 8];
  end endgenerate

  // The words in eight byte-wide block RAMs, so a store's bytes are each a plain write.
  genvar bl;
  generate for (bl = 0; bl < 8; bl = bl + 1) begin : lane
    (* ram_style = "block" *) reg [7:0] bytes [0:WORDS - 1];
    reg [7:0] out;
    // Block RAM configures to zero, as the board model's scratch RAM starts.
    integer w;
    initial for (w = 0; w < WORDS; w = w + 1) bytes[w] = 8'h00;
    always @(posedge clk) begin
      if (pre_valid) out <= bytes[pre_index[BITS-1:0]];
      if (store && be[bl]) bytes[index[BITS-1:0]] <= wdata[8 * bl +: 8];
    end
    assign data_out[8 * bl +: 8] = out;
  end endgenerate

  always @(posedge clk) begin
    if (rst) merge <= 1'b0;
    else if (pre_valid) begin
      merge <= store && pre_index[BITS-1:0] == index[BITS-1:0];
      merge_be <= be;
      merge_data <= wdata;
    end
  end
endmodule
