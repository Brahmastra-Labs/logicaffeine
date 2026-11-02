use super::*;

#[test]
fn solver_data_parses_and_carries_our_proof_size() {
    // Guards the `include_str!` contract the page renders — and, specifically, that a regenerated
    // solvers.json keeps OUR proof size, the answer to "why doesn't Logos show its proof size?".
    let data = &*SOLVER_DATA;
    assert!(!data.families.is_empty(), "solvers.json must have families");

    // The pigeonhole family reports our SR proof size on every row.
    let php = data
        .families
        .iter()
        .find(|f| f.id == "php")
        .expect("php family present");
    assert!(
        php.rows
            .iter()
            .all(|r| r.ours_proof_bytes.is_some() && r.ours_proof_fmt.is_some()),
        "every PHP row must carry our own SR proof size"
    );

    // Every structural family carries a proof, per its own certificate kind: GF/ring linear
    // certificate (parity/mod), the O(1) counting certificate (pigeonhole variants), the Hall
    // witness (chessboard). "Proofs for all", the answer to "shouldn't every family have one?".
    for id in [
        "tseitin",
        "mod_3_tseitin",
        "mod_6_tseitin",
        "php_variants",
        "mutilated_chessboard",
    ] {
        let fam = data
            .families
            .iter()
            .find(|f| f.id == id)
            .unwrap_or_else(|| panic!("{id} present"));
        assert!(
            fam.rows.iter().all(|r| r.ours_proof_bytes.is_some()),
            "the {id} family must carry a proof size on every row"
        );
    }

    // The random control carries a plain-CDCL DRAT wherever it refutes (its UNSAT row); a SAT row
    // has a model, not a refutation, so no proof size there — and that is the only allowed gap.
    let random = data
        .families
        .iter()
        .find(|f| f.id == "random_3sat")
        .expect("random present");
    assert!(
        random
            .rows
            .iter()
            .any(|r| r.ours_detail.contains("unsat") && r.ours_proof_bytes.is_some()),
        "the random control's UNSAT row must carry a DRAT proof size"
    );

    // CaDiCaL joined the field (CryptoMiniSat too when its binary is present at bench time).
    let solvers: std::collections::HashSet<&str> = data
        .families
        .iter()
        .flat_map(|f| f.rows.iter())
        .flat_map(|r| r.others.iter())
        .map(|o| o.solver.as_str())
        .collect();
    assert!(
        solvers.contains("cadical"),
        "CaDiCaL must appear in the field"
    );
}
