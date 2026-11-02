use super::*;
use crate::model::spec::{Attention, GateActivation, Gating, GatedDelta, Layer, Mla, Moe, Rope, Temperature, Window};

fn rope(scaling: RopeScaling) -> Rope {
    Rope { dims: 16, theta: 10_000_000.0, neox: true, scaling, sections: Some([2, 2, 4, 0]) }
}

fn attention() -> Attention {
    Attention {
        heads: 4,
        kv_heads: 2,
        head_dim: 32,
        value_dim: 32,
        rope: Some(rope(RopeScaling::None)),
        qk_norm: true,
        output_gate: true,
        fused_qkv: false,
        bias: false,
        output_bias: false,
        window: None,
        sinks: false,
        softcap: None,
        scale: 1.0 / (32f64).sqrt(),
        temperature: None,
        qk_unit_norm: false,
        qk_norm_whole: false,
    }
}

fn layer(mixer: Mixer) -> Layer {
    Layer { mixer, ffn: Ffn::Dense { dff: 128, act: Act::Silu }, post_mixer_norm: false, post_ffn_norm: false }
}

/// A Qwen3.5-shaped model: a Gated-DeltaNet block, then an attention block, every feature the engine carries.
fn qwen35() -> ModelSpec {
    ModelSpec {
        arch: "qwen35".into(),
        d: 64,
        vocab: 256,
        ctx_len: 4096,
        rms_eps: 1e-6,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: vec![
            layer(Mixer::GatedDelta(GatedDelta { k_heads: 2, v_heads: 4, head_dim: 16, conv_taps: 4, gate: GateActivation::Silu })),
            layer(Mixer::Attention(attention())),
        ],
        mtp_blocks: 0,
        ffn_norm: "post_attention_norm",
        hyper: None,
        ple: None,
        indexer: None,
    }
}

/// Every tensor the program of `spec` reads, in `ty`, norms and vectors in f32.
fn directory(spec: &ModelSpec, ty: GgmlType) -> Vec<(String, GgmlType)> {
    lower(spec, EngineNumerics::precise(), spec.ctx_len)
        .unwrap()
        .program
        .tensors()
        .into_iter()
        .map(|n| (n.to_string(), if n.contains("norm") || n.contains("ssm_a") || n.contains("dt.bias") || n.contains("conv1d") { GgmlType::F32 } else { ty }))
        .collect()
}

#[test]
fn the_model_the_engine_was_built_for_is_admitted_in_every_format_it_reads() {
    let spec = qwen35();
    assert!(spec_refusals(&spec).is_empty());
    for ty in [
        GgmlType::Q4_0,
        GgmlType::Q4_1,
        GgmlType::Q5_0,
        GgmlType::Q5_1,
        GgmlType::Q8_0,
        GgmlType::Q2K,
        GgmlType::Q3K,
        GgmlType::Q4K,
        GgmlType::Q5K,
        GgmlType::Q6K,
        GgmlType::Iq4Nl,
        GgmlType::Iq4Xs,
        GgmlType::F16,
        GgmlType::Bf16,
        GgmlType::F32,
    ] {
        assert_eq!(admit(&spec, &directory(&spec, ty)), Ok(()), "{}", ty.name());
    }
}

#[test]
fn a_format_no_unit_reads_is_refused_by_tensor() {
    let spec = qwen35();
    let mut tensors = directory(&spec, GgmlType::Q4K);
    let k = tensors.iter().position(|(n, _)| n == "blk.1.attn_q.weight").unwrap();
    tensors[k].1 = GgmlType::Mxfp4;
    assert_eq!(
        refusals(&spec, &tensors),
        vec![Refusal::Format { tensor: "blk.1.attn_q.weight".into(), format: GgmlType::Mxfp4.name() }]
    );
}

