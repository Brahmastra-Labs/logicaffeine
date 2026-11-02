//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! tensor_graph_vjp_lock — a derived backward pass is the derivative, op by op and composed.
//!
//! `tensor_graph::vjp::backward` writes the gradient of `Σ out ⊙ W` as ordinary LOGOS. The oracle is the forward
//! itself: for every element of every input, the central difference `(L(θ + h) − L(θ − h)) / 2h` of the loss run
//! on the VM in f64. The derived gradient, run on the same VM, must agree to 1e-6 (relative, floored at 1) at
//! every element — each op alone, then composed programs where a tensor feeds two stages and its gradients sum.
//!
//! Every op kind the front end lifts has a rule or a refusal that names it: `argmax` and an int8 model's integer forms
//! have no gradient, and the ops whose rules are not written yet refuse by name — each with a test here, so a refusal never goes quiet.
//!
//!   cargo nextest run -p logicaffeine-tests --test tensor_graph_vjp_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::tensor_graph::vjp::{backward, VjpRefusal};

/// A deterministic value in `[-1, 1]` for element `i` of stream `seed`, rounded to four places so the program
/// text carries it exactly.
fn wv(seed: u64, i: usize) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    let u = (x % 20_001) as f64 / 10_000.0 - 1.0;
    (u * 10_000.0).round() / 10_000.0
}

/// One tensor input: its name, shape and values.
#[derive(Clone)]
struct Input {
    name: &'static str,
    rows: usize,
    cols: usize,
    values: Vec<f64>,
    real: bool,
}

fn real(name: &'static str, rows: usize, cols: usize, seed: u64) -> Input {
    Input { name, rows, cols, values: (0..rows * cols).map(|i| wv(seed, i)).collect(), real: true }
}

fn real_with(name: &'static str, rows: usize, cols: usize, values: Vec<f64>) -> Input {
    assert_eq!(values.len(), rows * cols);
    Input { name, rows, cols, values, real: true }
}

fn ints(name: &'static str, rows: usize, cols: usize, values: Vec<i64>) -> Input {
    Input { name, rows, cols, values: values.into_iter().map(|v| v as f64).collect(), real: false }
}

fn declare(src: &mut String, t: &Input) {
    let kind = if t.real { "Real" } else { "Int" };
    src.push_str(&format!("Let {} be a {} by {} tensor of {kind}.\n", t.name, t.rows, t.cols));
    for (i, v) in t.values.iter().enumerate() {
        if t.real {
            src.push_str(&format!("Set item {} of {} to {v:?}.\n", i + 1, t.name));
        } else {
            src.push_str(&format!("Set item {} of {} to {}.\n", i + 1, t.name, *v as i64));
        }
    }
}

fn vm(src: &str) -> Vec<f64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error {:?} in\n{src}", out.error);
    out.output.split_whitespace().map(|s| s.parse().unwrap_or_else(|_| panic!("not a number: {s}"))).collect()
}

/// The forward program: inputs, the cotangent `W` of the output's shape, then the body.
fn forward(inputs: &[Input], w: &Input, body: &str) -> String {
    let mut src = String::from("## Main\n");
    for t in inputs {
        declare(&mut src, t);
    }
    declare(&mut src, w);
    src.push_str(body);
    src
}

/// `L = Σ out ⊙ W` as a program.
fn loss_program(inputs: &[Input], w: &Input, body: &str, out: &str) -> String {
    let mut src = forward(inputs, w, body);
    src.push_str("Let mutable vjp_loss be 0.0.\n");
    src.push_str(&format!("Repeat for vjp_i from 1 to {}:\n", w.rows * w.cols));
    src.push_str(&format!("    Set vjp_loss to vjp_loss + (item vjp_i of {out}) * (item vjp_i of W).\n"));
    src.push_str("Show vjp_loss.\n");
    src
}

