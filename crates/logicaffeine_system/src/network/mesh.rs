//! P2P Mesh Networking Layer
//!
//! Provides libp2p-based peer-to-peer networking with automatic peer discovery,
//! request-response messaging, and GossipSub pub/sub. This is the foundational
//! layer for distributed CRDT synchronization.
//!
//! # Architecture
//!
//! - [`MeshNode`]: Core swarm management with background event loop
//! - [`PeerAgent`]: Handle to a remote peer for sending messages
//! - Global functions: Simplified API for common operations
//!
//! # Transport
//!
//! Supports both TCP+Noise+Yamux and QUIC for flexible connectivity.
//! mDNS is used for automatic local network discovery.
//!
//! # Features
//!
//! Requires the `networking` feature.
//!
//! # Example
//!
//! ```rust,ignore
//! use logicaffeine_system::network::{listen, connect, send, PeerAgent};
//!
//! // Start listening
//! listen("/ip4/0.0.0.0/tcp/8000").await?;
//!
//! // Connect to a peer
//! connect("/ip4/192.168.1.100/tcp/8000").await?;
//!
//! // Send a message
//! let peer = PeerAgent::new("/ip4/192.168.1.100/tcp/8000/p2p/12D3Koo...")?;
//! send(&peer, &MyMessage { data: 42 }).await?;
//! ```

use crate::network::behaviour::{MeshBehaviour, MeshBehaviourEvent};
use crate::network::protocol::{LogosRequest, LogosResponse};
use crate::network::wire;
use crate::network::gossip;
use futures::prelude::*;
use libp2p::gossipsub;
use libp2p::request_response::{self, OutboundRequestId};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;
use tokio::sync::{mpsc, oneshot, Mutex};

/// Global command channel for gossip operations.
static GOSSIP_TX: OnceLock<mpsc::Sender<MeshCommand>> = OnceLock::new();

/// Error type for network operations.
///
/// All network operations can fail with one of these error variants.
/// Each variant includes a descriptive message for debugging.
#[derive(Debug, Clone)]
pub enum NetworkError {
    /// Failed to establish connection to a remote peer.
    ConnectionFailed(String),
    /// Failed to send a message (peer disconnected, timeout, etc.).
    SendFailed(String),
    /// Network operation timed out waiting for response.
    Timeout,
    /// Peer not found at the specified address.
    PeerNotFound(String),
    /// Message serialization or deserialization failed.
    SerializationFailed(String),
    /// Invalid multiaddr format (e.g., "/not/a/valid/addr").
    InvalidAddress(String),
    /// Mesh node not initialized (call `listen` or `connect` first).
    NotInitialized,
    /// Internal error in the networking layer.
    Internal(String),
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionFailed(addr) => write!(f, "Connection failed to {}", addr),
            Self::SendFailed(reason) => write!(f, "Send failed: {}", reason),
            Self::Timeout => write!(f, "Network operation timed out"),
            Self::PeerNotFound(addr) => write!(f, "Peer not found: {}", addr),
            Self::SerializationFailed(reason) => write!(f, "Serialization failed: {}", reason),
            Self::InvalidAddress(addr) => write!(f, "Invalid address: {}", addr),
            Self::NotInitialized => write!(f, "Mesh node not initialized"),
            Self::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for NetworkError {}

impl From<wire::WireError> for NetworkError {
    fn from(e: wire::WireError) -> Self {
        NetworkError::SerializationFailed(e.to_string())
    }
}

/// Handle for a remote agent on the mesh network.
///
/// `PeerAgent` represents a connection target on the P2P network. It wraps
/// a multiaddr (possibly including a peer ID) and provides a typed interface
/// for network operations.
///
/// # Creating a PeerAgent
///
/// ```rust,ignore
/// // Basic address (peer ID will be discovered on connect)
/// let peer = PeerAgent::new("/ip4/192.168.1.100/tcp/8000")?;
///
/// // Address with known peer ID (preferred for direct connections)
/// let peer = PeerAgent::new("/ip4/192.168.1.100/tcp/8000/p2p/12D3KooW...")?;
/// ```
///
/// # Sending Messages
///
/// ```rust,ignore
/// use logicaffeine_system::network::send;
///
/// let peer = PeerAgent::new("/ip4/192.168.1.100/tcp/8000/p2p/12D3KooW...")?;
/// send(&peer, &MyMessage { data: 42 }).await?;
/// ```
#[derive(Debug, Clone)]
pub struct PeerAgent {
    /// The remote peer's ID (extracted from `/p2p/...` component if present).
    pub peer_id: Option<PeerId>,
    /// The multiaddr for the peer.
    addr: Multiaddr,
}

impl PeerAgent {
    /// Create a new PeerAgent handle for a remote address.
    ///
    /// # Arguments
    /// * `addr` - A multiaddr string (e.g., "/ip4/127.0.0.1/tcp/8000")
    ///
    /// # Returns
    /// * `Ok(PeerAgent)` if the address is valid
    /// * `Err(NetworkError)` if the address format is invalid
    pub fn new(addr: &str) -> Result<Self, NetworkError> {
        if addr.is_empty() {
            return Err(NetworkError::InvalidAddress(
                "Address cannot be empty".to_string(),
            ));
        }

        let multiaddr: Multiaddr = addr
            .parse()
            .map_err(|e| NetworkError::InvalidAddress(format!("{}: {}", addr, e)))?;

        // Try to extract peer ID from multiaddr if present (e.g., /p2p/QmXyz...)
        let peer_id = multiaddr.iter().find_map(|proto| {
            if let libp2p::multiaddr::Protocol::P2p(id) = proto {
                Some(id)
            } else {
                None
            }
        });

        Ok(Self {
            peer_id,
            addr: multiaddr,
        })
    }

