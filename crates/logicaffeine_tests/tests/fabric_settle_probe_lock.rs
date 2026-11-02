#![cfg(feature = "handed-reds")]
//! fabric_settle_probe_lock — every combinational board top measures how many clocks its datapath takes.
//!
//! FABRIC-SETTLE-PROBE's RED tests (benchmarks/FUSIONS_WE_NEED.md, section 12.20). The fabric top measures its own
//! settle (FABRIC-REAL-CYCLE); the stencil tops, the streaming tile and the place-and-route probes still stream
//! `localparam CYCLES = 16'd1`. With the settle probe they report, for a datapath whose outputs arrive 0, 15, 35 or 95
//! ns after its inputs on a 10 ns clock, 1, 2, 4 and 10 clocks and the same results; past the window, 0.

#[path = "common/mod.rs"]
mod common;
#[path = "common/board_census.rs"]
mod board_census;
#[path = "common/board_settle.rs"]
mod board_settle;

use std::path::{Path, PathBuf};

fn silicon() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(silicon().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn no_board_top_streams_a_constant() {
    let mut offenders = Vec::new();
    for (file, text) in board_census::board_tops() {
        for (expr, why) in board_census::constant_streams(&text) {
            offenders.push(format!("{file}: `tx_byte <= {expr}` ({why})"));
        }
    }
    assert!(offenders.is_empty(), "board tops still stream a constant:\n{}", offenders.join("\n"));
}

#[test]
fn every_combinational_top_measures_its_settle() {
    if !common::require_tool("iverilog") {
        return;
    }
    let tops = [
        ("nexys_conv_soc.v", "nexys_conv_soc", "fabric_gaussblur3.v", "fabric_gaussblur3", 8),
        ("nexys_sobel_soc.v", "nexys_sobel_soc", "fabric_sobel3.v", "fabric_sobel3", 8),
    ];
    for (top_file, top, dp_file, dp, records) in tops {
        let run = |settle_ns: u32| {
            let dir = std::env::temp_dir().join(format!("logos_settle_probe_{top}_{settle_ns}_{}", std::process::id()));
            board_settle::stream(&dir, &read(top_file), top, ".CLKS_PER_BIT(4), .LMAX(16), .REPS(2)", &read(dp_file), dp, settle_ns, records)
                .unwrap_or_else(|e| panic!("{top}, settle {settle_ns} ns: {e}"))
        };
        let settled = run(0);
        for (settle_ns, want) in [(0u32, 1u16), (15, 2), (35, 4), (95, 10)] {
            for (i, (rec, base)) in run(settle_ns).iter().zip(&settled).enumerate() {
                assert_eq!(rec.result, base.result, "{top}, settle {settle_ns} ns, record {i}: the result moved");
                assert_eq!(rec.cycles, want, "{top}, settle {settle_ns} ns, record {i}: {rec:?}");
            }
        }
        for (i, rec) in run(200).iter().enumerate() {
            assert_eq!(rec.cycles, 0, "{top}, settle 200 ns is past the window, record {i}: {rec:?}");
        }
    }
}
