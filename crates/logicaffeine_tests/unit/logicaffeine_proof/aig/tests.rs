use super::*;

fn aig(text: &str) -> Aig {
    Aig::parse(text.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn outputs(g: &Aig, inputs: &[bool], latches: &[bool]) -> Vec<bool> {
    let v = g.eval(inputs, latches);
    g.outputs().iter().map(|o| Aig::value(&v, o.lit)).collect()
}

const HALF_ADDER: &str = "aag 7 2 0 2 3\n2\n4\n6\n12\n6 13 15\n12 2 4\n14 3 5\ni0 x\ni1 y\no0 s\no1 c\nc\nhalf adder\n";

#[test]
fn literals_carry_their_node_and_phase() {
    let l = AigLit::new(5, true);
    assert_eq!((l.node(), l.is_complemented(), l.code()), (5, true, 11));
    assert_eq!(!l, AigLit::new(5, false));
    assert_eq!(AigLit::from_code(11), l);
    assert_eq!(l.with_phase(true), !l);
    assert_eq!(l.with_phase(false), l);
    assert_eq!(!AigLit::FALSE, AigLit::TRUE);
}

#[test]
fn and_folds_constants_and_repeats() {
    let mut g = Aig::new();
    let a = g.add_input(Some("a".into()));
    let b = g.add_input(Some("b".into()));
    assert_eq!(g.and(a, AigLit::FALSE), AigLit::FALSE);
    assert_eq!(g.and(AigLit::TRUE, b), b);
    assert_eq!(g.and(a, a), a);
    assert_eq!(g.and(a, !a), AigLit::FALSE);
    assert_eq!(g.num_ands(), 0);
    let ab = g.and(a, b);
    assert_eq!(g.and(b, a), ab, "structurally hashed regardless of operand order");
    assert_eq!(g.num_ands(), 1);
    let nab = g.and(!a, b);
    assert_ne!(nab, ab);
    assert_eq!(g.num_ands(), 2);
}

#[test]
fn or_xor_and_mux_compute_their_truth_tables() {
    let mut g = Aig::new();
    let s = g.add_input(None);
    let a = g.add_input(None);
    let b = g.add_input(None);
    let or = g.or(a, b);
    let xor = g.xor(a, b);
    let xnor = g.xnor(a, b);
    let mux = g.mux(s, a, b);
    for o in [or, xor, xnor, mux] {
        g.add_output(o, None);
    }
    for bits in 0..8u32 {
        let (vs, va, vb) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0);
        let got = outputs(&g, &[vs, va, vb], &[]);
        assert_eq!(got, vec![va || vb, va ^ vb, va == vb, if vs { va } else { vb }], "{bits:03b}");
    }
}

#[test]
fn the_aiger_specification_examples_parse() {
    let empty = aig("aag 0 0 0 0 0\n");
    assert_eq!((empty.inputs().len(), empty.outputs().len()), (0, 0));
    let constant = aig("aag 0 0 0 1 0\n0\n");
    assert_eq!(constant.outputs()[0].lit, AigLit::FALSE);
    let buffer = aig("aag 1 1 0 1 0\n2\n2\n");
    assert_eq!(outputs(&buffer, &[true], &[]), vec![true]);
    let inverter = aig("aag 1 1 0 1 0\n2\n3\n");
    assert_eq!(outputs(&inverter, &[true], &[]), vec![false]);
    let and = aig("aag 3 2 0 1 1\n2\n4\n6\n6 2 4\n");
    let or = aig("aag 3 2 0 1 1\n2\n4\n7\n6 3 5\n");
    for bits in 0..4u32 {
        let (x, y) = (bits & 1 != 0, bits & 2 != 0);
        assert_eq!(outputs(&and, &[x, y], &[]), vec![x && y]);
        assert_eq!(outputs(&or, &[x, y], &[]), vec![x || y]);
    }
}

#[test]
fn a_half_adder_defined_out_of_order_parses_with_its_names_and_comments() {
    let g = aig(HALF_ADDER);
    assert_eq!(g.inputs().iter().map(|p| p.name.clone().unwrap()).collect::<Vec<_>>(), ["x", "y"]);
    assert_eq!(g.outputs().iter().map(|p| p.name.clone().unwrap()).collect::<Vec<_>>(), ["s", "c"]);
    assert_eq!(g.comments(), ["half adder"]);
    assert_eq!(g.input_index("y"), Some(1));
    assert_eq!(g.output_index("c"), Some(1));
    assert_eq!(g.output_index("z"), None);
    for bits in 0..4u32 {
        let (x, y) = (bits & 1 != 0, bits & 2 != 0);
        assert_eq!(outputs(&g, &[x, y], &[]), vec![x ^ y, x && y]);
    }
}

#[test]
fn the_canonical_text_lists_inputs_latches_then_gates_in_order() {
    let g = aig(HALF_ADDER);
    assert_eq!(
        g.to_ascii(),
        "aag 5 2 0 2 3\n2\n4\n10\n6\n6 4 2\n8 5 3\n10 9 7\ni0 x\ni1 y\no0 s\no1 c\nc\nhalf adder\n"
    );
}

#[test]
fn a_toggle_flip_flop_keeps_its_latch() {
    let g = aig("aag 1 0 1 2 0\n2 3\n2\n3\n");
    assert_eq!(g.latches().len(), 1);
    let latch = &g.latches()[0];
    assert_eq!(latch.init, Init::Zero);
    assert_eq!(latch.next, !AigLit::new(latch.node, false));
    assert_eq!(outputs(&g, &[], &[true]), vec![true, false]);
    assert_eq!(g.to_ascii(), "aag 1 0 1 2 0\n2 3\n2\n3\n");
}

#[test]
fn a_latch_reads_its_initial_value() {
    let init = |line: &str| aig(&format!("aag 1 0 1 0 0\n{line}\n")).latches()[0].init;
    assert_eq!(init("2 3"), Init::Zero);
    assert_eq!(init("2 3 0"), Init::Zero);
    assert_eq!(init("2 3 1"), Init::One);
    assert_eq!(init("2 3 2"), Init::Free);
    assert!(Aig::parse(b"aag 1 0 1 0 0\n2 3 4\n").is_err(), "an initial value is 0, 1 or the latch itself");
    for i in [Init::Zero, Init::One, Init::Free] {
        let mut g = Aig::new();
        let l = g.add_latch(Some("q".into()), i);
        let q = g.latch_lit(l);
        g.set_next(l, !q);
        g.add_output(q, Some("q".into()));
        let back = aig(&g.to_ascii());
        assert_eq!(back.latches()[0].init, i);
        assert_eq!(back.to_ascii(), g.to_ascii());
        assert_eq!(Aig::parse(&g.to_binary()).unwrap().to_ascii(), g.to_ascii());
    }
}

#[test]
fn the_binary_format_reads_and_writes_the_same_graph() {
    let and = Aig::parse(b"aig 3 2 0 1 1\n6\n\x02\x02").unwrap();
    for bits in 0..4u32 {
        let (x, y) = (bits & 1 != 0, bits & 2 != 0);
        assert_eq!(outputs(&and, &[x, y], &[]), vec![x && y]);
    }
    let g = aig(HALF_ADDER);
    let bin = g.to_binary();
    assert!(bin.starts_with(b"aig 5 2 0 2 3\n"));
    let back = Aig::parse(&bin).unwrap();
    assert_eq!(back.to_ascii(), g.to_ascii());
}

#[test]
fn large_deltas_take_several_bytes() {
    let mut g = Aig::new();
    let inputs: Vec<AigLit> = (0..300).map(|_| g.add_input(None)).collect();
    let mut acc = inputs[0];
    for &x in &inputs[1..] {
        acc = g.and(acc, x);
    }
    let far = g.and(inputs[0], acc);
    g.add_output(far, None);
    let back = Aig::parse(&g.to_binary()).unwrap();
    assert_eq!(back.to_ascii(), g.to_ascii());
}

#[test]
fn bad_states_and_constraints_round_trip() {
    let text = "aag 3 2 0 0 1 1 1\n2\n4\n6\n3\n6 2 4\nb0 both\nc0 first\n";
    let g = aig(text);
    assert_eq!(g.bad().len(), 1);
    assert_eq!(g.constraints().len(), 1);
    assert_eq!(g.bad()[0].name.as_deref(), Some("both"));
    assert_eq!(g.constraints()[0].name.as_deref(), Some("first"));
    assert_eq!(aig(&g.to_ascii()).to_ascii(), g.to_ascii());
    assert_eq!(Aig::parse(&g.to_binary()).unwrap().to_ascii(), g.to_ascii());
}

#[test]
fn a_symbol_name_keeps_its_spaces() {
    let g = aig("aag 1 1 0 1 0\n2\n2\ni0 the input\no0 out put\n");
    assert_eq!(g.inputs()[0].name.as_deref(), Some("the input"));
    assert_eq!(g.outputs()[0].name.as_deref(), Some("out put"));
}

#[test]
fn malformed_files_are_refused_by_name() {
    let refused = |text: &[u8]| Aig::parse(text).expect_err("refused");
    assert!(matches!(refused(b"aax 0 0 0 0 0\n"), AigerError::Header(_)));
    assert!(matches!(refused(b"aag 1 1 0 1\n2\n2\n"), AigerError::Header(_)));
    assert!(matches!(refused(b"aag 1 1 0 1 0\n3\n2\n"), AigerError::Literal { .. }), "an input is an even literal");
    assert!(matches!(refused(b"aag 1 1 0 1 0\n2\n9\n"), AigerError::Literal { .. }), "past 2M + 1");
    assert!(matches!(refused(b"aag 2 1 0 1 0\n2\n4\n"), AigerError::Undefined { .. }));
    assert!(matches!(refused(b"aag 1 2 0 0 0\n2\n2\n"), AigerError::Redefined { .. }));
    assert!(matches!(refused(b"aag 1 0 0 1 1\n2\n2 2 2\n"), AigerError::Cycle { .. }));
    assert!(matches!(refused(b"aag 2 0 0 1 2\n2\n2 4 4\n4 2 2\n"), AigerError::Cycle { .. }));
    assert!(matches!(refused(b"aag 1 1 0 0 0 0 0 1 0\n2\n1\n2\n"), AigerError::Unsupported(_)));
    assert!(matches!(refused(b"aig 4 2 0 1 1\n6\n\x02\x02"), AigerError::Header(_)), "binary M = I + L + A");
    assert!(matches!(refused(b"aig 3 2 0 1 1\n6\n\x02"), AigerError::Truncated));
    assert!(matches!(refused(b"aag 1 1 0 1 0\n2\n"), AigerError::Truncated));
}

#[test]
fn a_deep_chain_written_backwards_parses_without_recursion() {
    let depth = 200_000u32;
    let m = depth + 1;
    let mut text = format!("aag {m} 1 0 1 {depth}\n2\n{}\n", 2 * m);
    for k in (2..=m).rev() {
        text.push_str(&format!("{} {} 2\n", 2 * k, 2 * (k - 1)));
    }
    let g = aig(&text);
    assert_eq!(g.num_ands(), 0, "x ∧ x folds to x at every depth");
    assert_eq!(outputs(&g, &[true], &[]), vec![true]);
    // var 3 = y ∧ x, then var k = var (k − 1) ∧ x: one gate a line, nothing folds.
    let mut chain = format!("aag {} 2 0 1 {depth}\n2\n4\n{}\n", depth + 2, 2 * (depth + 2));
    for k in (3..=depth + 2).rev() {
        chain.push_str(&format!("{} {} 2\n", 2 * k, 2 * (k - 1)));
    }
    let g = aig(&chain);
    assert_eq!(g.num_ands(), depth as usize);
    assert_eq!(outputs(&g, &[true, true], &[]), vec![true]);
    assert_eq!(outputs(&g, &[true, false], &[]), vec![false]);
}

#[test]
fn simulation_agrees_with_evaluation() {
    let g = aig(HALF_ADDER);
    let words = 2;
    let pattern = [0x0123_4567_89ab_cdefu64, 0xfedc_ba98_7654_3210, 0x0f0f_f0f0_3c3c_c3c3, 0xaaaa_5555_cccc_3333];
    let sim = g.simulate(words, &pattern, &[]);
    for w in 0..words {
        for bit in 0..64 {
            let ins: Vec<bool> = (0..2).map(|i| pattern[i * words + w] >> bit & 1 == 1).collect();
            let want = outputs(&g, &ins, &[]);
            for (k, o) in g.outputs().iter().enumerate() {
                let word = Aig::word(&sim, words, o.lit, w);
                assert_eq!(word >> bit & 1 == 1, want[k], "output {k} word {w} bit {bit}");
            }
        }
    }
}

#[test]
fn a_cone_holds_exactly_what_its_roots_reach() {
    let mut g = Aig::new();
    let a = g.add_input(None);
    let b = g.add_input(None);
    let c = g.add_input(None);
    let ab = g.and(a, b);
    let bc = g.and(b, c);
    let top = g.and(ab, !c);
    let cone = g.cone(&[top]);
    assert_eq!(cone, vec![a.node(), b.node(), c.node(), ab.node(), top.node()]);
    assert!(!cone.contains(&bc.node()));
    assert_eq!(g.cone(&[AigLit::TRUE]), vec![0]);
}

#[test]
fn importing_a_graph_maps_its_leaves() {
    let src = aig(HALF_ADDER);
    let mut dst = Aig::new();
    let p = dst.add_input(Some("p".into()));
    let q = dst.add_input(Some("q".into()));
    let map = dst.import(&src, &mut |leaf| match leaf {
        Leaf::Input(0) => q,
        Leaf::Input(_) => !p,
        Leaf::Latch(_) => unreachable!(),
    });
    for o in src.outputs() {
        let lit = map[o.lit.node() as usize].with_phase(o.lit.is_complemented());
        dst.add_output(lit, o.name.clone());
    }
    for bits in 0..4u32 {
        let (vp, vq) = (bits & 1 != 0, bits & 2 != 0);
        assert_eq!(outputs(&dst, &[vp, vq], &[]), outputs(&src, &[vq, !vp], &[]));
    }
}

#[test]
fn compacting_keeps_what_the_outputs_latches_and_properties_read_and_drops_the_rest() {
    let mut g = Aig::new();
    let (a, b, c) = (g.add_input(Some("a".into())), g.add_input(Some("b".into())), g.add_input(None));
    let dead = g.and(a, b);
    let _deader = g.xor(dead, c);
    let q = g.add_latch(Some("q".into()), Init::Zero);
    let qn = g.latch_lit(q);
    let next = g.and(qn, c);
    g.set_next(q, next);
    let y = g.xor(a, qn);
    g.add_output(y, Some("y".into()));
    let bad = g.and(b, !c);
    g.add_bad(bad, Some("never".into()));
    let (small, map) = g.compact();
    assert!(small.num_ands() < g.num_ands(), "{} of {}", small.num_ands(), g.num_ands());
    assert_eq!(map[dead.node() as usize], None, "nothing reads it");
    assert!(map[y.node() as usize].is_some());
    assert_eq!(small.inputs().iter().map(|p| p.name.clone()).collect::<Vec<_>>(), [Some("a".into()), Some("b".into()), None], "every input kept, in order");
    assert_eq!(small.latches().len(), 1);
    assert_eq!((small.outputs().len(), small.bad().len()), (1, 1));
    for bits in 0..16u32 {
        let inputs: Vec<bool> = (0..3).map(|i| bits >> i & 1 == 1).collect();
        let latches = [bits >> 3 & 1 == 1];
        let (x, y) = (g.eval(&inputs, &latches), small.eval(&inputs, &latches));
        let read = |g: &Aig, v: &[bool]| -> Vec<bool> {
            g.outputs().iter().chain(g.bad()).map(|p| Aig::value(v, p.lit)).chain(g.latches().iter().map(|l| Aig::value(v, l.next))).collect()
        };
        assert_eq!(read(&g, &x), read(&small, &y), "inputs {inputs:?}, latch {latches:?}");
    }
}

/// A handshake `ready` read through two control registers (`busy` ← `armed` ← `go`), beside a data register `d` that
/// takes `x` and feeds an output `y` of its own and a register `e` that follows it: the data never reaches `ready`.
fn control_beside_data() -> Aig {
    let mut g = Aig::new();
    let (go, x) = (g.add_input(Some("go".into())), g.add_input(Some("x".into())));
    let armed = g.add_latch(Some("armed".into()), Init::Zero);
    let busy = g.add_latch(Some("busy".into()), Init::Free);
    let d = g.add_latch(Some("d".into()), Init::Free);
    let e = g.add_latch(Some("e".into()), Init::Zero);
    let (al, bl, dl, el) = (g.latch_lit(armed), g.latch_lit(busy), g.latch_lit(d), g.latch_lit(e));
    g.set_next(armed, go);
    let b_next = g.or(al, bl);
    g.set_next(busy, b_next);
    let d_next = g.xor(x, dl);
    g.set_next(d, d_next);
    g.set_next(e, dl);
    g.add_output(!bl, Some("ready".into()));
    let y = g.and(dl, el);
    g.add_output(y, Some("y".into()));
    g
}

#[test]
fn a_sequential_cone_keeps_the_latches_an_output_reads_through_any_clocks_and_drops_the_rest() {
    let g = control_beside_data();
    let ready = g.output_index("ready").unwrap();
    let cone = g.sequential_cone(&[ready]);
    assert_eq!(cone.latches().iter().map(|l| l.name.clone().unwrap()).collect::<Vec<_>>(), ["armed", "busy"], "the control registers, two clocks back, in order");
    assert_eq!(cone.latches().iter().map(|l| l.init).collect::<Vec<_>>(), [Init::Zero, Init::Free]);
    assert_eq!(cone.outputs().iter().map(|p| p.name.clone().unwrap()).collect::<Vec<_>>(), ["ready"]);
    assert_eq!(cone.inputs().iter().map(|p| p.name.clone()).collect::<Vec<_>>(), [Some("go".into()), Some("x".into())], "every input kept, in order");
    // Run from every start the kept latches can take, every input sequence of four clocks: ready is the same.
    for start in 0..4u32 {
        for seq in 0..256u32 {
            // armed from 0, busy and d from anything, e from 0; the cone's armed and busy alike.
            let (mut full, mut small) = (vec![false, start & 1 == 1, start & 2 == 2, false], vec![false, start & 1 == 1]);
            for k in 0..4 {
                let inputs = [seq >> (2 * k) & 1 == 1, seq >> (2 * k + 1) & 1 == 1];
                let (fv, sv) = (g.eval(&inputs, &full), cone.eval(&inputs, &small));
                assert_eq!(Aig::value(&fv, g.outputs()[ready].lit), Aig::value(&sv, cone.outputs()[0].lit), "start {start}, inputs {seq:08b}, clock {k}");
                full = g.latches().iter().map(|l| Aig::value(&fv, l.next)).collect();
                small = cone.latches().iter().map(|l| Aig::value(&sv, l.next)).collect();
            }
        }
    }
    let y = g.output_index("y").unwrap();
    let data = g.sequential_cone(&[y]);
    assert_eq!(data.latches().iter().map(|l| l.name.clone().unwrap()).collect::<Vec<_>>(), ["d", "e"]);
    let both = g.sequential_cone(&[y, ready]);
    assert_eq!(both.latches().len(), 4);
    assert_eq!(both.outputs().iter().map(|p| p.name.clone().unwrap()).collect::<Vec<_>>(), ["y", "ready"], "the outputs in the order asked");
}
