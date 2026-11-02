//! Wave 6 — conference media orchestration (the language/control-plane surface).
//!
//! A conference is assembled from the primitives built this session: rooms + presence are CRDTs
//! (`Shared`/`ConvergentCount`/`SharedSet`, proven under `potato`/`e2e_crdt`), peers are reached
//! with `Connect`/`Send … unreliably` (Wave 4), and the MEDIA plane is orchestrated with the
//! statement-level async handler `On <event> from <source>:` (Wave 3): `On track from peer:` wires
//! a per-remote-track callback, `On chunk from encoder:` wires a per-encoded-chunk callback (each
//! fires host-side from WebCodecs/WebRTC and hands Logos the opaque chunk to protect/encrypt/send).
//! Here we lock that the media-event handlers register on the right source for the right event, and
//! do so IDENTICALLY on the tree-walker and the VM — the conferencing vocabulary the platform speaks.
//! (The raw capture/encode/decode runtime is host-side WebCodecs; Logos holds the opaque handles.)

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom::{self, DomOp};

fn interp_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = tw_outcome(src);
    dom::op_log()
}

fn vm_log(src: &str) -> Vec<DomOp> {
    dom::reset();
    let _ = vm_outcome(src);
    dom::op_log()
}

fn assert_dom_parity(src: &str, expected: &[DomOp]) {
    let interp = interp_log(src);
    let vm = vm_log(src);
    assert_eq!(
        interp, vm,
        "tree-walker vs VM media-orchestration op-log diverged:\n{src}"
    );
    assert_eq!(interp, expected, "op-log mismatch:\n{src}");
}

/// A conference sender wires two media-event handlers — one per remote peer's track, one per
/// encoded chunk from the local encoder — and each registers on the correct opaque source handle
/// for the correct event, identically across tiers.
#[test]
fn conference_wires_per_peer_track_and_per_encoder_chunk_handlers() {
    let src = "## Main\n\
        Let peer be createElement(\"peer\").\n\
        Let encoder be createElement(\"encoder\").\n\
        On track from peer:\n\
        \x20   Call setText with peer and \"track-live\".\n\
        On chunk from encoder:\n\
        \x20   Call setText with encoder and \"chunk-ready\".\n";
    assert_dom_parity(
        src,
        &[
            DomOp::CreateElement {
                node: 1,
                tag: "peer".into(),
            },
            DomOp::CreateElement {
                node: 2,
                tag: "encoder".into(),
            },
            DomOp::AddEventListener {
                node: 1,
                event: "track".into(),
            },
            DomOp::AddEventListener {
                node: 2,
                event: "chunk".into(),
            },
        ],
    );
}
