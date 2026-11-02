// The LOGOS reversible SOFT-CORE — a chip that executes LOGOS bytecode straight off the wire.
// A wire opcode BYTE drives the reversible datapath end-to-end:
//     opcode byte  ─▶  isa_core_decoder  ─▶  sel  ─▶  rev_alu_core  ─▶  reversible state (a,b)
// The decoder's opcode bytes are `binisa::opcode` (the same bytes the network carries); the ALU
// transition is a BIJECTION for every op, so the whole core runs backward with zero erased bits.
// This is the register-VM ISA realized in silicon: send the bytecode, the chip executes it.
module rev_core(
  input clk,
  input [7:0] opcode,
  output [31:0] a,
  output [31:0] b,
  output core
);
  wire [2:0] sel;
  isa_core_decoder dec(.opcode(opcode), .sel(sel), .core(core));
  rev_alu_core alu(.clk(clk), .op(sel), .a(a), .b(b));
endmodule
