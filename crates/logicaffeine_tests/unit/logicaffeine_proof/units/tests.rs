use super::*;

/// A cut circuit `((a op b) op c)` over a 2-bit unit `op` (its results free inputs `<inst>.y[i]`, its operands outputs
/// `<inst>.a[i]`, `<inst>.b[i]`), the result `y` an output; `swap` feeds the outer unit (c, inner) instead of
/// (inner, c); `registered` holds every value a clock so the circuit has latency `stages`.
fn chain(swap: bool) -> (Aig, Vec<Unit>) {
    let mut g = Aig::new();
    let word = |g: &mut Aig, name: &str| -> Vec<AigLit> { (0..2).map(|i| g.add_input(Some(format!("{name}[{i}]")))).collect() };
    let (a, b, c) = (word(&mut g, "a"), word(&mut g, "b"), word(&mut g, "c"));
    let inner = word(&mut g, "inner.y");
    let outer = word(&mut g, "outer.y");
    for (inst, x, y) in [("inner", &a, &b), ("outer", if swap { &c } else { &inner }, if swap { &inner } else { &c })] {
        for i in 0..2 {
            g.add_output(x[i], Some(format!("{inst}.a[{i}]")));
            g.add_output(y[i], Some(format!("{inst}.b[{i}]")));
        }
    }
    for i in 0..2 {
        g.add_output(outer[i], Some(format!("y[{i}]")));
    }
    let unit = |name: &str| Unit {
        name: name.to_string(),
        function: "op".into(),
        latency: 0,
        operands: (0..2).flat_map(|i| [format!("{name}.a[{i}]"), format!("{name}.b[{i}]")]).collect(),
        results: (0..2).map(|i| format!("{name}.y[{i}]")).collect(),
    };
    (g, vec![unit("inner"), unit("outer")])
}

fn functions() -> Vec<(String, String)> {
    vec![("op".into(), "op".into())]
}

#[test]
fn a_composition_of_units_is_proven_over_its_units_round_by_round() {
    let (l, lu) = chain(false);
    let (r, ru) = chain(false);
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &r, latency: 0, units: &ru });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{:?}", report.verdicts);
    assert_eq!(report.verdicts.len(), 2);
    assert_eq!(report.rounds.len(), 3, "the inner unit, then the outer, then the outputs: {:?}", report.rounds.iter().map(|r| r.substituted.len()).collect::<Vec<_>>());
    assert!(report.check(&left, &right, &functions()), "every round's derivation and every substitution check");
}

#[test]
fn units_fed_other_operands_are_never_identified_and_the_outputs_are_left_unproven() {
    let (l, lu) = chain(false);
    let (r, ru) = chain(true);
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &r, latency: 0, units: &ru });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| matches!(v, Verdict::Unknown(_))), "over units it knows nothing of, no verdict: {:?}", report.verdicts);
    assert_eq!(report.rounds.iter().map(|r| r.substituted.len()).sum::<usize>(), 1, "the inner units alone are identified");
    assert!(report.check(&left, &right, &functions()));
}

#[test]
fn a_substitution_the_rounds_do_not_justify_does_not_check() {
    let (l, lu) = chain(false);
    let (r, ru) = chain(true);
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &r, latency: 0, units: &ru });
    let mut report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    // Forge the outer units' identification into the first round.
    let forged = Application { side: 0, unit: 1, frame: 0 };
    report.rounds[0].substituted.push((forged, Application { side: 1, unit: 1, frame: 0 }));
    assert!(!report.check(&left, &right, &functions()), "an identification of units whose operands were never proven equal");
    // A right side whose inner unit's first operand is a only by proof, (a ∧ x) ∨ (a ∧ ¬x): its round has a derivation.
    let (mut g, gu) = chain(false);
    let x = g.add_input(Some("x".into()));
    let mut redundant = Aig::new();
    let mut map = Vec::new();
    let names: Vec<Option<String>> = g.inputs().iter().map(|p| p.name.clone()).collect();
    for name in &names {
        map.push(redundant.add_input(name.clone()));
    }
    let lits = redundant.import(&g, &mut |leaf| match leaf {
        crate::aig::Leaf::Input(i) => map[i],
        crate::aig::Leaf::Latch(_) => unreachable!(),
    });
    let at = |l: AigLit| lits[l.node() as usize].with_phase(l.is_complemented());
    let xr = at(x);
    for o in g.outputs() {
        let name = o.name.clone().unwrap();
        let lit = at(o.lit);
        let lit = if name.starts_with("inner.a[") {
            let (p, q) = (redundant.and(lit, xr), redundant.and(lit, !xr));
            redundant.or(p, q)
        } else {
            lit
        };
        redundant.add_output(lit, Some(name));
    }
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &redundant, latency: 0, units: &gu });
    let good = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(good.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{:?}", good.verdicts);
    assert!(good.check(&left, &right, &functions()));
    assert!(!good.rounds[0].proof.steps.is_empty(), "the first operand's equality is derived");
    assert!(!good.check(&left, &right, &[("op".into(), "other".into())]), "an identification of functions never declared equal");
    let mut tampered = good.clone();
    let half = tampered.rounds[0].proof.steps.len() / 2;
    tampered.rounds[0].proof.steps.truncate(half);
    assert!(!tampered.check(&left, &right, &functions()), "a round's derivation cut short");
}

