use super::*;
use crate::aig::Aig;

fn named(g: &mut Aig, name: &str) -> AigLit {
    g.add_input(Some(name.to_string()))
}

fn word(g: &mut Aig, name: &str, width: usize) -> Vec<AigLit> {
    (0..width).map(|i| named(g, &format!("{name}[{i}]"))).collect()
}

fn output_word(g: &mut Aig, name: &str, bits: &[AigLit]) {
    for (i, &b) in bits.iter().enumerate() {
        g.add_output(b, Some(format!("{name}[{i}]")));
    }
}

/// A ripple-carry adder: every carry the majority of the bits and the carry below.
fn ripple(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
    let mut carry = AigLit::FALSE;
    let mut sum = Vec::new();
    for i in 0..a.len() {
        let p = g.xor(a[i], b[i]);
        sum.push(g.xor(p, carry));
        let gen = g.and(a[i], b[i]);
        let prop = g.and(p, carry);
        carry = g.or(gen, prop);
    }
    sum
}

/// A Kogge–Stone adder: carries from a parallel prefix over (generate, propagate) pairs.
fn kogge_stone(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
    let n = a.len();
    let p: Vec<AigLit> = (0..n).map(|i| g.xor(a[i], b[i])).collect();
    let mut gg: Vec<AigLit> = (0..n).map(|i| g.and(a[i], b[i])).collect();
    let mut pp = p.clone();
    let mut d = 1;
    while d < n {
        let (prev_g, prev_p) = (gg.clone(), pp.clone());
        for i in d..n {
            let t = g.and(prev_p[i], prev_g[i - d]);
            gg[i] = g.or(prev_g[i], t);
            pp[i] = g.and(prev_p[i], prev_p[i - d]);
        }
        d *= 2;
    }
    (0..n).map(|i| if i == 0 { p[0] } else { g.xor(p[i], gg[i - 1]) }).collect()
}

fn adder_pair(width: usize) -> (Aig, Aig) {
    let mut l = Aig::new();
    let (a, b) = (word(&mut l, "a", width), word(&mut l, "b", width));
    let s = ripple(&mut l, &a, &b);
    output_word(&mut l, "s", &s);
    let mut r = Aig::new();
    let (b2, a2) = (word(&mut r, "b", width), word(&mut r, "a", width));
    let s = kogge_stone(&mut r, &a2, &b2);
    output_word(&mut r, "s", &s);
    (l, r)
}

#[test]
fn distributivity_is_proven_with_a_certificate_that_checks() {
    let mut l = Aig::new();
    let (a, b, c) = (named(&mut l, "a"), named(&mut l, "b"), named(&mut l, "c"));
    let bc = l.or(b, c);
    let y = l.and(a, bc);
    l.add_output(y, Some("y".into()));
    let mut r = Aig::new();
    let (c2, a2, b2) = (named(&mut r, "c"), named(&mut r, "a"), named(&mut r, "b"));
    let ab = r.and(a2, b2);
    let ac = r.and(a2, c2);
    let y = r.or(ab, ac);
    r.add_output(y, Some("y".into()));
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert_eq!(report.verdicts, vec![("y".to_string(), Verdict::Equal)]);
    assert!(report.check());
}

#[test]
fn different_functions_are_separated_by_a_witness_that_replays() {
    let mut l = Aig::new();
    let (a, b) = (named(&mut l, "a"), named(&mut l, "b"));
    let y = l.and(a, b);
    l.add_output(y, Some("y".into()));
    let mut r = Aig::new();
    let (a2, b2) = (named(&mut r, "a"), named(&mut r, "b"));
    let y = r.or(a2, b2);
    r.add_output(y, Some("y".into()));
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    let Verdict::Differ { inputs, .. } = &report.verdicts[0].1 else { panic!("{:?}", report.verdicts) };
    let assign = report.named_inputs(inputs);
    let get = |n: &str| assign.iter().find(|(k, _)| k == n).unwrap().1;
    let (va, vb) = (get("a"), get("b"));
    assert_ne!(va && vb, va || vb, "the witness separates the two outputs");
    assert!(report.check(), "a report with a difference still checks its equalities (none)");
}

