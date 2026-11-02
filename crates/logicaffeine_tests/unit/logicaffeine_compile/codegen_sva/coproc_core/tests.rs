use super::*;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Emit the core RTL, its testbench, and the oracle register file (golden; regenerate with
/// `COPROC_RECORD=1`). iverilog PASS is wired into scripts/synth-soft-core.sh.
#[test]
fn coproc_core_artifacts_are_current() {
    let files = [
        (
            "coproc_core.v",
            emit_coproc_core_verilog(&COPROC_A, &COPROC_B, COPROC_PROGRAM),
        ),
        ("tb_coproc_core.v", emit_coproc_core_testbench()),
        (
            "coproc_exp.hex",
            coproc_exp_hex(&COPROC_A, &COPROC_B, COPROC_PROGRAM),
        ),
    ];
    let dir = silicon_dir();
    if std::env::var("COPROC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: COPROC_RECORD=1 cargo test coproc_core_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with COPROC_RECORD=1");
    }
}

/// The issue/retire model produces the coprocessors' own results in the addressed
/// registers — R2=A+B, R3=A−B, R4=A.lo·B.lo, R5=merge(A.lo128,B.lo128) — and leaves the
/// operands intact. (The shipped-BigInt / GCounter tie lives in coproc_core_silicon_lock.)
#[test]
fn the_core_routes_operands_and_retires_each_coprocessor_result() {
    let r = run_core(&COPROC_A, &COPROC_B, COPROC_PROGRAM);
    assert_eq!(r[0], COPROC_A, "R0 operand preserved");
    assert_eq!(r[1], COPROC_B, "R1 operand preserved");
    assert_eq!(r[2], limb_add(&COPROC_A, &COPROC_B).0, "BADD retired to R2");
    assert_eq!(r[3], limb_sub(&COPROC_A, &COPROC_B).0, "BSUB retired to R3");
    let p = limb_mul(COPROC_A[0], COPROC_B[0]);
    assert_eq!(
        r[4],
        [p as u64, (p >> 64) as u64, 0, 0],
        "BMUL retired to R4"
    );
    let m = merge_low128(&COPROC_A, &COPROC_B);
    assert_eq!(r[5], [m[0], m[1], 0, 0], "CMERGE retired to R5");
    assert_eq!(r[6], [0; 4], "untouched register stays zero");
}
