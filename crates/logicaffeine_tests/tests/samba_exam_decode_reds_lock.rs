//! samba_exam_decode_reds_lock — the rows of the Samba book's exam E3 (section 10.3 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! decode at the published batch sizes) still open, each a handed RED frozen in `fixtures/frozen_reds.tsv`.
//!
//!   • E3.2 — D8, D16 and D64: the SN50 bundle `largo emit rdu --batch B` writes reads every weight once a step for all B
//!     sequences. In each program a layer runs, every tensor the manifest binds to a checkpoint file is loaded by one
//!     section, whatever B is.
//!
//! A ticket that lands a row moves its test, unchanged, into `samba_exam_decode_lock`.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_decode_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use logicaffeine_compile::spatial::ir::StageKind;
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use samba_handoff::{Model, CTX};
use std::collections::BTreeSet;
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// The weights `largo emit rdu --chip sn50 --model <m> --ctx CTX --batch <batch>` binds that more than one section of
/// their program loads — `layer/tensor`, with the number of sections — and how many weights it binds in all.
fn weights_read_twice(m: &Model, batch: usize) -> (Vec<String>, usize) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("bundle");
    let o = cli::largo_in(
        dir.path(),
        &["emit", "rdu", "--chip", "sn50", "--model", m.path.to_str().unwrap(), "--ctx", &CTX.to_string(), "--batch", &batch.to_string(), "-o", out.to_str().unwrap()],
    );
    assert_eq!(o.status.code(), Some(0), "largo emit rdu --batch {batch} failed:\n{}", cli::stderr(&o));
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).expect("manifest.json")).unwrap();
    assert_eq!(manifest["batch"], batch, "the bundle is for {batch} sequences");
    let (mut twice, mut weights) = (Vec::new(), 0);
    for layer in manifest["layers"].as_array().expect("layers") {
        let kind = &manifest["kinds"][layer["kind"].as_u64().expect("a kind") as usize];
        let dir = kind["dir"].as_str().expect("a kind's directory");
        let p = parse_neutral(&std::fs::read_to_string(out.join(dir).join("neutral.json")).expect("neutral.json")).expect("the program parses");
        for e in layer["tensors"].as_array().expect("tensors").iter().filter(|e| e["role"] == "param" && !e["file"].is_null()) {
            let name = e["program"].as_str().expect("a program name");
            let w = p.tensors.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("`{name}` is in {dir}"));
            let sections: BTreeSet<Option<usize>> = p
                .stages
                .iter()
                .filter(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == w.id))
                .map(|s| p.section_of_stage(s.id))
                .collect();
            weights += 1;
            if sections.len() > 1 {
                twice.push(format!("layer {} `{}` ({} sections)", layer["layer"], e["layer"].as_str().unwrap_or(name), sections.len()));
            }
        }
    }
    (twice, weights)
}

#[test]
fn batched_decode_reads_every_weight_once_a_step() {
    let mut wrong = Vec::new();
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        for batch in [8, 16, 64] {
            let (twice, weights) = weights_read_twice(&m, batch);
            assert!(weights >= 8, "{t:?} at {batch}: the bundle binds its weights ({weights})");
            if !twice.is_empty() {
                wrong.push(format!("{t:?} at {batch}: {} of {weights} weights read by more than one section: {}", twice.len(), twice.join(", ")));
            }
        }
    }
    assert!(wrong.is_empty(), "every weight read once a step for the whole batch:\n  {}", wrong.join("\n  "));
}
