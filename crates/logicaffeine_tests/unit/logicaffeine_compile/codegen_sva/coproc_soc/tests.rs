use super::*;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Emit the wrapper RTL, its testbench, and the board constraints (golden; regenerate with
/// `COPROC_SOC_RECORD=1`). iverilog PASS is wired into scripts/synth-soft-core.sh.
#[test]
fn coproc_soc_artifacts_are_current() {
    let files = [
        ("coproc_soc.v", emit_coproc_soc_verilog()),
        ("tb_coproc_soc.v", emit_coproc_soc_testbench()),
        ("ulx3s_coproc.lpf", emit_ulx3s_coproc_lpf()),
    ];
    let dir = silicon_dir();
    if std::env::var("COPROC_SOC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!(
                "missing {n} ({e}) — record: COPROC_SOC_RECORD=1 cargo test coproc_soc_artifacts"
            )
        });
        assert_eq!(
            &got, b,
            "{n} is stale — regenerate with COPROC_SOC_RECORD=1"
        );
    }
}

/// The serial frame the wrapper streams reconstructs, byte-for-byte, the model's retired
/// result registers: sync 0xA5, then R2=BADD, R3=BSUB, R4=BMUL, R5=CMERGE, MSB byte first.
/// This ties the readout order (`expected_stream`) to `run_core` independent of the RTL, so
/// the host verifier and the testbench share one definition of the wire format.
#[test]
fn the_serial_frame_reconstructs_the_retired_registers() {
    let regs = run_core(&COPROC_A, &COPROC_B, COPROC_PROGRAM);
    let frame = expected_stream(&regs);
    assert_eq!(frame.len(), 1 + 128, "sync + 4x32 payload bytes");
    assert_eq!(frame[0], 0xA5, "sync byte leads the frame");
    // Rebuild each register from its 32 MSB-first bytes and compare to run_core.
    for (g, &r) in RESULT_REGS.iter().enumerate() {
        let mut v = [0u64; 4];
        for pos in 0..32usize {
            let byte = frame[1 + g * 32 + pos];
            let bit_hi = 255 - pos * 8;
            let limb = bit_hi / 64;
            let within = bit_hi % 64;
            v[limb] |= (byte as u64) << (within - 7);
        }
        assert_eq!(
            v, regs[r as usize],
            "R{r} reconstructed from its streamed bytes"
        );
    }
}
