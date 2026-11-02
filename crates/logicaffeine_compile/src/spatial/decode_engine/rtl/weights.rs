//! The weight decoders as Verilog: each GGUF block format the array reads, unpacked in place into one form the array
//! and the block-scale unit share.
//!
//! Every weight is `a·c_u·(code − z) + b·n_u`: `code` an unsigned integer (one or two 4-bit digits), `z` the format's
//! offset, `c_u` and `n_u` small signed integers of the weight's 16-element unit, `a` and `b` the block's binary16
//! scales. That is the coded row's `coef = c_u·(code − z)` and `offs = n_u`
//! ([`crate::spatial::decode_engine::numerics::CodedRow`]), so a decoder is held to the coded row element by element.
//!
//! A decoder `logos_dec_<format>` takes one block's bytes (`blk`, byte 0 in bits 7:0) and gives `code` (8 bits an
//! element), `c` and `n` (16 signed bits a unit), `a` and `b` (the scales' bits), and its offset `z` as a constant. An
//! IQ4 nibble indexes a non-linear level; its code is the level in excess form, `level + 128`.

use crate::model::quant::{GgmlType, KVALUES_IQ4NL};

/// A format's decoder: the offset it subtracts from its codes and the bits a code spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoder {
    pub ty: GgmlType,
    pub offset: u32,
    pub code_bits: u32,
}

/// Every format the engine reads, with its decoder.
pub const DECODERS: [Decoder; 12] = [
    Decoder { ty: GgmlType::Q4_0, offset: 8, code_bits: 4 },
    Decoder { ty: GgmlType::Q4_1, offset: 0, code_bits: 4 },
    Decoder { ty: GgmlType::Q5_0, offset: 16, code_bits: 5 },
    Decoder { ty: GgmlType::Q5_1, offset: 0, code_bits: 5 },
    Decoder { ty: GgmlType::Q8_0, offset: 128, code_bits: 8 },
    Decoder { ty: GgmlType::Q2K, offset: 0, code_bits: 2 },
    Decoder { ty: GgmlType::Q3K, offset: 4, code_bits: 3 },
    Decoder { ty: GgmlType::Q4K, offset: 0, code_bits: 4 },
    Decoder { ty: GgmlType::Q5K, offset: 0, code_bits: 5 },
    Decoder { ty: GgmlType::Q6K, offset: 32, code_bits: 6 },
    Decoder { ty: GgmlType::Iq4Nl, offset: 128, code_bits: 8 },
    Decoder { ty: GgmlType::Iq4Xs, offset: 128, code_bits: 8 },
];

/// Elements one unit multiplier `c`/`n` covers.
pub const UNIT: usize = 16;

/// One block as its decoder decodes it: every element's code, every unit's multipliers, the two scales' bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Units {
    pub code: Vec<u32>,
    pub c: Vec<i16>,
    pub n: Vec<i16>,
    pub a: u16,
    pub b: u16,
}