#[test]
fn a_ripple_adder_equals_a_kogge_stone_adder() {
    for width in [4, 8, 16] {
        let (l, r) = adder_pair(width);
        let report = equivalent(&l, &r, &Options::default()).unwrap();
        assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{width}: {:?}", report.verdicts);
        assert_eq!(report.verdicts.len(), width);
        assert!(report.check(), "{width}");
    }
}

#[test]
fn a_wrong_carry_is_found_at_the_bit_it_breaks() {
    let (l, _) = adder_pair(8);
    let mut r = Aig::new();
    let (a, b) = (word(&mut r, "a", 8), word(&mut r, "b", 8));
    let mut s = ripple(&mut r, &a, &b);
    s[5] = r.xor(a[5], b[5]);
    output_word(&mut r, "s", &s);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    for (name, v) in &report.verdicts {
        match v {
            Verdict::Equal => assert_ne!(name, "s[5]"),
            Verdict::Differ { inputs, .. } => {
                assert_eq!(name, "s[5]");
                let assign = report.named_inputs(inputs);
                let num = |p: &str| {
                    (0..8).fold(0u32, |acc, i| {
                        acc | (assign.iter().find(|(k, _)| *k == format!("{p}[{i}]")).unwrap().1 as u32) << i
                    })
                };
                let (x, y) = (num("a"), num("b"));
                let carry_in = ((x & 0x1f) + (y & 0x1f)) >> 5 & 1;
                assert_eq!(carry_in, 1, "the witness carries into bit 5: a={x} b={y}");
            }
            Verdict::Unknown(why) => panic!("{name}: {why}"),
        }
    }
    assert!(report.check());
}

#[test]
fn a_gate_that_is_constant_merges_with_false() {
    let mut l = Aig::new();
    let (a, b) = (named(&mut l, "a"), named(&mut l, "b"));
    let ab = l.and(a, b);
    let nand = l.or(!a, !b);
    let never = l.and(ab, nand);
    l.add_output(never, Some("y".into()));
    let mut r = Aig::new();
    named(&mut r, "a");
    named(&mut r, "b");
    r.add_output(AigLit::FALSE, Some("y".into()));
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert_eq!(report.verdicts[0].1, Verdict::Equal);
    assert!(report.check());
}

#[test]
fn parity_in_two_orders_is_equal() {
    let mut l = Aig::new();
    let x = word(&mut l, "x", 16);
    let chain = x[1..].iter().fold(x[0], |acc, &v| l.xor(acc, v));
    l.add_output(chain, Some("p".into()));
    let mut r = Aig::new();
    let mut level = word(&mut r, "x", 16);
    while level.len() > 1 {
        level = level.chunks(2).map(|c| r.xor(c[0], c[1])).collect();
    }
    r.add_output(level[0], Some("p".into()));
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert_eq!(report.verdicts[0].1, Verdict::Equal);
    assert!(report.check());
}

#[test]
fn every_lemma_refutes_its_own_query() {
    let (l, r) = adder_pair(8);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(!report.certificate.lemmas.is_empty(), "sweeping merges internal carries");
    let mut earlier = Vec::new();
    for lemma in &report.certificate.lemmas {
        let q = query_with_cut(&report.miter.aig, lemma.a, lemma.b, &lemma.cut, &earlier);
        assert!(lrat::check(q.num_vars, &q.clauses, &lemma.proof));
        assert!(crate::proof_emit::check_lrat(q.num_vars, &q.clauses, &lemma.proof.to_text()));
        earlier.push((lemma.a, lemma.b));
    }
}