/// Check the derived gradient of `L = Σ out ⊙ W` against central differences for every element of every input
/// in `wrt`.
fn check(what: &str, inputs: Vec<Input>, out: &str, out_shape: (usize, usize), body: &str, wrt: &[&'static str]) {
    let w = real("W", out_shape.0, out_shape.1, 977);
    let fwd = forward(&inputs, &w, body);
    let b = backward(&fwd, out, "W", wrt).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let mut prog = fwd.clone();
    prog.push_str(&b.source);
    for t in wrt {
        let g = &b.grads[*t];
        let n = inputs.iter().find(|i| i.name == *t).map(|i| i.values.len()).expect("a wrt input");
        for k in 1..=n {
            prog.push_str(&format!("Show item {k} of {g}.\n"));
        }
    }
    let derived = vm(&prog);
    let h = 1e-5;
    let mut at = 0;
    let mut checked = 0;
    for t in wrt {
        let idx = inputs.iter().position(|i| i.name == *t).unwrap();
        for e in 0..inputs[idx].values.len() {
            let mut plus = inputs.clone();
            plus[idx].values[e] += h;
            let mut minus = inputs.clone();
            minus[idx].values[e] -= h;
            let lp = vm(&loss_program(&plus, &w, body, out))[0];
            let lm = vm(&loss_program(&minus, &w, body, out))[0];
            let fd = (lp - lm) / (2.0 * h);
            let ad = derived[at];
            assert!(
                (ad - fd).abs() <= 1e-6 * fd.abs().max(1.0),
                "{what}: d L / d {t}[{}] derived {ad} vs central difference {fd}\nbackward:\n{}",
                e + 1,
                b.source
            );
            at += 1;
            checked += 1;
        }
    }
    assert!(checked > 0, "{what}: nothing checked");
}

// ── Each op alone ──────────────────────────────────────────────────────────────────────────────────────

#[test]
fn matmul_differentiates_into_two_transposed_matmuls() {
    check("matmul", vec![real("A", 2, 3, 1), real("B", 3, 2, 2)], "C", (2, 2), "Let C be A matmul B.\n", &["A", "B"]);
}

#[test]
fn plus_passes_its_gradient_to_both_operands() {
    check("plus", vec![real("A", 2, 3, 3), real("B", 2, 3, 4)], "C", (2, 3), "Let C be A plus B.\n", &["A", "B"]);
}

#[test]
fn hadamard_scales_each_gradient_by_the_other_operand() {
    check("hadamard", vec![real("A", 2, 3, 5), real("B", 2, 3, 6)], "C", (2, 3), "Let C be A hadamard B.\n", &["A", "B"]);
}

#[test]
fn a_tensor_read_twice_sums_its_gradients() {
    check("square", vec![real("A", 2, 3, 7)], "C", (2, 3), "Let C be A hadamard A.\n", &["A"]);
}

#[test]
fn scale_differentiates_into_the_same_scale() {
    check("scale", vec![real("X", 2, 3, 41)], "Y", (2, 3), "Let Y be X scaled by -1.75.\n", &["X"]);
    check(
        "scaled scores",
        vec![real("Q", 2, 4, 42), real("K", 4, 3, 43)],
        "P",
        (2, 3),
        "Let S be Q matmul K.\nLet T be S scaled by 0.125.\nLet P be softmax of T.\n",
        &["Q", "K"],
    );
}

#[test]
fn transpose_differentiates_into_a_transpose() {
    check("transpose", vec![real("A", 2, 3, 8)], "T", (3, 2), "Let T be transpose of A.\n", &["A"]);
}

#[test]
fn a_head_slice_scatters_its_gradient_into_the_slots_it_read() {
    check("first of every", vec![real("X", 2, 6, 44)], "P", (2, 4), "Let P be the first 2 of every 3 of X.\n", &["X"]);
    check("last of every", vec![real("X", 2, 6, 45)], "P", (2, 2), "Let P be the last 1 of every 3 of X.\n", &["X"]);
}

#[test]
fn a_join_splits_its_gradient_between_the_parts_it_joined() {
    check("join", vec![real("A", 2, 3, 46), real("B", 2, 2, 47)], "J", (2, 5), "Let J be A joined with B.\n", &["A", "B"]);
    check(
        "join across heads",
        vec![real("A", 2, 4, 48), real("B", 2, 6, 49)],
        "J",
        (2, 10),
        "Let J be A joined with B across 2 heads.\n",
        &["A", "B"],
    );
}

#[test]
fn a_per_head_matmul_differentiates_in_both_operands() {
    check(
        "matmul across heads",
        vec![real("X", 2, 6, 50), real("V", 6, 4, 51)],
        "Y",
        (2, 8),
        "Let Y be X matmul V across 2 heads.\n",
        &["X", "V"],
    );
}

#[test]
fn softmax_differentiates_per_row() {
    check("softmax", vec![real("S", 2, 4, 9)], "P", (2, 4), "Let P be softmax of S.\n", &["S"]);
}

#[test]
fn rmsnorm_with_a_weight_differentiates_in_input_and_weight() {
    check(
        "rmsnorm",
        vec![real("X", 2, 4, 10), real("G", 1, 4, 11)],
        "Y",
        (2, 4),
        "Let Y be rmsnorm of X with weight G.\n",
        &["X", "G"],
    );
}

#[test]
fn layernorm_differentiates_in_input_weight_and_bias() {
    check(
        "layernorm",
        vec![real("X", 2, 4, 12), real("G", 1, 4, 13), real("Bb", 1, 4, 14)],
        "Y",
        (2, 4),
        "Let Y be layernorm of X with weight G and bias Bb.\n",
        &["X", "G", "Bb"],
    );
}

#[test]
fn l2norm_differentiates_per_row() {
    check("l2norm", vec![real("X", 2, 3, 15)], "Y", (2, 3), "Let Y be l2norm of X.\n", &["X"]);
}

/// The fused residual form `rmsnorm of X plus R with weight G`: the norm is of `X + R`, so both summands take the one
/// gradient of the fused value, and the weight's gradient sums over it.
#[test]
fn a_fused_residual_rmsnorm_differentiates_in_both_summands_and_the_weight() {
    check(
        "fused residual rmsnorm",
        vec![real("X", 2, 4, 75), real("R", 2, 4, 76), real("G", 1, 4, 77)],
        "Y",
        (2, 4),
        "Let Y be rmsnorm of X plus R with weight G.\n",
        &["X", "R", "G"],
    );
}

#[test]
fn a_weightless_rmsnorm_differentiates_over_its_one_row() {
    check("weightless rmsnorm", vec![real("X", 1, 5, 78)], "Y", (1, 5), "Let Y be rmsnorm of X.\n", &["X"]);
}

/// A norm across heads normalizes each head's slice of the row on its own — the per-head query and key norm — with
/// one weight every head shares, so the weight's gradient sums over the heads.
#[test]
fn a_norm_across_heads_differentiates_each_head_on_its_own() {
    check(
        "rmsnorm across heads",
        vec![real("Q", 1, 8, 79), real("G", 1, 4, 80)],
        "Y",
        (1, 8),
        "Let Y be rmsnorm of Q with weight G across 2 heads.\n",
        &["Q", "G"],
    );
    check("l2norm across heads", vec![real("X", 1, 6, 81)], "Y", (1, 6), "Let Y be l2norm of X across 2 heads.\n", &["X"]);
}

/// The per-head query norm between two projections: the norm's gradient is the query's, in the query's own shape, so
/// the projection before it differentiates through it.
#[test]
fn a_per_head_norm_between_two_projections_differentiates_end_to_end() {
    for norm in ["rmsnorm of q with weight G across 2 heads", "l2norm of q across 2 heads"] {
        check(
            norm,
            vec![real("x", 1, 4, 103), real("Wq", 4, 8, 104), real("G", 1, 4, 105), real("Wo", 8, 3, 106)],
            "y",
            (1, 3),
            &format!("Let q be x matmul Wq.\nLet qn be {norm}.\nLet y be qn matmul Wo.\n"),
            &["x", "Wq", "Wo"],
        );
    }
}

/// Each norm differentiates at the epsilon its program gives, not the default one: at these inputs an epsilon of a few
/// hundredths moves every gradient by a percent or more, far past the check's tolerance.
#[test]
fn each_norm_differentiates_at_the_epsilon_its_program_gives() {
    check(
        "rmsnorm, epsilon given",
        vec![real("X", 2, 4, 82), real("G", 1, 4, 83)],
        "Y",
        (2, 4),
        "Let Y be rmsnorm of X with weight G with epsilon 0.05.\n",
        &["X", "G"],
    );
    check(
        "fused residual rmsnorm, epsilon given in scientific notation",
        vec![real("X", 2, 4, 84), real("R", 2, 4, 85), real("G", 1, 4, 86)],
        "Y",
        (2, 4),
        "Let Y be rmsnorm of X plus R with weight G with epsilon 2.5e-2.\n",
        &["X", "R", "G"],
    );
    check("weightless rmsnorm, epsilon given", vec![real("X", 1, 5, 87)], "Y", (1, 5), "Let Y be rmsnorm of X with epsilon 0.05.\n", &["X"]);
    check(
        "rmsnorm across heads, epsilon given",
        vec![real("Q", 1, 8, 88), real("G", 1, 4, 89)],
        "Y",
        (1, 8),
        "Let Y be rmsnorm of Q with weight G across 2 heads with epsilon 0.05.\n",
        &["Q", "G"],
    );
    check(
        "layernorm, epsilon given",
        vec![real("X", 2, 4, 90), real("G", 1, 4, 91), real("Bb", 1, 4, 92)],
        "Y",
        (2, 4),
        "Let Y be layernorm of X with weight G and bias Bb with epsilon 0.05.\n",
        &["X", "G", "Bb"],
    );
    check("l2norm, epsilon given", vec![real("X", 2, 3, 93)], "Y", (2, 3), "Let Y be l2norm of X with epsilon 0.05.\n", &["X"]);
}

#[test]
fn sigmoid_silu_and_gelu_differentiate_pointwise() {
    for (op, seed) in [("sigmoid", 16u64), ("silu", 17), ("gelu", 18)] {
        check(op, vec![real("X", 2, 3, seed)], "Y", (2, 3), &format!("Let Y be {op} of X.\n"), &["X"]);
    }
}

#[test]
fn swiglu_differentiates_in_gate_and_up() {
    check("swiglu", vec![real("Gt", 2, 3, 19), real("Up", 2, 3, 20)], "Y", (2, 3), "Let Y be swiglu of Gt and Up.\n", &["Gt", "Up"]);
}

#[test]
fn an_embedding_row_scatters_its_gradient_into_the_table() {
    check("row", vec![real("E", 3, 4, 21)], "x", (1, 4), "Let x be row 2 of E.\n", &["E"]);
}

#[test]
fn embedding_rows_scatter_and_sum_where_an_index_repeats() {
    check(
        "rows",
        vec![real("E", 3, 4, 22), ints("I", 1, 3, vec![2, 3, 2])],
        "X",
        (3, 4),
        "Let X be rows I of E.\n",
        &["E"],
    );
}

// ── Composed ───────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_two_layer_mlp_differentiates_end_to_end() {
    check(
        "mlp",
        vec![real("x", 1, 4, 23), real("W1", 4, 3, 24), real("W2", 3, 2, 25)],
        "y",
        (1, 2),
        "Let h be x matmul W1.\nLet a be silu of h.\nLet y be a matmul W2.\n",
        &["x", "W1", "W2"],
    );
}

