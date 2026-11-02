// tb_soft_core_divide — prove the shared_divider integration end-to-end: run a real binisa program
// on the modified soft_core that issues Div / Mod / (negative) Div, and check that each retired,
// stalled-until-done result equals the VM's truncated signed div/mod. Instruction word:
// [opcode:8][dst:8][a:8][b:8]; LoadConst uses [5][dst][imm:16].
`timescale 1ns/1ps
module tb_soft_core_divide;
  reg          clk = 0, rst = 1;
  wire [31:0]  out;
  wire         out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  always #5 clk = ~clk;

  reg [31:0] got [0:3];
  integer    n = 0, i;

  initial begin
    #1;  // let the dut's initial ($readmemh boot ROM) settle, then override with our program
    dut.imem[0] = 32'h05000064; // LoadConst r0 = 100
    dut.imem[1] = 32'h05010007; // LoadConst r1 = 7
    dut.imem[2] = 32'h0C020001; // Div  r2 = r0 / r1   -> 14
    dut.imem[3] = 32'h59000200; // Show r2
    dut.imem[4] = 32'h0F030001; // Mod  r3 = r0 % r1   -> 2
    dut.imem[5] = 32'h59000300; // Show r3
    dut.imem[6] = 32'h03000000; // RevNeg r0           -> r0 = -100
    dut.imem[7] = 32'h0C040001; // Div  r4 = r0 / r1   -> -14
    dut.imem[8] = 32'h59000400; // Show r4
    dut.imem[9]  = 32'h0E050001; // FloorDiv r5 = r0 floordiv r1  (-100 floordiv 7 = -15)
    dut.imem[10] = 32'h59000500; // Show r5
    dut.imem[11] = 32'h2B000000; // Halt
    #11 rst = 0;
    for (i = 0; i < 400 && !halted; i = i + 1) begin
      @(posedge clk);
      if (out_valid) begin got[n] = out; n = n + 1; end
    end
    if (n == 4 && got[0] == 32'sd14 && got[1] == 32'sd2 && got[2] == -32'sd14 && got[3] == -32'sd15)
      $display("PASS: soft_core via shared_divider: 100/7=%0d 100%%7=%0d -100/7=%0d -100floordiv7=%0d",
               $signed(got[0]), $signed(got[1]), $signed(got[2]), $signed(got[3]));
    else begin
      $display("FAIL: got %0d outputs: %0d %0d %0d %0d (expected 14 2 -14 -15)",
               n, $signed(got[0]), $signed(got[1]), $signed(got[2]), $signed(got[3]));
    end
    $finish;
  end
endmodule
