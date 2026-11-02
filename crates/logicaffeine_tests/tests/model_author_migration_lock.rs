//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE GENERIC AUTHOR WRITES QWEN3.8 EXACTLY AS ITS OWN GENERATORS DID.
//!
//! `model::author` writes any `ModelSpec` as a LOGOS program — the forward, a decode step, a prefill chunk, a batch,
//! and each of them cut into embedding, layers and head. Qwen3.8-27B was written by its own generators
//! (`codegen_cuda::qwen38::gen_qwen38_*`), and every Qwen3.8 lock, bundle and certified lowering rests on their
//! text. Moving the model onto the generic author must not move that text by one byte:
//!
//!   • `the_author_writes_every_qwen38_program_its_generators_write` compares the two over every form at the tiny,
//!     mini and real configurations;
//!   • `the_qwen38_programs_are_the_pinned_ones` holds both to `fixtures/qwen38_authored_digests.txt`, recorded from
//!     the generators before the move (regenerate only with `LOGOS_UPDATE_FIXTURES=1`, and say why) — so the pin
//!     survives the generators becoming calls into the author;
//!   • `the_tensor_parallel_programs_are_the_pinned_ones` does the same for every program split over chips, its
//!     placement manifest included (`fixtures/qwen38_tp_authored_digests.txt`).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

use std::collections::HashMap;

use logicaffeine_compile::codegen_cuda::qwen38::*;
use logicaffeine_compile::crypto_accel::sha256;
use logicaffeine_compile::model::author::{self, RowState};

type Prog = (String, HashMap<String, (String, usize)>, HashMap<String, String>);

