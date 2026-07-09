//! Phase 9c — the BROWSER interpreter networks end to end (headless).
//!
//! Drives `interpret_for_ui` itself inside a headless browser: an interpreted
//! `Connect` + `Sync` runs over the `web-sys` WebSocket relay client and a peer
//! (also in the browser) receives the published CRDT counter through a native
//! relay host. As the user noted, the browser can't host a relay, so a native one
//! must run first — `scripts/test-wasm-relay.sh` starts it on `127.0.0.1:9944`
//! and then runs `wasm-pack test --headless`.
//!
//! Only builds + runs on `wasm32` under `wasm-pack`; inert on a normal `cargo
//! test` (it is `cfg(target_arch = "wasm32")`).

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use futures::StreamExt;
use logicaffeine_compile::interpret_for_ui;
use logicaffeine_system::relay_browser::RelayBrowserClient;
use wasm_bindgen_test::*;

// No `run_in_browser`: runs under node with a `WebSocket` polyfill, so the suite
// exercises the browser interpreter networking WITHOUT a headless browser.

/// The relay host `scripts/test-wasm-relay.sh` starts before the browser test.
const RELAY_URL: &str = "ws://127.0.0.1:9944";

#[wasm_bindgen_test]
async fn browser_interpreter_sync_publishes_over_relay() {
    // A peer subscribes to observe what the browser interpreter publishes.
    let (open_tx, open_rx) = futures::channel::oneshot::channel::<()>();
    let open_tx = Rc::new(RefCell::new(Some(open_tx)));
    let (ev_tx, mut ev_rx) = futures::channel::mpsc::unbounded::<(String, Vec<u8>)>();

    let opener = open_tx.clone();
    let peer = RelayBrowserClient::connect(
        RELAY_URL,
        move || {
            if let Some(tx) = opener.borrow_mut().take() {
                let _ = tx.send(());
            }
        },
        move |topic, data| {
            let _ = ev_tx.unbounded_send((topic, data));
        },
    )
    .expect("peer dials the relay host");
    open_rx.await.expect("peer socket opens");
    peer.subscribe("counter").expect("peer subscribes");

    // The browser interpreter itself runs Connect + Sync.
    let program = format!(
        "## Main\n\
         \x20   Let counter be 5.\n\
         \x20   Connect to \"{RELAY_URL}\".\n\
         \x20   Sync counter on \"counter\".\n"
    );
    let result = interpret_for_ui(&program).await;
    assert!(result.error.is_none(), "browser interpreter networking ran");

    // The peer receives what the browser interpreter published.
    let (topic, data) = ev_rx.next().await.expect("the relay delivers the event");
    assert_eq!(topic, "counter");
    assert_eq!(String::from_utf8(data).expect("utf8"), r#"{"":5}"#);
}
