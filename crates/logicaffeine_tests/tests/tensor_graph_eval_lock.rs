//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WHAT A GRAPH COMPUTES, ANSWERED WITHOUT A CHIP — AND IT IS WHAT THE VM COMPUTES, BIT FOR BIT.
//!
//! `tensor_graph::eval` is the meaning every backend is held to (benchmarks/FUSIONS_WE_NEED.md, IR-2,
//! INV-DENOTATION). Each test runs one LOGOS text twice — on the VM, and as the graph the front end lifts from it
//! through the evaluator — and compares every value the VM prints with the evaluator's, as text: Rust prints a Real as
//! the shortest digits that read back to it, so equal text is equal bits.
//!
//!   • every tiny cut — the decode step, a prefill chunk of 4, a batch of 2 and the forward of every tiny architecture —
//!     with every weight and every piece of carried state given values that are not dyadic, so a sum taken in another
//!     order rounds differently; its logits, its next token and its state after the step;
//!   • every tensor op kind, by the probe program each sugar has;
//!   • every access and destination a rewrite gives an op — a strided read, a row write, a unit write, a view, a fused
//!     symbol — computing what the ops it replaces compute apart.
//!
//!   cargo nextest run -p logicaffeine-tests --test tensor_graph_eval_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use std::collections::BTreeMap;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::model::program::{self, Cut};
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::eval::{eval, Data, EvalError};
use logicaffeine_compile::tensor_graph::graph::{Access, Dst, Fused, Op, Role, RowSel, Storage, Strided, Sym, TensorGraph};
use logicaffeine_compile::tensor_graph::symbols::ClassId;
use tensor_probes::OP_PROBES;
use tiny_models::{Tiny, ALL};

const CTX: usize = 24;

/// A value for element `i` of `name`: seeded, within ±0.2, and not a dyadic rational, so every product and sum rounds.
fn value(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(salt.wrapping_mul(40_503)) ^ (salt >> 7);
    ((h % 401) as f64 - 200.0) / 1009.0
}