#[test]
fn a_tensor_the_program_reads_must_be_in_the_checkpoint() {
    let spec = qwen35();
    let tensors: Vec<_> = directory(&spec, GgmlType::Q8_0).into_iter().filter(|(n, _)| n != "output.weight").collect();
    assert_eq!(refusals(&spec, &tensors), vec![Refusal::MissingTensor("output.weight".into())]);
    let mut tied = qwen35();
    tied.tied_output = true;
    let tensors: Vec<_> = directory(&tied, GgmlType::Q8_0);
    assert!(tensors.iter().all(|(n, _)| n != "output.weight"), "a tied head reads the embedding");
    assert_eq!(admit(&tied, &tensors), Ok(()));
}

#[test]
fn every_attention_variant_the_unit_lacks_is_named() {
    let cases: [(fn(&mut Attention), &str); 7] = [
        (|a| a.bias = true, "projection biases"),
        (|a| a.fused_qkv = true, "one fused query, key and value projection"),
        (|a| a.window = Some(Window::Sliding(512)), "a sliding window"),
        (|a| a.sinks = true, "attention sinks"),
        (|a| a.softcap = Some(50.0), "soft-capped scores"),
        (|a| a.temperature = Some(Temperature { floor: 8192.0, scale: 0.1 }), "a per-position query temperature"),
        (|a| a.qk_unit_norm = true, "a weightless QK norm"),
    ];
    for (change, what) in cases {
        let mut spec = qwen35();
        let Mixer::Attention(a) = &mut spec.layers[1].mixer else { unreachable!() };
        change(a);
        assert_eq!(spec_refusals(&spec), vec![Refusal::Attention { layer: 1, what }]);
    }
}

#[test]
fn rotations_the_tables_carry_are_admitted() {
    for scaling in [
        RopeScaling::None,
        RopeScaling::Factors,
        RopeScaling::Linear { factor: 4.0 },
        RopeScaling::Yarn { factor: 4.0, original_ctx: 4096, beta_fast: 32.0, beta_slow: 1.0, magnitude: 1.1 },
    ] {
        let mut spec = qwen35();
        let Mixer::Attention(a) = &mut spec.layers[1].mixer else { unreachable!() };
        a.rope = Some(rope(scaling.clone()));
        assert!(spec_refusals(&spec).is_empty(), "{scaling:?}");
    }
    let mut nope = qwen35();
    let Mixer::Attention(a) = &mut nope.layers[1].mixer else { unreachable!() };
    a.rope = None;
    assert!(spec_refusals(&nope).is_empty(), "a layer without positions");
}

/// A rotation scaled by frequency factors turns by the checkpoint's table of them, which the program does not read but
/// the rotary tables are written from: a checkpoint without it is refused by the table's name.
#[test]
fn a_rotation_scaled_by_frequency_factors_needs_the_checkpoints_table_of_them() {
    let mut spec = qwen35();
    let Mixer::Attention(a) = &mut spec.layers[1].mixer else { unreachable!() };
    a.rope = Some(rope(RopeScaling::Factors));
    let without = directory(&spec, GgmlType::Q8_0);
    assert!(without.iter().all(|(n, _)| n != "rope_freqs.weight"), "the program reads no factor table");
    assert_eq!(refusals(&spec, &without), vec![Refusal::MissingTensor("rope_freqs.weight".into())]);
    let mut with = without.clone();
    with.push(("rope_freqs.weight".into(), GgmlType::F32));
    assert_eq!(admit(&spec, &with), Ok(()));
    assert_eq!(admit(&qwen35(), &without), Ok(()), "an unscaled rotation needs no table");
}

