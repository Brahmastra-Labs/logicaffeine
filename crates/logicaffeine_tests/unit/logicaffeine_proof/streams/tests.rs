use super::*;

/// A stream through `stages` registers: always ready, a token taken whenever offered, given `stages` clocks later with
/// `out_valid`; two data bits, the second inverted when `invert`; `in_ready` the first data bit when `ready_on_data`.
fn pipe(stages: usize, invert: bool, ready_on_data: bool) -> Aig {
    let mut g = Aig::new();
    let valid = g.add_input(Some("in_valid".into()));
    let data: Vec<AigLit> = (0..2).map(|i| g.add_input(Some(format!("in_data[{i}]")))).collect();
    let ready = if ready_on_data { data[0] } else { AigLit::TRUE };
    g.add_output(ready, Some("in_ready".into()));
    let (mut v, mut d) = (g.and(valid, ready), data.clone());
    for s in 0..stages {
        let kv = g.add_latch(Some(format!("v{s}")), Init::Zero);
        g.set_next(kv, v);
        v = g.latch_lit(kv);
        d = d
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let k = g.add_latch(Some(format!("d{s}_{i}")), Init::Zero);
                g.set_next(k, x);
                g.latch_lit(k)
            })
            .collect();
    }
    g.add_output(v, Some("out_valid".into()));
    g.add_output(d[0], Some("out_data[0]".into()));
    g.add_output(if invert { !d[1] } else { d[1] }, Some("out_data[1]".into()));
    g
}

fn token(j: usize, bit: &str) -> Binding {
    Binding::Shared(format!("tok{j}.{bit}"))
}

fn ports() -> Ports<'static> {
    Ports {
        in_valid: "in_valid".into(),
        in_ready: "in_ready".into(),
        in_data: vec!["in_data[0]".into(), "in_data[1]".into()],
        out_valid: "out_valid".into(),
        out_ready: None,
        out_data: vec!["out_data[0]".into(), "out_data[1]".into()],
        token: &token,
    }
}

fn nothing(_: &str, _: usize) -> Binding {
    Binding::Fresh
}

#[test]
fn two_pipes_of_different_depths_give_each_token_they_take_under_any_arrival_pattern_given() {
    let (one, two) = (pipe(1, false, false), pipe(2, false, false));
    for offer in [&(|_: usize| true) as &dyn Fn(usize) -> bool, &|k: usize| k % 3 != 1] {
        let accept = |_: usize| true;
        let env = Environment { frames: 10, offer, accept: &accept, other: &nothing };
        let (l, r) = (StreamSide { aig: &one, units: &[], ports: ports() }, StreamSide { aig: &two, units: &[], ports: ports() });
        let report = decide(&l, &env, &r, &env, &[], &fraig::Options::default()).unwrap();
        assert!(report.all_equal(), "{:?}", report.verdicts);
        assert_eq!(report.schedules[0].took, (0..10).filter(|&k| offer(k)).collect::<Vec<_>>(), "always ready: every offer taken");
        assert_eq!(report.gave, [report.schedules[0].took.iter().filter(|&&k| k + 1 < 10).count(), report.schedules[1].took.iter().filter(|&&k| k + 2 < 10).count()]);
        assert_eq!(report.verdicts.len(), 2 * report.gave[0].min(report.gave[1]), "every bit of every token both gave");
        assert!(report.check(&l, &env, &r, &env, &[]));
    }
}

#[test]
fn a_pipe_that_changes_a_bit_is_not_the_stream_and_one_whose_ready_reads_the_data_has_no_schedule() {
    let accept = |_: usize| true;
    let offer = |_: usize| true;
    let env = Environment { frames: 6, offer: &offer, accept: &accept, other: &nothing };
    let (one, flipped) = (pipe(1, false, false), pipe(2, true, false));
    let report = decide(&StreamSide { aig: &one, units: &[], ports: ports() }, &env, &StreamSide { aig: &flipped, units: &[], ports: ports() }, &env, &[], &fraig::Options::default()).unwrap();
    assert!(report.verdicts.iter().any(|(n, v)| n.ends_with(".1") && matches!(v, Verdict::Unknown(_))), "{:?}", report.verdicts);
    assert!(report.verdicts.iter().filter(|(n, _)| n.ends_with(".0")).all(|(_, v)| *v == Verdict::Equal));
    let reading = pipe(1, false, true);
    let refused = schedule(&StreamSide { aig: &reading, units: &[], ports: ports() }, &env, &fraig::Options::default()).unwrap_err();
    assert_eq!(refused, StreamError::DataDependent { port: "in_ready".into(), frame: 0 });
}

#[test]
fn a_schedule_claiming_a_handshake_the_side_never_makes_does_not_check() {
    let accept = |_: usize| true;
    let offer = |k: usize| k % 2 == 0;
    let env = Environment { frames: 6, offer: &offer, accept: &accept, other: &nothing };
    let one = pipe(1, false, false);
    let side = StreamSide { aig: &one, units: &[], ports: ports() };
    let s = schedule(&side, &env, &fraig::Options::default()).unwrap();
    assert_eq!((s.took.clone(), s.gave.clone()), (vec![0, 2, 4], vec![1, 3, 5]));
    assert!(s.check(&side, &env));
    let mut forged = s.clone();
    forged.gave = vec![1, 2, 3, 5];
    assert!(!forged.check(&side, &env), "the side gives nothing at clock 2");
}

