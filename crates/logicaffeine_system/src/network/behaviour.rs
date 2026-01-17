//! libp2p Behaviour Composition
//!
//! Combines multiple libp2p behaviours into a unified [`MeshBehaviour`]:
//!
//! - **Request-Response**: Point-to-point message exchange between agents
//! - **mDNS**: Automatic local peer discovery on the same network
//! - **GossipSub**: Pub/sub broadcast messaging for CRDT synchronization
//!
//! # Architecture
//!
//! The `MeshBehaviour` struct derives `NetworkBehaviour` which auto-generates
//! the event routing code. Each sub-behaviour handles its own protocol while
//! sharing the same libp2p swarm.
//!
//! # Features
//!
//! Requires the `networking` feature.

use crate::network::protocol::{LogosCodec, LOGOS_PROTOCOL};
use libp2p::gossipsub::{self, IdentTopic, MessageAuthenticity};
use libp2p::identity::Keypair;
use libp2p::mdns;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::NetworkBehaviour;
use std::time::Duration;

/// Combined network behaviour for the LOGOS mesh.
///
/// This struct composes three libp2p behaviours into a single unified behaviour:
///
/// - [`request_response`]: Point-to-point messaging with request/response semantics
/// - [`mdns`]: Zero-configuration local network peer discovery
/// - [`gossipsub`]: Pub/sub messaging for broadcast communication
///
/// The `#[derive(NetworkBehaviour)]` macro auto-generates event handling and
/// protocol negotiation code.
///
/// # Example
///
/// ```rust,ignore
/// let behaviour = MeshBehaviour::new(local_peer_id, &keypair);
/// behaviour.subscribe("my-topic")?;
/// behaviour.publish("my-topic", data)?;
/// ```
#[derive(NetworkBehaviour)]
pub struct MeshBehaviour {
    /// Request-response protocol for direct agent-to-agent communication.
    pub request_response: request_response::Behaviour<LogosCodec>,
    /// mDNS for automatic local network peer discovery.
    pub mdns: mdns::tokio::Behaviour,
    /// GossipSub for pub/sub broadcast messaging.
    pub gossipsub: gossipsub::Behaviour,
}

impl MeshBehaviour {
    /// Create a new mesh behaviour with default configuration.
    pub fn new(local_peer_id: libp2p::PeerId, keypair: &Keypair) -> Self {
        // Configure request-response
        let rr_config = request_response::Config::default()
            .with_request_timeout(Duration::from_secs(30));

        let request_response = request_response::Behaviour::new(
            [(LOGOS_PROTOCOL, ProtocolSupport::Full)],
            rr_config,
        );

        // Configure mDNS
        let mdns_config = mdns::Config::default();
        let mdns = mdns::tokio::Behaviour::new(mdns_config, local_peer_id)
            .expect("Failed to create mDNS behaviour");

        // Configure GossipSub with 1s heartbeat for reliable mesh formation
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .build()
            .expect("Valid gossipsub config");

        let gossipsub = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        ).expect("Valid gossipsub behaviour");

        Self {
            request_response,
            mdns,
            gossipsub,
        }
    }

    /// Subscribe to a GossipSub topic.
    pub fn subscribe(&mut self, topic: &str) -> Result<bool, gossipsub::SubscriptionError> {
        let topic = IdentTopic::new(topic);
        self.gossipsub.subscribe(&topic)
    }

    /// Publish to a GossipSub topic.
    pub fn publish(&mut self, topic: &str, data: Vec<u8>) -> Result<gossipsub::MessageId, gossipsub::PublishError> {
        let topic = IdentTopic::new(topic);
        self.gossipsub.publish(topic, data)
    }
}
