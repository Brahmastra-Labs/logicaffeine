//! scratchpad_rtl — the on-chip scratchpad subsystem in silicon.
//!
//! The handout claims "SECDED scratchpads" and "double-buffered: compute tile N while
//! DMA streams tile N+1" (`CHIP_MEMORY_ANSWERS.md` §3, `MEMORY_EDGE_SPEC.md`). This
//! module makes both real:
//!   • a **SECDED(72,64) code** — single-error-CORRECT, double-error-DETECT — as a Rust
//!     oracle (exhaustively tested over every single-bit and a sweep of double-bit
//!     faults) and a generated-from-the-same-parity-structure Verilog encoder/decoder,
//!     with the single-error-correction property **proven by our certified SAT** on a
//!     tractable Hamming instance and verified bit-exact under iverilog on the full 64;
//!   • a **double-buffer (ping-pong) controller** whose safety invariant — compute never
//!     reads the bank the DMA is filling — is checkable by construction.
//!
//! The SECDED construction is the textbook Hamming-SEC (check bits at power-of-two
//! codeword positions) plus one overall-parity bit for DED. Encoder and decoder are
//! GENERATED from the same position→check-bit incidence, so they cannot diverge.

/// Total codeword width (72) and data width (64) of the scratchpad ECC.
pub const CW_BITS: usize = 72; // position 0 = overall parity; 1..=71 = Hamming(71,64)
pub const DATA_BITS: usize = 64;
const HAMMING_HI: usize = 71; // Hamming positions are 1..=71
const CHECK_BITS: usize = 7; // powers of two 1,2,4,8,16,32,64 within 1..=71

/// Is `pos` a power of two (a Hamming check-bit position)?
fn is_check_pos(pos: usize) -> bool {
    pos != 0 && (pos & (pos - 1)) == 0
}

/// The data-bit index carried at Hamming codeword position `pos` (non-power-of-two),
/// or `None` if `pos` is a check position. Data bits fill non-power-of-two positions
/// 1..=71 in order.
fn data_index_at(pos: usize) -> Option<usize> {
    if pos == 0 || is_check_pos(pos) {
        return None;
    }
    let mut di = 0;
    for p in 1..=HAMMING_HI {
        if !is_check_pos(p) {
            if p == pos {
                return Some(di);
            }
            di += 1;
        }
    }
    None
}

/// The decode verdict.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decode {
    /// Syndrome zero, parity even — clean.
    NoError,
    /// A single bit error at Hamming position `pos` (1..=71) — corrected.
    Corrected(usize),
    /// The overall-parity bit itself flipped (syndrome zero, parity odd) — corrected.
    CorrectedParityBit,
    /// Uncorrectable, DETECTED: a non-zero syndrome with even parity (two errors), or odd parity with a syndrome
    /// naming no codeword position (an odd number of errors past one).
    DoubleError,
}

/// SECDED encode: 64 data bits → a 72-bit codeword (`bit 0` = overall parity,
/// bits 1..=71 = Hamming(71,64)).
pub fn encode(data: u64) -> u128 {
    let mut cw = [false; CW_BITS];
    // Place data into non-power-of-two positions 1..=71.
    for pos in 1..=HAMMING_HI {
        if let Some(di) = data_index_at(pos) {
            cw[pos] = (data >> di) & 1 == 1;
        }
    }
    // Each check bit c = XOR of the data positions (bit i set) so that the parity over
    // {positions with bit i set} is even.
    for i in 0..CHECK_BITS {
        let cp = 1usize << i;
        let mut parity = false;
        for pos in 1..=HAMMING_HI {
            if pos != cp && (pos & cp) != 0 {
                parity ^= cw[pos];
            }
        }
        cw[cp] = parity;
    }
    // Overall parity over 1..=71.
    let mut op = false;
    for pos in 1..=HAMMING_HI {
        op ^= cw[pos];
    }
    cw[0] = op;
    pack(&cw)
}

/// SECDED decode: a (possibly corrupted) 72-bit codeword → (recovered data, verdict).
pub fn decode(cw_bits: u128) -> (u64, Decode) {
    let cw = unpack(cw_bits);
    // Syndrome = which check equations fail.
    let mut syndrome = 0usize;
    for i in 0..CHECK_BITS {
        let cp = 1usize << i;
        let mut parity = false;
        for pos in 1..=HAMMING_HI {
            if pos & cp != 0 {
                parity ^= cw[pos];
            }
        }
        if parity {
            syndrome |= cp;
        }
    }
    // Overall parity across the WHOLE codeword (incl. bit 0) — odd ⇒ odd # of errors.
    let odd_errors = (0..CW_BITS).fold(false, |acc, p| acc ^ cw[p]);

    let mut fixed = cw;
    let verdict = match (syndrome, odd_errors) {
        (0, false) => Decode::NoError,
        (0, true) => Decode::CorrectedParityBit, // only the overall bit flipped
        (s, true) if s <= HAMMING_HI => {
            fixed[s] ^= true; // single error at position s — correct it
            Decode::Corrected(s)
        }
        _ => Decode::DoubleError, // syndrome set but parity even ⇒ 2 errors
    };
    let mut data = 0u64;
    for pos in 1..=HAMMING_HI {
        if let Some(di) = data_index_at(pos) {
            if fixed[pos] {
                data |= 1u64 << di;
            }
        }
    }
    (data, verdict)
}