#[test]
fn an_attention_core_differentiates_through_softmax_and_both_matmuls() {
    check(
        "attention core",
        vec![real("Q", 2, 3, 26), real("K", 2, 3, 27), real("V", 2, 3, 28)],
        "C",
        (2, 3),
        "Let Kt be transpose of K.\nLet S be Q matmul Kt.\nLet P be softmax of S.\nLet C be P matmul V.\n",
        &["Q", "K", "V"],
    );
}

#[test]
fn a_pre_norm_residual_block_differentiates_through_every_path() {
    check(
        "residual block",
        vec![real("x", 2, 4, 29), real("G", 1, 4, 30), real("Wa", 4, 4, 31)],
        "y",
        (2, 4),
        "Let n be rmsnorm of x with weight G.\nLet f be n matmul Wa.\nLet s be gelu of f.\nLet y be x plus s.\n",
        &["x", "G", "Wa"],
    );
}

#[test]
fn an_input_the_loss_does_not_reach_gets_a_zero_gradient() {
    let inputs = vec![real("A", 2, 2, 32), real("B", 2, 2, 33), real("U", 2, 2, 34)];
    let w = real("W", 2, 2, 977);
    let fwd = forward(&inputs, &w, "Let C be A matmul B.\n");
    let b = backward(&fwd, "C", "W", &["U"]).expect("a backward");
    let mut prog = fwd;
    prog.push_str(&b.source);
    for k in 1..=4 {
        prog.push_str(&format!("Show item {k} of {}.\n", b.grads["U"]));
    }
    assert_eq!(vm(&prog), vec![0.0; 4]);
}

