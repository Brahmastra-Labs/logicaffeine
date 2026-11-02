use super::*;

#[test]
fn canonical_topic_normalizes_multiaddr_and_url_to_same() {
    assert_eq!(
        canonical_topic("/ip4/127.0.0.1/tcp/8000"),
        "ws://127.0.0.1:8000"
    );
    assert_eq!(
        canonical_topic("ws://127.0.0.1:8000"),
        "ws://127.0.0.1:8000"
    );
    // The two surfaces name the same peer.
    assert_eq!(
        canonical_topic("/ip4/127.0.0.1/tcp/8000"),
        canonical_topic("ws://127.0.0.1:8000")
    );
}

#[test]
fn canonical_topic_passes_bare_names_through() {
    assert_eq!(canonical_topic("alice"), "alice");
    assert_eq!(canonical_topic("  bob  "), "bob");
    assert_eq!(canonical_topic("game-room-42"), "game-room-42");
}

#[test]
fn ip4_tcp_defaults_to_ws() {
    assert_eq!(
        multiaddr_to_ws_url("/ip4/127.0.0.1/tcp/9944").unwrap(),
        "ws://127.0.0.1:9944"
    );
}

#[test]
fn ip4_tcp_ws_suffix() {
    assert_eq!(
        multiaddr_to_ws_url("/ip4/127.0.0.1/tcp/9944/ws").unwrap(),
        "ws://127.0.0.1:9944"
    );
}

#[test]
fn ip4_tcp_wss_suffix_is_secure() {
    assert_eq!(
        multiaddr_to_ws_url("/ip4/10.0.0.5/tcp/443/wss").unwrap(),
        "wss://10.0.0.5:443"
    );
}

#[test]
fn dns4_hostname() {
    assert_eq!(
        multiaddr_to_ws_url("/dns4/relay.example.com/tcp/443/wss").unwrap(),
        "wss://relay.example.com:443"
    );
}

#[test]
fn dns_and_dns6_and_dnsaddr_are_hostnames() {
    assert_eq!(
        multiaddr_to_ws_url("/dns/example.com/tcp/80/ws").unwrap(),
        "ws://example.com:80"
    );
    assert_eq!(
        multiaddr_to_ws_url("/dns6/example.com/tcp/80").unwrap(),
        "ws://example.com:80"
    );
    assert_eq!(
        multiaddr_to_ws_url("/dnsaddr/example.com/tcp/8000").unwrap(),
        "ws://example.com:8000"
    );
}

#[test]
fn ip6_literal_is_bracketed() {
    assert_eq!(
        multiaddr_to_ws_url("/ip6/::1/tcp/9944").unwrap(),
        "ws://[::1]:9944"
    );
    assert_eq!(
        multiaddr_to_ws_url("/ip6/2001:db8::1/tcp/443/wss").unwrap(),
        "wss://[2001:db8::1]:443"
    );
}

#[test]
fn trailing_p2p_peer_id_is_ignored() {
    assert_eq!(
        multiaddr_to_ws_url("/ip4/127.0.0.1/tcp/9944/ws/p2p/12D3KooWABCDEF").unwrap(),
        "ws://127.0.0.1:9944"
    );
    assert_eq!(
        multiaddr_to_ws_url("/ip4/127.0.0.1/tcp/9944/p2p/12D3KooWABCDEF").unwrap(),
        "ws://127.0.0.1:9944"
    );
}

#[test]
fn ws_and_wss_urls_pass_through() {
    assert_eq!(
        multiaddr_to_ws_url("ws://127.0.0.1:9944").unwrap(),
        "ws://127.0.0.1:9944"
    );
    assert_eq!(
        multiaddr_to_ws_url("wss://relay.example.com/path").unwrap(),
        "wss://relay.example.com/path"
    );
    // case-insensitive scheme detection, value preserved verbatim
    assert_eq!(multiaddr_to_ws_url("WS://Host:1").unwrap(), "WS://Host:1");
}

#[test]
fn whitespace_is_trimmed() {
    assert_eq!(
        multiaddr_to_ws_url("  /ip4/127.0.0.1/tcp/9944  ").unwrap(),
        "ws://127.0.0.1:9944"
    );
}

#[test]
fn empty_is_error() {
    assert!(multiaddr_to_ws_url("").is_err());
    assert!(multiaddr_to_ws_url("   ").is_err());
}

#[test]
fn garbage_is_error() {
    assert!(multiaddr_to_ws_url("not-an-address").is_err());
    assert!(multiaddr_to_ws_url("/not/a/valid/addr").is_err());
}

#[test]
fn missing_port_is_error() {
    assert!(multiaddr_to_ws_url("/ip4/127.0.0.1").is_err());
    assert!(multiaddr_to_ws_url("/ip4/127.0.0.1/tcp").is_err());
}

#[test]
fn non_numeric_port_is_error() {
    assert!(multiaddr_to_ws_url("/ip4/127.0.0.1/tcp/notaport").is_err());
}

#[test]
fn missing_host_is_error() {
    assert!(multiaddr_to_ws_url("/tcp/9944").is_err());
}

#[test]
fn non_tcp_transport_is_error() {
    // The relay is TCP/WS only; a /udp/ multiaddr cannot map to a ws URL.
    assert!(multiaddr_to_ws_url("/ip4/127.0.0.1/udp/9944").is_err());
}
