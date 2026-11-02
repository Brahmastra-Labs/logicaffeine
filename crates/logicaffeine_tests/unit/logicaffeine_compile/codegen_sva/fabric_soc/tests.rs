use super::*;

fn silicon_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("benchmarks")
        .join("silicon")
}

/// Emit the reconfigurable-fabric RTL, the flashable SoC, and its self-checking testbench (golden;
/// regenerate with `FABRIC_SOC_RECORD=1 cargo test fabric_soc_artifacts`). The drift lock: a change to
/// the config table or the wrapper must be re-recorded, so the checked-in .v never silently rots.
#[test]
fn fabric_soc_artifacts_are_current() {
    let files = [
        ("config_fabric.v", emit_config_grid()),
        ("fabric_soc.v", emit_fabric_soc()),
        ("tb_fabric_soc.v", emit_fabric_soc_tb()),
        ("nexys_fabric_soc.v", emit_nexys_fabric_soc()),
        ("nexys_fabric_soc.xdc", emit_nexys_fabric_xdc()),
        // Stage X2c — the flashable STATE-PLANE SoC (sequenced LUTRAM-heap datapath + UART readback).
        ("state_grade.v", emit_state_soc_module()),
        ("state_soc.v", emit_state_soc()),
        ("tb_state_soc.v", emit_state_soc_tb()),
        ("nexys_state_soc.v", emit_nexys_state_soc()),
        ("nexys_state_soc.xdc", emit_nexys_state_xdc()),
        // Stage X3b — the flashable FLOAT-LANE SoC (fpu.v-bound sequenced float datapath + UART readback).
        ("float_fhypf.v", emit_float_soc_module()),
        ("float_soc.v", emit_float_soc()),
        ("tb_float_soc.v", emit_float_soc_tb()),
        ("nexys_float_soc.v", emit_nexys_float_soc()),
        ("nexys_float_soc.xdc", emit_nexys_float_xdc()),
    ];
    let dir = silicon_dir();
    if std::env::var("FABRIC_SOC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write artifact");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record with FABRIC_SOC_RECORD=1 cargo test fabric_soc_artifacts")
        });
        assert_eq!(
            &got, b,
            "{n} is stale — regenerate with FABRIC_SOC_RECORD=1"
        );
    }
}