// ── Refusals, each by name ─────────────────────────────────────────────────────────────────────────────

fn refusal(inputs: Vec<Input>, body: &str, out: &str, out_shape: (usize, usize), wrt: &[&str]) -> VjpRefusal {
    let w = real("W", out_shape.0, out_shape.1, 977);
    let fwd = forward(&inputs, &w, body);
    match backward(&fwd, out, "W", wrt) {
        Err(e) => e,
        Ok(b) => panic!("expected a refusal, got a backward pass:\n{}", b.source),
    }
}

#[test]
fn argmax_has_no_gradient_and_says_why() {
    let e = refusal(vec![real("X", 1, 4, 40)], "Let t be argmax of X.\nLet Y be a 1 by 1 tensor of Real.\n", "t", (1, 1), &["X"]);
    match e {
        VjpRefusal::NoGradient { op, why } => {
            assert_eq!(op, "ArgMax");
            assert!(why.contains("piecewise constant"), "{why}");
        }
        other => panic!("argmax: {other:?}"),
    }
}

/// An int8 model's forms compute integers: each is a step function of its operands, so no gradient flows through it,
/// and the refusal says so rather than answering zero.
#[test]
fn the_integer_forms_have_no_gradient_and_say_why() {
    let x = || ints("X", 1, 4, vec![-3, 1, 4, -1]);
    let lut = vec![
        ints("Q", 1, 4, vec![1, -2, 3, 0]),
        ints("K", 2, 4, vec![2, 0, -1, 1, 0, 3, 1, -2]),
        ints("V", 2, 4, vec![5, -4, 3, 2, -1, 0, 7, 6]),
        ints("E", 1, 8, vec![32, 28, 24, 21, 18, 16, 14, 12]),
    ];
    for (inputs, body, out, shape, op) in [
        (vec![x()], "Let Y be X times 3 shifted right by 1.\n", "Y", (1, 4), "IntRequant"),
        (vec![x()], "Let Y be X clamped between -2 and 2.\n", "Y", (1, 4), "IntClamp"),
        (vec![x()], "Let Y be relu of X.\n", "Y", (1, 4), "Relu"),
        (
            lut,
            "Let pos be 2.\nLet C be lut attention of Q over K and V with 1 heads 1 groups up to pos with table E and scale 3 shifted right by 2.\n",
            "C",
            (1, 4),
            "LutAttention",
        ),
    ] {
        let wrt = [inputs[0].name];
        match refusal(inputs, body, out, shape, &wrt) {
            VjpRefusal::NoGradient { op: got, why } => {
                assert_eq!(got, op, "{body}");
                assert!(why.contains("integer arithmetic"), "{op}: {why}");
            }
            other => panic!("{op}: {other:?}"),
        }
    }
}

