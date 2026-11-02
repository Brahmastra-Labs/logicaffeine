`timescale 1ns/1ps
// Functional proof that the wire-driven reversible SOFT-CORE executes LOGOS bytecode off the wire:
// drive rev_core with WIRE OPCODE BYTES (`binisa::opcode`: 0=RevAdd, 1=RevSub, 2=RevXor, 4=RevSwap),
// the decoder turns each byte into the ALU control, and the reversible datapath computes AND reverses.
module tb;
  reg clk = 0; reg [7:0] opcode; wire [31:0] a, b; wire core;
  rev_core dut(.clk(clk), .opcode(opcode), .a(a), .b(b), .core(core));
  task tick; begin #1 clk=1; #1 clk=0; end endtask
  integer errors = 0;
  initial begin
    // Seed the reversible state (sim-only hierarchical load through the ALU instance).
    dut.alu.a = 32'd5; dut.alu.b = 32'd3;

    opcode = 8'd0; #1;                          // RevAdd byte
    if (core !== 1'b1) begin errors=errors+1; $display("FAIL core-valid for opcode 0"); end
    tick;                                        // a = 5+3 = 8
    if (a !== 32'd8) begin errors=errors+1; $display("FAIL RevAdd byte: a=%0d", a); end

    opcode = 8'd1; tick;                         // RevSub byte: a = 8-3 = 5  (reversed)
    if (a !== 32'd5) begin errors=errors+1; $display("FAIL RevSub byte reverse: a=%0d", a); end

    opcode = 8'd2; tick; opcode = 8'd2; tick;    // RevXor byte twice → identity
    if (a !== 32'd5) begin errors=errors+1; $display("FAIL RevXor byte self-inverse: a=%0d", a); end

    opcode = 8'd4; tick; opcode = 8'd4; tick;    // RevSwap byte twice → identity
    if (a !== 32'd5 || b !== 32'd3) begin errors=errors+1; $display("FAIL RevSwap byte self-inverse"); end

    // An ILLEGAL wire byte: core=0 and the reversible state is UNDISTURBED (inert no-op).
    opcode = 8'd99; #1;
    if (core !== 1'b0) begin errors=errors+1; $display("FAIL illegal byte should be invalid"); end
    tick;
    if (a !== 32'd5 || b !== 32'd3) begin errors=errors+1; $display("FAIL illegal byte perturbed state"); end

    if (errors == 0) $display("PASS: wire opcode BYTES drive the reversible soft-core (decode + compute + reverse)");
    else $display("FAILED with %0d errors", errors);
    $finish;
  end
endmodule
