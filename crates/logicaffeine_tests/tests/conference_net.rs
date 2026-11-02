//! Conferencing networking composition — the P2P primitives ASSEMBLED into a call.
//!
//! Each primitive is unit-tested elsewhere (Connect, Listen, Sync/CRDT presence via `e2e_crdt` +
//! `interp_networking`, Send + its modifiers, Await, PeerAgent). This locks the specific
//! composition a conference relies on and that no other test covered: two peers connect over the
//! relay and one delivers the other a MEDIA CHUNK (a byte list) over the `unreliably` datagram
//! path — proving the `unreliable` send actually delivers end-to-end through Await, not just parses.

use logicaffeine_compile::interpret_for_ui;
use logicaffeine_system::relay::serve;
use std::time::Duration;

#[tokio::test]
async fn two_peers_exchange_a_media_chunk_over_the_unreliable_path() {
    let relay = serve("127.0.0.1:0").await.expect("relay binds");
    let url = relay.url();

    // The receiver connects, announces its identity, and awaits the sender's media chunk.
    let receiver = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"receiver\".\n\
         \x20   Let sender be a PeerAgent at \"sender\".\n\
         \x20   Await response from sender into payload.\n\
         \x20   Show payload.\n"
    );
    // The sender connects, waits for the receiver to subscribe, then sends UNRELIABLY.
    let sender = format!(
        "## Main\n\
         \x20   Connect to \"{url}\".\n\
         \x20   Listen on \"sender\".\n\
         \x20   Let receiver be a PeerAgent at \"receiver\".\n\
         \x20   Sleep 300.\n\
         \x20   Send unreliably [10, 20, 30] to receiver.\n"
    );

    let r = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&receiver));
    let s = tokio::time::timeout(Duration::from_secs(10), interpret_for_ui(&sender));
    let (rr, sr) = tokio::join!(r, s);
    let rr = rr.expect("receiver did not hang");
    let sr = sr.expect("sender did not hang");

    assert!(
        sr.error.is_none(),
        "sender (connect + unreliable send) ran clean: {:?}",
        sr.error
    );
    assert!(
        rr.error.is_none(),
        "receiver (connect + await) ran clean: {:?}",
        rr.error
    );
    assert!(
        rr.lines.iter().any(|l| l == "[10, 20, 30]"),
        "the receiver must get the media chunk over the unreliable path; got {:?}",
        rr.lines
    );
}
