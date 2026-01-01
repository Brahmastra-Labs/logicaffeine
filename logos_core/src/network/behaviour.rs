//! Mesh network behaviour combining request-response, mDNS, and GossipSub.

use crate::network::protocol::{LogosCodec, LOGOS_PROTOCOL};
use libp2p::gossipsub::{self, IdentTopic, MessageAuthenticity};
use libp2p::identity::Keypair;
use libp2p::mdns;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::NetworkBehaviour;
use std::time::Duration;

/// Combined network behaviour for the Logos mesh.
///
/// Integrates:
/// - Request-response: For sending messages between agents
/// - mDNS: For local peer discovery
/// - GossipSub: For pub/sub messaging (Phase 52)
#[derive(NetworkBehaviour)]
pub struct MeshBehaviour {
    /// Request-response protocol for agent communication
    pub request_response: request_response::Behaviour<LogosCodec>,
    /// mDNS for local network peer discovery
    pub mdns: mdns::tokio::Behaviour,
    /// GossipSub for pub/sub messaging (Phase 52)
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

        // Phase 52: Configure GossipSub
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