#[test]
fn a_real_relu_refuses_by_name_until_its_rule_is_written() {
    not_derived(refusal(vec![real("X", 1, 4, 44)], "Let Y be relu of X.\n", "Y", (1, 4), &["X"]), "Relu");
}

fn not_derived(e: VjpRefusal, want: &str) {
    assert_eq!(e, VjpRefusal::NotDerived { op: match want {
        "CausalConv" => "CausalConv",
        "Moe" => "Moe",
        "GatedDelta" => "GatedDelta",
        "Decay" => "Decay",
        "Relu" => "Relu",
        other => panic!("{other}"),
    } });
}

#[test]
fn rope_differentiates_as_the_inverse_rotation() {
    check(
        "rope",
        vec![real("Q", 1, 8, 41), real_with("RC", 1, 2, vec![0.6, 0.8]), real_with("RS", 1, 2, vec![0.8, 0.6])],
        "Qr",
        (1, 8),
        "Let Qr be rope of Q with cos RC and sin RS.\n",
        &["Q"],
    );
}

#[test]
fn a_partial_rope_across_heads_differentiates_and_passes_the_unrotated_dims_through() {
    check(
        "partial rope",
        vec![real("Q", 1, 12, 65), real_with("RC", 1, 2, vec![0.28, 0.96]), real_with("RS", 1, 2, vec![0.96, 0.28])],
        "Qr",
        (1, 12),
        "Let Qr be rope of Q with cos RC and sin RS across 2 heads.\n",
        &["Q"],
    );
}

