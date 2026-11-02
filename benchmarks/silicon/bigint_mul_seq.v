// bigint_mul_seq — sequential (shift-add) 64x64 -> 128 multiply. One reused 128-bit
// conditional-add plus shifts, iterated 64 times — there is NO flat combinational array, so
// the datapath's nets are short and it is antenna-free by construction (unlike `assign
// prod = a*b`, whose 64x64 array is the antenna source). Handshake: pulse `start` with a,b
// valid; `done` pulses for one cycle when `prod == a*b` is ready and holds until the next start.
module bigint_mul_seq(
  input          clk,
  input          rst,
  input          start,
  input  [63:0]  a,
  input  [63:0]  b,
  output [127:0] prod,
  output         done
);
  reg [127:0] acc;      // the product accumulates here
  reg [127:0] ashift;   // a, shifted left one bit per step (128-bit to hold a<<63)
  reg [63:0]  bshift;   // b, consumed LSB-first
  reg [6:0]   cnt;      // step counter 0..63
  reg         busy;
  reg         done_r;

  always @(posedge clk) begin
    if (rst) begin
      busy <= 1'b0; done_r <= 1'b0; cnt <= 7'd0;
      acc <= 128'd0; ashift <= 128'd0; bshift <= 64'd0;
    end else begin
      done_r <= 1'b0;
      if (start && !busy) begin
        busy   <= 1'b1;
        cnt    <= 7'd0;
        acc    <= 128'd0;
        ashift <= {64'd0, a};
        bshift <= b;
      end else if (busy) begin
        if (bshift[0]) acc <= acc + ashift;   // add this partial product
        ashift <= ashift << 1;
        bshift <= bshift >> 1;
        cnt    <= cnt + 7'd1;
        if (cnt == 7'd63) begin               // 64th (last) partial product this cycle
          busy   <= 1'b0;
          done_r <= 1'b1;
        end
      end
    end
  end
  assign prod = acc;
  assign done = done_r;
endmodule