#[test]
fn a_query_holds_the_cone_the_earlier_lemmas_touching_it_and_the_difference() {
    let (l, r) = adder_pair(4);
    let m = miter(&l, &r).unwrap();
    let (_, a, b) = m.pairs[3].clone();
    let q = query(&m.aig, a, b, &[]);
    let cone = m.aig.cone(&[a, b]);
    assert_eq!(q.cone, cone);
    assert_eq!(q.num_vars, cone.len());
    assert!(q.extra.is_empty());
    let ands = cone.iter().filter(|&&n| matches!(m.aig.node(n), crate::aig::Node::And(..))).count();
    let constant = cone.contains(&0) as usize;
    assert_eq!(q.clauses.len(), 3 * ands + constant + 2);
    let inside = (AigLit::new(cone[cone.len() - 1], false), AigLit::new(cone[cone.len() - 2], false));
    let outside: Vec<u32> = (0..m.aig.num_nodes() as u32).filter(|n| cone.binary_search(n).is_err()).take(3).collect();
    assert_eq!(outside.len(), 3, "the miter holds other outputs");
    let touching = (AigLit::new(outside[0], false), AigLit::new(cone[1], true));
    let apart = (AigLit::new(outside[1], false), AigLit::new(outside[2], false));
    let q2 = query(&m.aig, a, b, &[inside, touching, apart]);
    assert_eq!(q2.clauses.len(), q.clauses.len() + 4, "the lemma inside and the lemma touching the cone join");
    assert_eq!(q2.extra, [outside[0]], "the touching lemma's outside node is a free variable of its own");
    assert_eq!(q2.num_vars, cone.len() + 1);
    assert_eq!(&q.clauses[q.clauses.len() - 2..], &q2.clauses[q2.clauses.len() - 2..], "the difference closes the formula");
}

#[test]
fn the_checker_refuses_a_tampered_certificate() {
    let (l, r) = adder_pair(8);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    let claims = report.equal_pairs();
    assert!(check(&report.miter.aig, &report.certificate, &claims));
    let mut bad = report.certificate.clone();
    let k = bad.lemmas.iter().position(|l| l.proof.steps.iter().any(|s| s.hints.len() >= 2)).unwrap();
    let step = bad.lemmas[k].proof.steps.iter().position(|s| s.hints.len() >= 2).unwrap();
    let hints = &mut bad.lemmas[k].proof.steps[step].hints;
    let last = hints.len() - 1;
    hints[last] = hints[0];
    assert!(!check(&report.miter.aig, &bad, &claims), "a chain closed by a clause it already satisfied");
    let mut flipped = report.certificate.clone();
    flipped.lemmas[0].b = !flipped.lemmas[0].b;
    assert!(!check(&report.miter.aig, &flipped, &claims), "a lemma claiming the opposite phase");
    let (_, a, b) = report.miter.pairs[0].clone();
    let (_, c, _) = report.miter.pairs[1].clone();
    assert!(check(&report.miter.aig, &report.certificate, &[(a, b)]));
    assert!(!check(&report.miter.aig, &report.certificate, &[(a, c)]), "a claim no lemma supports");
}

#[test]
fn sweeping_off_still_decides_and_certifies() {
    let (l, r) = adder_pair(8);
    let opts = Options { sweep: false, ..Options::default() };
    let report = equivalent(&l, &r, &opts).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal));
    assert!(report.check());
}

#[test]
fn a_budgeted_query_that_runs_out_says_so() {
    let width = 10;
    let mul = |g: &mut Aig, a: &[AigLit], b: &[AigLit]| -> Vec<AigLit> {
        let mut acc: Vec<AigLit> = vec![AigLit::FALSE; width];
        for i in 0..width {
            let row: Vec<AigLit> =
                (0..width).map(|j| if j < i { AigLit::FALSE } else { g.and(a[j - i], b[i]) }).collect();
            acc = ripple(g, &acc, &row);
        }
        acc
    };
    let mut l = Aig::new();
    let (a, b) = (word(&mut l, "a", width), word(&mut l, "b", width));
    let p = mul(&mut l, &a, &b);
    output_word(&mut l, "p", &p);
    let mut r = Aig::new();
    let (a2, b2) = (word(&mut r, "a", width), word(&mut r, "b", width));
    let p = mul(&mut r, &b2, &a2);
    output_word(&mut r, "p", &p);
    let opts = Options { sweep: false, final_budget: 1, ..Options::default() };
    let report = equivalent(&l, &r, &opts).unwrap();
    assert!(report.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Unknown(_))));
    assert!(report.verdicts.iter().all(|(_, v)| !matches!(v, Verdict::Differ { .. })));
    assert!(report.check(), "the equalities it did prove still check");
}

