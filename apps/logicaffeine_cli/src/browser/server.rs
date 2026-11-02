//! A loopback HTTP server for one page bundle, and the relay its pages talk through.
//!
//! A module script must come from an HTTP(S) origin — Chrome refuses `type="module"` from
//! `file://` — so a built page is served the way it ships: `GET` for a file in the bundle, `404`
//! for anything else, one response per connection. It binds `127.0.0.1:0`, so parallel tests each
//! get a port of their own and nothing listens beyond the machine.
//!
//! `/_relay` is the relay a page's `Publish` and `When a message arrives` reach: a WebSocket that
//! speaks `logicaffeine_system::relay_proto::RelayFrame` exactly as the system's own relay does —
//! `Subscribe` answered with `SubAck`, each `Publish` delivered as an `Event` to every OTHER
//! connection subscribed to its topic. Pages a test opens on one [`RelayHub`] reach each other.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use logicaffeine_system::relay_proto::RelayFrame;

/// A relay the pages of one test share: every page opened with it publishes into it and hears
/// what the others publish.
#[derive(Clone, Default)]
pub struct RelayHub {
    state: Arc<Mutex<HubState>>,
}

impl std::fmt::Debug for RelayHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.state.lock().unwrap();
        f.debug_struct("RelayHub")
            .field("connections", &st.conns.len())
            .field("topics", &st.subs.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Default)]
struct HubState {
    /// Each open connection's writer, by the name the page's recorder gave it.
    conns: HashMap<String, Arc<Mutex<TcpStream>>>,
    subs: BTreeMap<String, Vec<String>>,
    /// Per connection: how many messages the hub has read from it, and written to it.
    counts: HashMap<String, (u64, u64)>,
    /// Every frame the hub read from a page, in order.
    received: Vec<RelayFrame>,
}

impl RelayHub {
    /// Deliver `data` on `topic` to every page subscribed to it, as another replica's publish
    /// reaches them.
    pub fn deliver(&self, topic: &str, data: &[u8]) {
        let mut st = self.state.lock().unwrap();
        let to: Vec<String> = st.subs.get(topic).cloned().unwrap_or_default();
        let frame = RelayFrame::Event {
            topic: topic.to_string(),
            data: data.to_vec(),
        };
        for conn in &to {
            st.write(conn, &frame);
        }
    }

    /// How many connections are subscribed to `topic`.
    pub fn subscribers(&self, topic: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .subs
            .get(topic)
            .map_or(0, Vec::len)
    }

    /// Every frame the relay read from a page, in the order it read them.
    pub fn received(&self) -> Vec<RelayFrame> {
        self.state.lock().unwrap().received.clone()
    }

    /// How many messages the hub has read from the connection `token` names, and written to it.
    pub(crate) fn counts(&self, token: &str) -> (u64, u64) {
        self.state
            .lock()
            .unwrap()
            .counts
            .get(token)
            .copied()
            .unwrap_or((0, 0))
    }

    fn join(&self, token: &str, writer: TcpStream) {
        self.state
            .lock()
            .unwrap()
            .conns
            .insert(token.to_string(), Arc::new(Mutex::new(writer)));
    }

    /// One whole message read from `conn`: a frame the hub acts on, or bytes no `RelayFrame`
    /// decodes, which it counts and drops as the system's relay does.
    fn receive(&self, conn: &str, message: &[u8]) {
        let mut st = self.state.lock().unwrap();
        st.counts.entry(conn.to_string()).or_default().0 += 1;
        let frame = RelayFrame::from_bytes(message);
        st.received.extend(frame.clone());
        match frame {
            Some(RelayFrame::Subscribe { topic }) => {
                let subs = st.subs.entry(topic.clone()).or_default();
                if !subs.iter().any(|c| c == conn) {
                    subs.push(conn.to_string());
                }
                st.write(conn, &RelayFrame::SubAck { topic });
            }
            Some(RelayFrame::Unsubscribe { topic }) => {
                if let Some(subs) = st.subs.get_mut(&topic) {
                    subs.retain(|c| c != conn);
                }
            }
            Some(RelayFrame::Publish { topic, data }) => {
                let to: Vec<String> = st
                    .subs
                    .get(&topic)
                    .into_iter()
                    .flatten()
                    .filter(|c| *c != conn)
                    .cloned()
                    .collect();
                let event = RelayFrame::Event { topic, data };
                for c in &to {
                    st.write(c, &event);
                }
            }
            Some(RelayFrame::SubAck { .. } | RelayFrame::Event { .. }) | None => {}
        }
    }

    fn leave(&self, conn: &str) {
        let mut st = self.state.lock().unwrap();
        st.conns.remove(conn);
        for subs in st.subs.values_mut() {
            subs.retain(|c| c != conn);
        }
    }
}