#[test]
fn every_block_and_model_feature_the_lowering_lacks_is_named() {
    let mla = Mla {
        heads: 4,
        q_lora_rank: None,
        kv_lora_rank: 32,
        qk_nope: 16,
        qk_rope: 8,
        v_dim: 16,
        rope: rope(RopeScaling::None),
        scale: 0.2,
        kv_b_split: false,
    };
    let mut spec = qwen35();
    spec.layers[0].mixer = Mixer::Mla(mla);
    assert_eq!(spec_refusals(&spec), vec![Refusal::Mixer { layer: 0, what: "multi-head latent attention" }]);

    let mut spec = qwen35();
    spec.layers[1].ffn = Ffn::Dense { dff: 128, act: Act::GeluTanh };
    assert_eq!(spec_refusals(&spec), vec![Refusal::Ffn { layer: 1, what: "GELU" }]);

    let mut spec = qwen35();
    spec.layers[0].ffn = Ffn::Dense { dff: 128, act: Act::SwigluOai { alpha: 1.702, limit: 7.0 } };
    assert_eq!(spec_refusals(&spec), vec![Refusal::Ffn { layer: 0, what: "gpt-oss's clamped SwiGLU" }]);

    let mut spec = qwen35();
    spec.layers[1].ffn = Ffn::Moe(Moe {
        experts: 8,
        top_k: 2,
        expert_dff: 32,
        act: Act::Silu,
        gating: Gating::Softmax,
        norm_top_k: true,
        weights_scale: 1.0,
        shared_experts: 0,
        shared_dff: 0,
        shared_gate: false,
        biases: false,
        groups: None,
    });
    assert_eq!(spec_refusals(&spec), vec![Refusal::Ffn { layer: 1, what: "experts" }]);

    let mut spec = qwen35();
    spec.layers[0].post_mixer_norm = true;
    spec.layers[1].post_ffn_norm = true;
    spec.embedding_scale = Some(8.0);
    spec.final_logit_softcap = Some(30.0);
    assert_eq!(
        spec_refusals(&spec),
        vec![
            Refusal::Norm { layer: 0, what: "a norm on the mixer's output" },
            Refusal::Norm { layer: 1, what: "a norm on the feed-forward output" },
            Refusal::EmbeddingScale,
            Refusal::LogitSoftcap,
        ]
    );
}

#[test]
fn a_refused_model_is_not_lowered_to_check_its_tensors() {
    let mut spec = qwen35();
    spec.layers[1].ffn = Ffn::Dense { dff: 128, act: Act::GeluTanh };
    assert_eq!(refusals(&spec, &[]), vec![Refusal::Ffn { layer: 1, what: "GELU" }], "no tensor refusals on top");
}

#[test]
fn every_refusal_reads_as_a_sentence() {
    let all = [
        Refusal::Mixer { layer: 3, what: "multi-head latent attention" },
        Refusal::Ffn { layer: 2, what: "experts" },
        Refusal::Norm { layer: 1, what: "a norm on the mixer's output" },
        Refusal::Attention { layer: 0, what: "a sliding window" },
        Refusal::EmbeddingScale,
        Refusal::LogitSoftcap,
        Refusal::MissingTensor("output.weight".into()),
        Refusal::Format { tensor: "blk.0.ffn_up.weight".into(), format: "mxfp4" },
    ];
    let texts: Vec<String> = all.iter().map(|r| r.to_string()).collect();
    assert_eq!(
        texts,
        [
            "layer 3: multi-head latent attention",
            "layer 2: a feed-forward block of experts",
            "layer 1: a norm on the mixer's output",
            "layer 0: attention with a sliding window",
            "a scaled token embedding",
            "soft-capped logits",
            "tensor `output.weight` is not in the checkpoint",
            "tensor `blk.0.ffn_up.weight` is mxfp4, which no unit reads",
        ]
    );
}

#[test]
fn a_gap_stands_once_with_the_layers_it_is_in() {
    let mut refused = Vec::new();
    for layer in [0, 1, 2, 4, 6, 7] {
        refused.push(Refusal::Attention { layer, what: "a sliding window" });
    }
    for layer in 0..8 {
        refused.push(Refusal::Ffn { layer, what: "GELU" });
    }
    refused.push(Refusal::Norm { layer: 3, what: "a norm on the mixer's output" });
    refused.push(Refusal::EmbeddingScale);
    assert_eq!(
        summarize(&refused, 8),
        [
            "attention with a sliding window (layers 0–2, 4, 6–7)",
            "a feed-forward block of GELU (every layer)",
            "a norm on the mixer's output (layer 3)",
            "a scaled token embedding",
        ]
    );
    assert!(summarize(&[], 8).is_empty());
}
