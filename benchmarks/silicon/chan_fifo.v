// chan_fifo — the language Pipe (bounded FIFO channel) in silicon.
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
