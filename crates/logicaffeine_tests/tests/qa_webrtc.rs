//! QA WebRTC locks — the browser P2P leg's dangerous edges. A mesh's worst silent failure is a
//! HANDLE COLLISION: if two peer connections or two data channels ever share a handle, one peer's
//! media routes to the wrong peer. These lock that every connection and channel in a mesh gets a
//! DISTINCT handle, that the byte-count carried in the send op is exact (including the empty-payload
//! boundary), and that the whole thing stays identical on the tree-walker and the VM.

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
        "tree-walker vs VM WebRTC op-log diverged:\n{src}"
    );
    assert_eq!(interp, expected, "op-log mismatch:\n{src}");
}

/// MESH SAFETY: every peer connection AND every data channel in a mesh gets a DISTINCT handle, so no
/// two peers' media can ever collide onto the same connection. Three peers, three channels, three
/// sends — all handles distinct, identically on both tiers.
#[test]
fn a_mesh_of_peers_gets_distinct_connection_and_channel_handles() {
    let src = "## Main\n\
        Let pc1 be peerConnection().\n\
        Let c1 be dataChannel(pc1, \"alice\").\n\
        Let pc2 be peerConnection().\n\
        Let c2 be dataChannel(pc2, \"bob\").\n\
        Let pc3 be peerConnection().\n\
        Let c3 be dataChannel(pc3, \"carol\").\n\
        Call channelSend with c1 and [1].\n\
        Call channelSend with c2 and [2, 2].\n\
        Call channelSend with c3 and [3, 3, 3].\n";
    assert_dom_parity(
        &src,
        &[
            DomOp::RtcPeerConnection { pc: 1 },
            DomOp::RtcDataChannel {
                pc: 1,
                channel: 2,
                label: "alice".into(),
            },
            DomOp::RtcPeerConnection { pc: 3 },
            DomOp::RtcDataChannel {
                pc: 3,
                channel: 4,
                label: "bob".into(),
            },
            DomOp::RtcPeerConnection { pc: 5 },
            DomOp::RtcDataChannel {
                pc: 5,
                channel: 6,
                label: "carol".into(),
            },
            DomOp::RtcSend {
                channel: 2,
                bytes: 1,
            },
            DomOp::RtcSend {
                channel: 4,
                bytes: 2,
            },
            DomOp::RtcSend {
                channel: 6,
                bytes: 3,
            },
        ],
    );

    // Belt-and-suspenders: prove the intent directly — the handles are pairwise distinct.
    let log = interp_log(src);
    let mut pcs: Vec<u32> = Vec::new();
    let mut channels: Vec<u32> = Vec::new();
    for op in &log {
        match op {
            DomOp::RtcPeerConnection { pc } => pcs.push(*pc),
            DomOp::RtcDataChannel { channel, .. } => channels.push(*channel),
            _ => {}
        }
    }
    let mut pcs_sorted = pcs.clone();
    pcs_sorted.sort_unstable();
    pcs_sorted.dedup();
    assert_eq!(
        pcs_sorted.len(),
        pcs.len(),
        "peer-connection handles must be pairwise distinct: {pcs:?}"
    );
    let mut ch_sorted = channels.clone();
    ch_sorted.sort_unstable();
    ch_sorted.dedup();
    assert_eq!(
        ch_sorted.len(),
        channels.len(),
        "data-channel handles must be pairwise distinct: {channels:?}"
    );
    // And no channel handle ever equals a peer-connection handle (disjoint handle spaces).
    for c in &channels {
        assert!(
            !pcs.contains(c),
            "a data-channel handle {c} collided with a peer-connection handle"
        );
    }
}

/// BOUNDARY: sending an EMPTY payload records exactly 0 bytes — no panic, no off-by-one, no fake
/// minimum. The degenerate send is a real case (an empty keepalive) and must be exact.
#[test]
fn an_empty_channel_send_records_exactly_zero_bytes() {
    let src = "## Main\n\
        Let pc be peerConnection().\n\
        Let chan be dataChannel(pc, \"keepalive\").\n\
        Call channelSend with chan and [].\n";
    assert_dom_parity(
        src,
        &[
            DomOp::RtcPeerConnection { pc: 1 },
            DomOp::RtcDataChannel {
                pc: 1,
                channel: 2,
                label: "keepalive".into(),
            },
            DomOp::RtcSend {
                channel: 2,
                bytes: 0,
            },
        ],
    );
}

/// FIDELITY: the send op reports the EXACT payload length, not a rounded or capped value — a 256-byte
/// media chunk records 256 bytes. Congestion accounting downstream depends on this being truthful.
#[test]
fn a_channel_send_reports_the_exact_payload_length() {
    let payload: Vec<String> = (0..256).map(|i| (i % 256).to_string()).collect();
    let src = format!(
        "## Main\n\
         Let pc be peerConnection().\n\
         Let chan be dataChannel(pc, \"media\").\n\
         Call channelSend with chan and [{}].\n",
        payload.join(", ")
    );
    assert_dom_parity(
        &src,
        &[
            DomOp::RtcPeerConnection { pc: 1 },
            DomOp::RtcDataChannel {
                pc: 1,
                channel: 2,
                label: "media".into(),
            },
            DomOp::RtcSend {
                channel: 2,
                bytes: 256,
            },
        ],
    );
}

/// COMPLETENESS: a peer may open MULTIPLE data channels on ONE connection (media + control), and each
/// channel gets its own distinct handle under the same peer connection — the multi-stream shape SVC
/// needs. Both channels ride pc 1; their handles differ.
#[test]
fn one_peer_connection_can_carry_multiple_distinct_channels() {
    let src = "## Main\n\
        Let pc be peerConnection().\n\
        Let media be dataChannel(pc, \"media\").\n\
        Let control be dataChannel(pc, \"control\").\n\
        Call channelSend with media and [9, 9, 9].\n\
        Call channelSend with control and [1].\n";
    assert_dom_parity(
        src,
        &[
            DomOp::RtcPeerConnection { pc: 1 },
            DomOp::RtcDataChannel {
                pc: 1,
                channel: 2,
                label: "media".into(),
            },
            DomOp::RtcDataChannel {
                pc: 1,
                channel: 3,
                label: "control".into(),
            },
            DomOp::RtcSend {
                channel: 2,
                bytes: 3,
            },
            DomOp::RtcSend {
                channel: 3,
                bytes: 1,
            },
        ],
    );
}