#[test]
fn a_miter_shares_inputs_by_name_and_pairs_outputs_by_name() {
    let mut l = Aig::new();
    let (a, b) = (named(&mut l, "a"), named(&mut l, "b"));
    l.add_input(None);
    let y = l.and(a, b);
    l.add_output(y, Some("y".into()));
    l.add_output(a, Some("z".into()));
    let mut r = Aig::new();
    let (c, b2) = (named(&mut r, "c"), named(&mut r, "b"));
    r.add_output(b2, Some("z".into()));
    r.add_output(c, Some("y".into()));
    let m = miter(&l, &r).unwrap();
    let names: Vec<_> = m.aig.inputs().iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, vec![Some("a".into()), Some("b".into()), None, Some("c".into())]);
    assert_eq!(m.pairs.iter().map(|p| p.0.clone()).collect::<Vec<_>>(), ["y", "z"]);
}

#[test]
fn a_miter_refuses_what_it_cannot_pair() {
    let mut l = Aig::new();
    let a = named(&mut l, "a");
    l.add_output(a, Some("y".into()));
    let mut r = Aig::new();
    let a2 = named(&mut r, "a");
    r.add_output(a2, Some("w".into()));
    assert_eq!(miter(&l, &r).err(), Some(MiterError::Unmatched("y".into())));
    let mut unnamed = Aig::new();
    let x = unnamed.add_input(None);
    unnamed.add_output(x, None);
    assert_eq!(miter(&unnamed, &unnamed).err(), Some(MiterError::Unnamed));
    let mut seq = Aig::new();
    let k = seq.add_latch(None, crate::aig::Init::Zero);
    let q = seq.latch_lit(k);
    seq.add_output(q, Some("y".into()));
    assert_eq!(miter(&seq, &l).err(), Some(MiterError::Sequential));
    let mut extra = l.clone();
    extra.add_output(a, Some("w".into()));
    assert_eq!(miter(&l, &extra).err(), Some(MiterError::Unmatched("w".into())));
}

#[test]
fn one_search_finds_an_input_that_separates_any_output() {
    let (l, _) = adder_pair(8);
    let mut r = Aig::new();
    let (a, b) = (word(&mut r, "a", 8), word(&mut r, "b", 8));
    let mut s = ripple(&mut r, &a, &b);
    s[7] = !s[7];
    output_word(&mut r, "s", &s);
    let (m, inputs) = separating_input(&l, &r, 0).unwrap().expect("bit 7 is inverted");
    let v = m.aig.eval(&inputs, &[]);
    assert!(m.pairs.iter().any(|(_, x, y)| Aig::value(&v, *x) != Aig::value(&v, *y)));
    let (same, _) = adder_pair(8);
    let (_, ks) = adder_pair(8);
    assert!(separating_input(&same, &ks, 0).unwrap().is_none(), "a ripple and a Kogge-Stone adder never differ");
}

#[test]
fn a_cut_query_leaves_its_frontier_free_and_stops_there() {
    let (l, r) = adder_pair(16);
    let m = miter(&l, &r).unwrap();
    let (_, a, b) = m.pairs[15].clone();
    let whole = query(&m.aig, a, b, &[]);
    let cut = frontier(&m.aig, &[a, b], 6);
    assert!(!cut.is_empty(), "a 16-bit carry chain is deeper than six levels");
    let narrow = query_with_cut(&m.aig, a, b, &cut, &[]);
    assert!(narrow.cone.len() < whole.cone.len());
    for &c in &cut {
        assert!(narrow.cone.binary_search(&c).is_ok(), "a cut node is a leaf of the cone");
    }
    let ands = narrow
        .cone
        .iter()
        .filter(|&&n| matches!(m.aig.node(n), crate::aig::Node::And(..)) && cut.binary_search(&n).is_err())
        .count();
    let constant = narrow.cone.contains(&0) as usize;
    assert_eq!(narrow.clauses.len(), 3 * ands + constant + 2, "no clauses define a cut node");
}

