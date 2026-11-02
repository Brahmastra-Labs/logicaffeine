use super::*;
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
            post_mixer_norm: false,
            post_ffn_norm: false,
        }],
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
    }
}

#[test]
fn an_identity_is_the_fips_sha256_of_the_source_text() {
    let empty = Identity::of(Cut::Decode { ctx: 1 }, "");
    assert_eq!(empty.hex(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    let abc = Identity::of(Cut::Decode { ctx: 1 }, "abc");
    assert_eq!(abc.hex(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn the_identity_line_names_the_cut_and_the_digest() {
    let id = Identity::of(Cut::Prefill { rows: 16, ctx: 1024 }, "abc");
    assert_eq!(
        id.line(),
        "LOGOS program: prefill chunk of 16 rows, ctx 1024, sha256 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let id = Identity::of(Cut::Decode { ctx: 64 }, "abc");
    assert!(id.line().starts_with("LOGOS program: decode step, ctx 64, sha256 "), "{}", id.line());
}

#[test]
fn each_cut_is_the_program_the_author_writes_for_it() {
    let s = spec();
    assert_eq!(Cut::Decode { ctx: 64 }.write(&s, ""), author::decode_step(&s, "", 64, 1, 1));
    assert_eq!(Cut::Prefill { rows: 4, ctx: 64 }.write(&s, ""), author::prefill(&s, "", 64, &[1, 1, 1, 1], 1));
    assert_eq!(Cut::Batch { rows: 2, ctx: 64 }.write(&s, "/w"), author::decode_batch(&s, "/w", 64, &[(1, 1), (1, 1)]));
    assert_eq!(Cut::DecodeLayer { layer: 0, ctx: 64 }.write(&s, "/w"), author::decode_layer(&s, 0, "/w", 64, 1));
    assert_eq!(Cut::DecodeEmbed.write(&s, ""), author::decode_embed(&s, "", 1));
    assert_eq!(Cut::DecodeHead.write(&s, ""), author::decode_head(&s, ""));
    assert_eq!(
        Cut::PrefillLayer { layer: 0, rows: 3, ctx: 64 }.write(&s, ""),
        author::stacked_layer(&s, 0, "", 64, &[1, 2, 3], author::RowState::Shared)
    );
    assert_eq!(
        Cut::BatchLayer { layer: 0, rows: 3, ctx: 64 }.write(&s, ""),
        author::stacked_layer(&s, 0, "", 64, &[1, 1, 1], author::RowState::PerSequence)
    );
}

#[test]
fn the_written_identity_is_the_digest_of_the_written_text() {
    let s = spec();
    for cut in [Cut::Decode { ctx: 64 }, Cut::Decode { ctx: 8 }, Cut::Prefill { rows: 2, ctx: 64 }] {
        let ((source, _, _), id) = written(&s, cut);
        assert_eq!(id, Identity::of(cut, &source), "{cut}");
        assert_eq!(id.cut, cut);
    }
}

#[test]
fn two_cuts_of_one_model_are_two_programs_and_one_cut_is_one_program() {
    let s = spec();
    let a = written(&s, Cut::Decode { ctx: 64 }).1;
    let b = written(&s, Cut::Decode { ctx: 8 }).1;
    let c = written(&s, Cut::Prefill { rows: 2, ctx: 64 }).1;
    assert_ne!(a.sha256, b.sha256, "the cache length is part of the decode step's text");
    assert_ne!(a.sha256, c.sha256);
    assert_eq!(a, written(&s, Cut::Decode { ctx: 64 }).1, "the author writes one text for one cut");
    let mut other = s.clone();
    other.vocab = 97;
    assert_ne!(a.sha256, written(&other, Cut::Decode { ctx: 64 }).1.sha256, "another model is another program");
}

#[test]
fn the_manifest_directory_never_enters_the_text() {
    let s = spec();
    for cut in [Cut::Decode { ctx: 64 }, Cut::DecodeLayer { layer: 0, ctx: 64 }, Cut::BatchEmbed { rows: 2 }, Cut::ForwardHead] {
        let (a, b) = (cut.write(&s, ""), cut.write(&s, "/w"));
        assert_eq!(a.0, b.0, "{cut}: the program text");
        assert_ne!((a.1, a.2), (b.1, b.2), "{cut}: only the manifest names the directory");
    }
}

#[test]
fn a_split_cut_names_its_chips_and_only_splittable_cuts_split() {
    let s = spec();
    let one = Identity::of(Cut::Decode { ctx: 64 }, "abc");
    let two = Identity::of_split(Cut::Decode { ctx: 64 }, 2, "abc");
    assert_eq!(one.chips, 1);
    assert!(two.line().starts_with("LOGOS program: decode step, ctx 64 over 2 chips, sha256 "), "{}", two.line());
    assert_ne!(one, two, "the same text split differently is another artifact's program");
    assert!(Cut::DecodeEmbed.write_split(&s, "", 2).is_err(), "an embedding is not written split");
    assert!(Cut::ForwardHead.write_split(&s, "", 2).is_err());
}
