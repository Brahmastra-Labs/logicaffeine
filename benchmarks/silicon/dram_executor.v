// dram_executor — the rung-3 in-order command-stream executor (COLD_HARD_FACTS Wave 3).
// The compile-time plan is delta-encoded ([delta:24][op:3][bank:5][row:32]); this FSM
// waits each delta and fires the command pins. Commands can only issue in strict word
// order — there is no queue or scheduling logic in the netlist to do otherwise. The
// countdown arithmetic is proven exact by our certified SAT (see codegen_sva::dram).
// cmd_ack is a FIFO-POP pulse asserted the cycle a word is CONSUMED (loaded), not the
// cycle it later issues — so the producer's read index advances immediately and a
// zero-delay command can never reload itself (the classic handshake-latency trap).
module dram_executor(
  input clk,
  input rst,
  input [63:0] cmd_word,
  input cmd_valid,
  output reg cmd_ack,
  output reg issue,
  output reg [2:0] op,
  output reg [4:0] bank,
  output reg [31:0] row
);
  reg loaded;
  reg [23:0] wait_left;
  reg [2:0] op_l;
  reg [4:0] bank_l;
  reg [31:0] row_l;
  always @(posedge clk) begin
    if (rst) begin
      loaded <= 1'b0; wait_left <= 24'd0; issue <= 1'b0; cmd_ack <= 1'b0;
      op <= 3'd0; bank <= 5'd0; row <= 32'd0; op_l <= 3'd0; bank_l <= 5'd0; row_l <= 32'd0;
    end else begin
      issue <= 1'b0;
      cmd_ack <= 1'b0;
      if (!loaded) begin
        if (cmd_valid) begin
          wait_left <= cmd_word[63:40];
          op_l <= cmd_word[39:37];
          bank_l <= cmd_word[36:32];
          row_l <= cmd_word[31:0];
          loaded <= 1'b1;
          cmd_ack <= 1'b1;
        end
      end else if (wait_left == 24'd0) begin
        issue <= 1'b1;
        op <= op_l; bank <= bank_l; row <= row_l;
        loaded <= 1'b0;
      end else begin
        wait_left <= wait_left - 24'd1;
      end
    end
  end
endmodule
