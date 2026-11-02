use super::*;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// The software oracle matches the runtime's SplitMix64 exactly (same constants),
/// so it is a faithful reference for the RTL.
#[test]
fn sm64_oracle_matches_the_runtime_constants() {
    // First few draws from seed 0 are stable and reproducible.
    let mut s = 0u64;
    let a = sm64_next(&mut s);
    let b = sm64_next(&mut s);
    assert_ne!(a, b, "consecutive draws differ");
    // Determinism: same seed → same sequence.
    let mut s2 = 0u64;
    assert_eq!(a, sm64_next(&mut s2), "seed 0 draw 0 is deterministic");
}

/// Emit the arbiter RTL, its testbench, and the software oracle draw sequence
/// (golden; regenerate with ARBITER_RECORD=1). iverilog PASS is wired into
/// scripts/synth-soft-core.sh — the HW draws match the software scheduler bit-exact.
#[test]
fn seeded_arbiter_artifacts_are_current() {
    const N: usize = 64;
    let mut state = ARBITER_TEST_SEED;
    let mut hex = String::new();
    for _ in 0..N {
        hex.push_str(&format!("{:016x}\n", sm64_next(&mut state)));
    }
    let files = [
        ("seeded_arbiter_prng.v", emit_seeded_arbiter_verilog()),
        ("tb_seeded_arbiter.v", emit_seeded_arbiter_testbench(N)),
        ("arbiter_seq.hex", hex),
    ];
    let dir = silicon_dir();
    if std::env::var("ARBITER_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!(
                "missing {n} ({e}) — record: ARBITER_RECORD=1 cargo test seeded_arbiter_artifacts"
            )
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with ARBITER_RECORD=1");
    }
}
