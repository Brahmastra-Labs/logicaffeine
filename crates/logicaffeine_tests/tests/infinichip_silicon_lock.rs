//! infinichip_silicon_lock — the Infinichip's ECC data path on the simulator, against the certified SECDED oracle
//! (`scratchpad_rtl::{encode, decode}`).
//!
//!   1. The emitted codec is the oracle bit for bit: the encoder's codeword for every data word tried, and the
//!      decoder's data, `corrected` and `double_err` for the clean codeword, every single fault, every double fault,
//!      and odd faults past one, including one for every syndrome that names no codeword position.
//!   2. The codec hands synthesis balanced parity. Before any optimizer touches it, no path through the encoder is
//!      longer than the reduction tree of its widest parity, and no path through the decoder is longer than that tree
//!      twice over plus the flip: the syndrome, then an error locator that is itself a tree over the syndrome.
//!   3. The composed top registers each codec at the die boundary: one edge after a data word and a codeword are
//!      presented, the written codeword and the read verdict are the oracle's for them, whatever the ports carry
//!      after that edge, with a new pair taken on every edge.
//!   4. The shipped `infinichip.v`, `tb_infinichip.v`, `secded.v` and `tb_secded.v` pass their testbenches.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test infinichip_silicon_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::infinichip_rtl::emit_infinichip_verilog;
use logicaffeine_compile::codegen_sva::scratchpad_rtl::{
    decode, emit_secded_verilog, encode, Decode, CW_BITS, DATA_BITS,
};
use std::path::{Path, PathBuf};
use std::process::Command;

fn silicon() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// The data words every sweep starts from: both constants, both alternating patterns, the single-bit extremes and
/// two irregular words.
const WORDS: [u64; 8] = [
    0,
    u64::MAX,
    0x5555_5555_5555_5555,
    0xAAAA_AAAA_AAAA_AAAA,
    1,
    1 << 63,
    0xDEAD_BEEF_CAFE_F00D,
    0x0123_4567_89AB_CDEF,
];

/// A deterministic 64-bit stream (SplitMix64), so every vector set is the same on every run.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A fault of exactly `weight` distinct codeword bits.
    fn fault(&mut self, weight: u32) -> u128 {
        let mut m = 0u128;
        while m.count_ones() < weight {
            m |= 1u128 << (self.next() % CW_BITS as u64);
        }
        m
    }
}

/// An odd fault of three bits whose syndrome is `s`: positions 64 and 1, and the one that makes their XOR `s`. Every
/// `s` from 72 to 127 names no codeword position, so the oracle calls each such fault uncorrectable.
fn out_of_range_fault(s: usize) -> u128 {
    let third = s ^ 64 ^ 1;
    assert!(third != 1 && third != 64 && third < CW_BITS, "syndrome {s}: third position {third}");
    (1u128 << 64) | (1u128 << 1) | (1u128 << third)
}

/// The decoder's two status outputs for an oracle verdict.
fn flags(v: Decode) -> (u8, u8) {
    match v {
        Decode::NoError => (0, 0),
        Decode::Corrected(_) | Decode::CorrectedParityBit => (1, 0),
        Decode::DoubleError => (0, 1),
    }
}

/// Every fault the codec sweep applies, against each word it is applied to.
fn codec_vectors() -> Vec<(u64, u128)> {
    let mut v = Vec::new();
    for &w in &WORDS {
        v.push((w, 0));
        for bit in 0..CW_BITS {
            v.push((w, 1u128 << bit));
        }
        for s in CW_BITS..128 {
            v.push((w, out_of_range_fault(s)));
        }
    }
    for &w in &WORDS[5..] {
        for a in 0..CW_BITS {
            for b in a + 1..CW_BITS {
                v.push((w, (1u128 << a) | (1u128 << b)));
            }
        }
    }
    let mut r = Stream(0x1F1F_C0DE);
    for k in 0..900u32 {
        let w = r.next();
        v.push((w, r.fault(3 + k % 3)));
    }
    v
}

fn iverilog(dir: &Path, sources: &[&str]) -> String {
    let build = Command::new("iverilog")
        .current_dir(dir)
        .args(["-g2012", "-o", "sim.vvp"])
        .args(sources)
        .output()
        .expect("run iverilog");
    assert!(build.status.success(), "iverilog failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new("vvp").current_dir(dir).arg("sim.vvp").output().expect("run vvp");
    assert!(run.status.success(), "vvp failed: {}", String::from_utf8_lossy(&run.stderr));
    String::from_utf8_lossy(&run.stdout).into_owned()
}

/// The `R ...` rows a testbench printed: codeword, data, corrected, double_err.
fn rows(out: &str) -> Vec<(u128, u64, u8, u8)> {
    out.lines()
        .filter_map(|l| l.strip_prefix("R "))
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            assert_eq!(f.len(), 4, "malformed row {l:?}");
            (
                u128::from_str_radix(f[0], 16).expect("codeword"),
                u64::from_str_radix(f[1], 16).expect("data"),
                f[2].parse().expect("corrected"),
                f[3].parse().expect("double_err"),
            )
        })
        .collect()
}

