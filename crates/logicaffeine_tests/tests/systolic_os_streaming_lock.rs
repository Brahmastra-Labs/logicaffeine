//! systolic_os_streaming_lock — the output-stationary GEMM array streams tile after tile.
//!
//! `emit_systolic_os_streaming_verilog` adds to the output-stationary array a `first` flag that rides
//! the activation wavefront: when a tile's first contraction beat reaches PE(i,j), the PE moves its
//! finished sum into its result register and starts the new tile's sum from that beat's product, so
//! tiles follow each other with no reset and no idle beat. A result register holds its sum a whole
//! period, so row `i` of a tile is readable, all `c` columns at once, `os_streaming_result_offset(period,
//! i, c)` cycles after the tile's first beat; only the columns of an array wider than its contraction wait
//! (`os_streaming_deskew`), and a square array waits for none. These tests stream tiles through the emitted
//! RTL under iverilog and compare every row with the integer GEMM, back to back and with idle beats between
//! tiles, and count the de-skew registers.

mod common;

use logicaffeine_compile::codegen_sva::systolic::{
    emit_systolic_os_streaming_verilog, eval_systolic_os, os_streaming_deskew, os_streaming_result_offset, SystolicOs,
};
use std::process::Command;

const BITS: u32 = 8;
const ACC: u32 = 32;

fn wrap(v: i64, bits: u32) -> i64 {
    let s = 64 - bits;
    (v << s) >> s
}

/// Deterministic int8 tiles: `(A r×k, W k×c)` row-major.
fn tiles(os: &SystolicOs, n: usize, seed: u64) -> Vec<(Vec<i64>, Vec<i64>)> {
    let mut x = seed;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        ((x & 0xff) as u8 as i8) as i64
    };
    (0..n)
        .map(|_| {
            let a = (0..os.r * os.k).map(|_| next()).collect();
            let w = (0..os.k * os.c).map(|_| next()).collect();
            (a, w)
        })
        .collect()
}

/// A testbench streaming `ts` (then one flush tile) with `period` beats per tile, printing every
/// result row at its offset as `ROW <tile> <i> <c values>`.
fn testbench(name: &str, os: &SystolicOs, ts: &[(Vec<i64>, Vec<i64>)], period: usize) -> String {
    let (r, c, k) = (os.r, os.c, os.k);
    let mut tb = format!("`timescale 1ns/1ps\nmodule tb;\n  reg clk = 0, rst = 1, first = 0;\n");
    for i in 0..r {
        tb.push_str(&format!("  reg signed [63:0] aw{i} = 0;\n"));
    }
    for j in 0..c {
        tb.push_str(&format!("  reg signed [63:0] wn{j} = 0;\n"));
    }
    for i in 0..r {
        for j in 0..c {
            tb.push_str(&format!("  wire signed [63:0] res{i}_{j};\n"));
        }
    }
    let mut conns = vec![".clk(clk)".to_string(), ".rst(rst)".into(), ".first(first)".into()];
    conns.extend((0..r).map(|i| format!(".aw{i}(aw{i})")));
    conns.extend((0..c).map(|j| format!(".wn{j}(wn{j})")));
    for i in 0..r {
        for j in 0..c {
            conns.push(format!(".res{i}_{j}(res{i}_{j})"));
        }
    }
    tb.push_str(&format!("  {name} dut({});\n  always #5 clk = ~clk;\n  integer cyc = 0;\n", conns.join(", ")));
    // The drive schedule: tile n beat t at cycle n*period + t, counted from the first cycle out of reset.
    tb.push_str("  always @(posedge clk) if (!rst) cyc <= cyc + 1;\n  always @(*) begin\n    first = 0;\n");
    for i in 0..r {
        tb.push_str(&format!("    aw{i} = 0;\n"));
    }
    for j in 0..c {
        tb.push_str(&format!("    wn{j} = 0;\n"));
    }
    tb.push_str("    if (!rst) case (cyc)\n");
    let all: Vec<(Vec<i64>, Vec<i64>)> = ts
        .iter()
        .cloned()
        .chain(std::iter::once((vec![0; r * k], vec![0; k * c])))
        .collect();
    for (n, (a, w)) in all.iter().enumerate() {
        for t in 0..k {
            let mut body = if t == 0 { "first = 1; ".to_string() } else { String::new() };
            for i in 0..r {
                body.push_str(&format!("aw{i} = {}; ", a[i * k + t]));
            }
            for j in 0..c {
                body.push_str(&format!("wn{j} = {}; ", w[t * c + j]));
            }
            tb.push_str(&format!("      {}: begin {body}end\n", n * period + t));
        }
    }
    tb.push_str("      default: ;\n    endcase\n  end\n");
    // Print each row of each real tile at its offset.
    tb.push_str("  always @(posedge clk) if (!rst) begin\n");
    for n in 0..ts.len() {
        for i in 0..r {
            let at = n * period + os_streaming_result_offset(period, i, c);
            let vals: Vec<String> = (0..c).map(|j| format!("res{i}_{j}")).collect();
            tb.push_str(&format!(
                "    if (cyc == {at}) $display(\"ROW {n} {i} {}\", {});\n",
                vec!["%0d"; c].join(" "),
                vals.join(", ")
            ));
        }
    }
    let end = ts.len() * period + os_streaming_result_offset(period, r - 1, c) + 2;
    tb.push_str(&format!("    if (cyc == {end}) $finish;\n  end\n  initial begin repeat (3) @(posedge clk); rst <= 0; end\nendmodule\n"));
    tb
}

