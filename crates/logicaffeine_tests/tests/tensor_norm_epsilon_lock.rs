//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A NORM TAKES THE EPSILON THE PROGRAM GIVES IT.
//!
//! `rmsnorm … with weight W`, `layernorm … with weight W and bias B` and `l2norm …` divide by `sqrt(… + eps)`; a model
//! fixes its own `eps` (Qwen 1e-6, Llama 3 and gpt-oss 1e-5), so the sugar takes `with epsilon E` after its operands,
//! and the default stays 1e-6. The inputs here are small enough that the two epsilons give different outputs, so a
//! tier that dropped the clause would show it.
//!
//!   • the VM computes the formula with the program's epsilon, and the tree-walker agrees with it;
//!   • the tensor front end lifts the epsilon into the op's spec (`FusedRowSpec::eps`, `L2NormSpec::eps`);
//!   • an epsilon that is not a positive number is refused by name;
//!   • the accelerators that lower a recognized norm emit the program's epsilon, not a constant of their own — the
//!     spatial IR in its `Rsqrt` transform's nano units, refusing an epsilon finer than a nano;
//!   • the gated delta rule's output norm takes the clause too.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_cuda_forward, compile_to_cuda_tensor_op, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_to_spatial};
use logicaffeine_compile::spatial::target::SpatialTarget;

const XS: [f64; 4] = [0.001, -0.002, 0.003, 0.004];
const WS: [f64; 2] = [1.5, -0.5];

