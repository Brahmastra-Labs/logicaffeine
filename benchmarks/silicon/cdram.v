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

// computedram_model — behavioral ComputeDRAM device: RowClone (2-ACT) + triple-row MAJORITY (3-ACT)
// + AMBIT bulk NOT (op 5, dual-contact cells): mem[bank] := ~mem[row]. NOT completes the boolean
// basis {copy, AND, OR, NOT} so XOR / set-difference / popcount kernels run on this engine.
module computedram_model #(parameter AW=8, DW=64) (
  input clk, input rst,
  input issue, input [2:0] op, input [4:0] bank, input [31:0] row,
  input pre_we, input [AW-1:0] pre_addr, input [DW-1:0] pre_data,
  input [AW-1:0] rd_addr, output [DW-1:0] rd_out
);
  (* ram_style = "registers" *) reg [DW-1:0] mem [0:(1<<AW)-1];
  reg [AW-1:0] r0_i, r1_i;
  reg [DW-1:0] snap0, snap1;
  reg cad_open, pre_int; reg [1:0] ncad;
  assign rd_out = mem[rd_addr];
  function [DW-1:0] maj3(input [DW-1:0] a, input [DW-1:0] b, input [DW-1:0] c);
    maj3 = (a & b) | (a & c) | (b & c);
  endfunction
  always @(posedge clk) begin
    if (pre_we) mem[pre_addr] <= pre_data;   // operand placement (works during reset too)
    if (rst) begin
      cad_open <= 1'b0; pre_int <= 1'b0; ncad <= 2'd0; r0_i <= 0; r1_i <= 0; snap0 <= 0; snap1 <= 0;
    end else if (issue) begin
      case (op)
        3'd0: begin // ACT
          if (cad_open && pre_int && ncad == 2'd1) begin
            // 2nd activation: tentative RowClone (dst := src), snapshot the originals
            r1_i <= row[AW-1:0]; snap1 <= mem[row[AW-1:0]];
            mem[row[AW-1:0]] <= snap0;
            ncad <= 2'd2; pre_int <= 1'b0;
          end else if (cad_open && pre_int && ncad == 2'd2) begin
            // 3rd activation: triple-row majority over the ORIGINAL snapshots → all three rows
            mem[r0_i] <= maj3(snap0, snap1, mem[row[AW-1:0]]);
            mem[r1_i] <= maj3(snap0, snap1, mem[row[AW-1:0]]);
            mem[row[AW-1:0]] <= maj3(snap0, snap1, mem[row[AW-1:0]]);
            ncad <= 2'd3; pre_int <= 1'b0;
          end else begin
            // fresh activation: start a new cadence
            r0_i <= row[AW-1:0]; snap0 <= mem[row[AW-1:0]]; ncad <= 2'd1; cad_open <= 1'b1; pre_int <= 1'b0;
          end
        end
        3'd3: if (cad_open) pre_int <= 1'b1;                               // interrupted precharge
        3'd1, 3'd2: begin cad_open <= 1'b0; pre_int <= 1'b0; ncad <= 2'd0; end // RD/WR closes the cadence
        3'd5: mem[bank[AW-1:0]] <= ~mem[row[AW-1:0]];                      // Ambit bulk NOT: dst(bank) := ~src(row)
        default: ;
      endcase
    end
  end
endmodule
