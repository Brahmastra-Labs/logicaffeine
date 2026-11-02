//! WebRTC signaling composition — bootstrapping a browser P2P call, in Logos.
//!
//! The WebRTC connection itself is browser-direct, but the SDP offer/answer that establish it must
//! be exchanged out-of-band — over the existing Logos transport (relay / `Sync`). This locks that
//! full signaling round-trip: the offerer mints a peer connection + data channel, creates an offer,
//! and sends it; the answerer receives the offer, sets it remote, creates an answer, and sends it
//! back; the offerer receives the answer and sets it remote — a complete two-way SDP handshake over
//! the relay, the way two conference peers negotiate a direct connection.

use logicaffeine_compile::interpret_for_ui;
use logicaffeine_system::relay::serve;
use std::time::Duration;

#[tokio::test]
async fn two_peers_complete_the_sdp_signaling_handshake_over_the_relay() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    // Answerer: peer connection, await the offer, set it remote, create + send the answer.
    let answerer = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"answerer\".\n\
         \x20   Let pc be peerConnection().\n\
         \x20   Let offerer be a PeerAgent at \"offerer\".\n\
         \x20   Await response from offerer into payload.\n\
         \x20   Call setRemoteDescription with pc and payload.\n\
         \x20   Let sdp be createAnswer(pc).\n\
         \x20   Send sdp to offerer.\n\
         \x20   Show payload.\n"
    );
    // Offerer: peer connection + data channel, create + send the offer, await + apply the answer.
    let offerer = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"offerer\".\n\
         \x20   Let pc be peerConnection().\n\
         \x20   Let chan be dataChannel(pc, \"media\").\n\
         \x20   Let sdp be createOffer(pc).\n\
         \x20   Call setLocalDescription with pc and sdp.\n\
         \x20   Let answerer be a PeerAgent at \"answerer\".\n\
         \x20   Sleep 300.\n\
         \x20   Send sdp to answerer.\n\
         \x20   Await response from answerer into payload.\n\
         \x20   Call setRemoteDescription with pc and payload.\n\
         \x20   Show payload.\n"
    );

    let a = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&answerer));
    let o = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&offerer));
    let (ar, or) = tokio::join!(a, o);
    let ar = ar.expect("answerer did not hang");
    let or = or.expect("offerer did not hang");

    assert!(
        or.error.is_none(),
        "offerer (create offer + send + await answer) ran clean: {:?}",
        or.error
    );
    assert!(
        ar.error.is_none(),
        "answerer (await offer + create + send answer) ran clean: {:?}",
        ar.error
    );
    // The answerer received the OFFER; the offerer received the ANSWER — the handshake closed.
    assert!(
        ar.lines.iter().any(|l| l == "offer"),
        "answerer must receive the offer SDP; got {:?}",
        ar.lines
    );
    assert!(
        or.lines.iter().any(|l| l == "answer"),
        "offerer must receive the answer SDP; got {:?}",
        or.lines
    );
}
