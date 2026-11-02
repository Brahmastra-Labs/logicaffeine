//! The certified equivalence engine through its public surface: circuits written to and read back from both AIGER
//! forms, combinational miters decided by SAT sweeping, pipelines unrolled to their latency, and every lemma's proof
//! replayed both by the structured LRAT checker and, as text, by the reference checker
//! (`logicaffeine_proof::proof_emit::check_lrat`).

use logicaffeine_proof::aig::{Aig, AigLit};
use logicaffeine_proof::lrat;
use logicaffeine_proof::proof_emit::check_lrat;
use logicaffeine_proof::seq_equiv::latency_equivalent;
use logicaffeine_proof::sweep::{equivalent, query_with_cut, Options, Report, Verdict};

fn word(g: &mut Aig, name: &str, width: usize) -> Vec<AigLit> {
    (0..width).map(|i| g.add_input(Some(format!("{name}[{i}]")))).collect()
}

fn named_outputs(g: &mut Aig, name: &str, bits: &[AigLit]) {
    for (i, &b) in bits.iter().enumerate() {
        g.add_output(b, Some(format!("{name}[{i}]")));
    }
}

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

/// A carry-select adder: each block of four computed for both carries in, the true carry choosing.
fn carry_select(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
    let mut carry = AigLit::FALSE;
    let mut sum = Vec::new();
    for block in (0..a.len()).collect::<Vec<_>>().chunks(4) {
        let run = |g: &mut Aig, cin: AigLit| -> (Vec<AigLit>, AigLit) {
            let mut c = cin;
            let mut s = Vec::new();
            for &i in block {
                let p = g.xor(a[i], b[i]);
                s.push(g.xor(p, c));
                let gen = g.and(a[i], b[i]);
                let prop = g.and(p, c);
                c = g.or(gen, prop);
            }
            (s, c)
        };
        let (s0, c0) = run(g, AigLit::FALSE);
        let (s1, c1) = run(g, AigLit::TRUE);
        for k in 0..s0.len() {
            sum.push(g.mux(carry, s1[k], s0[k]));
        }
        carry = g.mux(carry, c1, c0);
    }
    sum
}

fn adders(width: usize) -> (Aig, Aig) {
    let mut l = Aig::new();
    let (a, b) = (word(&mut l, "a", width), word(&mut l, "b", width));
    let s = ripple(&mut l, &a, &b);
    named_outputs(&mut l, "s", &s);
    let mut r = Aig::new();
    let (a2, b2) = (word(&mut r, "a", width), word(&mut r, "b", width));
    let s = carry_select(&mut r, &a2, &b2);
    named_outputs(&mut r, "s", &s);
    (l, r)
}

fn replay_every_lemma(report: &Report) {
    let mut earlier = Vec::new();
    for lemma in &report.certificate.lemmas {
        let q = query_with_cut(&report.miter.aig, lemma.a, lemma.b, &lemma.cut, &earlier);
        assert!(lrat::check(q.num_vars, &q.clauses, &lemma.proof));
        assert!(check_lrat(q.num_vars, &q.clauses, &lemma.proof.to_text()), "the reference checker reads the text");
        earlier.push((lemma.a, lemma.b));
    }
}

#[test]
fn a_ripple_adder_equals_a_carry_select_adder_and_every_lemma_replays() {
    let (l, r) = adders(16);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    assert!(report.all_equal(), "{:?}", report.verdicts);
    assert!(report.check());
    replay_every_lemma(&report);
}

#[test]
fn a_circuit_read_back_from_either_aiger_form_is_proven_the_same_circuit() {
    let (l, _) = adders(12);
    for back in [Aig::parse(l.to_ascii().as_bytes()).unwrap(), Aig::parse(&l.to_binary()).unwrap()] {
        assert_eq!(back.to_ascii(), l.to_ascii());
        let report = equivalent(&l, &back, &Options::default()).unwrap();
        assert!(report.all_equal());
        assert!(report.check());
    }
}

#[test]
fn a_miter_written_as_aiger_flags_exactly_the_outputs_that_differ() {
    let (l, _) = adders(8);
    let mut r = Aig::new();
    let (a, b) = (word(&mut r, "a", 8), word(&mut r, "b", 8));
    let mut s = carry_select(&mut r, &a, &b);
    s[6] = !s[6];
    named_outputs(&mut r, "s", &s);
    let report = equivalent(&l, &r, &Options::default()).unwrap();
    let written = Aig::parse(report.miter.aig.to_ascii().as_bytes()).unwrap();
    assert_eq!(written.outputs().len(), 8, "one difference output a pair");
    for ((name, verdict), out) in report.verdicts.iter().zip(written.outputs()) {
        assert_eq!(out.name.as_deref(), Some(name.as_str()));
        match verdict {
            Verdict::Differ { inputs, latches } => {
                assert_eq!(name, "s[6]");
                let v = written.eval(inputs, latches);
                assert!(Aig::value(&v, out.lit), "the written miter's output is true on the witness");
            }
            Verdict::Equal => assert_ne!(name, "s[6]"),
            Verdict::Unknown(why) => panic!("{name}: {why}"),
        }
    }
    assert!(report.check());
}

#[test]
fn a_shift_register_read_from_aiger_is_a_pipeline_of_its_length() {
    let pipe = Aig::parse(b"aag 3 1 2 1 0\n2\n4 2 4\n6 4 6\n6\ni0 d\nl0 first\nl1 second\no0 y\n").unwrap();
    let wire = Aig::parse(b"aag 1 1 0 1 0\n2\n2\ni0 d\no0 y\n").unwrap();
    for (latency, holds) in [(0, false), (1, false), (2, true), (3, false)] {
        let report = latency_equivalent(&pipe, &wire, latency, &Options::default()).unwrap();
        assert_eq!(report.all_equal(), holds, "latency {latency}");
        assert!(report.check());
        replay_every_lemma(&report);
    }
}

#[test]
fn a_constant_output_is_decided_without_a_solver_call() {
    let mut l = Aig::new();
    let x = l.add_input(Some("x".into()));
    l.add_output(x, Some("y".into()));
    let report = equivalent(&l, &l, &Options::default()).unwrap();
    assert!(report.all_equal());
    assert!(report.certificate.lemmas.is_empty(), "the same literal needs no lemma");
    assert_eq!(report.stats.sat_calls, 0);
}
