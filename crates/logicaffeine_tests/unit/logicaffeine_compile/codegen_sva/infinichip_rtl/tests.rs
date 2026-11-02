use super::super::scratchpad_rtl::{decode, encode, Decode};
use super::*;

/// The top is a STRUCTURAL composition of the four certified sub-blocks: it declares
/// `module infinichip` and instantiates each sub-block exactly once, binding the ports that
/// carry the composed interface. A missing or mis-named instantiation (a mis-wire) fails here.
#[test]
fn the_infinichip_top_instantiates_the_four_certified_subblocks() {
    let v = emit_infinichip_verilog();
    // All four certified sub-module DEFINITIONS are inlined (self-contained source).
    for m in [
        "module dram_executor",
        "module secded_enc",
        "module secded_dec",
        "module double_buffer",
        "module infinichip",
    ] {
        assert!(v.contains(m), "infinichip.v must inline {m}");
    }
    // Each sub-block is INSTANTIATED exactly once inside the top.
    for inst in [
        "dram_executor u_cmd",
        "secded_enc    u_enc",
        "secded_dec    u_dec",
        "double_buffer u_dbuf",
    ] {
        assert_eq!(
            v.matches(inst).count(),
            1,
            "infinichip must instantiate `{inst}` once"
        );
    }
    // The command player's payload pins are threaded out through the top (no dangling wire).
    for port in [
        ".cmd_word(cmd_word)",
        ".issue(issue)",
        ".cw(wr_cw)",
        ".cw_in(rd_cw)",
        ".fill_bank(fill_bank)",
    ] {
        assert!(v.contains(port), "infinichip must bind {port}");
    }
}

/// The composed ECC read path is data-preserving under any single fault: for a sweep of data
/// words and EVERY one of the 72 codeword bit positions, encode → flip that bit → decode
/// recovers the original data and NEVER reports a false double-error. This re-checks, over the
/// certified SECDED oracle the top instantiates, the subsystem's "zero silent corruption"
/// guarantee end-to-end.
#[test]
fn the_infinichip_ecc_path_round_trips_and_corrects_single_faults() {
    let data_words = [
        0x0000_0000_0000_0000u64,
        0xFFFF_FFFF_FFFF_FFFF,
        0xDEAD_BEEF_1234_5678,
        0x0123_4567_89AB_CDEF,
        0xA5A5_A5A5_5A5A_5A5A,
    ];
    for &data in &data_words {
        // Clean round-trip.
        let cw = encode(data);
        let (d0, v0) = decode(cw);
        assert_eq!(d0, data, "clean codeword must decode to the data");
        assert_eq!(v0, Decode::NoError, "clean codeword must report NoError");
        // Every single-bit fault is corrected back to the data, never a false double-error.
        for bit in 0..super::super::scratchpad_rtl::CW_BITS {
            let (d, verdict) = decode(cw ^ (1u128 << bit));
            assert_eq!(
                d, data,
                "single fault at bit {bit} must recover data {data:#018x}"
            );
            assert!(
                matches!(verdict, Decode::Corrected(_) | Decode::CorrectedParityBit),
                "single fault at bit {bit} must be CORRECTED, got {verdict:?}"
            );
            assert_ne!(
                verdict,
                Decode::DoubleError,
                "single fault must not read as double"
            );
        }
    }
}

/// The double-buffer safety invariant is present in the composed RTL by construction:
/// `fill_bank = ~read_bank`, so compute never reads the bank the DMA is filling.
#[test]
fn the_infinichip_double_buffer_is_disjoint_by_construction() {
    let v = emit_infinichip_verilog();
    assert!(
        v.contains("assign fill_bank = ~read_bank;"),
        "the composed subsystem must keep the scratchpad banks disjoint by construction"
    );
}

/// Emit the self-contained `infinichip.v` + `tb_infinichip.v` into benchmarks/silicon/ (golden;
/// regenerate with INFINICHIP_RECORD=1). The iverilog PASS/FAIL is wired into
/// scripts/synth-soft-core.sh; the OpenLane signoff is pinned by sky130_signoff_lock.
#[test]
fn infinichip_artifacts_are_current() {
    let rtl = emit_infinichip_verilog();
    let tb = emit_infinichip_testbench();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let files = [("infinichip.v", rtl), ("tb_infinichip.v", tb)];
    if std::env::var("INFINICHIP_RECORD").is_ok() {
        for (name, body) in &files {
            std::fs::write(dir.join(name), body).expect("write artifact");
        }
        return;
    }
    for (name, body) in &files {
        let path = dir.join(name);
        let got = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "missing {} ({e}) — record: INFINICHIP_RECORD=1 cargo test infinichip_artifacts",
                path.display()
            )
        });
        assert_eq!(
            &got, body,
            "{name} is stale — regenerate with INFINICHIP_RECORD=1"
        );
    }
}
