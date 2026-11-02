// hart_fifo — a first-in first-out queue of 2^DEPTH_BITS entries in one clock domain: `push` when not `full`, `pop`
// when not `empty`, and `head` is the oldest entry whenever there is one. `clear` empties it.
module hart_fifo #(
  parameter WIDTH = 8,
  parameter DEPTH_BITS = 4
) (
  input              clk,
  input              rst,
  input              clear,
  input              push,
  input  [WIDTH-1:0] din,
  input              pop,
  output [WIDTH-1:0] head,
  output             empty,
  output             full
);
  (* ram_style = "registers" *) reg [WIDTH-1:0] mem [0:(1 << DEPTH_BITS) - 1];
  reg [DEPTH_BITS:0] wr, rd;
  assign empty = wr == rd;
  assign full = wr == {~rd[DEPTH_BITS], rd[DEPTH_BITS-1:0]};
  assign head = mem[rd[DEPTH_BITS-1:0]];
  always @(posedge clk) begin
    if (push && !full) mem[wr[DEPTH_BITS-1:0]] <= din;
    if (rst || clear) begin
      wr <= 0;
      rd <= 0;
    end else begin
      if (push && !full) wr <= wr + 1'b1;
      if (pop && !empty) rd <= rd + 1'b1;
    end
  end
endmodule
