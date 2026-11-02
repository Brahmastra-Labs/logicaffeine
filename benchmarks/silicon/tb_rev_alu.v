`timescale 1ns/1ps
module tb;
  reg clk = 0; reg [2:0] op; wire [31:0] a, b;
  rev_alu_core dut(.clk(clk), .op(op), .a(a), .b(b));
  // force initial a,b via the DUT's initial (a=0,b=0); load by RevAdd from 0.
  task tick; begin #1 clk=1; #1 clk=0; end endtask
  integer errors = 0;
  initial begin
    // load a=5 (0+5 via delta? core has no load — use RevAdd from b). Set b then add.
    // Drive: force a=5,b=3 via hierarchical set (simulation only).
    dut.a = 32'd5; dut.b = 32'd3;
    op = 3'd0; tick;                       // RevAdd: a = 5+3 = 8
    if (a !== 32'd8) begin errors=errors+1; $display("FAIL RevAdd: a=%0d", a); end
    op = 3'd1; tick;                       // RevSub: a = 8-3 = 5  (reversed!)
    if (a !== 32'd5) begin errors=errors+1; $display("FAIL RevSub reverse: a=%0d", a); end
    op = 3'd2; tick; op = 3'd2; tick;      // RevXor twice → identity
    if (a !== 32'd5) begin errors=errors+1; $display("FAIL RevXor self-inverse: a=%0d", a); end
    op = 3'd4; tick; op = 3'd4; tick;      // RevSwap twice → identity
    if (a !== 32'd5 || b !== 32'd3) begin errors=errors+1; $display("FAIL RevSwap self-inverse"); end
    if (errors == 0) $display("PASS: reversible ALU computes and reverses (RevAdd/RevSub/RevXor/RevSwap)");
    else $display("FAILED with %0d errors", errors);
    $finish;
  end
endmodule
