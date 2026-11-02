//! Thin WebSocket relay — the v1 browser↔mesh bridge.
//!
//! A browser tab cannot open a raw socket or `listen`; its only outbound P2P
//! primitive is a WebSocket. So a browser peer reaches the world through a relay:
//! a plain WS server that a browser dials and that forwards its pub/sub traffic.
//! This is deliberately *thin* — it carries no libp2p stack into the browser. The
//! browser speaks the same [`RelayFrame`] wire protocol over `web-sys`/`gloo-net`
//! WebSocket; this module is the native server + a native client (used for tests
//! and native-to-native links).
//!
//! "Native servers are their own relays": any native node runs `serve_bridged`
//! beside its mesh, which cross-forwards the relay's topics with the libp2p
//! gossipsub mesh — so a browser that dials a native node is injected
//! straight into the real mesh, and mesh traffic flows back out to the browser.
//! The pure hub ([`serve`]) is the browser↔browser path and is fully testable
//! without a live mesh; the gossip bridge adds browser↔native.

#![cfg(all(not(target_arch = "wasm32"), feature = "relay"))]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;

pub use crate::relay_proto::RelayFrame;

/// Wrap a frame as a WebSocket binary message / read one back.
fn encode(frame: &RelayFrame) -> Message {
    Message::Binary(frame.to_bytes())
}
fn decode(bytes: &[u8]) -> Option<RelayFrame> {
    RelayFrame::from_bytes(bytes)
}

type ConnId = u64;

/// The relay's routing table: topic → the subscribed connections' outboxes.
/// `gossip_out`, when present, carries a WS client's publishes outward to the
/// libp2p mesh (the bridged mode); inbound mesh messages re-enter via
/// [`Hub::deliver`], which is local-only so there is no echo back to the mesh.
#[derive(Clone)]
struct Hub {
    inner: Arc<Mutex<HashMap<String, Vec<(ConnId, mpsc::UnboundedSender<RelayFrame>)>>>>,
    gossip_out: Option<mpsc::UnboundedSender<(String, Vec<u8>)>>,
}

impl Hub {
    fn new(gossip_out: Option<mpsc::UnboundedSender<(String, Vec<u8>)>>) -> Self {
        Hub {
            inner: Arc::new(Mutex::new(HashMap::new())),
            gossip_out,
        }
    }

    async fn subscribe(&self, conn: ConnId, topic: String, tx: mpsc::UnboundedSender<RelayFrame>) {
        let mut map = self.inner.lock().await;
        let subs = map.entry(topic).or_default();
        if !subs.iter().any(|(c, _)| *c == conn) {
            subs.push((conn, tx));
        }
    }

    async fn unsubscribe(&self, conn: ConnId, topic: &str) {
        let mut map = self.inner.lock().await;
        if let Some(subs) = map.get_mut(topic) {
            subs.retain(|(c, _)| *c != conn);
        }
    }

    /// Deliver `data` to every connection subscribed to `topic` (LOCAL ONLY),
    /// optionally skipping the originating connection `except` so a publisher does
    /// not receive its own message (standard pub/sub, and what keeps a CRDT from
    /// double-counting its own `Sync`). Dead outboxes are pruned; returns how many
    /// subscribers received it.
    async fn deliver_except(&self, topic: &str, data: &[u8], except: Option<ConnId>) -> usize {
        let mut map = self.inner.lock().await;
        let Some(subs) = map.get_mut(topic) else {
            return 0;
        };
        let event = RelayFrame::Event {
            topic: topic.to_string(),
            data: data.to_vec(),
        };
        subs.retain(|(conn, tx)| {
            if Some(*conn) == except {
                return true; // keep the subscription, just don't echo to it
            }
            tx.send(event.clone()).is_ok()
        });
        subs.iter()
            .filter(|(conn, _)| Some(*conn) != except)
            .count()
    }

    /// Deliver to every subscriber (no skip) — the inbound path (a mesh message or
    /// a server-side `inject`), which must not re-emit to the mesh.
    async fn deliver(&self, topic: &str, data: &[u8]) -> usize {
        self.deliver_except(topic, data, None).await
    }

