`timescale 1ns/1ps
// Load-store batch 6 on the soft-core: the Inspect/When engine. `a new Circle with radius 42` then
// `When Circle (r): Show r` (=42). Exercises NewInductive (tag + payload record), TestArm (tag compare),
// BindArm (payload read). Hand-assembled (== rtl.rs program (18)):
//   0 LoadConst r0=42  1 NewInductive c(r1) ctor=0 count=1  2 TestArm r2=(tag==0)  3 JumpIfFalse r2->7
//   4 BindArm r3=payload0  5 Show r3  6 Halt  7 Halt(unreached)
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0500002A;  // LoadConst r0 = 42
    dut.imem[1] = 32'h4D010001;  // NewInductive c(r1): ctor=0, args_start=0 count=1  (op 77=0x4D)
    dut.imem[2] = 32'h4E020100;  // TestArm r2 = (c's tag == 0)                       (op 78=0x4E)
    dut.imem[3] = 32'h23070200;  // JumpIfFalse r2 -> 7                               (op 35=0x23)
    dut.imem[4] = 32'h4F030100;  // BindArm r3 = c's payload[0]                       (op 79=0x4F)
    dut.imem[5] = 32'h59000300;  // Show r3
    dut.imem[6] = 32'h2B000000;  // Halt
    dut.imem[7] = 32'h2B000000;  // Halt (Rectangle arm — unreached)
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd42)
      $display("PASS: soft-core enum (Circle radius r = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 42)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