#[test]
fn the_rope_tables_are_not_differentiated_and_asking_is_refused() {
    let e = refusal(
        vec![real("Q", 1, 8, 66), real_with("RC", 1, 2, vec![0.6, 0.8]), real_with("RS", 1, 2, vec![0.8, 0.6])],
        "Let Qr be rope of Q with cos RC and sin RS.\n",
        "Qr",
        (1, 8),
        &["Q", "RC"],
    );
    assert_eq!(e, VjpRefusal::NotDifferentiated { tensor: "RC".into(), op: "Rope" });
}

#[test]
fn an_integer_index_is_not_differentiated_and_asking_is_refused() {
    let e = refusal(
        vec![real("E", 3, 4, 67), ints("I", 1, 2, vec![3, 1])],
        "Let X be rows I of E.\n",
        "X",
        (2, 4),
        &["I"],
    );
    assert_eq!(e, VjpRefusal::NotDifferentiated { tensor: "I".into(), op: "RowsGather" });
}

#[test]
fn causal_conv_refuses_by_name_until_its_rule_is_written() {
    let e = refusal(
        vec![real("X0", 1, 3, 42), real("WC", 2, 3, 43), real_with("CS", 2, 3, vec![0.0; 6])],
        "Let Y0 be causal conv of X0 with weight WC state CS over 2 taps.\n",
        "Y0",
        (1, 3),
        &["X0"],
    );
    not_derived(e, "CausalConv");
}

#[test]
fn full_attention_differentiates_in_queries_keys_and_values() {
    check(
        "full attention",
        vec![real("Q", 3, 4, 44), real("K", 3, 4, 45), real("V", 3, 4, 46)],
        "C",
        (3, 4),
        "Let C be full attention of Q over K and V with 2 heads.\n",
        &["Q", "K", "V"],
    );
}

#[test]
fn grouped_query_attention_over_a_cache_differentiates_into_the_shared_kv_heads() {
    check(
        "attention",
        vec![real("q", 1, 8, 47), real("k", 3, 4, 48), real("v", 3, 4, 49)],
        "ct",
        (1, 8),
        "Let ct be attention of q over k and v with 4 heads 2 groups.\n",
        &["q", "k", "v"],
    );
}

#[test]
fn attention_up_to_a_position_differentiates_only_the_positions_it_reads() {
    check(
        "attention up to",
        vec![real("q", 1, 8, 69), real("k", 3, 4, 70), real("v", 3, 4, 71)],
        "ct",
        (1, 8),
        "Let ct be attention of q over k and v with 4 heads 2 groups up to 2.\n",
        &["q", "k", "v"],
    );
}

#[test]
fn a_scaled_attention_differentiates_by_its_own_scale() {
    // `scaled by S` takes each score as `S·q·k` in place of `q·k/√hd`: a rule that kept `1/√hd` differentiates another
    // function, here by a factor of 2.6 on every score.
    check(
        "scaled attention",
        vec![real("q", 1, 8, 81), real("k", 3, 4, 82), real("v", 3, 4, 83)],
        "ct",
        (1, 8),
        "Let ct be attention of q over k and v with 4 heads 2 groups up to 3 scaled by 1.3.\n",
        &["q", "k", "v"],
    );
    check(
        "scaled causal block",
        vec![real("Q", 2, 8, 84), real("K", 3, 4, 85), real("V", 3, 4, 86)],
        "C",
        (2, 8),
        "Let C be causal attention of Q over K and V with 4 heads 2 groups from 2 scaled by 0.4.\n",
        &["Q", "K", "V"],
    );
}

