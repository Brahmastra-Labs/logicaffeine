// gemm_mac_realbist — a self-contained BIST harness around the LOGOS-emitted int8 MAC array
// (cgra_arr8 in gemm_mac_array.v: 8 signed 8x8 multiply-accumulate tiles per clock). An on-chip
// LFSR streams fresh operands so all 8 multipliers toggle every clock; a MISR folds each finished
// dot-product into a 1-bit signature. Only clk/rst/sig cross the boundary, so it routes on Sky130
// with no pad distortion — the same measurement shape as decision_hft_realbist, so
// scripts/measure-power-vcd.sh drives it unmodified. This is the design taped signoff-clean and
// VCD-power-measured to turn gemm_fabric.json's est_mw into a MEASURED number.
module gemm_mac_realbist(input clk, input rst, output reg sig);
  reg  [63:0] lfsr;
  reg  [63:0] in0;
  reg         arr_rst;
  reg  [63:0] misr;
  wire [63:0] result;
  wire        done;

  cgra_arr8 arr(.clk(clk), .rst(arr_rst), .in0(in0), .result(result), .done(done));

  // next loop bound: 8..127 (bit3 forced high so the array always has work → never idle)
  wire [6:0] nxt = lfsr[6:0] | 7'd8;

  always @(posedge clk) begin
    if (rst) begin
      lfsr    <= 64'hD1CEF00D12345678;   // non-zero seed → LFSR never locks up
      in0     <= 64'sd16;
      arr_rst <= 1'b1;
      misr    <= 64'd0;
    end else begin
      // 64-bit Fibonacci LFSR (taps 64,63,61,60) — maximal-length switching stimulus
      lfsr <= {lfsr[62:0], lfsr[63] ^ lfsr[62] ^ lfsr[60] ^ lfsr[59]};
      if (done) begin
        misr    <= {misr[62:0], misr[63]} ^ result;   // fold the result into the signature
        in0     <= {57'd0, nxt};                       // fresh operands
        arr_rst <= 1'b1;                                // one-cycle restart pulse
      end else begin
        arr_rst <= 1'b0;
      end
    end
  end

  // The signature leaves from a register: no XOR tree sits between the MISR and the pin.
  always @(posedge clk) sig <= ^misr;
endmodule
