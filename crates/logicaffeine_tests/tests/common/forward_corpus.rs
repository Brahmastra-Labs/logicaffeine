//! The older CUDA forward's corpus (`compile_to_cuda_forward*`): the whole programs it compiles, and its quantized
//! programs with the manifests their weights come from.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::codegen_cuda::qwen_emit::{gen_qwen30b_lg, MoeWeights, Q8Ref, Qwen30bCfg};

pub fn qwen30b_tiny() -> Qwen30bCfg {
    Qwen30bCfg { d: 8, dff: 4, ne: 4, k: 2, layers: 2, h: 2, kv: 1, hd: 4, vocab: 16, rope_theta: 1.0e6, rms_eps: 1e-6, attn_split: 8 }
}

fn tensor(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for p in 0..rows * cols {
        s.push_str(&format!("Set item {} of {name} to {:?}.\n", p + 1, (((p * 37 + salt * 11) % 101) as f64 - 50.0) / 25.0));
    }
    s
}

fn declared(name: &str, rows: usize, cols: usize) -> String {
    format!("Let {name} be a {rows} by {cols} tensor of Real.\n")
}

fn q8_ref(tag: &str, in_dim: usize, out_dim: usize) -> Q8Ref {
    Q8Ref { quants: format!("/w/{tag}.q"), scales: format!("/w/{tag}.s"), in_dim, out_dim }
}

/// The forward's quantized programs: each fusion the census lock claims (a norm, an add and a norm, an attention,
/// each quantizing into an int8 projection; an int8 projection dequantized in its GEMV; a sparse mixture), the row
/// kernels beside the RMS norm that quantize the same way (a softmax, a layernorm), and an attention whose heads are not
/// whole 32-value blocks, which leaves its projection to quantize — with the manifests their weights come from.
pub fn quantized_programs() -> Vec<(String, String, HashMap<String, Q8Ref>, HashMap<String, MoeWeights>)> {
    let q8 = |name: &str, tag: &str, i: usize, o: usize| HashMap::from([(name.to_string(), q8_ref(tag, i, o))]);
    let (d, dff, ne, k) = (64usize, 64usize, 4usize, 2usize);
    let moe = HashMap::from([(
        "H".to_string(),
        MoeWeights { router: q8_ref("wr", d, ne), gate: q8_ref("wg", d, ne * dff), up: q8_ref("wu", d, ne * dff), down: q8_ref("wd", dff, ne * d), d, dff, ne, k },
    )]);
    vec![
        (
            "norm-into-int8".into(),
            format!("## Main\n{}{}Let Xn be rmsnorm of X with weight W.\n{}Let Y be Xn matmul Wq.\nShow item 1 of Y.\n", tensor("X", 1, 64, 6), tensor("W", 1, 64, 7), declared("Wq", 64, 32)),
            q8("Wq", "wq", 64, 32),
            HashMap::new(),
        ),
        (
            "add-norm-into-int8".into(),
            format!(
                "## Main\n{}{}{}Let S be A plus B.\nLet Xn be rmsnorm of S with weight W.\n{}Let Y be Xn matmul Wq.\nShow item 1 of Y.\n",
                tensor("A", 1, 64, 8),
                tensor("B", 1, 64, 9),
                tensor("W", 1, 64, 10),
                declared("Wq", 64, 32)
            ),
            q8("Wq", "wq", 64, 32),
            HashMap::new(),
        ),
        (
            "int8-projection".into(),
            format!("## Main\n{}{}Let Y be X matmul Wq.\nShow item 1 of Y.\n", tensor("X", 1, 64, 11), declared("Wq", 64, 32)),
            q8("Wq", "wq", 64, 32),
            HashMap::new(),
        ),
        (
            "attention-into-int8".into(),
            format!(
                "## Main\n{}{}{}Let C be attention of Q over K and V with 2 heads 1 groups.\n{}Let Y be C matmul Wo.\nShow item 1 of Y.\n",
                tensor("Q", 1, 64, 13),
                tensor("K", 4, 32, 14),
                tensor("V", 4, 32, 15),
                declared("Wo", 64, 32)
            ),
            q8("Wo", "wo", 64, 32),
            HashMap::new(),
        ),
        (
            "softmax-into-int8".into(),
            format!("## Main\n{}Let S be softmax of X.\n{}Let Y be S matmul Wq.\nShow item 1 of Y.\n", tensor("X", 1, 64, 16), declared("Wq", 64, 32)),
            q8("Wq", "wq", 64, 32),
            HashMap::new(),
        ),
        (
            "layernorm-into-int8".into(),
            format!(
                "## Main\n{}{}{}Let Xn be layernorm of X with weight W and bias B.\n{}Let Y be Xn matmul Wq.\nShow item 1 of Y.\n",
                tensor("X", 1, 64, 17),
                tensor("W", 1, 64, 18),
                tensor("B", 1, 64, 19),
                declared("Wq", 64, 32)
            ),
            q8("Wq", "wq", 64, 32),
            HashMap::new(),
        ),
        (
            "narrow-heads-attention-into-int8".into(),
            format!(
                "## Main\n{}{}{}Let C be attention of Q over K and V with 4 heads 1 groups.\n{}Let Y be C matmul Wo.\nShow item 1 of Y.\n",
                tensor("Q", 1, 64, 20),
                tensor("K", 4, 16, 21),
                tensor("V", 4, 16, 22),
                declared("Wo", 64, 32)
            ),
            q8("Wo", "wo", 64, 32),
            HashMap::new(),
        ),
        (
            "sparse-moe".into(),
            format!(
                "## Main\n{}{}{}{}{}Let H be moe of X with router Wr gate Wg up Wu down Wd choosing {k} of {ne}.\nShow item 1 of H.\n",
                tensor("X", 1, d, 12),
                declared("Wr", d, ne),
                declared("Wg", ne * dff, d),
                declared("Wu", ne * dff, d),
                declared("Wd", ne * d, dff)
            ),
            HashMap::new(),
            moe,
        ),
    ]
}

/// The whole models the forward compiles: the tiny Qwen3.8, the tiny Qwen demo and the tiny Qwen3-30B.
pub fn model_programs() -> Vec<(String, String)> {
    let demo = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/qwen_tiny_demo.lg")).expect("the tiny demo");
    vec![
        ("qwen38-tiny".into(), gen_qwen38_lg(&Qwen38Cfg::tiny(), "/w").0),
        ("qwen-tiny-demo".into(), demo),
        ("qwen30b-tiny".into(), gen_qwen30b_lg(&qwen30b_tiny(), "/w").0),
    ]
}
