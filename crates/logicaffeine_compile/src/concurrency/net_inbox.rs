//! The shared peer-messaging inbox — the relay handle, this node's inbox topic, the received-message
//! buffer, the wire schema caches, and the receive primitives (drain / enqueue / take). Lifted out of
//! the tree-walker `Interpreter` so the bytecode VM's task driver owns the SAME inbox and networking
//! runs byte-identically on both tiers (no tier silently differs). This is a zero-cost relocation:
//! the holders embed a `NetInbox` by value and delegate, so every access monomorphises to the same
//! field access it was before — no dyn dispatch, no extra allocation, the relay I/O path untouched.
//!
//! The async *suspension* model differs per tier and stays with each holder: the tree-walker loops on
//! `poll_tick().await`; the VM blocks its task and the scheduler resumes it. Both call the same
//! non-blocking primitives here (`drain` + `try_take_message` / `try_take_stream`).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::concurrency::marshal::{self, WireSchemaCache, WireTypeRegistry};
use crate::interpreter::{ListRepr, RuntimeValue};

/// A buffered inbound message. A self-describing record list is kept as raw frame bytes so its decode
/// can be deferred to `Await` (where the `view` knob decides lazy-vs-eager); every other shape is
/// decoded in arrival order at drain (preserving the schema cache's keyframe ordering).
pub(crate) enum RecvSlot {
    /// Already decoded (scalars, structs, maps, `Send cached`/`compressed` bodies — order-sensitive).
    Decoded(RuntimeValue),
    /// A deferrable, self-describing record-list frame — decoded (lazily or eagerly) at `Await`.
    RawRecordList(Rc<Vec<u8>>),
    /// A batch STREAM frame (a `Stream … to` send) — deframed into a list by `Await stream`.
    Stream(Rc<Vec<u8>>),
}

/// The peer-messaging state shared by the tree-walker and the VM task. Fields are `pub(crate)` so a
/// holder's networking-statement handlers can read/establish them directly (e.g. set `net` on
/// `Connect`, `inbox` on `Listen`) while the receive primitives below own the buffer mechanics.
pub(crate) struct NetInbox {
    /// The live relay connection, established by `Connect`/`Listen`. `None` until the program connects.
    pub net: Option<logicaffeine_system::net::Net>,
    /// This node's inbox topic — its identity on the relay, set by `Listen`. `None` until then.
    pub inbox: Option<Rc<String>>,
    /// Messages delivered to `inbox` and not yet consumed by an `Await`, kept `(sender, slot)` so an
    /// `Await … from <peer>` matches by sender while leaving others queued for their own `Await`.
    pub received: VecDeque<(String, RecvSlot)>,
    /// `Send cached` schema dictionaries — one per destination peer. Content-addressed, so safe.
    pub send_schema: HashMap<String, WireSchemaCache>,
    /// The receive-side schema dictionary. Decoding ALWAYS goes through it (so `Send cached` resolves).
    pub recv_schema: WireSchemaCache,
    /// Monotonic id stamped on each `Send redundant` message so its FEC shards can be regrouped.
    pub send_msg_id: u64,
    /// Receive-side buffer of incoming FEC shards, keyed by message id, until K arrive to reconstruct.
    pub recv_shards: HashMap<u64, Vec<Vec<u8>>>,
    /// THIS node's published acceptance surface — advertised in our handshake AND the budget the
    /// decode path enforces. One declaration, both advertised and enforced (it cannot drift).
    pub my_profile: marshal::PeerProfile,
    /// Each peer's advertised profile, learned from its handshake. A peer not yet heard from is treated
    /// as the conservative default, so we never over-assume another node's capabilities.
    pub peer_profiles: HashMap<String, marshal::PeerProfile>,
    /// Peers we have already advertised our own profile to — so the handshake is sent once per peer, on
    /// first contact, not on every message.
    pub handshaked: std::collections::HashSet<String>,
    /// THIS program's wire type registry (struct/enum schemas → small ids), set once at startup from the
    /// analysis type table — IDENTICALLY on both tiers. Used to elide type NAMES from the wire when a
    /// peer advertised a MATCHING registry epoch. Empty (epoch 0) until set → never elide.
    pub my_registry: marshal::WireTypeRegistry,
    /// δ-CRDT `Sync`: the causal version we have already shipped to each topic, so the next sync
    /// publishes only the DELTA since then (not the whole set/sequence). Empty until first sync.
    pub sync_versions: HashMap<String, logicaffeine_data::crdt::VClock>,
    /// OFFLINE loopback outbox — with no relay (the VM/wasm-AOT determinism oracles), `publish` queues
    /// the framed bytes HERE instead of dropping them, and `drain` feeds them back through the normal
    /// decode path (topic-filtered to our own inbox). This makes a single-node `Send … to <self>` then
    /// `Await … from <self>` deterministic — the oracle output is transport-independent, so the local
    /// round-trip through the real wire codec is byte-faithful to what a relay would deliver.
    pub offline_loopback: VecDeque<(String, Vec<u8>)>,
}

