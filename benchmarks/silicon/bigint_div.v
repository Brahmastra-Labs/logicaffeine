// bigint_div — a sequential UNSIGNED long divider for the BigInt magnitude lane: q = a / b,
// r = a % b over W-bit unsigned magnitudes (W=256 = four 64-bit limbs), b==0 -> q=r=0. One
// 1-bit/cycle restoring loop (W cycles) — the wide analogue of shared_divider's magnitude core,
// without the signed fixup (BigInt carries its sign separately). Feeds the numeric-tower BigInt
// divide the way shared_divider feeds the 32-bit ALU. Proven == the reference magnitude division
// (`a / b`, `a % b`) by iverilog (tb_bigint_div), matching logicaffeine_base::numeric::mag_div.
module bigint_div #(parameter W = 256) (
  input              clk,
  input              rst,
  input              start,       // pulse high for one cycle to begin a division
  input      [W-1:0] a,           // dividend magnitude (unsigned)
  input      [W-1:0] b,           // divisor  magnitude (unsigned)
  output reg [W-1:0] q,           // quotient  (valid when done)
  output reg [W-1:0] r,           // remainder (valid when done)
  output reg         done         // high once q/r are valid; held until the next start
);
  localparam IDLE = 1'b0, RUN = 1'b1;
  reg          state;
  reg  [W-1:0] divb, quo, rem, dvd;  // |b| held; quotient built; running remainder; shifting |a|
  reg          bz;                   // divisor-is-zero latch
  reg  [8:0]   cnt;                  // iteration counter, W..0 (9 bits covers W up to 256)

  wire [W:0] shifted = {rem, dvd[W-1]};   // bring the next MSB of the dividend into the remainder
  wire [W:0] sub     = shifted - {1'b0, divb};
  wire       ge      = (shifted >= {1'b0, divb});

  // Power-on state (ECP5 honors initial values), matching the reset — consumers may deassert rst
  // before the first clock edge (like soft_core.v / shared_divider.v).
  initial begin
    state = IDLE; done = 1'b0; q = {W{1'b0}}; r = {W{1'b0}};
  end

  always @(posedge clk) begin
    if (rst) begin
      state <= IDLE; done <= 1'b0; q <= {W{1'b0}}; r <= {W{1'b0}};
    end else begin
      case (state)
        IDLE: if (start) begin
          bz   <= (b == {W{1'b0}});
          divb <= b;
          dvd  <= a;
          rem  <= {W{1'b0}};
          quo  <= {W{1'b0}};
          cnt  <= W[8:0];
          done <= 1'b0;
          state <= RUN;
        end
        RUN: begin
          if (bz) begin
            q <= {W{1'b0}}; r <= {W{1'b0}}; done <= 1'b1; state <= IDLE;   // divide-by-zero guard
          end else if (cnt == 9'd0) begin
            q <= quo; r <= rem; done <= 1'b1; state <= IDLE;
          end else begin
            rem <= ge ? sub[W-1:0] : shifted[W-1:0];  // restoring step (rem stays < b < 2^W)
            quo <= {quo[W-2:0], ge};                  // shift a quotient bit in
            dvd <= {dvd[W-2:0], 1'b0};                // shift dividend up
            cnt <= cnt - 9'd1;
          end
        end
      endcase
    end
  end
endmodule