    /// Get the address of this peer as a string.
    pub fn addr(&self) -> String {
        self.addr.to_string()
    }

    /// Get the multiaddr.
    pub fn multiaddr(&self) -> &Multiaddr {
        &self.addr
    }
}

/// Commands sent to the mesh event loop.
///
/// The mesh node runs in a background task and communicates via this command enum.
/// Commands are sent through an mpsc channel and processed in the event loop.
enum MeshCommand {
    /// Start listening on an address.
    Listen {
        addr: Multiaddr,
        response: oneshot::Sender<Result<Multiaddr, NetworkError>>,
    },
    /// Dial (connect to) a remote peer.
    Dial {
        addr: Multiaddr,
        response: oneshot::Sender<Result<(), NetworkError>>,
    },
    /// Send a request and await response.
    Send {
        peer_id: PeerId,
        data: Vec<u8>,
        response: oneshot::Sender<Result<Vec<u8>, NetworkError>>,
    },
    /// Get current listening addresses.
    GetListenAddrs {
        response: oneshot::Sender<Vec<Multiaddr>>,
    },
    /// Subscribe to a GossipSub topic.
    GossipSubscribe {
        topic: String,
    },
    /// Publish to a GossipSub topic with retry tracking.
    GossipPublish {
        topic: String,
        data: Vec<u8>,
        retry_count: u8,
    },
}

/// The mesh node - manages the libp2p swarm.
///
/// `MeshNode` is the core networking component that manages the libp2p swarm.
/// It runs a background event loop that handles:
///
/// - Incoming connections and messages
/// - Outgoing connection requests
/// - mDNS peer discovery
/// - GossipSub message routing
///
/// # Lifecycle
///
/// The node is created via [`MeshNode::new()`] which spawns the background
/// event loop. Commands are sent via an internal channel. The node remains
/// active until all references are dropped.
///
/// # Global Instance
///
/// For convenience, a global mesh instance is available via the module-level
/// functions [`listen`], [`connect`], and [`send`].
pub struct MeshNode {
    /// Channel to send commands to the event loop.
    command_tx: mpsc::Sender<MeshCommand>,
    /// Local peer ID (derived from the node's keypair).
    local_peer_id: PeerId,
}

impl MeshNode {
    /// Create and start a new mesh node.
    ///
    /// Spawns a background task to run the swarm event loop.
    pub async fn new() -> Result<Self, NetworkError> {
        let (command_tx, command_rx) = mpsc::channel(256);

        // Build the swarm with keypair for GossipSub message signing
        let swarm = libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )
            .map_err(|e| NetworkError::Internal(format!("TCP setup failed: {}", e)))?
            .with_quic()
            .with_behaviour(|key| MeshBehaviour::new(key.public().to_peer_id(), key))
            .map_err(|e| NetworkError::Internal(format!("Behaviour setup failed: {}", e)))?
            .build();