#[test]
fn lemmas_proven_on_a_cut_check_and_their_cut_is_part_of_the_claim() {
    let (l, r) = adder_pair(16);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(report.all_equal() && report.check());
    let k = report.certificate.lemmas.iter().position(|l| !l.cut.is_empty()).expect("a lemma proven a few levels deep");
    let mut widened = report.certificate.clone();
    widened.lemmas[k].cut.clear();
    assert!(!check(&report.miter.aig, &widened, &report.equal_pairs()), "the proof refutes the cut formula, not the whole cone's");
    let mut unsorted = report.certificate.clone();
    if unsorted.lemmas[k].cut.len() >= 2 {
        unsorted.lemmas[k].cut.swap(0, 1);
        assert!(!check(&report.miter.aig, &unsorted, &report.equal_pairs()), "a cut is given in increasing order");
    }
}

/// A circuit over two-bit words `a` and `b` whose two-bit `y` is `f` of them.
fn over_two_bits(f: impl Fn(&mut Aig, &[AigLit], &[AigLit]) -> Vec<AigLit>) -> Aig {
    let mut g = Aig::new();
    let (a, b) = (word(&mut g, "a", 2), word(&mut g, "b", 2));
    let y = f(&mut g, &a, &b);
    output_word(&mut g, "y", &y);
    g
}

/// AND; AND by De Morgan; OR; XOR; AND but 0 when both words are 3; XOR as `(a | b) & !(a & b)`.
fn hypotheses() -> Vec<Aig> {
    vec![
        over_two_bits(|g, a, b| (0..2).map(|i| g.and(a[i], b[i])).collect()),
        over_two_bits(|g, a, b| (0..2).map(|i| !g.or(!a[i], !b[i])).collect()),
        over_two_bits(|g, a, b| (0..2).map(|i| g.or(a[i], b[i])).collect()),
        over_two_bits(|g, a, b| (0..2).map(|i| g.xor(a[i], b[i])).collect()),
        over_two_bits(|g, a, b| {
            let both = g.and(a[0], a[1]);
            let both_b = g.and(b[0], b[1]);
            let threes = g.and(both, both_b);
            (0..2)
                .map(|i| {
                    let x = g.and(a[i], b[i]);
                    g.and(x, !threes)
                })
                .collect()
        }),
        over_two_bits(|g, a, b| {
            (0..2)
                .map(|i| {
                    let o = g.or(a[i], b[i]);
                    let n = g.and(a[i], b[i]);
                    g.and(o, !n)
                })
                .collect()
        }),
    ]
}

/// A member's outputs on an assignment of the distinction's inputs.
fn outputs_on(g: &Aig, d: &Distinction, v: &[bool]) -> Vec<bool> {
    let inputs: Vec<bool> = g.inputs().iter().map(|p| v[d.inputs.iter().position(|n| Some(n) == p.name.as_ref()).unwrap()]).collect();
    let values = g.eval(&inputs, &[]);
    d.outputs.iter().map(|o| Aig::value(&values, g.outputs()[g.output_index(o).unwrap()].lit)).collect()
}