fn digest(p: &Prog) -> String {
    let mut text = p.0.clone();
    let mut f32w: Vec<_> = p.1.iter().collect();
    f32w.sort();
    for (k, (file, n)) in f32w {
        text.push_str(&format!("\u{0}f32 {k} {file} {n}"));
    }
    let mut q8w: Vec<_> = p.2.iter().collect();
    q8w.sort();
    for (k, file) in q8w {
        text.push_str(&format!("\u{0}q8 {k} {file}"));
    }
    sha256(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn configs() -> Vec<(&'static str, Qwen38Cfg, usize)> {
    vec![
        ("tiny", Qwen38Cfg::tiny(), 64),
        ("mini", Qwen38Cfg::mini(), 64),
        ("real", Qwen38Cfg::real(), 4096),
    ]
}

/// Every form, by name, as the generators write it.
fn by_generators() -> Vec<(String, Prog)> {
    let mut out = Vec::new();
    for (name, cfg, ctx) in configs() {
        let mut push = |form: String, p: Prog| out.push((format!("{name}/{form}"), p));
        push("forward".into(), gen_qwen38_lg(&cfg, "/w"));
        push("decode_step".into(), gen_qwen38_decode_step_lg(&cfg, "/w", ctx, 3, 2));
        push("prefill".into(), gen_qwen38_prefill_lg(&cfg, "/w", ctx, &[3, 5, 7], 2));
        push("decode_batch".into(), gen_qwen38_decode_batch_lg(&cfg, "/w", ctx, &[(3, 2), (5, 4)]));
        push("head".into(), gen_qwen38_head_lg(&cfg, "/w"));
        push("decode_embed".into(), gen_qwen38_decode_embed_lg(&cfg, "/w", 3));
        push("decode_head".into(), gen_qwen38_decode_head_lg(&cfg, "/w"));
        push("batch_embed".into(), gen_qwen38_decode_batch_embed_lg(&cfg, "/w", &[3, 5]));
        push("prefill_head".into(), gen_qwen38_prefill_head_lg(&cfg, "/w", 3));
        push("batch_head".into(), gen_qwen38_decode_batch_head_lg(&cfg, "/w", 2));
        for l in 0..cfg.layers {
            push(format!("layer{l}"), gen_qwen38_layer_lg(&cfg, l, "/w"));
            push(format!("decode_layer{l}"), gen_qwen38_decode_layer_lg(&cfg, l, "/w", ctx, 2));
            push(format!("batch_layer{l}"), gen_qwen38_decode_batch_layer_lg(&cfg, l, "/w", ctx, &[2, 4]));
            push(format!("prefill_layer{l}"), gen_qwen38_prefill_layer_lg(&cfg, l, "/w", ctx, 2, 3));
        }
    }
    out
}

/// Every form, by name, as the generic author writes it from the configuration's spec.
fn by_author() -> Vec<(String, Prog)> {
    let mut out = Vec::new();
    for (name, cfg, ctx) in configs() {
        let spec = cfg.spec();
        let mut push = |form: String, p: Prog| out.push((format!("{name}/{form}"), p));
        push("forward".into(), author::forward(&spec, "/w"));
        push("decode_step".into(), author::decode_step(&spec, "/w", ctx, 3, 2));
        push("prefill".into(), author::prefill(&spec, "/w", ctx, &[3, 5, 7], 2));
        push("decode_batch".into(), author::decode_batch(&spec, "/w", ctx, &[(3, 2), (5, 4)]));
        push("head".into(), author::head(&spec, "/w"));
        push("decode_embed".into(), author::decode_embed(&spec, "/w", 3));
        push("decode_head".into(), author::decode_head(&spec, "/w"));
        push("batch_embed".into(), author::decode_batch_embed(&spec, "/w", &[3, 5]));
        push("prefill_head".into(), author::prefill_head(&spec, "/w", 3));
        push("batch_head".into(), author::decode_batch_head(&spec, "/w", 2));
        for l in 0..cfg.layers {
            push(format!("layer{l}"), author::layer(&spec, l, "/w"));
            push(format!("decode_layer{l}"), author::decode_layer(&spec, l, "/w", ctx, 2));
            push(
                format!("batch_layer{l}"),
                author::stacked_layer(&spec, l, "/w", ctx, &[2, 4], RowState::PerSequence),
            );
            push(
                format!("prefill_layer{l}"),
                author::stacked_layer(&spec, l, "/w", ctx, &[2, 3, 4], RowState::Shared),
            );
        }
    }
    out
}

#[test]
fn the_author_writes_every_qwen38_program_its_generators_write() {
    let (gens, auth) = (by_generators(), by_author());
    assert_eq!(gens.len(), auth.len());
    for ((name, g), (name2, a)) in gens.iter().zip(&auth) {
        assert_eq!(name, name2);
        if g.0 != a.0 {
            let at = g.0.lines().zip(a.0.lines()).position(|(x, y)| x != y).unwrap_or(0);
            panic!(
                "{name}: the author's line {} differs\n  generator: {:?}\n  author:    {:?}",
                at + 1,
                g.0.lines().nth(at),
                a.0.lines().nth(at)
            );
        }
        assert_eq!(g.1, a.1, "{name}: the f32 weight manifest");
        assert_eq!(g.2, a.2, "{name}: the q8 weight manifest");
    }
}

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qwen38_authored_digests.txt")
}

#[test]
fn the_qwen38_programs_are_the_pinned_ones() {
    let render = |progs: Vec<(String, Prog)>| -> String {
        progs.iter().map(|(n, p)| format!("{n} {}\n", digest(p))).collect()
    };
    let got = render(by_generators());
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok_and(|v| v == "1") {
        std::fs::write(fixture(), &got).expect("write the fixture");
        return;
    }
    let want = std::fs::read_to_string(fixture())
        .expect("no fixture — record it once, against the generators before the move, with LOGOS_UPDATE_FIXTURES=1");
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w, "a Qwen3.8 program moved");
    }
    assert_eq!(got.lines().count(), want.lines().count(), "the forms pinned");
    assert_eq!(render(by_author()), want, "the author writes the pinned programs");
}