#[test]
fn a_pipelined_unit_is_matched_its_latency_after_its_operands() {
    // The left registers its unit's result twice before the next unit reads it; the right reads it straight.
    let mut g = Aig::new();
    let word = |g: &mut Aig, name: &str| -> Vec<AigLit> { (0..2).map(|i| g.add_input(Some(format!("{name}[{i}]")))).collect() };
    let (a, b, c) = (word(&mut g, "a"), word(&mut g, "b"), word(&mut g, "c"));
    let inner = word(&mut g, "inner.y");
    let outer = word(&mut g, "outer.y");
    // c delayed two clocks to meet inner's result.
    let mut late = c.clone();
    for stage in 0..2 {
        late = late
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let j = g.add_latch(Some(format!("c{stage}[{i}]")), crate::aig::Init::Free);
                g.set_next(j, x);
                g.latch_lit(j)
            })
            .collect();
    }
    for i in 0..2 {
        g.add_output(a[i], Some(format!("inner.a[{i}]")));
        g.add_output(b[i], Some(format!("inner.b[{i}]")));
        g.add_output(inner[i], Some(format!("outer.a[{i}]")));
        g.add_output(late[i], Some(format!("outer.b[{i}]")));
    }
    for i in 0..2 {
        g.add_output(outer[i], Some(format!("y[{i}]")));
    }
    let unit = |name: &str, latency: usize| Unit {
        name: name.to_string(),
        function: if latency == 0 { "op".into() } else { "op_p".into() },
        latency,
        operands: (0..2).flat_map(|i| [format!("{name}.a[{i}]"), format!("{name}.b[{i}]")]).collect(),
        results: (0..2).map(|i| format!("{name}.y[{i}]")).collect(),
    };
    let lu = vec![unit("inner", 2), unit("outer", 2)];
    let (r, ru) = chain(false);
    let (left, right) = (Side { aig: &g, latency: 4, units: &lu }, Side { aig: &r, latency: 0, units: &ru });
    let pipelined = [("op_p".to_string(), "op".to_string())];
    let report = decide(&left, &right, &pipelined, &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{:?}", report.verdicts);
    assert!(report.check(&left, &right, &pipelined));
}

#[test]
fn an_operand_that_differs_on_one_input_in_a_million_keeps_the_units_apart() {
    // The right's inner unit reads a[0] ⊕ (x0 ∧ … ∧ x19): simulation all but never sees the difference, so the pair is
    // a candidate, and its first operand bit is refuted — the units are not one, whatever their other bits.
    let (l, lu) = chain(false);
    let (g, gu) = chain(false);
    let mut r = Aig::new();
    let map: Vec<AigLit> = g.inputs().iter().map(|p| r.add_input(p.name.clone())).collect();
    let lits = r.import(&g, &mut |leaf| match leaf {
        crate::aig::Leaf::Input(i) => map[i],
        crate::aig::Leaf::Latch(_) => unreachable!(),
    });
    let rare = (0..20).fold(AigLit::TRUE, |acc, i| {
        let x = r.add_input(Some(format!("x{i}")));
        r.and(acc, x)
    });
    for o in g.outputs() {
        let name = o.name.clone().unwrap();
        let lit = lits[o.lit.node() as usize].with_phase(o.lit.is_complemented());
        let lit = if name == "inner.a[0]" { r.xor(lit, rare) } else { lit };
        r.add_output(lit, Some(name));
    }
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &r, latency: 0, units: &gu });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| *v != Verdict::Equal), "{:?}", report.verdicts);
    assert_eq!(report.rounds.iter().map(|r| r.substituted.len()).sum::<usize>(), 0, "no unit identified");
    assert!(report.check(&left, &right, &functions()));
}

