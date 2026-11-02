// fpu_stub — empty stand-ins for the fpu.v units, port-compatible with what soft_core.v instantiates.
// The HFT tick-to-trade pipeline is 32-bit integer only and NEVER dispatches a float opcode (201..212),
// so these units are dead logic that yosys DCEs away — an integer-only build of the core that ROUTES on
// the open flow (the real 64-bit FPU is large and congests it). Use fpu_stub.v instead of fpu.v for the
// nexys_soc_hft bitstream; the on-chip result is bit-identical because no float op ever executes.
module fp_addsub(input clk, input rst, input start, input [63:0] a, input [63:0] b_in, input sub,
                 output [63:0] y, output done);
  assign y = 64'd0; assign done = 1'b1;
endmodule

module fp_mul(input clk, input rst, input start, input [63:0] a, input [63:0] b,
              output [63:0] y, output done);
  assign y = 64'd0; assign done = 1'b1;
endmodule

module fp_div(input clk, input rst, input start, input [63:0] a, input [63:0] b,
              output [63:0] y, output done);
  assign y = 64'd0; assign done = 1'b1;
endmodule

module fp_sqrt(input clk, input rst, input start, input [63:0] a,
               output [63:0] y, output done);
  assign y = 64'd0; assign done = 1'b1;
endmodule

module fp_cmp(input [63:0] a, input [63:0] b, output lt, output eq);
  assign lt = 1'b0; assign eq = 1'b0;
endmodule