#[test]
fn a_schedule_numbering_its_tokens_out_of_the_order_it_takes_or_gives_them_does_not_check() {
    let accept = |_: usize| true;
    let offer = |k: usize| k % 2 == 0;
    let env = Environment { frames: 6, offer: &offer, accept: &accept, other: &nothing };
    let one = pipe(1, false, false);
    let side = StreamSide { aig: &one, units: &[], ports: ports() };
    let s = schedule(&side, &env, &fraig::Options::default()).unwrap();
    assert!(s.check(&side, &env));
    let mut swapped = s.clone();
    swapped.took = vec![2, 0, 4];
    assert!(!swapped.check(&side, &env), "token 0 is the one taken first");
    let mut twice = s.clone();
    twice.took = vec![0, 2, 2, 4];
    assert!(!twice.check(&side, &env), "a clock takes one token");
    let mut given = s.clone();
    given.gave = vec![1, 5, 3];
    assert!(!given.check(&side, &env), "token 0 is the one given first");
}

#[test]
fn a_handshake_before_the_environment_uses_any_is_no_part_of_the_schedule() {
    // One stage whose valid register starts anywhere: watched from clock 1, it gives what it took; watched from clock 0,
    // its first valid is whatever it powered up holding.
    let mut g = pipe(1, false, false);
    let k = g.latches().iter().position(|l| l.name.as_deref() == Some("v0")).unwrap();
    let mut free = Aig::new();
    let mut map = Vec::new();
    for p in g.inputs() {
        map.push(free.add_input(p.name.clone()));
    }
    let latches: Vec<usize> = g.latches().iter().enumerate().map(|(j, l)| free.add_latch(l.name.clone(), if j == k { Init::Free } else { l.init })).collect();
    let held: Vec<AigLit> = latches.iter().map(|&j| free.latch_lit(j)).collect();
    let lits = free.import(&g, &mut |leaf| match leaf {
        crate::aig::Leaf::Input(i) => map[i],
        crate::aig::Leaf::Latch(j) => held[j],
    });
    let at = |l: AigLit| lits[l.node() as usize].with_phase(l.is_complemented());
    for (j, l) in g.latches().iter().enumerate() {
        free.set_next(latches[j], at(l.next));
    }
    for o in g.outputs() {
        free.add_output(at(o.lit), o.name.clone());
    }
    g = free;
    let side = StreamSide { aig: &g, units: &[], ports: ports() };
    let offer = |k: usize| k >= 1;
    let later = |k: usize| k >= 1;
    let env = Environment { frames: 5, offer: &offer, accept: &later, other: &nothing };
    let s = schedule(&side, &env, &fraig::Options::default()).unwrap();
    assert_eq!((s.took, s.gave), (vec![1, 2, 3, 4], vec![2, 3, 4]));
    let always = |_: usize| true;
    let watched = Environment { frames: 5, offer: &offer, accept: &always, other: &nothing };
    assert_eq!(schedule(&side, &watched, &fraig::Options::default()).unwrap_err(), StreamError::DataDependent { port: "out_valid".into(), frame: 0 });
}

#[test]
fn a_schedule_taking_a_token_where_none_was_offered_does_not_check() {
    let accept = |_: usize| true;
    let offer = |k: usize| k % 2 == 0;
    let env = Environment { frames: 6, offer: &offer, accept: &accept, other: &nothing };
    let one = pipe(1, false, false);
    let side = StreamSide { aig: &one, units: &[], ports: ports() };
    let s = schedule(&side, &env, &fraig::Options::default()).unwrap();
    let mut forged = s.clone();
    forged.took = vec![0, 1, 2, 4];
    assert!(!forged.check(&side, &env), "nothing is offered at clock 1, so nothing is taken there, however ready the side");
}

#[test]
fn the_data_is_free_in_a_schedule_whatever_the_environment_holds_the_other_inputs_at() {
    // Every other input held high: a ready that reads the data is still no constant, the data being no input the
    // environment holds.
    let high = |_: &str, _: usize| Binding::Const(true);
    let (offer, accept) = (|_: usize| true, |_: usize| true);
    let env = Environment { frames: 4, offer: &offer, accept: &accept, other: &high };
    let reading = pipe(1, false, true);
    let refused = schedule(&StreamSide { aig: &reading, units: &[], ports: ports() }, &env, &fraig::Options::default()).unwrap_err();
    assert_eq!(refused, StreamError::DataDependent { port: "in_ready".into(), frame: 0 });
}

#[test]
fn a_ready_that_falls_on_one_data_word_in_a_million_has_no_schedule() {
    // in_ready is low only when all twenty data bits are high: random simulation all but never sees it move, so it is
    // guessed high and the proof refutes the guess.
    let mut g = Aig::new();
    let valid = g.add_input(Some("in_valid".into()));
    let data: Vec<AigLit> = (0..20).map(|i| g.add_input(Some(format!("in_data[{i}]")))).collect();
    let all = data.iter().fold(AigLit::TRUE, |acc, &d| g.and(acc, d));
    g.add_output(!all, Some("in_ready".into()));
    let k = g.add_latch(Some("v".into()), Init::Zero);
    let take = g.and(valid, !all);
    g.set_next(k, take);
    let v = g.latch_lit(k);
    g.add_output(v, Some("out_valid".into()));
    let ports = Ports {
        in_valid: "in_valid".into(),
        in_ready: "in_ready".into(),
        in_data: (0..20).map(|i| format!("in_data[{i}]")).collect(),
        out_valid: "out_valid".into(),
        out_ready: None,
        out_data: vec![],
        token: &token,
    };
    let (offer, accept) = (|_: usize| true, |_: usize| true);
    let env = Environment { frames: 3, offer: &offer, accept: &accept, other: &nothing };
    let refused = schedule(&StreamSide { aig: &g, units: &[], ports }, &env, &fraig::Options::default()).unwrap_err();
    assert_eq!(refused, StreamError::DataDependent { port: "in_ready".into(), frame: 0 });
}
