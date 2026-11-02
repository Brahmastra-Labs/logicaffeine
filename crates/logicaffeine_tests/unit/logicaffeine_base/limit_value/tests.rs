use super::*;
use crate::numeric::{BigInt, Rational};

fn qr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn a(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

/// The value of Thomson's lamp after `H` toggles from off: `H mod 2`.
fn lamp(ty: &ResidueType) -> Depends<Algebraic> {
    Depends::from_fn(2, ty, |r| a(r as i64)).unwrap()
}

fn too_fine<T>(classes: u64) -> Result<T, ResidueError> {
    Err(ResidueError::TooFine {
        classes: BigInt::from_u64(classes),
    })
}

// ── residue types of the generator ────────────────────────────────────────────────────────────

#[test]
fn residue_types_record_what_is_known_about_the_generator() {
    let any = ResidueType::any();
    assert!(any.allows(0, 2) && any.allows(1, 2) && any.allows(5, 7));
    let even = ResidueType::congruent(0, 2);
    assert!(even.allows(0, 2) && !even.allows(1, 2));
    assert!(even.allows(2, 4) && !even.allows(3, 4), "even rules out odd residues mod 4");
    assert!(even.allows(1, 3), "evenness says nothing modulo 3");
    let six = ResidueType::congruent(0, 6);
    assert!(six.allows(0, 2) && six.allows(0, 3) && !six.allows(1, 3));
    assert!(six.allows(2, 4), "H = 6 is divisible by 6 and ≡ 2 (mod 4)");
    assert!(!six.allows(3, 4) && !six.allows(1, 2), "a multiple of 6 is even");
    let both = even.and(&ResidueType::congruent(0, 3)).unwrap();
    assert_eq!(both, six, "even and divisible by 3 is divisible by 6");
    assert_eq!(even.and(&ResidueType::congruent(1, 2)), Err(ResidueError::Contradiction), "even and odd contradict");
    assert!(ResidueType::congruent(1, 4).and(&ResidueType::congruent(3, 6)).is_ok());
    assert_eq!(ResidueType::congruent(1, 4).and(&ResidueType::congruent(0, 6)), Err(ResidueError::Contradiction));
}

// ── values that depend on the residue ─────────────────────────────────────────────────────────

#[test]
fn thomsons_lamp_depends_on_the_parity_of_the_generator() {
    let any = ResidueType::any();
    let v = lamp(&any);
    assert_eq!(v.single(), None);
    assert_eq!(v.modulus(), 2);
    assert_eq!(
        v.render("⌈1/dt⌉", |x| x.to_string()),
        "0 when ⌈1/dt⌉ is even, 1 when it is odd"
    );
    let even = ResidueType::congruent(0, 2);
    assert_eq!(lamp(&even).single(), Some(&a(0)));
    assert_eq!(lamp(&even).render("⌈1/dt⌉", |x| x.to_string()), "0");
    let odd = ResidueType::congruent(1, 2);
    assert_eq!(lamp(&odd).single(), Some(&a(1)));
}

#[test]
fn values_collapse_to_the_smallest_modulus_that_still_separates_them() {
    let any = ResidueType::any();
    let v = Depends::from_fn(6, &any, |r| a((r % 2) as i64)).unwrap();
    assert_eq!(v.modulus(), 2, "a value that only reads parity is held modulo 2");
    let c = Depends::from_fn(12, &any, |_| a(7)).unwrap();
    assert_eq!(c.modulus(), 1);
    assert_eq!(c.single(), Some(&a(7)));
    let three = Depends::from_fn(3, &any, |r| a(r as i64)).unwrap();
    assert_eq!(
        three.render("H", |x| x.to_string()),
        "0 when H ≡ 0 (mod 3), 1 when H ≡ 1 (mod 3), 2 when H ≡ 2 (mod 3)"
    );
    let six = ResidueType::congruent(0, 6);
    let w = Depends::from_fn(4, &six, |r| a(r as i64)).unwrap();
    assert_eq!(w.render("H", |x| x.to_string()), "0 when H ≡ 0 (mod 4), 2 when H ≡ 2 (mod 4)",
        "residues the declared type rules out are not shown");
}

#[test]
fn arithmetic_lifts_branch_by_branch() {
    let any = ResidueType::any();
    let parity = Depends::from_fn(2, &any, |r| a(r as i64)).unwrap();
    let third = Depends::from_fn(3, &any, |r| a(r as i64)).unwrap();
    let sum = parity.zip_with(&third, |x, y| x.add(y)).unwrap();
    assert_eq!(sum.modulus(), 6);
    for h in 0..12u64 {
        assert_eq!(sum.at(h), Some(&a(((h % 2) + (h % 3)) as i64)), "H = {h}");
    }
    let doubled = parity.map(|x| x.mul(&a(2)));
    assert_eq!(doubled.at(5), Some(&a(2)));
    let cancel = parity.zip_with(&parity, |x, y| x.sub(y)).unwrap();
    assert_eq!(cancel.single(), Some(&a(0)), "x − x is the same on every branch");
}

// ── limit rules ───────────────────────────────────────────────────────────────────────────────

#[test]
fn every_classical_answer_to_thomsons_lamp() {
    let v = lamp(&ResidueType::any());
    assert_eq!(v.apply(LimitRule::LimitSuperior).unwrap(), a(1), "the infinite-time machine's limsup");
    assert_eq!(v.apply(LimitRule::LimitInferior).unwrap(), a(0));
    assert_eq!(v.apply(LimitRule::CesaroMean).unwrap(), Algebraic::from_rational(qr(1, 2)), "Grandi's ½");
    let err = v.apply(LimitRule::Limit).unwrap_err();
    assert!(err.contains("no limit"), "{err}");
    assert!(err.contains("0 when ⌈1/dt⌉ is even, 1 when it is odd"), "{err}");
    assert!(v.apply(LimitRule::StandardPart).is_err());
    let settled = lamp(&ResidueType::congruent(1, 2));
    assert_eq!(settled.apply(LimitRule::Limit).unwrap(), a(1));
    assert_eq!(settled.apply(LimitRule::CesaroMean).unwrap(), a(1));
}

#[test]
fn the_cesaro_mean_weights_only_the_allowed_residues() {
    let v = Depends::from_fn(4, &ResidueType::any(), |r| a(r as i64)).unwrap();
    assert_eq!(v.apply(LimitRule::CesaroMean).unwrap(), Algebraic::from_rational(qr(3, 2)));
    let even = ResidueType::congruent(0, 2);
    let w = Depends::from_fn(4, &even, |r| a(r as i64)).unwrap();
    assert_eq!(w.apply(LimitRule::CesaroMean).unwrap(), a(1), "residues 0 and 2 only");
    assert_eq!(w.apply(LimitRule::LimitSuperior).unwrap(), a(2));
}

// ── provisional answers ───────────────────────────────────────────────────────────────────────

#[test]
fn provisional_answers_print_their_evidence() {
    let p = Converging::provisional(false, 10_000_000, 0);
    assert_eq!(
        p.render(|b| b.to_string()),
        "false (provisional: checked 10,000,000 cases, 0 revisions; not proved)"
    );
    let r = Converging::provisional(a(3), 1_234, 1);
    assert_eq!(r.render(|x| x.to_string()), "3 (provisional: checked 1,234 cases, 1 revision; not proved)");
    let s = Converging::settled(true, "kernel-checked proof of ∀n");
    assert_eq!(s.render(|b| b.to_string()), "true");
    assert!(s.is_settled() && !p.is_settled());
    assert_eq!(s.certificate(), Some("kernel-checked proof of ∀n"));
    assert_eq!(p.value(), &false);
    assert_eq!(p.steps(), 10_000_000);
}

#[test]
fn a_residue_type_is_built_from_its_allowed_residues_and_lists_them() {
    assert_eq!(ResidueType::from_allowed(2, &[0]), Ok(ResidueType::congruent(0, 2)));
    assert_eq!(ResidueType::from_allowed(4, &[0, 2]), Ok(ResidueType::congruent(0, 2)), "canonical");
    assert_eq!(ResidueType::from_allowed(1, &[0]), Ok(ResidueType::any()));
    assert_eq!(ResidueType::from_allowed(3, &[0, 1, 2]), Ok(ResidueType::any()));
    assert_eq!(ResidueType::from_allowed(6, &[]), Err(ResidueError::Contradiction), "nothing allowed is a contradiction");
    assert_eq!(ResidueType::from_allowed(6, &[7]), Ok(ResidueType::congruent(1, 6)), "residues reduce");
    assert_eq!(ResidueType::congruent(3, 6).residues(), vec![3]);
    assert_eq!(ResidueType::any().residues(), vec![0]);
    assert_eq!(ResidueType::from_allowed(4, &[1, 3]).unwrap().residues(), vec![1], "odd is 1 mod 2");
}

#[test]
fn a_limit_error_names_the_generator_it_was_given() {
    let v = lamp(&ResidueType::any());
    let err = v.apply_with(LimitRule::Limit, "⌈1/h⌉").unwrap_err();
    assert!(err.contains("0 when ⌈1/h⌉ is even, 1 when it is odd"), "{err}");
    assert_eq!(v.apply_with(LimitRule::CesaroMean, "H").unwrap(), Algebraic::from_rational(qr(1, 2)));
}

#[test]
fn residues_at_or_above_the_modulus_reduce() {
    let odd = ResidueType::congruent(1, 2);
    assert!(odd.allows(5, 2) && odd.allows(3, 4) && !odd.allows(4, 2));
    let even = ResidueType::congruent(0, 2);
    assert!(even.allows(4, 2) && even.allows(6, 4) && !even.allows(7, 4));
}

#[test]
fn a_dependence_is_rebuilt_from_its_classes_or_refused() {
    let even = ResidueType::congruent(0, 2);
    let d = Depends::from_fn(4, &even, |r| a(r as i64)).unwrap();
    let raw = d.raw_branches();
    assert_eq!(raw.len() as u64, d.modulus());
    assert_eq!(Depends::from_branches(even.clone(), d.modulus(), raw.clone()), Some(d.clone()));
    let mut forged = raw.clone();
    forged[1] = Some(a(9));
    assert_eq!(Depends::from_branches(even.clone(), d.modulus(), forged), None, "a value for a class the type rules out");
    let mut missing = raw;
    missing[0] = None;
    assert_eq!(Depends::from_branches(even.clone(), d.modulus(), missing), None, "an admitted class with no value");
    let uncollapsed = vec![Some(a(5)), Some(a(5))];
    assert_eq!(Depends::from_branches(ResidueType::any(), 2, uncollapsed).unwrap().modulus(), 1, "collapsed on the way in");
    assert_eq!(Depends::<Algebraic>::from_branches(ResidueType::any(), 0, vec![]), None);
}

// ── the bound on a residue system ─────────────────────────────────────────────────────────────

#[test]
fn the_finest_residue_system_a_value_holds_is_the_wire_bound() {
    assert_eq!(MAX_MODULUS, 1 << 16);
    assert_eq!(MAX_MODULUS as usize, crate::exact_codec::MAX_WIRE_LEN, "every value a program holds can travel");
    assert_eq!(ResidueType::from_allowed(MAX_MODULUS, &[0]).unwrap().modulus(), MAX_MODULUS);
    assert_eq!(Depends::from_fn(MAX_MODULUS, &ResidueType::any(), |r| r).unwrap().modulus(), MAX_MODULUS);
}

#[test]
fn a_residue_system_finer_than_the_bound_is_refused_before_it_is_built() {
    assert_eq!(ResidueType::from_allowed(MAX_MODULUS + 1, &[0]), too_fine(MAX_MODULUS + 1));
    assert_eq!(ResidueType::from_allowed(u64::MAX, &[0]), too_fine(u64::MAX), "refused without allocating");
    assert_eq!(ResidueType::from_allowed(u64::MAX, &[]), too_fine(u64::MAX), "the bound is checked first");
    let p = ResidueType::congruent(0, 65_521);
    let q = ResidueType::congruent(0, 65_519);
    assert_eq!(p.and(&q), too_fine(65_521 * 65_519), "two coprime moduli below the bound meet above it");
    assert_eq!(Depends::from_fn(MAX_MODULUS + 2, &ResidueType::any(), |r| r), too_fine(MAX_MODULUS + 2));
    assert_eq!(Depends::from_fn(65_521, &q, |r| r), too_fine(65_521 * 65_519), "the type's own modulus counts");
    let x = Depends::from_fn(65_521, &ResidueType::any(), |r| r).unwrap();
    let y = Depends::from_fn(65_519, &ResidueType::any(), |r| r).unwrap();
    assert_eq!(x.zip_with(&y, |a, b| a + b), too_fine(65_521 * 65_519));
}

#[test]
fn a_refusal_names_the_generator_and_the_bound() {
    let e = ResidueError::TooFine {
        classes: BigInt::from_u64(1_000_003),
    };
    assert_eq!(
        e.describe("⌈1/dt⌉"),
        "⌈1/dt⌉ would split over 1000003 residue classes; one value holds at most 65,536"
    );
    assert_eq!(
        ResidueError::Contradiction.describe("1/dt"),
        "the facts about 1/dt contradict each other: no natural satisfies them all"
    );
    assert_eq!(e.to_string(), e.describe("the generator"));
}

#[test]
fn values_of_two_incompatible_types_do_not_combine() {
    let even = Depends::from_fn(2, &ResidueType::congruent(0, 2), |r| r).unwrap();
    let odd = Depends::from_fn(2, &ResidueType::congruent(1, 2), |r| r).unwrap();
    assert_eq!(even.zip_with(&odd, |a, b| a + b), Err(ResidueError::Contradiction));
}

#[test]
fn a_question_about_any_modulus_is_answered_by_the_chinese_remainder_theorem() {
    let even = ResidueType::congruent(0, 2);
    assert!(even.allows(4, 1 << 40) && !even.allows(3, 1 << 40));
    let one_mod_three = ResidueType::congruent(1, 3);
    assert!(one_mod_three.allows(5, 1 << 40), "3 and 2^40 are coprime: every class mod 2^40 meets 1 mod 3");
    assert!(one_mod_three.allows(1, u64::MAX) && !one_mod_three.allows(0, u64::MAX), "3 divides 2^64 − 1");
    let six = ResidueType::congruent(0, 6);
    for m in 1..=36u64 {
        for r in 0..m {
            let brute = (0..m * 6).any(|x| x % m == r && six.admits(x));
            assert_eq!(six.allows(r, m), brute, "{r} mod {m}");
        }
    }
}

#[test]
fn the_widest_dependence_decodes_in_linear_time() {
    let ty = ResidueType::from_allowed(MAX_MODULUS, &(0..MAX_MODULUS).step_by(2).collect::<Vec<_>>()).unwrap();
    assert_eq!(ty.modulus(), 2);
    let wide = Depends::from_fn(MAX_MODULUS, &ty, |r| r).unwrap();
    assert_eq!(wide.modulus(), MAX_MODULUS);
    let back = Depends::from_branches(ty.clone(), MAX_MODULUS, wide.raw_branches()).unwrap();
    assert_eq!(back, wide);
}

#[test]
fn a_dependence_whose_classes_and_type_meet_past_the_bound_is_refused() {
    let ty = ResidueType::congruent(0, MAX_MODULUS);
    let raw = vec![Some(1u64); (MAX_MODULUS - 1) as usize];
    assert_eq!(Depends::from_branches(ty.clone(), MAX_MODULUS - 1, raw), None);
    let mut fits = vec![None; MAX_MODULUS as usize / 2];
    fits[0] = Some(1u64);
    let ok = Depends::from_branches(ty, MAX_MODULUS / 2, fits).unwrap();
    assert_eq!(ok.single(), Some(&1));
}
