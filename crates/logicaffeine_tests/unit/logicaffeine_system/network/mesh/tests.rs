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