    /// A WS client's publish: deliver to local subscribers (except the publisher)
    /// AND, when bridged, forward to the libp2p mesh so native peers receive it.
    async fn publish(&self, topic: &str, data: &[u8], from: ConnId) -> usize {
        if let Some(gossip) = &self.gossip_out {
            let _ = gossip.send((topic.to_string(), data.to_vec()));
        }
        self.deliver_except(topic, data, Some(from)).await
    }

    async fn drop_conn(&self, conn: ConnId) {
        let mut map = self.inner.lock().await;
        for subs in map.values_mut() {
            subs.retain(|(c, _)| *c != conn);
        }
    }
}

/// A running relay server. Dropping the handle aborts the accept loop.
pub struct RelayServer {
    addr: SocketAddr,
    hub: Hub,
    accept: tokio::task::JoinHandle<()>,
}

impl RelayServer {
    /// The bound address (resolve `127.0.0.1:0` to its actual port for clients).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The `ws://…` URL a client dials.
    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Inject a message into the relay from the server side — local delivery
    /// only (no re-emit to the mesh), the hook the gossip bridge uses to forward
    /// a received mesh message to local WS subscribers.
    pub async fn inject(&self, topic: &str, data: &[u8]) -> usize {
        self.hub.deliver(topic, data).await
    }
}

impl Drop for RelayServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

/// Start a relay (a pure WebSocket pub/sub hub) on `addr` (e.g. `127.0.0.1:0`).
/// This is the browser↔browser-via-relay path and needs no mesh.
pub async fn serve(addr: &str) -> std::io::Result<RelayServer> {
    serve_inner(addr, Hub::new(None)).await
}

/// Start a relay that is **bridged to the libp2p gossipsub mesh** — "a native
/// server being its own relay". A WS client's publish is also gossiped to native
/// peers, and mesh messages on `topics` are injected to WS subscribers. The
/// browser dials this node's WS port and is thereby joined to the real mesh.
///
/// Requires a running mesh node ([`crate::network::MeshNode`]); without one the
/// bridge tasks simply see no mesh traffic and it degrades to a local hub.
///
/// Only available with the `networking` feature (the gossip mesh it bridges to).
#[cfg(feature = "networking")]
pub async fn serve_bridged(addr: &str, topics: Vec<String>) -> std::io::Result<RelayServer> {
    // Outbound: WS publishes → mesh. A pump task drains the channel and gossips.
    let (gtx, mut grx) = mpsc::unbounded_channel::<(String, Vec<u8>)>();
    let server = serve_inner(addr, Hub::new(Some(gtx))).await?;
    tokio::spawn(async move {
        while let Some((topic, data)) = grx.recv().await {
            let _ = crate::network::gossip::publish_raw(&topic, data).await;
        }
    });
    // Inbound: subscribe to each mesh topic; forward received messages to local
    // WS subscribers via `deliver` (local-only ⇒ no echo back to the mesh).
    for topic in topics {
        let hub = server.hub.clone();
        let mut rx = crate::network::gossip::subscribe(&topic).await;
        tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                hub.deliver(&topic, &data).await;
            }
        });
    }
    Ok(server)
}

async fn serve_inner(addr: &str, hub: Hub) -> std::io::Result<RelayServer> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let hub_for_loop = hub.clone();
    let next_id = Arc::new(AtomicU64::new(0));
    let accept = tokio::spawn(async move {
        while let Ok((stream, _peer)) = listener.accept().await {
            let hub = hub_for_loop.clone();
            let id = next_id.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let _ = handle_conn(stream, hub, id).await;
            });
        }
    });
    Ok(RelayServer { addr, hub, accept })
}

async fn handle_conn(stream: TcpStream, hub: Hub, id: ConnId) -> Result<(), ()> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|_| ())?;
    run_conn(ws, hub, id).await;
    Ok(())
}

