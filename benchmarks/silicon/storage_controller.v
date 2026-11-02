// storage_controller — the in-order NAND command-stream executor (storage analog of
// dram_executor). The compile-time plan is delta-encoded ([delta:24][op:3][chan:5][die:8]);
// this FSM waits each delta and fires the command pins. Commands can only issue in strict
// word order — there is no queue or scheduling logic in the netlist to do otherwise. The
// countdown arithmetic is proven exact by our certified SAT (see codegen_sva::storage_controller_rtl).
module storage_controller(
  input clk,
  input rst,
  input [63:0] cmd_word,
  input cmd_valid,
  output reg cmd_ack,
  output reg issue,
  output reg [2:0] op,
  output reg [4:0] chan,
  output reg [7:0] die
);
  reg loaded;
  reg [23:0] wait_left;
  reg [2:0] op_l;
  reg [4:0] chan_l;
  reg [7:0] die_l;
  always @(posedge clk) begin
    if (rst) begin
      loaded <= 1'b0; wait_left <= 24'd0; issue <= 1'b0; cmd_ack <= 1'b0;
      op <= 3'd0; chan <= 5'd0; die <= 8'd0; op_l <= 3'd0; chan_l <= 5'd0; die_l <= 8'd0;
    end else begin
      issue <= 1'b0;
      cmd_ack <= 1'b0;
      if (!loaded) begin
        if (cmd_valid) begin
          wait_left <= cmd_word[63:40];
          op_l <= cmd_word[39:37];
          chan_l <= cmd_word[36:32];
          die_l <= cmd_word[31:24];
          loaded <= 1'b1;
          cmd_ack <= 1'b1;
        end
      end else if (wait_left == 24'd0) begin
        issue <= 1'b1;
        op <= op_l; chan <= chan_l; die <= die_l;
        loaded <= 1'b0;
      end else begin
        wait_left <= wait_left - 24'd1;
      end
    end
  end
endmodule
