// state_grade — a COLLECTION/STRUCT program as a SEQUENCED state-plane controller: a hardwired
// micro-sequencer (no fetch, no decode — the cell sequence IS the program) over ONE shared heap
// (LUTRAM: async read, sync write) with a bump-pointer allocator. Each cell is one step; pure-compute
// cells latch a combinational result, heap cells read/write the heap in program order. This is the
// Futamura projection for STATEFUL code: the residual dataflow-with-memory IS the circuit; proven ==
// the running VM by iverilog (mutable heap is outside QF_BV — eval+iverilog, not SAT). 64-bit wraps.
module state_grade(input clk, input rst, input [63:0] in0, input [63:0] in1, input [63:0] in2, output [63:0] result, output done);
  localparam M = 18;
  localparam HEAP = 64;
  // ram_style=registers: the open FOSS flow (yosys+nextpnr-xilinx) drops write-ported LUT-RAM, so
  // the shared heap + cell registers are forced to real FFs (a small register file) for silicon.
  (* ram_style = "registers" *) reg  [63:0] R [0:M-1];
  (* ram_style = "registers" *) reg  [63:0] heap [0:HEAP-1];
  reg  [63:0] hp;
  reg  [31:0] step;
  reg  running;
  integer j;
  wire [63:0] size0 = 64'sd4;
  wire [63:0] addr1 = R[0];
  wire [63:0] wval1 = 64'sd3;
  wire [63:0] v2 = $signed(R[0]) + $signed(64'sd1);
  wire [63:0] addr3 = R[2];
  wire [63:0] wval3 = in0;
  wire [63:0] v4 = $signed(R[0]) + $signed(64'sd2);
  wire [63:0] addr5 = R[4];
  wire [63:0] wval5 = in1;
  wire [63:0] v6 = $signed(R[0]) + $signed(64'sd3);
  wire [63:0] addr7 = R[6];
  wire [63:0] wval7 = in2;
  wire [63:0] v8 = $signed(R[0]) + $signed(64'sd1);
  wire [63:0] addr9 = R[8];
  wire [63:0] v10 = $signed(R[0]) + $signed(64'sd2);
  wire [63:0] addr11 = R[10];
  wire [63:0] v12 = $signed(R[9]) + $signed(R[11]);
  wire [63:0] v13 = $signed(R[0]) + $signed(64'sd3);
  wire [63:0] addr14 = R[13];
  wire [63:0] v15 = $signed(R[12]) + $signed(R[14]);
  wire [63:0] v16 = ($signed(R[15]) > $signed(64'sd100)) ? 64'sd100 : R[15];
  wire [63:0] v17 = ($signed(R[15]) < $signed(64'sd0)) ? 64'sd0 : R[16];
  always @(posedge clk) begin
    if (rst) begin
      step <= 0; hp <= 0; running <= (M > 0);
      for (j = 0; j < HEAP; j = j + 1) heap[j] <= 64'd0;
    end else if (running) begin
      case (step)
        0: begin R[0] <= hp; hp <= hp + size0; end
        1: begin heap[addr1[5:0]] <= wval1; R[1] <= wval1; if ($signed(addr1) + 64'sd1 > $signed(hp)) hp <= addr1 + 64'sd1; end
        2: R[2] <= v2;
        3: begin heap[addr3[5:0]] <= wval3; R[3] <= wval3; if ($signed(addr3) + 64'sd1 > $signed(hp)) hp <= addr3 + 64'sd1; end
        4: R[4] <= v4;
        5: begin heap[addr5[5:0]] <= wval5; R[5] <= wval5; if ($signed(addr5) + 64'sd1 > $signed(hp)) hp <= addr5 + 64'sd1; end
        6: R[6] <= v6;
        7: begin heap[addr7[5:0]] <= wval7; R[7] <= wval7; if ($signed(addr7) + 64'sd1 > $signed(hp)) hp <= addr7 + 64'sd1; end
        8: R[8] <= v8;
        9: R[9] <= heap[addr9[5:0]];
        10: R[10] <= v10;
        11: R[11] <= heap[addr11[5:0]];
        12: R[12] <= v12;
        13: R[13] <= v13;
        14: R[14] <= heap[addr14[5:0]];
        15: R[15] <= v15;
        16: R[16] <= v16;
        17: R[17] <= v17;
        default: ;
      endcase
      if (step == M - 1) running <= 1'b0;
      else step <= step + 1;
    end
  end
  assign done = !running;
  assign result = R[17];
endmodule