#[test]
fn the_vectors_reach_every_verdict_and_every_syndrome_past_the_codeword() {
    let v = codec_vectors();
    let mut seen = [0usize; 4];
    for &(w, m) in &v {
        let (_, verdict) = decode(encode(w) ^ m);
        seen[match verdict {
            Decode::NoError => 0,
            Decode::Corrected(_) => 1,
            Decode::CorrectedParityBit => 2,
            Decode::DoubleError => 3,
        }] += 1;
    }
    assert!(seen.iter().all(|&n| n > 0), "every verdict is exercised: {seen:?}");
    for s in CW_BITS..128 {
        let m = out_of_range_fault(s);
        assert_eq!(m.count_ones(), 3, "syndrome {s}: an odd fault");
        for &w in &WORDS {
            assert_eq!(
                decode(encode(w) ^ m).1,
                Decode::DoubleError,
                "syndrome {s} names no position: the oracle reports it uncorrectable"
            );
        }
    }
}

#[test]
fn the_emitted_codec_is_the_oracle_on_every_fault_class() {
    if !common::require_tool("iverilog") {
        return;
    }
    let v = codec_vectors();
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("secded.v"), emit_secded_verilog()).unwrap();
    let hex: String = v.iter().map(|(w, m)| format!("{w:016x} {m:018x}\n")).collect();
    std::fs::write(dir.path().join("vec.hex"), hex).unwrap();
    std::fs::write(
        dir.path().join("tb.v"),
        r#"module tb;
  reg [63:0] din; reg [71:0] mask; integer fd, r;
  wire [71:0] cw; wire [63:0] dout; wire corrected, double_err;
  secded_enc enc(.din(din), .cw(cw));
  secded_dec dec(.cw_in(cw ^ mask), .dout(dout), .corrected(corrected), .double_err(double_err));
  initial begin
    fd = $fopen("vec.hex", "r");
    while (!$feof(fd)) begin
      r = $fscanf(fd, "%h %h\n", din, mask);
      if (r == 2) begin #1; $display("R %018x %016x %b %b", cw, dout, corrected, double_err); end
    end
    $finish;
  end
endmodule
"#,
    )
    .unwrap();
    let got = rows(&iverilog(dir.path(), &["tb.v", "secded.v"]));
    assert_eq!(got.len(), v.len(), "one row per vector");
    for (k, (&(w, m), &(cw, data, c, d))) in v.iter().zip(&got).enumerate() {
        assert_eq!(cw, encode(w), "vector {k}: the encoder's codeword for {w:#018x}");
        let (want, verdict) = decode(encode(w) ^ m);
        assert_eq!(
            (data, (c, d)),
            (want, flags(verdict)),
            "vector {k}: {w:#018x} with fault {m:#020x} ({} bits): the oracle says {verdict:?}",
            m.count_ones()
        );
    }
}

/// The longest path, in gates, through `top` as the RTL writes it: elaborated, flattened and mapped to yosys's
/// one-bit gates, with constants folded and nothing restructured.
fn gate_depth(dir: &Path, file: &str, top: &str) -> usize {
    let out = Command::new("yosys")
        .current_dir(dir)
        .args([
            "-q",
            "-p",
            &format!("read_verilog {file}; hierarchy -top {top}; proc; flatten; opt; techmap; opt; tee -o ltp.txt ltp -noff"),
        ])
        .output()
        .expect("run yosys");
    assert!(out.status.success(), "yosys failed: {}", String::from_utf8_lossy(&out.stderr));
    let ltp = std::fs::read_to_string(dir.join("ltp.txt")).expect("ltp report");
    let at = ltp.find("length=").expect("ltp reports a length") + "length=".len();
    ltp[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().expect("length")
}

/// The depth of a balanced two-input reduction over `n` inputs.
fn tree_depth(n: usize) -> usize {
    (usize::BITS - (n.max(1) - 1).leading_zeros()) as usize
}

#[test]
fn the_codec_hands_synthesis_balanced_parity() {
    if !common::require_tool("yosys") {
        return;
    }
    // Each codeword bit is the parity of the data bits whose unit word sets it: the code is linear.
    let widest_check = (0..CW_BITS)
        .map(|j| (0..DATA_BITS).filter(|&d| encode(1u64 << d) >> j & 1 == 1).count())
        .max()
        .expect("a codeword bit");
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("secded.v"), emit_secded_verilog()).unwrap();
    let enc = gate_depth(dir.path(), "secded.v", "secded_enc");
    assert!(
        enc <= tree_depth(widest_check),
        "secded_enc is {enc} gates deep; the widest parity, over {widest_check} data bits, needs a tree of {}",
        tree_depth(widest_check)
    );
    // The decoder's widest parity is the overall one, over the whole codeword.
    let parity = tree_depth(CW_BITS);
    let dec = gate_depth(dir.path(), "secded.v", "secded_dec");
    assert!(
        dec <= 2 * parity + 1,
        "secded_dec is {dec} gates deep; a {parity}-deep parity tree, a locator no deeper and the flip make {}",
        2 * parity + 1
    );
}

