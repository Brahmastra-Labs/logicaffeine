//! computedram_honesty_lock — Phase 3/6 pit-of-success: the ComputeDRAM artifacts stay HONEST.
//!
//! The characterization JSON must keep `measured: null` (no on-board measurement fabricated) and the
//! energy JSON must keep `measured: false` (energy is projected, not measured on silicon). Flipping
//! either to a claimed measurement without a real board run, or fabricating success rates, trips these
//! locks. The `predicted` results are pinned to the RowStore/VM oracle so they can't drift silently.

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

fn silicon(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/silicon")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("missing {}: {e}", p.display()))
}

/// Characterization is GATED on hardware and never fabricated; predictions equal the oracle.
#[test]
fn computedram_characterization_json_is_honest_and_gated() {
    let j = silicon("computedram_characterization.json");
    assert!(
        j.contains("\"measured\": null"),
        "measured must be null until the board runs it"
    );
    assert!(
        j.contains("\"provenance\": \"pending_hardware\""),
        "provenance = pending_hardware"
    );
    assert!(j.contains("MT47H64M16"), "names the real on-board part");
    assert!(
        j.to_lowercase().contains("ddr3"),
        "honesty note flags the DDR2-vs-DDR3 unknown"
    );
    // `predicted` results are the RowStore/VM oracle: rowclone copies, AND = a&b, OR = a|b.
    let (a, b) = (0xDEAD_BEEF_CAFE_F00Du64, 0x0123_4567_89AB_CDEFu64);
    assert!(
        j.contains(&format!("{:016x}", a & b)),
        "predicted AND == oracle a&b"
    );
    assert!(
        j.contains(&format!("{:016x}", a | b)),
        "predicted OR == oracle a|b"
    );
}

/// Energy is PROJECTED (never measured on silicon); the bytes-not-moved win is exact.
#[test]
fn computedram_energy_json_is_honest_and_projected() {
    let j = silicon("computedram_energy.json");
    assert!(
        j.contains("\"measured\": false"),
        "no on-silicon energy is measured"
    );
    assert!(
        j.contains("projected"),
        "energy figures are projected + flagged"
    );
    assert!(
        j.contains("\"bytes_not_moved\""),
        "the exact structural win is present"
    );
    assert!(
        j.contains("pj_per_bit_bus"),
        "projection assumptions stated for audit"
    );
    // A two-operand row-AND moves 3 row-widths (2 reads + 1 write) on the CPU path — exact: 3*2048.
    assert!(
        j.contains("6144"),
        "bytes-not-moved for a row-AND is exact (3 * 2048)"
    );
}