        let local_peer_id = *swarm.local_peer_id();

        // Store command channel for gossip functions
        let command_tx_clone = command_tx.clone();
        GOSSIP_TX.get_or_init(|| command_tx_clone);

        // Spawn the event loop
        tokio::spawn(Self::event_loop(swarm, command_rx));

        Ok(Self {
            command_tx,
            local_peer_id,
        })
    }

    /// The main event loop for the mesh node.
    async fn event_loop(mut swarm: Swarm<MeshBehaviour>, mut command_rx: mpsc::Receiver<MeshCommand>) {
        // Track pending outbound requests
        let mut pending_requests: HashMap<OutboundRequestId, oneshot::Sender<Result<Vec<u8>, NetworkError>>> =
            HashMap::new();

        // Track pending dials (by multiaddr string -> response sender)
        let mut pending_dials: HashMap<String, oneshot::Sender<Result<(), NetworkError>>> =
            HashMap::new();

        // Track known peers by address
        let mut addr_to_peer: HashMap<String, PeerId> = HashMap::new();

        loop {
            tokio::select! {
                // Handle commands from the API
                Some(cmd) = command_rx.recv() => {
                    match cmd {
                        MeshCommand::Listen { addr, response } => {
                            match swarm.listen_on(addr.clone()) {
                                Ok(_) => {
                                    // Will send actual address once we get NewListenAddr event
                                    // For now, send the requested address
                                    let _ = response.send(Ok(addr));
                                }
                                Err(e) => {
                                    let _ = response.send(Err(NetworkError::Internal(e.to_string())));
                                }
                            }
                        }
                        MeshCommand::Dial { addr, response } => {
                            let addr_str = addr.to_string();
                            match swarm.dial(addr) {
                                Ok(_) => {
                                    pending_dials.insert(addr_str, response);
                                }
                                Err(e) => {
                                    let _ = response.send(Err(NetworkError::ConnectionFailed(e.to_string())));
                                }
                            }
                        }
                        MeshCommand::Send { peer_id, data, response } => {
                            let request_id = swarm.behaviour_mut()
                                .request_response
                                .send_request(&peer_id, LogosRequest(data));
                            pending_requests.insert(request_id, response);
                        }
                        MeshCommand::GetListenAddrs { response } => {
                            let addrs: Vec<Multiaddr> = swarm.listeners().cloned().collect();
                            let _ = response.send(addrs);
                        }
                        // GossipSub commands
                        MeshCommand::GossipSubscribe { topic } => {
                            match swarm.behaviour_mut().subscribe(&topic) {
                                Ok(_) => {
                                    eprintln!("[GOSSIP] Subscribed to '{}'", topic);
                                }
                                Err(e) => {
                                    eprintln!("[GOSSIP] Subscribe failed: {:?}", e);
                                }
                            }
                        }
                        MeshCommand::GossipPublish { topic, data, retry_count } => {
                            // Test hook: drop message if network is paused
                            #[cfg(test)]
                            if test_control::is_paused() {
                                eprintln!("[GOSSIP] Network paused, dropping publish to '{}'", topic);
                                continue;
                            }

                            const MAX_RETRIES: u8 = 5;
                            match swarm.behaviour_mut().publish(&topic, data.clone()) {
                                Ok(_) => {
                                    eprintln!("[GOSSIP] Published to '{}'", topic);
                                }
                                Err(gossipsub::PublishError::InsufficientPeers) if retry_count < MAX_RETRIES => {
                                    // Retry with delay - spawn a task to re-queue the publish
                                    eprintln!("[GOSSIP] InsufficientPeers, scheduling retry ({}/{})", retry_count + 1, MAX_RETRIES);
                                    #[cfg(test)]
                                    test_control::increment_retry();
                                    if let Some(tx) = GOSSIP_TX.get() {
                                        let tx = tx.clone();
                                        let topic = topic.clone();
                                        let data = data.clone();
                                        let next_retry = retry_count + 1;
                                        tokio::spawn(async move {
                                            // Wait for mesh to form (exponential backoff: 1s, 2s, 4s, 8s, 16s)
                                            let delay = std::time::Duration::from_secs(1 << retry_count);
                                            tokio::time::sleep(delay).await;
                                            let _ = tx.send(MeshCommand::GossipPublish { topic, data, retry_count: next_retry }).await;
                                        });
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[GOSSIP] Publish failed after {} retries: {:?}", retry_count, e);
                                }
                            }
                        }
                    }
                }

                // Handle swarm events
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::NewListenAddr { address, .. } => {
                            eprintln!("[MESH] Listening on {}", address);
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                            let addr = endpoint.get_remote_address().to_string();
                            eprintln!("[MESH] Connected to {} at {}", peer_id, addr);
                            addr_to_peer.insert(addr.clone(), peer_id);

                            // Complete any pending dial for this address
                            if let Some(response) = pending_dials.remove(&addr) {
                                let _ = response.send(Ok(()));
                            }
                        }
                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            eprintln!("[MESH] Disconnected from {}", peer_id);
                        }
                        SwarmEvent::Behaviour(MeshBehaviourEvent::RequestResponse(event)) => {
                            match event {
                                request_response::Event::Message { peer, message } => {
                                    match message {
                                        request_response::Message::Request { request, channel, .. } => {
                                            eprintln!("[MESH] Received request from {}", peer);
                                            // Echo the request back as response for now
                                            let _ = swarm.behaviour_mut()
                                                .request_response
                                                .send_response(channel, LogosResponse(request.0));
                                        }
                                        request_response::Message::Response { request_id, response } => {
                                            if let Some(sender) = pending_requests.remove(&request_id) {
                                                let _ = sender.send(Ok(response.0));
                                            }
                                        }
                                    }
                                }
                                request_response::Event::OutboundFailure { request_id, error, .. } => {
                                    if let Some(sender) = pending_requests.remove(&request_id) {
                                        let _ = sender.send(Err(NetworkError::SendFailed(error.to_string())));
                                    }
                                }
                                request_response::Event::InboundFailure { error, .. } => {
                                    eprintln!("[MESH] Inbound request failed: {}", error);
                                }
                                request_response::Event::ResponseSent { .. } => {}
                            }
                        }
                        SwarmEvent::Behaviour(MeshBehaviourEvent::Mdns(event)) => {
                            match event {
                                libp2p::mdns::Event::Discovered(peers) => {
                                    for (peer_id, addr) in peers {
                                        eprintln!("[MESH] mDNS discovered {} at {}", peer_id, addr);
                                        // Only dial if not already connected
                                        if !swarm.is_connected(&peer_id) {
                                            // Dial using the full multiaddr for better reliability
                                            if let Err(e) = swarm.dial(addr.clone()) {
                                                eprintln!("[MESH] Failed to dial {}: {:?}", addr, e);
                                            } else {
                                                eprintln!("[MESH] Dialing {}", addr);
                                            }
                                        }
                                        swarm.add_peer_address(peer_id, addr.clone());
                                        addr_to_peer.insert(addr.to_string(), peer_id);
                                    }
                                }
                                libp2p::mdns::Event::Expired(peers) => {
                                    for (peer_id, addr) in peers {
                                        eprintln!("[MESH] mDNS expired {} at {}", peer_id, addr);
                                    }
                                }
                            }
                        }
                        // GossipSub events
                        SwarmEvent::Behaviour(MeshBehaviourEvent::Gossipsub(event)) => {
                            match event {
                                gossipsub::Event::Message { message, .. } => {
                                    let topic = message.topic.as_str().to_string();
                                    let data = message.data;
                                    eprintln!("[GOSSIP] Received {} bytes on '{}'", data.len(), topic);
                                    // Route to subscription handler
                                    tokio::spawn(async move {
                                        gossip::on_message(&topic, data).await;
                                    });
                                }
                                gossipsub::Event::Subscribed { peer_id, topic } => {
                                    eprintln!("[GOSSIP] {} subscribed to {}", peer_id, topic);
                                }
                                gossipsub::Event::Unsubscribed { peer_id, topic } => {
                                    eprintln!("[GOSSIP] {} unsubscribed from {}", peer_id, topic);
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Listen on an address.
    pub async fn listen(&self, addr: &str) -> Result<Multiaddr, NetworkError> {
        let multiaddr: Multiaddr = addr
            .parse()
            .map_err(|e| NetworkError::InvalidAddress(format!("{}: {}", addr, e)))?;

        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(MeshCommand::Listen {
                addr: multiaddr,
                response: tx,
            })
            .await
            .map_err(|_| NetworkError::Internal("Command channel closed".to_string()))?;

        rx.await
            .map_err(|_| NetworkError::Internal("Response channel closed".to_string()))?
    }

    /// Dial a remote peer.
    pub async fn dial(&self, addr: &str) -> Result<(), NetworkError> {
        let multiaddr: Multiaddr = addr
            .parse()
            .map_err(|e| NetworkError::InvalidAddress(format!("{}: {}", addr, e)))?;

        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(MeshCommand::Dial {
                addr: multiaddr,
                response: tx,
            })
            .await
            .map_err(|_| NetworkError::Internal("Command channel closed".to_string()))?;

        rx.await
            .map_err(|_| NetworkError::Internal("Response channel closed".to_string()))?
    }

    /// Send a message to a peer and await response.
    pub async fn send_bytes(
        &self,
        peer_id: PeerId,
        data: Vec<u8>,
    ) -> Result<Vec<u8>, NetworkError> {
        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send(MeshCommand::Send {
                peer_id,
                data,
                response: tx,
            })
            .await
            .map_err(|_| NetworkError::Internal("Command channel closed".to_string()))?;

        rx.await
            .map_err(|_| NetworkError::Internal("Response channel closed".to_string()))?
    }

    /// Send a serializable message to a peer.
    pub async fn send<T: Serialize, R: DeserializeOwned>(
        &self,
        peer: &PeerAgent,
        msg: &T,
    ) -> Result<R, NetworkError> {
        let peer_id = peer
            .peer_id
            .ok_or_else(|| NetworkError::PeerNotFound("No peer ID in address".to_string()))?;

        let data = wire::encode(msg)?;
        let response_bytes = self.send_bytes(peer_id, data).await?;
        let response: R = wire::decode(&response_bytes)?;
        Ok(response)
    }

    /// Get the local peer ID.
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Get current listening addresses.
    pub async fn listen_addrs(&self) -> Vec<Multiaddr> {
        let (tx, rx) = oneshot::channel();
        if self
            .command_tx
            .send(MeshCommand::GetListenAddrs { response: tx })
            .await
            .is_err()
        {
            return vec![];
        }
        rx.await.unwrap_or_default()
    }
}

// =============================================================================
// Global MESH instance for simple API
// =============================================================================

static MESH: OnceLock<Mutex<Option<MeshNode>>> = OnceLock::new();

/// Initialize the global mesh node.
async fn ensure_mesh() -> Result<(), NetworkError> {
    let mutex = MESH.get_or_init(|| Mutex::new(None));
    let mut guard = mutex.lock().await;
    if guard.is_none() {
        *guard = Some(MeshNode::new().await?);
    }
    Ok(())
}

/// Get a reference to the global mesh node.
async fn get_mesh() -> Result<tokio::sync::MutexGuard<'static, Option<MeshNode>>, NetworkError> {
    ensure_mesh().await?;
    let mutex = MESH.get().ok_or(NetworkError::NotInitialized)?;
    Ok(mutex.lock().await)
}

/// Listen for incoming connections on the specified address.
///
/// # Arguments
/// * `addr` - A multiaddr string (e.g., "/ip4/0.0.0.0/tcp/8000")
///
/// # Example (LOGOS)
/// ```logos
/// Listen on "/ip4/0.0.0.0/tcp/8000".
/// ```
pub async fn listen(addr: &str) -> Result<(), NetworkError> {
    if addr.is_empty() {
        return Err(NetworkError::InvalidAddress(
            "Address cannot be empty".to_string(),
        ));
    }

    let guard = get_mesh().await?;
    let mesh = guard.as_ref().ok_or(NetworkError::NotInitialized)?;
    mesh.listen(addr).await?;
    Ok(())
}

/// Connect to a remote peer at the specified address.
///
/// # Arguments
/// * `addr` - A multiaddr string (e.g., "/ip4/127.0.0.1/tcp/8000")
///
/// # Example (LOGOS)
/// ```logos
/// Connect to "/ip4/127.0.0.1/tcp/8000".
/// ```
pub async fn connect(addr: &str) -> Result<(), NetworkError> {
    if addr.is_empty() {
        return Err(NetworkError::InvalidAddress(
            "Address cannot be empty".to_string(),
        ));
    }

    let guard = get_mesh().await?;
    let mesh = guard.as_ref().ok_or(NetworkError::NotInitialized)?;
    mesh.dial(addr).await
}

/// Send a serializable message to a peer agent.
///
/// # Example (LOGOS)
/// ```logos
/// Let remote be a PeerAgent at "/ip4/127.0.0.1/tcp/8000/p2p/QmXyz...".
/// Send msg to remote.
/// ```
pub async fn send<T: Serialize>(peer: &PeerAgent, msg: &T) -> Result<(), NetworkError> {
    let guard = get_mesh().await?;
    let mesh = guard.as_ref().ok_or(NetworkError::NotInitialized)?;

    let peer_id = peer
        .peer_id
        .ok_or_else(|| NetworkError::PeerNotFound("No peer ID in address".to_string()))?;

    let data = wire::encode(msg)?;
    let _ = mesh.send_bytes(peer_id, data).await?;
    Ok(())
}

/// Get the local peer ID.
pub async fn local_peer_id() -> Result<PeerId, NetworkError> {
    let guard = get_mesh().await?;
    let mesh = guard.as_ref().ok_or(NetworkError::NotInitialized)?;
    Ok(mesh.local_peer_id())
}

// =============================================================================
// GossipSub public API
// =============================================================================

/// Publishes data to a GossipSub topic.
///
/// Messages are broadcast to all peers subscribed to the topic. The mesh
/// automatically handles retries if no peers are initially available.
///
/// # Arguments
///
/// * `topic` - The topic name (any string)
/// * `data` - Raw bytes to publish
///
/// # Example (LOGOS)
/// ```logos
/// Sync state on "game-room".
/// Increase state's clicks by 1.  // Auto-publishes via GossipSub
/// ```
pub async fn gossip_publish(topic: &str, data: Vec<u8>) {
    // Ensure mesh is initialized
    if ensure_mesh().await.is_err() {
        eprintln!("[GOSSIP] Mesh not initialized, cannot publish");
        return;
    }

    if let Some(tx) = GOSSIP_TX.get() {
        if tx.send(MeshCommand::GossipPublish {
            topic: topic.to_string(),
            data,
            retry_count: 0,
        }).await.is_err() {
            eprintln!("[GOSSIP] Command channel closed");
        }
    }
}

/// Subscribes to a GossipSub topic.
///
/// After subscribing, incoming messages on the topic will be routed to the
/// appropriate handler (typically via [`gossip::subscribe`](crate::network::gossip::subscribe)).
///
/// # Arguments
///
/// * `topic` - The topic name to subscribe to
pub async fn gossip_subscribe(topic: &str) {
    // Ensure mesh is initialized
    if ensure_mesh().await.is_err() {
        eprintln!("[GOSSIP] Mesh not initialized, cannot subscribe");
        return;
    }

    if let Some(tx) = GOSSIP_TX.get() {
        if tx.send(MeshCommand::GossipSubscribe {
            topic: topic.to_string(),
        }).await.is_err() {
            eprintln!("[GOSSIP] Command channel closed");
        }
    }
}

// =============================================================================
// Test infrastructure (compiles out in release)
// =============================================================================

#[cfg(test)]
pub mod test_control {
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::OnceLock;

    pub struct MeshTestControl {
        pub pause_publish: AtomicBool,
        pub pause_receive: AtomicBool,
        pub retry_count: AtomicU32,
    }

    static CONTROL: OnceLock<MeshTestControl> = OnceLock::new();

    pub fn get() -> &'static MeshTestControl {
        CONTROL.get_or_init(|| MeshTestControl {
            pause_publish: AtomicBool::new(false),
            pause_receive: AtomicBool::new(false),
            retry_count: AtomicU32::new(0),
        })
    }

    pub fn pause_network() {
        get().pause_publish.store(true, Ordering::SeqCst);
    }

    pub fn resume_network() {
        get().pause_publish.store(false, Ordering::SeqCst);
    }

    pub fn is_paused() -> bool {
        get().pause_publish.load(Ordering::Relaxed)
    }

    pub fn increment_retry() {
        get().retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_retry_count() -> u32 {
        get().retry_count.load(Ordering::Relaxed)
    }

    pub fn reset_retry_count() {
        get().retry_count.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_agent_new() {
        let peer = PeerAgent::new("/ip4/127.0.0.1/tcp/8000");
        assert!(peer.is_ok());
        let peer = peer.unwrap();
        assert_eq!(peer.addr(), "/ip4/127.0.0.1/tcp/8000");
        assert!(peer.peer_id.is_none()); // No /p2p/ component
    }

    #[test]
    fn test_peer_agent_empty_fails() {
        let peer = PeerAgent::new("");
        assert!(peer.is_err());
    }

    #[test]
    fn test_peer_agent_invalid_fails() {
        let peer = PeerAgent::new("not-a-multiaddr");
        assert!(peer.is_err());
    }

    #[tokio::test]
    async fn test_mesh_node_creation() {
        let node = MeshNode::new().await;
        assert!(node.is_ok());
        let node = node.unwrap();
        // Peer ID should be valid
        assert!(!node.local_peer_id().to_string().is_empty());
    }

    #[tokio::test]
    async fn test_listen_empty_fails() {
        // This tests the early return before mesh initialization
        let result = listen("").await;
        assert!(matches!(result, Err(NetworkError::InvalidAddress(_))));
    }

    #[tokio::test]
    async fn test_connect_empty_fails() {
        // This tests the early return before mesh initialization
        let result = connect("").await;
        assert!(matches!(result, Err(NetworkError::InvalidAddress(_))));
    }
}
