// The LOGOS ISA opcode decoder (reversible core) — the silicon FRONT-END that turns a WIRE opcode
// BYTE (the exact byte `logicaffeine_compile::vm::binisa::encode_program` puts on the wire) into the
// reversible ALU's 3-bit control select plus a `core` valid flag. The case labels ARE the stable wire
// opcodes (`binisa::opcode`): the byte the network carries is the byte the chip decodes — no
// translation layer. Locked to the emitter by `codegen_sva::rtl::tests::
// the_wire_opcode_is_the_reversible_silicon_control_select_proven_by_our_sat`.
module isa_core_decoder(input [7:0] opcode, output reg [2:0] sel, output reg core);
  always @(*) begin
    case (opcode)
      8'd0: begin sel = 3'd0; core = 1'b1; end  // RevAdd
      8'd1: begin sel = 3'd1; core = 1'b1; end  // RevSub
      8'd2: begin sel = 3'd2; core = 1'b1; end  // RevXor
      8'd3: begin sel = 3'd3; core = 1'b1; end  // RevNeg
      8'd4: begin sel = 3'd4; core = 1'b1; end  // RevSwap
      default: begin sel = 3'd7; core = 1'b0; end  // illegal byte: ALU-identity no-op, invalid flag
    endcase
  end
endmodule