#[test]
fn a_family_falls_into_proven_classes_told_apart_on_a_cover_of_vectors() {
    let members = hypotheses();
    let d = distinguish(&members, &[], 0, &Options::default()).unwrap();
    assert_eq!(d.classes, [vec![0, 1], vec![2], vec![3, 5], vec![4]]);
    assert_eq!(d.inputs, ["a[0]", "a[1]", "b[0]", "b[1]"]);
    assert_eq!(d.outputs, ["y[0]", "y[1]"]);
    let proven: Vec<(usize, usize)> = d.proofs.iter().map(|(rep, m, _)| (*rep, *m)).collect();
    assert_eq!(proven, [(0, 1), (3, 5)]);
    assert!(d.vectors.iter().any(|v| v.iter().all(|&b| b)), "only a = b = 3 tells member 4 from AND: {:?}", d.vectors);
    assert!(!d.cover.is_empty() && d.cover.len() <= d.vectors.len());
    for (i, x) in d.classes.iter().enumerate() {
        for y in &d.classes[i + 1..] {
            assert!(
                d.cover.iter().any(|&v| outputs_on(&members[x[0]], &d, &d.vectors[v]) != outputs_on(&members[y[0]], &d, &d.vectors[v])),
                "classes {x:?} and {y:?} differ on a cover vector"
            );
        }
    }
    for (c, class) in d.classes.iter().enumerate() {
        for (k, &v) in d.cover.iter().enumerate() {
            for &m in class {
                assert_eq!(d.table[c][k], outputs_on(&members[m], &d, &d.vectors[v]), "class {c}, member {m}, cover vector {k}");
            }
        }
    }
    assert!(d.check(&members));
}

#[test]
fn the_cover_picks_first_the_vector_telling_the_most_classes_apart() {
    let members = hypotheses();
    let named = |a: u32, b: u32| -> Vec<(String, bool)> {
        (0..2).map(|i| (format!("a[{i}]"), a >> i & 1 == 1)).chain((0..2).map(|i| (format!("b[{i}]"), b >> i & 1 == 1))).collect()
    };
    // a = 1, b = 3 gives AND 1, OR 3, XOR 2 and the exception's AND 1: three of the four classes apart. a = b = 3
    // tells the exception from AND alone.
    let d = distinguish(&members, &[named(0, 0), named(3, 3), named(1, 3)], 0, &Options::default()).unwrap();
    assert_eq!(d.vectors.len(), 3, "the seeds already tell every two classes apart: no search adds one");
    assert_eq!(d.vectors[2], [true, false, true, true], "a seed over the inputs, a's bits then b's");
    assert_eq!(d.cover, [2, 1], "the most pairs first, then what is left; a = b = 0 tells nothing");
    assert!(d.check(&members));
}

#[test]
fn one_member_is_one_class_and_members_that_cannot_be_paired_are_refused() {
    let members = hypotheses();
    let one = distinguish(&members[..1], &[], 0, &Options::default()).unwrap();
    assert_eq!(one.classes, [vec![0]]);
    assert!(one.vectors.is_empty() && one.cover.is_empty() && one.table == [Vec::<Vec<bool>>::new()]);
    let mut other = Aig::new();
    let a = word(&mut other, "a", 2);
    output_word(&mut other, "z", &a);
    assert!(distinguish(&[members[0].clone(), other], &[], 0, &Options::default()).is_err(), "no output y to pair");
    let mut unnamed = Aig::new();
    let x = unnamed.add_input(None);
    unnamed.add_output(x, Some("y[0]".into()));
    unnamed.add_output(x, Some("y[1]".into()));
    assert!(distinguish(&[members[0].clone(), unnamed], &[], 0, &Options::default()).is_err(), "an input a vector cannot name");
}

#[test]
fn a_distinction_whose_table_or_proof_is_tampered_does_not_check() {
    let members = hypotheses();
    let d = distinguish(&members, &[], 0, &Options::default()).unwrap();
    let mut table = d.clone();
    table.table[0][0][0] ^= true;
    assert!(!table.check(&members));
    let mut proof = d.clone();
    proof.proofs[0].1 = 2;
    assert!(!proof.check(&members), "a proof binds the two members it was made for");
    let mut cover = d.clone();
    cover.cover.clear();
    cover.table.iter_mut().for_each(|t| t.clear());
    assert!(!cover.check(&members), "an empty cover tells no two classes apart");
    let mut merged = d.clone();
    merged.classes = vec![vec![0, 1, 2], vec![3, 5], vec![4]];
    merged.table.remove(1);
    assert!(!merged.check(&members), "OR is not proven equal to AND");
}

