// The LOGOS reversible ALU core — the abelian charge-recovery datapath. For every fixed `op` the
// transition on the state (a,b) is a BIJECTION, so the whole ALU runs backward with zero erased bits.
module rev_alu_core(input clk, input [2:0] op, output reg [31:0] a, output reg [31:0] b);
  initial begin a = 0; b = 0; end
  always @(posedge clk) begin
    case (op)
      3'd0: a <= a + b;               // RevAdd  (reverse: a - b)
      3'd1: a <= a - b;               // RevSub  (reverse: a + b)
      3'd2: a <= a ^ b;               // RevXor  (self-inverse)
      3'd3: a <= (~a) + 32'd1;        // RevNeg  (two's-complement, self-inverse)
      3'd4: begin a <= b; b <= a; end // RevSwap (self-inverse)
      default: ;                      // identity (reversible)
    endcase
  end
endmodule