fn run(os: &SystolicOs, ts: &[(Vec<i64>, Vec<i64>)], period: usize) -> Vec<(usize, usize, Vec<i64>)> {
    let dir = tempfile::Builder::new().prefix("os_streaming").tempdir().unwrap();
    let rtl = dir.path().join("os.v");
    std::fs::write(&rtl, emit_systolic_os_streaming_verilog("os_stream", os, BITS, ACC)).unwrap();
    let tb = dir.path().join("tb.v");
    std::fs::write(&tb, testbench("os_stream", os, ts, period)).unwrap();
    let exe = dir.path().join("sim");
    let b = Command::new("iverilog").args(["-g2012", "-o"]).arg(&exe).arg(&rtl).arg(&tb).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let out = Command::new("vvp").arg("-n").arg(&exe).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("ROW "))
        .map(|l| {
            let w: Vec<i64> = l.split_whitespace().map(|x| x.parse().unwrap()).collect();
            (w[0] as usize, w[1] as usize, w[2..].to_vec())
        })
        .collect()
}

fn check(os: &SystolicOs, n: usize, period: usize, seed: u64) {
    let ts = tiles(os, n, seed);
    let rows = run(os, &ts, period);
    assert_eq!(rows.len(), n * os.r, "every row of every tile was read");
    for (t, i, vals) in rows {
        let (a, w) = &ts[t];
        let want = eval_systolic_os(os, a, w, BITS);
        for j in 0..os.c {
            assert_eq!(vals[j], wrap(want[i * os.c + j], ACC), "tile {t} row {i} col {j} (period {period})");
        }
    }
}

#[test]
fn tiles_stream_back_to_back_with_no_reset_between_them() {
    if !common::require_tool("iverilog") {
        return;
    }
    let os = SystolicOs::new(8, 8, 8);
    check(&os, 6, os.k, 0x5EED_0001);
}

#[test]
fn idle_beats_between_tiles_leave_every_result_intact() {
    if !common::require_tool("iverilog") {
        return;
    }
    let os = SystolicOs::new(8, 8, 8);
    for gap in [1, 3, 9] {
        check(&os, 4, os.k + gap, 0x5EED_0100 + gap as u64);
    }
}

#[test]
fn a_rectangular_array_streams_too() {
    if !common::require_tool("iverilog") {
        return;
    }
    check(&SystolicOs::new(3, 5, 4), 5, 4, 0x5EED_0200);
    check(&SystolicOs::new(4, 2, 6), 5, 7, 0x5EED_0300);
    check(&SystolicOs::new(2, 7, 3), 5, 3, 0x5EED_0400);
}

#[test]
fn the_streaming_array_has_the_first_flag_and_a_result_port_per_pe() {
    let os = SystolicOs::new(2, 3, 4);
    let v = emit_systolic_os_streaming_verilog("os_probe", &os, BITS, ACC);
    assert!(v.contains("input first"));
    for i in 0..2 {
        for j in 0..3 {
            assert!(v.contains(&format!("output signed [63:0] res{i}_{j}")), "res{i}_{j}");
        }
    }
    assert_eq!(os_streaming_result_offset(4, 0, 3), 4 + 3);
    assert_eq!(os_streaming_result_offset(8, 7, 8), 8 + 7 + 8);
}

/// Only an array wider than its contraction de-skews, and only its first `c - k` columns, one register a
/// cycle a column waits: the 8×8×8 tile has none, the 3×5×4 array's column 0 one per row.
#[test]
fn only_the_columns_a_short_tile_would_miss_are_delayed() {
    for (os, want) in [
        (SystolicOs::new(8, 8, 8), 0usize),
        (SystolicOs::new(3, 5, 4), 3),
        (SystolicOs::new(4, 2, 6), 0),
        (SystolicOs::new(2, 7, 3), 2 * (4 + 3 + 2 + 1)),
    ] {
        let v = emit_systolic_os_streaming_verilog("os_count", &os, BITS, ACC);
        let regs = v.lines().filter(|l| l.trim_start().starts_with("reg") && l.contains(" dres_")).count();
        assert_eq!(regs, want, "{}×{}×{}", os.r, os.c, os.k);
        let per_row: usize = (0..os.c).map(|j| os_streaming_deskew(&os, j)).sum();
        assert_eq!(regs, os.r * per_row, "{}×{}×{}", os.r, os.c, os.k);
    }
}
