// bigint_mul_comba — 64x64 -> 128 multiply via sequential column (Comba) accumulation. ONE reused
// 32x32 multiplier, all accumulators <= 65-bit: no 128-bit register AND no wide combinational
// array, so its nets are both narrow AND short (antenna-free). P = a0*b0 + (a0*b1 + a1*b0)*2^32 +
// a1*b1*2^64, one 32x32 product per cycle. Handshake: pulse start; done pulses with {phi,plo}=a*b.
module bigint_mul_comba(input clk, input rst, input start,
                        input [63:0] a, input [63:0] b,
                        output [63:0] plo, output [63:0] phi, output done);
  reg [63:0] areg, breg;
  reg [1:0]  step;
  reg        busy, done_r;
  reg [63:0] t0_r;       // a0*b0
  reg [64:0] mid_r;      // a0*b1 + a1*b0 (65-bit)
  reg [63:0] plo_r, phi_r;

  // the single reused 32x32 multiplier; operands selected by step
  reg  [31:0] ma, mb;
  wire [63:0] prod = ma * mb;
  always @(*) begin
    case (step)
      2'd0:    begin ma = areg[31:0];  mb = breg[31:0];  end   // a0*b0
      2'd1:    begin ma = areg[31:0];  mb = breg[63:32]; end   // a0*b1
      2'd2:    begin ma = areg[63:32]; mb = breg[31:0];  end   // a1*b0
      default: begin ma = areg[63:32]; mb = breg[63:32]; end   // a1*b1
    endcase
  end

  wire [64:0] lo_sum = {1'b0, t0_r} + {mid_r[31:0], 32'b0};    // t0 + (mid_lo << 32)

  always @(posedge clk) begin
    if (rst) begin
      busy <= 1'b0; done_r <= 1'b0; step <= 2'd0;
      areg <= 64'd0; breg <= 64'd0; t0_r <= 64'd0; mid_r <= 65'd0; plo_r <= 64'd0; phi_r <= 64'd0;
    end else begin
      done_r <= 1'b0;
      if (start && !busy) begin
        busy <= 1'b1; step <= 2'd0; areg <= a; breg <= b;
      end else if (busy) begin
        case (step)
          2'd0: begin t0_r  <= prod;                  step <= 2'd1; end  // t0 = a0*b0
          2'd1: begin mid_r <= {1'b0, prod};          step <= 2'd2; end  // mid = a0*b1
          2'd2: begin mid_r <= mid_r + {1'b0, prod};  step <= 2'd3; end  // mid += a1*b0
          2'd3: begin                                                    // t3 = a1*b1 (= prod); assemble
            plo_r <= lo_sum[63:0];
            phi_r <= prod + mid_r[64:32] + {63'b0, lo_sum[64]};
            busy <= 1'b0; done_r <= 1'b1;
          end
        endcase
      end
    end
  end
  assign plo  = plo_r;
  assign phi  = phi_r;
  assign done = done_r;
endmodule
