//! WebRTC — the browser P2P leg, orchestrated in Logos.
//!
//! A conference peer drives the full WebRTC handshake from Logos: mint an `RTCPeerConnection`, open
//! a `DataChannel`, create + exchange the SDP offer/answer, add the remote ICE candidate, and send
//! media over the channel. The real peer connection and data channel run host-side in the browser
//! (web-sys); here we lock that Logos issues the handshake in the CORRECT ORDER with the right
//! handles, IDENTICALLY on the tree-walker and the VM — the browser-P2P vocabulary the platform
//! speaks. (Signaling — carrying the offer/answer/ICE between peers — rides the existing relay /
//! `Sync`, proven in `conference_net`.)

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

/// The offerer: open a peer connection + data channel, create the offer and set it local, take the
/// remote answer + ICE, then send a media chunk down the channel — the caller side of a P2P call.
#[test]
fn a_peer_drives_the_full_offerer_handshake_and_sends_media() {
    let src = "## Main\n\
        Let pc be peerConnection().\n\
        Let chan be dataChannel(pc, \"media\").\n\
        Let offer be createOffer(pc).\n\
        Call setLocalDescription with pc and offer.\n\
        Call setRemoteDescription with pc and \"answer-sdp\".\n\
        Call addIceCandidate with pc and \"candidate:1 udp\".\n\
        Call channelSend with chan and [10, 20, 30].\n";
    assert_dom_parity(
        src,
        &[
            DomOp::RtcPeerConnection { pc: 1 },
            DomOp::RtcDataChannel {
                pc: 1,
                channel: 2,
                label: "media".into(),
            },
            DomOp::RtcCreateOffer { pc: 1 },
            DomOp::RtcSetLocal {
                pc: 1,
                sdp: "offer".into(),
            },
            DomOp::RtcSetRemote {
                pc: 1,
                sdp: "answer-sdp".into(),
            },
            DomOp::RtcAddIce {
                pc: 1,
                candidate: "candidate:1 udp".into(),
            },
            DomOp::RtcSend {
                channel: 2,
                bytes: 3,
            },
        ],
    );
}

/// The answerer: take the remote offer, create the answer and set it local, add the remote ICE —
/// the callee side, completing the symmetric handshake.
#[test]
fn a_peer_drives_the_answerer_handshake() {
    let src = "## Main\n\
        Let pc be peerConnection().\n\
        Call setRemoteDescription with pc and \"offer-sdp\".\n\
        Let answer be createAnswer(pc).\n\
        Call setLocalDescription with pc and answer.\n\
        Call addIceCandidate with pc and \"candidate:2 udp\".\n";
    assert_dom_parity(
        src,
        &[
            DomOp::RtcPeerConnection { pc: 1 },
            DomOp::RtcSetRemote {
                pc: 1,
                sdp: "offer-sdp".into(),
            },
            DomOp::RtcCreateAnswer { pc: 1 },
            DomOp::RtcSetLocal {
                pc: 1,
                sdp: "answer".into(),
            },
            DomOp::RtcAddIce {
                pc: 1,
                candidate: "candidate:2 udp".into(),
            },
        ],
    );
}
