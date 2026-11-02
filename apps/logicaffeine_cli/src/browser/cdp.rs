//! The DevTools protocol over `--remote-debugging-pipe`.
//!
//! Chrome reads commands from its fd 3 and writes replies and events to its fd 4, one JSON
//! message per NUL-terminated frame. One reader thread owns the incoming half: a reply is matched
//! to the command that is waiting for it by `id`, and an event is handed to the [`SessionSink`]
//! registered for its `sessionId` (flat sessions, `Target.attachToTarget { flatten: true }`).
//!
//! A sink runs ON the reader thread, so it may only issue commands whose replies nobody waits for
//! ([`Wire::send_detached`]) — waiting there would wait on the thread that delivers the answer.
//!
//! `LOGOS_BROWSER_TRACE=1` prints every frame crossing the pipe to stderr, with each reply's
//! round-trip time.

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::BrowserError;

/// Receives the events of one attached session, on the reader thread.
pub(crate) trait SessionSink: Send + Sync {
    fn event(&self, wire: &Wire, method: &str, params: &Value);
}

/// The writing half plus the reply table — everything a command needs, shared with the reader.
pub(crate) struct Wire {
    writer: Mutex<File>,
    trace: Option<Instant>,
    next_id: AtomicU64,
    state: Mutex<WireState>,
    arrived: Condvar,
}

#[derive(Default)]
struct WireState {
    /// Commands whose caller is blocked on the reply: `None` until it arrives.
    waiting: HashMap<u64, Option<Result<Value, String>>>,
    /// Commands nobody waits for; their replies are dropped on arrival.
    detached: HashSet<u64>,
    sessions: HashMap<String, Arc<dyn SessionSink>>,
    /// Why the pipe closed, once it has.
    closed: Option<String>,
}

impl Wire {
    /// Start the reader thread over `from_browser` and return the connection.
    pub(crate) fn start(to_browser: File, from_browser: File) -> Arc<Wire> {
        let wire = Arc::new(Wire {
            writer: Mutex::new(to_browser),
            trace: std::env::var_os("LOGOS_BROWSER_TRACE")
                .filter(|v| v != "0")
                .map(|_| Instant::now()),
            next_id: AtomicU64::new(1),
            state: Mutex::new(WireState::default()),
            arrived: Condvar::new(),
        });
        let reader = Arc::clone(&wire);
        std::thread::Builder::new()
            .name("cdp-reader".into())
            .spawn(move || reader.read_loop(from_browser))
            .expect("spawn the CDP reader thread");
        wire
    }

    fn read_loop(&self, from_browser: File) {
        let mut input = BufReader::new(from_browser);
        let mut frame = Vec::new();
        let why = loop {
            frame.clear();
            match input.read_until(0, &mut frame) {
                Ok(0) => break "the browser closed its end of the DevTools pipe".to_string(),
                Ok(_) => {}
                Err(e) => break format!("reading the DevTools pipe failed: {e}"),
            }
            if frame.last() == Some(&0) {
                frame.pop();
            }
            if let Some(t0) = self.trace {
                let text = String::from_utf8_lossy(&frame);
                let cut = text.char_indices().nth(240).map_or(text.len(), |(i, _)| i);
                eprintln!(
                    "[cdp {:>8.3}s] <- {}",
                    t0.elapsed().as_secs_f64(),
                    &text[..cut]
                );
            }
            let Ok(msg) = serde_json::from_slice::<Value>(&frame) else {
                continue;
            };
            self.dispatch(msg);
        };
        let mut st = self.state.lock().unwrap();
        st.closed = Some(why);
        drop(st);
        self.arrived.notify_all();
    }

    fn dispatch(&self, msg: Value) {
        if let Some(id) = msg.get("id").and_then(Value::as_u64) {
            let result = match msg.get("error") {
                Some(err) => Err(err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string()
                    + &err
                        .get("data")
                        .and_then(Value::as_str)
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            let mut st = self.state.lock().unwrap();
            if st.detached.remove(&id) {
                return;
            }
            if let Some(slot) = st.waiting.get_mut(&id) {
                *slot = Some(result);
            }
            drop(st);
            self.arrived.notify_all();
            return;
        }
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            return;
        };
        // Browser-level events (no `sessionId`) carry nothing a page needs.
        let Some(session) = msg.get("sessionId").and_then(Value::as_str) else {
            return;
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let sink = self.state.lock().unwrap().sessions.get(session).cloned();
        if let Some(sink) = sink {
            sink.event(self, method, &params);
        }
    }

    fn write(
        &self,
        id: u64,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<(), BrowserError> {
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = Value::String(s.to_string());
        }
        let mut bytes = serde_json::to_vec(&msg).expect("a CDP command serializes");
        if let Some(t0) = self.trace {
            let text = String::from_utf8_lossy(&bytes);
            let cut = text.char_indices().nth(240).map_or(text.len(), |(i, _)| i);
            eprintln!(
                "[cdp {:>8.3}s] -> {}",
                t0.elapsed().as_secs_f64(),
                &text[..cut]
            );
        }
        bytes.push(0);
        let mut w = self.writer.lock().unwrap();
        w.write_all(&bytes)
            .and_then(|_| w.flush())
            .map_err(|e| BrowserError::Protocol {
                method: method.to_string(),
                message: format!("writing to the DevTools pipe failed: {e}"),
            })
    }

    /// Send a command and wait for its reply, failing loudly after `timeout`.
    pub(crate) fn send(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, BrowserError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut st = self.state.lock().unwrap();
            if let Some(why) = &st.closed {
                return Err(BrowserError::Closed(why.clone()));
            }
            st.waiting.insert(id, None);
        }
        if let Err(e) = self.write(id, session, method, params) {
            self.state.lock().unwrap().waiting.remove(&id);
            return Err(e);
        }
        let deadline = Instant::now() + timeout;
        let mut st = self.state.lock().unwrap();
        loop {
            if let Some(Some(_)) = st.waiting.get(&id) {
                let reply = st.waiting.remove(&id).flatten().expect("checked above");
                return reply.map_err(|message| BrowserError::Protocol {
                    method: method.to_string(),
                    message,
                });
            }
            if let Some(why) = st.closed.clone() {
                st.waiting.remove(&id);
                return Err(BrowserError::Closed(why));
            }
            let now = Instant::now();
            if now >= deadline {
                st.waiting.remove(&id);
                st.detached.insert(id);
                return Err(BrowserError::Timeout {
                    what: format!("`{method}`"),
                    after: timeout,
                });
            }
            st = self.arrived.wait_timeout(st, deadline - now).unwrap().0;
        }
    }

    /// Send a command whose reply is dropped — the only kind a [`SessionSink`] may send.
    pub(crate) fn send_detached(&self, session: Option<&str>, method: &str, params: Value) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.state.lock().unwrap().detached.insert(id);
        if self.write(id, session, method, params).is_err() {
            self.state.lock().unwrap().detached.remove(&id);
        }
    }

    pub(crate) fn attach(&self, session: &str, sink: Arc<dyn SessionSink>) {
        self.state
            .lock()
            .unwrap()
            .sessions
            .insert(session.to_string(), sink);
    }

    pub(crate) fn detach(&self, session: &str) {
        self.state.lock().unwrap().sessions.remove(session);
    }

    /// Why the pipe closed, if it has.
    pub(crate) fn closed(&self) -> Option<String> {
        self.state.lock().unwrap().closed.clone()
    }
}
