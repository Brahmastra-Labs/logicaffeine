use super::*;
use crate::polycalc::ns_lower_bound_witness_polys;

/// **Orbit types align across scales.** The canonical structure names an orbit
/// scale-independently: at each scale the map orbit → type is a bijection (same orbit ⟹ same
/// type by group-invariance of the structure; distinct orbits ⟹ distinct types by completeness
/// of the canonization), and at each fixed degree the type SET stabilizes across scales once
/// every structure is realizable — the cross-`m` alignment the stability theorem stands on.
/// Reproduces the fixed-degree orbit-count stabilization of
/// `php_symmetric_ns_width_is_constant_in_m_at_fixed_degree` through the naming layer, for both
/// pigeonhole (two sorts) and modular counting (one sort).
#[test]
fn php_monomial_orbit_types_align_across_m_by_bipartite_graph_type() {
    for d in [1usize, 2, 3] {
        let mut prev: Option<BTreeSet<Structure>> = None;
        for m in [d + 1, d + 2, d + 3] {
            let inst = php_instance_clause(m);
            let orbits = monomial_orbits_bounded(inst.num_vars, d, &inst.sym);
            let type_set: BTreeSet<Structure> =
                orbits.iter().map(|o| inst.type_of(o[0])).collect();
            // Bijection: one type per orbit, every orbit one type.
            assert_eq!(
                type_set.len(),
                orbits.len(),
                "PHP({m}) d={d}: distinct orbits get distinct canonical types"
            );
            for orbit in &orbits {
                let t = inst.type_of(orbit[0]);
                for &mono in orbit.iter().take(8) {
                    assert_eq!(
                        inst.type_of(mono),
                        t,
                        "PHP({m}) d={d}: the canonical type is constant on the orbit"
                    );
                }
            }
            if let Some(p) = &prev {
                assert_eq!(
                    p, &type_set,
                    "PHP d={d}: the type set is IDENTICAL across scales m={m}−1, {m}"
                );
            }
            prev = Some(type_set);
        }
    }
    // The one-sort family: Count_3 types stabilize across its scale window too.
    for d in [1usize, 2] {
        let mut prev: Option<BTreeSet<Structure>> = None;
        for n in [7usize, 8] {
            let inst = count_instance_linear(n, 3);
            let orbits = monomial_orbits_bounded(inst.num_vars, d, &inst.sym);
            let type_set: BTreeSet<Structure> =
                orbits.iter().map(|o| inst.type_of(o[0])).collect();
            assert_eq!(type_set.len(), orbits.len(), "Count_3({n}) d={d}: orbit↔type bijection");
            if let Some(p) = &prev {
                assert_eq!(p, &type_set, "Count_3 d={d}: type set identical at n=7, 8");
            }
            prev = Some(type_set);
        }
    }
}

/// **The invariance lemma, machine-checked.** For an invariant functional `L` (constant on
/// monomial orbits) and any group element `σ`: `⟨L, σ(p)⟩ = ⟨L, p⟩`. Hence a constraint checked
/// on one representative per joint (multiplier-orbit × generator) is checked on all — the
/// soundness of the collapsed system's representative rows. Verified on pigeonhole with random
/// invariant functionals against every generator polynomial and symmetry generator.
#[test]
fn an_invariant_functional_checked_on_orbit_representatives_is_checked_on_all_generators() {
    let mut seed = 0xA11C_E5EEDu64;
    let mut lcg = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed >> 33
    };
    for m in [3usize, 4] {
        let inst = php_instance_clause(m);
        let d = 3;
        let orbits = monomial_orbits_bounded(inst.num_vars, d, &inst.sym);
        for _trial in 0..8 {
            // A random invariant L: each orbit all-in or all-out.
            let l: BTreeSet<Mono> = orbits
                .iter()
                .filter(|_| lcg() & 1 == 1)
                .flat_map(|o| o.iter().copied())
                .collect();
            let pair = |p: &Poly| p.iter().filter(|t| l.contains(t)).count() % 2;
            for g in &inst.gens {
                for sigma in &inst.sym {
                    let image: Poly =
                        g.iter().map(|&t| apply_perm_to_mono(sigma, t)).collect();
                    assert_eq!(
                        pair(g),
                        pair(&image),
                        "PHP({m}): ⟨L, σ(p)⟩ = ⟨L, p⟩ for invariant L"
                    );
                }
            }
        }
    }
}

