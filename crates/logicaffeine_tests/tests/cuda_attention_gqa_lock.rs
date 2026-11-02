//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ SINGLE-QUERY MULTI-HEAD GQA ATTENTION ★ — the decode-step attention AUTHORED IN LOGOS as a first-class op.
//!
//! `Let C be attention of Q over K and V with H heads KV groups` computes, for each query head `g`, the softmax-
//! weighted sum of the cached values: attend `Q_g` to kv head `(g-1)/(H/KV)` over all `L` cached keys, scale by
//! `1/√HD`, softmax, and combine the values. This is exactly one decode step of llama.cpp's attention (the query
//! is one token; K/V are the KV cache). It desugars to plain LOGOS loops so the VM runs it, and the CUDA emitter
//! recognizes the block to emit ONE parallel `attn_decode` kernel (a block per query head).
//!
//! Ground truth: the LOGOS `attention` op on the VM must equal a clean-room f64 GQA reference, byte-for-byte.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;

/// Deterministic, well-spread pseudo-value for element `i` of tensor `salt` — no RNG.
fn wv(salt: u64, i: usize) -> f64 {
    let h = (i as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 101) as f64 / 101.0 - 0.5) * 1.4
}

/// Clean-room single-query GQA attention reference (all f64). Q is `1×(h·hd)`, K/V are `l×(kv·hd)`.
fn attn_ref(q: &[f64], k: &[f64], v: &[f64], h: usize, kv: usize, hd: usize, l: usize) -> Vec<f64> {
    let group = h / kv;
    let scale = 1.0 / (hd as f64).sqrt();
    let kvd = kv * hd;
    let mut out = vec![0.0f64; h * hd];
    for g in 0..h {
        let kvh = g / group;
        // scores[j] = scale * Σ_e Q[g,e]·K[j,kvh,e]
        let scores: Vec<f64> = (0..l)
            .map(|j| {
                let s: f64 = (0..hd)
                    .map(|e| q[g * hd + e] * k[j * kvd + kvh * hd + e])
                    .sum();
                s * scale
            })
            .collect();
        // softmax
        let mx = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let den: f64 = scores.iter().map(|&s| (s - mx).exp()).sum();
        // out[g,e] = Σ_j p[j]·V[j,kvh,e]
        for e in 0..hd {
            let acc: f64 = (0..l)
                .map(|j| ((scores[j] - mx).exp() / den) * v[j * kvd + kvh * hd + e])
                .sum();
            out[g * hd + e] = acc;
        }
    }
    out
}

/// Emit the LOGOS `.lg` for a GQA attention of the given dims, filling Q/K/V from `wv`.
fn attn_lg(h: usize, kv: usize, hd: usize, l: usize) -> (String, Vec<f64>, Vec<f64>, Vec<f64>) {
    let (qn, kvn) = (h * hd, kv * hd);
    let q: Vec<f64> = (0..qn).map(|i| wv(1, i)).collect();
    let k: Vec<f64> = (0..l * kvn).map(|i| wv(2, i)).collect();
    let v: Vec<f64> = (0..l * kvn).map(|i| wv(3, i)).collect();
    let mut s = String::from("## Main\n");
    let mut decl = |name: &str, r: usize, c: usize, vals: &[f64]| {
        s.push_str(&format!("Let {name} be a {r} by {c} tensor of Real.\n"));
        for (i, val) in vals.iter().enumerate() {
            s.push_str(&format!("Set item {} of {name} to {:?}.\n", i + 1, val));
        }
    };
    decl("Q", 1, qn, &q);
    decl("K", l, kvn, &k);
    decl("V", l, kvn, &v);
    s.push_str(&format!(
        "Let C be attention of Q over K and V with {h} heads {kv} groups.\n"
    ));
    for o in 0..qn {
        s.push_str(&format!("Show item {} of C.\n", o + 1));
    }
    (s, q, k, v)
}