/// `source` with every Real tensor it declares filled from [`value`] right after its declaration, every Int tensor
/// filled from `ints`, and every scalar in `scalars` given its value.
fn filled(source: &str, ints: &BTreeMap<String, Vec<i64>>, scalars: &BTreeMap<String, i64>) -> String {
    let mut out = String::new();
    for line in source.lines() {
        if let Some((var, _)) = line.strip_prefix("Let ").and_then(|r| r.split_once(" be ")).filter(|(_, rest)| rest.trim_end_matches('.').parse::<i64>().is_ok()) {
            if let Some(v) = scalars.get(var) {
                out.push_str(&format!("Let {var} be {v}.\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
        let Some((var, decl)) = line.strip_prefix("Let ").and_then(|r| r.split_once(" be a ")) else { continue };
        let Some((dims, kind)) = decl.split_once(" tensor of ") else { continue };
        let Some((r, c)) = dims.split_once(" by ") else { continue };
        let len = r.trim().parse::<usize>().unwrap() * c.trim().parse::<usize>().unwrap();
        if kind.starts_with("Int") {
            for (i, v) in ints.get(var).into_iter().flatten().enumerate() {
                out.push_str(&format!("Set item {} of {var} to {v}.\n", i + 1));
            }
        } else {
            for i in 0..len {
                let v = value(var, i);
                if v != 0.0 {
                    out.push_str(&format!("Set item {} of {var} to {v}.\n", i + 1));
                }
            }
        }
    }
    out
}

/// The program's VM output, line by line.
fn vm_lines(what: &str, source: &str) -> Vec<String> {
    let run = vm_outcome(source);
    assert!(run.error.is_none(), "{what}: the VM refuses the program: {:?}", run.error);
    run.output.lines().map(str::to_string).collect()
}

fn show(d: &Data) -> Vec<String> {
    match d {
        Data::Real(v) => v.iter().map(|x| format!("{x}")).collect(),
        Data::Int(v) => v.iter().map(|x| format!("{x}")).collect(),
    }
}

fn lift(source: &str) -> TensorGraph {
    let rp = recognize_stages(source).unwrap_or_else(|e| panic!("the front end refuses the program: {e:?}"));
    TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("the graph: {e}"))
}

/// Run `source` with `Show` lines for every value of `names` appended, on the VM and through the evaluator, and say
/// where they differ — past the `skip` lines the program prints itself.
fn agree(what: &str, source: &str, names: &[String]) {
    agree_after(what, source, names, 0)
}

fn agree_after(what: &str, source: &str, names: &[String], skip: usize) {
    let g = lift(source);
    let env = eval(&g, &BTreeMap::new()).unwrap_or_else(|e| panic!("{what}: the evaluator refuses: {e}"));
    let mut shown = source.to_string();
    let mut want: Vec<(String, String)> = Vec::new();
    // An arg-max binds a scalar, which the program shows whole.
    let scalars: Vec<&str> = g.ops.iter().filter(|o| o.kind() == "ArgMax").map(|o| o.out.as_str()).collect();
    for n in names {
        let values = show(env.get(n).unwrap_or_else(|| panic!("{what}: the evaluator has no `{n}`")));
        for (i, v) in values.into_iter().enumerate() {
            if scalars.contains(&n.as_str()) {
                shown.push_str(&format!("Show {n}.\n"));
            } else {
                shown.push_str(&format!("Show item {} of {n}.\n", i + 1));
            }
            want.push((format!("{n}[{i}]"), v));
        }
    }
    let printed = vm_lines(what, &shown);
    assert!(printed.len() >= skip, "{what}: the program printed {} of its own {skip} lines", printed.len());
    let got = &printed[skip..];
    assert_eq!(got.len(), want.len(), "{what}: the VM printed {} values, the evaluator has {}", got.len(), want.len());
    let differ: Vec<String> = got.iter().zip(&want).filter(|(g, (_, w))| *g != w).map(|(g, (at, w))| format!("{at}: VM {g}, evaluator {w}")).collect();
    assert!(differ.is_empty(), "{what}: {} of {} values differ:\n  {}", differ.len(), want.len(), differ.iter().take(20).cloned().collect::<Vec<_>>().join("\n  "));
}

/// A cut of a tiny architecture, its inputs filled, and the names it computes and carries: its logits, its next
/// tokens and every piece of its state.
fn cut_program(t: Tiny, cut: Cut) -> (String, Vec<String>) {
    let spec = t.spec(CTX);
    let ((source, _, _), _) = program::written(&spec, cut);
    let named = |pairs: Vec<(String, i64)>| pairs.into_iter().collect::<BTreeMap<String, i64>>();
    let (ints, scalars): (BTreeMap<String, Vec<i64>>, BTreeMap<String, i64>) = match cut {
        Cut::Decode { .. } => (BTreeMap::new(), named(vec![("tok".into(), 5), ("pos".into(), 3)])),
        Cut::Prefill { rows, .. } => ([("toks".to_string(), vec![5, 17, 2, 30][..rows].to_vec())].into(), named(vec![("pos_s0".into(), 2)])),
        Cut::Batch { rows, .. } => (
            [("toks".to_string(), (0..rows as i64).map(|b| 5 + 12 * b).collect())].into(),
            named((0..rows).map(|b| (format!("pos_s{b}"), 3 + b as i64)).collect()),
        ),
        _ => (BTreeMap::new(), named(vec![("tok".into(), 5)])),
    };
    let source = filled(&source, &ints, &scalars);
    let g = lift(&source);
    let names = g
        .tensors
        .values()
        .filter(|x| x.role == Role::State || x.name.starts_with("logits") || x.name.starts_with("next"))
        .map(|x| x.name.clone())
        .collect();
    (source, names)
}

/// Every tiny architecture's decode step, prefill chunk of 4, batch of 2 and forward, each on its own thread (the VM's
/// state is the thread's own), every disagreement reported together.
#[test]
fn the_evaluator_computes_what_the_vm_computes_for_every_tiny_cut() {
    let cuts = [Cut::Decode { ctx: CTX }, Cut::Prefill { rows: 4, ctx: CTX }, Cut::Batch { rows: 2, ctx: CTX }, Cut::Forward];
    let items: Vec<(Tiny, Cut)> = ALL.iter().flat_map(|&t| cuts.iter().map(move |&c| (t, c))).collect();
    let next = std::sync::Mutex::new(0usize);
    let failures = std::sync::Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = {
                    let mut n = next.lock().unwrap();
                    *n += 1;
                    *n - 1
                };
                let Some(&(t, cut)) = items.get(i) else { break };
                let what = format!("{} {cut:?}", t.name());
                let outcome = std::panic::catch_unwind(|| {
                    let (source, names) = cut_program(t, cut);
                    assert!(names.iter().any(|n| n.starts_with("logits")), "{what}: the cut computes logits");
                    agree(&what, &source, &names);
                });
                if let Err(e) = outcome {
                    let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    failures.lock().unwrap().push(msg);
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{} of {} tiny cuts disagree with the VM:\n{}", failures.len(), items.len(), failures.join("\n\n"));
}

/// Every tensor op kind, by its sugar's probe program: the result of the op the probe lifts, element for element.
#[test]
fn every_op_kind_has_an_evaluation_or_a_named_refusal() {
    let mut kinds = std::collections::BTreeSet::new();
    for p in OP_PROBES {
        let g = lift(p.program);
        let results: Vec<String> = g.ops.iter().map(|o| o.out.clone()).collect();
        for op in &g.ops {
            kinds.insert(op.kind().to_string());
        }
        agree_after(&format!("probe {}", p.op), p.program, &results, p.shows);
    }
    for kind in [
        "Matmul", "Elementwise", "SwiGlu", "FusedRow", "Rope", "Silu", "Sigmoid", "Gelu", "Scale", "Decay", "L2Norm", "Transpose", "HeadSlice", "Join",
        "HeadMatmul", "CausalConv", "GatedDelta", "Attention", "FullAttention", "Moe", "RowGather", "RowsGather", "ArgMax", "SetRow", "IntRequant",
        "IntClamp", "Relu", "LutAttention",
    ] {
        assert!(kinds.contains(kind), "no probe lifts a `{kind}`, so its evaluation is unchecked: {kinds:?}");
    }
}

// ── Accesses and destinations ──────────────────────────────────────────────────────────────────────

fn real(len: usize, name: &str) -> Data {
    Data::Real((0..len).map(|i| value(name, i)).collect())
}

/// The lifted ops of `source` with `rewrite` applied, evaluated, against the lifted ops evaluated apart.
fn rewrites_alike(what: &str, source: &str, rewrite: impl Fn(&mut TensorGraph), compare: &[&str]) {
    let apart = lift(source);
    let mut rewritten = apart.clone();
    rewrite(&mut rewritten);
    let (a, b) = (eval(&apart, &BTreeMap::new()).unwrap(), eval(&rewritten, &BTreeMap::new()).unwrap_or_else(|e| panic!("{what}: {e}")));
    for n in compare {
        assert_eq!(a.get(*n), b.get(*n), "{what}: `{n}` apart and rewritten");
    }
}

fn op_of<'g>(g: &'g TensorGraph, out: &str) -> usize {
    g.ops.iter().position(|o| o.out == out).unwrap_or_else(|| panic!("no op computes `{out}`"))
}

#[test]
fn access_and_destination_mean_what_they_say() {
    // A strided read: the rotation reads every head's last 8 of 16 values where they lie, the slice no op — the last
    // unit included.
    let sliced = "## Main\nLet X be a 1 by 64 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\n\
                  Let P be the last 8 of every 16 of X.\nLet Y be rope of P with cos RC and sin RS.\n";
    let source = filled(sliced, &BTreeMap::new(), &BTreeMap::new());
    rewrites_alike(
        "strided read",
        &source,
        |g| {
            let s = op_of(g, "P");
            let r = op_of(g, "Y");
            g.ops[r].ins[0] = "X".into();
            g.ops[r].access[0] = Access::Strided(Strided { pitch: 16, offset: 8, width: 8 });
            g.ops[r].covers.push(s);
            g.ops.remove(s);
        },
        &["Y"],
    );
    // A row write: the product lands straight in the table's row the position names — the first row and the last.
    for (pos, label) in [(1, "the first row"), (4, "the last row")] {
        let written = format!(
            "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 8 by 8 tensor of Real.\nLet T be a 4 by 8 tensor of Real.\nLet pos be {pos}.\n\
             Let K be X matmul W.\nSet row pos of T to K.\n"
        );
        let source = filled(&written, &BTreeMap::new(), &BTreeMap::new());
        rewrites_alike(
            &format!("row write at {label}"),
            &source,
            |g| {
                let k = op_of(g, "K");
                let set = g.ops.iter().position(|o| o.kind() == "SetRow").unwrap();
                g.ops[k].dst = Dst { buf: "T".into(), row: Some((RowSel::Param("pos".into()), 8)), unit: None };
                g.ops[k].covers.push(set);
                g.ops.remove(set);
            },
            &["T"],
        );
    }
    // A unit write: each half of a join lands in its part of every unit of the joined tensor.
    let joined = "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 4 tensor of Real.\nLet SA be A scaled by 2.0.\nLet SB be B scaled by 3.0.\n\
                  Let J be SA joined with SB across 4 heads.\n";
    let source = filled(joined, &BTreeMap::new(), &BTreeMap::new());
    rewrites_alike(
        "unit write",
        &source,
        |g| {
            let (a, b, j) = (op_of(g, "SA"), op_of(g, "SB"), op_of(g, "J"));
            let unit = |offset, width| Some(Strided { pitch: 3, offset, width });
            g.ops[a].dst = Dst { buf: "J".into(), row: None, unit: unit(0, 2) };
            g.ops[b].dst = Dst { buf: "J".into(), row: None, unit: unit(2, 1) };
            g.ops[b].covers.push(j);
            g.ops.remove(j);
        },
        &["J"],
    );
    // A fused symbol: an add then a norm as one op keeping the sum; its intermediates do not outlive it.
    let added = "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\n\
                 Let S be A plus B.\nLet Y be rmsnorm of S with weight W.\n";
    let source = filled(added, &BTreeMap::new(), &BTreeMap::new());
    rewrites_alike(
        "fused symbol",
        &source,
        |g| {
            let parts = g.ops.clone();
            g.ops = vec![Op {
                sym: Sym::Fused(Fused { class: ClassId::AddNorm, parts }),
                ins: vec!["A".into(), "B".into(), "W".into()],
                access: vec![Access::Whole; 3],
                out: "Y".into(),
                dst: Dst::to("Y"),
                keeps: vec!["S".into()],
                covers: vec![0, 1],
            }];
        },
        &["Y", "S"],
    );
    let g = {
        let mut g = lift(&source);
        let parts = g.ops.clone();
        g.ops = vec![Op {
            sym: Sym::Fused(Fused { class: ClassId::AddNorm, parts }),
            ins: vec!["A".into(), "B".into(), "W".into()],
            access: vec![Access::Whole; 3],
            out: "Y".into(),
            dst: Dst::to("Y"),
            keeps: Vec::new(),
            covers: vec![0, 1],
        }];
        g
    };
    let env = eval(&g, &BTreeMap::new()).unwrap();
    assert!(env.contains_key("Y") && !env.contains_key("S"), "a fused op's intermediate it does not keep does not outlive it");
}

/// The state a step writes is the state the next step reads: two decode steps of the hybrid, the second over the first's
/// caches, rings and memories, alike on the VM and the evaluator.
#[test]
fn state_carries_from_one_step_to_the_next() {
    let (source, names) = cut_program(Tiny::Qwen35, Cut::Decode { ctx: CTX });
    let g = lift(&source);
    let first = eval(&g, &BTreeMap::new()).unwrap();
    let state: BTreeMap<String, Data> = g.of_role(Role::State).map(|t| (t.name.clone(), first[&t.name].clone())).collect();
    // The second step: the same program, its state declared with the first step's values, at the next position.
    let mut second = String::new();
    for line in source.lines() {
        let declared = line.strip_prefix("Set item ").and_then(|r| r.split_once(" of ")).map(|(_, r)| r.split_once(" to ").map_or(r, |(t, _)| t));
        if declared.is_some_and(|t| state.contains_key(t)) {
            continue;
        }
        if line == "Let pos be 3." {
            second.push_str("Let pos be 4.\n");
            continue;
        }
        second.push_str(line);
        second.push('\n');
        if let Some((var, _)) = line.strip_prefix("Let ").and_then(|r| r.split_once(" be a ")) {
            if let Some(Data::Real(v)) = state.get(var) {
                for (i, x) in v.iter().enumerate() {
                    if *x != 0.0 {
                        second.push_str(&format!("Set item {} of {var} to {x}.\n", i + 1));
                    }
                }
            }
        }
    }
    let at_next = source.replace("Let pos be 3.\n", "Let pos be 4.\n");
    let by_inputs = eval(&lift(&second), &BTreeMap::new()).unwrap();
    let carried = eval(&lift(&at_next), &state).unwrap();
    for n in &names {
        assert_eq!(by_inputs.get(n), carried.get(n), "`{n}`: the second step over the first's state, declared or carried");
    }
    agree("the second decode step", &second, &names);
}

/// A window of one position attends only the newest; a window as long as the cache attends it all — each as the VM does.
#[test]
fn windows_of_one_position_and_of_the_whole_cache_attend_as_the_vm_does() {
    for w in [1usize, 4] {
        let src = format!(
            "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet pos be 4.\n\
             Let C be attention of Q over K and V with 2 heads 1 groups up to pos within a window of {w}.\n"
        );
        let source = filled(&src, &BTreeMap::new(), &BTreeMap::new());
        agree(&format!("a window of {w}"), &source, &["C".to_string()]);
    }
}

/// A Real `-0.0` stays negative through a copy and gives way to `0.0` in a ReLU's `max`; a NaN propagates through a
/// product; an integer past 64 bits is refused by name.
#[test]
fn signed_zeros_nans_and_integer_overflow_mean_what_the_language_says() {
    let src = "## Main\nLet X be a 1 by 2 tensor of Real.\nSet item 1 of X to -0.0.\nSet item 2 of X to 1.5.\nLet Y be relu of X.\n";
    agree("relu of -0.0", src, &["Y".to_string()]);
    let g = lift("## Main\nLet A be a 1 by 2 tensor of Real.\nLet B be a 2 by 1 tensor of Real.\nLet C be A matmul B.\n");
    let inputs = BTreeMap::from([("A".to_string(), Data::Real(vec![f64::NAN, 1.0])), ("B".to_string(), real(2, "B"))]);
    let env = eval(&g, &inputs).unwrap();
    assert!(matches!(&env["C"], Data::Real(v) if v[0].is_nan()), "a NaN propagates through a product");
    let g = lift("## Main\nLet A be a 1 by 2 tensor of Int.\nSet item 1 of A to 4611686018427387904.\nSet item 2 of A to 3.\nLet Y be A times 4 shifted right by 2.\n");
    assert!(matches!(eval(&g, &BTreeMap::new()), Err(EvalError::Overflow { .. })), "an Int past 64 bits is refused by name");
}
