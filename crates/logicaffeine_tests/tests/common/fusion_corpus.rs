//! The programs the shared rules are judged on: every class's exemplar and every tiny architecture's cuts, lifted with
//! their matrices as checkpoint rows and as values, and each rule's witness that its dataflow forbids it, beside its twin
//! with that taken away, and the inputs every comparison of two of them reads. `fusion_class_lock` runs them on the
//! evaluator; `tensor_rule_proof_lock` proves them; `numerics_contract_lock` runs them under every chip's shim.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::model::program::{self, Cut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::eval::Data;
use logicaffeine_compile::tensor_graph::graph::{Elem, Role, SeqIo, StepIo, Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::symbols::{Exemplar, CLASSES};

use super::tiny_models;

/// A value for element `i` of a tensor, the same on both sides of every comparison.
pub fn value(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(salt.wrapping_mul(40_503)) ^ (salt >> 7);
    ((h % 401) as f64 - 200.0) / 1009.0
}

/// Inputs for every Real tensor a graph declares (its weights and state), and the positions and tokens a cut reads.
pub fn inputs(g: &TensorGraph) -> BTreeMap<String, Data> {
    let mut v = BTreeMap::new();
    for t in g.tensors.values().filter(|t| t.role != Role::Activation) {
        if t.role == Role::Param {
            let n = t.len();
            v.insert(t.name.clone(), Data::Int((0..n as i64).map(|k| 2 + k).collect()));
        } else if t.elem == Elem::Float {
            v.insert(t.name.clone(), Data::Real((0..t.len()).map(|k| value(&t.name, k)).collect()));
        }
    }
    v
}

/// The constants a product reads as its matrix: a matmul's, a per-head matmul's, a mixture's router and experts'.
pub fn matrices(g: &TensorGraph) -> BTreeSet<String> {
    let mut m = BTreeSet::new();
    for op in &g.ops {
        match op.kind() {
            "Matmul" | "HeadMatmul" => {
                m.insert(op.ins[1].clone());
            }
            "Moe" => m.extend(op.ins[1..5].iter().cloned()),
            _ => {}
        }
    }
    m
}

/// `source` lifted with its matrices stored as checkpoint rows (`rows`) or as values, every other constant as values.
pub fn lifted(source: &str, rows: bool, io: Option<StepIo>) -> TensorGraph {
    let rp = recognize_stages(source).unwrap_or_else(|e| panic!("{e:?}"));
    let plain = TensorGraph::lift(&rp, None, io.clone(), &|_| Storage::Values).unwrap_or_else(|e| panic!("{e}"));
    let m = matrices(&plain);
    TensorGraph::lift(&rp, None, io, &|t| if rows && m.contains(t) { Storage::Rows { format: GgmlType::Q8_0, transposed: true } } else { Storage::Values })
        .unwrap()
}

pub fn io(cut: Cut) -> StepIo {
    match cut {
        Cut::Prefill { .. } => StepIo { token: "toks".into(), position: "pos_s0".into(), logits: "logits".into(), next: "next".into(), more: Vec::new() },
        Cut::Batch { rows, .. } => StepIo {
            token: "toks".into(),
            position: "pos_s0".into(),
            logits: "logits_s0".into(),
            next: "next_s0".into(),
            more: (1..rows).map(|b| SeqIo { position: format!("pos_s{b}"), logits: format!("logits_s{b}"), next: format!("next_s{b}") }).collect(),
        },
        _ => StepIo { token: "tok".into(), position: "pos".into(), logits: "logits".into(), next: "next".into(), more: Vec::new() },
    }
}

/// Every witness the rules are judged on: each class's exemplar, and every tiny architecture's decode step, prefill
/// chunk and batch — each with its matrices as checkpoint rows and as values.
pub fn witnesses() -> Vec<(String, TensorGraph)> {
    let mut w = Vec::new();
    for c in CLASSES {
        if let Exemplar::Program { source, .. } = c.exemplar {
            for rows in [true, false] {
                w.push((format!("{} exemplar (rows {rows})", c.id), lifted(source, rows, None)));
            }
        }
    }
    for &t in tiny_models::ALL {
        for cut in [Cut::Decode { ctx: 24 }, Cut::Prefill { rows: 4, ctx: 24 }, Cut::Batch { rows: 2, ctx: 24 }] {
            let ((source, _, _), _) = program::written(&t.spec(24), cut);
            for rows in [true, false] {
                w.push((format!("{} {cut:?} (rows {rows})", t.name()), lifted(&source, rows, Some(io(cut)))));
            }
        }
    }
    w
}

/// Each rule's witness that its dataflow forbids it — a second reader of what it would fold, a write between, an
/// observed intermediate — and its twin with that taken away, where it fires: `(rule, why, forbidding, twin)`.
pub const HOLD_BACKS: &[(&str, &str, &str, &str)] = &[
    (
        "add_norm",
        "a sum read first by something other than the norm",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet T be S scaled by 2.0.\nLet Y be rmsnorm of S with weight W.\n",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet Y be rmsnorm of S with weight W.\n",
    ),
    (
        "head_norm_rope",
        "a per-head norm read by a second op",
        "## Main\nLet Q be a 1 by 16 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet M be rmsnorm of Q with weight N across 2 heads.\nLet P be rope of M with cos RC and sin RS.\nLet Z be M scaled by 2.0.\n",
        "## Main\nLet Q be a 1 by 16 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet M be rmsnorm of Q with weight N across 2 heads.\nLet P be rope of M with cos RC and sin RS.\n",
    ),
    (
        "conv_silu",
        "a convolution's output read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 4 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet Y be causal conv of X with weight W state C over 4 taps.\nLet S be silu of Y.\nLet Z be Y scaled by 2.0.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 4 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet Y be causal conv of X with weight W state C over 4 taps.\nLet S be silu of Y.\n",
    ),
    (
        "gather_scale",
        "a gathered row read by a second op",
        "## Main\nLet E be a 16 by 8 tensor of Real.\nLet tok be 3.\nLet X be row tok of E.\nLet Y be X scaled by 2.0.\nLet Z be X scaled by 3.0.\n",
        "## Main\nLet E be a 16 by 8 tensor of Real.\nLet tok be 3.\nLet X be row tok of E.\nLet Y be X scaled by 2.0.\n",
    ),
    (
        "row_write",
        "the table read between the product and its row's write",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet T be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet K be X matmul M.\nLet R be row 1 of T.\nLet U be R scaled by 2.0.\nSet row pos of T to K.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet T be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet K be X matmul M.\nSet row pos of T to K.\n",
    ),
    (
        "sandwich_norm",
        "the inner norm's input written in place between it and the outer norm",
        "## Main\nLet X be a 4 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet A be a 4 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet pos be 2.\nLet N be rmsnorm of X with weight W.\nSet row pos of X to R.\nLet S be A plus N.\nLet Y be rmsnorm of S with weight V.\n",
        "## Main\nLet X be a 4 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet A be a 4 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet pos be 2.\nLet N be rmsnorm of X with weight W.\nLet S be A plus N.\nLet Y be rmsnorm of S with weight V.\n",
    ),
    (
        "norm_scale",
        "a norm read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet N be rmsnorm of X with weight W.\nLet Y be N scaled by 0.5.\nLet Z be N scaled by 2.0.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet N be rmsnorm of X with weight W.\nLet Y be N scaled by 0.5.\n",
    ),
    (
        "gemm_scale",
        "a product read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet P be X matmul M.\nLet Y be P scaled by 0.25.\nLet Z be P scaled by 2.0.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet P be X matmul M.\nLet Y be P scaled by 0.25.\n",
    ),
    (
        "affine_input_fold",
        "a scaled activation read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet S be X scaled by 0.5.\nLet Y be S matmul M.\nLet Z be S plus X.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet S be X scaled by 0.5.\nLet Y be S matmul M.\n",
    ),
    (
        "relu_sqr",
        "a ReLU read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be relu of X.\nLet Y be R hadamard R.\nLet Z be R plus X.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be relu of X.\nLet Y be R hadamard R.\n",
    ),
    (
        "elementwise_chains",
        "a chain's carried result read by a second op",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet C be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet Y be S hadamard C.\nLet Z be S plus C.\n",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet C be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet Y be S hadamard C.\n",
    ),
    (
        "elementwise_regions",
        "a region's result read before its last op",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet P be A hadamard B.\nLet Z be P plus A.\nLet Y be S plus P.\n",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet S be A plus B.\nLet P be A hadamard B.\nLet Y be S plus P.\n",
    ),
    (
        "recompute_fanout",
        "a cheap producer the program shows",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet S be silu of X.\nLet A be S plus X.\nLet B be S hadamard X.\nShow item 1 of S.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet S be silu of X.\nLet A be S plus X.\nLet B be S hadamard X.\n",
    ),
    (
        "dual_norm",
        "a second norm of the first one's result",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet Y be rmsnorm of A with weight W.\nLet Z be rmsnorm of Y with weight V.\n",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet Y be rmsnorm of A with weight W.\nLet B be a 1 by 8 tensor of Real.\nLet Z be rmsnorm of B with weight V.\n",
    ),
    (
        "embed_norm",
        "a gathered row read first by something other than the norm",
        "## Main\nLet E be a 16 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet tok be 3.\nLet X be row tok of E.\nLet Y be X plus X.\nLet Z be rmsnorm of X with weight W.\n",
        "## Main\nLet E be a 16 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet tok be 3.\nLet X be row tok of E.\nLet Z be rmsnorm of X with weight W.\n",
    ),
    (
        "gemv_norm_epilogue",
        "a product read by an op besides the add and norm",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet P be X matmul M.\nLet S be R plus P.\nLet Y be rmsnorm of S with weight W.\nLet Z be P plus R.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet W be a 1 by 8 tensor of Real.\nLet P be X matmul M.\nLet S be R plus P.\nLet Y be rmsnorm of S with weight W.\n",
    ),
    (
        "qkv_head_epilogue",
        "a product read by an op besides the norm and rotation",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet K be X matmul M.\nLet H be rmsnorm of K with weight N across 1 heads.\nLet P be rope of H with cos RC and sin RS.\nLet Z be K plus X.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet M be a 8 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet K be X matmul M.\nLet H be rmsnorm of K with weight N across 1 heads.\nLet P be rope of H with cos RC and sin RS.\n",
    ),
    (
        "cross_layer_launch",
        "the second table read between the two writes",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet D be a 4 by 8 tensor of Real.\nLet pos be 2.\nSet row pos of C to A.\nLet R be row 2 of D.\nLet U be R scaled by 2.0.\nSet row pos of D to B.\n",
        "## Main\nLet A be a 1 by 8 tensor of Real.\nLet B be a 1 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet D be a 4 by 8 tensor of Real.\nLet pos be 2.\nSet row pos of C to A.\nSet row pos of D to B.\n",
    ),
    (
        "merged_projections",
        "a later product's cache row read between the products",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet A be a 8 by 8 tensor of Real.\nLet B be a 8 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet Q be X matmul A.\nLet R be row 2 of C.\nLet U be R scaled by 2.0.\nLet V be X matmul B.\nSet row pos of C to V.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet A be a 8 by 8 tensor of Real.\nLet B be a 8 by 8 tensor of Real.\nLet C be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet Q be X matmul A.\nLet V be X matmul B.\nSet row pos of C to V.\n",
    ),
    (
        "merged_projections",
        "a later product's rotation table written between the products",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet A be a 8 by 8 tensor of Real.\nLet B be a 8 by 8 tensor of Real.\nLet RC be a 4 by 4 tensor of Real.\nLet RS be a 4 by 4 tensor of Real.\nLet Z be a 1 by 4 tensor of Real.\nLet pos be 2.\nLet Q be X matmul A.\nSet row 2 of RC to Z.\nLet K be X matmul B.\nLet P be rope of K with cos RC and sin RS from position pos across 1 heads.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet A be a 8 by 8 tensor of Real.\nLet B be a 8 by 8 tensor of Real.\nLet RC be a 4 by 4 tensor of Real.\nLet RS be a 4 by 4 tensor of Real.\nLet Z be a 1 by 4 tensor of Real.\nLet pos be 2.\nLet Q be X matmul A.\nLet K be X matmul B.\nLet P be rope of K with cos RC and sin RS from position pos across 1 heads.\n",
    ),
    (
        "slice_read",
        "the source written between the slice and its reader",
        "## Main\nLet X be a 4 by 16 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet V be a 1 by 16 tensor of Real.\nLet pos be 2.\nLet P be the last 8 of every 16 of X.\nSet row pos of X to V.\nLet Y be rope of P with cos RC and sin RS.\n",
        "## Main\nLet X be a 4 by 16 tensor of Real.\nLet RC be a 1 by 4 tensor of Real.\nLet RS be a 1 by 4 tensor of Real.\nLet V be a 1 by 16 tensor of Real.\nLet pos be 2.\nLet P be the last 8 of every 16 of X.\nLet Y be rope of P with cos RC and sin RS.\n",
    ),
    (
        "gdn_decode_whole",
        "a convolution's result read by a second op",
        "## Main\nLet XQ be a 1 by 64 tensor of Real.\nLet XK be a 1 by 64 tensor of Real.\nLet XV be a 1 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 1 by 4 tensor of Real.\nLet AR be a 1 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\nLet E be V scaled by 2.0.\n",
        "## Main\nLet XQ be a 1 by 64 tensor of Real.\nLet XK be a 1 by 64 tensor of Real.\nLet XV be a 1 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 1 by 4 tensor of Real.\nLet AR be a 1 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
    ),
    (
        "gdn_decode_whole",
        "a convolution's ring written between it and the rule",
        "## Main\nLet XQ be a 1 by 64 tensor of Real.\nLet XK be a 1 by 64 tensor of Real.\nLet XV be a 1 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 1 by 4 tensor of Real.\nLet AR be a 1 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet R be a 1 by 64 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nSet row 2 of CQ to R.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
        "## Main\nLet XQ be a 1 by 64 tensor of Real.\nLet XK be a 1 by 64 tensor of Real.\nLet XV be a 1 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 1 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 1 by 4 tensor of Real.\nLet AR be a 1 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet R be a 1 by 64 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
    ),
    (
        "gdn_post_conv_prep",
        "a chunk's convolution result read by a second op",
        "## Main\nLet XQ be a 2 by 64 tensor of Real.\nLet XK be a 2 by 64 tensor of Real.\nLet XV be a 2 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 2 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 2 by 4 tensor of Real.\nLet AR be a 2 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 4 heads.\nLet K be l2norm of SK across 4 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\nLet E be V scaled by 2.0.\n",
        "## Main\nLet XQ be a 2 by 64 tensor of Real.\nLet XK be a 2 by 64 tensor of Real.\nLet XV be a 2 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 2 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 2 by 4 tensor of Real.\nLet AR be a 2 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 4 heads.\nLet K be l2norm of SK across 4 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
    ),
    (
        "gdn_post_conv_prep",
        "a chunk's convolution ring written between it and the gates",
        "## Main\nLet XQ be a 2 by 64 tensor of Real.\nLet XK be a 2 by 64 tensor of Real.\nLet XV be a 2 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 2 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 2 by 4 tensor of Real.\nLet AR be a 2 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet R be a 1 by 64 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 4 heads.\nLet K be l2norm of SK across 4 heads.\nSet row 2 of CQ to R.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
        "## Main\nLet XQ be a 2 by 64 tensor of Real.\nLet XK be a 2 by 64 tensor of Real.\nLet XV be a 2 by 128 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet Z be a 2 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet BR be a 2 by 4 tensor of Real.\nLet AR be a 2 by 4 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet R be a 1 by 64 tensor of Real.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 4 heads.\nLet K be l2norm of SK across 4 heads.\nLet BT be sigmoid of BR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
    ),
    (
        "gdn_input_epilogue",
        "the values projected from a second activation",
        "## Main\nLet X be a 1 by 16 tensor of Real.\nLet X2 be a 1 by 16 tensor of Real.\nLet PXQ be a 16 by 64 tensor of Real.\nLet PXK be a 16 by 64 tensor of Real.\nLet PXV be a 16 by 128 tensor of Real.\nLet PZ be a 16 by 128 tensor of Real.\nLet PBR be a 16 by 4 tensor of Real.\nLet PAR be a 16 by 4 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet XQ be X matmul PXQ.\nLet XK be X matmul PXK.\nLet XV be X2 matmul PXV.\nLet Z be X matmul PZ.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nLet BR be X matmul PBR.\nLet BT be sigmoid of BR.\nLet AR be X matmul PAR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
        "## Main\nLet X be a 1 by 16 tensor of Real.\nLet PXQ be a 16 by 64 tensor of Real.\nLet PXK be a 16 by 64 tensor of Real.\nLet PXV be a 16 by 128 tensor of Real.\nLet PZ be a 16 by 128 tensor of Real.\nLet PBR be a 16 by 4 tensor of Real.\nLet PAR be a 16 by 4 tensor of Real.\nLet WQ be a 4 by 64 tensor of Real.\nLet WK be a 4 by 64 tensor of Real.\nLet WV be a 4 by 128 tensor of Real.\nLet CQ be a 4 by 64 tensor of Real.\nLet CK be a 4 by 64 tensor of Real.\nLet CV be a 4 by 128 tensor of Real.\nLet S be a 128 by 32 tensor of Real.\nLet G be a 1 by 32 tensor of Real.\nLet DT be a 1 by 4 tensor of Real.\nLet AL be a 1 by 4 tensor of Real.\nLet XQ be X matmul PXQ.\nLet XK be X matmul PXK.\nLet XV be X matmul PXV.\nLet Z be X matmul PZ.\nLet YQ be causal conv of XQ with weight WQ state CQ over 4 taps.\nLet YK be causal conv of XK with weight WK state CK over 4 taps.\nLet YV be causal conv of XV with weight WV state CV over 4 taps.\nLet SQ be silu of YQ.\nLet SK be silu of YK.\nLet V be silu of YV.\nLet Q be l2norm of SQ across 2 heads.\nLet K be l2norm of SK across 2 heads.\nLet BR be X matmul PBR.\nLet BT be sigmoid of BR.\nLet AR be X matmul PAR.\nLet DY be decay of AR with bias DT rate AL.\nLet O be gated delta of Q over K and V with gate Z beta BT decay DY state S norm G with 4 heads 2 groups.\n",
    ),
    (
        "moe_deferred_finalize",
        "a mixture's result read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet S be R plus M.\nLet Y be rmsnorm of S with weight N.\nLet Z be M scaled by 2.0.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet S be R plus M.\nLet Y be rmsnorm of S with weight N.\n",
    ),
    (
        "moe_deferred_finalize",
        "a mixture summed with its shared expert, the sum read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet E be M plus SD.\nLet S be R plus E.\nLet Y be rmsnorm of S with weight N.\nLet Z be E scaled by 2.0.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet E be M plus SD.\nLet S be R plus E.\nLet Y be rmsnorm of S with weight N.\n",
    ),
    (
        "moe_deferred_finalize",
        "a mixture summed with its shared expert, the mixture read by a second op",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet E be M plus SD.\nLet S be R plus E.\nLet Y be rmsnorm of S with weight N.\nLet Z be M scaled by 2.0.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet E be M plus SD.\nLet S be R plus E.\nLet Y be rmsnorm of S with weight N.\n",
    ),
    (
        "moe_deferred_finalize",
        "the mixture's activation written between it and the add",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nSet row 1 of X to V.\nLet S be R plus M.\nLet Y be rmsnorm of S with weight N.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet V be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet S be R plus M.\nLet Y be rmsnorm of S with weight N.\n",
    ),
    (
        "moe_deferred_finalize",
        "a mixture multiplied by a second operand, not summed with a shared expert",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet E be M hadamard SD.\nLet S be R plus E.\nLet Y be rmsnorm of S with weight N.\n",
        "## Main\nLet X be a 1 by 8 tensor of Real.\nLet R be a 1 by 8 tensor of Real.\nLet WR be a 8 by 4 tensor of Real.\nLet WG be a 32 by 8 tensor of Real.\nLet WU be a 32 by 8 tensor of Real.\nLet WD be a 32 by 8 tensor of Real.\nLet SD be a 1 by 8 tensor of Real.\nLet N be a 1 by 8 tensor of Real.\nLet M be moe of X with router WR gate WG up WU down WD choosing 2 of 4.\nLet E be M plus SD.\nLet S be R plus E.\nLet Y be rmsnorm of S with weight N.\n",
    ),
    (
        "attn_output_gate",
        "an attention's result read by a second op",
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet G be a 1 by 8 tensor of Real.\nLet pos be 3.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos.\nLet S be sigmoid of G.\nLet Y be C hadamard S.\nLet Z be C scaled by 2.0.\n",
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet G be a 1 by 8 tensor of Real.\nLet pos be 3.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos.\nLet S be sigmoid of G.\nLet Y be C hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "an attention's result the program shows",
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet G be a 1 by 8 tensor of Real.\nLet pos be 3.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos.\nLet S be sigmoid of G.\nLet Y be C hadamard S.\nShow item 1 of C.\n",
        "## Main\nLet Q be a 1 by 8 tensor of Real.\nLet K be a 4 by 4 tensor of Real.\nLet V be a 4 by 4 tensor of Real.\nLet G be a 1 by 8 tensor of Real.\nLet pos be 3.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos.\nLet S be sigmoid of G.\nLet Y be C hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "a batch's table read by a second op",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\nLet Z be T scaled by 2.0.\n",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "a batch's table the program shows",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\nShow item 1 of T.\n",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "a batch's attention's result the program shows",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\nShow item 1 of C0.\n",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "a row of a batch's table set from no attention",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nSet row 2 of T to Q1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "a batch's table with a row no set writes",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
    ),
    (
        "attn_output_gate",
        "a row of a batch's table set twice",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet C2 be attention of Q1 over K0 and V0 with 2 heads 1 groups up to p1.\nSet row 2 of T to C2.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
        "## Main\nLet Q0 be a 1 by 8 tensor of Real.\nLet Q1 be a 1 by 8 tensor of Real.\nLet K0 be a 4 by 4 tensor of Real.\nLet V0 be a 4 by 4 tensor of Real.\nLet K1 be a 4 by 4 tensor of Real.\nLet V1 be a 4 by 4 tensor of Real.\nLet T be a 2 by 8 tensor of Real.\nLet G be a 2 by 8 tensor of Real.\nLet p0 be 3.\nLet p1 be 2.\nLet C0 be attention of Q0 over K0 and V0 with 2 heads 1 groups up to p0.\nSet row 1 of T to C0.\nLet C1 be attention of Q1 over K1 and V1 with 2 heads 1 groups up to p1.\nSet row 2 of T to C1.\nLet S be sigmoid of G.\nLet Y be T hadamard S.\n",
    ),
];