/// One output `y` read straight from an input no name is given (an undefined bit), or from the named input `a`.
fn unnamed_read(unnamed: bool) -> Aig {
    let mut g = Aig::new();
    let a = g.add_input(Some("a".into()));
    let free = g.add_input(None);
    g.add_output(if unnamed { free } else { a }, Some("y".into()));
    g
}

#[test]
fn an_input_no_name_is_given_is_its_own_sides_at_every_clock_never_the_other_sides() {
    let (l, r) = (unnamed_read(true), unnamed_read(true));
    for latency in [0, 2] {
        let (left, right) = (Side { aig: &l, latency, units: &[] }, Side { aig: &r, latency, units: &[] });
        let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
        assert!(matches!(report.verdicts[0].1, Verdict::Unknown(_)), "two undefined bits are two values: {:?}", report.verdicts);
        assert!(report.check(&left, &right, &functions()));
    }
    let (n, u) = (unnamed_read(false), unnamed_read(false));
    let report = decide(&Side { aig: &n, latency: 0, units: &[] }, &Side { aig: &u, latency: 0, units: &[] }, &functions(), &fraig::Options::default()).unwrap();
    assert_eq!(report.verdicts[0].1, Verdict::Equal, "a named input is one value on both sides");
}

#[test]
fn a_latch_no_name_is_given_starts_from_its_own_value_never_a_named_latchs() {
    // Two latches from any value, the first named as the second's index would be: their sum is no constant.
    let mut l = Aig::new();
    let named = l.add_latch(Some("latch1".into()), Init::Free);
    let unnamed = l.add_latch(None, Init::Free);
    let (p, q) = (l.latch_lit(named), l.latch_lit(unnamed));
    l.set_next(named, p);
    l.set_next(unnamed, q);
    let y = l.xor(p, q);
    l.add_output(y, Some("y".into()));
    let mut r = Aig::new();
    r.add_output(AigLit::FALSE, Some("y".into()));
    let (left, right) = (Side { aig: &l, latency: 0, units: &[] }, Side { aig: &r, latency: 0, units: &[] });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(matches!(report.verdicts[0].1, Verdict::Unknown(_)), "two latches are two values: {:?}", report.verdicts);
    assert!(report.check(&left, &right, &functions()));
    // One latch with no name, from any value, is no constant either.
    let mut one = Aig::new();
    let k = one.add_latch(None, Init::Free);
    let held = one.latch_lit(k);
    one.set_next(k, held);
    one.add_output(held, Some("y".into()));
    let lone = Side { aig: &one, latency: 0, units: &[] };
    let report = decide(&lone, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(matches!(report.verdicts[0].1, Verdict::Unknown(_)), "a latch with no reset value starts anywhere: {:?}", report.verdicts);
}

/// A one-bit register `y` taking `x` each clock its enable `en` is high, `y` an output; or with no register, `y` = `x`.
fn register(registered: bool) -> Aig {
    let mut g = Aig::new();
    let x = g.add_input(Some("x".into()));
    let en = g.add_input(Some("en".into()));
    if registered {
        let k = g.add_latch(Some("q".into()), Init::Zero);
        let q = g.latch_lit(k);
        let next = g.mux(en, x, q);
        g.set_next(k, next);
        g.add_output(q, Some("y".into()));
    } else {
        g.add_output(x, Some("y".into()));
    }
    g
}

fn claim(name: &str, output: &str, frame: usize) -> Claim {
    Claim { name: name.into(), output: output.into(), frame }
}

#[test]
fn a_token_read_at_other_clocks_by_each_side_is_one_value_and_each_sides_output_is_claimed_where_it_makes_it() {
    let (reg, wire) = (register(true), register(false));
    // The register takes token 0 at clock 0 and token 1 at clock 2, its enable low between; the wire passes token 0 at
    // clock 0 and token 1 at clock 1.
    let reg_bind = |name: &str, k: usize| match (name, k) {
        ("x", 0) => Binding::Shared("tok0".into()),
        ("x", 2) => Binding::Shared("tok1".into()),
        ("x", _) => Binding::Fresh,
        ("en", k) => Binding::Const(k == 0 || k == 2),
        _ => Binding::Fresh,
    };
    let wire_bind = |name: &str, k: usize| match (name, k) {
        ("x", 0) => Binding::Shared("tok0".into()),
        ("x", 1) => Binding::Shared("tok1".into()),
        _ => Binding::Fresh,
    };
    let reg_claims = [claim("out0", "y", 1), claim("out1", "y", 3)];
    let wire_claims = [claim("out0", "y", 0), claim("out1", "y", 1)];
    let left = Unrolled { aig: &reg, frames: 4, bind: &reg_bind, claims: &reg_claims, units: &[] };
    let right = Unrolled { aig: &wire, frames: 2, bind: &wire_bind, claims: &wire_claims, units: &[] };
    let report = decide_unrolled(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert_eq!(report.verdicts.iter().map(|(n, v)| (n.as_str(), v.clone())).collect::<Vec<_>>(), [("out0", Verdict::Equal), ("out1", Verdict::Equal)]);
    assert!(report.check_unrolled(&left, &right, &functions()));
    // The enable held low at clock 2: the register keeps token 0, no longer token 1.
    let stalled = |name: &str, k: usize| if name == "en" { Binding::Const(k == 0) } else { reg_bind(name, k) };
    let left = Unrolled { aig: &reg, frames: 4, bind: &stalled, claims: &reg_claims, units: &[] };
    let report = decide_unrolled(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert_eq!(report.verdicts[0].1, Verdict::Equal);
    assert!(matches!(report.verdicts[1].1, Verdict::Unknown(_)), "a token not taken is not the token: {:?}", report.verdicts);
}

#[test]
fn a_claim_on_no_output_or_past_the_clocks_unrolled_is_refused() {
    let wire = register(false);
    let bind = |name: &str, _: usize| Binding::Shared(name.into());
    let fine = [claim("y", "y", 0)];
    let missing = [claim("y", "z", 0)];
    let late = [claim("y", "y", 2)];
    let side = |claims| Unrolled { aig: &wire, frames: 2, bind: &bind, claims, units: &[] };
    assert!(decide_unrolled(&side(&fine), &side(&fine), &functions(), &fraig::Options::default()).is_ok());
    assert!(matches!(decide_unrolled(&side(&missing), &side(&fine), &functions(), &fraig::Options::default()), Err(MiterError::Unmatched(_))));
    assert!(matches!(decide_unrolled(&side(&late), &side(&fine), &functions(), &fraig::Options::default()), Err(MiterError::Unmatched(_))));
}

/// Units of function `op` named `insts`, each fed (a, b) — the first operand's bit 0 of each named in `rare` xored with
/// x0 ∧ … ∧ x19, so it differs on one input in a million — and the output `y` the result of `read`.
fn beside(insts: &[&str], rare: &[&str], read: &str) -> (Aig, Vec<Unit>) {
    let mut g = Aig::new();
    let word = |g: &mut Aig, name: &str| -> Vec<AigLit> { (0..2).map(|i| g.add_input(Some(format!("{name}[{i}]")))).collect() };
    let (a, b) = (word(&mut g, "a"), word(&mut g, "b"));
    let results: Vec<Vec<AigLit>> = insts.iter().map(|inst| word(&mut g, &format!("{inst}.y"))).collect();
    let one_in_a_million = (0..20).fold(AigLit::TRUE, |acc, i| {
        let x = g.add_input(Some(format!("x{i}")));
        g.and(acc, x)
    });
    for inst in insts {
        for i in 0..2 {
            let first = if i == 0 && rare.contains(inst) { g.xor(a[i], one_in_a_million) } else { a[i] };
            g.add_output(first, Some(format!("{inst}.a[{i}]")));
            g.add_output(b[i], Some(format!("{inst}.b[{i}]")));
        }
    }
    let at = insts.iter().position(|i| *i == read).expect("the unit read");
    for i in 0..2 {
        g.add_output(results[at][i], Some(format!("y[{i}]")));
    }
    let unit = |name: &str| Unit {
        name: name.to_string(),
        function: "op".into(),
        latency: 0,
        operands: (0..2).flat_map(|i| [format!("{name}.a[{i}]"), format!("{name}.b[{i}]")]).collect(),
        results: (0..2).map(|i| format!("{name}.y[{i}]")).collect(),
    };
    (g, insts.iter().map(|i| unit(i)).collect())
}

#[test]
fn two_units_of_one_side_on_equal_operands_are_one_whichever_its_output_reads() {
    let (one, ou) = beside(&["only"], &[], "only");
    for read in ["first", "second"] {
        let (two, tu) = beside(&["first", "second"], &[], read);
        for (l, lu, r, ru) in [(&two, &tu, &one, &ou), (&one, &ou, &two, &tu)] {
            let (left, right) = (Side { aig: l, latency: 0, units: lu }, Side { aig: r, latency: 0, units: ru });
            let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
            assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{read}: {:?}", report.verdicts);
            assert!(report.check(&left, &right, &functions()), "{read}: an identification within a side checks");
        }
    }
}

#[test]
fn a_unit_refuted_on_one_input_in_a_million_never_keeps_the_units_it_was_tried_against_apart() {
    // The left's first unit (`odd`) reads a[0] ⊕ (x0 ∧ … ∧ x19): it simulates as every other unit does, is tried against
    // them first and refuted, and the left's `even` and the right's `only` are still one.
    let (two, tu) = beside(&["odd", "even"], &["odd"], "even");
    let (one, ou) = beside(&["only"], &[], "only");
    let (left, right) = (Side { aig: &two, latency: 0, units: &tu }, Side { aig: &one, latency: 0, units: &ou });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{:?}", report.verdicts);
    let odd = Application { side: 0, unit: 0, frame: 0 };
    assert!(
        report.rounds.iter().flat_map(|r| &r.substituted).all(|&(p, q)| p != odd && q != odd),
        "the unit fed other operands is never identified: {:?}",
        report.rounds.iter().map(|r| &r.substituted).collect::<Vec<_>>()
    );
    assert!(report.check(&left, &right, &functions()));
    // Read through the odd unit, the output is unproven.
    let (odd_read, _) = beside(&["odd", "even"], &["odd"], "odd");
    let left = Side { aig: &odd_read, latency: 0, units: &tu };
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| matches!(v, Verdict::Unknown(_))), "{:?}", report.verdicts);
}

#[test]
fn an_identification_made_twice_or_closing_a_cycle_does_not_check_and_never_hangs_the_check() {
    let (two, tu) = beside(&["first", "second"], &[], "second");
    let (one, ou) = beside(&["only"], &[], "only");
    let (left, right) = (Side { aig: &two, latency: 0, units: &tu }, Side { aig: &one, latency: 0, units: &ou });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.check(&left, &right, &functions()));
    let made: Vec<(Application, Application)> = report.rounds.iter().flat_map(|r| r.substituted.iter().copied()).collect();
    assert_eq!(made.len(), 2, "the three applications are one by two identifications: {made:?}");
    let (first, second, only) = (Application { side: 0, unit: 0, frame: 0 }, Application { side: 0, unit: 1, frame: 0 }, Application { side: 1, unit: 0, frame: 0 });
    for forged in [(second, first), (only, first), (second, only), (only, second), (first, first)] {
        let mut tampered = report.clone();
        tampered.rounds[0].substituted.push(forged);
        assert!(!tampered.check(&left, &right, &functions()), "{forged:?}: an application made one twice over, or a cycle");
    }
    // Within a side the units are one function by their name alone; across, only as `functions` declares.
    let other = [("op".to_string(), "other".to_string())];
    assert!(!report.check(&left, &right, &other), "the right's op is not declared the left's");
}

