`timescale 1ns/1ps
// TaskAwait (op 100=0x64) — the instant JOIN. SpawnHandle runs the child (returns), TaskAwait joins it,
// then Show 7 — control passes THROUGH the await. (The awaited result is the VM's Nothing, not observed.)
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h63000500;  // SpawnHandle handle=r0, entry=5    (op 99=0x63)
    dut.imem[1] = 32'h64010000;  // TaskAwait handle=r0 -> join       (op 100=0x64)
    dut.imem[2] = 32'h05020007;  // r2 = 7
    dut.imem[3] = 32'h59000200;  // Show r2
    dut.imem[4] = 32'h2B000000;  // Halt
    dut.imem[5] = 32'h2A000000;  // [5] child: ReturnNothing          (op 42=0x2A)
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd7) $display("PASS: soft-core TaskAwait join (control passed through = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 7)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
