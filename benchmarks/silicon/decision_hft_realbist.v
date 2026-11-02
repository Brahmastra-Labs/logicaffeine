// decision_hft_realbist — like decision_hft_bist, but the LFSR is scaled to REALISTIC market ranges
// so the decision actually makes varied buy / sell / flat / risk-blocked decisions (not no-trade on
// every extreme random input). Self-driving (~10 pins) so it tapes out signoff-clean at high
// utilization; the gate-level sim of THIS gives a workload-representative switching VCD for power.
module decision_hft_realbist(input clk, input rst, output reg [15:0] sig);
  reg  [63:0] lfsr;
  reg  [63:0] misr;
  wire [63:0] y;

  // realistic decision inputs (bit-sliced from the LFSR -> cheap, no dividers)
  wire [63:0] micro100  = 64'd10000 + {57'd0, lfsr[6:0]};        // 10000..10127 (straddles fair +/- edge)
  wire [63:0] fair100   = 64'd10050;
  wire [63:0] edge100   = {60'd0, lfsr[11:8]};                    // 0..15
  wire [63:0] base_size = 64'd1  + {58'd0, lfsr[18:13]};          // 1..64
  wire [63:0] max_order = 64'd10 + {59'd0, lfsr[24:20]};          // 10..41
  wire [63:0] inventory = {{56{lfsr[31]}}, lfsr[31:24]};          // signed -128..127
  wire [63:0] pos_cap   = 64'd100;
  wire [63:0] book_ok   = {63'd0, lfsr[33]};                      // 0/1

  decision_hft u_dec(.in0(micro100), .in1(fair100), .in2(edge100), .in3(base_size),
                     .in4(max_order), .in5(inventory), .in6(pos_cap), .in7(book_ok), .result(y));

  function [63:0] nxt(input [63:0] s);
    reg [63:0] x; begin x=s; x=x^(x<<13); x=x^(x>>7); x=x^(x<<17); nxt=x; end
  endfunction

  always @(posedge clk) begin
    if (rst) begin lfsr <= 64'h1234_5678_9ABC_DEF1; misr <= 64'd0; sig <= 16'd0; end
    else begin
      lfsr <= nxt(lfsr);
      misr <= {misr[62:0], ^misr} ^ y;
      sig  <= misr[15:0];
    end
  end
endmodule