const TOP_TB: &str = r#"module tb;
  reg clk = 0, rst = 1, cmd_valid = 0, swap = 0;
  reg [63:0] cmd_word = 0, wr_data = 0; reg [71:0] rd_cw = 0;
  wire cmd_ack, issue, rd_corrected, rd_double_err, read_bank, fill_bank;
  wire [2:0] op; wire [4:0] bank; wire [31:0] row; wire [71:0] wr_cw; wire [63:0] rd_data;
  reg [63:0] d, pd; reg [71:0] c, pc; integer fd, r;
  infinichip dut(.clk(clk), .rst(rst), .cmd_word(cmd_word), .cmd_valid(cmd_valid), .cmd_ack(cmd_ack),
                 .issue(issue), .op(op), .bank(bank), .row(row), .wr_data(wr_data), .wr_cw(wr_cw),
                 .rd_cw(rd_cw), .rd_data(rd_data), .rd_corrected(rd_corrected), .rd_double_err(rd_double_err),
                 .swap(swap), .read_bank(read_bank), .fill_bank(fill_bank));
  initial begin
    #1 clk = 1; #1 clk = 0; rst = 0;
    fd = $fopen("vec.hex", "r");
    while (!$feof(fd)) begin
      r = $fscanf(fd, "%h %h %h %h\n", d, c, pd, pc);
      if (r == 4) begin
        wr_data = d; rd_cw = c;
        #1 clk = 1;
        #1 wr_data = pd; rd_cw = pc;
        #1 $display("R %018x %016x %b %b", wr_cw, rd_data, rd_corrected, rd_double_err);
        #1 clk = 0;
      end
    end
    $finish;
  end
endmodule
"#;

#[test]
fn the_top_registers_each_codec_at_the_die_boundary() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut r = Stream(0xB0DA_7E57);
    let mut v: Vec<(u64, u128)> = Vec::new();
    for &w in &WORDS {
        v.push((w, 0));
        for bit in (0..CW_BITS).step_by(7) {
            v.push((w, 1u128 << bit));
        }
        v.push((w, out_of_range_fault(CW_BITS + (w as usize % (128 - CW_BITS)))));
    }
    for k in 0..300u32 {
        let w = r.next();
        v.push((w, if k % 4 == 0 { 0 } else { r.fault(k % 4) }));
    }
    // The pair each written word and codeword is followed by, between its edge and the next: the complements of the
    // written word and of the read data, clean, so neither output can be right unless it holds what the edge took.
    let poison = |&(w, m): &(u64, u128)| (!w, encode(!decode(encode(w) ^ m).0));
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("infinichip.v"), emit_infinichip_verilog()).unwrap();
    let hex: String = v
        .iter()
        .map(|p| {
            let (w, m) = *p;
            let (pw, pc) = poison(p);
            format!("{w:016x} {:018x} {pw:016x} {pc:018x}\n", encode(w) ^ m)
        })
        .collect();
    std::fs::write(dir.path().join("vec.hex"), hex).unwrap();
    std::fs::write(dir.path().join("tb.v"), TOP_TB).unwrap();
    let got = rows(&iverilog(dir.path(), &["tb.v", "infinichip.v"]));
    assert_eq!(got.len(), v.len(), "one row per edge");
    for (k, (&(w, m), &(cw, data, c, d))) in v.iter().zip(&got).enumerate() {
        assert_eq!(cw, encode(w), "edge {k}: wr_cw is the codeword of the word the edge took, {w:#018x}");
        let (want, verdict) = decode(encode(w) ^ m);
        assert_eq!(
            (data, (c, d)),
            (want, flags(verdict)),
            "edge {k}: the read verdict is the oracle's for the codeword the edge took ({w:#018x} with fault {m:#020x})"
        );
    }
}

fn shipped_testbench_passes(dir_files: &[&str], tb: &str, pass: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    for f in dir_files {
        std::fs::copy(silicon().join(f), dir.path().join(f)).unwrap_or_else(|e| panic!("benchmarks/silicon/{f}: {e}"));
    }
    let sources: Vec<&str> = dir_files.iter().copied().filter(|f| f.ends_with(".v")).collect();
    let out = iverilog(dir.path(), &sources);
    assert!(
        out.contains(pass) && !out.contains("FAIL"),
        "{tb} under iverilog:\n{out}"
    );
}

#[test]
fn the_shipped_infinichip_passes_its_testbench() {
    if !common::require_tool("iverilog") {
        return;
    }
    shipped_testbench_passes(
        &["infinichip.v", "tb_infinichip.v"],
        "tb_infinichip",
        "PASS: infinichip integration",
    );
}

#[test]
fn the_shipped_secded_codec_passes_its_testbench() {
    if !common::require_tool("iverilog") {
        return;
    }
    shipped_testbench_passes(&["secded.v", "tb_secded.v", "secded_vec.hex"], "tb_secded", "PASS: ");
}
