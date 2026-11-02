`timescale 1ns/1ps
// The LOAD-STORE unit on the soft-core: build a list [10,20,30] on the heap, read `item 2 of xs`, Show it.
// Exercises NewList (dmem gather + hp bump), Index (dmem[ptr+i]), the heap the VM's RuntimeValue::List
// mirrors. Hand-assembled chip words (same program as rtl.rs differential program (5), which the Rust
// `run_soft_core` model runs ≡ vm_outcome):
//   0 LoadConst r0=10   1 LoadConst r1=20   2 LoadConst r2=30
//   3 NewList xs(r3)=[r0..r0+2] count=3     4 LoadConst r4=2
//   5 Index r5 = item r4 of xs              6 Show r5           7 Halt
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h0500000A;  // LoadConst r0 = 10
    dut.imem[1] = 32'h05010014;  // LoadConst r1 = 20
    dut.imem[2] = 32'h0502001E;  // LoadConst r2 = 30
    dut.imem[3] = 32'h30030003;  // NewList r3 = [r0..], count=3   (op 48=0x30)
    dut.imem[4] = 32'h05040002;  // LoadConst r4 = 2
    dut.imem[5] = 32'h3B050304;  // Index r5 = item r4 of r3       (op 59=0x3B)
    dut.imem[6] = 32'h59000500;  // Show r5                        (op 89=0x59)
    dut.imem[7] = 32'h2B000000;  // Halt                           (op 43=0x2B)
    #3 rst = 0;
    wait (halted);
    #10;
    if (got && shown == 32'd20)
      $display("PASS: soft-core load-store unit (item 2 of [10,20,30] = %0d)", shown);
    else
      $display("FAIL: got=%0b shown=%0d (expected 20)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