fn pack(cw: &[bool; CW_BITS]) -> u128 {
    let mut v = 0u128;
    for (i, &b) in cw.iter().enumerate() {
        if b {
            v |= 1u128 << i;
        }
    }
    v
}

fn unpack(v: u128) -> [bool; CW_BITS] {
    let mut cw = [false; CW_BITS];
    for (i, slot) in cw.iter_mut().enumerate() {
        *slot = (v >> i) & 1 == 1;
    }
    cw
}

/// The data positions (0-indexed data bits) each check bit `i` XORs — the generator
/// incidence shared by the Rust oracle and the emitted RTL.
fn check_bit_terms(i: usize) -> Vec<usize> {
    let cp = 1usize << i;
    let mut terms = Vec::new();
    for pos in 1..=HAMMING_HI {
        if pos != cp && (pos & cp) != 0 {
            if let Some(di) = data_index_at(pos) {
                terms.push(di);
            }
        }
    }
    terms
}

/// A Verilog expression true iff the unsigned number on `bits` (LSB first) exceeds `c`, folded from the LSB up: past
/// bit `i`, the number exceeds `c` iff bit `i` is set and the lower bits already exceed it, where `c` has a one there,
/// or bit `i` is set or they do, where `c` has a zero. The fold keeps one gate per bit and drops the constant terms.
fn exceeds(bits: &[String], c: usize) -> String {
    if c >> bits.len() != 0 {
        return "1'b0".to_string();
    }
    let mut acc: Option<String> = None;
    for (i, b) in bits.iter().enumerate() {
        acc = match (c >> i & 1 == 1, acc) {
            (true, None) => None,
            (true, Some(a)) => Some(format!("({b} & {a})")),
            (false, None) => Some(b.clone()),
            (false, Some(a)) => Some(format!("({b} | {a})")),
        };
    }
    acc.unwrap_or_else(|| "1'b0".to_string())
}