#[test]
fn a_windowed_attention_differentiates_only_inside_its_window() {
    // The keys and values before the window's floor reach no score: their gradients are zero, and those inside it are
    // the softmax's over the window alone.
    check(
        "sliding window",
        vec![real("q", 1, 8, 87), real("k", 5, 4, 88), real("v", 5, 4, 89)],
        "ct",
        (1, 8),
        "Let ct be attention of q over k and v with 4 heads 2 groups up to 5 within a window of 2.\n",
        &["q", "k", "v"],
    );
    check(
        "causal block over a window",
        vec![real("Q", 3, 8, 90), real("K", 5, 4, 91), real("V", 5, 4, 92)],
        "C",
        (3, 8),
        "Let C be causal attention of Q over K and V with 4 heads 2 groups from 3 within a window of 2.\n",
        &["Q", "K", "V"],
    );
    check(
        "chunks",
        vec![real("Q", 3, 8, 93), real("K", 5, 4, 94), real("V", 5, 4, 95)],
        "C",
        (3, 8),
        "Let C be causal attention of Q over K and V with 4 heads 2 groups from 3 within chunks of 2.\n",
        &["Q", "K", "V"],
    );
}

#[test]
fn an_attention_with_sinks_differentiates_into_its_sinks() {
    // Each head's sink joins its softmax's denominator: it takes weight from every position, and its own gradient is
    // minus its weight times the row's expected `dp`.
    check(
        "sinks",
        vec![real("q", 1, 8, 96), real("k", 3, 4, 97), real("v", 3, 4, 98), real("S", 1, 4, 99)],
        "ct",
        (1, 8),
        "Let ct be attention of q over k and v with 4 heads 2 groups up to 3 with sinks S.\n",
        &["q", "k", "v", "S"],
    );
}

#[test]
fn values_unlike_the_keys_in_width_differentiate() {
    // Keys 2 wide a head and values 3: the output and the values' gradient run 3 a head, the scores 2.
    check(
        "wide values",
        vec![real("q", 1, 8, 100), real("k", 3, 4, 101), real("v", 3, 6, 102)],
        "ct",
        (1, 12),
        "Let ct be attention of q over k and v with 4 heads 2 groups up to 3.\n",
        &["q", "k", "v"],
    );
}

#[test]
fn a_cap_differentiates_through_its_tanh() {
    // `C·tanh(s/C)` passes `1 − (y/C)²` of its input's gradient: on an attention's scores (scaled so the cap binds) and
    // on a tensor's elements.
    check(
        "capped attention",
        vec![real("q", 1, 8, 103), real("k", 3, 4, 104), real("v", 3, 4, 105)],
        "ct",
        (1, 8),
        "Let ct be attention of q over k and v with 4 heads 2 groups up to 3 scaled by 3.0 capped at 0.5.\n",
        &["q", "k", "v"],
    );
    check("capped tensor", vec![real("X", 2, 4, 106)], "Y", (2, 4), "Let Y be X capped at 0.3.\n", &["X"]);
}

#[test]
fn every_clause_of_an_attention_differentiates_together() {
    check(
        "every clause",
        vec![real("Q", 3, 8, 107), real("K", 5, 4, 108), real("V", 5, 6, 109), real("S", 1, 4, 110)],
        "C",
        (3, 12),
        "Let C be causal attention of Q over K and V with 4 heads 2 groups from 3 within a window of 3 scaled by 2.0 capped at 0.8 with sinks S.\n",
        &["Q", "K", "V", "S"],
    );
}

#[test]
fn a_causal_block_of_queries_differentiates_each_row_over_its_own_prefix() {
    check(
        "causal block",
        vec![real("Q", 2, 8, 72), real("K", 3, 4, 73), real("V", 3, 4, 74)],
        "C",
        (2, 8),
        "Let C be causal attention of Q over K and V with 4 heads 2 groups from 2.\n",
        &["Q", "K", "V"],
    );
}

/// A tensor of `rows × heads·hd` whose every head rides a large offset in its first dimension — `sign · 40` and a small
/// spread — over ordinary values in the rest. A query head and a key head riding the same sign score about `+1131`
/// (`1600/√2`), opposite signs about `−1131`, the spread between keys a few units either way.
fn riding(name: &'static str, rows: usize, signs: &[f64], hd: usize, seed: u64) -> Input {
    let cols = signs.len() * hd;
    let values = (0..rows * cols)
        .map(|i| match i % hd {
            0 => signs[(i % cols) / hd] * 40.0 + 0.05 * wv(seed, i),
            _ => wv(seed, i),
        })
        .collect();
    real_with(name, rows, cols, values)
}