/// **The collapsed dual agrees with the direct orbit-collapsed solver, and its lifted witnesses
/// re-check.** Two independent implementations of "does an invariant degree-`d`
/// pseudo-expectation exist": the type-named representative system ([`collapsed_dual_system`])
/// and the everything-explicit direct solver ([`invariant_witness_exists_direct`]). They must
/// agree at every scale and degree — and every positive verdict lifts to a full witness passing
/// `check_ns_lower_bound_polys` with zero trust. This pins representative completeness and type
/// canonization at once. The `Count_3` degree-2 row reproduces today's Lucas-schedule discovery:
/// invariant witness at `n = 7` (`n ≡ 3 mod 4`), NO invariant witness at `n = 8` — machine-decided
/// through both paths.
#[test]
fn collapsed_dual_system_agrees_with_full_symmetric_ns_on_php_small_m() {
    // PHP, clause and linear encodings, over the window.
    for m in [3usize, 4, 5] {
        for (label, inst) in
            [("clause", php_instance_clause(m)), ("linear", php_instance_linear(m))]
        {
            for d in 1..=3usize {
                let sys = collapsed_dual_system(&inst, d);
                let collapsed = sys.solve();
                let direct = invariant_witness_exists_direct(&inst, d);
                assert_eq!(
                    collapsed.is_some(),
                    direct.is_some(),
                    "PHP({m}) {label} d={d}: type-system and direct solver agree"
                );
                if let Some(sol) = collapsed {
                    let witness = sys.lift(&inst, d, sol);
                    assert!(
                        check_ns_lower_bound_polys(inst.num_vars, &inst.gens, d, &witness),
                        "PHP({m}) {label} d={d}: the lifted invariant witness re-checks"
                    );
                }
                if let Some(w) = direct {
                    assert!(
                        check_ns_lower_bound_polys(inst.num_vars, &inst.gens, d, &w),
                        "PHP({m}) {label} d={d}: the direct invariant witness re-checks"
                    );
                }
            }
        }
    }
    // Count_3 at degree 2: the Lucas schedule, decided by the collapsed machinery.
    for (n, invariant_exists) in [(7usize, true), (8, false)] {
        let inst = count_instance_linear(n, 3);
        let sys = collapsed_dual_system(&inst, 2);
        let collapsed = sys.solve();
        let direct = invariant_witness_exists_direct(&inst, 2);
        assert_eq!(collapsed.is_some(), direct.is_some(), "Count_3({n}): paths agree");
        assert_eq!(
            collapsed.is_some(),
            invariant_exists,
            "Count_3({n}) d=2: invariant witness iff n ≡ 3 (mod 4) — the Lucas schedule"
        );
        if let Some(sol) = collapsed {
            let witness = sys.lift(&inst, 2, sol);
            assert!(
                check_ns_lower_bound_polys(inst.num_vars, &inst.gens, 2, &witness),
                "Count_3({n}): the lifted witness re-checks"
            );
        } else {
            // The char-2 gap, live: no invariant witness, yet a general witness exists.
            let general = ns_lower_bound_witness_polys(inst.num_vars, &inst.gens, 2);
            assert!(
                general.is_some(),
                "Count_3({n}): the general witness survives where every invariant one dies"
            );
        }
    }
}

