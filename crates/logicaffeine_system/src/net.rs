//! `Net` — the cross-target relay-backed network handle the interpreter holds.
//!
//! One API, two transports selected at compile time: native dials with the async
//! [`crate::relay::RelayClient`]; the browser dials with the callback-based
//! [`crate::relay_browser::RelayBrowserClient`]. Both speak the same `RelayFrame`
//! protocol to the same relay, so the interpreter's `Sync`/`Connect` lowering is
//! written once and runs on both targets. This is the platform-capability seam
//! for networking, the analogue of `get_platform_vfs()` for files.
//!
//! Events are drained, not awaited: the interpreter's `Sync` is a *sync point*
//! (publish local state, then merge whatever has arrived) — [`Net::drain`] returns
//! the pending messages without blocking, which keeps the tree-walker's linear
//! execution model intact (no background task mutating the environment).

#![cfg(any(all(not(target_arch = "wasm32"), feature = "relay"), target_arch = "wasm32"))]

#[cfg(all(not(target_arch = "wasm32"), feature = "relay"))]
mod imp {
    use crate::relay::RelayClient;

    /// Native relay handle (async `tokio-tungstenite` client).
    pub struct Net {
        client: RelayClient,
    }

    impl Net {
        pub async fn connect(url: &str) -> Result<Net, String> {
            Ok(Net { client: RelayClient::connect(url).await? })
        }

        pub async fn subscribe(&mut self, topic: &str) -> Result<(), String> {
            self.client.subscribe(topic).await
        }

        pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), String> {
            self.client.publish(topic, data)
        }

        /// Non-blocking: every message delivered since the last drain.
        pub fn drain(&mut self) -> Vec<(String, Vec<u8>)> {
            self.client.drain_events()
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use crate::relay_browser::RelayBrowserClient;

    /// Browser relay handle (`web-sys` WebSocket). Received events are pushed onto
    /// a queue by the socket callback; `drain` empties it.
    pub struct Net {
        client: RelayBrowserClient,
        events: Rc<RefCell<VecDeque<(String, Vec<u8>)>>>,
    }

    impl Net {
        pub async fn connect(url: &str) -> Result<Net, String> {
            let (open_tx, open_rx) = futures::channel::oneshot::channel::<()>();
            let open_tx = Rc::new(RefCell::new(Some(open_tx)));
            let events = Rc::new(RefCell::new(VecDeque::new()));

            let opener = open_tx.clone();
            let queue = events.clone();
            let client = RelayBrowserClient::connect(
                url,
                move || {
                    if let Some(tx) = opener.borrow_mut().take() {
                        let _ = tx.send(());
                    }
                },
                move |topic, data| queue.borrow_mut().push_back((topic, data)),
            )
            .map_err(|e| format!("relay connect failed: {e:?}"))?;

            open_rx.await.map_err(|_| "socket closed before it opened".to_string())?;
            Ok(Net { client, events })
        }

        pub async fn subscribe(&mut self, topic: &str) -> Result<(), String> {
            // The browser socket is ordered, so a following publish cannot race
            // the subscribe; no SubAck round-trip is needed.
            self.client.subscribe(topic).map_err(|e| format!("subscribe failed: {e:?}"))
        }

        pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), String> {
            self.client.publish(topic, data).map_err(|e| format!("publish failed: {e:?}"))
        }

        pub fn drain(&mut self) -> Vec<(String, Vec<u8>)> {
            self.events.borrow_mut().drain(..).collect()
        }
    }
}

pub use imp::Net;
