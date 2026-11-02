`timescale 1ns/1ps
// FailWith (op 91=0x5B) — a failed assertion traps the core: it halts at the failure point (error strobe
// `trapped`), and no later Show runs. `Show 1. Assert that 1 is 2. Show 2.` emits only 1.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] seen [0:3]; integer n = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h05000001;  // r0 = 1
    dut.imem[1] = 32'h59000000;  // Show r0 -> 1
    dut.imem[2] = 32'h05010002;  // r1 = 2
    dut.imem[3] = 32'h1D020001;  // Eq r2 = (r0 == r1) = 0   (op 29=0x1D)
    dut.imem[4] = 32'h24060200;  // JumpIfTrue r2 -> 6       (op 36=0x24)
    dut.imem[5] = 32'h5B000000;  // FailWith -> trap-halt    (op 91=0x5B)
    dut.imem[6] = 32'h05030002;  // [6] r3 = 2 (unreached)
    dut.imem[7] = 32'h59000300;  // Show r3 (unreached)
    dut.imem[8] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (n == 1 && seen[0] == 32'd1 && dut.trapped)
      $display("PASS: soft-core FailWith (emitted only %0d, trapped=%0b)", seen[0], dut.trapped);
    else
      $display("FAIL: n=%0d seen0=%0d trapped=%0b (expected n=1, 1, trapped)", n, seen[0], dut.trapped);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
