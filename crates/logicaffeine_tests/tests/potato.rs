//! Wave 7 — "runs on a potato" elastic media adaptation + the end-to-end potato pipeline.
//!
//! Quality is CONTINUOUS: `adaptRate` is an AIMD bitrate controller (multiplicative decrease
//! under loss, additive increase toward the measured link capacity, capped at 90% so the jitter
//! buffer keeps slack) — it glides smoothly across the whole range instead of snapping between
//! tiers. `elasticBuffer` grows/shrinks the jitter buffer with the link. `tierName` is a display
//! label derived from the continuous rate. The capstone composes the whole stack built this
//! session — the LGB1 wire codec (`wireBytes`, the compact structured-presence encoder), the
//! ChaCha20 cipher (`encrypt`/`decrypt`), and the Reed-Solomon math codec (`fecProtect`/
//! `fecRestore`) — and proves a frame survives packet loss byte-exact. These are the spec; they
//! bite (a broken controller diverges from the AIMD fixed point; a broken pipeline garbles the
//! recovered frame).

mod common;
use common::assert_all_tiers;
use logicaffeine_compile::compile::tw_outcome;

/// AIMD core: fast multiplicative back-off under loss, gentle additive climb when clean, a hard
/// cap at 90% of the link, and a minimum trickle floor.
#[test]
fn adapt_rate_is_a_correct_aimd_controller() {
    assert_all_tiers(
        "## Main\n\
         Show adaptRate(1000, 5000, 10).\n\
         Show adaptRate(1000, 5000, 0).\n\
         Show adaptRate(4600, 5000, 0).\n\
         Show adaptRate(10, 20, 50).\n",
        "600\n1200\n4500\n8",
    );
}

/// Continuity/elasticity: driven repeatedly on a clean fat link, the rate glides up and settles
/// exactly at the 90%-of-link cap (the AIMD fixed point) — no overshoot, no oscillation.
#[test]
fn continuous_rate_converges_smoothly_to_the_cap() {
    assert_all_tiers(
        "## Main\n\
         Let mutable r be 500.\n\
         Repeat for i from 1 to 100:\n\
         \x20   Set r to adaptRate(r, 5000, 0).\n\
         Show r.\n",
        "4500",
    );
}

/// The jitter buffer is elastic: it grows under jitter + loss and shrinks back on a clean link.
#[test]
fn elastic_buffer_grows_under_stress_and_shrinks_when_clean() {
    assert_all_tiers(
        "## Main\n\
         Show elasticBuffer(100, 80, 10).\n\
         Show elasticBuffer(200, 5, 0).\n",
        "220\n180",
    );
}

/// The presentation label tracks the continuous rate across the whole range.
#[test]
fn tier_label_tracks_the_continuous_rate() {
    assert_all_tiers(
        "## Main\n\
         Show tierName(2500).\n\
         Show tierName(150).\n\
         Show tierName(10).\n",
        "video-hd\navatar\ntranscript",
    );
}

/// General one-to-many streaming: ONE cumulative layer set, and every receiver pulls the richest
/// quality its own link sustains — a 2G phone, a DSL line, cable, fibre, gigabit — each a different
/// number of layers from the SAME encode. This is the scalable-streaming heart, not a per-call hack.
#[test]
fn scalable_layers_give_every_receiver_its_own_quality() {
    assert_all_tiers(
        "## Main\n\
         Let layers be [100, 400, 1000, 2500, 8000].\n\
         Show layersFor(50, layers).\n\
         Show layersFor(500, layers).\n\
         Show layersFor(1500, layers).\n\
         Show layersFor(3000, layers).\n\
         Show layersFor(9000, layers).\n",
        "0\n2\n3\n4\n5",
    );
}

/// The CRDT half of the format: structured state (roster, reactions, whiteboard, pose keypoints)
/// ships as DELTAS, not whole frames. The current merged state is the "keyframe" a late-joiner
/// receives; each delta is a "P-frame" merged in. A replica that only saw its own +5 and a remote
/// delta of +3 converges to 8 by MERGE alone — no central tally, no full-state resend — which is
/// why CRDT deltas scale to huge rooms and pair naturally with `unreliably` + FEC.
///
/// The counter merge is a REAL state-based CRDT (per-replica `GCounter`), so it is IDEMPOTENT:
/// merging the same delta TWICE — exactly what an unreliable link + FEC redelivery does — still
/// converges to 8, never 11. This is the property the streaming format leans on; a bare op-based
/// counter would double-count every retransmitted frame.
#[test]
fn crdt_deltas_stream_the_diff_and_converge() {
    assert_all_tiers(
        "## Definition\n\
         A Reactions is Shared and has:\n\
         \x20   a thumbsUp, which is ConvergentCount.\n\
         \n\
         ## Main\n\
         Let mutable local be a new Reactions.\n\
         Let mutable delta be a new Reactions.\n\
         Increase local's thumbsUp by 5.\n\
         Increase delta's thumbsUp by 3.\n\
         Merge delta into local.\n\
         Merge delta into local.\n\
         Show local's thumbsUp.\n",
        "8",
    );
}

/// The capstone: a structured-presence frame is compacted by the LGB1 wire codec, encrypted with
/// ChaCha20, split into loss-resilient packets by the Reed-Solomon math codec, LOSES TWO packets
/// in transit, and is still reconstructed and decrypted BYTE-EXACT — the whole potato stack in
/// one line of Logos each.
#[test]
fn capstone_potato_pipeline_recovers_a_frame_byte_exact_after_loss() {
    let program = "## Main\n\
        Let pose be [128, 64, 200, 33, 90, 150, 12, 240, 7, 199].\n\
        Let payload be wireBytes(pose).\n\
        Let key be [word32(1), word32(2), word32(3), word32(4), word32(5), word32(6), word32(7), word32(8)].\n\
        Let nonce be [word32(9), word32(10), word32(11)].\n\
        Let sealed be encrypt(key, nonce, payload, word32(1)).\n\
        Let packets be fecProtect(sealed, 4, 2).\n\
        Let p1 be item 1 of packets.\n\
        Let p3 be item 3 of packets.\n\
        Let p4 be item 4 of packets.\n\
        Let p6 be item 6 of packets.\n\
        Let received be [p1, nothing, p3, p4, nothing, p6].\n\
        Let recovered be fecRestore(received, 4).\n\
        Let opened be decrypt(key, nonce, recovered, word32(1)).\n\
        Show payload.\n\
        Show opened.\n";
    let r = tw_outcome(program);
    assert_eq!(
        r.error, None,
        "the potato pipeline runs clean: {:?}",
        r.error
    );
    let lines: Vec<&str> = r.output.trim().lines().collect();
    assert_eq!(lines.len(), 2, "payload then recovered payload");
    assert!(
        !lines[0].is_empty() && lines[0] != "[]",
        "the wire-codec payload is a non-empty compact encoding: {:?}",
        lines[0]
    );
    assert_eq!(
        lines[0], lines[1],
        "wire-codec → encrypt → FEC → lose 2 of 6 → restore → decrypt must recover the frame byte-exact"
    );
}
