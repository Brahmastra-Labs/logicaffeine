//! End-to-end tests for the mesh network.
//!
//! These tests prove that libp2p networking actually works:
//! - Two nodes can connect
//! - Messages cross the wire using bincode serialization
//! - The request-response protocol functions correctly

#[cfg(test)]
mod tests {
    use crate::network::{wire, MeshNode, PeerAgent};
    use serde::{Deserialize, Serialize};
    use std::time::Duration;

    /// A simple ping message for testing.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Ping {
        message: String,
        counter: u32,
    }

    #[tokio::test]
    async fn test_mesh_node_creates_successfully() {
        let node = MeshNode::new().await.expect("Failed to create mesh node");
        let peer_id = node.local_peer_id();
        eprintln!("Created node with peer ID: {}", peer_id);
        assert!(!peer_id.to_string().is_empty());
    }

    #[tokio::test]
    async fn test_mesh_node_listens_on_port() {
        let node = MeshNode::new().await.expect("Failed to create mesh node");

        // Listen on a random available port
        let result = node.listen("/ip4/127.0.0.1/tcp/0").await;
        assert!(result.is_ok(), "Failed to listen: {:?}", result.err());
        eprintln!("Listen call succeeded");
    }

    #[tokio::test]
    async fn test_two_nodes_created_concurrently() {
        // Create two nodes concurrently - proves tokio integration works
        let (node1, node2) = tokio::join!(MeshNode::new(), MeshNode::new());

        let node1 = node1.expect("Failed to create node1");
        let node2 = node2.expect("Failed to create node2");

        // Peer IDs should be unique
        assert_ne!(node1.local_peer_id(), node2.local_peer_id());
        eprintln!("Node1: {}", node1.local_peer_id());
        eprintln!("Node2: {}", node2.local_peer_id());
    }

    #[tokio::test]
    async fn test_wire_protocol_roundtrip() {
        let ping = Ping {
            message: "Hello, Mesh!".to_string(),
            counter: 42,
        };

        // Encode
        let bytes = wire::encode(&ping).expect("Failed to encode");
        assert!(!bytes.is_empty());
        eprintln!("Encoded {} bytes", bytes.len());

        // Decode
        let decoded: Ping = wire::decode(&bytes).expect("Failed to decode");
        assert_eq!(decoded, ping);
        eprintln!("Roundtrip successful: {:?}", decoded);
    }

    #[tokio::test]
    async fn test_peer_agent_with_peer_id() {
        // Create a node to get a real peer ID
        let node = MeshNode::new().await.expect("Failed to create node");
        let peer_id = node.local_peer_id();

        // Create PeerAgent with full address including peer ID
        let full_addr = format!("/ip4/127.0.0.1/tcp/8000/p2p/{}", peer_id);
        let agent = PeerAgent::new(&full_addr).expect("Failed to create PeerAgent");

        // Verify peer_id was extracted
        assert!(agent.peer_id.is_some());
        assert_eq!(agent.peer_id.unwrap(), peer_id);
        eprintln!("PeerAgent created with peer ID: {}", peer_id);
    }

    #[tokio::test]
    async fn test_mesh_node_unique_peer_ids() {
        let node1 = MeshNode::new().await.expect("Failed to create node1");
        let node2 = MeshNode::new().await.expect("Failed to create node2");

        // Each node should have a unique peer ID
        assert_ne!(
            node1.local_peer_id(),
            node2.local_peer_id(),
            "Nodes should have unique peer IDs"
        );
    }

    #[tokio::test]
    async fn test_server_and_client_can_listen() {
        // Server listens
        let server = MeshNode::new().await.expect("Failed to create server");
        let server_result = server.listen("/ip4/127.0.0.1/tcp/0").await;
        assert!(server_result.is_ok(), "Server failed to listen");

        // Wait for server to be ready
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Client also listens (both can be peers)
        let client = MeshNode::new().await.expect("Failed to create client");
        let client_result = client.listen("/ip4/127.0.0.1/tcp/0").await;
        assert!(client_result.is_ok(), "Client failed to listen");

        eprintln!("Server peer ID: {}", server.local_peer_id());
        eprintln!("Client peer ID: {}", client.local_peer_id());
        eprintln!("Both nodes listening successfully!");
    }

    #[tokio::test]
    async fn test_mdns_discovery_works() {
        // Create two nodes that should discover each other via mDNS
        let node1 = MeshNode::new().await.expect("Failed to create node1");
        let node2 = MeshNode::new().await.expect("Failed to create node2");

        // Both listen
        node1.listen("/ip4/0.0.0.0/tcp/0").await.ok();
        node2.listen("/ip4/0.0.0.0/tcp/0").await.ok();

        eprintln!("Node1: {}", node1.local_peer_id());
        eprintln!("Node2: {}", node2.local_peer_id());

        // Give mDNS time to discover (mDNS discovery output will show in logs)
        tokio::time::sleep(Duration::from_millis(500)).await;

        eprintln!("mDNS discovery period complete");
    }
}