/// The program: `X` (2 by 2, the values above) and `W` (length 2) declared, `Y` bound by `op`, every entry of `Y` shown
/// as `round(y·1e9)`.
fn program(op: &str) -> String {
    let mut s = String::from("## Main\nLet X be a 2 by 2 tensor of Real.\n");
    for (i, x) in XS.iter().enumerate() {
        s.push_str(&format!("Set item {} of X to {x}.\n", i + 1));
    }
    s.push_str("Let R be a 2 by 2 tensor of Real.\n");
    for i in 0..4 {
        s.push_str(&format!("Set item {} of R to {}.\n", i + 1, 0.001 * (i as f64 + 1.0)));
    }
    s.push_str("Let W be a 1 by 2 tensor of Real.\nLet B be a 1 by 2 tensor of Real.\n");
    for (i, w) in WS.iter().enumerate() {
        s.push_str(&format!("Set item {} of W to {w}.\nSet item {} of B to {}.\n", i + 1, i + 1, 0.25 * (i as f64 + 1.0)));
    }
    s.push_str(&format!("Let Y be {op}.\n"));
    for i in 1..=4 {
        s.push_str(&format!("Show round(item {i} of Y * 1000000000).\n"));
    }
    s
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

fn round9(v: f64) -> i64 {
    (v * 1e9).round() as i64
}

/// Per-row weighted RMSNorm of `X (+ R)` over rows of `cols`, the weight repeating every `cols`.
fn rmsnorm(cols: usize, residual: bool, eps: f64) -> Vec<i64> {
    let r: Vec<f64> = (0..4).map(|i| 0.001 * (i as f64 + 1.0)).collect();
    let v: Vec<f64> = XS.iter().zip(&r).map(|(x, r)| if residual { x + r } else { *x }).collect();
    v.chunks(cols)
        .flat_map(|row| {
            let den = (row.iter().map(|x| x * x).sum::<f64>() / cols as f64 + eps).sqrt();
            row.iter().enumerate().map(move |(i, x)| round9(x / den * WS[i % 2])).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn an_rmsnorm_takes_the_programs_epsilon() {
    let given = shown(&program("rmsnorm of X with weight W with epsilon 0.00001"));
    assert_eq!(given, rmsnorm(2, false, 1e-5));
    assert_ne!(given, rmsnorm(2, false, 1e-6), "the epsilon changes these outputs");
    assert_eq!(shown(&program("rmsnorm of X with weight W")), rmsnorm(2, false, 1e-6), "the default is 1e-6");
    assert_eq!(shown(&program("rmsnorm of X plus R with weight W with epsilon 0.00001")), rmsnorm(2, true, 1e-5));
    assert_eq!(shown(&program("rmsnorm of X with weight W across 2 heads with epsilon 0.00001")), rmsnorm(2, false, 1e-5));
    assert_eq!(shown(&program("rmsnorm of X with weight W with epsilon 1e-5")), rmsnorm(2, false, 1e-5), "scientific notation");
}

#[test]
fn a_layernorm_and_an_l2norm_take_the_programs_epsilon() {
    let layer = |eps: f64| -> Vec<i64> {
        XS.chunks(2)
            .flat_map(|row| {
                let mean = row.iter().sum::<f64>() / 2.0;
                let var = row.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / 2.0;
                let den = (var + eps).sqrt();
                row.iter().enumerate().map(move |(i, x)| round9((x - mean) / den * WS[i] + 0.25 * (i as f64 + 1.0))).collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(shown(&program("layernorm of X with weight W and bias B with epsilon 0.00001")), layer(1e-5));
    assert_ne!(layer(1e-5), layer(1e-6));
    let l2 = |eps: f64| -> Vec<i64> {
        XS.chunks(2)
            .flat_map(|row| {
                let den = (row.iter().map(|x| x * x).sum::<f64>() + eps).sqrt();
                row.iter().map(move |x| round9(x / den)).collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(shown(&program("l2norm of X across 2 heads with epsilon 0.00001")), l2(1e-5));
    assert_ne!(l2(1e-5), l2(1e-6));
}

#[test]
fn the_front_end_lifts_the_epsilon_into_the_spec() {
    let lifted = |op: &str| {
        let rp = recognize_stages(&program(op)).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{op}: {:?}", rp.unrecognized);
        rp.stages.into_iter().next().expect("one op").op
    };
    for (op, eps) in [
        ("rmsnorm of X with weight W with epsilon 0.00001", 1e-5),
        ("rmsnorm of X plus R with weight W with epsilon 0.00002", 2e-5),
        ("rmsnorm of X with weight W", 1e-6),
        ("layernorm of X with weight W and bias B with epsilon 0.00001", 1e-5),
    ] {
        match lifted(op) {
            TensorOpSpec::FusedRow(r) => assert_eq!(r.eps.get(), eps, "{op}"),
            other => panic!("{op} lifts as a fused row, not {other:?}"),
        }
    }
    for (op, eps) in [("l2norm of X across 2 heads with epsilon 0.00001", 1e-5), ("l2norm of X across 2 heads", 1e-6)] {
        match lifted(op) {
            TensorOpSpec::L2Norm(l) => assert_eq!(l.eps.get(), eps, "{op}"),
            other => panic!("{op} lifts as an L2 norm, not {other:?}"),
        }
    }
}

#[test]
fn an_epsilon_that_is_not_a_positive_number_is_refused() {
    for (op, why) in [
        ("rmsnorm of X with weight W with epsilon", "epsilon"),
        ("rmsnorm of X with weight W with epsilon W", "epsilon"),
        ("rmsnorm of X with weight W with epsilon 0", "positive"),
        ("rmsnorm of X with weight W with epsilon -0.001", "positive"),
        ("l2norm of X with epsilon 0.0", "positive"),
    ] {
        let vm = vm_outcome(&program(op));
        let err = vm.error.unwrap_or_else(|| panic!("`{op}` is refused"));
        assert!(err.to_lowercase().contains(why), "`{op}` is refused naming its {why}: {err}");
    }
}

#[test]
fn the_accelerators_emit_the_programs_epsilon() {
    let src = program("rmsnorm of X with weight W with epsilon 0.00001");
    let kernel = compile_to_cuda_tensor_op(&src).expect("the CUDA tensor-op tier lowers the norm");
    assert!(kernel.contains("1e-5") || kernel.contains("1e-05") || kernel.contains("0.00001"), "the CUDA kernel's epsilon:\n{kernel}");
    assert!(!kernel.contains("1e-6"), "no epsilon of the kernel's own:\n{kernel}");
    let forward = compile_to_cuda_forward(&src).expect("the CUDA forward lowers the norm");
    assert!(forward.contains("1e-5") || forward.contains("1e-05") || forward.contains("0.00001"), "the CUDA forward's epsilon");
    assert!(!forward.contains("+1e-6") && !forward.contains("+ 1e-6"), "no epsilon of the forward's own");
}

#[test]
fn the_spatial_ir_carries_the_programs_epsilon() {
    let lowering = |op: &str| format!("{:?}", compile_program_to_spatial(&program(op), &SpatialTarget::wormhole()).expect("a tensor program"));
    let lowered = |op: &str| {
        let l = lowering(op);
        assert!(l.contains("eps_nano"), "`{op}` lowers its norm:\n{l}");
        l
    };
    let given = lowered("rmsnorm of X with weight W with epsilon 0.00001");
    assert!(given.contains("eps_nano: 10000,") && !given.contains("eps_nano: 1000,"), "the rsqrt takes 1e-5:\n{given}");
    let default = lowered("rmsnorm of X with weight W");
    assert!(default.contains("eps_nano: 1000,") && !default.contains("eps_nano: 10000,"), "the default is 1e-6");
    let single = |clause: &str| {
        let src = format!(
            "## Main\nLet X be a 2 by 2 tensor of Real.\nLet W be a 1 by 2 tensor of Real.\nLet B be a 1 by 2 tensor of Real.\n\
             Let Y be layernorm of X with weight W and bias B{clause}.\n"
        );
        format!("{:?}", compile_to_spatial(&src, &SpatialTarget::wormhole()).expect("the layernorm lowers"))
    };
    let layer = single("");
    assert!(layer.contains("eps_nano: 1000,") && !layer.contains("eps_nano: 10000,"), "a layernorm's default is the sugar's 1e-6");
    assert!(single(" with epsilon 0.00001").contains("eps_nano: 10000,"), "a layernorm takes the program's epsilon");
    let l2 = lowered("l2norm of X across 2 heads with epsilon 0.00002");
    assert!(l2.contains("eps_nano: 20000,"), "the l2 norm's rsqrt takes 2e-5");
    let fine = lowering("rmsnorm of X with weight W with epsilon 1.5e-10");
    assert!(fine.contains("not a whole number of nano"), "an epsilon finer than a nano is refused by name:\n{fine}");
}

/// A one-row gated delta step over 2 value heads of width 2, 1 key head, its output shown as `round(o·1e9)`.
fn delta(clause: &str) -> String {
    let mut s = String::from("## Main\n");
    for (name, rows, cols, vals) in [
        ("Q", 1, 2, vec![0.001, -0.002]),
        ("K", 1, 2, vec![0.002, 0.001]),
        ("V", 1, 4, vec![0.003, -0.001, 0.002, 0.004]),
        ("Z", 1, 4, vec![0.5, -0.25, 1.0, 0.75]),
        ("B", 1, 2, vec![0.5, 0.25]),
        ("A", 1, 2, vec![0.9, 0.8]),
        ("S", 4, 2, vec![0.001, 0.002, -0.001, 0.003, 0.002, -0.002, 0.001, 0.001]),
        ("G", 1, 2, vec![1.5, -0.5]),
    ] {
        s.push_str(&format!("Let {name} be a {rows} by {cols} tensor of Real.\n"));
        for (i, v) in vals.iter().enumerate() {
            s.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
        }
    }
    s.push_str(&format!("Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm G with 2 heads 1 groups{clause}.\n"));
    for i in 1..=4 {
        s.push_str(&format!("Show round(item {i} of O * 1000000000).\n"));
    }
    s
}

#[test]
fn the_gated_delta_rules_output_norm_takes_the_programs_epsilon() {
    let given = shown(&delta(" with epsilon 0.00001"));
    assert_ne!(given, shown(&delta("")), "the epsilon changes the normed output");
    assert_eq!(shown(&delta(" with epsilon 0.000001")), shown(&delta("")), "the default is 1e-6");
    let rp = recognize_stages(&delta(" with epsilon 0.00001")).expect("parses");
    match &rp.stages.first().expect("one op").op {
        TensorOpSpec::GatedDelta(g) => assert_eq!(g.eps.get(), 1e-5),
        other => panic!("lifts as a gated delta, not {other:?}"),
    }
}
