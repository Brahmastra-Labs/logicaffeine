use super::*;
use std::io::{Read, Write};

#[test]
fn user_data_encoding_round_trips() {
    for (kind, gen, slot) in [
        (KIND_RECV, 0u16, 0u32),
        (KIND_SEND, u16::MAX, u32::MAX),
        (KIND_ACCEPT, 7, 42),
    ] {
        let encoded = ud(kind, gen, slot);
        assert_eq!(ud_kind(encoded), kind);
        assert_eq!(ud_gen(encoded), gen);
        assert_eq!(ud_slot(encoded), slot);
    }
    assert_eq!(ud_kind(ACCEPT_UD), KIND_ACCEPT);
}

#[test]
fn bind_reuseport_allows_a_second_bind_on_the_same_port() {
    // THE property the shard architecture stands on: N listeners, one port.
    let first = bind_reuseport("127.0.0.1:0".parse().unwrap()).expect("first bind");
    let addr = first.local_addr().expect("addr");
    let second = bind_reuseport(addr).expect("second SO_REUSEPORT bind on the same port");
    assert_eq!(second.local_addr().unwrap().port(), addr.port());
}

#[test]
fn prepare_accepted_socket_sets_tcp_nodelay_and_keeps_the_fd_open() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    let mut client = TcpStream::connect(addr).expect("connect");
    let (accepted, _) = listener.accept().expect("accept");
    assert!(
        !accepted.nodelay().unwrap_or(true),
        "fresh accepted socket starts with Nagle on"
    );
    let fd = accepted.as_raw_fd();
    prepare_accepted_socket(fd).expect("set nodelay");
    assert!(
        accepted.nodelay().expect("query nodelay"),
        "TCP_NODELAY must be set"
    );
    // The fd survived the borrow-only preparation: bytes still flow.
    client.write_all(b"ping").unwrap();
    let mut byte = [0u8; 4];
    let mut server_side = accepted;
    server_side.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"ping");
}

#[test]
fn engine_probe_initializes_a_ring_on_this_box() {
    // This test runs only where the feature+platform gate admits it; a
    // failure here means the box genuinely cannot io_uring.
    probe().expect("io_uring probe ring");
}