/// **Entry counts are bounded-degree integer polynomials in the scale, with an interpolation
/// certificate.** Every labeled entry of the collapsed dual, observed across a consecutive
/// window, fits a degree-`≤ 2` polynomial in the finite-difference basis — and for PHP the
/// window (5 points, 3 fitted) leaves two exact verification points per entry, which is the
/// certificate that the fit is the truth and not an artifact. For `Count_3` the machinery
/// locates the hand-derived quadratic: some entry's difference table is exactly `[1, 2, 1]` —
/// the `C(n−4, 2)` count behind the mod-4 witness schedule, machine-found.
#[test]
fn collapsed_entry_counts_are_integer_polynomials_in_m_with_an_interpolation_certificate() {
    // PHP linear encoding, degree 2, window m = 4..8 — every entry certified with 2 spare points.
    let window: Vec<usize> = (4..=8).collect();
    let mut per_scale = Vec::new();
    let mut labels: BTreeSet<RowLabel> = BTreeSet::new();
    let mut cols: BTreeSet<Structure> = BTreeSet::new();
    for &m in &window {
        let counts = labeled_dual_counts(&php_instance_linear(m), 2);
        labels.extend(counts.keys().cloned());
        for c in counts.values() {
            cols.extend(c.keys().cloned());
        }
        per_scale.push(counts);
    }
    let mut fitted = 0usize;
    for l in &labels {
        for c in &cols {
            let values: Vec<i128> = per_scale
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    *s.get(l)
                        .unwrap_or_else(|| panic!("label unrealized at m={}", window[i]))
                        .get(c)
                        .unwrap_or(&0) as i128
                })
                .collect();
            let f = FittedCount::fit(window[0], &values, 2)
                .expect("every PHP entry is a degree-≤2 polynomial, verified on 2 spare points");
            for (i, &m) in window.iter().enumerate() {
                assert_eq!(f.eval(m), values[i], "the fit reproduces every window point");
            }
            fitted += 1;
        }
    }
    assert!(fitted > 20, "a non-trivial system was fitted ({fitted} entries)");

    // Count_3, degree 2, window n = 6..8: the machinery finds the hand-derived C(n−4,2) entry.
    let cwindow: Vec<usize> = (6..=8).collect();
    let mut cscale = Vec::new();
    for &n in &cwindow {
        cscale.push(labeled_dual_counts(&count_instance_linear(n, 3), 2));
    }
    let clabels: BTreeSet<RowLabel> = cscale.iter().flat_map(|s| s.keys().cloned()).collect();
    let ccols: BTreeSet<Structure> = cscale
        .iter()
        .flat_map(|s| s.values().flat_map(|c| c.keys().cloned()))
        .collect();
    let mut found_quadratic = false;
    for l in &clabels {
        for c in &ccols {
            let values: Vec<i128> = cscale
                .iter()
                .map(|s| s.get(l).map(|m| *m.get(c).unwrap_or(&0)).unwrap_or(0) as i128)
                .collect();
            let f = FittedCount::fit(cwindow[0], &values, 2)
                .expect("every Count_3 entry fits at degree ≤ 2");
            if f.diffs == vec![1, 2, 1] {
                found_quadratic = true; // C(n−4, 2) at n = 6, 7, 8 is 1, 3, 6 — diffs [1, 2, 1]
                assert_eq!(f.parity_period(), 4, "the quadratic entry has Lucas period 4");
            }
        }
    }
    assert!(
        found_quadratic,
        "the machinery locates the hand-derived C(n−4,2) quadratic behind the mod-4 schedule"
    );
}

/// **Parity of binomial-basis polynomials is periodic, exactly as Lucas says.** The bit-trick
/// `C(a, k) odd ⟺ k & a == k` is pinned against exact binomials; fitted counts round-trip
/// through evaluation; and every [`FittedCount`]'s parity repeats with its declared period —
/// swept far past the fitting window. This is the finite arithmetic fact that turns a window of
/// evaluations into a verdict for every scale.
#[test]
fn parity_of_binomial_entries_is_eventually_periodic_by_lucas() {
    // Lucas bit-trick vs exact binomials.
    fn binom_exact(a: u64, k: u64) -> u128 {
        if k > a {
            return 0;
        }
        let mut c: u128 = 1;
        for i in 0..k {
            c = c * (a - i) as u128 / (i + 1) as u128;
        }
        c
    }
    for a in 0u64..=40 {
        for k in 0u64..=12 {
            assert_eq!(
                binom_exact(a, k) % 2 == 1,
                k & a == k,
                "Lucas: C({a},{k}) parity by the digit condition"
            );
        }
    }
    // Fit round-trips and declared periods hold, far past the window.
    let mut seed = 0xB1_A5EDu64;
    let mut lcg = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as i128
    };
    for _ in 0..50 {
        let base = 4 + (lcg() % 5) as usize;
        let coeffs: Vec<i128> = (0..=3).map(|_| lcg() % 7 - 3).collect();
        let poly = |m: usize| -> i128 {
            let a = (m - base) as u64;
            coeffs
                .iter()
                .enumerate()
                .map(|(k, &c)| c * binom_exact(a, k as u64) as i128)
                .sum()
        };
        let values: Vec<i128> = (base..base + 6).map(&poly).collect();
        let f = FittedCount::fit(base, &values, 3).expect("a degree-3 polynomial fits");
        let period = f.parity_period();
        assert!(period.is_power_of_two() && period <= 4, "period is a small power of two");
        for m in base..base + 96 {
            assert_eq!(f.eval(m), poly(m), "evaluation reproduces the polynomial everywhere");
            assert_eq!(
                f.parity(m),
                f.parity(m + period),
                "parity repeats with the declared period (m={m})"
            );
        }
    }
}

