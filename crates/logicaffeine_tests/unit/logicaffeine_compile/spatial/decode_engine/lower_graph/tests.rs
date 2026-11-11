use super::*;
use crate::tensor_graph::graph::TensorGraph;
use crate::model::spec::{Act, Attention, Ffn, Layer, Mixer, ModelSpec};

/// One attention layer and a dense SiLU FFN, 64 wide.
fn spec() -> ModelSpec {
    ModelSpec {
        arch: "llama".into(),
        d: 64,
        vocab: 96,
        ctx_len: 64,
        rms_eps: 1e-6,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: vec![Layer {
            mixer: Mixer::Attention(Attention {
                heads: 4,
                kv_heads: 2,
                head_dim: 16,
                value_dim: 16,
                rope: None,
                qk_norm: false,
                output_gate: false,
                fused_qkv: false,
                bias: false,
                output_bias: false,
                window: None,
                sinks: false,
                softcap: None,
                scale: 0.25,
                temperature: None,
                qk_unit_norm: false,
                qk_norm_whole: false,
            }),
            ffn: Ffn::Dense { dff: 96, act: Act::Silu },
            pre_norms: true,
            post_mixer_norm: false,
            post_ffn_norm: false,
        }],
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
        residual_scale: None,
        logit_scale: None,
        norm: crate::model::spec::Norm::Rms,
        head_bias: false,
        parts: Vec::new(),
    }
}

fn empty() -> TensorGraph {
    TensorGraph { identity: None, io: None, tensors: Default::default(), ops: Vec::new() }
}

fn numerics() -> EngineNumerics {
    EngineNumerics::all()[0]
}

#[test]
fn a_step_with_nothing_in_it_ends_in_the_embedding() {
    assert_eq!(lower_graph(&empty(), &spec(), numerics()), Err(LowerError::Ended { block: "the embedding".into(), expected: "RowGather".into() }));
}

#[test]
fn what_the_engine_has_no_unit_for_is_refused_by_name_before_the_step_is_read() {
    let mut scaled = spec();
    scaled.embedding_scale = Some(8.0);
    assert_eq!(lower_graph(&empty(), &scaled, numerics()), Err(LowerError::Unsupported { layer: None, what: "a scaled embedding".into() }));
    let mut capped = spec();
    capped.final_logit_softcap = Some(30.0);
    assert_eq!(lower_graph(&empty(), &capped, numerics()), Err(LowerError::Unsupported { layer: None, what: "capped logits".into() }));
}

#[test]
fn every_refusal_says_where_and_what() {
    let shown = [
        LowerError::Unexpected { op: 7, kind: "Silu", block: "layer 2's FFN".into(), expected: "Matmul".into() }.to_string(),
        LowerError::Dataflow { op: 9, block: "the head".into(), expected: "xf".into(), found: "xo3".into() }.to_string(),
        LowerError::Unsupported { layer: Some(4), what: "attention sinks".into() }.to_string(),
        LowerError::Binding { constant: "Wq3".into(), why: "bound through Same, not as a whole matrix".into() }.to_string(),
    ];
    assert_eq!(shown[0], "op 7 (Silu) in layer 2's FFN, where Matmul comes next");
    assert_eq!(shown[1], "op 9 in the head reads xo3, where it reads xf");
    assert_eq!(shown[2], "layer 4: the engine has no unit for attention sinks");
    assert_eq!(shown[3], "Wq3: bound through Same, not as a whole matrix");
}
