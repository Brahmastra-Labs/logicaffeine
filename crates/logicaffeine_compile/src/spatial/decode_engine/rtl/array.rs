//! The array's processing element as Verilog: two weight rows' partial dot products with one token-row's activations,
//! four multiply-adds a DSP block.
//!
//! A Stratix 10 DSP block in its multiplier-adder sum mode computes `ax·ay + bx·by` plus the chain-in of the block
//! before it, its operands two's complement (19 bits for `ax`/`bx`, 18 for `ay`/`by`; unsigned operands only as
//! 18×18), the chain 64 bits wide. The element packs two rows into each 19-bit operand:
//! `W = q₁ + (q₂ − 8)·2^15` for 4-bit digits `q₁` (row one) and `q₂` (row two), which lies in `[−2^18, 2^18)`; the
//! activation excess codes `a′ = a + 128` (0..255) ride the 18-bit operands. A chain of [`CHAIN`] blocks sums
//! `L + H·2^15` with `L = Σ a′·q₁ ≤ 8·255·15 < 2^15` never negative and `H = Σ a′·(q₂ − 8)` signed, so the two rows
//! split from the chain's two's-complement sum exactly: `L` its low 15 bits, `H` the rest shifted down. Row two's
//! partial is `H + 8·Σa′`.
//!
//! `logos_dsp_sum2` is the block's arithmetic, behavioural for simulation; synthesis maps it to the device's DSP
//! block in that mode. `logos_pe8` chains four and splits the lanes; `logos_subdot32` sums four elements into a
//! 32-element sub-block's two partials. `logos_pe8_p` is `logos_pe8` for the user clock, systolic: each block's sum
//! registered on the chain, block `k`'s activations given `k` clocks after block 0's (`logos_column_act_stagger`
//! delays them once for every core) and its digits delayed `k` clocks to meet them, the lanes [`PE8_P_LATENCY`] clocks
//! after block 0's operands.

/// DSP blocks a processing element chains.
pub const CHAIN: usize = 4;
/// Elements one chain covers: two products a block.
pub const CHAIN_ELEMENTS: usize = 2 * CHAIN;
/// The place row two's lane starts at.
pub const LANE: u32 = 15;
/// The offset row two's digit carries so its packed operand fits the signed port.
pub const HIGH_OFFSET: i64 = 8;
/// Clocks from a systolic element's operands to its lanes in `logos_pe8_p`: the first block's input register, then a
/// chain register a block.
pub const PE8_P_LATENCY: usize = CHAIN + 1;

/// `W = q₁ + (q₂ − 8)·2^15`: two rows' digits in one signed 19-bit operand.
pub fn pack(q1: u8, q2: u8) -> i64 {
    q1 as i64 + ((q2 as i64 - HIGH_OFFSET) << LANE)
}

/// A chain's sum split into its two rows' partials: `(Σ a′·q₁, Σ a′·q₂)`.
pub fn split(sum: i64, act_sum: i64) -> (i64, i64) {
    let low = sum & ((1 << LANE) - 1);
    (low, ((sum - low) >> LANE) + HIGH_OFFSET * act_sum)
}

/// Every module of the element.
pub fn array_verilog() -> &'static str {
    ARRAY_VERILOG
}

const ARRAY_VERILOG: &str = r#"// The array's processing element: two rows' partial dots with one token-row, four multiply-adds a DSP block.

// A DSP block in 18x19 multiplier-adder sum mode with chain-in: signed 19-bit x and 18-bit y operands, 64-bit chain.
module logos_dsp_sum2(input signed [18:0] ax, input signed [17:0] ay, input signed [18:0] bx, input signed [17:0] by,
                      input signed [63:0] chainin, output signed [63:0] chainout);
  assign chainout = ax * ay + bx * by + chainin;
endmodule

