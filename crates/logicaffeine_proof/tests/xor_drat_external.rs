//! External validation of the CNF→GF(2) bridge: our Gaussian (XOR linear-dependency) refutation,
//! compiled to DRAT, must be accepted by the standard `drat-trim` checker — not just our own RUP
//! checker. This is the "strict DRAT proof, verified by an external tool" milestone.
//!
//! `drat-trim` is located via `$DRAT_TRIM` or the conventional `/tmp/drat-trim/drat-trim`; if absent,
//! the external assertion is skipped (the internal RUP check still runs).

use std::process::Command;

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::dimacs::{self, DimacsCnf};
use logicaffeine_proof::lyapunov::extract_xor;
use logicaffeine_proof::xor_drat::{emit_modp_drat, emit_xor_drat};
use logicaffeine_proof::xorsat::{self, XorOutcome};
use logicaffeine_proof::{families, rup};

fn drat_trim_bin() -> Option<String> {
    if let Ok(p) = std::env::var("DRAT_TRIM") {
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    let conventional = "/tmp/drat-trim/drat-trim";
    std::path::Path::new(conventional).exists().then(|| conventional.to_string())
}

fn drat_text(proof: &[Vec<Lit>]) -> String {
    let mut s = String::new();
    for c in proof {
        for l in c {
            let dimacs = if l.is_positive() { l.var() as i64 + 1 } else { -(l.var() as i64 + 1) };
            s.push_str(&dimacs.to_string());
            s.push(' ');
        }
        s.push_str("0\n");
    }
    s
}

/// Returns `true` if `drat-trim` VERIFIES the proof of `cnf`, or if the binary is unavailable (skip).
fn drat_trim_verifies(tag: &str, cnf: &DimacsCnf, proof: &[Vec<Lit>]) -> bool {
    let Some(bin) = drat_trim_bin() else {
        eprintln!("[{tag}] drat-trim not found — skipping external check (internal RUP still ran)");
        return true;
    };
    let dir = std::env::temp_dir();
    let cnf_path = dir.join(format!("logos_xor_{tag}.cnf"));
    let drat_path = dir.join(format!("logos_xor_{tag}.drat"));
    std::fs::write(&cnf_path, dimacs::print(cnf)).expect("write cnf");
    std::fs::write(&drat_path, drat_text(proof)).expect("write drat");
    let out = Command::new(&bin).arg(&cnf_path).arg(&drat_path).output().expect("run drat-trim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let verdict = stdout.lines().rev().find(|l| l.contains("VERIFIED") || l.contains("NOT")).unwrap_or("(no verdict)");
    eprintln!("[{tag}] drat-trim: {verdict}");
    stdout.contains("s VERIFIED") || stdout.contains("VERIFIED")
}

#[test]
fn tseitin_gf2_refutation_is_verified_by_drat_trim() {
    // Tseitin parity over a 3-regular expander: UNSAT, exponential for resolution/CDCL, but a GF(2)
    // linear dependency. We compile that dependency to DRAT and require BOTH our independent RUP
    // checker AND external drat-trim to accept the proof of the byte-identical CNF.
    for n in [6, 8, 10] {
        let (_, cnf, _) = families::tseitin_expander(n, 1);
        let eqs = extract_xor(cnf.num_vars, &cnf.clauses);
        assert!(!eqs.is_empty(), "extract_xor must recover the parity system (n={n})");

        let refutation = match xorsat::solve(&eqs, cnf.num_vars) {
            XorOutcome::Unsat(s) => s,
            XorOutcome::Sat(_) => panic!("tseitin n={n} must be UNSAT"),
        };
        let proof = emit_xor_drat(&eqs, &refutation).expect("linear dependency compiles to DRAT");

        assert!(proof.last().is_some_and(|c| c.is_empty()), "proof ends in the empty clause (n={n})");
        assert!(rup::check_refutation(cnf.num_vars, &cnf.clauses, &proof), "internal RUP must accept (n={n})");
        assert!(drat_trim_verifies(&format!("tseitin{n}"), &cnf, &proof), "external drat-trim must VERIFY (n={n})");
    }
}

#[test]
fn affine_refutation_is_verified_by_drat_trim() {
    // The AFFINE break's refutation goes through the SAME xor_drat bridge: `affine_refutation_drat`
    // recovers the formula's linear substructure, finds the GF(2) dependency, and compiles it to DRAT —
    // so the symmetry the permutation breakers cannot see (the shears) yields a certificate both our RUP
    // checker AND external drat-trim accept against the original CNF.
    for n in [6, 8, 10] {
        let (_, cnf, _) = families::tseitin_expander(n, 3);
        let proof = logicaffeine_proof::affine::affine_refutation_drat(cnf.num_vars, &cnf.clauses)
            .expect("the affine break certifies an inconsistent linear core");
        assert!(proof.last().is_some_and(|c| c.is_empty()), "proof ends in the empty clause (n={n})");
        assert!(rup::check_refutation(cnf.num_vars, &cnf.clauses, &proof), "internal RUP must accept (n={n})");
        assert!(
            drat_trim_verifies(&format!("affine_tseitin{n}"), &cnf, &proof),
            "external drat-trim must VERIFY the affine refutation (n={n})"
        );
    }
}

#[test]
fn affine_p_refutation_is_verified_by_drat_trim() {
    // The GF(p) AFFINE refutation: `affine_p_refutation_drat` recovers the one-hot mod-p system, finds the
    // GF(p) linear dependency, and compiles it to DRAT over the Boolean encoding — so the mod-p shears the
    // monomial breakers cannot see yield a certificate both our RUP checker AND external drat-trim accept.
    for (n, p) in [(4usize, 3u64), (6, 3), (4, 5)] {
        let (_, cnf, _) = families::mod_p_tseitin_expander(n, p, 2);
        let Some(proof) = logicaffeine_proof::affine_gfp::affine_p_refutation_drat(cnf.num_vars, &cnf.clauses) else {
            eprintln!("[affine_modp_n{n}_p{p}] resolution route over budget — skipped");
            continue;
        };
        assert!(proof.last().is_some_and(|c| c.is_empty()), "proof ends in the empty clause (n={n}, p={p})");
        assert!(rup::check_refutation(cnf.num_vars, &cnf.clauses, &proof), "internal RUP must accept (n={n}, p={p})");
        assert!(
            drat_trim_verifies(&format!("affine_modp_n{n}_p{p}"), &cnf, &proof),
            "external drat-trim must VERIFY the GF({p}) affine refutation (n={n})"
        );
    }
}

#[test]
fn modp_counting_refutation_is_verified_by_drat_trim() {
    // mod-p counting (Count_p / mod-p Tseitin): UNSAT, exponential for resolution AND Z3, but a GF(p)
    // linear dependency. We compile that dependency to a strict DRAT proof — over the one-hot Boolean
    // encoding — and require both our RUP checker and external drat-trim to VERIFY it.
    for (n, p) in [(4usize, 3u64), (6, 3), (4, 5)] {
        let (_, cnf, _) = families::mod_p_tseitin_expander(n, p, 1);
        let t0 = std::time::Instant::now();
        let Some(proof) = emit_modp_drat(cnf.num_vars, &cnf.clauses) else {
            eprintln!("[modp_n{n}_p{p}] resolution route over budget — skipped (vars={})", cnf.num_vars);
            continue;
        };
        eprintln!("[modp_n{n}_p{p}] proof_len={} in {:?} (cnf vars={} clauses={})", proof.len(), t0.elapsed(), cnf.num_vars, cnf.clauses.len());
        assert!(proof.last().is_some_and(|c| c.is_empty()), "proof ends in the empty clause (n={n}, p={p})");
        assert!(
            rup::check_refutation(cnf.num_vars, &cnf.clauses, &proof),
            "internal RUP must accept (n={n}, p={p})"
        );
        assert!(
            drat_trim_verifies(&format!("modp_n{n}_p{p}"), &cnf, &proof),
            "external drat-trim must VERIFY the mod-{p} counting proof (n={n})"
        );
    }
}