#[test]
fn an_unnamed_input_no_output_reads_is_left_alone() {
    // OR, with an input no output reads: the free bit a synthesis leaves where its source is undefined.
    let or_with = |read: bool| {
        let mut g = Aig::new();
        let (a, b) = (word(&mut g, "a", 2), word(&mut g, "b", 2));
        let free = g.add_input(None);
        let mut y: Vec<AigLit> = (0..2).map(|i| g.or(a[i], b[i])).collect();
        if read {
            y[0] = g.xor(y[0], free);
        }
        output_word(&mut g, "y", &y);
        g
    };
    let mut members = hypotheses();
    members[2] = or_with(false);
    let d = distinguish(&members, &[], 0, &Options::default()).unwrap();
    assert_eq!(d.classes, [vec![0, 1], vec![2], vec![3, 5], vec![4]]);
    assert_eq!(d.inputs, ["a[0]", "a[1]", "b[0]", "b[1]"], "an input no output reads takes no place in a vector");
    assert!(d.check(&members));
    members[2] = or_with(true);
    assert!(distinguish(&members, &[], 0, &Options::default()).is_err(), "an unnamed input an output reads cannot be assigned");
}

#[test]
fn every_query_is_counted_at_the_cut_it_was_put_at_with_its_verdict() {
    let (l, r) = adder_pair(16);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(report.all_equal());
    let s = &report.stats;
    assert_eq!(s.by_cut.len(), CUT_DEPTHS.len() + 2, "each depth, where the cones meet, the whole cone");
    for c in &s.by_cut {
        assert_eq!(c.queries, c.refuted + c.models + c.budget, "every query has one verdict: {c:?}");
        assert_eq!(c.queries == 0, c.vars == 0, "a query has variables: {c:?}");
    }
    let refuted: usize = s.by_cut.iter().map(|c| c.refuted).sum();
    assert_eq!(refuted, report.certificate.lemmas.len(), "a lemma per refutation");
    assert!(s.by_cut[0].queries > 0, "the narrowest cut is tried first");
    let queries: usize = s.by_cut.iter().map(|c| c.queries).sum();
    assert!(queries >= s.sat_calls, "each candidate is put at least once: {queries} of {}", s.sat_calls);
    let conflicts: u64 = s.by_cut.iter().map(|c| c.conflicts).sum();
    assert!(conflicts >= s.hardest.iter().map(|h| h.conflicts).sum::<u64>(), "the hardest are some of all");
    assert!(s.cone > 0 && s.swept == s.cone, "the sweep visits its whole cone: {} of {}", s.swept, s.cone);
    let unswept = equivalent(&l, &r, &Options { sweep: false, ..Options::default() }).unwrap();
    assert_eq!((unswept.stats.swept, unswept.stats.cone), (0, 0), "no sweep, no cone");
    assert!(unswept.stats.by_cut.iter().map(|c| c.queries).sum::<usize>() > 0, "the pairs are still put");
}

static REPORTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn reported(s: &Stats) {
    assert!(s.sat_calls > 0, "a report comes after a query");
    REPORTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn a_long_decision_reports_its_progress_every_so_many_candidates_and_decides_the_same() {
    let (l, r) = adder_pair(16);
    let quiet = equivalent(&l, &r, &Options::default()).unwrap();
    let every = 3;
    let loud = equivalent(&l, &r, &Options { progress: Some(Progress { every, report: reported }), ..Options::default() }).unwrap();
    assert!(loud.stats.sat_calls >= 2 * every, "enough candidates to report more than once: {}", loud.stats.sat_calls);
    assert_eq!(REPORTED.load(std::sync::atomic::Ordering::SeqCst), loud.stats.sat_calls / every, "one report each `every` candidates");
    assert_eq!(loud.verdicts, quiet.verdicts, "reporting decides nothing");
    assert_eq!(loud.stats, quiet.stats);
}