/// The pub/sub protocol loop, over ANY already-framing-ready WebSocket stream — the one
/// implementation both [`handle_conn`] (which does its own handshake on a raw TCP connection to
/// this module's own listener) and [`RelayHub::accept`] (which takes over a stream an EMBEDDING
/// server already upgraded) run once framing is ready. Subscribe/unsubscribe/publish and the
/// per-connection outbox writer pump are exactly the same either way — a WebSocket frame carries
/// no memory of who answered its handshake.
async fn run_conn<S>(ws: tokio_tungstenite::WebSocketStream<S>, hub: Hub, id: ConnId)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut write, mut read) = ws.split();

    // Per-connection outbox: the hub pushes Events/SubAcks here; a writer pump
    // forwards them to the socket.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RelayFrame>();
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if write.send(encode(&frame)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = read.next().await {
        let bytes = match msg {
            Message::Binary(b) => b,
            Message::Text(t) => t.into_bytes(),
            Message::Close(_) => break,
            _ => continue,
        };
        let Some(frame) = decode(&bytes) else {
            continue;
        };
        match frame {
            RelayFrame::Subscribe { topic } => {
                hub.subscribe(id, topic.clone(), out_tx.clone()).await;
                let _ = out_tx.send(RelayFrame::SubAck { topic });
            }
            RelayFrame::Unsubscribe { topic } => hub.unsubscribe(id, &topic).await,
            RelayFrame::Publish { topic, data } => {
                hub.publish(&topic, &data, id).await;
            }
            // Server→client frames are never received from a client.
            RelayFrame::SubAck { .. } | RelayFrame::Event { .. } => {}
        }
    }

    hub.drop_conn(id).await;
    writer.abort();
}

/// A `Hub` with no listener of its own — for embedding inside ANOTHER server's accept loop
/// (`largo serve`'s `/_relay` upgrade, registered on `logicaffeine_system::http::server`'s
/// shared loop, or a compiled full-stack binary's own `Serve`). [`serve`] is bind-a-listener plus
/// exactly this; the pub/sub semantics — subscribe, publish (never echoed to the publisher),
/// unsubscribe on drop — are the identical `Hub` either way.
#[derive(Clone)]
pub struct RelayHub {
    hub: Hub,
    next_id: Arc<AtomicU64>,
}