// Eight elements of one token-row against two rows: four chained blocks, the lanes split from the chain's sum.
// a: eight 8-bit excess codes; q1, q2: eight 4-bit digits of rows one and two; lo = sum a'.q1; hi = sum a'.(q2 - 8).
module logos_pe8(input [63:0] a, input [31:0] q1, input [31:0] q2, output [14:0] lo, output signed [16:0] hi);
  wire signed [63:0] chain [0:4];
  assign chain[0] = 64'sd0;
  genvar k;
  generate
    for (k = 0; k < 4; k = k + 1) begin : blk
      wire [7:0] a0 = a[16*k +: 8], a1 = a[16*k + 8 +: 8];
      wire [3:0] p0 = q1[8*k +: 4], p1 = q1[8*k + 4 +: 4], r0 = q2[8*k +: 4], r1 = q2[8*k + 4 +: 4];
      wire signed [18:0] w0 = $signed({15'd0, p0}) + (($signed({15'd0, r0}) - 19'sd8) <<< 15);
      wire signed [18:0] w1 = $signed({15'd0, p1}) + (($signed({15'd0, r1}) - 19'sd8) <<< 15);
      logos_dsp_sum2 d(.ax(w0), .ay($signed({10'd0, a0})), .bx(w1), .by($signed({10'd0, a1})),
                       .chainin(chain[k]), .chainout(chain[k + 1]));
    end
  endgenerate
  assign lo = chain[4][14:0];
  assign hi = chain[4][31:15];
endmodule

// logos_pe8 for the user clock, systolic: block k's sum registered on the chain, its operands taken into its input
// registers k clocks after block 0's, so lo and hi are logos_pe8's of the operands five clocks before. Block k's two
// activations come in k clocks after block 0's (a's block k delayed by logos_column_act_stagger, once for every core);
// its four digits are delayed here.
module logos_pe8_p(input clk, input [63:0] a, input [31:0] q1, input [31:0] q2, output [14:0] lo, output signed [16:0] hi);
  wire signed [63:0] chain [0:4];
  assign chain[0] = 64'sd0;
  // Block k's digits of both rows, {q2 byte, q1 byte}, delayed k clocks.
  reg [15:0] n1, n2a, n2, n3a, n3b, n3;
  always @(posedge clk) begin
    n1 <= {q2[15:8], q1[15:8]};
    n2a <= {q2[23:16], q1[23:16]}; n2 <= n2a;
    n3a <= {q2[31:24], q1[31:24]}; n3b <= n3a; n3 <= n3b;
  end
  wire [63:0] nibs = {n3, n2, n1, q2[7:0], q1[7:0]};
  genvar k;
  generate
    for (k = 0; k < 4; k = k + 1) begin : blk
      wire [3:0] p0 = nibs[16*k +: 4], p1 = nibs[16*k + 4 +: 4], r0 = nibs[16*k + 8 +: 4], r1 = nibs[16*k + 12 +: 4];
      wire signed [18:0] w0 = $signed({15'd0, p0}) + (($signed({15'd0, r0}) - 19'sd8) <<< 15);
      wire signed [18:0] w1 = $signed({15'd0, p1}) + (($signed({15'd0, r1}) - 19'sd8) <<< 15);
      reg signed [18:0] x0, x1;
      reg [7:0] y0, y1;
      reg signed [63:0] sum;
      always @(posedge clk) begin
        x0 <= w0; x1 <= w1; y0 <= a[16*k +: 8]; y1 <= a[16*k + 8 +: 8];
        sum <= x0 * $signed({10'd0, y0}) + x1 * $signed({10'd0, y1}) + chain[k];
      end
      assign chain[k + 1] = sum;
    end
  endgenerate
  assign lo = chain[4][14:0];
  assign hi = chain[4][31:15];
endmodule

// A 32-element sub-block: four elements' lanes summed. s is the sub-block's sum of excess codes, which restores row
// two's offset: p2 = sum hi + 8.s.
module logos_subdot32(input [255:0] a, input [127:0] q1, input [127:0] q2, input [12:0] s,
                      output [16:0] p1, output signed [18:0] p2);
  wire [14:0] lo [0:3];
  wire signed [16:0] hi [0:3];
  genvar j;
  generate
    for (j = 0; j < 4; j = j + 1) begin : pe
      logos_pe8 u(.a(a[64*j +: 64]), .q1(q1[32*j +: 32]), .q2(q2[32*j +: 32]), .lo(lo[j]), .hi(hi[j]));
    end
  endgenerate
  assign p1 = {2'd0, lo[0]} + {2'd0, lo[1]} + {2'd0, lo[2]} + {2'd0, lo[3]};
  assign p2 = hi[0] + hi[1] + hi[2] + hi[3] + $signed({3'd0, s, 3'd0});
endmodule
"#;
