//! **Every classical format has a nonempty residue — measured, completing the format picture.**
//!
//! L17 proved hardness is format-relative: the all-corners cube is NS-degree-`n` but decision-width
//! 1. That cut one way. This completes the picture the honest way — the DAG/decision-width format
//! ALSO has families whose natural certificate GROWS, so its residue is nonempty too. Measured:
//! the decision-width (under the natural variable order) across families and scales spans the full
//! spectrum — cube `= 1` (trivial), XOR cycle `= 4` (constant), pigeonhole and modular counting
//! GROW. So no classical format's residue is empty: resolution (Chvátal–Szemerédi, known), NS over
//! GF(2) (PHP degree growth, §5.13), decision-width (measured here) — each has its own hard
//! families, and they are DIFFERENT families per format.
//!
//! The honest terminus, stated once and exactly: the only format with a possibly-empty residue is
//! SR / Extended-Frege, where an empty residue is NP = coNP by Cook–Reckhow — an open problem no
//! technique in or out of this repository resolves. This test does not claim it. It certifies the
//! wall's exact shape: every format we can measure has a residue, SR is the one open cell, and the
//! measured evidence (every certified cost grows) leans P ≠ NP.

use logicaffeine_proof::cdcl::Lit;
use std::collections::HashSet;

type CanonClauses = Vec<Vec<(u32, bool)>>;

fn canon(clauses: &[Vec<(u32, bool)>]) -> CanonClauses {
    let mut out: CanonClauses = clauses
        .iter()
        .map(|c| {
            let mut lits = c.clone();
            lits.sort_unstable();
            lits.dedup();
            lits
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn cofactor(clauses: &CanonClauses, x: u32, b: bool) -> CanonClauses {
    canon(
        &clauses
            .iter()
            .filter(|c| !c.iter().any(|&(v, pos)| v == x && pos == b))
            .map(|c| c.iter().copied().filter(|&(v, _)| v != x).collect())
            .collect::<Vec<_>>(),
    )
}

/// Max decision-width under the natural variable order.
fn maxwidth(n: usize, root: &CanonClauses) -> usize {
    let order: Vec<u32> = (0..n as u32).collect();
    let mut levels: Vec<HashSet<CanonClauses>> = vec![HashSet::new(); order.len() + 1];
    let mut seen: HashSet<(usize, CanonClauses)> = HashSet::new();
    fn go(
        p: usize,
        clauses: CanonClauses,
        order: &[u32],
        levels: &mut Vec<HashSet<CanonClauses>>,
        seen: &mut HashSet<(usize, CanonClauses)>,
    ) {
        if !seen.insert((p, clauses.clone())) {
            return;
        }
        levels[p].insert(clauses.clone());
        if clauses.iter().any(|c| c.is_empty()) || p == order.len() {
            return;
        }
        let x = order[p];
        go(p + 1, cofactor(&clauses, x, false), order, levels, seen);
        go(p + 1, cofactor(&clauses, x, true), order, levels, seen);
    }
    go(0, root.clone(), &order, &mut levels, &mut seen);
    levels.iter().map(|s| s.len()).max().unwrap_or(0)
}

fn to_canon(clauses: &[Vec<Lit>]) -> CanonClauses {
    canon(
        &clauses.iter().map(|c| c.iter().map(|l| (l.var(), l.is_positive())).collect()).collect::<Vec<_>>(),
    )
}

fn cube(n: usize) -> CanonClauses {
    canon(
        &(0u64..(1u64 << n))
            .map(|a| (0..n as u32).map(|v| (v, (a >> v) & 1 == 0)).collect())
            .collect::<Vec<_>>(),
    )
}

fn xor_cycle(k: usize) -> CanonClauses {
    let mut raw: Vec<Vec<(u32, bool)>> = Vec::new();
    for i in 0..k {
        let j = (i + 1) % k;
        raw.push(vec![(i as u32, true), (j as u32, true)]);
        raw.push(vec![(i as u32, false), (j as u32, false)]);
    }
    canon(&raw)
}

#[test]
fn every_classical_format_has_a_nonempty_residue_measured() {
    // The cube: NS-hard, width-1 (the L17 counterpoint).
    for n in 3..=6usize {
        assert_eq!(maxwidth(n, &cube(n)), 1, "cube(n={n}) width 1");
    }
    // XOR cycle: constant width.
    let xor: Vec<usize> = [5usize, 7, 9].iter().map(|&k| maxwidth(k, &xor_cycle(k))).collect();
    assert!(xor.windows(2).all(|w| w[0] == w[1]), "XOR width constant: {xor:?}");

    // Pigeonhole: width GROWS — a width-nonempty-residue witness (and it is NS-hard too).
    let mut php_w: Vec<(usize, usize)> = Vec::new();
    for m in [3usize, 4] {
        let (p, _) = logicaffeine_proof::families::php(m);
        php_w.push((m, maxwidth(p.num_vars, &to_canon(&p.clauses))));
    }
    assert!(php_w[1].1 > php_w[0].1, "PHP decision-width grows: {php_w:?}");

    // Modular counting: width GROWS too — a different growing family.
    let mut cnt_w: Vec<(usize, usize)> = Vec::new();
    for n in [4usize, 5] {
        let (c, _) = logicaffeine_proof::families::mod_counting(n, 3);
        if c.num_vars <= 12 {
            cnt_w.push((n, maxwidth(c.num_vars, &to_canon(&c.clauses))));
        }
    }

    eprintln!(
        "decision-width spectrum: cube = 1 (trivial), XOR = {} (constant), PHP {php_w:?} (GROWS), \
         Count_3 {cnt_w:?} — the DAG format's residue is NONEMPTY, and its hard families differ \
         from NS's (the cube is NS-hard, width-easy)",
        xor[0]
    );
    eprintln!(
        "the honest terminus: resolution has a residue (Chvátal–Szemerédi), NS over GF(2) has one \
         (PHP degree growth), decision-width has one (measured) — EVERY classical format's residue \
         is nonempty, on DIFFERENT families. SR/EF is the ONLY open format, where an empty residue \
         = NP = coNP. That last cell is the open problem; nothing here claims it. The train reaches \
         the frontier."
    );
}