/// Units of `op`: r0 and r1 on (a, b), q0 on (r0, c), q1 and q2 on (r1, c); the output `y` is q2's result.
fn doubled() -> (Aig, Vec<Unit>) {
    let mut g = Aig::new();
    let word = |g: &mut Aig, name: &str| -> Vec<AigLit> { (0..2).map(|i| g.add_input(Some(format!("{name}[{i}]")))).collect() };
    let (a, b, c) = (word(&mut g, "a"), word(&mut g, "b"), word(&mut g, "c"));
    let names = ["r0", "r1", "q0", "q1", "q2"];
    let results: Vec<Vec<AigLit>> = names.iter().map(|n| word(&mut g, &format!("{n}.y"))).collect();
    let feeds = [(&a, &b), (&a, &b), (&results[0], &c), (&results[1], &c), (&results[1], &c)];
    for (n, (x, y)) in names.iter().zip(feeds) {
        for i in 0..2 {
            g.add_output(x[i], Some(format!("{n}.a[{i}]")));
            g.add_output(y[i], Some(format!("{n}.b[{i}]")));
        }
    }
    for i in 0..2 {
        g.add_output(results[4][i], Some(format!("y[{i}]")));
    }
    let unit = |name: &str| Unit {
        name: name.to_string(),
        function: "op".into(),
        latency: 0,
        operands: (0..2).flat_map(|i| [format!("{name}.a[{i}]"), format!("{name}.b[{i}]")]).collect(),
        results: (0..2).map(|i| format!("{name}.y[{i}]")).collect(),
    };
    (g, names.iter().map(|n| unit(n)).collect())
}