impl HubState {
    /// Write `frame` to `conn` as one binary WebSocket message.
    fn write(&mut self, conn: &str, frame: &RelayFrame) {
        let Some(writer) = self.conns.get(conn).cloned() else {
            return;
        };
        let payload = frame.to_bytes();
        let mut out = vec![0x82];
        match payload.len() {
            n if n < 126 => out.push(n as u8),
            n if n <= 0xFFFF => {
                out.push(126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                out.push(127);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        out.extend_from_slice(&payload);
        if writer.lock().unwrap().write_all(&out).is_ok() {
            self.counts.entry(conn.to_string()).or_default().1 += 1;
        }
    }
}

pub(crate) struct StaticServer {
    origin: String,
    stop: Arc<AtomicBool>,
    port: u16,
    hub: RelayHub,
}

impl StaticServer {
    /// Serve `files` (path → content type and bytes, paths beginning with `/`), and `/_relay` on
    /// `hub`, until dropped.
    pub(crate) fn start(
        files: BTreeMap<String, (String, Vec<u8>)>,
        hub: RelayHub,
    ) -> std::io::Result<StaticServer> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let files = Arc::new(files);
        let halt = Arc::clone(&stop);
        let relay = hub.clone();
        std::thread::Builder::new()
            .name("page-server".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if halt.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conn) = conn else { continue };
                    let files = Arc::clone(&files);
                    let relay = relay.clone();
                    let _ = std::thread::Builder::new()
                        .name("page-server-conn".into())
                        .spawn(move || respond(conn, &files, &relay));
                }
            })?;
        Ok(StaticServer {
            origin: format!("http://127.0.0.1:{port}"),
            stop,
            port,
            hub,
        })
    }

    /// `http://127.0.0.1:<port>`.
    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    /// The relay `/_relay` reaches.
    pub(crate) fn relay(&self) -> &RelayHub {
        &self.hub
    }
}

impl Drop for StaticServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // `incoming()` blocks in `accept`; one connection wakes it to see the flag.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn respond(conn: TcpStream, files: &BTreeMap<String, (String, Vec<u8>)>, hub: &RelayHub) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(&conn);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut websocket_key = None;
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(0) => break,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            Ok(_) => {
                if let Some((name, value)) = header.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("sec-websocket-key") {
                        websocket_key = Some(value.trim().to_string());
                    }
                }
            }
            Err(_) => return,
        }
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let path = target.split(['?', '#']).next().unwrap_or("/");
    if path == "/_relay" {
        if let Some(key) = websocket_key {
            drop(reader);
            let token = target
                .split_once('?')
                .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("harness=")))
                .map(str::to_string)
                .unwrap_or_else(|| format!("unnamed-{}", UNNAMED.fetch_add(1, Ordering::SeqCst)));
            relay(conn, &key, &token, hub);
        }
        return;
    }
    let found = if method == "GET" || method == "HEAD" {
        files.get(path)
    } else {
        None
    };
    let mut out = &conn;
    let head = match found {
        Some((content_type, bytes)) => format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
            bytes.len()
        ),
        None => "HTTP/1.1 404 Not Found\r\ncontent-type: text/plain\r\ncontent-length: 9\r\nconnection: close\r\n\r\nnot found"
            .to_string(),
    };
    let _ = out.write_all(head.as_bytes());
    if let (Some((_, bytes)), true) = (found, method == "GET") {
        let _ = out.write_all(bytes);
    }
    let _ = out.flush();
    let _ = conn.shutdown(Shutdown::Both);
}

/// A name for a relay connection no recorder named.
static UNNAMED: AtomicU64 = AtomicU64::new(1);

/// One page's relay connection, named `token`: the WebSocket handshake, then every message it sends
/// handed to the hub, until it closes.
fn relay(mut conn: TcpStream, key: &str, token: &str, hub: &RelayHub) {
    let accept = {
        let digest = logicaffeine_base::hash::sha1(
            format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
        );
        base64::engine::general_purpose::STANDARD.encode(digest)
    };
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
    );
    if conn.write_all(handshake.as_bytes()).is_err() {
        return;
    }
    let _ = conn.set_read_timeout(None);
    let Ok(writer) = conn.try_clone() else { return };
    hub.join(token, writer);
    let mut message = Vec::new();
    loop {
        let Some((fin, opcode, payload)) = read_frame(&mut conn) else {
            break;
        };
        match opcode {
            0x8 => break,
            0x9 => {
                let mut pong = vec![0x8A, payload.len() as u8];
                pong.extend_from_slice(&payload);
                let _ = conn.write_all(&pong);
            }
            0x0 | 0x1 | 0x2 => {
                message.extend_from_slice(&payload);
                if fin {
                    hub.receive(token, &message);
                    message.clear();
                }
            }
            _ => {}
        }
    }
    hub.leave(token);
    let _ = conn.shutdown(Shutdown::Both);
}

/// One WebSocket frame from a client: `(fin, opcode, unmasked payload)`, or `None` when the
/// connection ends.
fn read_frame(conn: &mut TcpStream) -> Option<(bool, u8, Vec<u8>)> {
    let mut head = [0u8; 2];
    conn.read_exact(&mut head).ok()?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            conn.read_exact(&mut b).ok()?;
            u16::from_be_bytes(b) as usize
        }
        127 => {
            let mut b = [0u8; 8];
            conn.read_exact(&mut b).ok()?;
            u64::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut mask = [0u8; 4];
    if masked {
        conn.read_exact(&mut mask).ok()?;
    }
    let mut payload = vec![0u8; len];
    conn.read_exact(&mut payload).ok()?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Some((fin, opcode, payload))
}
