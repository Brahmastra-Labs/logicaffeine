// seeded_arbiter_prng — the NoC arbiter's deterministic entropy core.
// SplitMix64, bit-identical to logicaffeine_system::concurrency::SplitMix64.
module seeded_arbiter_prng(
  input clk,
  input rst,
  input [63:0] seed,
  input load,
  input step,
  output reg [63:0] draw,
  output reg draw_valid
);
  reg [63:0] state;
  // Combinational mix of the NEXT state (state + gamma).
  wire [63:0] s1 = state + 64'h9e3779b97f4a7c15;
  wire [63:0] z1 = (s1 ^ (s1 >> 30)) * 64'hbf58476d1ce4e5b9;
  wire [63:0] z2 = (z1 ^ (z1 >> 27)) * 64'h94d049bb133111eb;
  wire [63:0] mixed = z2 ^ (z2 >> 31);
  always @(posedge clk) begin
    if (rst) begin
      state <= 64'd0; draw <= 64'd0; draw_valid <= 1'b0;
    end else begin
      draw_valid <= 1'b0;
      if (load) begin
        state <= seed;
      end else if (step) begin
        state <= s1;
        draw <= mixed;
        draw_valid <= 1'b1;
      end
    end
  end
endmodule