/// The rule recomputes the forward's softmax, and recomputes it as the forward does: less the row's own largest score.
/// Scores far outside `exp`'s range — one head near `+1131`, the other near `−1131` — leave the softmax and its
/// gradient as they are at small scores, where subtracting anything else overflows `exp` on the first head or
/// underflows every weight of the second to zero. The causal form's first row attends one key alone, whose score is
/// then its own largest.
#[test]
fn attention_differentiates_at_scores_far_outside_exps_range() {
    check(
        "causal attention, scores far out",
        vec![riding("Q", 2, &[1.0, -1.0], 2, 94), riding("K", 2, &[1.0], 2, 95), real("V", 2, 2, 96)],
        "C",
        (2, 4),
        "Let C be causal attention of Q over K and V with 2 heads 1 groups from 1.\n",
        &["Q", "K", "V"],
    );
    check(
        "attention over a cache, scores far out",
        vec![riding("q", 1, &[1.0, -1.0], 2, 97), riding("k", 3, &[1.0], 2, 98), real("v", 3, 2, 99)],
        "ct",
        (1, 4),
        "Let ct be attention of q over k and v with 2 heads 1 groups.\n",
        &["q", "k", "v"],
    );
    check(
        "full attention, scores far out",
        vec![riding("Q", 2, &[1.0, -1.0], 2, 100), riding("K", 2, &[1.0, 1.0], 2, 101), real("V", 2, 4, 102)],
        "C",
        (2, 4),
        "Let C be full attention of Q over K and V with 2 heads.\n",
        &["Q", "K", "V"],
    );
}

#[test]
fn moe_refuses_by_name_until_its_rule_is_written() {
    let (d, dff, ne) = (4usize, 4usize, 2usize);
    let e = refusal(
        vec![real("X", 1, d, 50), real("Wr", d, ne, 51), real("Wg", ne * dff, d, 52), real("Wu", ne * dff, d, 53), real("Wd", ne * d, dff, 54)],
        "Let H be moe of X with router Wr gate Wg up Wu down Wd choosing 1 of 2.\n",
        "H",
        (1, d),
        &["X"],
    );
    not_derived(e, "Moe");
}

#[test]
fn a_row_write_routes_the_row_gradient_to_its_source_and_stops_it_at_the_table() {
    check(
        "set row",
        vec![real("T", 3, 2, 55), real("x", 1, 2, 56), real("M", 3, 2, 68)],
        "Y",
        (3, 2),
        "Set row 2 of T to x.\nLet Y be T hadamard M.\n",
        &["T", "x", "M"],
    );
}

#[test]
fn gated_delta_refuses_by_name_until_its_rule_is_written() {
    let (hv, hk, d) = (2usize, 1usize, 2usize);
    let (qn, vn) = (hk * d, hv * d);
    let e = refusal(
        vec![
            real("Q", 1, qn, 57),
            real("K", 1, qn, 58),
            real("V", 1, vn, 59),
            real("Z", 1, vn, 60),
            real_with("B", 1, hv, vec![0.6, 0.4]),
            real_with("A", 1, hv, vec![0.7, 0.3]),
            real("S", vn, d, 61),
            real_with("GN", 1, d, vec![1.1, 0.9]),
        ],
        &format!("Let O be gated delta of Q over K and V with gate Z beta B decay A state S norm GN with {hv} heads {hk} groups.\n"),
        "O",
        (1, vn),
        &["Q"],
    );
    not_derived(e, "GatedDelta");
}

#[test]
fn decay_refuses_by_name_until_its_rule_is_written() {
    let e = refusal(
        vec![real("Ar", 1, 2, 62), real("Dt", 1, 2, 63), real("Al", 1, 2, 64)],
        "Let D be decay of Ar with bias Dt rate Al.\n",
        "D",
        (1, 2),
        &["Ar"],
    );
    not_derived(e, "Decay");
}