/// `ty`'s decoder in software, field for field: what `logos_dec_<format>` gives for the block `blk`.
pub fn units(ty: GgmlType, blk: &[u8]) -> Units {
    assert_eq!(blk.len(), ty.block_bytes(), "one {} block", ty.name());
    let half = |at: usize| u16::from_le_bytes([blk[at], blk[at + 1]]);
    let word = |at: usize| u32::from_le_bytes([blk[at], blk[at + 1], blk[at + 2], blk[at + 3]]);
    let n = ty.block_len();
    let mut code = vec![0u32; n];
    let mut c = vec![1i16; n / UNIT];
    let mut nn = vec![0i16; n / UNIT];
    let (a, b) = match ty {
        GgmlType::Q4_0 | GgmlType::Q4_1 => {
            let qs = if ty == GgmlType::Q4_0 { 2 } else { 4 };
            for j in 0..16 {
                code[j] = (blk[qs + j] & 15) as u32;
                code[j + 16] = (blk[qs + j] >> 4) as u32;
            }
            if ty == GgmlType::Q4_1 {
                nn = vec![1; 2];
                (half(0), half(2))
            } else {
                (half(0), 0)
            }
        }
        GgmlType::Q5_0 | GgmlType::Q5_1 => {
            let one = ty == GgmlType::Q5_1;
            let (qh, qs) = if one { (word(4), 8) } else { (word(2), 6) };
            for j in 0..16 {
                code[j] = (blk[qs + j] & 15) as u32 | ((qh >> j) & 1) << 4;
                code[j + 16] = (blk[qs + j] >> 4) as u32 | ((qh >> (j + 16)) & 1) << 4;
            }
            if one {
                nn = vec![1; 2];
                (half(0), half(2))
            } else {
                (half(0), 0)
            }
        }
        GgmlType::Q8_0 => {
            for j in 0..32 {
                code[j] = (blk[2 + j] ^ 0x80) as u32;
            }
            (half(0), 0)
        }
        GgmlType::Q2K => {
            for h in 0..2 {
                for sh in 0..4 {
                    for hf in 0..2 {
                        let u = 8 * h + 2 * sh + hf;
                        c[u] = (blk[u] & 15) as i16;
                        nn[u] = -((blk[u] >> 4) as i16);
                        for l in 0..16 {
                            code[128 * h + 32 * sh + 16 * hf + l] = ((blk[16 + 32 * h + 16 * hf + l] >> (2 * sh)) & 3) as u32;
                        }
                    }
                }
            }
            (half(80), half(82))
        }
        GgmlType::Q3K => {
            let (w0, w1, w2) = (word(96), word(100), word(104));
            let (k1, k2) = (0x0303_0303u32, 0x0f0f_0f0fu32);
            let scales: Vec<u8> = [
                (w0 & k2) | ((w2 & k1) << 4),
                (w1 & k2) | (((w2 >> 2) & k1) << 4),
                ((w0 >> 4) & k2) | (((w2 >> 4) & k1) << 4),
                ((w1 >> 4) & k2) | (((w2 >> 6) & k1) << 4),
            ]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
            for h in 0..2 {
                for sh in 0..4 {
                    for hf in 0..2 {
                        let u = 8 * h + 2 * sh + hf;
                        c[u] = scales[u] as i16 - 32;
                        for l in 0..16 {
                            let low = (blk[32 + 32 * h + 16 * hf + l] >> (2 * sh)) & 3;
                            let high = (blk[16 * hf + l] >> (4 * h + sh)) & 1;
                            code[128 * h + 32 * sh + 16 * hf + l] = (high << 2 | low) as u32;
                        }
                    }
                }
            }
            (half(108), 0)
        }
        GgmlType::Q4K | GgmlType::Q5K => {
            let five = ty == GgmlType::Q5K;
            let q = &blk[4..16];
            let scale_min = |j: usize| -> (u8, u8) {
                if j < 4 {
                    (q[j] & 63, q[j + 4] & 63)
                } else {
                    ((q[j + 4] & 15) | ((q[j - 4] >> 6) << 4), (q[j + 4] >> 4) | ((q[j] >> 6) << 4))
                }
            };
            let qs = if five { 48 } else { 16 };
            for p in 0..4 {
                let (s1, m1) = scale_min(2 * p);
                let (s2, m2) = scale_min(2 * p + 1);
                for (u, (sc, mn)) in [(4 * p, (s1, m1)), (4 * p + 1, (s1, m1)), (4 * p + 2, (s2, m2)), (4 * p + 3, (s2, m2))] {
                    c[u] = sc as i16;
                    nn[u] = -(mn as i16);
                }
                for l in 0..32 {
                    let byte = blk[qs + 32 * p + l];
                    let (hi1, hi2) = if five { (((blk[16 + l] >> (2 * p)) & 1) as u32, ((blk[16 + l] >> (2 * p + 1)) & 1) as u32) } else { (0, 0) };
                    code[64 * p + l] = (byte & 15) as u32 | hi1 << 4;
                    code[64 * p + 32 + l] = (byte >> 4) as u32 | hi2 << 4;
                }
            }
            (half(0), half(2))
        }
        GgmlType::Q6K => {
            for part in 0..2 {
                for u in 0..8 {
                    c[8 * part + u] = blk[192 + 8 * part + u] as i8 as i16;
                }
                for l in 0..32 {
                    let (ql0, ql1, qh) = (blk[64 * part + l], blk[64 * part + l + 32], blk[128 + 32 * part + l]);
                    code[128 * part + l] = (ql0 & 15) as u32 | ((qh & 3) as u32) << 4;
                    code[128 * part + l + 32] = (ql1 & 15) as u32 | (((qh >> 2) & 3) as u32) << 4;
                    code[128 * part + l + 64] = (ql0 >> 4) as u32 | (((qh >> 4) & 3) as u32) << 4;
                    code[128 * part + l + 96] = (ql1 >> 4) as u32 | (((qh >> 6) & 3) as u32) << 4;
                }
            }
            (half(208), 0)
        }
        GgmlType::Iq4Nl | GgmlType::Iq4Xs => {
            // A nibble's level as an excess code: the array reads `level + 128` as two digits.
            let level = |nibble: u8| (KVALUES_IQ4NL[nibble as usize] as i32 + 128) as u32;
            if ty == GgmlType::Iq4Nl {
                for j in 0..16 {
                    code[j] = level(blk[2 + j] & 15);
                    code[j + 16] = level(blk[2 + j] >> 4);
                }
            } else {
                let scales_h = half(2);
                for ib in 0..8 {
                    let ls = ((blk[4 + ib / 2] >> (4 * (ib % 2))) & 0xf) as i16 | ((((scales_h >> (2 * ib)) & 3) as i16) << 4);
                    c[2 * ib] = ls - 32;
                    c[2 * ib + 1] = ls - 32;
                    for j in 0..16 {
                        code[32 * ib + j] = level(blk[8 + 16 * ib + j] & 15);
                        code[32 * ib + 16 + j] = level(blk[8 + 16 * ib + j] >> 4);
                    }
                }
            }
            (half(0), 0)
        }
        other => panic!("no decoder for {}", other.name()),
    };
    Units { code, c, n: nn, a, b }
}

