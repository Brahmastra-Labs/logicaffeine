use super::*;
use crate::aig::{Aig, AigLit, Init};
use crate::sweep::Verdict;

fn word(g: &mut Aig, name: &str, width: usize) -> Vec<AigLit> {
    (0..width).map(|i| g.add_input(Some(format!("{name}[{i}]")))).collect()
}

fn add(g: &mut Aig, a: &[AigLit], b: &[AigLit]) -> Vec<AigLit> {
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

/// Registers every literal: a latch per bit whose next state is the bit.
fn register(g: &mut Aig, bits: &[AigLit], name: &str) -> Vec<AigLit> {
    bits.iter()
        .enumerate()
        .map(|(i, &b)| {
            let k = g.add_latch(Some(format!("{name}[{i}]")), Init::Free);
            g.set_next(k, b);
            g.latch_lit(k)
        })
        .collect()
}

fn outputs(g: &mut Aig, name: &str, bits: &[AigLit]) {
    for (i, &b) in bits.iter().enumerate() {
        g.add_output(b, Some(format!("{name}[{i}]")));
    }
}

fn comb_adder(width: usize) -> Aig {
    let mut g = Aig::new();
    let (a, b) = (word(&mut g, "a", width), word(&mut g, "b", width));
    let s = add(&mut g, &a, &b);
    outputs(&mut g, "y", &s);
    g
}

/// Operands registered, then the sum registered: the sum of the operands two clocks before.
fn pipelined_adder(width: usize, extra_stages: usize) -> Aig {
    let mut g = Aig::new();
    g.add_input(Some("clk".into()));
    let (a, b) = (word(&mut g, "a", width), word(&mut g, "b", width));
    let (ra, rb) = (register(&mut g, &a, "ra"), register(&mut g, &b, "rb"));
    let s = add(&mut g, &ra, &rb);
    let mut y = register(&mut g, &s, "y");
    for k in 0..extra_stages {
        y = register(&mut g, &y, &format!("z{k}"));
    }
    outputs(&mut g, "y", &y);
    g
}

#[test]
fn a_pipeline_equals_its_function_at_its_latency_and_nowhere_else() {
    let (pipe, comb) = (pipelined_adder(6, 0), comb_adder(6));
    let report = latency_equivalent(&pipe, &comb, 2, &Options::default()).unwrap();
    assert!(report.all_equal(), "{:?}", report.verdicts);
    assert!(report.check());
    for wrong in [0, 1, 3] {
        let report = latency_equivalent(&pipe, &comb, wrong, &Options::default()).unwrap();
        assert!(
            report.verdicts.iter().any(|(_, v)| matches!(v, Verdict::Differ { .. })),
            "latency {wrong} must not hold"
        );
        assert!(report.check(), "latency {wrong}: its witnesses replay");
    }
}

#[test]
fn two_pipelines_of_different_depths_align() {
    let (short, long) = (pipelined_adder(5, 0), pipelined_adder(5, 2));
    let report = aligned(&short, 2, &long, 4, &Options::default()).unwrap();
    assert!(report.all_equal());
    assert!(report.check());
    let report = aligned(&short, 2, &long, 3, &Options::default()).unwrap();
    assert!(!report.all_equal());
}

#[test]
fn feedback_is_not_a_pipeline() {
    let mut acc = Aig::new();
    let a = word(&mut acc, "a", 4);
    let k: Vec<usize> = (0..4).map(|i| acc.add_latch(Some(format!("s[{i}]")), Init::Zero)).collect();
    let s: Vec<AigLit> = k.iter().map(|&k| acc.latch_lit(k)).collect();
    let next = add(&mut acc, &s, &a);
    for (i, &k) in k.iter().enumerate() {
        acc.set_next(k, next[i]);
    }
    outputs(&mut acc, "y", &s);
    let mut id = Aig::new();
    let a = word(&mut id, "a", 4);
    outputs(&mut id, "y", &a);
    let report = latency_equivalent(&acc, &id, 1, &Options::default()).unwrap();
    assert!(report.all_equal(), "from a zero state, one step of an accumulator is its input");
    let report = latency_equivalent(&acc, &id, 2, &Options::default()).unwrap();
    assert!(!report.all_equal(), "two steps add the second input too");
}

#[test]
fn initial_values_unroll_to_constants_or_free_inputs() {
    for (init, want_equal) in [(Init::One, true), (Init::Zero, false), (Init::Free, false)] {
        let mut g = Aig::new();
        let k = g.add_latch(Some("q".into()), init);
        let q = g.latch_lit(k);
        g.set_next(k, q);
        g.add_output(q, Some("y".into()));
        let mut one = Aig::new();
        one.add_output(AigLit::TRUE, Some("y".into()));
        let report = latency_equivalent(&g, &one, 3, &Options::default()).unwrap();
        assert_eq!(report.all_equal(), want_equal, "{init:?}");
        assert!(report.check());
    }
}

#[test]
fn frames_are_named_by_their_clock() {
    let pipe = pipelined_adder(2, 0);
    let g = at_latency(&pipe, 2);
    assert!(g.latches().is_empty());
    let names: Vec<String> = g.inputs().iter().filter_map(|p| p.name.clone()).collect();
    assert_eq!(&names[..5], ["clk", "a[0]", "a[1]", "b[0]", "b[1]"], "frame 0 keeps the circuit's names, in order");
    for n in ["ra[0]@init", "y[1]@init", "a[1]@1", "b[0]@2", "clk@2"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    assert_eq!(g.outputs().iter().map(|o| o.name.clone().unwrap()).collect::<Vec<_>>(), ["y[0]", "y[1]"]);
    let all = unroll(&pipe, 3);
    let outs: Vec<String> = all.outputs().iter().map(|o| o.name.clone().unwrap()).collect();
    assert_eq!(outs, ["y[0]@0", "y[1]@0", "y[0]@1", "y[1]@1", "y[0]@2", "y[1]@2"]);
}

#[test]
fn a_combinational_reference_is_its_own_frame_zero() {
    let comb = comb_adder(3);
    assert_eq!(at_latency(&comb, 0).to_ascii(), comb.to_ascii());
}

/// A `width`-bit counter from `reset`, enabled by input `en`, its outputs `tick` (the count equals `at`) and `count`.
fn counter(width: usize, reset: u64, at: u64, copies: usize) -> Aig {
    let mut g = Aig::new();
    let en = g.add_input(Some("en".into()));
    let mut ticks = Vec::new();
    let mut first = Vec::new();
    for c in 0..copies {
        let k: Vec<usize> = (0..width)
            .map(|i| g.add_latch(Some(format!("c{c}[{i}]")), if reset >> i & 1 == 1 { Init::One } else { Init::Zero }))
            .collect();
        let q: Vec<AigLit> = k.iter().map(|&k| g.latch_lit(k)).collect();
        let mut carry = en;
        for i in 0..width {
            let next = g.xor(q[i], carry);
            carry = g.and(q[i], carry);
            g.set_next(k[i], next);
        }
        let eq: Vec<AigLit> = (0..width).map(|i| if at >> i & 1 == 1 { q[i] } else { !q[i] }).collect();
        let tick = eq[1..].iter().fold(eq[0], |acc, &b| g.and(acc, b));
        ticks.push(tick);
        if c == 0 {
            first = q;
        }
    }
    let tick = ticks[1..].iter().fold(ticks[0], |acc, &t| g.and(acc, t));
    g.add_output(tick, Some("tick".into()));
    for (i, &b) in first.iter().enumerate() {
        g.add_output(b, Some(format!("count[{i}]")));
    }
    g
}

#[test]
fn a_replicated_counter_is_the_counter_for_all_time() {
    let one = counter(5, 3, 17, 1);
    let three = counter(5, 3, 17, 3);
    let v = always_equal(&one, &three, &SeqOptions::default()).unwrap();
    let SeqVerdict::Equal(cert) = &v.verdict else { panic!("{:?}", v.verdict) };
    assert!(check_always(&v.product, cert), "the induction certificate checks");
    assert!(!cert.invariant.is_empty(), "the copies' latches correspond");
}

#[test]
fn a_different_reset_is_a_difference_with_a_trace_that_replays() {
    let a = counter(4, 0, 9, 1);
    let b = counter(4, 1, 9, 1);
    let v = always_equal(&a, &b, &SeqOptions::default()).unwrap();
    let SeqVerdict::Differ(trace) = &v.verdict else { panic!("{:?}", v.verdict) };
    assert!(replays(&v.product, trace), "the inputs, clock by clock, drive the two apart");
    assert_eq!(trace.frames.len(), 1, "the counts differ from the first clock");
}

#[test]
fn a_difference_deep_in_time_is_found_at_its_clock() {
    let a = counter(4, 0, 9, 1);
    // The same counter, its tick one count late.
    let b = {
        let mut g = Aig::new();
        let en = g.add_input(Some("en".into()));
        let k: Vec<usize> = (0..4).map(|i| g.add_latch(Some(format!("c0[{i}]")), Init::Zero)).collect();
        let q: Vec<AigLit> = k.iter().map(|&k| g.latch_lit(k)).collect();
        let mut carry = en;
        for i in 0..4 {
            let next = g.xor(q[i], carry);
            carry = g.and(q[i], carry);
            g.set_next(k[i], next);
        }
        let eq: Vec<AigLit> = (0..4).map(|i| if 10u64 >> i & 1 == 1 { q[i] } else { !q[i] }).collect();
        let tick = eq[1..].iter().fold(eq[0], |acc, &x| g.and(acc, x));
        g.add_output(tick, Some("tick".into()));
        for (i, &x) in q.iter().enumerate() {
            g.add_output(x, Some(format!("count[{i}]")));
        }
        g
    };
    let v = always_equal(&a, &b, &SeqOptions::default()).unwrap();
    let SeqVerdict::Differ(trace) = &v.verdict else { panic!("{:?}", v.verdict) };
    assert!(replays(&v.product, trace));
    assert_eq!(trace.frames.len(), 10, "the ticks part at the tenth clock: nine enables then the compare");
}

#[test]
fn equal_outputs_beyond_latch_correspondence_are_unknown_never_equal() {
    // A two-bit binary counter against a four-bit one-hot ring: the same tick, no latch of one equal to a latch of the
    // other.
    let mut bin = Aig::new();
    let en = bin.add_input(Some("en".into()));
    let k: Vec<usize> = (0..2).map(|i| bin.add_latch(Some(format!("b{i}")), Init::Zero)).collect();
    let q: Vec<AigLit> = k.iter().map(|&k| bin.latch_lit(k)).collect();
    let n0 = bin.xor(q[0], en);
    let c = bin.and(q[0], en);
    let n1 = bin.xor(q[1], c);
    bin.set_next(k[0], n0);
    bin.set_next(k[1], n1);
    let tick = bin.and(q[0], q[1]);
    bin.add_output(tick, Some("tick".into()));
    let mut ring = Aig::new();
    let en = ring.add_input(Some("en".into()));
    let k: Vec<usize> = (0..4).map(|i| ring.add_latch(Some(format!("r{i}")), if i == 0 { Init::One } else { Init::Zero })).collect();
    let q: Vec<AigLit> = k.iter().map(|&k| ring.latch_lit(k)).collect();
    for i in 0..4 {
        let prev = q[(i + 3) % 4];
        let next = ring.mux(en, prev, q[i]);
        ring.set_next(k[i], next);
    }
    ring.add_output(q[3], Some("tick".into()));
    let v = always_equal(&bin, &ring, &SeqOptions::default()).unwrap();
    assert!(matches!(v.verdict, SeqVerdict::Equal(_) | SeqVerdict::Unknown(_)), "{:?}", v.verdict);
    if let SeqVerdict::Equal(cert) = &v.verdict {
        assert!(check_always(&v.product, cert), "an equality is only ever claimed with a certificate that checks");
    }
}

#[test]
fn a_tampered_induction_certificate_is_refused() {
    let one = counter(4, 2, 11, 1);
    let two = counter(4, 2, 11, 2);
    let v = always_equal(&one, &two, &SeqOptions::default()).unwrap();
    let SeqVerdict::Equal(cert) = &v.verdict else { panic!("{:?}", v.verdict) };
    assert!(check_always(&v.product, cert));
    let mut wrong = cert.clone();
    let (a, b) = wrong.invariant[0];
    wrong.invariant[0] = (a, !b);
    assert!(!check_always(&v.product, &wrong), "an invariant the reset state breaks");
    let mut short = cert.clone();
    short.steps.pop();
    assert!(!check_always(&v.product, &short), "a step of the invariant left unproven");
    let mut no_outputs = cert.clone();
    no_outputs.outputs.clear();
    assert!(!check_always(&v.product, &no_outputs), "the outputs left unproven");
}

#[test]
fn a_latch_with_no_reset_value_never_anchors_the_invariant() {
    let mut a = Aig::new();
    let en = a.add_input(Some("en".into()));
    let k = a.add_latch(Some("q".into()), Init::Free);
    let q = a.latch_lit(k);
    a.set_next(k, en);
    a.add_output(q, Some("y".into()));
    let v = always_equal(&a, &a, &SeqOptions::default()).unwrap();
    assert!(
        matches!(v.verdict, SeqVerdict::Differ(_) | SeqVerdict::Unknown(_)),
        "two copies of a register with no reset value can start apart: {:?}",
        v.verdict
    );
}

#[test]
fn the_miter_a_pipeline_is_decided_on_holds_only_the_cones_of_its_outputs() {
    let width = 8;
    let comb = comb_adder(width);
    for stages in [1, 4, 12] {
        let report = latency_equivalent(&pipelined_adder(width, stages), &comb, 2 + stages, &Options::default()).unwrap();
        assert!(report.all_equal() && report.check(), "{stages} stages");
        assert!(
            report.miter.aig.num_ands() <= 2 * comb.num_ands() + 3 * width,
            "{stages} stages: {} ands where the function is {}: every clock but the one a stage computes in is dead",
            report.miter.aig.num_ands(),
            comb.num_ands()
        );
    }
}