/// The dedicated topic a peer's handshake travels on — a deterministic transform of its DATA topic, so
/// both peers derive the same one. Handshakes ride HERE, never the data topic, so they never interleave
/// with data: a raw / non-Logos peer listening only on the data topic is entirely unaffected.
pub fn handshake_topic_for(data_topic: &str) -> String {
    format!("{data_topic}#hs")
}

thread_local! {
    /// OFFLINE (single-node) networking mode. When set, `Connect` is a LOCAL NO-OP: the deterministic
    /// engines (the tree-walker / VM oracles, driven on `futures::block_on` with NO relay transport
    /// reachable) have nothing to dial, so `net` stays `None` and the following `Listen`/`Send`/`Sync`
    /// run in their existing offline mode. A real deployment (the browser relay client, or a
    /// relay-connected driver) leaves this unset and dials for real. This is the networking analogue of
    /// [`crate::semantics::temporal::set_fixed_clock`] — a deterministic-execution knob the oracles set
    /// so a `Connect` program has a byte-identical outcome across tree-walker, VM, and the WASM AOT.
    static NET_OFFLINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Enter (or leave) OFFLINE networking mode — see [`NET_OFFLINE`]. Set by the deterministic
/// tree-walker / VM oracles; unset in the browser/relay paths.
pub fn set_net_offline(offline: bool) {
    NET_OFFLINE.with(|c| c.set(offline));
}

/// Whether `Connect` should skip the real dial and run as a single-node local no-op.
pub fn net_is_offline() -> bool {
    NET_OFFLINE.with(|c| c.get())
}

impl NetInbox {
    pub fn new() -> Self {
        NetInbox {
            net: None,
            inbox: None,
            received: VecDeque::new(),
            send_schema: HashMap::new(),
            recv_schema: WireSchemaCache::content_addressed(),
            send_msg_id: 0,
            recv_shards: HashMap::new(),
            my_profile: marshal::PeerProfile::default(),
            peer_profiles: HashMap::new(),
            handshaked: std::collections::HashSet::new(),
            my_registry: WireTypeRegistry::new(Vec::new()),
            sync_versions: HashMap::new(),
            offline_loopback: VecDeque::new(),
        }
    }

    /// Install this program's wire type registry (once, at startup) and derive our advertised registry
    /// epoch from it. Two same-program peers compute the same epoch → they may elide type names.
    pub fn set_registry(&mut self, registry: marshal::WireTypeRegistry) {
        self.my_profile.registry_epoch = registry.epoch();
        self.my_registry = registry;
    }

    /// The handshake to publish (to `peer`'s HANDSHAKE topic) the FIRST time we contact `peer`, and
    /// never again — so each peer learns our acceptance surface exactly once, before any data. `None`
    /// thereafter. The CALLER publishes the returned bytes to `handshake_topic_for(<peer data topic>)`.
    pub fn first_contact_handshake(&mut self, peer: &str) -> Option<Vec<u8>> {
        if self.handshaked.insert(peer.to_string()) {
            Some(self.my_handshake())
        } else {
            None
        }
    }

    /// This node's handshake frame, advertising `my_profile` under our inbox identity (empty until
    /// `Listen` sets it). Published to a peer right after `Connect`/`Listen` so it learns our surface.
    pub fn my_handshake(&self) -> Vec<u8> {
        let from = self.inbox.as_ref().map(|s| s.as_str()).unwrap_or("");
        marshal::make_handshake_frame(from, &self.my_profile)
    }

    /// The profile `peer` advertised — assumed CONSERVATIVE (self-describing, no compression/type-id)
    /// until we have absorbed its handshake, so we never send a form an unannounced peer can't decode.
    pub fn peer_profile(&self, peer: &str) -> marshal::PeerProfile {
        self.peer_profiles.get(peer).copied().unwrap_or_else(marshal::PeerProfile::conservative)
    }

    /// How to encode a message TO `peer`: my surface ∩ the peer's advertised surface. The `Send` path
    /// consults this so it automatically stays within exactly what the receiver exposed.
    pub fn negotiation_for(&self, peer: &str) -> marshal::Negotiated {
        marshal::negotiate(&self.my_profile, &self.peer_profile(peer))
    }

    /// Encode `value` for `dest` using the negotiated MAXIMAL CRUSH — the auto-tuner's full dial search
    /// within the peer's advertised compression surface, type-id name-elision when epochs matched, the
    /// computed-send gate. BOTH the tree-walker and the VM net path call this for a plain send, so the
    /// two tiers produce byte-identical wire by construction (the cross-tier lock holds). Self-
    /// describing, so the receiver decodes it with no hint.
    pub fn encode_negotiated(
        &self,
        from: &str,
        value: &RuntimeValue,
        dest: &str,
        registry: marshal::WireTypeRegistry,
    ) -> Result<Vec<u8>, String> {
        marshal::message_to_wire_negotiated(from, value, &self.negotiation_for(dest), registry)
    }

    /// The next `Send redundant` message id (post-increment), so its FEC shards regroup on receipt.
    pub fn next_msg_id(&mut self) -> u64 {
        let id = self.send_msg_id;
        self.send_msg_id = self.send_msg_id.wrapping_add(1);
        id
    }

    /// Publish raw wire bytes to a peer topic. LOCAL/OFFLINE mode (no relay `Connect`ed — the
    /// playground/test path): a single node has no peer to reach, so a `Send` is a fire-and-forget
    /// no-op, deterministically. A relay-connected node publishes to the transport. Either way `Send`
    /// never ERRORS, so a networked program runs identically on tree-walker, VM, and AOT.
    pub fn publish(&mut self, topic: &str, bytes: Vec<u8>) -> Result<(), String> {
        match self.net.as_ref() {
            Some(net) => net.publish(topic, bytes),
            // OFFLINE: loop the framed message back into our own outbox (single-node determinism), so a
            // following `Await` on this topic reads it — rather than dropping it.
            None => {
                self.offline_loopback.push_back((topic.to_string(), bytes));
                Ok(())
            }
        }
    }

    /// Drain the relay into `received`, keeping only messages addressed to our own inbox. Non-blocking.
    /// `registry` (the program's struct/enum type table) is passed in so a `Send shared` type-id
    /// message resolves; transparent to ordinary self-describing messages.
    pub fn drain(&mut self, registry: WireTypeRegistry) {
        let Some(inbox) = self.inbox.clone() else { return };
        // Own the drained batch so the `net` borrow ends before we push into `received`. With a relay,
        // this is the relay's delivery; OFFLINE it is our own loopback outbox (the messages we've sent
        // to ourselves) — fed through the SAME filter + decode path a relay message would take.
        let drained: Vec<(String, Vec<u8>)> = match self.net.as_mut() {
            Some(net) => net.drain(),
            None => self.offline_loopback.drain(..).collect(),
        };
        let my_handshake_topic = handshake_topic_for(&inbox);
        marshal::with_type_registry(registry, || {
            for (topic, data) in drained {
                // A peer's handshake arrives on our dedicated handshake topic: ABSORB its advertised
                // profile (so a later send to that peer negotiates against its real surface) and never
                // deliver it as a data message.
                if topic.as_str() == my_handshake_topic.as_str() {
                    if let Some((from, profile)) = marshal::parse_handshake_frame(&data) {
                        self.peer_profiles.insert(from, profile);
                    }
                    continue;
                }
                if topic.as_str() != inbox.as_str() {
                    continue;
                }
                // A `Send redundant` FEC shard: buffer by message id and reconstruct the exact
                // message once K shards of that id have arrived, then decode normally.
                if let Some((msg_id, k, _n)) = crate::concurrency::fec::shard_header(&data) {
                    let buf = self.recv_shards.entry(msg_id).or_default();
                    buf.push(data);
                    let recovered = if buf.len() >= k {
                        crate::concurrency::fec::reconstruct_redundant(buf)
                    } else {
                        None
                    };
                    if let Some((_, payload)) = recovered {
                        self.recv_shards.remove(&msg_id);
                        self.enqueue_received(payload);
                    }
                } else {
                    self.enqueue_received(data);
                }
            }
        });
    }

    /// Buffer one inbound frame. A self-describing record list is held UNDECODED so `Await view` can
    /// wrap it zero-copy; every other shape decodes eagerly through the receive-side schema cache.
    fn enqueue_received(&mut self, data: Vec<u8>) {
        // Open the crypto envelope under the active suite (pass-through when none is engaged);
        // a tampered/foreign frame opens to `None` and is dropped here, never decoded.
        let Some(data) = crate::concurrency::channel::open_active(data) else { return };
        // A capability handshake is ABSORBED (the peer's advertised profile is stored), never delivered
        // as a data message — so a subsequent `Send` to that peer negotiates against its real surface.
        if let Some((from, profile)) = marshal::parse_handshake_frame(&data) {
            self.peer_profiles.insert(from, profile);
            return;
        }
        if let Some(sender) = marshal::peek_stream_sender(&data) {
            self.received.push_back((sender, RecvSlot::Stream(Rc::new(data))));
        } else if let Some(sender) = marshal::peek_deferrable_sender(&data) {
            self.received.push_back((sender, RecvSlot::RawRecordList(Rc::new(data))));
        } else if let Some((from, value)) =
            marshal::message_from_wire_cached(&data, &mut self.recv_schema)
        {
            self.received.push_back((from, RecvSlot::Decoded(value)));
        }
    }

    /// Non-blocking: take the first buffered (non-stream) message from `want`, if any. A plain `Await`
    /// ignores STREAM slots (those belong to `Await stream`).
    pub fn try_take_message(&mut self, want: &str, view: bool) -> Option<RuntimeValue> {
        let pos = self
            .received
            .iter()
            .position(|(from, slot)| from == want && !matches!(slot, RecvSlot::Stream(_)))?;
        Some(self.take_received(pos, view))
    }

    /// Non-blocking: take the first buffered STREAM batch from `want`, deframed into a list, if any.
    pub fn try_take_stream(&mut self, want: &str) -> Option<RuntimeValue> {
        let pos = self
            .received
            .iter()
            .position(|(from, slot)| from == want && matches!(slot, RecvSlot::Stream(_)))?;
        Some(self.take_stream(pos))
    }

    /// Pop the buffered message at `pos` and realize its payload. A deferred record list is wrapped
    /// LAZILY under `view` (zero-copy) or fully decoded; an already-decoded slot is returned as-is.
    fn take_received(&mut self, pos: usize, view: bool) -> RuntimeValue {
        let (_, slot) = self.received.remove(pos).unwrap();
        match slot {
            RecvSlot::Decoded(value) => value,
            RecvSlot::RawRecordList(bytes) => {
                if view {
                    ListRepr::from_received_view(bytes)
                        .map(|l| RuntimeValue::List(Rc::new(RefCell::new(l))))
                        .unwrap_or(RuntimeValue::Nothing)
                } else {
                    marshal::message_from_wire(&bytes)
                        .map(|(_, v)| v)
                        .unwrap_or(RuntimeValue::Nothing)
                }
            }
            // Unreachable on the plain `Await` path (it skips stream slots); deframe defensively.
            RecvSlot::Stream(bytes) => RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(
                marshal::deframe_stream_message(&bytes).unwrap_or_default(),
            )))),
        }
    }

    /// Pop the stream slot at `pos` and deframe it into a `List` of its values (malformed → empty).
    fn take_stream(&mut self, pos: usize) -> RuntimeValue {
        let (_, slot) = self.received.remove(pos).unwrap();
        let values = match slot {
            RecvSlot::Stream(bytes) => marshal::deframe_stream_message(&bytes).unwrap_or_default(),
            _ => Vec::new(),
        };
        RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concurrency::marshal::{
        make_handshake_frame, PeerProfile, ReceiveLimits, WireCompression, FEAT_COMPUTED, FEAT_LZ4,
        FEAT_TYPE_ID, FEAT_ZSTD,
    };

    #[test]
    fn absorbs_a_peer_handshake_and_negotiates_to_its_exposed_surface() {
        let mut inbox = NetInbox::new();
        // I speak zstd+lz4+type-id+computed and carry type-registry epoch 5.
        inbox.my_profile.registry_epoch = 5;
        inbox.my_profile.features = FEAT_ZSTD | FEAT_LZ4 | FEAT_TYPE_ID | FEAT_COMPUTED;

        // Before any handshake, a peer is treated CONSERVATIVELY (no optional capabilities).
        assert_eq!(inbox.peer_profile("bob"), PeerProfile::conservative());
        // …so a send to an unheard peer negotiates to a plain, uncompressed, self-describing form.
        let cold = inbox.negotiation_for("bob");
        assert!(!cold.use_type_id && !cold.may_send_computed);
        assert_eq!(cold.compression, WireCompression::None);

        // Bob advertises a RESTRICTIVE surface: lz4 only (no zstd), type-id, same epoch, declines code,
        // small byte budget.
        let bob = PeerProfile {
            limits: ReceiveLimits { max_bytes: 2048, accept_computed: false, ..Default::default() },
            registry_epoch: 5,
            features: FEAT_LZ4 | FEAT_TYPE_ID,
        };
        inbox.enqueue_received(make_handshake_frame("bob", &bob));

        // The handshake was absorbed — stored, NOT queued as a data message.
        assert!(inbox.received.is_empty(), "a handshake is absorbed, never delivered as data");
        assert_eq!(inbox.peer_profile("bob"), bob);

        // Sending to bob now negotiates to exactly his exposed surface.
        let n = inbox.negotiation_for("bob");
        assert!(n.use_type_id, "matching epochs + both type-id → names elided");
        assert!(!n.may_send_computed, "bob declined computed → never ship code to him");
        assert_eq!(n.compression, WireCompression::Lz4, "lz4 is the strongest compression both speak");
        assert_eq!(n.peer_max_bytes, 2048, "stay under bob's byte budget");
    }

    #[test]
    fn null_suite_seal_engages_through_enqueue_and_off_is_byte_identical() {
        use crate::concurrency::channel::{seal_active, with_suite, SUITE_NULL};
        use crate::concurrency::marshal::{message_to_wire_with, WireCodec, WireIntegrity};

        let blob =
            message_to_wire_with("bob", &RuntimeValue::Int(7), WireCodec::Native, WireIntegrity::Checked)
                .expect("encode");

        // OFF: no active suite → the wire is the raw blob and the receive path is unchanged.
        let mut off = NetInbox::new();
        let wire_off = seal_active(blob.clone());
        assert_eq!(wire_off, blob, "no suite → byte-identical wire");
        off.enqueue_received(wire_off);
        assert_eq!(off.received.len(), 1, "message delivered with the channel disengaged");

        // ON (null suite): the wire is the framed envelope and the receiver opens it back to the
        // same message — the seam engages through the real receive path.
        with_suite(Some(SUITE_NULL), || {
            let mut on = NetInbox::new();
            let wire_on = seal_active(blob.clone());
            assert_ne!(wire_on, blob, "active suite → framed envelope on the wire");
            on.enqueue_received(wire_on);
            assert_eq!(on.received.len(), 1, "sealed message opened + delivered under the null suite");

            // A body byte flipped after sealing: open() succeeds (null framing carries no tag of its
            // own) but the inner FNV checksum fails, so the message is dropped — the transitional
            // tamper-detection the plan relies on until an AEAD tag lands.
            let mut tampered = seal_active(blob.clone());
            let last = tampered.len() - 1;
            tampered[last] ^= 0xFF;
            on.enqueue_received(tampered);
            assert_eq!(on.received.len(), 1, "tampered frame dropped, not delivered");
        });
    }

    #[test]
    fn my_handshake_advertises_my_inbox_identity_and_profile() {
        let mut inbox = NetInbox::new();
        inbox.inbox = Some(Rc::new("alice".to_string()));
        inbox.my_profile.registry_epoch = 9;
        let frame = inbox.my_handshake();
        let (from, prof) = marshal::parse_handshake_frame(&frame).expect("my handshake parses");
        assert_eq!(from, "alice", "advertises our inbox identity");
        assert_eq!(prof, inbox.my_profile, "advertises our exact profile");
    }
}
