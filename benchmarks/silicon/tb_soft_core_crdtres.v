`timescale 1ns/1ps
// CrdtResolve (op 84=0x54) — register conflict resolution; single-replica last-write-wins = field
// overwrite. Set title=10, Resolve title to 42, read back = 42.
module tb;
  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;
  soft_core dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  reg [31:0] shown; reg got = 0;
  always #5 clk = ~clk;
  always @(posedge clk) if (out_valid) begin shown <= out; got <= 1; end
  initial begin
    #1;
    dut.imem[0] = 32'h4A000001;  // NewStruct p(r0), 1 field  (op 74=0x4A)
    dut.imem[1] = 32'h0501000A;  // r1 = 10
    dut.imem[2] = 32'h4B000100;  // StructInsert p.title = r1  (op 75=0x4B)
    dut.imem[3] = 32'h0502002A;  // r2 = 42
    dut.imem[4] = 32'h54000200;  // CrdtResolve p.title = r2   (op 84=0x54)
    dut.imem[5] = 32'h4C030000;  // GetField r3 = p.title      (op 76=0x4C)
    dut.imem[6] = 32'h59000300;  // Show r3
    dut.imem[7] = 32'h2B000000;  // Halt
    #3 rst = 0; wait (halted); #10;
    if (got && shown == 32'd42) $display("PASS: soft-core CrdtResolve (last-write = %0d)", shown);
    else $display("FAIL: got=%0b shown=%0d (expected 42)", got, shown);
    $finish;
  end
  initial begin #50000 $display("FAIL: timeout"); $finish; end
endmodule