/// **THE CROWN: fixed-degree invariant-witness verdicts decided for EVERY scale by one finite
/// computation.** For each family and degree the machinery fits the collapsed dual across a
/// window, certifies the entry polynomials, and solves once per residue class — validated at
/// every window scale against the direct solver, with every positive lifted to a re-checked
/// full-basis witness. A `true` residue is a machine-decided `NS-degree > d` lower bound for
/// every scale in the class — including scales whose explicit monomial basis (`C(n,3)` variables
/// at `n ≥ 10`) exceeds anything the per-scale engines can represent. Verdicts locked from the
/// machinery's own first run; the `Count_3` ones agree with (and extend beyond) the
/// independently-derived mod-4 schedule.
#[test]
fn fixed_degree_symmetric_ns_verdict_for_php_is_decided_for_all_m_by_finite_computation() {
    // Pigeonhole, linear encoding, degree 2 — window m = 4..8, entries certified with cushion.
    let php = decide_invariant_witness_for_all_scales(
        &|m| php_instance_linear(m),
        &(4..=8).collect::<Vec<_>>(),
        2,
        2,
    );
    eprintln!(
        "PHP-linear d=2: onset={} period={} by_residue={:?} validated={:?}",
        php.system.onset, php.system.period, php.by_residue, php.validated
    );
    // LOCKED (machine-decided ∀m ≥ 4): the linear-encoded pigeonhole dual has NO invariant
    // witness at degree 2, at ANY scale — while general witnesses exist (the gap test) — so for
    // this family the char-2 gap is the RULE, not the exception: every degree-2 witness of
    // linear PHP is necessarily asymmetric, at every m.
    assert_eq!(php.system.period, 2, "PHP-linear entries are degree-≤1 in m — Lucas period 2");
    assert_eq!(
        php.by_residue,
        vec![false, false],
        "PHP-linear d=2: the invariant dual is EMPTY at every scale m ≥ 4"
    );

    // Pigeonhole, clause encoding, degree 2 — the contrast: past m = d + 1 the wide pigeon
    // clauses leave the degree budget and the hole-injective indicator survives invariantly.
    let php_clause = decide_invariant_witness_for_all_scales(
        &|m| php_instance_clause(m),
        &(4..=8).collect::<Vec<_>>(),
        2,
        2,
    );
    eprintln!(
        "PHP-clause d=2: onset={} period={} by_residue={:?} validated={:?}",
        php_clause.system.onset,
        php_clause.system.period,
        php_clause.by_residue,
        php_clause.validated
    );
    assert!(
        php_clause.by_residue.iter().all(|&v| v),
        "PHP-clause d=2: the invariant witness exists at EVERY scale m ≥ 4 — the encoding \
             chooses whether symmetry can see the bound"
    );

    // Modular counting, linear encoding, degree 2 — window n = 6..8 (the post-onset scales the
    // u64 basis can represent; the entry certificate rests on the located closed forms and the
    // per-scale differential validation).
    let count = decide_invariant_witness_for_all_scales(
        &|n| count_instance_linear(n, 3),
        &(6..=8).collect::<Vec<_>>(),
        2,
        2,
    );
    eprintln!(
        "Count_3 d=2: onset={} period={} by_residue={:?} validated={:?}",
        count.system.onset, count.system.period, count.by_residue, count.validated
    );
    // The window validations already hard-assert agreement + witness re-checks inside decide().
    assert_eq!(count.system.period, 4, "Count_3's schedule is mod 4 — the located quadratic");
    // LOCKED (machine-decided ∀n ≥ 6): invariant witnesses exist EXACTLY on the class
    // n ≡ 3 (mod 4) — so NS-degree(Count_3(n)) ≥ 3 for every n ≡ 3 (mod 4), including scales
    // (n = 11: 165 variables; n = 15: 455) far beyond any representable monomial basis.
    assert_eq!(
        count.by_residue,
        vec![false, false, false, true],
        "Count_3 d=2: the invariant witness lives exactly on n ≡ 3 (mod 4)"
    );
}