/// Emit the SECDED(72,64) encoder + decoder/corrector as synthesizable Verilog,
/// generated from the SAME incidence as [`encode`]/[`decode`], so they cannot diverge.
/// Interface: `enc` takes 64-bit `din` → 72-bit `cw`; `dec` takes a 72-bit `cw_in` →
/// 64-bit `dout` + `corrected` + `double_err`.
pub fn emit_secded_verilog() -> String {
    let mut w = String::new();
    // Data-bit index at each Hamming position (for extract), and check-bit equations.
    // ── encoder ──
    w.push_str("// secded_enc — SECDED(72,64) encoder (Hamming SEC + overall parity).\n");
    w.push_str("// GENERATED from the same incidence as codegen_sva::scratchpad_rtl::encode.\n");
    w.push_str("module secded_enc(input [63:0] din, output [71:0] cw);\n");
    // Assign data bits to their Hamming positions.
    for pos in 1..=HAMMING_HI {
        if let Some(di) = data_index_at(pos) {
            w.push_str(&format!("  wire p{pos} = din[{di}];\n"));
        }
    }
    // Every parity is one reduction over its support, which synthesis maps as a balanced tree; a chained `a ^ b ^ …`
    // stays as deep as it is written.
    let reduce = |terms: &[usize]| {
        let t: Vec<String> = terms.iter().map(|d| format!("din[{d}]")).collect();
        format!("^{{{}}}", t.join(", "))
    };
    for i in 0..CHECK_BITS {
        w.push_str(&format!("  wire p{} = {};\n", 1usize << i, reduce(&check_bit_terms(i))));
    }
    // The overall parity over positions 1..=71, expanded to the data bits: each data bit counts once for its own
    // position and once more for every check bit that covers it, so it is in the parity iff that count is odd.
    let mut count = [1usize; DATA_BITS];
    for i in 0..CHECK_BITS {
        for d in check_bit_terms(i) {
            count[d] += 1;
        }
    }
    let overall: Vec<usize> = (0..DATA_BITS).filter(|&d| count[d] % 2 == 1).collect();
    w.push_str(&format!("  wire p0 = {};\n", reduce(&overall)));
    // Pack cw[0..71].
    w.push_str("  assign cw = {");
    for pos in (0..=HAMMING_HI).rev() {
        w.push_str(&format!("p{pos}"));
        if pos != 0 {
            w.push_str(", ");
        }
    }
    w.push_str("};\nendmodule\n\n");

    // ── decoder / corrector ──
    w.push_str("// secded_dec — SECDED(72,64) decoder: corrects any single-bit error,\n");
    w.push_str("// detects any double-bit error (double_err), recovers the 64 data bits.\n");
    w.push_str("module secded_dec(input [71:0] cw_in, output [63:0] dout,\n");
    w.push_str("                  output corrected, output double_err);\n");
    // Unpack positions.
    for pos in 0..=HAMMING_HI {
        w.push_str(&format!("  wire q{pos} = cw_in[{pos}];\n"));
    }
    // Syndrome bits: recompute each check equation over ALL positions with bit i set.
    for i in 0..CHECK_BITS {
        let cp = 1usize << i;
        let terms: Vec<String> = (1..=HAMMING_HI)
            .filter(|p| p & cp != 0)
            .map(|p| format!("q{p}"))
            .collect();
        w.push_str(&format!("  wire s{i} = ^{{{}}};\n", terms.join(", ")));
    }
    w.push_str(&format!(
        "  wire [{hi}:0] syndrome = {{{bits}}};\n",
        hi = CHECK_BITS - 1,
        bits = (0..CHECK_BITS)
            .rev()
            .map(|i| format!("s{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // Overall parity across all 72 bits.
    w.push_str("  wire odd_err = ^cw_in;\n");
    w.push_str("  wire syn_nz = |syndrome;\n");
    // An odd error is correctable only where the syndrome names a codeword position (zero names the overall parity
    // bit); one past the codeword is an odd number of errors past one, uncorrectable like every even one.
    let syndrome_bits: Vec<String> = (0..CHECK_BITS).map(|i| format!("s{i}")).collect();
    w.push_str(&format!(
        "  wire beyond = {};\n",
        exceeds(&syndrome_bits, HAMMING_HI)
    ));
    w.push_str("  assign corrected = odd_err & ~beyond;\n");
    w.push_str("  assign double_err = syn_nz & (~odd_err | beyond);\n");
    // Corrected value at each data position: flip qpos iff syndrome == pos and odd_err.
    for pos in 1..=HAMMING_HI {
        if let Some(di) = data_index_at(pos) {
            w.push_str(&format!(
                "  wire f{di} = q{pos} ^ (odd_err & (syndrome == {pos})); // pos {pos}\n"
            ));
        }
    }
    w.push_str("  assign dout = {");
    for di in (0..DATA_BITS).rev() {
        w.push_str(&format!("f{di}"));
        if di != 0 {
            w.push_str(", ");
        }
    }
    w.push_str("};\nendmodule\n");
    w
}

/// Emit the ping-pong double-buffer controller: `swap` toggles which bank compute reads
/// (`read_bank`) vs the DMA fills (`fill_bank` = ~read_bank). The safety invariant
/// `read_bank != fill_bank` holds by construction (they are complements).
pub fn emit_double_buffer_verilog() -> String {
    r#"// double_buffer — ping-pong scratchpad bank select. Compute reads read_bank while
// the DMA fills fill_bank; a swap flips both. read_bank != fill_bank by construction.
module double_buffer(input clk, input rst, input swap,
                     output reg read_bank, output fill_bank);
  assign fill_bank = ~read_bank;
  always @(posedge clk) begin
    if (rst) read_bank <= 1'b0;
    else if (swap) read_bank <= ~read_bank;
  end
endmodule
"#
    .to_string()
}

/// Emit a self-checking testbench: encode `din`, inject a fault pattern from
/// `secded_vec.hex` (each line: `din  flip_mask  expect_dout  expect_corrected
/// expect_double`), decode, and compare. Prints PASS/FAIL.
pub fn emit_secded_testbench() -> String {
    r#"// tb_secded — SECDED(72,64) encode -> inject fault -> decode, vs the oracle.
`timescale 1ns/1ps
module tb_secded;
  reg [63:0] din, edout; reg [71:0] mask; reg ecorr, edbl;
  integer fd, r, errs = 0, n = 0;
  wire [71:0] cw; wire [63:0] dout; wire corrected, double_err;
  secded_enc enc(.din(din), .cw(cw));
  wire [71:0] cw_corrupt = cw ^ mask;
  secded_dec dec(.cw_in(cw_corrupt), .dout(dout), .corrected(corrected), .double_err(double_err));
  initial begin
    fd = $fopen("secded_vec.hex", "r");
    while (!$feof(fd)) begin
      r = $fscanf(fd, "%h %h %h %b %b\n", din, mask, edout, ecorr, edbl);
      if (r == 5) begin
        #1;
        if (double_err !== edbl) begin errs=errs+1; $display("FAIL row %0d: double_err %b!=%b", n, double_err, edbl); end
        else if (!edbl && dout !== edout) begin errs=errs+1; $display("FAIL row %0d: data %h!=%h", n, dout, edout); end
        n = n + 1;
      end
    end
    if (errs == 0) $display("PASS: %0d SECDED fault cases match the oracle (SEC + DED)", n);
    $finish;
  end
endmodule
"#
    .to_string()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/scratchpad_rtl/tests.rs"]
mod tests;