type TpProg = (
    String,
    HashMap<String, (String, usize)>,
    HashMap<String, String>,
    logicaffeine_compile::spatial::tensor_parallel::TpManifest,
);

fn tp_digest(p: &TpProg) -> String {
    let manifest = serde_json::to_string(&p.3).expect("the manifest serializes");
    let base = digest(&(p.0.clone(), p.1.clone(), p.2.clone()));
    sha256(format!("{base}\u{0}{manifest}").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Every tensor-parallel form, by name, as the generators write it: the mini model over 2 and 4 chips, the real one
/// over 4 and 8.
fn tp_by_generators() -> Vec<(String, TpProg)> {
    let mut out = Vec::new();
    for (name, cfg, ctx, widths) in [
        ("mini", Qwen38Cfg::mini(), 64, [2usize, 4]),
        ("real", Qwen38Cfg::real(), 4096, [4, 8]),
    ] {
        for tp in widths {
            let mut push = |form: String, p: TpProg| out.push((format!("{name}/tp{tp}/{form}"), p));
            push("decode_step".into(), gen_qwen38_tp_decode_step_lg(&cfg, "/w", ctx, 3, 2, tp));
            push("decode_batch".into(), gen_qwen38_decode_batch_tp_lg(&cfg, "/w", ctx, &[(3, 2), (5, 4)], tp));
            for l in 0..cfg.layers {
                push(format!("decode_layer{l}"), gen_qwen38_tp_decode_layer_lg(&cfg, l, "/w", ctx, 2, tp));
                push(format!("batch_layer{l}"), gen_qwen38_decode_batch_tp_layer_lg(&cfg, l, "/w", ctx, &[2, 4], tp));
            }
        }
    }
    out
}

/// Every tensor-parallel form, by name, as the generic author writes it.
fn tp_by_author() -> Vec<(String, TpProg)> {
    let mut out = Vec::new();
    for (name, cfg, ctx, widths) in [
        ("mini", Qwen38Cfg::mini(), 64, [2usize, 4]),
        ("real", Qwen38Cfg::real(), 4096, [4, 8]),
    ] {
        let spec = cfg.spec();
        for tp in widths {
            let mut push = |form: String, p: TpProg| out.push((format!("{name}/tp{tp}/{form}"), p));
            push("decode_step".into(), author::tp_decode_step(&spec, "/w", ctx, 3, 2, tp).expect("splits"));
            push("decode_batch".into(), author::tp_decode_batch(&spec, "/w", ctx, &[(3, 2), (5, 4)], tp).expect("splits"));
            for l in 0..cfg.layers {
                push(format!("decode_layer{l}"), author::tp_decode_layer(&spec, l, "/w", ctx, 2, tp).expect("splits"));
                push(format!("batch_layer{l}"), author::tp_stacked_layer(&spec, l, "/w", ctx, &[2, 4], tp).expect("splits"));
            }
        }
    }
    out
}

fn tp_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qwen38_tp_authored_digests.txt")
}

#[test]
fn the_tensor_parallel_programs_are_the_pinned_ones() {
    let got: String = tp_by_generators()
        .iter()
        .map(|(n, p)| format!("{n} {}\n", tp_digest(p)))
        .collect();
    if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok_and(|v| v == "1") {
        std::fs::write(tp_fixture(), &got).expect("write the fixture");
        return;
    }
    let want = std::fs::read_to_string(tp_fixture())
        .expect("no fixture — record it once, against the generators before the move, with LOGOS_UPDATE_FIXTURES=1");
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w, "a tensor-parallel Qwen3.8 program moved");
    }
    assert_eq!(got.lines().count(), want.lines().count(), "the forms pinned");
    let authored: String = tp_by_author()
        .iter()
        .map(|(n, p)| format!("{n} {}\n", tp_digest(p)))
        .collect();
    for (a, w) in authored.lines().zip(want.lines()) {
        assert_eq!(a, w, "the author writes the pinned tensor-parallel program");
    }
}