impl Default for RelayHub {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayHub {
    /// A hub with no bridge to the libp2p mesh — the browser↔browser-via-relay path, the same
    /// pub/sub [`serve`] itself runs.
    pub fn new() -> Self {
        RelayHub {
            hub: Hub::new(None),
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Take over an ALREADY-UPGRADED stream: the embedding server has already answered the
    /// client's `101 Switching Protocols` off a request it parsed itself — `prefix` is whatever
    /// bytes it had already read past that request (almost always empty; a client waits for the
    /// `101` before framing its first message). Runs the connection's pub/sub loop until it
    /// closes. Spawns nothing itself — the caller decides whether to await it inline or spawn it,
    /// exactly the choice an embedding accept loop already makes for an ordinary connection.
    pub async fn accept(&self, stream: TcpStream, prefix: Vec<u8>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
            PrefixedStream::new(prefix, stream),
            tokio_tungstenite::tungstenite::protocol::Role::Server,
            None,
        )
        .await;
        run_conn(ws, self.hub.clone(), id).await;
    }

    /// This hub as the upgrade table a server registers: [`RELAY_PATH`], the one path every LOGOS
    /// page dials for `Publish`, `When a message arrives` and a `shared on` store.
    #[cfg(feature = "http")]
    pub fn upgrades(&self) -> Vec<(&'static str, crate::http::server::UpgradeHandler)> {
        let hub = self.clone();
        let handler: crate::http::server::UpgradeHandler = Arc::new(move |stream, prefix| {
            let hub = hub.clone();
            Box::pin(async move { hub.accept(stream, prefix).await })
        });
        vec![(RELAY_PATH, handler)]
    }
}

/// The path a page's relay connection dials on the server that served it.
pub const RELAY_PATH: &str = "/_relay";

/// A stream that reads `prefix` before it reads anything from `inner` — writes always go straight
/// to `inner`. The wrapper an already-upgraded WebSocket stream needs when a caller handed over a
/// few bytes read past the HTTP request it parsed, so those bytes are not lost to the framing that
/// begins right where the HTTP parse left off.
struct PrefixedStream {
    prefix: Vec<u8>,
    at: usize,
    inner: TcpStream,
}

impl PrefixedStream {
    fn new(prefix: Vec<u8>, inner: TcpStream) -> Self {
        PrefixedStream {
            prefix,
            at: 0,
            inner,
        }
    }
}

impl tokio::io::AsyncRead for PrefixedStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.at < this.prefix.len() {
            let take = (this.prefix.len() - this.at).min(buf.remaining());
            buf.put_slice(&this.prefix[this.at..this.at + take]);
            this.at += take;
            return std::task::Poll::Ready(Ok(()));
        }
        std::pin::Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for PrefixedStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// A native relay client — and the reference for what the browser does over
/// `web-sys` WebSocket with the same [`RelayFrame`] protocol.
pub struct RelayClient {
    out: mpsc::UnboundedSender<RelayFrame>,
    events: mpsc::UnboundedReceiver<RelayFrame>,
    acks: mpsc::UnboundedReceiver<String>,
    _pump: tokio::task::JoinHandle<()>,
}

impl RelayClient {
    /// Dial a relay at `url` (`ws://host:port`).
    pub async fn connect(url: &str) -> Result<RelayClient, String> {
        let (ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| format!("relay dial failed: {e}"))?;
        let (mut write, mut read) = ws.split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<RelayFrame>();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel::<RelayFrame>();
        let (ack_tx, ack_rx) = mpsc::unbounded_channel::<String>();

        // One pump task owns the socket: it writes outgoing frames and routes
        // incoming frames to the events / acks channels.
        let pump = tokio::spawn(async move {
            loop {
                tokio::select! {
                    out = out_rx.recv() => match out {
                        Some(frame) => {
                            if write.send(encode(&frame)).await.is_err() { break; }
                        }
                        None => break,
                    },
                    inb = read.next() => match inb {
                        Some(Ok(Message::Binary(b))) => {
                            if let Some(frame) = decode(&b) {
                                match frame {
                                    RelayFrame::SubAck { topic } => { let _ = ack_tx.send(topic); }
                                    ev @ RelayFrame::Event { .. } => { let _ = ev_tx.send(ev); }
                                    _ => {}
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(_)) => break,
                    },
                }
            }
        });

        Ok(RelayClient {
            out: out_tx,
            events: ev_rx,
            acks: ack_rx,
            _pump: pump,
        })
    }

    /// Subscribe to `topic` and await the relay's acknowledgement, so a
    /// subsequent publish from anyone cannot race ahead of the registration.
    pub async fn subscribe(&mut self, topic: &str) -> Result<(), String> {
        self.out
            .send(RelayFrame::Subscribe {
                topic: topic.to_string(),
            })
            .map_err(|_| "relay connection closed".to_string())?;
        loop {
            match self.acks.recv().await {
                Some(t) if t == topic => return Ok(()),
                Some(_) => continue,
                None => return Err("relay closed before subscribe ack".to_string()),
            }
        }
    }

    /// Publish `data` to `topic`.
    pub fn publish(&self, topic: &str, data: Vec<u8>) -> Result<(), String> {
        self.out
            .send(RelayFrame::Publish {
                topic: topic.to_string(),
                data,
            })
            .map_err(|_| "relay connection closed".to_string())
    }

    /// Await the next event delivered for a subscribed topic.
    pub async fn next_event(&mut self) -> Option<(String, Vec<u8>)> {
        match self.events.recv().await {
            Some(RelayFrame::Event { topic, data }) => Some((topic, data)),
            _ => None,
        }
    }

    /// Drain every event delivered so far WITHOUT blocking — the non-blocking
    /// "drain at a sync point" primitive the interpreter's `Sync` uses.
    pub fn drain_events(&mut self) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        while let Ok(RelayFrame::Event { topic, data }) = self.events.try_recv() {
            out.push((topic, data));
        }
        out
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_system/relay/tests.rs"]
mod tests;