fn run_vm(src: &str) -> Vec<f64> {
    let out = vm_outcome(src);
    assert!(
        out.error.is_none(),
        "VM error running the attention forward: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .map(|t| t.parse().expect("f64"))
        .collect()
}

/// The GQA attention op, several shapes, each authored in LOGOS and run on the VM ≡ the clean-room reference.
#[test]
fn the_attention_op_authored_in_logos_matches_the_clean_room_reference_on_the_vm() {
    for &(h, kv, hd, l) in &[
        (4usize, 2usize, 4usize, 3usize),
        (2, 2, 8, 5),
        (8, 4, 4, 6),
        (4, 1, 6, 4),
    ] {
        let (src, q, k, v) = attn_lg(h, kv, hd, l);
        let vm = run_vm(&src);
        let reference = attn_ref(&q, &k, &v, h, kv, hd, l);
        assert_eq!(
            vm.len(),
            h * hd,
            "attention (h={h},kv={kv},hd={hd},l={l}) must emit {} outputs, got {}",
            h * hd,
            vm.len()
        );
        for (o, (g, r)) in vm.iter().zip(&reference).enumerate() {
            assert!(
                (g - r).abs() < 1e-9,
                "attention (h={h},kv={kv},hd={hd},l={l}) out[{o}]: VM {g} vs reference {r} (Δ={:e})",
                (g - r).abs()
            );
        }
    }
    println!("GQA attention op authored in LOGOS ≡ clean-room f64 reference on the VM across 4 shapes (Q·K/√HD → softmax → weighted V, grouped-query).");
}

/// The emitter LIFTS the desugared block to a typed `Stage::Attention` with the right dims, and the CUDA emission
/// contains the parallel `attn_decode` kernel (not the scalar loops).
#[test]
fn recognize_attention_lifts_the_op_and_emits_the_parallel_kernel() {
    use logicaffeine_compile::compile::compile_to_cuda_forward;
    let (src, _q, _k, _v) = attn_lg(8, 4, 4, 6);
    let cu = compile_to_cuda_forward(&src).expect("attention .lg must lower to CUDA");
    // The block was RECOGNIZED (not left as a scalar `<<<1,1>>>` device-lift) → the parallel `k{i}_attn` kernel
    // (one block per query head, warp-shuffle block softmax) appears with the right dims.
    assert!(
        cu.contains("_attn("),
        "the attention op must emit the parallel `k{{i}}_attn` kernel, not scalar loops:\n{}",
        &cu[..cu.len().min(500)]
    );
    assert!(
        cu.contains("_attn<<<8,256,"),
        "k{{i}}_attn must launch one block per query head (H=8)"
    );
    assert!(
        cu.contains(",6,8,4,4,"),
        "k{{i}}_attn launch must carry (L=6,H=8,KV=4,HD=4)"
    );
    assert!(
        cu.contains("__shfl_down_sync"),
        "k{{i}}_attn must use a warp-shuffle block softmax"
    );
    println!("recognize_attention lifts `attention of Q over K and V with 8 heads 4 groups` → parallel k{{i}}_attn kernel (L=6,H=8,KV=4,HD=4).");
}

/// An attention whose values are its keys (`over K and K`: a latent cache read as both) weighs the keys on the VM, as the
/// reference does with them as the values, and the shared front end lifts it with the one tensor as both operands.
#[cfg(feature = "codegen")]
#[test]
fn an_attention_over_one_tensor_as_keys_and_values_lifts_it_as_both() {
    use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
    for (h, kv, hd, l) in [(4usize, 2usize, 4usize, 3usize), (2, 1, 8, 5)] {
        let (src, q, k, _) = attn_lg(h, kv, hd, l);
        let src = src.replace("over K and V", "over K and K");
        let vm = run_vm(&src);
        let reference = attn_ref(&q, &k, &k, h, kv, hd, l);
        assert_eq!(vm.len(), h * hd, "h={h} kv={kv} hd={hd} l={l}");
        for (o, (g, r)) in vm.iter().zip(&reference).enumerate() {
            assert!((g - r).abs() < 1e-9, "h={h} kv={kv} hd={hd} l={l} out[{o}]: VM {g} vs reference {r}");
        }
        let rp = recognize_stages(&src).expect("parses");
        assert!(rp.unrecognized.is_empty(), "every statement lifts: {:?}", rp.unrecognized);
        let st = rp.stages.iter().find(|s| matches!(s.op, TensorOpSpec::Attention(_))).expect("the attention lifts");
        assert_eq!(st.ins, ["Q", "K", "K"], "the keys are the values too");
    }
}