/// **Refining the gap: the asymmetric witnesses are NOT one-marking-symmetric — the
/// symmetry-breaking depth is ≥ 2.** The first rung between "fully invariant" and "arbitrary":
/// restrict the symmetry to a point/hole/pigeon stabilizer (one marked object, its own sort)
/// and re-run the collapsed dual. Predicted by hand before measurement and CONFIRMED at all
/// seven cells: one marking extends nothing — `Count_3` stays exactly on its `n ≡ 3 (mod 4)`
/// schedule, and all four marked PHP-linear cells stay empty. The parity mechanics say why:
/// marking one object shifts the counting-polynomial arguments by one, and the dead constraint
/// rows are dead by an *evenness* a single mark cannot split (e.g. the marked-point row
/// `1 + C(n−2, 2)-type` counts stay even off schedule; PHP's cross rows keep their even
/// hole-sums). So the char-2 gap witnesses carry **symmetry-breaking depth ≥ 2** — the witness-
/// level analog of the census's composite-shear depth measure. Every marked search space
/// contains the fully-invariant one, so on-schedule scales stay positive (a fortiori); both
/// engine paths agree and every positive lifts to a re-checked witness, at every cell.
#[test]
fn the_off_schedule_witnesses_are_probed_for_stabilizer_invariance() {
    // Count_3 at degree 2: one marked point does NOT extend the mod-4 schedule (locked).
    for n in [6usize, 7, 8] {
        let marked = count_instance_linear_marked(n, 3);
        let sys = collapsed_dual_system(&marked, 2);
        let collapsed = sys.solve();
        let direct = invariant_witness_exists_direct(&marked, 2);
        assert_eq!(collapsed.is_some(), direct.is_some(), "Count_3({n}) marked: paths agree");
        assert_eq!(
            collapsed.is_some(),
            n % 4 == 3,
            "Count_3({n}): one marked point does not extend the mod-4 schedule (locked)"
        );
        if let Some(sol) = collapsed {
            let w = sys.lift(&marked, 2, sol);
            assert!(
                check_ns_lower_bound_polys(marked.num_vars, &marked.gens, 2, &w),
                "Count_3({n}) marked: the lifted witness re-checks"
            );
        }
        eprintln!(
            "MARKED | Count_3({n}) d=2 (n mod 4 = {}): Stab(point)-invariant witness = {}",
            n % 4,
            collapsed.is_some()
        );
    }
    // PHP-linear at degree 2: fully-invariant is empty at EVERY scale; is one marked hole or
    // one marked pigeon enough to see the bound?
    for m in [4usize, 5] {
        for (which, inst) in [
            ("hole", php_instance_linear_marked_hole(m)),
            ("pigeon", php_instance_linear_marked_pigeon(m)),
        ] {
            let sys = collapsed_dual_system(&inst, 2);
            let collapsed = sys.solve();
            let direct = invariant_witness_exists_direct(&inst, 2);
            assert_eq!(
                collapsed.is_some(),
                direct.is_some(),
                "PHP-linear({m}) marked-{which}: paths agree"
            );
            assert!(
                collapsed.is_none(),
                "PHP-linear({m}) marked-{which}: one marking sees nothing — the witnesses are \
                     ≥ 2-deep (locked)"
            );
            eprintln!(
                "MARKED | PHP-linear({m}) d=2 marked-{which}: stabilizer-invariant witness = {}",
                collapsed.is_some()
            );
        }
    }
}

/// **The symmetric primal–dual gap over `GF(2)`, measured.** Forced by the char-2 Reynolds
/// annihilation, "no invariant witness" need not mean "no witness". The map of
/// `(invariant, general)` witness existence across families, scales and degrees: soundness
/// demands `invariant ⟹ general` everywhere (no `(true, false)` cell), and the gap is real —
/// `Count_3(8)` at degree 2 is `(false, true)`, an asymmetric witness surviving where every
/// invariant candidate dies.
#[test]
fn the_symmetric_primal_dual_gap_over_gf2_is_measured() {
    let mut gap_cells = Vec::new();
    let mut cases: Vec<(String, SymmetricInstance, usize)> = Vec::new();
    for m in [3usize, 4, 5] {
        for d in 1..=3usize {
            cases.push((format!("PHP-linear({m}) d={d}"), php_instance_linear(m), d));
        }
    }
    for n in [7usize, 8] {
        cases.push((format!("Count_3({n}) d=2"), count_instance_linear(n, 3), 2));
    }
    for (label, inst, d) in &cases {
        let invariant = invariant_witness_exists_direct(inst, *d).is_some();
        let general = ns_lower_bound_witness_polys(inst.num_vars, &inst.gens, *d).is_some();
        assert!(
            !invariant || general,
            "{label}: an invariant witness IS a witness — (true, false) is impossible"
        );
        if !invariant && general {
            gap_cells.push(label.clone());
        }
        eprintln!("{label}: invariant={invariant} general={general}");
    }
    assert!(
        gap_cells.contains(&"Count_3(8) d=2".to_string()),
        "the char-2 gap is real: Count_3(8) at degree 2 has only asymmetric witnesses"
    );
    // The systematic face of the gap: linear-encoded pigeonhole's degree-2 witnesses are
    // asymmetric at every measured scale where they exist at all.
    for cell in ["PHP-linear(4) d=2", "PHP-linear(5) d=2", "PHP-linear(5) d=3"] {
        assert!(
            gap_cells.contains(&cell.to_string()),
            "{cell}: a gap cell — the witness exists and is necessarily asymmetric"
        );
    }
}
