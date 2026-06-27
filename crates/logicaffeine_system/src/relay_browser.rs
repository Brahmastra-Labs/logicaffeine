//! Browser relay client — a `web-sys` WebSocket that speaks [`RelayFrame`].
//!
//! This is the browser's only P2P door: a tab cannot listen or open a raw
//! socket, so it dials a native node's relay (`ws://…`) and rides the relay into
//! the libp2p mesh. The wire protocol is byte-identical to the native client
//! ([`crate::relay::RelayClient`]) — both serialize [`RelayFrame`] — which is the
//! whole point of sharing [`crate::relay_proto`].
//!
//! wasm-only; carries no libp2p. Events are delivered to a callback (the idiomatic
//! browser shape); the Studio wires that into the interpreter's output/CRDT path.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::relay_proto::RelayFrame;

/// A live relay connection in the browser. Keep it alive for as long as you want
/// to receive events; dropping it tears down the socket + its callbacks.
pub struct RelayBrowserClient {
    ws: web_sys::WebSocket,
    // The closures must outlive the socket, so we own them here.
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_open: Closure<dyn FnMut()>,
    // Opt-in `SubAck` sink, shared with the message closure. The native
    // `RelayClient` awaits a subscribe acknowledgement so a later publish from
    // anyone cannot race the registration; surfacing it here lets the browser do
    // the same when it needs cross-connection ordering.
    suback: Rc<RefCell<Option<Box<dyn Fn(String)>>>>,
}

impl RelayBrowserClient {
    /// Open a relay connection to `url` (`ws://host:port`). `on_open()` fires once
    /// the socket is ready to send (subscribe/publish before it would throw);
    /// `on_event(topic, data)` fires for every `Event` the relay pushes. The
    /// socket is binary, so frames arrive as `ArrayBuffer`.
    pub fn connect(
        url: &str,
        on_open: impl Fn() + 'static,
        on_event: impl Fn(String, Vec<u8>) + 'static,
    ) -> Result<RelayBrowserClient, JsValue> {
        let ws = web_sys::WebSocket::new(url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let suback: Rc<RefCell<Option<Box<dyn Fn(String)>>>> = Rc::new(RefCell::new(None));
        let suback_for_msg = suback.clone();
        let on_message = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
            let Ok(buf) = e.data().dyn_into::<js_sys::ArrayBuffer>() else { return };
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            match RelayFrame::from_bytes(&bytes) {
                Some(RelayFrame::Event { topic, data }) => on_event(topic, data),
                Some(RelayFrame::SubAck { topic }) => {
                    if let Some(cb) = suback_for_msg.borrow().as_ref() {
                        cb(topic);
                    }
                }
                _ => {}
            }
        }) as Box<dyn FnMut(web_sys::MessageEvent)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let on_open_cb = Closure::wrap(Box::new(move || on_open()) as Box<dyn FnMut()>);
        ws.set_onopen(Some(on_open_cb.as_ref().unchecked_ref()));

        Ok(RelayBrowserClient {
            ws,
            _on_message: on_message,
            _on_open: on_open_cb,
            suback,
        })
    }

    /// Whether the socket is open and ready to send.
    pub fn is_open(&self) -> bool {
        self.ws.ready_state() == web_sys::WebSocket::OPEN
    }

    /// Register a sink for subscribe acknowledgements. The relay replies to every
    /// `Subscribe` with a `SubAck`; awaiting it before publishing from a *second*
    /// connection is how a subscriber guarantees its registration is live (the
    /// browser analogue of [`crate::relay::RelayClient::subscribe`] awaiting its
    /// ack). Same-socket ordering already covers the single-connection case, so
    /// this is opt-in.
    pub fn on_suback(&self, cb: impl Fn(String) + 'static) {
        *self.suback.borrow_mut() = Some(Box::new(cb));
    }

    /// Subscribe to `topic` — the relay starts pushing its `Event`s to this
    /// connection's `on_event` and replies with a `SubAck` (see [`Self::on_suback`]).
    pub fn subscribe(&self, topic: &str) -> Result<(), JsValue> {
        self.send(&RelayFrame::Subscribe { topic: topic.to_string() })
    }

    /// Stop receiving `topic`.
    pub fn unsubscribe(&self, topic: &str) -> Result<(), JsValue> {
        self.send(&RelayFrame::Unsubscribe { topic: topic.to_string() })
    }

    /// Publish `data` to `topic` — delivered to every subscriber (browser or, via
    /// the relay's mesh bridge, native) except the publisher itself.
    pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), JsValue> {
        self.send(&RelayFrame::Publish { topic: topic.to_string(), data })
    }

    fn send(&self, frame: &RelayFrame) -> Result<(), JsValue> {
        self.ws.send_with_u8_array(&frame.to_bytes())
    }
}

impl Drop for RelayBrowserClient {
    /// Detach the socket's handlers before the owned [`Closure`]s free, then close
    /// the socket. The relay can deliver a frame (a late `SubAck` or `Event`)
    /// after a short-lived client goes out of scope; without this, the socket
    /// would invoke a freed closure — a hard "closure invoked after being dropped"
    /// throw. Detaching first makes teardown safe whatever is still in flight.
    fn drop(&mut self) {
        self.ws.set_onmessage(None);
        self.ws.set_onopen(None);
        let _ = self.ws.close();
    }
}
