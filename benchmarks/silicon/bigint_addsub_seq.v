// bigint_addsub_seq — 256-bit (4x64 limb) add/subtract, computed LIMB-SERIAL over 4 cycles
// with one reused 65-bit adder (no 256-bit combinational carry chain), so its nets are short
// and it is antenna-free by construction (unlike bigint_alu's `a+b` / `a-b`). result = sub ?
// a-b : a+b (two's-complement; magnitude when a>=b, matching bigint_alu.diff). Handshake: pulse
// `start` with a,b,sub valid; `done` pulses when `result` is ready and holds until next start.
module bigint_addsub_seq(
  input          clk,
  input          rst,
  input          start,
  input          sub,        // 0 = add, 1 = subtract
  input  [255:0] a,
  input  [255:0] b,
  output [255:0] result,
  output         done
);
  reg  [255:0] res;
  reg  [255:0] areg, breg;   // a,b shifted down one 64-bit limb per step
  reg  [2:0]   cnt;          // 0..3
  reg          carry;        // carry-in (add) / +1 seed then carry (sub)
  reg          sub_r;        // op latched at start (self-contained; no caller-hold requirement)
  reg          busy, done_r;

  wire [63:0]  bl = sub_r ? ~breg[63:0] : breg[63:0];    // subtract = add of ~b (+1 via carry seed)
  wire [64:0]  s  = {1'b0, areg[63:0]} + {1'b0, bl} + carry;

  always @(posedge clk) begin
    if (rst) begin
      busy <= 1'b0; done_r <= 1'b0; cnt <= 3'd0; carry <= 1'b0; sub_r <= 1'b0;
      res <= 256'd0; areg <= 256'd0; breg <= 256'd0;
    end else begin
      done_r <= 1'b0;
      if (start && !busy) begin
        busy  <= 1'b1; cnt <= 3'd0; res <= 256'd0;
        areg  <= a; breg <= b; sub_r <= sub; carry <= sub ? 1'b1 : 1'b0;   // sub seeds the +1
      end else if (busy) begin
        res   <= {s[63:0], res[255:64]};   // new limb enters at the top, prior limbs shift down
        carry <= s[64];
        areg  <= areg >> 64;
        breg  <= breg >> 64;
        cnt   <= cnt + 3'd1;
        if (cnt == 3'd3) begin busy <= 1'b0; done_r <= 1'b1; end
      end
    end
  end
  assign result = res;
  assign done   = done_r;
endmodule
