`timescale 1ns/1ps
// The soft-core executes a REAL LOGOS program from instruction memory: the loop
//     Let sum be 0. Let i be 1. While i <= 5: Set sum to sum + i. Set i to i + 1. Show sum.
// hand-assembled to `binisa`-opcode chip words. Expected Show output: 15.
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));

  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end

  initial begin
    // The program is the boot ROM (`program.hex`, loaded by soft_core's $readmemh) — the same image
    // synthesis bakes in. Chip words: r0=sum r1=i r2=limit(5) r3=one(1) r4=cond ;  {opcode,dst,a,b}:
    //   05000000 LoadConst r0,0   05010001 r1,1   05020005 r2,5   05030001 r3,1
    //   1B040102 LtEq r4=(i<=5)   23090400 JumpIfFalse r4->9   08000001 Add r0=sum+i
    //   08010103 Add r1=i+1       22040000 Jump->4   59000000 Show r0   2B000000 Halt
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd15)
      $display("PASS: soft-core executed the loop end-to-end (sum 1..5 = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 15)", got, shown);
    $finish;
  end

  initial begin #20000 $display("FAIL: timeout (core did not halt)"); $finish; end
endmodule
