//! samba_exam_correctness_reds_lock — the rows of the Samba book's exam E2 (section 10.2 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`, correctness against the references) still open, each a handed RED frozen in
//! `fixtures/frozen_reds.tsv`.
//!
//! A row runs the SN50 bundle `largo emit rdu` writes, read back off disk, certified and run through the spatial
//! evaluator as a host runs it (`common/samba_handoff.rs`, `common/samba_exam.rs`), against the f64 reference over the
//! same checkpoint. A prompt's state comes from the authored prefill on the GPU target and is handed to the bundle by
//! the decode's contract (`spatial::handoff`), so a row tests what it names and not the prefill's chip.
//!
//! A ticket that lands a row moves its test, unchanged, into `samba_exam_correctness_lock`.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_correctness_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;
#[path = "common/samba_exam.rs"]
#[allow(dead_code)]
mod samba_exam;

use logicaffeine_compile::spatial::handoff::{contract, Handoff, HandoffError};
use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_exam::{decode_batch_on_bundle, fp32_twin, reference_after, within_the_datapath};
use samba_handoff::{decode_on_bundle, prefill_on, Model, CTX, NEXT, PROMPT};
use std::collections::BTreeMap;
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn a_sequence_decodes_alike_alone_and_in_a_batch_of_64() {
    // Sequence 0 is the prompt; the other 63 stand at every position the cache holds, with state of their own, and
    // tokens of their own. Sequence 0's logits are the ones it decodes alone, bit for bit.
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let first = prefill_on(&m, &SpatialTarget::rtx3060(), &PROMPT);
        let alone = decode_on_bundle(&m, Some(&first));
        let c = contract(&m.spec, CTX).expect("the contract");
        let mut handed = vec![first];
        for b in 1..64usize {
            let state: BTreeMap<String, Vec<f64>> = c
                .carried
                .iter()
                .map(|s| (s.name.clone(), (0..s.rows * s.cols).map(|i| (((i * 7 + b * 13) % 23) as f64 - 11.0) / 32.0).collect()))
                .collect();
            handed.push(Handoff { position: b % (CTX - 1), state });
        }
        let toks: Vec<u32> = (0..64u32).map(|b| if b == 0 { NEXT } else { (b * 3 + 1) % m.spec.vocab as u32 }).collect();
        let batch = decode_batch_on_bundle(&m, &handed, &toks);
        assert_eq!(bits(&batch[0]), bits(&alone), "{t:?}: the sequence's logits beside 63 others are its logits alone");
    }
}

#[test]
fn the_last_position_decodes_and_the_next_is_refused() {
    // A prompt of CTX − 1 tokens fills every row of the cache but the last; the token after it is written there.
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let prompt: Vec<u32> = (0..CTX as u32 - 1).map(|i| (i * 5 + 3) % m.spec.vocab as u32).collect();
        let h = prefill_on(&m, &fp32_twin(), &prompt);
        assert_eq!(h.position, CTX - 1);
        let logits = decode_on_bundle(&m, Some(&h));
        within_the_datapath(&m, &logits, &reference_after(&m, &prompt, NEXT), &format!("{t:?} at the last position"));
        let full = Handoff { position: CTX, state: h.state.clone() };
        assert_eq!(
            contract(&m.spec, CTX).expect("the contract").check(&full),
            Err(HandoffError::Position { position: CTX, ctx: CTX }),
            "{t:?}: a decode past the cache's last row is refused by name"
        );
    }
}
