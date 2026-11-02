// coproc_core — a minimal core that ISSUES coprocessor instructions to the real
// synthesized blocks (bigint_alu / bigint_mul / crdt_merge) and RETIRES results to a register
// file: fetch (pc->rom) -> decode (op/rd/rs/rt) -> route R[rs]/R[rt] to the coprocessor ->
// latch to R[rd] -> advance pc. Proven == the software oracle by iverilog. op: 0=HALT 1=BADD
// 2=BSUB 3=BMUL 4=CMERGE. Read back the register file via rsel/rout after halt.
module coproc_core(input clk, input rst, input [2:0] rsel, output [255:0] rout, output halted);
  reg [255:0] R [0:7];
  reg [3:0]   pc;
  reg         done;
  integer j;

  // Combinational instruction ROM + decode.
  reg [12:0] instr;
  always @(*) begin
    case (pc)
      4'd0: instr = {4'd1, 3'd2, 3'd0, 3'd1};
      4'd1: instr = {4'd2, 3'd3, 3'd0, 3'd1};
      4'd2: instr = {4'd3, 3'd4, 3'd0, 3'd1};
      4'd3: instr = {4'd4, 3'd5, 3'd0, 3'd1};
      4'd4: instr = {4'd0, 3'd0, 3'd0, 3'd0};
      default: instr = {4'd0, 9'd0}; // HALT
    endcase
  end
  wire [3:0] op = instr[12:9];
  wire [2:0] rd = instr[8:6];
  wire [2:0] rs = instr[5:3];
  wire [2:0] rt = instr[2:0];

  // The coprocessor functional units, fed the decoded source registers.
  wire [255:0] alu_sum, alu_diff;
  wire         alu_cout, alu_borrow, alu_eq, alu_lt;
  bigint_alu u_alu(.a(R[rs]), .b(R[rt]), .sum(alu_sum), .cout(alu_cout),
                   .diff(alu_diff), .borrow(alu_borrow), .eq(alu_eq), .lt(alu_lt));
  wire [127:0] mul_prod;
  bigint_mul u_mul(.a(R[rs][63:0]), .b(R[rt][63:0]), .prod(mul_prod));
  wire [127:0] cm_merged;
  wire [33:0]  cm_value;
  crdt_merge u_cm(.a(R[rs][127:0]), .b(R[rt][127:0]), .merged(cm_merged), .value(cm_value));

  assign rout   = R[rsel];
  assign halted = done;

  always @(posedge clk) begin
    if (rst) begin
      pc <= 4'd0; done <= 1'b0;
      R[0] <= 256'h000000000000000300000000000000021111111111111111ffffffffffffffff; R[1] <= 256'h0000000000000001000000000000000100000000000000010000000000000001;
      for (j = 2; j < 8; j = j + 1) R[j] <= 256'd0;
    end else if (!done) begin
      case (op)
        4'd0: done <= 1'b1;                    // HALT
        4'd1: R[rd] <= alu_sum;                // BADD
        4'd2: R[rd] <= alu_diff;               // BSUB
        4'd3: R[rd] <= {128'd0, mul_prod};   // BMUL
        4'd4: R[rd] <= {128'd0, cm_merged};  // CMERGE
        default: ;
      endcase
      if (op != 4'd0) pc <= pc + 4'd1;
    end
  end
endmodule
