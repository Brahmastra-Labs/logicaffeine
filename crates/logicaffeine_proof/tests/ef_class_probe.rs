//! **The EF-class experimental probe** — automated proof search in the PR/SR fragment (the
//! Extended-Frege class), pointed at families where the short-proof question is open or where
//! resolution provably fails.
//!
//! The instrument: `sdcl::solve_certified` — satisfaction-driven clause learning, discovering
//! propagation-redundant clauses by positive reduct with no symmetry hints — over
//! (a) the **mutilated chessboard** (Heule–Kiesl–Biere exhibited short *hand-built* PR proofs; can
//! the automatic search find its own?), and (b) **random 3-CNF above the satisfiability threshold**
//! (the family conjectured hard for every proof system — the honest boundary of NP vs coNP).
//! Every UNSAT verdict is re-checked by the independent PR checker against the original formula
//! alone; the pure-RUP paths are exported to DRAT and pushed through the external `drat-trim` where
//! the binary is available. **These are measurements, not theorems**: discovered-clause counts and
//! proof sizes are the data; a negative result (SDCL discovering nothing and falling back to
//! resolution-shaped search) is as reportable as a positive one.

use std::process::Command;
use std::time::Instant;

use logicaffeine_proof::dimacs::{self, DimacsCnf};
use logicaffeine_proof::families;
use logicaffeine_proof::pr::check_pr_refutation;
use logicaffeine_proof::proof_emit::emit_drat;
use logicaffeine_proof::sdcl::{solve_certified, CertifiedOutcome};

fn drat_trim_bin() -> Option<String> {
    if let Ok(p) = std::env::var("DRAT_TRIM") {
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    let conventional = "/tmp/drat-trim/drat-trim";
    std::path::Path::new(conventional).exists().then(|| conventional.to_string())
}

/// `true` iff drat-trim VERIFIES, or the binary is unavailable (skip with a note).
fn externally_verified(tag: &str, cnf: &DimacsCnf, drat: &str) -> bool {
    let Some(bin) = drat_trim_bin() else {
        eprintln!("[{tag}] drat-trim not found — external check skipped (internal checks ran)");
        return true;
    };
    let dir = std::env::temp_dir();
    let cnf_path = dir.join(format!("ef_probe_{tag}.cnf"));
    let drat_path = dir.join(format!("ef_probe_{tag}.drat"));
    std::fs::write(&cnf_path, dimacs::print(cnf)).expect("write cnf");
    std::fs::write(&drat_path, drat).expect("write drat");
    let out = Command::new(&bin).arg(&cnf_path).arg(&drat_path).output().expect("run drat-trim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!(
        "[{tag}] drat-trim: {}",
        stdout.lines().rev().find(|l| l.contains("VERIFIED") || l.contains("NOT")).unwrap_or("(none)")
    );
    stdout.contains("VERIFIED") && !stdout.contains("NOT VERIFIED")
}

/// **Does the automatic EF-class search find its own short chessboard proof?** The mutilated
/// chessboard has short PR proofs (Heule–Kiesl–Biere) — built by hand, from the two-coloring
/// argument. Here SDCL gets only the opaque clauses. Measured per scale: the verdict, the number of
/// PR clauses *discovered*, total proof steps, wall time — and soundness is asserted (the composed
/// proof re-checks against the original formula with zero trust in the search).
#[test]
#[ignore = "scale measurement — SDCL positive-reduct probing is minutes at board ≥ 6"]
fn sdcl_discovers_sr_refutations_of_the_mutilated_chessboard_with_measured_scaling() {
    for n in [4usize, 6] {
        let (cnf, verdict) = families::mutilated_chessboard(n);
        assert!(matches!(verdict, families::ExpectedVerdict::Unsat));
        let t = Instant::now();
        match solve_certified(cnf.num_vars, &cnf.clauses) {
            CertifiedOutcome::Unsat { steps, discovered } => {
                assert!(
                    check_pr_refutation(cnf.num_vars, &cnf.clauses, &steps),
                    "chessboard({n}): the composed proof re-checks against the original formula"
                );
                eprintln!(
                    "PROBE | chessboard({n}): {} vars, {} clauses → UNSAT; discovered PR = {discovered}, \
                     total steps = {}, {:?}",
                    cnf.num_vars,
                    cnf.clauses.len(),
                    steps.len(),
                    t.elapsed()
                );
            }
            CertifiedOutcome::Sat(_) => panic!("chessboard({n}) is UNSAT"),
        }
    }
}

/// **Random 3-CNF above the threshold: the honest boundary.** Ratio 5.0 (safely UNSAT-side at these
/// sizes), several seeds per size. Measured: verdict, discovered-PR count, proof steps — and every
/// pure-RUP refutation is exported to DRAT and externally verified by `drat-trim` where available.
/// If proof sizes scale badly here, that is the expected face of conjectured hardness; if SDCL ever
/// finds substantial PR shortcuts on random instances, that is data the field does not have.
#[test]
#[ignore = "scale measurement — run explicitly or via the fast suite"]
fn random_threshold_cnf_sr_size_scaling_is_measured_with_external_verification() {
    for vars in [16usize, 20, 24] {
        for seed in [1u64, 2, 3] {
            let clauses_n = (vars as f64 * 5.0) as usize;
            let cnf = families::random_3sat(vars, clauses_n, 0xD1CE_0000 + seed * 7919 + vars as u64);
            let t = Instant::now();
            match solve_certified(cnf.num_vars, &cnf.clauses) {
                CertifiedOutcome::Unsat { steps, discovered } => {
                    assert!(
                        check_pr_refutation(cnf.num_vars, &cnf.clauses, &steps),
                        "random({vars},{seed}): the proof re-checks"
                    );
                    let external = match emit_drat(cnf.num_vars, &cnf.clauses, &steps) {
                        Ok(drat) => {
                            let tag = format!("rand_{vars}_{seed}");
                            if externally_verified(&tag, &cnf, &drat) { "drat-trim" } else { "FAILED" }
                        }
                        Err(_) => "sr-only", // PR steps with real witnesses do not fit plain DRAT
                    };
                    assert_ne!(external, "FAILED", "random({vars},{seed}): external check must not fail");
                    eprintln!(
                        "PROBE | random({vars}, seed {seed}): {} clauses → UNSAT; discovered PR = \
                         {discovered}, steps = {}, external = {external}, {:?}",
                        clauses_n,
                        steps.len(),
                        t.elapsed()
                    );
                }
                CertifiedOutcome::Sat(model) => {
                    assert!(
                        cnf.clauses.iter().all(|c| c
                            .iter()
                            .any(|l| model[l.var() as usize] == l.is_positive())),
                        "random({vars},{seed}): the SAT model re-checks"
                    );
                    eprintln!("PROBE | random({vars}, seed {seed}): SAT (below-threshold draw)");
                }
            }
        }
    }
}
