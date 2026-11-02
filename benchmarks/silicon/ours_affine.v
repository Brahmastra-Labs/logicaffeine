// ours_adder — the smallest design that exercises what the fabric SoC gets wrong.
//
// On silicon the fabric's `affine` datapath (`3x + 7`, a 64-bit add) comes back with its low bits
// exactly right and ONE high bit stuck — bit 61, the same value every lap, for hours. The RTL
// simulates correctly, the carry-chain configuration is complete and self-consistent (2,652
// CARRY4s, 2,511 continuation links, 2,511 matching carry-ins), and no subslice output mux is
// claimed twice. So the fault is real, ours, and hiding in four thousand slices.
//
// This is the same construct — a SIXTY-FOUR BIT add, sixteen CARRY4s in one vertical chain — and
// nothing else. Around a hundred slices instead of four thousand, which is small enough to read the
// whole FASM by hand and small enough to place in seconds.
//
// The framing is deliberately byte-identical to `nexys_fabric_soc`'s, so the same host reader
// works: {0xA5, vec, sum[63:0] MSB-first, 0x0001}, twelve bytes a record, looping forever.
// `vec` counts 0..3 and the sum is `vec + 7`, so a correct board sends 7, 8, 9, 10 and any bit
// above the third that is ever set is the bug, named by its position.
module ours_affine #(parameter CLKS_PER_BIT = 868) (
  input         clk,
  input         rst,
  output reg    uart_tx = 1'b1,
  output [15:0] led
);
  localparam NVEC = 64;

  reg [5:0] vec = 6'd0;
  // The whole point: arithmetic that is genuinely SIXTY-FOUR BITS WIDE.
  //
  // A narrow operand is not enough — synthesis folds `{58'd0, vec} + 7` down to a handful of
  // slices, which is exactly what happened on the first attempt: twenty-nine slices, and every bit
  // perfect, proving nothing about a design four thousand slices deep. So the operand has bits set
  // across its whole width and the top bits move with `vec`, and the operation is a multiply-add,
  // which is what the fabric's `affine` datapath (`3x + 7`) actually is. Now every one of the
  // sixty-four output bits depends on real logic and a real carry chain, and the host knows what
  // each of them should be.
  // THE FAILING DATAPATH, ON ITS OWN.
  //
  // This is the fabric itself — the same `config_fabric` the SoC instantiates — with its config
  // word tied to the one setting that comes back wrong. Synthesis then folds the other four
  // datapaths and the five-way per-tile muxes away, leaving the `affine` grid and nothing else:
  // the same logic, a fraction of the size. If the wrong bit survives that, it is reproducible in a
  // design small enough to read; if it does not, the fault needs the muxes or the scale, and that
  // is worth knowing too.
  // Sixty-four samples, not four. Four values were enough to prove the bitstream wrong and not
  // enough to say WHICH logic is wrong: several different faults fit them equally well. Sweeping
  // the whole six-bit input makes the failure a function of x, which a wrong LUT and a wrong carry
  // cannot both produce.
  // The sweep lives in the TOP bits, not the bottom.
  //
  // Sweeping 0..63 exercises only bits 0 to 8 of the result, and the full SoC's surviving fault is
  // at BIT 61 — so the first version of this reproduction passed sixty-four out of sixty-four while
  // never once testing the bit that is wrong. Shifting the input up puts the sweep where the fault
  // is: `3x + 7` for x = vec << 58 moves bits 58 through 63, and the low bits stay a known constant.
  wire signed [63:0] x = {vec, 58'd0};
  wire signed [63:0] sum;
  config_fabric u_fab(.in0(x), .in1(64'sd0), .in2(64'sd0), .cfg(3'd0), .result(sum));

  localparam [15:0] CYCLES = 16'd1;
  localparam S_RESET=3'd0, S_LOAD=3'd1, S_START=3'd2, S_DATA=3'd3, S_STOP=3'd4, S_DONE=3'd5;
  reg [2:0]  state = S_RESET;
  reg [15:0] clk_cnt;
  reg [2:0]  bit_idx;
  reg [7:0]  tx_byte;
  reg [3:0]  pos;
  reg [3:0]  rst_cnt;
  assign led = {state, vec[4:0], 8'd0};
  wire [7:0] res_byte = sum[(9 - pos) * 8 +: 8];

  always @(posedge clk) begin
   if (rst) begin
    state<=S_RESET; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; vec<=6'd0; pos<=4'd0; rst_cnt<=4'd0; tx_byte<=8'd0;
   end else case (state)
    S_RESET: if (rst_cnt==4'd8) state<=S_LOAD; else rst_cnt<=rst_cnt+4'd1;
    S_LOAD: begin
     case (pos)
      4'd0: tx_byte<=8'hA5;
      4'd1: tx_byte<={2'd0, vec};
      4'd10: tx_byte<=CYCLES[15:8];
      4'd11: tx_byte<=CYCLES[7:0];
      default: tx_byte<=res_byte;
     endcase
     clk_cnt<=16'd0; bit_idx<=3'd0; state<=S_START;
    end
    S_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; state<=S_DATA; end else clk_cnt<=clk_cnt+16'd1; end
    S_DATA: begin uart_tx<=tx_byte[bit_idx];
     if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) state<=S_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;
    end
    S_STOP: begin uart_tx<=1'b1;
     if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;
      if (pos==4'd11) begin
       pos<=4'd0;
       if (vec==NVEC-1) vec<=6'd0; else vec<=vec+6'd1; state<=S_LOAD;
      end else begin pos<=pos+4'd1; state<=S_LOAD; end
     end else clk_cnt<=clk_cnt+16'd1;
    end
    S_DONE: uart_tx<=1'b1;
    default: state<=S_DONE;
   endcase
  end
endmodule
