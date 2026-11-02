// shared_divider — one SEQUENTIAL restoring divider that computes signed quotient AND remainder,
// replacing the ~24,600-cell COMBINATIONAL `$signed(a)/$signed(b)` + `$signed(a)%$signed(b)` the ALU
// instantiates per-op. Truncated (round-toward-zero) division matching the VM: quotient sign =
// sign(a)^sign(b), remainder sign = sign(a); b==0 yields q=r=0 (the ALU's guard). One 1-bit/cycle
// restoring loop over |a|/|b| (W cycles) then a sign fixup — the soft-core pulses `start` and stalls
// until `done`. Proven == the combinational oracle by iverilog (tb_shared_divider) over edge cases +
// a wide random sweep, and the (a/b)*b + a%b == a identity is certified at reduced width by our SAT.
module shared_divider #(parameter W = 32) (
  input              clk,
  input              rst,
  input              start,       // pulse high for one cycle to begin a division
  input      [W-1:0] a,           // dividend  (two's-complement signed)
  input      [W-1:0] b,           // divisor
  output reg [W-1:0] q,           // quotient  (valid when done)
  output reg [W-1:0] r,           // remainder (valid when done)
  output reg         done         // high once q/r are valid; held until the next start
);
  localparam IDLE = 1'b0, RUN = 1'b1;
  reg          state;
  reg  [W-1:0] absb, quo, rem, dvd;  // |b| held; quotient built; running remainder; shifting |a|
  reg          qneg, rneg, bz;       // result sign flags; divisor-is-zero latch
  reg  [6:0]   cnt;                  // iteration counter, W..0

  // Power-on state (ECP5 honors initial values). The soft-core deasserts rst before the first clock
  // edge, so — like soft_core.v — the divider self-initializes here rather than only via a reset pulse.
  initial begin
    state = IDLE; done = 1'b0; q = {W{1'b0}}; r = {W{1'b0}};
  end

  // Bring the next MSB of the shifting dividend into the remainder, then a trial subtract of |b|.
  wire [W:0] shifted = {rem, dvd[W-1]};
  wire [W:0] sub     = shifted - {1'b0, absb};
  wire       ge      = (shifted >= {1'b0, absb});

  always @(posedge clk) begin
    if (rst) begin
      state <= IDLE; done <= 1'b0; q <= {W{1'b0}}; r <= {W{1'b0}};
    end else begin
      case (state)
        IDLE: if (start) begin
          bz   <= (b == {W{1'b0}});
          absb <= b[W-1] ? (~b + 1'b1) : b;
          dvd  <= a[W-1] ? (~a + 1'b1) : a;
          rem  <= {W{1'b0}};
          quo  <= {W{1'b0}};
          qneg <= a[W-1] ^ b[W-1];
          rneg <= a[W-1];
          cnt  <= W[6:0];
          done <= 1'b0;
          state <= RUN;
        end
        RUN: begin
          if (bz) begin
            q <= {W{1'b0}}; r <= {W{1'b0}}; done <= 1'b1; state <= IDLE;   // divide-by-zero guard
          end else if (cnt == 7'd0) begin
            q <= qneg ? (~quo + 1'b1) : quo;          // |a|/|b|  → signed quotient
            r <= rneg ? (~rem + 1'b1) : rem;          // |a|%|b|  → remainder, dividend's sign
            done <= 1'b1; state <= IDLE;
          end else begin
            rem <= ge ? sub[W-1:0] : shifted[W-1:0];  // restoring step (rem stays < |b| < 2^W)
            quo <= {quo[W-2:0], ge};                  // shift a quotient bit in
            dvd <= {dvd[W-2:0], 1'b0};                // shift dividend up
            cnt <= cnt - 1'b1;
          end
        end
      endcase
    end
  end
endmodule