#[test]
fn an_application_kept_in_one_round_and_made_one_in_a_later_carries_those_made_it_with_it() {
    // Round one makes q2 q1 (their operands are one wire) and r1 r0; only then are q1's operands q0's, and round two
    // makes q1 q0: the output, q2's result, is q0's through q1.
    let (l, lu) = doubled();
    let (r, ru) = chain(false);
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &r, latency: 0, units: &ru });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().all(|(_, v)| *v == Verdict::Equal), "{:?}", report.verdicts);
    let q1 = Application { side: 0, unit: 3, frame: 0 };
    let kept_at = report.rounds.iter().position(|r| r.substituted.iter().any(|&(k, _)| k == q1));
    let made_at = report.rounds.iter().position(|r| r.substituted.iter().any(|&(_, m)| m == q1));
    assert!(matches!((kept_at, made_at), (Some(k), Some(m)) if k < m), "q1 kept, then made one: {:?}", report.rounds.iter().map(|r| &r.substituted).collect::<Vec<_>>());
    assert!(report.check(&left, &right, &functions()));
}

#[test]
fn an_identification_of_one_functions_units_whose_results_differ_in_shape_does_not_check() {
    // The right's inner unit names one result bit, its second an input of its own: no group holds the two inner units.
    let (l, lu) = chain(false);
    let (r, mut ru) = chain(false);
    ru[0].results.truncate(1);
    let (left, right) = (Side { aig: &l, latency: 0, units: &lu }, Side { aig: &r, latency: 0, units: &ru });
    let report = decide(&left, &right, &functions(), &fraig::Options::default()).unwrap();
    let inner = (Application { side: 0, unit: 0, frame: 0 }, Application { side: 1, unit: 0, frame: 0 });
    assert!(report.rounds.iter().flat_map(|r| &r.substituted).all(|p| *p != inner), "{:?}", report.rounds.iter().map(|r| &r.substituted).collect::<Vec<_>>());
    assert!(report.verdicts.iter().all(|(_, v)| matches!(v, Verdict::Unknown(_))));
    assert!(report.check(&left, &right, &functions()));
    // A report identifying the inner units, their operands one wire on both sides so nothing is to derive, and the
    // outputs left unknown: it checks where the units are of one shape, never where a two-bit result is made a one-bit
    // one's.
    let empty = || Lrat { steps: Vec::new() };
    let forged = Report {
        verdicts: report.verdicts.clone(),
        rounds: vec![Round { substituted: vec![inner], proof: empty() }, Round { substituted: Vec::new(), proof: empty() }],
        stats: report.stats.clone(),
    };
    let (whole, wu) = chain(false);
    let shaped = Side { aig: &whole, latency: 0, units: &wu };
    assert!(forged.check(&left, &shaped, &functions()), "the forged report's frame checks where the shapes agree");
    assert!(!forged.check(&left, &right, &functions()), "a two-bit result made a one-bit one's");
}
