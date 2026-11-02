`timescale 1ns/1ps
// Functional proof that WIRE OPCODE BYTES drive the general integer ALU: each arithmetic / bitwise /
// signed-comparison instruction, decoded from its `binisa` opcode byte, computes the VM's result.
module tb;
  reg [7:0] opcode; reg [31:0] a, b; wire [31:0] y;
  alu dut(.opcode(opcode), .a(a), .b(b), .y(y));
  integer errors = 0;
  task chk(input [7:0] oc, input [31:0] av, input [31:0] bv, input [31:0] exp);
    begin
      opcode = oc; a = av; b = bv; #1;
      if (y !== exp) begin
        errors = errors + 1;
        $display("FAIL op=%0d a=%0d b=%0d -> %0d (exp %0d)", oc, $signed(av), $signed(bv), y, exp);
      end
    end
  endtask
  initial begin
    chk(8'd8,  5, 3, 8);              // Add
    chk(8'd10, 5, 3, 2);             // Sub
    chk(8'd11, 5, 3, 15);            // Mul
    // Div / ExactDiv / Mod / FloorDiv (12/13/15/14) are no longer ALU ops — they issue to the
    // sequential shared_divider (proven by tb_shared_divider + tb_soft_core_divide). DivPow2 stays.
    chk(8'd16, 17, 2, 4);           // DivPow2   17 / 4 = 4  (b=2 -> /2^2, trunc)
    chk(8'd16, -17, 2, -4);         // DivPow2  -17 / 4 = -4 (trunc toward zero)
    chk(8'd16, -1, 3, 0);           // DivPow2   -1 / 8 = 0  (trunc toward zero, not -1)
    chk(8'd19, 6, 3, 5);            // BitXor  110 ^ 011 = 101
    chk(8'd20, 6, 3, 2);            // BitAnd  110 & 011 = 010
    chk(8'd21, 6, 3, 7);            // BitOr   110 | 011 = 111
    chk(8'd24, 32'd0, 32'd0, 32'hFFFFFFFF); // Not (~0 = all ones)
    chk(8'd22, 3, 2, 12);          // Shl  3 << 2 = 12
    chk(8'd23, -16, 2, -4);        // Shr  arithmetic: -16 >> 2 = -4
    chk(8'd23, 32'hFFFFFFFF, 1, 32'hFFFFFFFF); // Shr arithmetic: -1 >> 1 = -1 (sign fill)
    chk(8'd25, -5, 3, 1);          // Lt  signed: -5 < 3
    chk(8'd26, -5, 3, 0);          // Gt  signed: not(-5 > 3)
    chk(8'd27, 3, 3, 1);           // LtEq signed: 3 <= 3
    chk(8'd28, 4, 3, 1);           // GtEq signed: 4 >= 3
    chk(8'd29, 7, 7, 1);           // Eq
    chk(8'd31, 7, 7, 0);           // NotEq
    chk(8'd99, 1, 1, 0);           // illegal opcode -> 0 (inert)
    if (errors == 0)
      $display("PASS: wire opcode BYTES drive the general integer ALU (arith/bitwise/signed-compare)");
    else
      $display("FAILED with %0d errors", errors);
    $finish;
  end
endmodule
