`timescale 1ns/1ps
// CheckPolicy (op 44=0x2C) GRANT — a boolean-capability check grants iff the subject's capability field is
// truthy. verified=true (field=1) -> the check passes, control continues, Show 1. trapped stays 0.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h4A000001;  // NewStruct u(r0), 1 field   (op 74=0x4A)
    dut.imem[1] = 32'h05010001;  // r1 = 1 (verified = true)
    dut.imem[2] = 32'h4B000100;  // StructInsert u.field0 = r1 (op 75=0x4B)
    dut.imem[3] = 32'h2C000000;  // CheckPolicy subject=r0, field 0 -> GRANT (op 44=0x2C)
    dut.imem[4] = 32'h05020001;  // r2 = 1
    dut.imem[5] = 32'h59000200;  // Show r2
    dut.imem[6] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd1 && !dut.trapped)
      $display("PASS: soft-core CheckPolicy GRANT (Show=%0d, trapped=%0b)", shown, dut.trapped);
    else $display("FAIL: got=%0b shown=%0d trapped=%0b (expected 1, not trapped)", got, shown, dut.trapped);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