/// The module name of `ty`'s decoder.
pub fn decoder_name(ty: GgmlType) -> String {
    format!("logos_dec_{}", ty.name().to_lowercase())
}

/// Every decoder.
pub fn weights_verilog() -> &'static str {
    WEIGHTS_VERILOG
}

const WEIGHTS_VERILOG: &str = r#"// The block decoders: GGUF bytes to codes, unit multipliers and the block's scales.

// ggml's get_scale_min_k4: sub-block j's 6-bit scale and min from a K-quant's 12 packed bytes.
module logos_scale_min_k4(input [95:0] q, input [2:0] j, output [5:0] sc, output [5:0] mn);
  function [7:0] qb(input [95:0] v, input [3:0] i); qb = v[8*i +: 8]; endfunction
  wire [7:0] here = qb(q, {1'b0, j}), ahead = qb(q, {1'b0, j} + 4'd4), behind = qb(q, {1'b0, j} - 4'd4);
  assign sc = (j < 3'd4) ? here[5:0] : {behind[7:6], ahead[3:0]};
  assign mn = (j < 3'd4) ? ahead[5:0] : {here[7:6], ahead[7:4]};
endmodule

module logos_dec_q4_0(input [143:0] blk, output [255:0] code, output [31:0] c, output [31:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = 16'd0;
  assign c = {16'sd1, 16'sd1};
  assign n = 32'd0;
  genvar j;
  generate for (j = 0; j < 16; j = j + 1) begin : e
    assign code[8*j +: 8] = {4'd0, blk[16 + 8*j +: 4]};
    assign code[8*(j+16) +: 8] = {4'd0, blk[20 + 8*j +: 4]};
  end endgenerate
endmodule

module logos_dec_q4_1(input [159:0] blk, output [255:0] code, output [31:0] c, output [31:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = blk[31:16];
  assign c = {16'sd1, 16'sd1};
  assign n = {16'sd1, 16'sd1};
  genvar j;
  generate for (j = 0; j < 16; j = j + 1) begin : e
    assign code[8*j +: 8] = {4'd0, blk[32 + 8*j +: 4]};
    assign code[8*(j+16) +: 8] = {4'd0, blk[36 + 8*j +: 4]};
  end endgenerate
endmodule

module logos_dec_q5_0(input [175:0] blk, output [255:0] code, output [31:0] c, output [31:0] n, output [15:0] a, output [15:0] b);
  wire [31:0] qh = blk[47:16];
  assign a = blk[15:0];
  assign b = 16'd0;
  assign c = {16'sd1, 16'sd1};
  assign n = 32'd0;
  genvar j;
  generate for (j = 0; j < 16; j = j + 1) begin : e
    assign code[8*j +: 8] = {3'd0, qh[j], blk[48 + 8*j +: 4]};
    assign code[8*(j+16) +: 8] = {3'd0, qh[j+16], blk[52 + 8*j +: 4]};
  end endgenerate
endmodule

module logos_dec_q5_1(input [191:0] blk, output [255:0] code, output [31:0] c, output [31:0] n, output [15:0] a, output [15:0] b);
  wire [31:0] qh = blk[63:32];
  assign a = blk[15:0];
  assign b = blk[31:16];
  assign c = {16'sd1, 16'sd1};
  assign n = {16'sd1, 16'sd1};
  genvar j;
  generate for (j = 0; j < 16; j = j + 1) begin : e
    assign code[8*j +: 8] = {3'd0, qh[j], blk[64 + 8*j +: 4]};
    assign code[8*(j+16) +: 8] = {3'd0, qh[j+16], blk[68 + 8*j +: 4]};
  end endgenerate
endmodule

module logos_dec_q8_0(input [271:0] blk, output [255:0] code, output [31:0] c, output [31:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = 16'd0;
  assign c = {16'sd1, 16'sd1};
  assign n = 32'd0;
  genvar j;
  generate for (j = 0; j < 32; j = j + 1) begin : e
    assign code[8*j +: 8] = blk[16 + 8*j +: 8] ^ 8'h80;
  end endgenerate
endmodule

module logos_dec_q2_k(input [671:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[655:640];
  assign b = blk[671:656];
  genvar h, s, hf, l;
  generate
    for (h = 0; h < 2; h = h + 1) begin : nb
      for (s = 0; s < 4; s = s + 1) begin : sh
        for (hf = 0; hf < 2; hf = hf + 1) begin : half
          localparam integer U = 8*h + 2*s + hf;
          wire [7:0] sc = blk[8*U +: 8];
          assign c[16*U +: 16] = {12'd0, sc[3:0]};
          assign n[16*U +: 16] = -$signed({12'd0, sc[7:4]});
          for (l = 0; l < 16; l = l + 1) begin : el
            wire [7:0] q = blk[128 + 8*(32*h + 16*hf + l) +: 8];
            assign code[8*(128*h + 32*s + 16*hf + l) +: 8] = {6'd0, q[2*s +: 2]};
          end
        end
      end
    end
  endgenerate
endmodule

module logos_dec_q3_k(input [879:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[879:864];
  assign b = 16'd0;
  assign n = 256'd0;
  // ggml's 6-bit scales from the 12 packed bytes at 96..107.
  wire [31:0] w0 = blk[768 +: 32], w1 = blk[800 +: 32], w2 = blk[832 +: 32];
  wire [31:0] k1 = 32'h0303_0303, k2 = 32'h0f0f_0f0f;
  wire [31:0] s2 = ((w0 >> 4) & k2) | (((w2 >> 4) & k1) << 4);
  wire [31:0] s3 = ((w1 >> 4) & k2) | (((w2 >> 6) & k1) << 4);
  wire [31:0] s0 = (w0 & k2) | ((w2 & k1) << 4);
  wire [31:0] s1 = (w1 & k2) | (((w2 >> 2) & k1) << 4);
  wire [127:0] scales = {s3, s2, s1, s0};
  genvar h, s, hf, l;
  generate
    for (h = 0; h < 2; h = h + 1) begin : nb
      for (s = 0; s < 4; s = s + 1) begin : sh
        for (hf = 0; hf < 2; hf = hf + 1) begin : half
          localparam integer U = 8*h + 2*s + hf;
          wire [7:0] sc = scales[8*U +: 8];
          assign c[16*U +: 16] = $signed({8'd0, sc}) - 16'sd32;
          for (l = 0; l < 16; l = l + 1) begin : el
            wire [7:0] q = blk[256 + 8*(32*h + 16*hf + l) +: 8];
            wire [7:0] hm = blk[8*(16*hf + l) +: 8];
            assign code[8*(128*h + 32*s + 16*hf + l) +: 8] = {5'd0, hm[4*h + s], q[2*s +: 2]};
          end
        end
      end
    end
  endgenerate
endmodule

module logos_dec_q4_k(input [1151:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = blk[31:16];
  wire [95:0] scq = blk[127:32];
  genvar p, l;
  generate
    for (p = 0; p < 4; p = p + 1) begin : pair
      wire [5:0] s1, m1, s2, m2;
      logos_scale_min_k4 lo(.q(scq), .j(2*p), .sc(s1), .mn(m1));
      logos_scale_min_k4 hi(.q(scq), .j(2*p+1), .sc(s2), .mn(m2));
      assign c[16*(4*p) +: 32] = {{10'd0, s1}, {10'd0, s1}};
      assign c[16*(4*p+2) +: 32] = {{10'd0, s2}, {10'd0, s2}};
      assign n[16*(4*p) +: 32] = {-$signed({10'd0, m1}), -$signed({10'd0, m1})};
      assign n[16*(4*p+2) +: 32] = {-$signed({10'd0, m2}), -$signed({10'd0, m2})};
      for (l = 0; l < 32; l = l + 1) begin : el
        wire [7:0] q = blk[128 + 8*(32*p + l) +: 8];
        assign code[8*(64*p + l) +: 8] = {4'd0, q[3:0]};
        assign code[8*(64*p + 32 + l) +: 8] = {4'd0, q[7:4]};
      end
    end
  endgenerate
endmodule

module logos_dec_q5_k(input [1407:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = blk[31:16];
  wire [95:0] scq = blk[127:32];
  genvar p, l;
  generate
    for (p = 0; p < 4; p = p + 1) begin : pair
      wire [5:0] s1, m1, s2, m2;
      logos_scale_min_k4 lo(.q(scq), .j(2*p), .sc(s1), .mn(m1));
      logos_scale_min_k4 hi(.q(scq), .j(2*p+1), .sc(s2), .mn(m2));
      assign c[16*(4*p) +: 32] = {{10'd0, s1}, {10'd0, s1}};
      assign c[16*(4*p+2) +: 32] = {{10'd0, s2}, {10'd0, s2}};
      assign n[16*(4*p) +: 32] = {-$signed({10'd0, m1}), -$signed({10'd0, m1})};
      assign n[16*(4*p+2) +: 32] = {-$signed({10'd0, m2}), -$signed({10'd0, m2})};
      for (l = 0; l < 32; l = l + 1) begin : el
        wire [7:0] qh = blk[128 + 8*l +: 8];
        wire [7:0] q = blk[384 + 8*(32*p + l) +: 8];
        assign code[8*(64*p + l) +: 8] = {3'd0, qh[2*p], q[3:0]};
        assign code[8*(64*p + 32 + l) +: 8] = {3'd0, qh[2*p+1], q[7:4]};
      end
    end
  endgenerate
endmodule

module logos_dec_q6_k(input [1679:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[1679:1664];
  assign b = 16'd0;
  assign n = 256'd0;
  genvar part, u, l;
  generate
    for (part = 0; part < 2; part = part + 1) begin : pt
      for (u = 0; u < 8; u = u + 1) begin : un
        wire [7:0] sc = blk[8*(192 + 8*part + u) +: 8];
        assign c[16*(8*part + u) +: 16] = {{8{sc[7]}}, sc};
      end
      for (l = 0; l < 32; l = l + 1) begin : el
        wire [7:0] ql0 = blk[8*(64*part + l) +: 8];
        wire [7:0] ql1 = blk[8*(64*part + l + 32) +: 8];
        wire [7:0] qh = blk[8*(128 + 32*part + l) +: 8];
        assign code[8*(128*part + l) +: 8] = {2'd0, qh[1:0], ql0[3:0]};
        assign code[8*(128*part + l + 32) +: 8] = {2'd0, qh[3:2], ql1[3:0]};
        assign code[8*(128*part + l + 64) +: 8] = {2'd0, qh[5:4], ql0[7:4]};
        assign code[8*(128*part + l + 96) +: 8] = {2'd0, qh[7:6], ql1[7:4]};
      end
    end
  endgenerate
endmodule

// An IQ4 nibble's level as an excess code (ggml's kvalues_iq4nl, plus 128).
module logos_iq4_level(input [3:0] q, output reg [7:0] level);
  always @* begin
    case (q)
      4'd0: level = 8'd1;
      4'd1: level = 8'd24;
      4'd2: level = 8'd45;
      4'd3: level = 8'd63;
      4'd4: level = 8'd79;
      4'd5: level = 8'd93;
      4'd6: level = 8'd106;
      4'd7: level = 8'd118;
      4'd8: level = 8'd129;
      4'd9: level = 8'd141;
      4'd10: level = 8'd153;
      4'd11: level = 8'd166;
      4'd12: level = 8'd181;
      4'd13: level = 8'd197;
      4'd14: level = 8'd217;
      4'd15: level = 8'd241;
    endcase
  end
endmodule

module logos_dec_iq4_nl(input [143:0] blk, output [255:0] code, output [31:0] c, output [31:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = 16'd0;
  assign c = {16'sd1, 16'sd1};
  assign n = 32'd0;
  genvar j;
  generate for (j = 0; j < 16; j = j + 1) begin : e
    logos_iq4_level lo(.q(blk[16 + 8*j +: 4]), .level(code[8*j +: 8]));
    logos_iq4_level hi(.q(blk[20 + 8*j +: 4]), .level(code[8*(j+16) +: 8]));
  end endgenerate
endmodule

module logos_dec_iq4_xs(input [1087:0] blk, output [2047:0] code, output [255:0] c, output [255:0] n, output [15:0] a, output [15:0] b);
  assign a = blk[15:0];
  assign b = 16'd0;
  assign n = 256'd0;
  wire [15:0] scales_h = blk[31:16];
  genvar ib, j;
  generate
    for (ib = 0; ib < 8; ib = ib + 1) begin : sub
      wire [5:0] ls = {scales_h[2*ib +: 2], blk[32 + 8*(ib/2) + 4*(ib%2) +: 4]};
      wire signed [15:0] cs = $signed({10'd0, ls}) - 16'sd32;
      assign c[16*(2*ib) +: 16] = cs;
      assign c[16*(2*ib + 1) +: 16] = cs;
      for (j = 0; j < 16; j = j + 1) begin : e
        logos_iq4_level lo(.q(blk[64 + 8*(16*ib + j) +: 4]), .level(code[8*(32*ib + j) +: 8]));
        logos_iq4_level hi(.q(blk[68 + 8*(16*ib + j) +: 4]), .level(code[8*(32*ib + 16 + j) +: 8]));
      end
    end
  endgenerate
endmodule
"#;
